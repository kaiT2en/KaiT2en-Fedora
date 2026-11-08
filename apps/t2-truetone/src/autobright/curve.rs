// SPDX-License-Identifier: GPL-3.0-or-later
//! The brightness model macOS uses on T2 Intel Macs.
//!
//! Brightness is linear: the fraction of the panel's maximum luminance. The
//! curve is a straight line through two anchors and learns from every manual
//! adjustment by turning around the point the user picked.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

/// Factory anchors as `DisplayALSManager` picks them from the model name:
/// MacBookPro from 13,1 on, MacBookAir from 8,0 on, everything else keeps the
/// initial values. The two MacBook pairs are the same 53.75 / 170.9 nits on
/// 500 and 400 nit panels. All share the bright anchor at 262.5 lux.
const FACTORY_PRO: (f32, f32) = (0.107_505, 0.341_785);
const FACTORY_AIR: (f32, f32) = (0.134_381_25, 0.427_231_25);
const FACTORY_OTHER: (f32, f32) = (0.14, 0.615);
const FACTORY_LUX_HIGH: f32 = 262.5;
const MAX_LUX: f32 = 5000.0;

/// Bounds for the slope after learning, so the line stays between nearly flat
/// and steep. Both have a floor of their own.
const MIN_SLOPE_FLOOR: f32 = 0.000_353_857_03;
const MAX_SLOPE_FLOOR: f32 = 0.003_184_713_4;

/// `log2(500)`: the perceptual scale spans 500:1.
const LOG2_RANGE: f32 = 8.965_784;

/// Ramp duration in seconds against the perceptual step. Dimming always takes
/// 5 s, brightening takes 5 s for small steps and 1.5 s for large ones.
const RAMP_TABLE: [(f32, f32); 11] = [
    (-1.0, 5.0),
    (-0.3, 5.0),
    (-0.2, 5.0),
    (-0.1, 5.0),
    (-0.04, 5.0),
    (0.0, 5.0),
    (0.04, 5.0),
    (0.1, 1.5),
    (0.2, 1.5),
    (0.3, 1.5),
    (1.0, 1.5),
];

#[derive(Clone, Debug, PartialEq)]
pub struct Curve {
    pub e1: f32,
    pub e2: f32,
    pub bl1: f32,
    pub bl2: f32,
    /// Lux of the last adjustment on the dim side.
    pub lux_low: f32,
    /// Lux of the last adjustment on the bright side.
    pub lux_high: f32,
    /// Lux that separates the dim side from the bright side.
    pub threshold: f32,
}

impl Curve {
    /// `product` is the DMI product name, such as `MacBookPro16,1`.
    pub fn factory(product: &str) -> Self {
        let version = |prefix: &str| -> Option<f32> {
            product.trim().strip_prefix(prefix)?.replace(',', ".").parse().ok()
        };
        let (bl1, bl2) = if version("MacBookPro").is_some_and(|v| v >= 13.1) {
            FACTORY_PRO
        } else if version("MacBookAir").is_some_and(|v| v >= 8.0) {
            FACTORY_AIR
        } else {
            FACTORY_OTHER
        };
        let mut curve = Curve {
            e1: 0.0,
            e2: FACTORY_LUX_HIGH,
            bl1,
            bl2,
            lux_low: 0.0,
            lux_high: FACTORY_LUX_HIGH,
            threshold: 0.0,
        };
        curve.update_threshold();
        curve
    }

    fn slope(&self) -> f32 {
        (self.bl2 - self.bl1) / (self.e2 - self.e1)
    }

    pub fn linear_for_lux(&self, lux: f32, min: f32, max: f32) -> f32 {
        let lux = lux.min(MAX_LUX);
        (self.bl1 + self.slope() * lux).clamp(min, max)
    }

    /// Makes the line pass through `(lux, linear)`. On the dim side the bright
    /// anchor stays, on the bright side the dark anchor stays.
    pub fn learn(&mut self, lux: f32, linear: f32) {
        let lux = if lux <= 0.0 { 0.01 } else { lux.min(MAX_LUX) };
        let min_slope = (2.0 * linear / (2.0 * lux + 500.0)).max(MIN_SLOPE_FLOOR);
        let max_slope = (71.0 * linear / (71.0 * lux + 500.0)).max(MAX_SLOPE_FLOOR);
        let bound = |slope: f32| {
            if slope.is_nan() { min_slope } else { slope.min(max_slope).max(min_slope) }
        };
        if lux <= self.threshold {
            self.lux_low = lux;
            let slope = bound((self.bl2 - linear) / (self.e2 - lux));
            self.bl1 = slope * (self.e1 - lux) + linear;
            self.bl2 = slope * (self.e2 - lux) + linear;
        } else {
            self.lux_high = lux;
            let slope = bound((linear - self.bl1) / (lux - self.e1));
            self.e2 = lux.max(150.0);
            self.bl2 = if lux < 150.0 { linear + (150.0 - lux) * slope } else { linear };
            self.bl1 = (self.e1 - self.e2) * slope + self.bl2;
        }
        self.update_threshold();
    }

    fn update_threshold(&mut self) {
        let slope = self.slope();
        let crossing = (self.bl1 - slope * self.e1) / slope;
        let mut threshold = if crossing <= 0.0 { (self.e1 + self.e2) * 0.5 } else { crossing };
        threshold = threshold.min(500.0);
        if threshold > self.lux_high / 1.2 {
            threshold = self.lux_high / 1.2;
        }
        if threshold < 20.0 {
            threshold = 20.0;
        }
        if threshold < self.lux_low * 1.2 {
            threshold = self.lux_low * 1.2;
        }
        self.threshold = threshold;
    }

