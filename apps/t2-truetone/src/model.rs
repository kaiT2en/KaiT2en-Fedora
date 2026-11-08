// SPDX-License-Identifier: GPL-3.0-or-later
//! Ambient white point adaptation.
//!
//! Per sensor sample (lux, x, y, CCT):
//! 1. bright clamp, at 35000 lx and above the target is D65
//! 2. low light, below 10 lx the target is a fixed warm white point, between
//!    10 and 15 lx the measured chromaticity is blended toward it
//! 3. Planckian restriction, chromaticities below the Planckian locus (magenta
//!    side) are moved onto it
//! 4. CIELAB with a D65 white, ab limited along the ray from neutral to an
//!    ellipse
//! 5. strength, ab is scaled down by a smooth hue and chroma dependent factor
//! 6. von Kries adaptation (Bradford) in linear sRGB from D65 to the target


pub type Xy = [f64; 2];
pub type Vec3 = [f64; 3];
pub type Mat3 = [[f64; 3]; 3];

pub const D65: Xy = [0.312716, 0.329001];
pub const WARM: Xy = [0.382661, 0.384643];
const LAB_WHITE: Vec3 = [95.05, 100.0, 108.9];

const LUX_BRIGHT: f64 = 35000.0;
const LUX_LOW: f64 = 10.0;
const LUX_HIGH: f64 = 15.0;
const LUX_COLD_GATE: f64 = 2.0;
const CCT_COLD: f64 = 6000.0;

const ELLIPSE_CENTER: [f64; 2] = [3.117, 11.655];
const ELLIPSE_AXES: [f64; 2] = [36.332, 16.077];
const ELLIPSE_ANGLE_DEG: f64 = -101.85;

pub const T_MIN: f64 = 1000.0;
pub const T_MAX: f64 = 15000.0;

/// Strength surfaces f(C, h) = sum_k (C/40)^k (c0 + c1 cos h + c2 sin h + c3 cos 2h + c4 sin 2h)
const SURF_SCALE: f64 = 40.0;
const SURF_1: [f64; 10] = [
    0.04406636583347863, 0.007735283363718516, -0.13690758463674238, -0.11276389867857102,
    0.06421907882858718, 0.2797082421352734, -0.07316656665790894, -0.015841294312432523,
    0.2000951275786206, -0.14616751992673194,
];
const SURF_2: [f64; 10] = [
    0.5432280947030717, 0.5657884462395486, -0.491216618776706, -0.08494555700963291,
    0.060031900939200036, 0.3285744049625581, -0.6326289950245043, 0.29424332042563084,
    0.36403781809346136, -0.1209553579832101,
];

pub const DEFAULT_STRENGTH: f64 = 0.2;

const BRADFORD: Mat3 = [
    [0.8951, 0.2664, -0.1614],
    [-0.7502, 1.7135, 0.0367],
    [0.0389, -0.0685, 1.0296],
];
const SRGB_TO_XYZ: Mat3 = [
    [0.412391, 0.357584, 0.180481],
    [0.212639, 0.715169, 0.0721923],
    [0.0193308, 0.119195, 0.950532],
];

pub fn mul(a: &Mat3, b: &Mat3) -> Mat3 {
    let mut r = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            r[i][j] = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    r
}

pub fn apply(a: &Mat3, v: &Vec3) -> Vec3 {
    [0, 1, 2].map(|i| a[i][0] * v[0] + a[i][1] * v[1] + a[i][2] * v[2])
}

pub fn inverse(m: &Mat3) -> Mat3 {
    let [[a, b, c], [d, e, f], [g, h, i]] = *m;
    let det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    [
        [(e * i - f * h) / det, (c * h - b * i) / det, (b * f - c * e) / det],
        [(f * g - d * i) / det, (a * i - c * g) / det, (c * d - a * f) / det],
        [(d * h - e * g) / det, (b * g - a * h) / det, (a * e - b * d) / det],
    ]
}

pub fn xy_to_xyz(xy: Xy, y_lum: f64) -> Vec3 {
    [xy[0] / xy[1] * y_lum, y_lum, (1.0 - xy[0] - xy[1]) / xy[1] * y_lum]
}

pub fn xyz_to_xy(v: &Vec3) -> Xy {
    let s = v[0] + v[1] + v[2];
    [v[0] / s, v[1] / s]
}

/// McCamy 1992 CCT approximation.
pub fn mccamy(xy: Xy) -> f64 {
    let n = (xy[0] - 0.3320) / (0.1858 - xy[1]);
    449.0 * n.powi(3) + 3525.0 * n * n + 6823.3 * n + 5520.33
}

/// Planckian locus, Krystek 1985 rational approximation (1000 to 15000 K).
pub fn planck_xy(t: f64) -> Xy {
    let u = (0.860117757 + 1.54118254e-4 * t + 1.28641212e-7 * t * t)
        / (1.0 + 8.42420235e-4 * t + 7.08145163e-7 * t * t);
    let v = (0.317398726 + 4.22806245e-5 * t + 4.20481691e-8 * t * t)
        / (1.0 - 2.89741816e-5 * t + 1.61456053e-7 * t * t);
    let d = 2.0 * u - 8.0 * v + 4.0;
    [3.0 * u / d, 2.0 * v / d]
}

