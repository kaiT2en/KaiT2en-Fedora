// SPDX-License-Identifier: GPL-3.0-or-later
//! Session daemon: sensor in, gamma LUT and brightness target out, at 10 Hz.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::Result;

use crate::als::{self, Sample};
use crate::autobright;
use crate::config::{self, Config};
use crate::gamma::{self, Base, LUT_LEN};
use crate::gnome;
use crate::model::{self, Locus, Vec3};
use crate::mutter::{Mutter, Panel};
use crate::night;
use crate::status::{self, Status};

const TICK: Duration = Duration::from_millis(100);
const CHECK_EVERY: Duration = Duration::from_secs(3);
const GAIN_EPS: f64 = 2e-4;

static STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_: libc::c_int) {
    STOP.store(true, Ordering::SeqCst);
}

struct Output {
    mutter: Mutter,
    panel: Panel,
    base: Base,
    sent: Option<[f64; 3]>,
    checked: Instant,
}

impl Output {
    fn open() -> Result<Self> {
        let mutter = Mutter::connect()?;
        let panel = mutter.panel()?;
        let cur = mutter.get_gamma(&panel)?;
        let base = if cur[0].len() != LUT_LEN {
            let b = gamma::base_from(&cur);
            gamma::save_base(&b);
            b
        } else {
            gamma::load_base().unwrap_or_else(gamma::identity)
        };
        Ok(Output { mutter, panel, base, sent: None, checked: Instant::now() })
    }

    /// Re-read the panel and detect a LUT that mutter rewrote on its own.
    fn check(&mut self) -> Result<()> {
        self.checked = Instant::now();
        let panel = self.mutter.panel()?;
        if panel.serial != self.panel.serial || panel.crtc != self.panel.crtc {
            self.panel = panel;
            self.sent = None;
        }
        let len = self.mutter.gamma_len(&self.panel)?;
        if len != LUT_LEN {
            let cur = self.mutter.get_gamma(&self.panel)?;
            self.base = gamma::base_from(&cur);
            gamma::save_base(&self.base);
            self.sent = None;
        }
        Ok(())
    }

    fn apply(&mut self, gains: [f64; 3]) -> Result<()> {
        if let Some(s) = self.sent {
            if (0..3).all(|i| (s[i] - gains[i]).abs() < GAIN_EPS) {
                return Ok(());
            }
        }
        self.mutter.set_gamma(&self.panel, &gamma::build(&self.base, gains))?;
        self.sent = Some(gains);
        Ok(())
    }

    /// Fade back to the base LUT over about 1.5 s.
    fn restore(&mut self) {
        let from = self.sent.unwrap_or([1.0; 3]);
        for i in 1..=15 {
            let t = i as f64 / 15.0;
            let k = t * t * (3.0 - 2.0 * t);
            let g = [0, 1, 2].map(|c| from[c] + (1.0 - from[c]) * k);
            if let Err(e) = self.mutter.set_gamma(&self.panel, &gamma::build(&self.base, g)) {
                eprintln!("t2-truetone: could not restore the panel LUT: {e:#}");
                return;
            }
            std::thread::sleep(TICK);
        }
    }
}

fn gnome_sync(c: &Config) {
    if c.true_tone || c.night {
        gnome::take_over(gnome::NIGHT_LIGHT);
    } else {
        gnome::give_back(gnome::NIGHT_LIGHT);
    }
}

fn smooth_lab(state: &mut Option<Vec3>, target: Vec3, dt: f64, tau: f64) -> Vec3 {
    let s = state.get_or_insert(target);
    let k = if tau <= 0.0 { 1.0 } else { 1.0 - (-dt / tau).exp() };
    for i in 0..3 {
        s[i] += k * (target[i] - s[i]);
    }
    *s
}

fn night_gains(c: &Config, f: f64) -> [f64; 3] {
    if f <= 0.0 {
        return [1.0; 3];
    }
    let g = |t: f64| model::white_gains(&model::xy_to_xyz(model::planck_xy(t), 1.0));
    let (n, d) = (g(night::kelvin(c, f)), g(6504.0));
    let r = [n[0] / d[0], n[1] / d[1], n[2] / d[2]];
    let m = r.iter().cloned().fold(f64::MIN, f64::max);
    r.map(|v| v / m)
}

/// The automatic brightness thread, started and stopped with the setting.
struct Brightness {
    tx: mpsc::Sender<f32>,
    stop: Arc<AtomicBool>,
    handle: std::thread::JoinHandle<()>,
}

impl Brightness {
    fn start(live: Arc<Mutex<autobright::Live>>) -> Self {
        let (tx, rx) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let handle = std::thread::spawn(move || {
            if let Err(e) = autobright::run(rx, flag, live) {
                autobright::log(&format!("{e:#}"));
            }
        });
        Brightness { tx, stop, handle }
    }

    /// Stop and wait, which gives GNOME its settings back.
    fn stop(self) {
        self.stop.store(true, Ordering::SeqCst);
        if self.handle.join().is_err() {
            autobright::log("the brightness thread panicked");
        }
    }
}

