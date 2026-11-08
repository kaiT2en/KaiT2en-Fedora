use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

use signal_hook::consts::signal::{SIGINT, SIGTERM};

use crate::{
    actuators::{cpu_floor, CpuCap, DGPU_STEPS, DGpuCap, ForcedIdle, IGpuCap},
    error::{Result, ZonePowerError},
    plimit::PLimits,
    sysfs::{find_t2smc, read_value, write_value},
};

const TICK: Duration = Duration::from_millis(250);
const REDISCOVER: Duration = Duration::from_secs(10);

/// Applies the SMC power limit steps like macOS does.
/// With debug every step change is logged.
pub fn run(debug: bool) -> Result<()> {
    let stop = Arc::new(AtomicBool::new(false));
    for signal in [SIGTERM, SIGINT] {
        signal_hook::flag::register(signal, stop.clone()).map_err(|source| ZonePowerError::Io {
            path: PathBuf::from("<signal-hook>"),
            source,
        })?;
    }

    let t2smc = find_t2smc()?;

    // macOS reports the CPU floor on every PLimit version
    let floor_path = t2smc.join("smc_cpu_floor");
    if let (true, Some((turbo, floor))) = (floor_path.exists(), cpu_floor()) {
        report(write_value(&floor_path, format!("{turbo} {floor}")));
    }

    // Only the data version 3 layout is known
    let version: u8 = read_value(&t2smc.join("smc_plimit_version")).unwrap_or(0);
    if version != 3 {
        eprintln!("SMC power limit data version {version} is not supported, nothing is applied");
        while !stop.load(Ordering::Relaxed) {
            thread::sleep(TICK);
        }
        return Ok(());
    }

    let cpu = CpuCap::discover()?;
    let idle = ForcedIdle::discover()?;
    let igpu = IGpuCap::discover()?;
    if let Some(igpu) = &igpu {
        // macOS tells the SMC the iGPU step count as well
        report(write_value(&t2smc.join("smc_igpu_steps"), igpu.steps()));
    }
    let mut dgpu = DGpuCap::discover()?;
    set_dgpu_steps(&t2smc, &dgpu);
    let mut last_discovery = Instant::now();
    let mut last = None;
    let mut first_error = true;
    report_missing(&cpu, &idle, &igpu, &dgpu);

    while !stop.load(Ordering::Relaxed) {
        if dgpu.is_none() && last_discovery.elapsed() >= REDISCOVER {
            dgpu = DGpuCap::discover()?;
            set_dgpu_steps(&t2smc, &dgpu);
            last_discovery = Instant::now();
        }

        match PLimits::read(&t2smc) {
            Ok(limits) => {
                if last != Some(limits) {
                    if debug {
                        eprintln!(
                            "plimit cpu {} igpu {} mem {} idle {} dgpu {}",
                            format_step(limits.cpu.step, limits.cpu.hard),
                            format_step(limits.igpu.step, limits.igpu.hard),
                            format_step(limits.memory.step, limits.memory.hard),
                            format_step(limits.idle.step, limits.idle.hard),
                            format_step(limits.dgpu.step, limits.dgpu.hard),
                        );
                    }
                    last = Some(limits);
                }
                apply(&limits, cpu.as_ref(), idle.as_ref(), igpu.as_ref(), dgpu.as_ref());
            }
            // Logged once, the read fails every tick while it lasts
            Err(error) => {
                if last.take().is_some() || first_error {
                    eprintln!("{error}");
                }
                first_error = false;
            }
        }
        thread::sleep(TICK);
    }

    restore(cpu.as_ref(), idle.as_ref(), igpu.as_ref(), dgpu.as_ref());
    Ok(())
}

fn apply(
    limits: &PLimits,
    cpu: Option<&CpuCap>,
    idle: Option<&ForcedIdle>,
    igpu: Option<&IGpuCap>,
    dgpu: Option<&DGpuCap>,
) {
    if let Some(cpu) = cpu {
        report(cpu.apply(cpu.target_khz(limits.cpu.step)));
    }
    if let Some(idle) = idle {
        report(idle.apply(idle.target_percent(limits.idle.step)));
    }
    if let Some(igpu) = igpu {
        report(igpu.apply(igpu.target_mhz(limits.igpu.step)));
    }
    if let Some(dgpu) = dgpu {
        report(dgpu.apply(dgpu.target_w(limits.dgpu.step)));
    }
}

fn restore(cpu: Option<&CpuCap>, idle: Option<&ForcedIdle>, igpu: Option<&IGpuCap>, dgpu: Option<&DGpuCap>) {
    if let Some(cpu) = cpu {
        report(cpu.restore());
    }
    if let Some(idle) = idle {
        report(idle.restore());
    }
    if let Some(igpu) = igpu {
        report(igpu.restore());
    }
    if let Some(dgpu) = dgpu {
        report(dgpu.restore());
    }
}

/// macOS tells the SMC the dGPU step count only when it drives a dGPU
fn set_dgpu_steps(t2smc: &std::path::Path, dgpu: &Option<DGpuCap>) {
    if dgpu.is_some() {
        report(write_value(&t2smc.join("smc_dgpu_steps"), DGPU_STEPS));
    }
}

fn report(result: Result<()>) {
    if let Err(error) = result {
        eprintln!("{error}");
    }
}

fn report_missing(cpu: &Option<CpuCap>, idle: &Option<ForcedIdle>, igpu: &Option<IGpuCap>, dgpu: &Option<DGpuCap>) {
    if cpu.is_none() {
        eprintln!("no cpufreq policy, CPU step is not applied");
    }
    if idle.is_none() {
        eprintln!("intel_powerclamp is missing, idle step is not applied");
    }
    if igpu.is_none() {
        eprintln!("no i915 RP0/RPn in sysfs, iGPU step is not applied");
    }
    if dgpu.is_none() {
        eprintln!("no supported dGPU found yet, dGPU step is not applied");
    }
}

fn format_step(step: u8, hard: bool) -> String {
    if hard { format!("{step}!") } else { step.to_string() }
}
