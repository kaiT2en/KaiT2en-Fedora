// SPDX-License-Identifier: GPL-3.0-or-later

use std::fs;

use ab_glyph::{Font, FontVec, OutlineCurve, VariableFont};
use anyhow::{Context, Result, anyhow};
use input_linux::Key;
use tiny_skia::{
    Color, FillRule, LineCap, LineJoin, Paint, Path, PathBuilder, Pixmap, Rect, Stroke, Transform,
};

use crate::policy::Mode;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Action {
    Key(Key),
    /// Index into the personal keys from keys.toml.
    Custom(usize),
    /// Index into the keys of the focused app's profile from apps.toml.
    App(usize),
}

#[derive(Clone, Copy)]
enum Glyph {
    Text(&'static str),
    Icon(Icon),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LevelKind {
    Volume,
    Brightness,
}

#[derive(Clone, Copy)]
enum Icon {
    Brightness(bool),
    Microphone,
    Keyboard(bool),
    Previous,
    PlayPause,
    Next,
    Mute,
    Volume(bool),
}

const KEY_GAP: u16 = 10;
const KEY_GAP_F: f32 = KEY_GAP as f32;
/// Swipe level feedback: icon slot, gap and meter.
const LEVEL_ICON_SLOT: f32 = 60.0;
const LEVEL_SPACING: f32 = 22.0;
const LEVEL_METER: f32 = 320.0;
const GROUP_GAP: u16 = 56;

/// Previous and next key spans (left, right) of the track overlay.
#[derive(Clone, Copy, Debug, Default)]
pub struct TrackLayout {
    pub prev: (u16, u16),
    pub next: (u16, u16),
}

#[derive(Clone, Copy)]
pub struct Button {
    pub left: u16,
    pub right: u16,
    glyph: Glyph,
    pub action: Action,
}

pub struct Layout {
    pub buttons: Vec<Button>,
}

impl Layout {
    pub fn new(mode: Mode, width: u16, physical_escape: bool) -> Self {
        Self::with_custom(mode, width, physical_escape, &[])
    }

    /// Like `new`, but the special row also shows `custom` labels in its
    /// free space next to esc, as many as fit. The app row shows `custom`
    /// as its keys.
    pub fn with_custom(
        mode: Mode,
        width: u16,
        physical_escape: bool,
        custom: &[&'static str],
    ) -> Self {
        // Keys inside a group sit KEY_GAP apart. Groups are separated by the
        // wider GROUP_GAP, like the clusters of a hardware function row.
        let mut groups: Vec<Vec<(Glyph, Key)>> = Vec::new();
        match mode {
            Mode::Function => {
                const LABELS: [&str; 12] = [
                    "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "F12",
                ];
                let keys = [
                    Key::F1,
                    Key::F2,
                    Key::F3,
                    Key::F4,
                    Key::F5,
                    Key::F6,
                    Key::F7,
                    Key::F8,
                    Key::F9,
                    Key::F10,
                    Key::F11,
                    Key::F12,
                ];
                groups.push(
                    LABELS
                        .into_iter()
                        .zip(keys)
                        .map(|(label, key)| (Glyph::Text(label), key))
                        .collect(),
                );
            }
            Mode::Media => groups.extend([
                vec![
                    (Glyph::Icon(Icon::Brightness(false)), Key::BrightnessDown),
                    (Glyph::Icon(Icon::Brightness(true)), Key::BrightnessUp),
                ],
                vec![
                    (Glyph::Icon(Icon::Keyboard(false)), Key::IllumDown),
                    (Glyph::Icon(Icon::Keyboard(true)), Key::IllumUp),
                ],
                vec![
                    (Glyph::Icon(Icon::Previous), Key::PreviousSong),
                    (Glyph::Icon(Icon::PlayPause), Key::PlayPause),
                    (Glyph::Icon(Icon::Next), Key::NextSong),
                ],
                vec![
                    (Glyph::Icon(Icon::Microphone), Key::MicMute),
                    (Glyph::Icon(Icon::Mute), Key::Mute),
                    (Glyph::Icon(Icon::Volume(false)), Key::VolumeDown),
                    (Glyph::Icon(Icon::Volume(true)), Key::VolumeUp),
                ],
            ]),
            Mode::App => {}
            Mode::Special => groups.extend([
                vec![(Glyph::Text("print"), Key::Sysrq)],
                vec![
                    (Glyph::Text("insert"), Key::Insert),
                    (Glyph::Text("delete"), Key::Delete),
                ],
                vec![
                    (Glyph::Text("home"), Key::Home),
                    (Glyph::Text("end"), Key::End),
                    (Glyph::Text("page up"), Key::PageUp),
                    (Glyph::Text("page down"), Key::PageDown),
                ],
            ]),
        }

        // esc keeps the same slot in every mode, so switching layers only
        // swaps the keys to its right and never moves or resizes it.
        let mut buttons = Vec::new();
        let mut start = KEY_GAP;
        if !physical_escape {
            let right = start + width / 14;
            buttons.push(Button {
                left: start,
                right,
                glyph: Glyph::Text("esc"),
                action: Action::Key(Key::Esc),
            });
            start = right + GROUP_GAP;
        }

        if mode == Mode::App {
            // App keys keep the media key width and run from esc to the
            // right, as many as fit.
            let cell = Self::media_cell(width, physical_escape);
            let fit = usize::from((width.saturating_sub(start) + KEY_GAP) / (cell + KEY_GAP));
            let mut left = start;
            for (index, label) in custom.iter().enumerate().take(fit) {
                buttons.push(Button {
                    left,
                    right: left + cell,
                    glyph: Glyph::Text(label),
                    action: Action::App(index),
                });
                left += cell + KEY_GAP;
            }
            return Self { buttons };
        }

        if mode == Mode::Special {
            // Fewer keys than fit: keep the media key width and align them
            // to the right edge, leaving the space next to esc free.
            let cell = Self::media_cell(width, physical_escape);
            let mut right = width.saturating_sub(KEY_GAP);
            let mut placed = Vec::new();
            for group in groups.into_iter().rev() {
                for (glyph, key) in group.into_iter().rev() {
                    let left = right.saturating_sub(cell);
                    placed.push(Button {
                        left,
                        right,
                        glyph,
                        action: Action::Key(key),
                    });
                    right = left.saturating_sub(KEY_GAP);
                }
                right = right.saturating_sub(GROUP_GAP - KEY_GAP);
            }
            // Personal keys form one more group in the space that is left.
            let space = right.saturating_sub(start);
            let fit = usize::from((space + KEY_GAP) / (cell + KEY_GAP));
            for (index, label) in custom.iter().enumerate().take(fit).rev() {
                let left = right.saturating_sub(cell);
                placed.push(Button {
                    left,
                    right,
                    glyph: Glyph::Text(label),
                    action: Action::Custom(index),
                });
                right = left.saturating_sub(KEY_GAP);
            }
            buttons.extend(placed.into_iter().rev());
            return Self { buttons };
        }

        // The remaining span is filled edge to edge. Rounding leftovers widen
        // the first keys by one pixel instead of shifting the row.
        let count = groups.iter().map(Vec::len).sum::<usize>().max(1) as u16;
        let group_count = groups.len().max(1) as u16;
        let spacing = KEY_GAP * (count - group_count) + GROUP_GAP * (group_count - 1);
        let usable = width.saturating_sub(start + KEY_GAP + spacing);
        let cell = usable / count;
        let mut extra = usable - cell * count;
        let mut left = start;
        for group in groups {
            for (glyph, key) in group {
                let mut right = left + cell;
                if extra > 0 {
                    right += 1;
                    extra -= 1;
                }
                buttons.push(Button {
                    left,
                    right,
                    glyph,
                    action: Action::Key(key),
                });
                left = right + KEY_GAP;
            }
            left += GROUP_GAP - KEY_GAP;
        }
        Self { buttons }
    }

    /// Width of a media key, the size the shorter rows reuse.
    fn media_cell(width: u16, physical_escape: bool) -> u16 {
        Self::new(Mode::Media, width, physical_escape)
            .buttons
            .last()
            .map_or(width / 14, |button| button.right - button.left)
    }

    pub fn hit(&self, x: f64, y: f64, height: u16) -> Option<usize> {
        if y < 5.0 || y > f64::from(height.saturating_sub(5)) {
            return None;
        }
        self.buttons
            .iter()
            .position(|button| {
                // Let the virtual Esc key respond all the way to the left edge
                let left = if button.action == Action::Key(Key::Esc) {
                    0.0
                } else {
                    f64::from(button.left)
                };
                x >= left && x < f64::from(button.right)
            })
    }
}

/// Variable sans fonts that match the San Francisco legends of the keyboard
/// closely enough. Adwaita Sans ships with GNOME on Fedora.
const FONT_PATHS: [&str; 3] = [
    "/usr/share/fonts/adwaita-sans-fonts/AdwaitaSans-Regular.ttf",
    "/usr/share/fonts/rsms-inter-vf-fonts/InterVariable.ttf",
    "/usr/share/fonts/abattis-cantarell-vf-fonts/Cantarell-VF.otf",
];
const FONT_WEIGHT: f32 = 500.0;
/// Emphasis in the Touch ID prompt ("**Unlock** with Touch ID").
const STRONG_WEIGHT: f32 = 700.0;
const PROMPT_MARGIN: f32 = 18.0;
const TRACK_FONT_SIZE: f32 = 24.0;
const TRACK_GAP: f32 = 40.0;
const ARROW_LENGTH: f32 = 22.0;
const ARROW_GAP: f32 = 14.0;
const ARROW_STROKE: f32 = 3.0;
/// How far the arrow's tail moves towards the sensor. The tip moves twice as
/// far.
const ARROW_TAIL_TRAVEL: f32 = 7.0;
const KEY_FONT_SIZE: f32 = 23.0;

/// Icons are drawn on a 24 px design grid and centered by their bounds so
/// every key gets the same optical margin.
const ICON_SCALE: f32 = 1.3;
const ICON_STROKE: f32 = 2.3;
const KEY_INSET: f32 = 6.5;
const KEY_RADIUS: f32 = 8.0;
const KEY_OUTLINE: f32 = 1.5;

pub struct Canvas {
    pub width: u16,
    pub height: u16,
    pub pixels: Vec<u32>,
    pixmap: Pixmap,
    font: FontVec,
    /// The configured key color at full intensity.
    base: Rgb,
    /// `base` dimmed to the current key level.
    color: Rgb,
}

#[derive(Clone, Copy)]
struct Rgb(u8, u8, u8);

impl Rgb {
    fn from_u32(color: u32) -> Self {
        Self((color >> 16) as u8, (color >> 8) as u8, color as u8)
    }

    fn scale(self, percent: u32) -> Self {
        let channel = |value: u8| (u32::from(value) * percent / 100).min(255) as u8;
        Self(channel(self.0), channel(self.1), channel(self.2))
    }

    fn paint(self) -> Paint<'static> {
        let mut paint = Paint::default();
        paint.set_color_rgba8(self.0, self.1, self.2, 255);
        paint.anti_alias = true;
        paint
    }
}

const BLACK: Rgb = Rgb(0, 0, 0);

impl Canvas {
    pub fn new(width: u16, height: u16, key_color: u32) -> Result<Self> {
        let pixmap = Pixmap::new(u32::from(width), u32::from(height))
            .ok_or_else(|| anyhow!("invalid canvas size {width}x{height}"))?;
        let mut canvas = Self {
            width,
            height,
            pixels: vec![0; width as usize * height as usize],
            pixmap,
            font: load_font()?,
            base: Rgb::from_u32(key_color),
            color: Rgb::from_u32(key_color),
        };
        canvas.clear();
        Ok(canvas)
    }

    /// Dims everything drawn in the key color, in percent of full intensity.
    pub fn set_level(&mut self, percent: u32) {
        self.color = self.base.scale(percent.min(100));
    }

    pub fn clear(&mut self) {
        self.pixmap.fill(Color::BLACK);
        self.pixels.fill(0);
    }

    pub fn keys(&mut self, layout: &Layout, active: Option<usize>) {
        self.pixmap.fill(Color::BLACK);
        for (index, button) in layout.buttons.iter().enumerate() {
            self.button(button, active == Some(index));
        }
        self.flush();
    }

    /// Level feedback for a swipe on the dark bar: icon and meter beside the
    /// fingers. `fingers` is the x range from the leftmost to the rightmost
    /// finger. `right` is how much of it shows on their right side (0 to 1),
    /// the rest shows on their left, so a side change can crossfade.
    pub fn level(
        &mut self,
        kind: LevelKind,
        percent: Option<u32>,
        muted: bool,
        fingers: (f32, f32),
        right: f32,
        fade: u32,
    ) {
        // From the touch point of the finger nearest to the feedback, so any
        // number of fingers keeps the same clearance.
        const DISTANCE: f32 = 110.0;
        let total = LEVEL_ICON_SLOT + LEVEL_SPACING + LEVEL_METER;
        let width = f32::from(self.width);
        let (first, last) = fingers;
        let full = self.color;
        self.pixmap.fill(Color::BLACK);
        let right = right.clamp(0.0, 1.0);
        for (left, share) in [
            (first - DISTANCE - total, 1.0 - right),
            (last + DISTANCE, right),
        ] {
            let opacity = (fade.min(100) as f32 * share).round() as u32;
            if opacity == 0 {
                continue;
            }
            let left = left.clamp(KEY_GAP_F, width - KEY_GAP_F - total).round();
            self.color = full.scale(opacity);
            self.level_at(left, kind, percent, muted);
        }
        self.flush();
        self.color = full;
    }

    fn level_at(&mut self, left: f32, kind: LevelKind, percent: Option<u32>, muted: bool) {
        const ICON_SIZE: f32 = 1.6;
        const METER_HEIGHT: f32 = 10.0;
        let cy = f32::from(self.height) / 2.0;
        let icon = match (kind, muted) {
            (LevelKind::Volume, true) => Icon::Mute,
            (LevelKind::Volume, false) => Icon::Volume(true),
            (LevelKind::Brightness, _) => Icon::Brightness(true),
        };
        self.icon_sized(left + LEVEL_ICON_SLOT / 2.0, cy, icon, BLACK, ICON_SIZE);

        let meter_left = left + LEVEL_ICON_SLOT + LEVEL_SPACING;
        let (top, bottom) = (cy - METER_HEIGHT / 2.0, cy + METER_HEIGHT / 2.0);
        let radius = METER_HEIGHT / 2.0;
        let track = rounded_rect(meter_left, top, meter_left + LEVEL_METER, bottom, radius);
        self.fill(&track, self.color.scale(22));
        let fraction = percent.map_or(0.0, |percent| percent.min(100) as f32 / 100.0);
        if fraction > 0.0 && !muted {
            let right = meter_left + (LEVEL_METER * fraction).max(METER_HEIGHT);
            self.fill(
                &rounded_rect(meter_left, top, right, bottom, radius),
                self.color,
            );
        }
    }

    /// "**Artist** – Title" on the dark bar with borderless previous/next
    /// keys at `keys`, the fixed slots at both ends of the bar. `fade`
    /// scales everything (percent) for the fade in and out; `pressed`
    /// highlights next (`true`) or previous (`false`). `esc` is the esc
    /// key with its own opacity, drawn while it is shown on the dark bar.
    pub fn track(
        &mut self,
        artist: &str,
        title: &str,
        fade: u32,
        pressed: Option<bool>,
        keys: TrackLayout,
        esc: Option<(Button, u32)>,
    ) {
        let space_left = f32::from(keys.prev.1) + TRACK_GAP;
        let space_right = f32::from(keys.next.0) - TRACK_GAP;
        let (artist, title) = self.fit_track(artist, title, space_right - space_left);
        let separator = if artist.is_empty() { "" } else { " – " };
        let parts = [
            (artist.as_str(), STRONG_WEIGHT),
            (separator, FONT_WEIGHT),
            (title.as_str(), FONT_WEIGHT),
        ];
        let text_width: f32 = parts
            .iter()
            .map(|(text, weight)| self.text_width_at(text, TRACK_FONT_SIZE, *weight))
            .sum();

        let full = self.color;
        self.pixmap.fill(Color::BLACK);
        if let Some((esc, opacity)) = esc {
            self.color = full.scale(opacity.min(100));
            self.button(&esc, false);
        }
        self.color = full.scale(fade.min(100));
        let top = KEY_INSET;
        let bottom = f32::from(self.height) - KEY_INSET;
        let cy = f32::from(self.height) / 2.0;
        for ((left, right), icon, forward) in [
            (keys.prev, Icon::Previous, false),
            (keys.next, Icon::Next, true),
        ] {
            let (left, right) = (f32::from(left), f32::from(right));
            let background = if pressed == Some(forward) {
                let fill = self.color.scale(20);
                self.fill(&rounded_rect(left, top, right, bottom, KEY_RADIUS), fill);
                fill
            } else {
                BLACK
            };
            self.icon((left + right) / 2.0, cy, icon, background);
        }
        let baseline = (cy + self.cap_height(TRACK_FONT_SIZE) / 2.0).round();
        let mut path = PathBuilder::new();
        let mut x = ((space_left + space_right - text_width) / 2.0).round();
        for (text, weight) in parts {
            x = self.append_text(&mut path, x, baseline, text, TRACK_FONT_SIZE, weight);
        }
        if let Some(path) = path.finish() {
            self.fill(&path, self.color);
        }
        self.color = full;
        self.flush();
    }

    /// Shortens the title, then the artist, with an ellipsis until the line
    /// fits.
    fn fit_track(&mut self, artist: &str, title: &str, max: f32) -> (String, String) {
        let mut artist: Vec<char> = artist.chars().collect();
        let mut title: Vec<char> = title.chars().collect();
        let (mut artist_cut, mut title_cut) = (false, false);
        loop {
            let shown = |chars: &[char], cut: bool| {
                let mut text: String = chars.iter().collect();
                if cut {
                    text = format!("{}…", text.trim_end());
                }
                text
            };
            let (a, t) = (shown(&artist, artist_cut), shown(&title, title_cut));
            let separator = if a.is_empty() {
                0.0
            } else {
                self.text_width_at(" – ", TRACK_FONT_SIZE, FONT_WEIGHT)
            };
            let width = self.text_width_at(&a, TRACK_FONT_SIZE, STRONG_WEIGHT)
                + separator
                + self.text_width_at(&t, TRACK_FONT_SIZE, FONT_WEIGHT);
            if width <= max || (artist.len() <= 1 && title.len() <= 1) {
                return (a, t);
            }
            if title.len() > 12 || artist.len() <= 12 {
                title.pop();
                title_cut = true;
            } else {
                artist.pop();
                artist_cut = true;
            }
        }
    }

    /// One key on an otherwise black bar, with optional pressed styling and
    /// opacity for the activity escape key.
    pub fn single_key(&mut self, layout: &Layout, index: usize, active: bool, fade: u32) {
        let full = self.color;
        self.color = full.scale(fade.min(100));
        self.pixmap.fill(Color::BLACK);
        if let Some(button) = layout.buttons.get(index) {
            self.button(button, active);
        }
        self.flush();
        self.color = full;
    }

    fn button(&mut self, button: &Button, active: bool) {
        let top = KEY_INSET;
        let bottom = f32::from(self.height) - KEY_INSET;
        let cy = f32::from(self.height) / 2.0;
        let (left, right) = (f32::from(button.left), f32::from(button.right));
        let background = if active {
            let fill = self.color.scale(20);
            self.fill(&rounded_rect(left, top, right, bottom, KEY_RADIUS), fill);
            fill
        } else {
            BLACK
        };
        let half = KEY_OUTLINE / 2.0;
        self.stroke(
            &rounded_rect(
                left + half,
                top + half,
                right - half,
                bottom - half,
                KEY_RADIUS,
            ),
            self.color.scale(if active { 34 } else { 22 }),
            KEY_OUTLINE,
        );
        let cx = (left + right) / 2.0;
        match button.glyph {
            Glyph::Text(text) => self.center_text(cx, cy, text, KEY_FONT_SIZE, self.color),
            Glyph::Icon(icon) => self.icon(cx, cy, icon, background),
        }
    }

    /// Authentication prompt: a line of text with an arrow towards the
    /// sensor right of the bar. `nudge` (0..1) pushes the arrow towards the
    /// sensor: its tail moves by `ARROW_TAIL_TRAVEL`, its tip twice as far,
    /// so it stretches while it moves.
    pub fn touch_id(&mut self, state: &str, nudge: f32, esc: Option<Button>, active: bool) {
        self.pixmap.fill(Color::BLACK);
        if let Some(esc) = esc {
            self.button(&esc, active);
        }
        let (parts, color, arrow): (&[(&str, f32)], Rgb, bool) = match state {
            "matched" => (&[("Unlocked", STRONG_WEIGHT)], Rgb(0x34, 0xc7, 0x59), false),
            "retry" => (&[("Try Again", STRONG_WEIGHT)], Rgb(0xff, 0x9f, 0x0a), true),
            "failed" => (
                &[("Use Password", STRONG_WEIGHT)],
                Rgb(0xff, 0x45, 0x3a),
                false,
            ),
            _ => (
                &[("Unlock", STRONG_WEIGHT), (" with Touch ID", FONT_WEIGHT)],
                self.color,
                true,
            ),
        };
        let size = KEY_FONT_SIZE;
        let height = f32::from(self.height);
        let cy = height / 2.0;
        let text_width: f32 = parts
            .iter()
            .map(|(text, weight)| self.text_width_at(text, size, *weight))
            .sum();
        // Room for the fully stretched arrow, so the text never moves.
        let arrow_width = if arrow {
            ARROW_GAP + ARROW_LENGTH + 2.0 * ARROW_TAIL_TRAVEL
        } else {
            0.0
        };
        let right = f32::from(self.width) - PROMPT_MARGIN;
        let left = (right - arrow_width - text_width).round();

        let baseline = (cy + self.cap_height(size) / 2.0).round();
        let mut text = PathBuilder::new();
        let mut x = left;
        for (part, weight) in parts {
            x = self.append_text(&mut text, x, baseline, part, size, *weight);
        }
        let mut arrow_path = PathBuilder::new();
        if arrow {
            let nudge = nudge.clamp(0.0, 1.0) * ARROW_TAIL_TRAVEL;
            let tail = right - 2.0 * ARROW_TAIL_TRAVEL - ARROW_LENGTH + nudge;
            let tip = right - 2.0 * ARROW_TAIL_TRAVEL + 2.0 * nudge;
            let head = ARROW_LENGTH * 0.42;
            arrow_path.move_to(tail, cy);
            arrow_path.line_to(tip, cy);
            arrow_path.move_to(tip - head, cy - head);
            arrow_path.line_to(tip, cy);
            arrow_path.line_to(tip - head, cy + head);
        }

        let paint = color.paint();
        if let Some(path) = text.finish() {
            self.pixmap.fill_path(
                &path,
                &paint,
                FillRule::Winding,
                Transform::identity(),
                None,
            );
        }
        if let Some(path) = arrow_path.finish() {
            let stroke = Stroke {
                width: ARROW_STROKE,
                line_cap: LineCap::Round,
                line_join: LineJoin::Round,
                ..Stroke::default()
            };
            self.pixmap
                .stroke_path(&path, &paint, &stroke, Transform::identity(), None);
        }
        self.flush();
    }

    /// Converts the premultiplied, always opaque RGBA pixmap into XRGB8888.
    fn flush(&mut self) {
        for (target, pixel) in self.pixels.iter_mut().zip(self.pixmap.pixels()) {
            *target = (u32::from(pixel.red()) << 16)
                | (u32::from(pixel.green()) << 8)
                | u32::from(pixel.blue());
        }
    }

    fn fill(&mut self, path: &Path, color: Rgb) {
        self.pixmap.fill_path(
            path,
            &color.paint(),
            FillRule::Winding,
            Transform::identity(),
            None,
        );
    }

    fn stroke(&mut self, path: &Path, color: Rgb, width: f32) {
        let stroke = Stroke {
            width,
            line_cap: LineCap::Round,
            line_join: LineJoin::Round,
            ..Stroke::default()
        };
        self.pixmap
            .stroke_path(path, &color.paint(), &stroke, Transform::identity(), None);
    }

    fn icon(&mut self, cx: f32, cy: f32, icon: Icon, background: Rgb) {
        self.icon_sized(cx, cy, icon, background, 1.0);
    }

    /// `size` scales the key-sized icon, including its stroke weight.
    fn icon_sized(&mut self, cx: f32, cy: f32, icon: Icon, background: Rgb, size: f32) {
        let scale = ICON_SCALE * size;
        let stroke = ICON_STROKE * size;
        let shapes = icon_shapes(icon);
        let mut bounds: Option<Rect> = None;
        for shape in &shapes {
            let pad = shape.pad() * size;
            let Some(path) = shape
                .path()
                .clone()
                .transform(Transform::from_scale(scale, scale))
            else {
                continue;
            };
            let path_bounds = path.bounds();
            let rect = Rect::from_ltrb(
                path_bounds.left() - pad,
                path_bounds.top() - pad,
                path_bounds.right() + pad,
                path_bounds.bottom() + pad,
            );
            bounds = match (bounds, rect) {
                (Some(old), Some(new)) => Rect::from_ltrb(
                    old.left().min(new.left()),
                    old.top().min(new.top()),
                    old.right().max(new.right()),
                    old.bottom().max(new.bottom()),
                ),
                (old, new) => old.or(new),
            };
        }
        let Some(bounds) = bounds else { return };
        let dx = (cx - (bounds.left() + bounds.right()) / 2.0).round();
        let dy = (cy - (bounds.top() + bounds.bottom()) / 2.0).round();
        let transform = Transform::from_row(scale, 0.0, 0.0, scale, dx, dy);
        for shape in shapes {
            match shape {
                Shape::Stroke(path) => {
                    if let Some(path) = path.transform(transform) {
                        self.stroke(&path, self.color, stroke);
                    }
                }
                Shape::Solid(path) => {
                    if let Some(path) = path.transform(transform) {
                        // A thin round-joined stroke softens the corners the
                        // same way the keycap glyphs are rounded.
                        self.fill(&path, self.color);
                        self.stroke(&path, self.color, 1.4 * size);
                    }
                }
                Shape::Slash(path) => {
                    if let Some(path) = path.transform(transform) {
                        self.stroke(&path, background, stroke * 2.6);
                        self.stroke(&path, self.color, stroke);
                    }
                }
            }
        }
    }

    fn text_width(&self, text: &str, size: f32) -> f32 {
        let scale = size / self.font.units_per_em().unwrap_or(1000.0);
        let mut width = 0.0;
        let mut previous = None;
        for character in text.chars() {
            let id = self.font.glyph_id(character);
            if let Some(previous) = previous {
                width += self.font.kern_unscaled(previous, id) * scale;
            }
            width += self.font.h_advance_unscaled(id) * scale;
            previous = Some(id);
        }
        width
    }

    fn cap_height(&self, size: f32) -> f32 {
        let scale = size / self.font.units_per_em().unwrap_or(1000.0);
        self.font
            .outline(self.font.glyph_id('H'))
            .map_or(size * 0.7, |outline| outline.bounds.height().abs() * scale)
    }

    fn text_width_at(&mut self, text: &str, size: f32, weight: f32) -> f32 {
        self.font.set_variation(b"wght", weight);
        let width = self.text_width(text, size);
        self.font.set_variation(b"wght", FONT_WEIGHT);
        width
    }

    fn center_text(&mut self, cx: f32, cy: f32, text: &str, size: f32, color: Rgb) {
        let x = (cx - self.text_width(text, size) / 2.0).round();
        let baseline = (cy + self.cap_height(size) / 2.0).round();
        let mut path = PathBuilder::new();
        self.append_text(&mut path, x, baseline, text, size, FONT_WEIGHT);
        if let Some(path) = path.finish() {
            self.fill(&path, color);
        }
    }

    /// Appends the glyph outlines of `text` at `weight` and returns the pen
    /// position after the last glyph.
    fn append_text(
        &mut self,
        path: &mut PathBuilder,
        mut x: f32,
        baseline: f32,
        text: &str,
        size: f32,
        weight: f32,
    ) -> f32 {
        self.font.set_variation(b"wght", weight);
        let scale = size / self.font.units_per_em().unwrap_or(1000.0);
        let mut previous = None;
        for character in text.chars() {
            let id = self.font.glyph_id(character);
            if let Some(previous) = previous {
                x += self.font.kern_unscaled(previous, id) * scale;
            }
            if let Some(outline) = self.font.outline(id) {
                let point = |p: ab_glyph::Point| (x + p.x * scale, baseline - p.y * scale);
                let mut last = None;
                for curve in outline.curves {
                    let start = match curve {
                        OutlineCurve::Line(a, _)
                        | OutlineCurve::Quad(a, _, _)
                        | OutlineCurve::Cubic(a, _, _, _) => a,
                    };
                    if last != Some(start) {
                        let (sx, sy) = point(start);
                        path.move_to(sx, sy);
                    }
                    last = Some(match curve {
                        OutlineCurve::Line(_, b) => {
                            let (bx, by) = point(b);
                            path.line_to(bx, by);
                            b
                        }
                        OutlineCurve::Quad(_, c, b) => {
                            let ((cx, cy), (bx, by)) = (point(c), point(b));
                            path.quad_to(cx, cy, bx, by);
                            b
                        }
                        OutlineCurve::Cubic(_, c1, c2, b) => {
                            let ((ax, ay), (cx, cy), (bx, by)) = (point(c1), point(c2), point(b));
                            path.cubic_to(ax, ay, cx, cy, bx, by);
                            b
                        }
                    });
                }
            }
            x += self.font.h_advance_unscaled(id) * scale;
            previous = Some(id);
        }
        self.font.set_variation(b"wght", FONT_WEIGHT);
        x
    }
}

fn load_font() -> Result<FontVec> {
    for path in FONT_PATHS {
        let Ok(data) = fs::read(path) else { continue };
        let mut font = FontVec::try_from_vec(data).with_context(|| format!("parse font {path}"))?;
        font.set_variation(b"wght", FONT_WEIGHT);
        return Ok(font);
    }
    Err(anyhow!(
        "no Touch Bar font found; install adwaita-sans-fonts ({})",
        FONT_PATHS.join(", ")
    ))
}

enum Shape {
    Stroke(Path),
    Solid(Path),
    /// A crossing stroke that cuts a gap into the strokes below it.
    Slash(Path),
}

impl Shape {
    fn path(&self) -> &Path {
        match self {
            Self::Stroke(path) | Self::Solid(path) | Self::Slash(path) => path,
        }
    }

    fn pad(&self) -> f32 {
        match self {
            Self::Stroke(_) | Self::Slash(_) => ICON_STROKE / 2.0,
            Self::Solid(_) => 0.7,
        }
    }
}

fn icon_shapes(icon: Icon) -> Vec<Shape> {
    let mut shapes = Vec::new();
    let build = |build: &dyn Fn(&mut PathBuilder)| {
        let mut path = PathBuilder::new();
        build(&mut path);
        path.finish()
    };
    match icon {
        Icon::Brightness(up) => {
            let (core, inner, outer) = if up {
                (4.6, 8.2, 11.0)
            } else {
                (3.6, 7.0, 8.0)
            };
            if let Some(path) = build(&|path| {
                path.push_circle(0.0, 0.0, core);
                rays(path, 0.0, 0.0, inner, outer, 0.0, 360.0, 8);
            }) {
                shapes.push(Shape::Stroke(path));
            }
        }
        Icon::Keyboard(up) => {
            let (inner, outer) = if up { (7.5, 10.5) } else { (7.0, 8.0) };
            if let Some(path) = build(&|path| {
                arc(path, 0.0, 3.0, 4.2, 180.0, 360.0);
                path.close();
                rays(path, 0.0, 3.0, inner, outer, 180.0, 360.0, 5);
                path.move_to(-6.0, 8.0);
                path.line_to(6.0, 8.0);
            }) {
                shapes.push(Shape::Stroke(path));
            }
        }
        Icon::Microphone => {
            if let Some(path) = build(&|path| {
                path.push_path(&rounded_rect(-3.6, -11.0, 3.6, 2.5, 3.6));
                arc(path, 0.0, -2.0, 7.4, 0.0, 180.0);
                path.move_to(0.0, 5.4);
                path.line_to(0.0, 9.5);
            }) {
                shapes.push(Shape::Stroke(path));
            }
            if let Some(path) = build(&|path| {
                path.move_to(-8.5, -10.0);
                path.line_to(8.5, 9.0);
            }) {
                shapes.push(Shape::Slash(path));
            }
        }
        Icon::Previous | Icon::Next => {
            let side = if matches!(icon, Icon::Next) {
                1.0
            } else {
                -1.0
            };
            if let Some(path) = build(&|path| {
                for offset in [-9.0f32, 0.5] {
                    path.move_to(side * offset, -6.8);
                    path.line_to(side * (offset + 9.5), 0.0);
                    path.line_to(side * offset, 6.8);
                    path.close();
                }
                // The bar at the tips marks a track skip (|◀◀, ▶▶|).
                let (near, far) = (side * 11.2, side * 13.8);
                path.push_path(&rounded_rect(near.min(far), -6.8, near.max(far), 6.8, 0.6));
            }) {
                shapes.push(Shape::Solid(path));
            }
        }
        Icon::PlayPause => {
            if let Some(path) = build(&|path| {
                path.move_to(-11.0, -6.8);
                path.line_to(-1.5, 0.0);
                path.line_to(-11.0, 6.8);
                path.close();
                path.push_path(&rounded_rect(2.5, -6.8, 5.5, 6.8, 0.6));
                path.push_path(&rounded_rect(8.0, -6.8, 11.0, 6.8, 0.6));
            }) {
                shapes.push(Shape::Solid(path));
            }
        }
        Icon::Mute | Icon::Volume(_) => {
            if let Some(path) = build(&|path| {
                path.move_to(-10.0, -3.6);
                path.line_to(-6.0, -3.6);
                path.line_to(-0.8, -8.4);
                path.line_to(-0.8, 8.4);
                path.line_to(-6.0, 3.6);
                path.line_to(-10.0, 3.6);
                path.close();
            }) {
                shapes.push(Shape::Solid(path));
            }
            let waves: &[f32] = match icon {
                Icon::Volume(true) => &[4.6, 8.4, 12.2],
                Icon::Volume(false) => &[4.6],
                _ => &[],
            };
            if let Some(path) = build(&|path| {
                for radius in waves {
                    let sweep = 46.0 - radius * 1.2;
                    arc(path, 0.8, 0.0, *radius, -sweep, sweep);
                }
            }) {
                shapes.push(Shape::Stroke(path));
            }
            if matches!(icon, Icon::Mute)
                && let Some(path) = build(&|path| {
                    path.move_to(-11.0, -9.5);
                    path.line_to(6.0, 9.5);
                })
            {
                shapes.push(Shape::Slash(path));
            }
        }
    }
    shapes
}

/// Appends an open arc. Angles are in degrees, clockwise from +x.
fn arc(path: &mut PathBuilder, cx: f32, cy: f32, radius: f32, from: f32, to: f32) {
    let steps = ((to - from).abs() / 6.0).ceil().max(2.0) as usize;
    for step in 0..=steps {
        let angle = (from + (to - from) * step as f32 / steps as f32).to_radians();
        let (x, y) = (cx + radius * angle.cos(), cy + radius * angle.sin());
        if step == 0 {
            path.move_to(x, y);
        } else {
            path.line_to(x, y);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn rays(
    path: &mut PathBuilder,
    cx: f32,
    cy: f32,
    inner: f32,
    outer: f32,
    from: f32,
    to: f32,
    count: usize,
) {
    let full = (to - from - 360.0).abs() < f32::EPSILON;
    let divisions = if full { count } else { count - 1 };
    for index in 0..count {
        let angle = (from + (to - from) * index as f32 / divisions as f32).to_radians();
        let (sin, cos) = angle.sin_cos();
        path.move_to(cx + inner * cos, cy + inner * sin);
        path.line_to(cx + outer * cos, cy + outer * sin);
    }
}

fn rounded_rect(left: f32, top: f32, right: f32, bottom: f32, radius: f32) -> Path {
    let radius = radius.min((right - left) / 2.0).min((bottom - top) / 2.0);
    let mut path = PathBuilder::new();
    // Cubic quarter circles with the usual 0.5523 control distance.
    let k = radius * (1.0 - 0.552_284_8);
    path.move_to(left + radius, top);
    path.line_to(right - radius, top);
    path.cubic_to(right - k, top, right, top + k, right, top + radius);
    path.line_to(right, bottom - radius);
    path.cubic_to(right, bottom - k, right - k, bottom, right - radius, bottom);
    path.line_to(left + radius, bottom);
    path.cubic_to(left + k, bottom, left, bottom - k, left, bottom - radius);
    path.line_to(left, top + radius);
    path.cubic_to(left, top + k, left + k, top, left + radius, top);
    path.close();
    path.finish()
        .expect("rounded rectangle has a non-empty outline")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layouts_have_no_overlapping_buttons() {
        for mode in [Mode::Media, Mode::Function, Mode::Special] {
            let layout = Layout::new(mode, 2170, false);
            for pair in layout.buttons.windows(2) {
                assert!(pair[0].right < pair[1].left);
            }
        }
    }

    #[test]
    fn esc_and_row_edges_stay_put_across_modes() {
        let media = Layout::new(Mode::Media, 2170, false);
        let function = Layout::new(Mode::Function, 2170, false);
        let edges = |layout: &Layout| {
            let first = layout.buttons[0];
            let last = layout.buttons[layout.buttons.len() - 1];
            (first.left, first.right, layout.buttons[1].left, last.right)
        };
        assert_eq!(edges(&media), edges(&function));
        // The special row keeps esc and the right edge, and the media width.
        let special = Layout::new(Mode::Special, 2170, false);
        let (esc, last) = (
            special.buttons[0],
            special.buttons[special.buttons.len() - 1],
        );
        let media_last = media.buttons[media.buttons.len() - 1];
        assert_eq!(
            (esc.left, esc.right),
            (media.buttons[0].left, media.buttons[0].right)
        );
        assert_eq!(last.right, media_last.right);
        assert_eq!(last.right - last.left, media_last.right - media_last.left);
    }

    #[test]
    fn personal_keys_fill_only_the_free_space() {
        let labels = ["~", "|", "a", "b", "c", "d", "e"];
        let special = Layout::with_custom(Mode::Special, 2170, false, &labels);
        let custom: Vec<_> = special
            .buttons
            .iter()
            .filter(|button| matches!(button.action, Action::Custom(_)))
            .collect();
        assert_eq!(custom.len(), 4, "four keys fit between esc and print");
        assert_eq!(custom[0].action, Action::Custom(0));
        assert!(custom[0].left > special.buttons[0].right + GROUP_GAP - 1);
        for pair in special.buttons.windows(2) {
            assert!(pair[0].right < pair[1].left);
        }
    }

    #[test]
    fn app_keys_follow_esc_with_the_media_width() {
        let media = Layout::new(Mode::Media, 2170, false);
        let last = media.buttons[media.buttons.len() - 1];
        let media_cell = last.right - last.left;
        let labels: Vec<&'static str> = vec!["k"; 40];
        let app = Layout::with_custom(Mode::App, 2170, false, &labels);
        assert_eq!(app.buttons[0].action, Action::Key(Key::Esc));
        assert_eq!(app.buttons[1].action, Action::App(0));
        assert!(app.buttons.len() < labels.len(), "only as many as fit");
        for button in &app.buttons[1..] {
            assert_eq!(button.right - button.left, media_cell);
            assert!(button.right <= 2170);
        }
        for pair in app.buttons.windows(2) {
            assert!(pair[0].right < pair[1].left);
        }
        // Without a profile the row holds only esc.
        assert_eq!(Layout::new(Mode::App, 2170, false).buttons.len(), 1);
    }

    #[test]
    fn touch_id_draws_only_near_the_sensor_and_label() {
        let mut canvas = Canvas::new(2170, 60, 0x00dce6ff).unwrap();
        canvas.touch_id("waiting", 0.5, None, false);
        assert!(canvas.pixels.iter().any(|pixel| *pixel != 0));
        assert_eq!(canvas.pixels[0], 0);
    }

    /// Renders both layers and the Touch ID prompt for design review:
    /// `KAIT2EN_TOUCHBAR_PREVIEW=/tmp/tb cargo test preview -- --ignored`
    #[test]
    #[ignore]
    fn preview() {
        let dir = std::env::var("KAIT2EN_TOUCHBAR_PREVIEW").unwrap_or_else(|_| ".".into());
        let mut canvas = Canvas::new(2170, 60, 0x00dce6ff).unwrap();
        let save = |name: &str, canvas: &Canvas| {
            let mut ppm = format!("P6 {} {} 255\n", canvas.width, canvas.height).into_bytes();
            for pixel in &canvas.pixels {
                ppm.extend([(pixel >> 16) as u8, (pixel >> 8) as u8, *pixel as u8]);
            }
            fs::write(format!("{dir}/{name}.ppm"), ppm).unwrap();
        };
        canvas.keys(&Layout::new(Mode::Media, 2170, false), None);
        save("media", &canvas);
        canvas.keys(&Layout::new(Mode::Function, 2170, false), None);
        save("function", &canvas);
        canvas.keys(
            &Layout::with_custom(Mode::Special, 2170, false, &["~", "|", "term"]),
            None,
        );
        save("special", &canvas);
        canvas.touch_id("waiting", 0.0, None, false);
        save("touchid", &canvas);
        canvas.touch_id("waiting", 1.0, None, false);
        save("touchid-nudged", &canvas);
        let media = Layout::new(Mode::Media, 2170, false);
        let keys = TrackLayout {
            prev: (media.buttons[1].left, media.buttons[1].right),
            next: (2170 - media.buttons[1].right, 2170 - media.buttons[1].left),
        };
        canvas.track("Slipknot", "Duality", 100, None, keys, None);
        save("track", &canvas);
        canvas.track(
            "Slipknot",
            "Duality",
            100,
            None,
            keys,
            Some((media.buttons[0], 100)),
        );
        save("track-esc", &canvas);
        canvas.track(
            "Heino",
            "Blau blüht der Enzian (Live aus der Festhalle mit Orchester und sehr langem Titel, der gekürzt werden muss, weil er sonst nicht passt)",
            60,
            Some(true),
            keys,
            None,
        );
        save("track-long", &canvas);
        canvas.touch_id("retry", 0.0, None, false);
        save("touchid-retry", &canvas);
        canvas.level(LevelKind::Volume, Some(45), false, (560.0, 640.0), 1.0, 100);
        save("volume", &canvas);
        canvas.level(
            LevelKind::Brightness,
            Some(80),
            false,
            (1620.0, 1780.0),
            0.0,
            100,
        );
        save("brightness", &canvas);
        canvas.level(
            LevelKind::Volume,
            Some(45),
            false,
            (1000.0, 1100.0),
            0.3,
            100,
        );
        save("volume-crossfade", &canvas);
    }
}
