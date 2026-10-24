// SPDX-License-Identifier: GPL-3.0-or-later

mod apps;
mod backlight;
mod config;
mod display;
mod focus;
mod gesture;
mod haptic;
mod kbdlight;
mod keyboard;
mod keys;
mod levels;
mod mpris;
mod policy;
mod renderer;
mod touchid;
mod usb;

use std::{
    collections::HashMap,
    fs::{File, OpenOptions},
    io::Read,
    os::{
        fd::{AsFd, AsRawFd, OwnedFd},
        unix::{fs::OpenOptionsExt, net::UnixStream},
    },
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use input::{
    Device as InputDevice, DeviceCapability, Libinput, LibinputInterface, SendEventsMode,
    event::{
        DeviceEvent, Event, EventTrait,
        keyboard::{KeyState, KeyboardEvent, KeyboardEventTrait},
        touch::{TouchEvent, TouchEventPosition, TouchEventSlot},
    },
};
use input_linux::Key;
use libc::{O_ACCMODE, O_RDONLY, O_RDWR, O_WRONLY, pollfd};

use apps::AppProfile;
use backlight::Backlight;
use config::{Config, state_path};
use display::Display;
use gesture::{Swipe, SwipeKind};
use haptic::Haptic;
use keyboard::VirtualKeyboard;
use keys::{CustomAction, CustomKey};
use levels::{Level, Reader as LevelReader};
use mpris::{Mpris, Track};
use policy::{Mode, PersistentState, TimeoutLearner};
use renderer::{Action, Canvas, Layout, LevelKind, TrackLayout};
use touchid::TouchIdState;

/// How long the esc key stays lit after it was used on the dark bar, so even
/// a quick tap is visible.
const ESC_LINGER_MS: u64 = 150;
/// Swipe feedback stays a little after the fingers lift to show the result.
const LEVEL_LINGER_MS: u64 = 700;
const LEVEL_FADE_MS: u64 = 350;
const LEVEL_FRAME_MS: u64 = 33;
const ACTIVITY_FADE_MS: u64 = 700;
const ACTIVITY_FRAME_MS: u64 = 33;
/// The desktop applies a volume or brightness key asynchronously, so the
/// value is read back repeatedly for a short while after each step.
const LEVEL_POLL_MS: u64 = 50;
const LEVEL_POLL_WINDOW_MS: u64 = 600;
/// The level feedback crossfades to the other side of the fingers once
/// they pass the middle of the bar by this much.
const LEVEL_SIDE_HYSTERESIS_PX: f64 = 60.0;
const LEVEL_SIDE_FADE_MS: u64 = 700;
/// Switching rows move far enough to read as a swipe across the bar.
const LAYER_SLIDE_PX: f32 = 190.0;
/// Switching rows take 195 ms.
const LAYER_FADE_MS: u64 = 195;
const LAYER_FRAME_MS: u64 = 20;
/// Track overlay timeline: fade in, hold, fade out.
const TRACK_FADE_MS: u64 = 700;
const TRACK_HOLD_MS: u64 = 5_000;
/// About 30 fps while the overlay fades.
const TRACK_FRAME_MS: u64 = 33;
/// Holding previous/next this long from touch-down starts seeking.
/// The Touch ID arrow swings towards the sensor once per period, at about
/// 30 fps.
const ARROW_PERIOD_MS: u64 = 1_500;
const ARROW_FRAME_MS: u64 = 33;
const MEDIA_HOLD_MS: u64 = 450;
const SEEK_INTERVAL_MS: u64 = 250;
/// Seek steps grow after this long, so long distances stay quick.
const SEEK_FAST_AFTER_MS: u64 = 1_500;
const SEEK_STEP: Duration = Duration::from_secs(5);
const SEEK_FAST_STEP: Duration = Duration::from_secs(15);

static STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn stop(_: i32) {
    STOP.store(true, Ordering::Relaxed);
}

struct Interface;

impl LibinputInterface for Interface {
    fn open_restricted(&mut self, path: &Path, flags: i32) -> std::result::Result<OwnedFd, i32> {
        let mode = flags & O_ACCMODE;
        OpenOptions::new()
            .custom_flags(flags)
            .read(mode == O_RDONLY || mode == O_RDWR)
            .write(mode == O_WRONLY || mode == O_RDWR)
            .open(path)
            .map(Into::into)
            .map_err(|error| error.raw_os_error().unwrap_or(libc::EIO))
    }

    fn close_restricted(&mut self, fd: OwnedFd) {
        drop(File::from(fd));
    }
}

#[derive(Clone, Copy, Debug)]
enum Contact {
    Wake,
    Button(usize),
    TouchIdEsc(usize),
    Cancelled,
    /// First finger, waiting whether a second one turns this into a swipe.
    Pending,
    Swipe,
    /// Esc held on the dark bar. The bar stays dark.
    DarkEsc,
    /// Previous/next held: a tap on release, seeking once held long enough.
    MediaHold(usize),
    /// A key of the track overlay on the dark bar; `true` is next.
    TrackKey(bool),
}

/// Crossfade from the previous row to the current one.
struct LayerFade {
    /// The frame that was on screen when the switch started.
    from: Vec<u32>,
    start_ms: u64,
    next_ms: u64,
    /// `1` when the new row comes in from the right, `-1` from the left.
    direction: i32,
    /// Everything left of this column (esc) stays in place.
    clip_left: usize,
    /// A swipe switched the row: a firm click confirms it once the new row
    /// has settled.
    confirm: bool,
}

/// "Artist – Title" shown on the dark bar when a new track starts.
struct TrackToast {
    track: Track,
    /// Origin of the fade in / hold / fade out timeline.
    start_ms: u64,
    next_ms: Option<u64>,
    layout: TrackLayout,
    pressed: Option<bool>,
}

struct BacklightFade {
    start_ms: u64,
    next_ms: u64,
    from: u32,
    to: u32,
}

struct EscapeFade {
    start_ms: u64,
    next_ms: u64,
    from: f32,
    to: f32,
}

/// Previous/next under a finger. Released early it skips the track, held
/// past `MEDIA_HOLD_MS` it seeks in steps until released.
struct MediaHold {
    forward: bool,
    next_ms: u64,
    seeking_since: Option<u64>,
    player: Option<String>,
}

/// A first touch held back for `two_finger_window_ms`. On the dark bar it
/// becomes a wake, on the lit bar a key press, unless a second finger joins.
#[derive(Clone, Copy, Debug)]
struct PendingTouch {
    slot: u32,
    deadline_ms: u64,
    dark: bool,
    button: Option<usize>,
}

struct Runtime {
    config: Config,
    state: PersistentState,
    state_path: std::path::PathBuf,
    learner: TimeoutLearner,
    started: Instant,
    visible: bool,
    touch_id: TouchIdState,
    deadline_ms: Option<u64>,
    fn_down_ms: Option<u64>,
    fn_toggled: bool,
    contacts: HashMap<u32, Contact>,
    positions: HashMap<u32, f64>,
    pending: Option<PendingTouch>,
    swipe: Option<Swipe>,
    /// The dark bar currently lights only the esc key.
    overlay_shown: bool,
    overlay_hide_ms: Option<u64>,
    /// Level feedback while a dark-bar swipe runs.
    level_kind: Option<LevelKind>,
    level: Level,
    /// Leftmost and rightmost finger the feedback was last placed against.
    level_fingers: (f64, f64),
    /// How much of the feedback shows right of the fingers, moving towards
    /// `level_right_target` for the crossfade.
    level_right: f32,
    level_right_target: bool,
    level_right_ms: u64,
    level_poll_ms: Option<u64>,
    level_poll_until_ms: u64,
    /// The level feedback changed and is drawn once the pending input is
    /// processed, so a burst of touch events costs one frame.
    level_dirty: bool,
    level_fade_start_ms: Option<u64>,
    level_frame_ms: Option<u64>,
    /// This swipe has sent a key, so a level at 0 or 100 was reached by it.
    level_stepped: bool,
    /// The stop the level sits at was already felt as a firm click.
    level_stop_felt: bool,
    activity_deadline_ms: Option<u64>,
    activity_dimmed: bool,
    activity_restore_brightness: Option<u32>,
    activity_max_brightness: Option<u32>,
    activity_esc_shown: bool,
    activity_current_brightness: Option<u32>,
    backlight_fade: Option<BacklightFade>,
    escape_opacity: f32,
    escape_fade: Option<EscapeFade>,
    /// Last values seen per kind, shown until a fresh read arrives.
    level_cache: [Level; 2],
    level_reader: LevelReader,
    level_wake: UnixStream,
    key_level: u32,
    media_hold: Option<MediaHold>,
    mpris: Mpris,
    toast: Option<TrackToast>,
    layer_fade: Option<LayerFade>,
    /// The blended frame while a crossfade runs.
    blended: Vec<u32>,
    touches_armed: bool,
    active_button: Option<usize>,
    layout: Layout,
    canvas: Canvas,
    display: Display,
    backlight: Backlight,
    keyboard: VirtualKeyboard,
    haptic: Haptic,
    physical_escape: bool,
    /// Personal keys from keys.toml, shown in the special row.
    custom_keys: Vec<CustomKey>,
    custom_labels: Vec<&'static str>,
    /// App profiles from apps.toml and the one of the focused window.
    app_profiles: Vec<AppProfile>,
    app_active: Option<usize>,
    /// The row to return to when the focus leaves a profiled app.
    mode_before_app: Mode,
    animation_start_ms: u64,
    next_animation_ms: Option<u64>,
}

impl Runtime {
    fn new(config: Config, mut state: PersistentState) -> Result<Self> {
        let mut display = Display::open()?;
        let (width, height) = display.dimensions();
        let canvas = Canvas::new(width, height, config.key_color())?;
        display.present(&canvas.pixels, width, height)?;
        let mut backlight = Backlight::open()?;
        backlight.set(0)?;
        let physical_escape = has_physical_escape();
        let custom_keys = keys::load();
        let custom_labels: Vec<&'static str> = custom_keys.iter().map(|key| key.label).collect();
        let app_profiles = apps::load();
        // The virtual keyboard declares its keys once, so it covers the keys
        // of every profile up front.
        let custom_codes: Vec<Key> = custom_keys
            .iter()
            .chain(app_profiles.iter().flat_map(|profile| &profile.keys))
            .flat_map(|key| key.keys().iter().copied())
            .collect();
        // No window is known yet. The focus listener brings the app row
        // back if the focused app still has a profile.
        if state.mode == Mode::App {
            state.mode = Mode::Media;
        }
        let layout = Layout::with_custom(state.mode, width, physical_escape, &custom_labels);
        let haptic = Haptic::open(config.haptic_feedback, config.haptic_touch_up);
        let (level_reader, level_wake) = LevelReader::spawn()?;
        level_wake.set_nonblocking(true)?;
        let initial_keyboard_brightness = config
            .activity_backlight
            .then(|| kbdlight::brightness().ok())
            .flatten();
        let activity_restore_brightness =
            initial_keyboard_brightness.filter(|brightness| *brightness > 0);
        let activity_dimmed = config.activity_backlight && initial_keyboard_brightness == Some(0);
        let activity_deadline_ms = config
            .activity_backlight
            .then_some(config.activity_timeout_ms);
        Ok(Self {
            config,
            state,
            state_path: state_path(),
            learner: TimeoutLearner::new(),
            started: Instant::now(),
            visible: false,
            touch_id: TouchIdState::Idle,
            deadline_ms: None,
            fn_down_ms: None,
            fn_toggled: false,
            contacts: HashMap::new(),
            positions: HashMap::new(),
            pending: None,
            swipe: None,
            overlay_shown: false,
            overlay_hide_ms: None,
            level_kind: None,
            level: Level::default(),
            level_fingers: (0.0, 0.0),
            level_right: 1.0,
            level_right_target: true,
            level_right_ms: 0,
            level_poll_ms: None,
            level_poll_until_ms: 0,
            level_dirty: false,
            level_fade_start_ms: None,
            level_frame_ms: None,
            level_stepped: false,
            level_stop_felt: false,
            activity_deadline_ms,
            activity_dimmed,
            activity_restore_brightness,
            activity_max_brightness: None,
            activity_esc_shown: false,
            activity_current_brightness: initial_keyboard_brightness,
            backlight_fade: None,
            escape_opacity: 0.0,
            escape_fade: None,
            level_cache: [Level::default(); 2],
            level_reader,
            level_wake,
            key_level: 100,
            media_hold: None,
            mpris: Mpris::default(),
            toast: None,
            layer_fade: None,
            blended: Vec::new(),
            touches_armed: true,
            active_button: None,
            layout,
            canvas,
            display,
            backlight,
            keyboard: VirtualKeyboard::open(&custom_codes)?,
            haptic,
            physical_escape,
            custom_keys,
            custom_labels,
            app_profiles,
            app_active: None,
            mode_before_app: Mode::Media,
            animation_start_ms: 0,
            next_animation_ms: None,
        })
    }

    fn now_ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }

    fn save(&self) {
        if let Err(error) = self.state.save(&self.state_path) {
            eprintln!("kait2en-touchbar: could not save state: {error:#}");
        }
    }

    fn present_keys(&mut self) -> Result<()> {
        let labels: &[&'static str] = match self.state.mode {
            Mode::Special => &self.custom_labels,
            Mode::App => self
                .app_active
                .map_or(&[], |index| &self.app_profiles[index].labels),
            Mode::Media | Mode::Function => &[],
        };
        self.layout =
            Layout::with_custom(self.state.mode, self.canvas.width, self.physical_escape, labels);
        self.canvas.keys(&self.layout, self.active_button);
        if self.layer_fade.is_some() {
            return self.present_layer_fade(self.now_ms());
        }
        self.display
            .present(&self.canvas.pixels, self.canvas.width, self.canvas.height)
    }

    /// Shows the crossfade between the previous row and the freshly drawn
    /// current one for `now`, and ends it once the current row is complete.
    fn present_layer_fade(&mut self, now: u64) -> Result<()> {
        let Some(fade) = self.layer_fade.as_mut() else {
            return Ok(());
        };
        let elapsed = now.saturating_sub(fade.start_ms);
        if elapsed >= LAYER_FADE_MS {
            if fade.confirm {
                self.haptic.firm();
            }
            self.layer_fade = None;
            return self.display.present(
                &self.canvas.pixels,
                self.canvas.width,
                self.canvas.height,
            );
        }
        fade.next_ms = now + LAYER_FRAME_MS;
        let progress = elapsed as f32 / LAYER_FADE_MS as f32;
        self.blended.resize(self.canvas.pixels.len(), 0);
        compose_slide(
            &fade.from,
            &self.canvas.pixels,
            &mut self.blended,
            usize::from(self.canvas.width),
            fade.clip_left,
            fade.direction,
            progress,
        );
        self.display
            .present(&self.blended, self.canvas.width, self.canvas.height)
    }

    fn wake(&mut self, quarantine_touch: bool) -> Result<()> {
        if self.touch_id != TouchIdState::Idle {
            return Ok(());
        }
        let now = self.now_ms();
        self.refresh_key_level();
        if !self.visible {
            self.learner.wake(now, &self.config);
        }
        self.visible = true;
        self.overlay_shown = false;
        self.activity_esc_shown = false;
        self.escape_fade = None;
        self.escape_opacity = 0.0;
        self.overlay_hide_ms = None;
        self.level_kind = None;
        self.level_poll_ms = None;
        self.toast = None;
        self.layer_fade = None;
        if quarantine_touch {
            self.touches_armed = false;
        }
        self.deadline_ms = Some(now + self.state.learned(self.state.mode).timeout_ms);
        self.present_keys()?;
        self.backlight.set(self.config.active_brightness)?;
        Ok(())
    }

    fn go_dark(&mut self, learned_off: bool) -> Result<()> {
        self.go_dark_inner(learned_off, false)
    }

    fn go_dark_to_activity_esc(&mut self, learned_off: bool) -> Result<()> {
        self.go_dark_inner(learned_off, true)
    }

    fn go_dark_inner(&mut self, learned_off: bool, preserve_activity_esc: bool) -> Result<()> {
        let now = self.now_ms();
        let escape = (preserve_activity_esc
            && self.config.activity_backlight
            && self.touch_id == TouchIdState::Idle
            && self
                .activity_deadline_ms
                .is_some_and(|deadline| now < deadline))
        .then(|| {
            self.layout
                .buttons
                .iter()
                .position(|button| button.action == Action::Key(Key::Esc))
        })
        .flatten();
        self.keyboard.release()?;
        self.active_button = None;
        self.clear_touches();
        self.touches_armed = true;
        self.deadline_ms = None;
        if self.visible && learned_off {
            self.learner
                .auto_off(self.now_ms(), self.state.mode, &self.config);
        }
        self.visible = false;
        self.overlay_shown = escape.is_some();
        self.activity_esc_shown = escape.is_some();
        self.escape_fade = None;
        self.escape_opacity = if escape.is_some() { 1.0 } else { 0.0 };
        self.overlay_hide_ms = None;
        self.level_kind = None;
        self.level_poll_ms = None;
        self.toast = None;
        self.layer_fade = None;
        if let Some(index) = escape {
            self.canvas.single_key(&self.layout, index, false, 100);
        } else {
            self.canvas.clear();
        }
        self.display
            .present(&self.canvas.pixels, self.canvas.width, self.canvas.height)?;
        if escape.is_none() {
            self.backlight.set(0)?;
        }
        Ok(())
    }

    fn clear_touches(&mut self) {
        self.contacts.clear();
        self.positions.clear();
        self.pending = None;
        self.swipe = None;
        self.media_hold = None;
    }

    fn touch_down(&mut self, slot: u32, x: f64, y: f64) -> Result<()> {
        let now = self.now_ms();
        self.user_activity(now, false)?;
        self.resolve_pending(now)?;
        self.positions.insert(slot, x);
        if self.touch_id == TouchIdState::Idle
            && self
                .swipe
                .is_some_and(|swipe| swipe.kind != SwipeKind::Mode)
        {
            return self.join_dark_swipe(slot);
        }
        if self.touch_id != TouchIdState::Idle {
            let esc_index = self
                .layout
                .buttons
                .iter()
                .position(|button| button.action == Action::Key(Key::Esc));
            if let Some(index) = esc_index.filter(|&index| {
                self.layout.hit(x, y, self.canvas.height) == Some(index)
            }) {
                self.active_button = Some(index);
                self.contacts.insert(slot, Contact::TouchIdEsc(index));
                self.render_touch_id(now)?;
            } else {
                self.contacts.insert(slot, Contact::Cancelled);
            }
            return Ok(());
        }
        if self.swipe.is_some() {
            self.contacts.insert(slot, Contact::Cancelled);
            return Ok(());
        }
        if let Some(pending) = self.pending.take() {
            return self.start_swipe(pending, slot);
        }
        if !self.contacts.is_empty()
            || (self.visible && (!self.touches_armed || self.active_button.is_some()))
        {
            self.contacts.insert(slot, Contact::Cancelled);
            return Ok(());
        }
        if !self.visible
            && let Some(toast) = self.toast.as_mut()
        {
            let inside = |(left, right): (u16, u16)| x >= f64::from(left) && x < f64::from(right);
            let forward = if inside(toast.layout.next) {
                Some(true)
            } else if inside(toast.layout.prev) {
                Some(false)
            } else {
                None
            };
            if let Some(forward) = forward {
                toast.pressed = Some(forward);
                self.contacts.insert(slot, Contact::TrackKey(forward));
                return self.track_frame(now);
            }
            // Anything else on the bar takes over from the overlay.
            self.toast = None;
        }
        let button = self.layout.hit(x, y, self.canvas.height);
        // The esc slot never moves, so it stays usable while the bar is dark.
        let button = if self.visible {
            button
        } else {
            button.filter(|&index| self.is_esc(index))
        };
        self.pending = Some(PendingTouch {
            slot,
            deadline_ms: now + self.config.two_finger_window_ms,
            dark: !self.visible,
            button,
        });
        self.contacts.insert(slot, Contact::Pending);
        if let Some(index) = button {
            if self.visible {
                // Highlight at once. Only the key event waits for the window.
                self.active_button = button;
                self.present_keys()?;
            } else {
                self.show_dark_esc(index)?;
            }
        }
        Ok(())
    }

    fn start_swipe(&mut self, pending: PendingTouch, slot: u32) -> Result<()> {
        let (kind, step) = if pending.dark {
            (SwipeKind::Volume, self.config.volume_swipe_step_px)
        } else {
            (SwipeKind::Mode, self.config.mode_swipe_px)
        };
        self.contacts.insert(pending.slot, Contact::Swipe);
        self.contacts.insert(slot, Contact::Swipe);
        self.swipe = Some(Swipe::new(kind, self.swipe_centroid(), f64::from(step)));
        if pending.dark {
            return self.begin_level(LevelKind::Volume);
        }
        if pending.button.is_some() {
            self.active_button = None;
            self.present_keys()?;
        }
        Ok(())
    }

    /// Three fingers on the dark bar always mean brightness.
    fn join_dark_swipe(&mut self, slot: u32) -> Result<()> {
        if self
            .swipe
            .is_some_and(|swipe| swipe.kind == SwipeKind::Brightness)
        {
            self.contacts.insert(slot, Contact::Cancelled);
            return Ok(());
        }
        self.contacts.insert(slot, Contact::Swipe);
        self.swipe = Some(Swipe::new(
            SwipeKind::Brightness,
            self.swipe_centroid(),
            f64::from(self.config.brightness_swipe_step_px),
        ));
        self.begin_level(LevelKind::Brightness)
    }

    fn swipe_span(&self) -> (f64, f64) {
        let xs = self
            .contacts
            .iter()
            .filter(|(_, contact)| matches!(contact, Contact::Swipe))
            .filter_map(|(slot, _)| self.positions.get(slot).copied());
        xs.fold((f64::MAX, f64::MIN), |(first, last), x| {
            (first.min(x), last.max(x))
        })
    }

    fn swipe_centroid(&self) -> f64 {
        let xs: Vec<f64> = self
            .contacts
            .iter()
            .filter(|(_, contact)| matches!(contact, Contact::Swipe))
            .filter_map(|(slot, _)| self.positions.get(slot).copied())
            .collect();
        xs.iter().sum::<f64>() / xs.len().max(1) as f64
    }

    /// Turns a first touch whose window has expired into its single-finger
    /// meaning: a wake on the dark bar, a held key on the lit bar.
    fn resolve_pending(&mut self, now: u64) -> Result<()> {
        let Some(pending) = self.pending.filter(|pending| now >= pending.deadline_ms) else {
            return Ok(());
        };
        self.pending = None;
        if pending.dark && pending.button.is_some() {
            self.contacts.insert(pending.slot, Contact::DarkEsc);
            self.keyboard.press(Key::Esc)?;
            self.haptic.key_press();
            return Ok(());
        }
        if pending.dark {
            self.contacts.insert(pending.slot, Contact::Wake);
            return self.wake(true);
        }
        match pending.button {
            Some(index) if self.media_direction(index).is_some() => {
                // Stays highlighted. The action is decided on release or hold.
                let touched = pending.deadline_ms - self.config.two_finger_window_ms;
                self.contacts
                    .insert(pending.slot, Contact::MediaHold(index));
                self.media_hold = Some(MediaHold {
                    forward: self.media_direction(index) == Some(true),
                    next_ms: touched + MEDIA_HOLD_MS,
                    seeking_since: None,
                    player: None,
                });
                self.deadline_ms = Some(now + self.state.learned(self.state.mode).timeout_ms);
                Ok(())
            }
            Some(index) => {
                self.contacts.insert(pending.slot, Contact::Button(index));
                self.press_button(index, now)
            }
            None => {
                self.contacts.insert(pending.slot, Contact::Cancelled);
                Ok(())
            }
        }
    }

    fn track_changed(&mut self, track: Track) -> Result<()> {
        let now = self.now_ms();
        if let Some(toast) = self.toast.as_mut() {
            toast.track = track;
            toast.start_ms = now.saturating_sub(TRACK_FADE_MS);
            return self.track_frame(now);
        }
        // Only on a quiet dark bar: never over keys, gestures or Touch ID.
        // The esc key from activity stays and is drawn alongside.
        if self.visible
            || (self.overlay_shown && !self.activity_esc_shown)
            || self.touch_id != TouchIdState::Idle
            || !self.contacts.is_empty()
        {
            return Ok(());
        }
        self.refresh_key_level();
        self.toast = Some(TrackToast {
            track,
            start_ms: now,
            next_ms: None,
            layout: self.track_keys(),
            pressed: None,
        });
        self.track_frame(now)
    }

    /// Draws the track overlay for `now` and schedules the next frame: about
    /// 30 fps while fading, nothing while it holds or a key is pressed.
    fn track_frame(&mut self, now: u64) -> Result<()> {
        let Some(toast) = self.toast.as_mut() else {
            return Ok(());
        };
        let elapsed = now.saturating_sub(toast.start_ms);
        let fade_out = TRACK_FADE_MS + TRACK_HOLD_MS;
        let (opacity, next) = if toast.pressed.is_some() {
            (1.0, None)
        } else if elapsed < TRACK_FADE_MS {
            (
                elapsed as f32 / TRACK_FADE_MS as f32,
                Some(now + TRACK_FRAME_MS),
            )
        } else if elapsed < fade_out {
            (1.0, Some(toast.start_ms + fade_out))
        } else if elapsed < fade_out + TRACK_FADE_MS {
            (
                1.0 - (elapsed - fade_out) as f32 / TRACK_FADE_MS as f32,
                Some(now + TRACK_FRAME_MS),
            )
        } else {
            self.toast = None;
            if self.activity_esc_shown {
                return self.render_activity_esc();
            }
            return self.hide_overlay();
        };
        // Smoothstep, so the fades start and end softly.
        let eased = opacity * opacity * (3.0 - 2.0 * opacity);
        toast.next_ms = next;
        let (track, pressed, keys) = (toast.track.clone(), toast.pressed, toast.layout);
        let esc = self
            .activity_esc_shown
            .then(|| {
                self.layout
                    .buttons
                    .iter()
                    .find(|button| button.action == Action::Key(Key::Esc))
                    .copied()
            })
            .flatten()
            .map(|button| (button, (self.escape_opacity * 100.0).round() as u32));
        self.canvas.track(
            &track.artist,
            &track.title,
            (eased * 100.0).round() as u32,
            pressed,
            keys,
            esc,
        );
        self.present_overlay()
    }

    /// The overlay's previous key takes the first key slot after esc. Next
    /// mirrors it on the right, as if an invisible esc sat there too. The
    /// keys stay in fixed places, esc keeps working and the text centers on
    /// the bar.
    fn track_keys(&self) -> TrackLayout {
        // Taken from the media row, whose keys fill the whole bar.
        let media = Layout::new(Mode::Media, self.canvas.width, self.physical_escape);
        let first = &media.buttons[usize::from(!self.physical_escape)];
        let width = self.canvas.width;
        TrackLayout {
            prev: (first.left, first.right),
            next: (width - first.right, width - first.left),
        }
    }

    fn show_dark_esc(&mut self, index: usize) -> Result<()> {
        self.refresh_key_level();
        self.level_kind = None;
        self.activity_esc_shown = self.config.activity_backlight;
        self.escape_fade = None;
        self.escape_opacity = 1.0;
        self.level_fade_start_ms = None;
        self.level_frame_ms = None;
        self.toast = None;
        self.overlay_hide_ms = None;
        self.canvas.single_key(&self.layout, index, true, 100);
        self.present_overlay()
    }

    fn begin_level(&mut self, kind: LevelKind) -> Result<()> {
        self.refresh_key_level();
        self.toast = None;
        self.level_kind = Some(kind);
        self.activity_esc_shown = false;
        self.escape_fade = None;
        self.escape_opacity = 0.0;
        self.level_fade_start_ms = None;
        self.level_frame_ms = None;
        self.level = self.level_cache[cache_slot(kind)];
        self.level_stepped = false;
        self.level_stop_felt = false;
        self.level_reader.request(kind);
        self.level_fingers = self.swipe_span();
        self.update_level_side(self.now_ms(), true);
        self.overlay_hide_ms = None;
        self.level_dirty = true;
        Ok(())
    }

    /// Places the feedback on the side of the fingers with more room. A
    /// change crossfades unless `snap` is set.
    fn update_level_side(&mut self, now: u64, snap: bool) {
        let centre = (self.level_fingers.0 + self.level_fingers.1) / 2.0;
        let middle = f64::from(self.canvas.width) / 2.0;
        // Progress so far counts towards the old target.
        self.advance_level_side(now);
        let margin = if snap { 0.0 } else { LEVEL_SIDE_HYSTERESIS_PX };
        if centre < middle - margin {
            self.level_right_target = true;
        } else if centre >= middle + margin {
            self.level_right_target = false;
        }
        if snap {
            self.level_right = f32::from(u8::from(self.level_right_target));
        }
        if self.level_right != f32::from(u8::from(self.level_right_target)) {
            self.level_frame_ms.get_or_insert(now + LEVEL_FRAME_MS);
        }
    }

    /// Moves the side crossfade on to `now`. Returns whether it still runs.
    fn advance_level_side(&mut self, now: u64) -> bool {
        let target = f32::from(u8::from(self.level_right_target));
        let step = now.saturating_sub(self.level_right_ms) as f32 / LEVEL_SIDE_FADE_MS as f32;
        self.level_right_ms = now;
        self.level_right = if self.level_right < target {
            (self.level_right + step).min(target)
        } else {
            (self.level_right - step).max(target)
        };
        self.level_right != target
    }

    /// Draws the level feedback if anything changed since the last frame.
    fn flush_level(&mut self) -> Result<()> {
        if !std::mem::take(&mut self.level_dirty) {
            return Ok(());
        }
        let Some(kind) = self.level_kind.filter(|_| !self.visible) else {
            return Ok(());
        };
        let now = self.now_ms();
        self.advance_level_side(now);
        let right = self.level_right * self.level_right * (3.0 - 2.0 * self.level_right);
        let opacity = self.level_fade_start_ms.map_or(100, |start| {
            let elapsed = self.now_ms().saturating_sub(start).min(LEVEL_FADE_MS);
            let opacity = 1.0 - elapsed as f32 / LEVEL_FADE_MS as f32;
            (opacity * opacity * (3.0 - 2.0 * opacity) * 100.0).round() as u32
        });
        self.canvas.level(
            kind,
            self.level.percent,
            self.level.muted,
            (self.level_fingers.0 as f32, self.level_fingers.1 as f32),
            right,
            opacity,
        );
        self.present_overlay()
    }

    /// Shows whatever the canvas holds on the otherwise dark bar.
    fn present_overlay(&mut self) -> Result<()> {
        self.display
            .present(&self.canvas.pixels, self.canvas.width, self.canvas.height)?;
        if !self.overlay_shown {
            self.backlight.set(self.config.active_brightness)?;
            self.overlay_shown = true;
        }
        Ok(())
    }

    fn hide_overlay(&mut self) -> Result<()> {
        self.overlay_hide_ms = None;
        self.level_kind = None;
        self.toast = None;
        self.level_poll_ms = None;
        self.level_fade_start_ms = None;
        self.level_frame_ms = None;
        self.activity_esc_shown = false;
        self.escape_fade = None;
        self.escape_opacity = 0.0;
        if !self.overlay_shown || self.visible {
            return Ok(());
        }
        self.overlay_shown = false;
        self.canvas.clear();
        self.display
            .present(&self.canvas.pixels, self.canvas.width, self.canvas.height)?;
        self.backlight.set(0)
    }

    fn poll_level(&mut self, now: u64) {
        self.level_poll_ms = (now < self.level_poll_until_ms).then_some(now + LEVEL_POLL_MS);
        if let Some(kind) = self.level_kind {
            self.level_reader.request(kind);
        }
    }

    fn levels_read(&mut self) {
        let now = self.now_ms();
        let results: Vec<_> = self.level_reader.results().collect();
        for (kind, level) in results {
            self.level_cache[cache_slot(kind)] = level;
            if self.level_kind != Some(kind) || level == self.level {
                continue;
            }
            self.level = level;
            self.level_dirty = true;
            // Reaching a stop is only known once the desktop has applied
            // the key, so the firm click follows the read.
            if !at_stop(kind, level, None) {
                self.level_stop_felt = false;
            } else if self.level_stepped && !self.level_stop_felt {
                self.level_stop_felt = true;
                self.haptic.firm();
            }
            // Keep a lingering overlay alive until the value has settled.
            self.overlay_hide_ms = self
                .overlay_hide_ms
                .map(|deadline| deadline.max(now + LEVEL_POLL_MS));
        }
    }

    /// Follows the keyboard backlight. Returns whether the level changed.
    fn refresh_key_level(&mut self) -> bool {
        let level = if self.config.follow_keyboard_backlight {
            kbdlight::key_level().unwrap_or(100)
        } else {
            100
        };
        if level == self.key_level {
            return false;
        }
        self.key_level = level;
        self.canvas.set_level(level);
        true
    }

    fn keyboard_backlight_changed(&mut self) -> Result<()> {
        if let Ok(brightness) = kbdlight::brightness() {
            self.activity_current_brightness = Some(brightness);
            if self.activity_dimmed && self.backlight_fade.is_none() && brightness > 0 {
                self.activity_restore_brightness = Some(brightness);
                self.activity_dimmed = false;
                self.user_activity(self.now_ms(), true)?;
            }
        }
        let changed = self.refresh_key_level();
        if self.touch_id != TouchIdState::Idle {
            return Ok(());
        }
        if self.activity_esc_shown && changed {
            return self.render_activity_esc();
        }
        if !changed {
            return Ok(());
        }
        if self.visible {
            return self.present_keys();
        }
        self.level_dirty = true;
        Ok(())
    }

    fn user_activity(&mut self, now: u64, show_esc: bool) -> Result<()> {
        if !self.config.activity_backlight {
            return Ok(());
        }
        self.activity_deadline_ms = Some(now + self.config.activity_timeout_ms);
        let fading_out = self
            .backlight_fade
            .as_ref()
            .is_some_and(|fade| fade.to == 0);
        if self.activity_dimmed || fading_out {
            let brightness = match self.activity_restore_brightness {
                Some(brightness) if brightness > 0 => Some(brightness),
                _ => match self
                    .activity_max_brightness
                    .map(Ok)
                    .unwrap_or_else(kbdlight::maximum_brightness)
                {
                    Ok(brightness) if brightness > 0 => {
                        self.activity_max_brightness = Some(brightness);
                        Some(brightness)
                    }
                    Ok(_) => None,
                    Err(error) => {
                        eprintln!("kait2en-touchbar: read maximum keyboard brightness: {error:#}");
                        None
                    }
                },
            };
            if let Some(brightness) = brightness {
                let already_fading_in = self
                    .backlight_fade
                    .as_ref()
                    .is_some_and(|fade| fade.to == brightness && brightness > 0);
                if !already_fading_in {
                    let current = self.activity_current_brightness.unwrap_or(0);
                    self.start_backlight_fade(now, current, brightness);
                }
            }
        }
        if self.visible {
            self.deadline_ms = Some(now + self.state.learned(self.state.mode).timeout_ms);
        } else if show_esc && self.touch_id == TouchIdState::Idle {
            self.show_activity_esc()?;
        }
        Ok(())
    }

    fn show_activity_esc(&mut self) -> Result<()> {
        if self.visible || self.touch_id != TouchIdState::Idle {
            return Ok(());
        }
        if self.activity_esc_shown {
            if self.escape_fade.as_ref().is_some_and(|fade| fade.to <= 0.0) {
                self.start_escape_fade(self.now_ms(), self.escape_opacity, 1.0);
            }
            return Ok(());
        }
        let Some(index) = self
            .layout
            .buttons
            .iter()
            .position(|button| button.action == Action::Key(Key::Esc))
        else {
            return Ok(());
        };
        // A track overlay stays and draws the esc key with it.
        self.level_kind = None;
        self.overlay_hide_ms = None;
        self.level_fade_start_ms = None;
        self.level_frame_ms = None;
        self.refresh_key_level();
        self.activity_esc_shown = true;
        self.start_escape_fade(self.now_ms(), self.escape_opacity, 1.0);
        self.render_activity_esc_with_index(index)
    }

    fn activity_timeout(&mut self) {
        self.activity_deadline_ms = None;
        if self.visible && self.touch_id == TouchIdState::Idle {
            if let Err(error) = self.go_dark(false) {
                eprintln!("kait2en-touchbar: darken Touch Bar after inactivity: {error:#}");
            }
        }
        match kbdlight::brightness() {
            Ok(brightness) => {
                self.activity_current_brightness = Some(brightness);
                if brightness > 0 {
                    self.activity_restore_brightness = Some(brightness);
                    self.start_backlight_fade(self.now_ms(), brightness, 0);
                } else {
                    self.backlight_fade = None;
                    self.activity_dimmed = true;
                }
            }
            Err(error) => {
                eprintln!("kait2en-touchbar: read keyboard brightness before dimming: {error:#}")
            }
        }
        if self.activity_esc_shown {
            self.start_escape_fade(self.now_ms(), self.escape_opacity, 0.0);
        }
    }

    fn start_backlight_fade(&mut self, now: u64, from: u32, to: u32) {
        if from == to {
            self.activity_current_brightness = Some(to);
            self.activity_dimmed = to == 0;
            self.backlight_fade = None;
            return;
        }
        self.backlight_fade = Some(BacklightFade {
            start_ms: now,
            next_ms: now,
            from,
            to,
        });
    }

    fn start_escape_fade(&mut self, now: u64, from: f32, to: f32) {
        if (from - to).abs() < f32::EPSILON {
            self.escape_opacity = to;
            self.escape_fade = None;
            return;
        }
        self.escape_fade = Some(EscapeFade {
            start_ms: now,
            next_ms: now,
            from,
            to,
        });
    }

    fn render_activity_esc(&mut self) -> Result<()> {
        let Some(index) = self
            .layout
            .buttons
            .iter()
            .position(|button| button.action == Action::Key(Key::Esc))
        else {
            return Ok(());
        };
        self.render_activity_esc_with_index(index)
    }

    fn render_activity_esc_with_index(&mut self, index: usize) -> Result<()> {
        if self.toast.is_some() {
            return self.track_frame(self.now_ms());
        }
        self.canvas.single_key(
            &self.layout,
            index,
            false,
            (self.escape_opacity * 100.0).round() as u32,
        );
        self.present_overlay()
    }

    fn advance_backlight_fade(&mut self, now: u64) {
        let Some(fade) = self.backlight_fade.as_mut() else {
            return;
        };
        if now < fade.next_ms {
            return;
        }
        let elapsed = now.saturating_sub(fade.start_ms).min(ACTIVITY_FADE_MS);
        let progress = elapsed as f32 / ACTIVITY_FADE_MS as f32;
        let eased = progress * progress * (3.0 - 2.0 * progress);
        let value = (fade.from as f32 + (fade.to as f32 - fade.from as f32) * eased).round() as u32;
        let target = fade.to;
        fade.next_ms = now + ACTIVITY_FRAME_MS;
        if self.activity_current_brightness != Some(value) {
            match kbdlight::set_brightness(value) {
                Ok(()) => self.activity_current_brightness = Some(value),
                Err(error) => {
                    eprintln!("kait2en-touchbar: fade keyboard backlight: {error:#}");
                    self.backlight_fade = None;
                    return;
                }
            }
        }
        if elapsed >= ACTIVITY_FADE_MS {
            self.activity_current_brightness = Some(target);
            self.activity_dimmed = target == 0;
            self.backlight_fade = None;
        }
    }

    fn advance_escape_fade(&mut self, now: u64) -> Result<()> {
        let Some(fade) = self.escape_fade.as_mut() else {
            return Ok(());
        };
        if now < fade.next_ms {
            return Ok(());
        }
        let elapsed = now.saturating_sub(fade.start_ms).min(ACTIVITY_FADE_MS);
        let progress = elapsed as f32 / ACTIVITY_FADE_MS as f32;
        let eased = progress * progress * (3.0 - 2.0 * progress);
        self.escape_opacity = fade.from + (fade.to - fade.from) * eased;
        let target = fade.to;
        fade.next_ms = now + ACTIVITY_FRAME_MS;
        self.render_activity_esc()?;
        if elapsed >= ACTIVITY_FADE_MS {
            self.escape_opacity = target;
            self.escape_fade = None;
            if target <= 0.0 {
                if self.toast.is_some() {
                    // The track overlay goes on alone.
                    self.activity_esc_shown = false;
                    self.track_frame(now)?;
                } else {
                    self.hide_overlay()?;
                }
            }
        }
        Ok(())
    }

    fn restore_activity_backlight_on_exit(&mut self) {
        if self.config.activity_backlight
            && (self.activity_dimmed
                || self
                    .backlight_fade
                    .as_ref()
                    .is_some_and(|fade| fade.to == 0))
            && let Some(brightness) = self.activity_restore_brightness
        {
            if let Err(error) = kbdlight::set_brightness(brightness) {
                eprintln!("kait2en-touchbar: restore keyboard brightness on exit: {error:#}");
            }
        }
    }

    /// `Some(true)` for next, `Some(false)` for previous.
    fn media_direction(&self, index: usize) -> Option<bool> {
        match self.layout.buttons[index].action {
            Action::Key(Key::NextSong) => Some(true),
            Action::Key(Key::PreviousSong) => Some(false),
            _ => None,
        }
    }

    fn seek_step(&mut self, now: u64) -> Result<()> {
        let Some(mut hold) = self.media_hold.take() else {
            return Ok(());
        };
        let since = match hold.seeking_since {
            Some(since) => since,
            None => {
                // The hold turned into seeking: no track skip on release.
                hold.player = self.mpris.active_player();
                self.haptic.click();
                if self
                    .learner
                    .action(now, self.state.mode, &mut self.state, &self.config)
                {
                    self.save();
                }
                *hold.seeking_since.insert(now)
            }
        };
        let step = if now - since >= SEEK_FAST_AFTER_MS {
            SEEK_FAST_STEP
        } else {
            SEEK_STEP
        };
        let seeked = hold
            .player
            .as_deref()
            .is_some_and(|player| self.mpris.seek(player, step, hold.forward).is_ok());
        if !seeked {
            // No MPRIS player: leave it to whoever handles the seek keys.
            let key = if hold.forward {
                Key::FastForward
            } else {
                Key::Rewind
            };
            self.keyboard.press(key)?;
            self.keyboard.release()?;
        }
        hold.next_ms = now + SEEK_INTERVAL_MS;
        self.deadline_ms = Some(now + self.state.learned(self.state.mode).timeout_ms);
        self.media_hold = Some(hold);
        Ok(())
    }

    fn is_esc(&self, index: usize) -> bool {
        self.layout.buttons[index].action == Action::Key(Key::Esc)
    }

    fn press_button(&mut self, index: usize, now: u64) -> Result<()> {
        match self.layout.buttons[index].action {
            Action::Key(key) => self.keyboard.press(key)?,
            // Personal keys act once per tap. Holding repeats nothing.
            Action::Custom(custom) => match self.custom_keys.get(custom).map(|key| &key.action) {
                Some(CustomAction::Send(keys)) => self.keyboard.tap_combination(keys)?,
                Some(CustomAction::Run(command)) => CustomKey::run(command),
                None => {}
            },
            Action::App(key) => match self
                .app_active
                .and_then(|profile| self.app_profiles[profile].keys.get(key))
                .map(|key| &key.action)
            {
                Some(CustomAction::Send(keys)) => self.keyboard.tap_combination(keys)?,
                Some(CustomAction::Run(command)) => CustomKey::run(command),
                None => {}
            },
        }
        self.haptic.key_press();
        self.active_button = Some(index);
        if self
            .learner
            .action(now, self.state.mode, &mut self.state, &self.config)
        {
            self.save();
        }
        self.deadline_ms = Some(now + self.state.learned(self.state.mode).timeout_ms);
        self.present_keys()
    }

    fn touch_motion(&mut self, slot: u32, x: f64, y: f64) -> Result<()> {
        let now = self.now_ms();
        self.user_activity(now, false)?;
        self.resolve_pending(now)?;
        self.positions.insert(slot, x);
        match self.contacts.get(&slot).copied() {
            Some(Contact::Swipe) => self.swipe_moved(now),
            Some(Contact::Pending) => {
                let Some(pending) = self.pending else {
                    return Ok(());
                };
                if let Some(index) = pending.button
                    && self.layout.hit(x, y, self.canvas.height) != Some(index)
                {
                    if pending.dark {
                        // Slid off the dark esc slot: an ordinary wake touch.
                        self.pending = Some(PendingTouch {
                            button: None,
                            ..pending
                        });
                        return self.hide_overlay();
                    }
                    self.pending = None;
                    self.active_button = None;
                    self.contacts.insert(slot, Contact::Cancelled);
                    self.present_keys()?;
                }
                Ok(())
            }
            Some(Contact::Button(index)) => {
                if self.layout.hit(x, y, self.canvas.height) != Some(index) {
                    self.keyboard.release()?;
                    self.active_button = None;
                    self.contacts.insert(slot, Contact::Cancelled);
                    self.present_keys()?;
                }
                Ok(())
            }
            Some(Contact::TouchIdEsc(index)) => {
                if self.layout.hit(x, y, self.canvas.height) != Some(index) {
                    self.active_button = None;
                    self.contacts.insert(slot, Contact::Cancelled);
                    self.render_touch_id(now)?;
                }
                Ok(())
            }
            Some(Contact::MediaHold(index)) => {
                if self.layout.hit(x, y, self.canvas.height) != Some(index) {
                    self.media_hold = None;
                    self.active_button = None;
                    self.contacts.insert(slot, Contact::Cancelled);
                    self.present_keys()?;
                }
                Ok(())
            }
            Some(Contact::TrackKey(forward)) => {
                let Some(toast) = self.toast.as_mut() else {
                    return Ok(());
                };
                let (left, right) = if forward {
                    toast.layout.next
                } else {
                    toast.layout.prev
                };
                if x < f64::from(left) || x >= f64::from(right) {
                    toast.pressed = None;
                    toast.start_ms = now.saturating_sub(TRACK_FADE_MS);
                    self.contacts.insert(slot, Contact::Cancelled);
                    return self.track_frame(now);
                }
                Ok(())
            }
            Some(Contact::DarkEsc) => {
                if !self
                    .layout
                    .hit(x, y, self.canvas.height)
                    .is_some_and(|index| self.is_esc(index))
                {
                    self.keyboard.release()?;
                    self.contacts.insert(slot, Contact::Cancelled);
                    self.hide_overlay()?;
                    self.show_activity_esc()?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn swipe_moved(&mut self, now: u64) -> Result<()> {
        let centroid = self.swipe_centroid();
        let Some(swipe) = self.swipe.as_mut() else {
            return Ok(());
        };
        let steps = swipe.advance(centroid);
        let kind = swipe.kind;
        let span = self.swipe_span();
        if self.level_kind.is_some()
            && ((span.0 - self.level_fingers.0).abs() >= 3.0
                || (span.1 - self.level_fingers.1).abs() >= 3.0)
        {
            // The feedback follows the fingers.
            self.level_fingers = span;
            self.level_dirty = true;
            self.update_level_side(now, false);
        }
        if steps == 0 {
            return Ok(());
        }
        match kind {
            SwipeKind::Volume | SwipeKind::Brightness => {
                // A fast swipe steps more often than the poll interval, so
                // a pending poll is never pushed back.
                self.level_poll_ms.get_or_insert(now + LEVEL_POLL_MS);
                self.level_poll_until_ms = now + LEVEL_POLL_WINDOW_MS;
                let key = match (kind, steps > 0) {
                    (SwipeKind::Volume, true) => Key::VolumeUp,
                    (SwipeKind::Volume, false) => Key::VolumeDown,
                    (_, true) => Key::BrightnessUp,
                    (_, false) => Key::BrightnessDown,
                };
                for _ in 0..steps.unsigned_abs() {
                    self.keyboard.press(key)?;
                    self.keyboard.release()?;
                }
                self.level_stepped = true;
                // Pushing against a stop clicks firmly once, then stays
                // silent like the level does.
                if self
                    .level_kind
                    .is_some_and(|level_kind| at_stop(level_kind, self.level, Some(steps > 0)))
                {
                    if !std::mem::replace(&mut self.level_stop_felt, true) {
                        self.haptic.firm();
                    }
                } else {
                    self.haptic.click();
                }
                Ok(())
            }
            SwipeKind::Mode => {
                // The rows follow the fingers: swiping left brings in the
                // next row from the right. The ring wraps around.
                let app = self.app_active.is_some();
                let mode = if steps < 0 {
                    self.state.mode.next(app)
                } else {
                    self.state.mode.previous(app)
                };
                self.switch_mode(mode, now, true)?;
                if let Some(fade) = self.layer_fade.as_mut() {
                    fade.confirm = true;
                }
                Ok(())
            }
        }
    }

    fn touch_up(&mut self, slot: u32) -> Result<()> {
        let now = self.now_ms();
        self.resolve_pending(now)?;
        self.positions.remove(&slot);
        match self.contacts.remove(&slot) {
            Some(Contact::Pending) => {
                // Lifted within the window: a plain tap.
                if let Some(pending) = self.pending.take() {
                    if pending.dark && pending.button.is_some() {
                        self.keyboard.press(Key::Esc)?;
                        self.keyboard.release()?;
                        self.haptic.key_press();
                        self.haptic.key_release();
                        self.overlay_hide_ms = Some(now + ESC_LINGER_MS);
                    } else if pending.dark {
                        self.wake(true)?;
                    } else if let Some(index) = pending.button {
                        self.press_button(index, now)?;
                        self.keyboard.release()?;
                        self.haptic.key_release();
                        self.active_button = None;
                        self.present_keys()?;
                    }
                }
            }
            Some(Contact::Button(_)) => {
                self.keyboard.release()?;
                self.haptic.key_release();
                self.active_button = None;
                self.present_keys()?;
            }
            Some(Contact::TouchIdEsc(_)) => {
                self.keyboard.press(Key::Esc)?;
                self.keyboard.release()?;
                self.haptic.key_press();
                self.haptic.key_release();
                self.active_button = None;
                self.render_touch_id(now)?;
            }
            Some(Contact::MediaHold(index)) => {
                let seeking = self
                    .media_hold
                    .take()
                    .is_some_and(|hold| hold.seeking_since.is_some());
                if !seeking {
                    self.press_button(index, now)?;
                    self.keyboard.release()?;
                }
                self.haptic.key_release();
                self.active_button = None;
                self.present_keys()?;
            }
            Some(Contact::DarkEsc) => {
                self.keyboard.release()?;
                self.haptic.key_release();
                if self.activity_esc_shown {
                    self.escape_opacity = 1.0;
                    self.escape_fade = None;
                    self.render_activity_esc()?;
                } else {
                    self.overlay_hide_ms = Some(now + ESC_LINGER_MS);
                }
            }
            Some(Contact::TrackKey(forward)) => {
                let key = if forward {
                    Key::NextSong
                } else {
                    Key::PreviousSong
                };
                self.keyboard.press(key)?;
                self.keyboard.release()?;
                self.haptic.key_press();
                self.haptic.key_release();
                if let Some(toast) = self.toast.as_mut() {
                    // Stay fully visible. The next track replaces the text.
                    toast.pressed = None;
                    toast.start_ms = now.saturating_sub(TRACK_FADE_MS);
                }
                self.track_frame(now)?;
            }
            Some(Contact::Swipe) => {
                let swipe_fingers = self
                    .contacts
                    .values()
                    .filter(|contact| matches!(contact, Contact::Swipe))
                    .count();
                if swipe_fingers >= 2 {
                    // Lifting to two fingers drops the swipe back to volume.
                    self.swipe = Some(Swipe::new(
                        SwipeKind::Volume,
                        self.swipe_centroid(),
                        f64::from(self.config.volume_swipe_step_px),
                    ));
                    return self.begin_level(LevelKind::Volume);
                }
                // The centroid would jump to the remaining finger, so the
                // first lift ends the swipe.
                self.swipe = None;
                if self.level_kind.is_some() {
                    self.overlay_hide_ms = Some(now + LEVEL_LINGER_MS);
                }
                for contact in self.contacts.values_mut() {
                    if matches!(contact, Contact::Swipe) {
                        *contact = Contact::Cancelled;
                    }
                }
            }
            _ => {}
        }
        if self.contacts.is_empty() {
            self.touches_armed = true;
            self.swipe = None;
        }
        Ok(())
    }

    fn fn_event(&mut self, pressed: bool) -> Result<()> {
        if self.touch_id != TouchIdState::Idle {
            return Ok(());
        }
        let now = self.now_ms();
        if pressed {
            self.fn_down_ms = Some(now);
            self.fn_toggled = false;
            self.wake(false)?;
        } else {
            self.fn_down_ms = None;
            self.fn_toggled = false;
            if self.visible {
                self.deadline_ms = Some(now + self.state.learned(self.state.mode).timeout_ms);
            }
        }
        Ok(())
    }

    fn toggle_mode_if_due(&mut self, now: u64) -> Result<()> {
        let Some(down) = self.fn_down_ms else {
            return Ok(());
        };
        if self.fn_toggled || !long_press_due(down, now, self.config.fn_long_press_ms) {
            return Ok(());
        }
        self.fn_toggled = true;
        self.switch_mode(self.state.mode.toggled(self.state.alternate), now, true)
    }

    /// Slides to `mode`. `persist` keeps it for the next start.
    fn switch_mode(&mut self, mode: Mode, now: u64, persist: bool) -> Result<()> {
        self.keyboard.release()?;
        self.active_button = None;
        // Fade from whatever is on screen, even from a running fade.
        let from = match self.layer_fade {
            Some(_) => self.blended.clone(),
            None => self.canvas.pixels.clone(),
        };
        // Forward around the ring comes in from the right, like a swipe to
        // the left; Fn hold picks its direction the same way.
        let direction = if mode == self.state.mode.next(self.app_active.is_some()) {
            1
        } else {
            -1
        };
        let clip_left = if self.physical_escape {
            0
        } else {
            usize::from(self.layout.buttons[0].right) + 1
        };
        self.layer_fade = Some(LayerFade {
            from,
            start_ms: now,
            next_ms: now,
            direction,
            clip_left,
            confirm: false,
        });
        self.state.set_mode(mode);
        if persist {
            self.save();
        }
        self.deadline_ms = Some(now + self.state.learned(self.state.mode).timeout_ms);
        self.present_keys()
    }

    /// Shows the app row when the focused app has a profile, and returns to
    /// the previous row when the focus leaves it. The dark bar only takes
    /// note, and the next wake shows the row. Focus changes are not saved:
    /// the app row is not remembered across restarts.
    fn focus_changed(&mut self, window: focus::Window) -> Result<()> {
        let active = apps::find(&self.app_profiles, &window);
        if active == self.app_active {
            return Ok(());
        }
        let previous = std::mem::replace(&mut self.app_active, active);
        let target = match active {
            Some(_) => {
                if previous.is_none() && self.state.mode != Mode::App {
                    self.mode_before_app = self.state.mode;
                }
                Mode::App
            }
            None if self.state.mode == Mode::App => self.mode_before_app,
            None => return Ok(()),
        };
        if !self.visible || self.touch_id != TouchIdState::Idle {
            self.state.set_mode(target);
            return Ok(());
        }
        let now = self.now_ms();
        if target == self.state.mode {
            // Another profiled app: same row, new keys.
            self.keyboard.release()?;
            self.active_button = None;
            return self.present_keys();
        }
        self.switch_mode(target, now, false)
    }

    fn touch_id_changed(&mut self, state: TouchIdState) -> Result<()> {
        self.touch_id = state;
        self.layer_fade = None;
        self.keyboard.release()?;
        self.active_button = None;
        self.clear_touches();
        self.touches_armed = false;
        self.deadline_ms = None;
        if state == TouchIdState::Idle {
            self.next_animation_ms = None;
            return self.go_dark_to_activity_esc(false);
        }
        self.visible = true;
        let now = self.now_ms();
        // Waiting, scanning and retry share the arrow. Keep its swing smooth
        // across those transitions.
        if self.next_animation_ms.is_none() {
            self.animation_start_ms = now;
        }
        self.next_animation_ms = None;
        self.render_touch_id(now)?;
        self.backlight.set(self.config.active_brightness)
    }

    /// Draws the prompt and schedules the next frame while it shows an arrow.
    fn render_touch_id(&mut self, now: u64) -> Result<()> {
        let animated = matches!(
            self.touch_id,
            TouchIdState::Waiting | TouchIdState::Scanning | TouchIdState::Retry
        );
        let nudge = if animated {
            self.next_animation_ms = Some(now + ARROW_FRAME_MS);
            let elapsed = now.saturating_sub(self.animation_start_ms) % ARROW_PERIOD_MS;
            arrow_nudge(elapsed as f32 / ARROW_PERIOD_MS as f32)
        } else {
            self.next_animation_ms = None;
            0.0
        };
        let esc = self
            .layout
            .buttons
            .iter()
            .find(|button| button.action == Action::Key(Key::Esc))
            .copied();
        let active = esc.is_some_and(|button| {
            self.active_button.is_some_and(|index| {
                self.layout
                    .buttons
                    .get(index)
                    .is_some_and(|active_button| active_button.action == button.action)
            })
        });
        self.canvas.touch_id(self.touch_id.as_str(), nudge, esc, active);
        self.display
            .present(&self.canvas.pixels, self.canvas.width, self.canvas.height)
    }

    fn tick(&mut self) -> Result<()> {
        let now = self.now_ms();
        self.resolve_pending(now)?;
        if self
            .layer_fade
            .as_ref()
            .is_some_and(|fade| now >= fade.next_ms)
        {
            self.present_layer_fade(now)?;
        }
        if self
            .toast
            .as_ref()
            .and_then(|toast| toast.next_ms)
            .is_some_and(|deadline| now >= deadline)
        {
            self.track_frame(now)?;
        }
        if self.level_poll_ms.is_some_and(|deadline| now >= deadline) {
            self.poll_level(now);
        }
        if self.overlay_hide_ms.is_some_and(|deadline| now >= deadline) {
            if self.level_kind.is_some() {
                self.overlay_hide_ms = None;
                self.level_fade_start_ms = Some(now);
                self.level_frame_ms = Some(now + LEVEL_FRAME_MS);
                self.level_dirty = true;
                self.flush_level()?;
            } else {
                self.hide_overlay()?;
            }
        }
        if self.level_frame_ms.is_some_and(|deadline| now >= deadline) {
            if self
                .level_fade_start_ms
                .is_some_and(|start| now.saturating_sub(start) >= LEVEL_FADE_MS)
            {
                self.hide_overlay()?;
            } else {
                self.level_dirty = true;
                self.flush_level()?;
                // Frames run while fading out or crossfading sides.
                let moving = self.level_right != f32::from(u8::from(self.level_right_target));
                self.level_frame_ms =
                    (self.level_fade_start_ms.is_some() || moving).then_some(now + LEVEL_FRAME_MS);
            }
        }
        if self
            .activity_deadline_ms
            .is_some_and(|deadline| now >= deadline)
        {
            self.activity_timeout();
        }
        self.advance_backlight_fade(now);
        self.advance_escape_fade(now)?;
        if self
            .media_hold
            .as_ref()
            .is_some_and(|hold| now >= hold.next_ms)
        {
            self.seek_step(now)?;
        }
        self.toggle_mode_if_due(now)?;
        if self.touch_id == TouchIdState::Idle
            && self.visible
            && self.deadline_ms.is_some_and(|deadline| now >= deadline)
        {
            self.go_dark_to_activity_esc(true)?;
        }
        if self.learner.settle(now, &mut self.state, &self.config) {
            self.save();
        }
        if self
            .next_animation_ms
            .is_some_and(|deadline| now >= deadline)
        {
            self.render_touch_id(now)?;
        }
        Ok(())
    }

    fn next_timeout(&self) -> i32 {
        let now = self.now_ms();
        let mut deadlines = Vec::new();
        if let Some(fade) = &self.layer_fade {
            deadlines.push(fade.next_ms);
        }
        if let Some(deadline) = self.toast.as_ref().and_then(|toast| toast.next_ms) {
            deadlines.push(deadline);
        }
        if let Some(hold) = &self.media_hold {
            deadlines.push(hold.next_ms);
        }
        if let Some(deadline) = self.deadline_ms {
            deadlines.push(deadline);
        }
        if let Some(down) = self.fn_down_ms
            && !self.fn_toggled
        {
            deadlines.push(down + self.config.fn_long_press_ms);
        }
        if let Some(deadline) = self.learner.next_settle_ms(&self.config) {
            deadlines.push(deadline);
        }
        if let Some(deadline) = self.next_animation_ms {
            deadlines.push(deadline);
        }
        if let Some(pending) = self.pending {
            deadlines.push(pending.deadline_ms);
        }
        if let Some(deadline) = self.level_poll_ms {
            deadlines.push(deadline);
        }
        if let Some(deadline) = self.overlay_hide_ms {
            deadlines.push(deadline);
        }
        if let Some(deadline) = self.level_frame_ms {
            deadlines.push(deadline);
        }
        if let Some(deadline) = self.activity_deadline_ms {
            deadlines.push(deadline);
        }
        if let Some(fade) = &self.backlight_fade {
            deadlines.push(fade.next_ms);
        }
        if let Some(fade) = &self.escape_fade {
            deadlines.push(fade.next_ms);
        }
        deadlines
            .into_iter()
            .min()
            .map(|deadline| deadline.saturating_sub(now).min(i32::MAX as u64) as i32)
            .unwrap_or(-1)
    }
}

/// One frame of a row switch. The old row leaves before the new row fully
/// appears, while both travel farther than a simple crossfade. Columns left
/// of `clip_left` show the new row as is.
fn compose_slide(
    from: &[u32],
    to: &[u32],
    out: &mut [u32],
    width: usize,
    clip_left: usize,
    direction: i32,
    progress: f32,
) {
    let progress = progress.clamp(0.0, 1.0);
    let shift_out = (direction as f32 * LAYER_SLIDE_PX * progress).round() as isize;
    let shift_in = (direction as f32 * LAYER_SLIDE_PX * (1.0 - progress)).round() as isize;
    let smooth = |value: f32| value * value * (3.0 - 2.0 * value);
    let old_opacity = 1.0 - smooth((progress / 0.68).clamp(0.0, 1.0));
    let new_opacity = smooth(((progress - 0.24) / 0.76).clamp(0.0, 1.0));
    let sample = |frame: &[u32], row: usize, x: isize| -> u32 {
        if x < clip_left as isize || x >= width as isize {
            0
        } else {
            frame[row + x as usize]
        }
    };
    for (row_index, out_row) in out.chunks_mut(width).enumerate() {
        let row = row_index * width;
        for (x, pixel) in out_row.iter_mut().enumerate() {
            if x < clip_left {
                *pixel = to[row + x];
                continue;
            }
            // The old row moved by -shift_out, the new one still lags by
            // shift_in, so each samples its source that far to the side.
            let old = sample(from, row, x as isize + shift_out);
            let new = sample(to, row, x as isize - shift_in);
            let channel = |shift: u32| {
                let a = ((old >> shift) & 0xff) as f32 * old_opacity;
                let b = ((new >> shift) & 0xff) as f32 * new_opacity;
                ((a + b).round().min(255.0) as u32) << shift
            };
            *pixel = channel(16) | channel(8) | channel(0);
        }
    }
}

/// Eased swing: rest at 0, fully nudged halfway through the period.
fn arrow_nudge(phase: f32) -> f32 {
    (1.0 - (phase * std::f32::consts::TAU).cos()) / 2.0
}

fn main() -> Result<()> {
    if std::env::args().any(|arg| arg == "--attach") {
        return usb::ensure_display_configuration();
    }
    if std::env::args().any(|arg| arg == "--detach") {
        return usb::restore_firmware_configuration();
    }
    let config = Config::load()?;
    let state_file = state_path();
    if std::env::args().any(|arg| arg == "--reset-learning") {
        let state = PersistentState::reset(&config);
        state.save(&state_file)?;
        println!("reset {}", state_file.display());
        return Ok(());
    }
    if std::env::args().any(|arg| arg == "--status") {
        let state = PersistentState::load(&state_file, &config);
        println!(
            "mode={:?}\nmedia_timeout_ms={}\nfunction_timeout_ms={}",
            state.mode, state.media.timeout_ms, state.function.timeout_ms
        );
        return Ok(());
    }

    let state = PersistentState::load(&state_file, &config);
    let mut runtime = Runtime::new(config, state)?;
    runtime.go_dark(false)?;

    unsafe {
        libc::signal(libc::SIGINT, stop as *const () as libc::sighandler_t);
        libc::signal(libc::SIGTERM, stop as *const () as libc::sighandler_t);
    }

    let mut touch_input = Libinput::new_with_udev(Interface);
    let mut main_input = Libinput::new_with_udev(Interface);
    touch_input
        .udev_assign_seat("seat-touchbar")
        .map_err(|()| anyhow::anyhow!("assign Touch Bar seat"))?;
    main_input
        .udev_assign_seat("seat0")
        .map_err(|()| anyhow::anyhow!("assign main seat"))?;
    let (touch_id_rx, mut touch_id_wake) = touchid::watch()?;
    touch_id_wake.set_nonblocking(true)?;
    let mut kbd_wake = kbdlight::watch()?;
    kbd_wake.set_nonblocking(true)?;
    let tracks = if runtime.config.show_track_changes {
        let (receiver, wake) = mpris::watch_tracks()?;
        wake.set_nonblocking(true)?;
        Some((receiver, wake))
    } else {
        None
    };
    // Only profiles need to know the focused window.
    let focus = if runtime.app_profiles.is_empty() {
        None
    } else {
        let (receiver, wake) = focus::watch()?;
        wake.set_nonblocking(true)?;
        Some((receiver, wake))
    };
    let mut touch_device: Option<InputDevice> = None;

    while !STOP.load(Ordering::Relaxed) {
        let mut poll_fds = [
            pollfd {
                fd: touch_input.as_fd().as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
            pollfd {
                fd: main_input.as_fd().as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
            pollfd {
                fd: touch_id_wake.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
            pollfd {
                fd: kbd_wake.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
            pollfd {
                fd: runtime.level_wake.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
            pollfd {
                // poll ignores negative descriptors.
                fd: tracks.as_ref().map_or(-1, |(_, wake)| wake.as_raw_fd()),
                events: libc::POLLIN,
                revents: 0,
            },
            pollfd {
                fd: focus.as_ref().map_or(-1, |(_, wake)| wake.as_raw_fd()),
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        let result = unsafe {
            libc::poll(
                poll_fds.as_mut_ptr(),
                poll_fds.len() as _,
                runtime.next_timeout(),
            )
        };
        if result < 0 && std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
            return Err(std::io::Error::last_os_error()).context("poll input devices");
        }

        if poll_fds[2].revents & libc::POLLIN != 0 {
            let mut discard = [0u8; 64];
            while touch_id_wake.read(&mut discard).is_ok() {}
            while let Ok(state) = touch_id_rx.try_recv() {
                runtime.touch_id_changed(state)?;
            }
        }

        if poll_fds[3].revents & libc::POLLIN != 0 {
            let mut discard = [0u8; 64];
            while kbd_wake.read(&mut discard).is_ok() {}
            runtime.keyboard_backlight_changed()?;
        }

        if poll_fds[5].revents & libc::POLLIN != 0
            && let Some((receiver, wake)) = &tracks
        {
            let mut discard = [0u8; 64];
            while (&*wake).read(&mut discard).is_ok() {}
            while let Ok(track) = receiver.try_recv() {
                runtime.track_changed(track)?;
            }
        }

        if poll_fds[6].revents & libc::POLLIN != 0
            && let Some((receiver, wake)) = &focus
        {
            let mut discard = [0u8; 64];
            while (&*wake).read(&mut discard).is_ok() {}
            // Only the newest window matters after a burst of changes.
            if let Some(window) = receiver.try_iter().last() {
                runtime.focus_changed(window)?;
            }
        }

        if poll_fds[4].revents & libc::POLLIN != 0 {
            let mut discard = [0u8; 64];
            while (&runtime.level_wake).read(&mut discard).is_ok() {}
            runtime.levels_read();
        }

        if poll_fds[0].revents & libc::POLLIN != 0 {
            touch_input.dispatch()?;
            for event in &mut touch_input {
                match event {
                    Event::Device(DeviceEvent::Added(added))
                        if is_touch_bar(added.device().name()) =>
                    {
                        touch_device = Some(added.device());
                    }
                    Event::Touch(event)
                        if touch_device
                            .as_ref()
                            .is_none_or(|device| *device == event.device()) =>
                    {
                        match event {
                            TouchEvent::Down(down) => runtime.touch_down(
                                down.seat_slot(),
                                down.x_transformed(runtime.canvas.width as u32),
                                down.y_transformed(runtime.canvas.height as u32),
                            )?,
                            TouchEvent::Motion(motion) => runtime.touch_motion(
                                motion.seat_slot(),
                                motion.x_transformed(runtime.canvas.width as u32),
                                motion.y_transformed(runtime.canvas.height as u32),
                            )?,
                            TouchEvent::Up(up) => runtime.touch_up(up.seat_slot())?,
                            TouchEvent::Cancel(cancel) => runtime.touch_up(cancel.seat_slot())?,
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
        }

        if poll_fds[1].revents & libc::POLLIN != 0 {
            main_input.dispatch()?;
            for event in &mut main_input {
                match event {
                    Event::Device(DeviceEvent::Added(added)) => {
                        let device = added.device();
                        if !provides_fn(&device)
                            && !(runtime.config.activity_backlight && provides_activity(&device))
                        {
                            let _ = device.config_send_events_set_mode(SendEventsMode::DISABLED);
                        }
                    }
                    Event::Keyboard(KeyboardEvent::Key(key))
                        if key.device().name() != "T2 Touch Bar" =>
                    {
                        let pressed = key.key_state() == KeyState::Pressed;
                        runtime.user_activity(runtime.now_ms(), true)?;
                        if key.key() == Key::Fn as u32 {
                            runtime.fn_event(pressed)?;
                        }
                    }
                    Event::Pointer(pointer) if pointer.device().name() != "T2 Touch Bar" => {
                        runtime.user_activity(runtime.now_ms(), true)?;
                    }
                    _ => {}
                }
            }
        }
        runtime.tick()?;
        runtime.flush_level()?;
    }
    runtime.go_dark(false)?;
    runtime.restore_activity_backlight_on_exit();
    Ok(())
}

fn provides_fn(device: &InputDevice) -> bool {
    device.has_capability(DeviceCapability::Keyboard)
        && device.keyboard_has_key(Key::Fn as u32) == Ok(true)
}

fn provides_activity(device: &InputDevice) -> bool {
    device.name() != "T2 Touch Bar"
        && (device.has_capability(DeviceCapability::Keyboard)
            || device.has_capability(DeviceCapability::Pointer))
}

/// Whether `level` sits at the bottom or at 100, or only at the one `up`
/// points to. GNOME never dims the display below 1%, which is its bottom.
fn at_stop(kind: LevelKind, level: Level, up: Option<bool>) -> bool {
    let Some(percent) = level.percent else {
        return false;
    };
    let bottom = match kind {
        LevelKind::Volume => 0,
        LevelKind::Brightness => 1,
    };
    match up {
        Some(true) => percent >= 100,
        Some(false) => percent <= bottom,
        None => percent <= bottom || percent >= 100,
    }
}

fn cache_slot(kind: LevelKind) -> usize {
    match kind {
        LevelKind::Volume => 0,
        LevelKind::Brightness => 1,
    }
}

fn is_touch_bar(name: &str) -> bool {
    name.contains("Touch Bar") || name.contains("TouchBar")
}

fn long_press_due(down_ms: u64, now_ms: u64, threshold_ms: u64) -> bool {
    now_ms.saturating_sub(down_ms) >= threshold_ms
}

fn has_physical_escape() -> bool {
    let Ok(product) = std::fs::read_to_string("/sys/class/dmi/id/product_name") else {
        return false;
    };
    product
        .trim()
        .strip_prefix("MacBookPro")
        .and_then(|rest| rest.split(',').next())
        .and_then(|major| major.parse::<u32>().ok())
        .is_some_and(|major| major >= 16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn touch_bar_names_are_narrow() {
        assert!(is_touch_bar("Apple Inc. Touch Bar Display Touchpad"));
        assert!(!is_touch_bar("Apple Internal Keyboard / Trackpad"));
    }

    #[test]
    fn row_switch_slides_and_keeps_esc() {
        // One row, 300 px: esc left of column 10, a lit pixel at 150.
        let width = 300;
        let mut from = vec![0; width];
        let mut to = vec![0; width];
        from[150] = 0x00ff_ffff;
        to[150] = 0x00ff_ffff;
        from[5] = 0x0011_1111;
        to[5] = 0x0022_2222;
        let mut out = vec![0; width];
        // Halfway through a forward switch, the rows have moved apart and
        // each is independently fading through black.
        compose_slide(&from, &to, &mut out, width, 10, 1, 0.5);
        assert_eq!(out[55], 0x002c_2c2c);
        assert_eq!(out[245], 0x0045_4545);
        assert_eq!(out[150], 0);
        assert_eq!(out[5], 0x0022_2222, "esc is never moved or blended");
        // At the end only the new row remains, in place.
        compose_slide(&from, &to, &mut out, width, 10, 1, 1.0);
        assert_eq!(out[150], 0x00ff_ffff);
        assert_eq!(out[100], 0);
    }

    #[test]
    fn fn_hold_switches_at_the_configured_boundary() {
        assert!(!long_press_due(1_000, 1_599, 600));
        assert!(long_press_due(1_000, 1_600, 600));
    }
}
