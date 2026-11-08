// SPDX-License-Identifier: GPL-3.0-or-later
//! Gamma LUT built from per channel linear gains on top of mutter's own LUT.
//!
//! mutter writes the panel LUT from the color profile whenever its color
//! device updates. That LUT has the hardware size. Ours has LUT_LEN entries,
//! mutter resamples it to the hardware size. A LUT of any other length than
//! LUT_LEN is therefore mutter's and becomes the new base.

use std::fs;
use std::path::PathBuf;

pub const LUT_LEN: usize = 1024;

pub type Base = [Vec<f64>; 3];

fn srgb_decode(v: f64) -> f64 {
    if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

fn srgb_encode(v: f64) -> f64 {
    if v <= 0.0031308 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
}

pub fn identity() -> Base {
    let ch: Vec<f64> = (0..LUT_LEN).map(|i| i as f64 / (LUT_LEN - 1) as f64).collect();
    [ch.clone(), ch.clone(), ch]
}

/// Resample a mutter LUT (any length) to LUT_LEN normalized entries.
pub fn base_from(lut: &[Vec<u16>; 3]) -> Base {
    lut.clone().map(|ch| {
        if ch.len() < 2 {
            return identity()[0].clone();
        }
        (0..LUT_LEN)
            .map(|i| {
                let p = i as f64 / (LUT_LEN - 1) as f64 * (ch.len() - 1) as f64;
                let j = (p.floor() as usize).min(ch.len() - 2);
                let t = p - j as f64;
                (ch[j] as f64 * (1.0 - t) + ch[j + 1] as f64 * t) / 65535.0
            })
            .collect()
    })
}

fn sample(base: &[f64], v: f64) -> f64 {
    let p = v.clamp(0.0, 1.0) * (base.len() - 1) as f64;
    let j = (p.floor() as usize).min(base.len() - 2);
    let t = p - j as f64;
    base[j] * (1.0 - t) + base[j + 1] * t
}

/// LUT for linear channel gains (each in 0..=1) composed with the base.
pub fn build(base: &Base, gains: [f64; 3]) -> [Vec<u16>; 3] {
    [0, 1, 2].map(|c| {
        (0..LUT_LEN)
            .map(|i| {
                let v = i as f64 / (LUT_LEN - 1) as f64;
                let out = srgb_encode(srgb_decode(v) * gains[c].clamp(0.0, 1.0));
                (sample(&base[c], out) * 65535.0).round().clamp(0.0, 65535.0) as u16
            })
            .collect()
    })
}

fn base_path() -> Option<PathBuf> {
    let dir = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
    Some(dir.join("t2-truetone").join("base-lut"))
}

/// Kept so that a restart after a crash, with our LUT still on the panel,
/// knows what to fade back to.
pub fn save_base(base: &Base) {
    let Some(p) = base_path() else { return };
    let text: String = base
        .iter()
        .map(|ch| ch.iter().map(|v| format!("{v:.6}")).collect::<Vec<_>>().join(" ") + "\n")
        .collect();
    let res = p.parent().map_or(Ok(()), fs::create_dir_all).and_then(|_| fs::write(&p, text));
    if let Err(e) = res {
        eprintln!("t2-truetone: could not save {}: {e}", p.display());
    }
}

pub fn load_base() -> Option<Base> {
    let text = fs::read_to_string(base_path()?).ok()?;
    let chans: Vec<Vec<f64>> = text
        .lines()
        .map(|l| l.split_whitespace().filter_map(|v| v.parse().ok()).collect())
        .collect();
    if chans.len() == 3 && chans.iter().all(|c| c.len() == LUT_LEN) {
        Some([chans[0].clone(), chans[1].clone(), chans[2].clone()])
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unity_gains_reproduce_base() {
        let b = identity();
        let l = build(&b, [1.0, 1.0, 1.0]);
        for (i, v) in l[0].iter().enumerate() {
            let want = (i as f64 / (LUT_LEN - 1) as f64 * 65535.0).round() as i64;
            assert!((*v as i64 - want).abs() <= 1);
        }
    }

    #[test]
    fn white_scaled_by_gain() {
        let l = build(&identity(), [1.0, 0.5, 0.25]);
        let top = |c: usize| srgb_decode(l[c][LUT_LEN - 1] as f64 / 65535.0);
        assert!((top(1) - 0.5).abs() < 1e-3);
        assert!((top(2) - 0.25).abs() < 1e-3);
    }
}