pub fn run() -> Result<()> {
    unsafe {
        libc::signal(libc::SIGTERM, on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t);
        libc::signal(libc::SIGINT, on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t);
    }
    let (tx, rx) = mpsc::channel::<Sample>();
    std::thread::spawn(move || als::run(vec![tx]));

    let locus = Locus::new();
    let mut cfg = config::load();
    let mut cfg_mtime = config::mtime();
    gnome_sync(&cfg);

    let mut out: Option<Output> = None;
    let mut out_retry = Instant::now();
    let mut last_err: Option<String> = None;
    let mut sample: Option<Sample> = None;
    let d65_lab = model::lab(&model::xy_to_xyz(model::D65, 100.0));
    let mut lab_state: Option<Vec3> = Some(d65_lab);
    let live = Arc::new(Mutex::new(autobright::Live::default()));
    let mut bright: Option<Brightness> = cfg.auto_brightness.then(|| Brightness::start(live.clone()));
    let mut last = Instant::now();
    let mut last_status = Instant::now() - Duration::from_secs(10);
    let mut status_failed = false;

    while !STOP.load(Ordering::SeqCst) {
        std::thread::sleep(TICK);
        let dt = last.elapsed().as_secs_f64();
        last = Instant::now();

        let m = config::mtime();
        if m != cfg_mtime {
            cfg_mtime = m;
            let new = config::load();
            if new.auto_brightness && bright.is_none() {
                bright = Some(Brightness::start(live.clone()));
            } else if !new.auto_brightness {
                if let Some(b) = bright.take() {
                    b.stop();
                }
            }
            cfg = new;
            gnome_sync(&cfg);
        }
        while let Ok(s) = rx.try_recv() {
            sample = Some(s);
            // The thread ends on its own when there is no panel to control.
            if bright.as_ref().is_some_and(|b| b.tx.send(s.lux as f32).is_err()) {
                if let Some(b) = bright.take() {
                    b.stop();
                }
            }
        }

        let target = match (&sample, cfg.true_tone) {
            (Some(s), true) => model::ambient_lab(&locus, s.lux, s.xy, s.cct),
            _ => d65_lab,
        };
        let lab_now = smooth_lab(&mut lab_state, target, dt, cfg.fade_s);
        let white = model::lab_to_xyz(&model::adapted_lab(&lab_now, cfg.strength));
        let tt = model::white_gains(&white);
        let nf = night::factor(&cfg, night::local_minutes());
        let ng = night_gains(&cfg, nf);
        let mut gains = [tt[0] * ng[0], tt[1] * ng[1], tt[2] * ng[2]];
        let gm = gains.iter().cloned().fold(f64::MIN, f64::max);
        gains = gains.map(|v| v / gm);

        if out.is_none() && out_retry <= Instant::now() {
            match Output::open() {
                Ok(o) => {
                    out = Some(o);
                    last_err = None;
                }
                Err(e) => {
                    last_err = Some(format!("{e:#}"));
                    out_retry = Instant::now() + Duration::from_secs(5);
                }
            }
        }
        if let Some(o) = out.as_mut() {
            let mut res = Ok(());
            if o.checked.elapsed() >= CHECK_EVERY {
                res = o.check();
            }
            if res.is_ok() {
                res = o.apply(gains);
            }
            if let Err(e) = res {
                last_err = Some(format!("{e:#}"));
                out = None;
                out_retry = Instant::now() + Duration::from_secs(2);
            }
        }

        if last_status.elapsed() >= Duration::from_secs(1) {
            last_status = Instant::now();
            let s = sample.unwrap_or(Sample { lux: 0.0, xy: [0.0; 2], cct: 0.0, at: Instant::now() });
            let amb = model::ambient_xy(&locus, s.lux, s.xy, s.cct);
            let mut st = Status {
                pid: std::process::id(),
                unix_time: SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
                sensor: sample.is_some(),
                lux: s.lux,
                sensor_cct: s.cct,
                sensor_xy: s.xy,
                ambient_cct: model::mccamy(amb),
                white_cct: model::mccamy(model::xyz_to_xy(&white)),
                gains,
                night_factor: nf,
                brightness: live.lock().unwrap().linear.map(f64::from),
                error: last_err.clone(),
            };
            if sample.is_none() {
                st.ambient_cct = 0.0;
            }
            match status::write(&st) {
                Ok(()) => status_failed = false,
                Err(e) if !status_failed => {
                    eprintln!("t2-truetone: could not write {}: {e}", status::path().display());
                    status_failed = true;
                }
                Err(_) => {}
            }
        }
    }
    if let Some(b) = bright.take() {
        b.stop();
    }
    if let Some(o) = out.as_mut() {
        o.restore();
    }
    if let Err(e) = std::fs::remove_file(status::path()) {
        if e.kind() != std::io::ErrorKind::NotFound {
            eprintln!("t2-truetone: could not remove {}: {e}", status::path().display());
        }
    }
    Ok(())
}
