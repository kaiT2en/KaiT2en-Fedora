// SPDX-License-Identifier: GPL-3.0-or-later
//! Evening mode: a warmer Planckian white on a daily schedule with smooth ramps.

use crate::config::{Config, parse_hhmm};

/// Local time in minutes after midnight.
pub fn local_minutes() -> f64 {
    let now = unsafe { libc::time(std::ptr::null_mut()) };
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&now, &mut tm) };
    tm.tm_hour as f64 * 60.0 + tm.tm_min as f64 + tm.tm_sec as f64 / 60.0
}

fn smoothstep(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Minutes from a to b going forward around the clock.
fn forward(a: f64, b: f64) -> f64 {
    (b - a).rem_euclid(1440.0)
}

/// 0.0 (day) to 1.0 (full evening mode). Fades in over the ramp starting at
/// `night_from` and fades out over the ramp ending at `night_to`.
pub fn factor(c: &Config, now_min: f64) -> f64 {
    if !c.night {
        return 0.0;
    }
    let (Some(from), Some(to)) = (parse_hhmm(&c.night_from), parse_hhmm(&c.night_to)) else {
        return 0.0;
    };
    let span = forward(from, to);
    let pos = forward(from, now_min);
    if span == 0.0 || pos >= span {
        return 0.0;
    }
    let ramp = c.night_ramp_min.min(span / 2.0);
    if ramp <= 0.0 {
        return 1.0;
    }
    smoothstep(pos / ramp).min(smoothstep((span - pos) / ramp))
}

/// Color temperature for a given factor, interpolated in mired from D65.
pub fn kelvin(c: &Config, f: f64) -> f64 {
    let day = 1e6 / 6504.0;
    let night = 1e6 / c.night_kelvin;
    1e6 / (day + (night - day) * f)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedule_wraps_midnight_and_is_smooth() {
        let c = Config { night: true, ..Config::default() };
        assert_eq!(factor(&c, 20.0 * 60.0), 0.0);
        assert!((factor(&c, 23.0 * 60.0) - 1.0).abs() < 1e-12);
        assert!((factor(&c, 2.0 * 60.0) - 1.0).abs() < 1e-12);
        assert_eq!(factor(&c, 8.0 * 60.0), 0.0);
        // smoothstep rises at most 1.5 per ramp length
        let step = 0.1;
        let bound = 1.5 * step / c.night_ramp_min * 1.01;
        let mut prev = factor(&c, 0.0);
        let mut m = 0.0;
        while m < 1440.0 {
            let f = factor(&c, m);
            assert!((f - prev).abs() <= bound, "jump at {m}");
            prev = f;
            m += 0.1;
        }
    }
}