    pub fn load(path: &Path) -> Option<Self> {
        let text = fs::read_to_string(path).ok()?;
        let value = |key: &str| -> Option<f32> {
            text.lines()
                .filter_map(|line| line.split_once('='))
                .find(|(name, _)| name.trim() == key)
                .and_then(|(_, raw)| raw.trim().parse().ok())
        };
        let mut curve = Curve {
            e1: value("e1")?,
            e2: value("e2")?,
            bl1: value("bl1")?,
            bl2: value("bl2")?,
            lux_low: value("lux_low")?,
            lux_high: value("lux_high")?,
            threshold: 0.0,
        };
        if curve.e2 <= curve.e1 {
            return None;
        }
        curve.update_threshold();
        Some(curve)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let text = format!(
            "e1={}\ne2={}\nbl1={}\nbl2={}\nlux_low={}\nlux_high={}\n",
            self.e1, self.e2, self.bl1, self.bl2, self.lux_low, self.lux_high
        );
        fs::write(path, text).with_context(|| format!("writing {}", path.display()))
    }
}

pub fn perceptual(linear: f32) -> f32 {
    if linear <= 0.0 { 0.0 } else { 1.0 + linear.log2() / LOG2_RANGE }
}

pub fn linear(perceptual: f32) -> f32 {
    500f32.powf(perceptual - 1.0)
}

pub fn ramp_seconds(delta: f32) -> f32 {
    if delta <= RAMP_TABLE[0].0 {
        return RAMP_TABLE[0].1;
    }
    if delta >= RAMP_TABLE[RAMP_TABLE.len() - 1].0 {
        return RAMP_TABLE[RAMP_TABLE.len() - 1].1;
    }
    let upper = RAMP_TABLE.iter().position(|&(step, _)| delta <= step).unwrap_or(1).max(1);
    let (p0, t0) = RAMP_TABLE[upper - 1];
    let (p1, t1) = RAMP_TABLE[upper];
    t0 + (t1 - t0) * (delta - p0) / (p1 - p0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32, tolerance: f32) -> bool {
        (a - b).abs() <= tolerance
    }

    #[test]
    fn factory_follows_the_model_branches() {
        let pro = Curve::factory("MacBookPro16,1");
        assert!(close(pro.bl1, 0.107_505, 1e-6) && close(pro.bl2, 0.341_785, 1e-6));
        let air = Curve::factory("MacBookAir9,1");
        assert!(close(air.bl1, 0.134_381, 1e-6) && close(air.bl2, 0.427_231, 1e-6));
        for other in ["iMac20,1", "iMacPro1,1", "MacBookPro12,1", "MacBookAir7,2", ""] {
            let curve = Curve::factory(other);
            assert!(close(curve.bl1, 0.14, 1e-6) && close(curve.bl2, 0.615, 1e-6), "{other}");
        }
        assert!(close(Curve::factory("MacBookPro13,1\n").bl1, 0.107_505, 1e-6));
    }

    /// The MacBookPro16,1 under test reports E2 473, BL2 0.38316 and an
    /// unchanged BL1, which is one adjustment on the bright side at 473 lux.
    #[test]
    fn learning_reproduces_the_live_16_1_curve() {
        let mut curve = Curve::factory("MacBookPro16,1");
        curve.learn(473.0, 0.383_159_9);
        assert!(close(curve.e2, 473.0, 1e-3));
        assert!(close(curve.bl2, 0.383_159_9, 1e-6));
        assert!(close(curve.bl1, 0.107_505, 1e-5));
        assert!(close(curve.threshold, 184.469, 0.05));
    }

    /// Targets logged by DisplayALSManager on the 16,1 for floor(lux).
    #[test]
    fn targets_match_the_live_trace() {
        let mut curve = Curve::factory("MacBookPro16,1");
        curve.learn(473.0, 0.383_159_9);
        for (lux, target) in [(131.0, 0.183_849), (5.0, 0.110_419), (1.0, 0.108_088), (41.0, 0.131_399)] {
            assert!(close(curve.linear_for_lux(lux, 0.011, 1.0), target, 2e-5), "{lux}");
        }
    }

    #[test]
    fn perceptual_round_trip() {
        for value in [0.002, 0.05, 0.3, 1.0] {
            assert!(close(linear(perceptual(value)), value, 1e-5));
        }
        assert!(close(perceptual(1.0), 1.0, 1e-6));
    }

    #[test]
    fn ramp_table_edges() {
        assert_eq!(ramp_seconds(-2.0), 5.0);
        assert_eq!(ramp_seconds(-0.09), 5.0);
        assert_eq!(ramp_seconds(0.2), 1.5);
        assert_eq!(ramp_seconds(3.0), 1.5);
        assert!(close(ramp_seconds(0.07), 3.25, 1e-4));
    }

    #[test]
    fn dim_side_keeps_the_bright_anchor() {
        let mut curve = Curve::factory("MacBookPro16,1");
        let (e2, bl2) = (curve.e2, curve.bl2);
        curve.learn(10.0, 0.15);
        assert!(close(curve.linear_for_lux(10.0, 0.0, 1.0), 0.15, 1e-5));
        assert!(close(curve.e2, e2, 1e-6));
        assert!(close(curve.linear_for_lux(e2, 0.0, 1.0), bl2, 1e-3));
    }
}