pub struct Locus {
    pts: Vec<Xy>,
}

impl Locus {
    pub fn new() -> Self {
        let n = 400;
        let pts = (0..n)
            .map(|i| {
                let t = T_MIN * (T_MAX / T_MIN).powf(i as f64 / (n - 1) as f64);
                planck_xy(t)
            })
            .collect();
        Locus { pts }
    }

    /// Locus y at abscissa x, linearly extended beyond both ends.
    fn y_at(&self, x: f64) -> f64 {
        let p = &self.pts;
        let (a, b) = if x >= p[0][0] {
            (p[0], p[1])
        } else if x <= p[p.len() - 1][0] {
            (p[p.len() - 2], p[p.len() - 1])
        } else {
            let i = p.iter().position(|q| q[0] < x).unwrap_or(p.len() - 1);
            (p[i - 1], p[i])
        };
        a[1] + (x - a[0]) * (b[1] - a[1]) / (b[0] - a[0])
    }

    /// Move xy onto the locus if it lies below it.
    pub fn restrict(&self, xy: Xy) -> Xy {
        if xy[1] >= self.y_at(xy[0]) {
            return xy;
        }
        let mut best = xy;
        let mut bd = f64::MAX;
        for w in self.pts.windows(2) {
            let (p, q) = (w[0], w[1]);
            let d = [q[0] - p[0], q[1] - p[1]];
            let t = (((xy[0] - p[0]) * d[0] + (xy[1] - p[1]) * d[1]) / (d[0] * d[0] + d[1] * d[1]))
                .clamp(0.0, 1.0);
            let c = [p[0] + t * d[0], p[1] + t * d[1]];
            let dist = (xy[0] - c[0]).hypot(xy[1] - c[1]);
            if dist < bd {
                bd = dist;
                best = c;
            }
        }
        best
    }
}

impl Default for Locus {
    fn default() -> Self {
        Self::new()
    }
}

pub fn lab(v: &Vec3) -> Vec3 {
    let f = |t: f64| if t > 0.0088565 { t.cbrt() } else { 7.787 * t + 0.13793 };
    let fx = f(v[0] / LAB_WHITE[0]);
    let fy = f(v[1] / LAB_WHITE[1]);
    let fz = f(v[2] / LAB_WHITE[2]);
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

pub fn lab_to_xyz(l: &Vec3) -> Vec3 {
    let fy = (l[0] + 16.0) / 116.0;
    let fx = fy + l[1] / 500.0;
    let fz = fy - l[2] / 200.0;
    let g = |t: f64| if t.powi(3) > 0.0088565 { t.powi(3) } else { (t - 0.13793) / 7.787 };
    [g(fx) * LAB_WHITE[0], g(fy) * LAB_WHITE[1], g(fz) * LAB_WHITE[2]]
}

/// Limit ab along the ray from neutral, hue is kept.
pub fn ellipse_limit(ab: [f64; 2]) -> [f64; 2] {
    let r = ab[0].hypot(ab[1]);
    if r == 0.0 {
        return ab;
    }
    let (s, c) = ELLIPSE_ANGLE_DEG.to_radians().sin_cos();
    let rot = |v: [f64; 2]| [c * v[0] + s * v[1], -s * v[0] + c * v[1]];
    let o = rot([-ELLIPSE_CENTER[0], -ELLIPSE_CENTER[1]]);
    let p0 = [o[0] / ELLIPSE_AXES[0], o[1] / ELLIPSE_AXES[1]];
    let dr = rot([ab[0] / r, ab[1] / r]);
    let d = [dr[0] / ELLIPSE_AXES[0], dr[1] / ELLIPSE_AXES[1]];
    let qa = d[0] * d[0] + d[1] * d[1];
    let qb = 2.0 * (p0[0] * d[0] + p0[1] * d[1]);
    let qc = p0[0] * p0[0] + p0[1] * p0[1] - 1.0;
    let t = (-qb + (qb * qb - 4.0 * qa * qc).sqrt()) / (2.0 * qa);
    if r <= t { ab } else { [ab[0] / r * t, ab[1] / r * t] }
}

fn surface(c: &[f64; 10], a: f64, b: f64) -> f64 {
    let chroma = a.hypot(b);
    let h = b.atan2(a);
    let mut acc = 0.0;
    for k in 0..2 {
        let r = (chroma / SURF_SCALE).powi(k as i32 + 1);
        let o = 5 * k;
        acc += r
            * (c[o] + c[o + 1] * h.cos() + c[o + 2] * h.sin() + c[o + 3] * (2.0 * h).cos()
                + c[o + 4] * (2.0 * h).sin());
    }
    acc
}

/// Fraction of ab removed at strength s (0 = no adaptation, 1 = full).
pub fn reduction(a: f64, b: f64, s: f64) -> f64 {
    let f1 = surface(&SURF_1, a, b);
    let f2 = surface(&SURF_2, a, b);
    if s <= 0.35 {
        let u = (0.35 - s) / 0.35;
        (1.0 - u) * f2 + u
    } else if s <= 0.7 {
        let u = (0.7 - s) / 0.35;
        u * f2 + (1.0 - u) * f1
    } else if s <= 0.85 {
        f1 * (0.85 - s) / 0.15
    } else {
        0.0
    }
}

/// Restricted ambient chromaticity (steps 1 to 3).
pub fn ambient_xy(locus: &Locus, lux: f64, xy: Xy, cct: f64) -> Xy {
    if lux >= LUX_BRIGHT {
        return D65;
    }
    let cct1 = if xy[1] >= 0.1958 { mccamy(xy) } else { cct };
    let fixed = if cct1 >= CCT_COLD && lux > LUX_COLD_GATE { D65 } else { WARM };
    let p = if lux <= LUX_LOW {
        fixed
    } else if lux < LUX_HIGH {
        let al = (LUX_HIGH - lux) / (LUX_HIGH - LUX_LOW);
        [al * fixed[0] + (1.0 - al) * xy[0], al * fixed[1] + (1.0 - al) * xy[1]]
    } else {
        xy
    };
    locus.restrict(p)
}

/// Target in Lab for a sensor sample (steps 1 to 4).
pub fn ambient_lab(locus: &Locus, lux: f64, xy: Xy, cct: f64) -> Vec3 {
    let l = lab(&xy_to_xyz(ambient_xy(locus, lux, xy, cct), 100.0));
    let ab = ellipse_limit([l[1], l[2]]);
    [l[0], ab[0], ab[1]]
}

/// Lab target after the strength step (step 5).
pub fn adapted_lab(l: &Vec3, strength: f64) -> Vec3 {
    let f = reduction(l[1], l[2], strength);
    [l[0], (1.0 - f) * l[1], (1.0 - f) * l[2]]
}

/// Linear sRGB matrix adapting D65 to the target white (step 6), max row sum 1.
pub fn adaptation_matrix(target_xyz: &Vec3) -> Mat3 {
    let a = mul(&BRADFORD, &SRGB_TO_XYZ);
    let ai = inverse(&a);
    let src = apply(&BRADFORD, &xy_to_xyz(D65, 1.0));
    let t = [target_xyz[0] / target_xyz[1], 1.0, target_xyz[2] / target_xyz[1]];
    let dst = apply(&BRADFORD, &t);
    let mut d = [[0.0; 3]; 3];
    for i in 0..3 {
        d[i][i] = dst[i] / src[i];
    }
    let m = mul(&ai, &mul(&d, &a));
    let maxrow = m.iter().map(|r| r.iter().sum::<f64>()).fold(f64::MIN, f64::max);
    m.map(|r| r.map(|v| v / maxrow))
}

/// Per channel gains of linear RGB that map D65 white onto the target white.
pub fn white_gains(target_xyz: &Vec3) -> Vec3 {
    let m = adaptation_matrix(target_xyz);
    [0, 1, 2].map(|i| m[i].iter().sum::<f64>())
}

/// Lab of a Planckian white at temperature t.
pub fn planck_lab(t: f64) -> Vec3 {
    lab(&xy_to_xyz(planck_xy(t.clamp(T_MIN, T_MAX)), 100.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warm_fixed_point_matches_macos() {
        // macOS: target Lab ab (7.65, 35.59) at strength 0.2 gave diag 0.9149 0.8466 0.7023
        let l = [100.0, 7.65, 35.59];
        let m = adaptation_matrix(&lab_to_xyz(&adapted_lab(&l, 0.2)));
        assert!((m[0][0] - 0.9149).abs() < 0.005, "{:?}", m);
        assert!((m[1][1] - 0.8466).abs() < 0.005, "{:?}", m);
        assert!((m[2][2] - 0.7023).abs() < 0.005, "{:?}", m);
    }

    #[test]
    fn d65_is_identity() {
        let g = white_gains(&xy_to_xyz(D65, 1.0));
        for v in g {
            assert!((v - 1.0).abs() < 1e-6);
        }
    }

    #[test]
    fn continuous_over_lux() {
        let locus = Locus::new();
        let xy = planck_xy(3000.0);
        let mut prev: Option<Vec3> = None;
        let mut lux = 0.0;
        while lux < 40.0 {
            let g = white_gains(&lab_to_xyz(&adapted_lab(&ambient_lab(&locus, lux, xy, 3000.0), 0.2)));
            if let Some(p) = prev {
                for i in 0..3 {
                    assert!((g[i] - p[i]).abs() < 1e-3);
                }
            }
            prev = Some(g);
            lux += 0.01;
        }
    }
}
