// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>

/// Horizontal multi-finger swipe, quantized into fixed-size steps.
#[derive(Clone, Copy, Debug)]
pub struct Swipe {
    pub kind: SwipeKind,
    anchor: f64,
    step: f64,
    fired: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SwipeKind {
    /// Dark bar: every step is one volume key.
    Volume,
    /// Dark bar, three fingers: every step is one brightness key.
    Brightness,
    /// Lit bar: the first step switches the layer, the rest of the swipe is
    /// ignored.
    Mode,
}

impl Swipe {
    pub fn new(kind: SwipeKind, x: f64, step: f64) -> Self {
        Self {
            kind,
            anchor: x,
            step: step.max(1.0),
            fired: false,
        }
    }

    /// Feeds the current centroid and returns the signed number of steps
    /// crossed since the previous call. A mode swipe reports at most one.
    pub fn advance(&mut self, x: f64) -> i32 {
        if self.kind == SwipeKind::Mode && self.fired {
            return 0;
        }
        let steps = ((x - self.anchor) / self.step).trunc() as i32;
        if steps == 0 {
            return 0;
        }
        self.fired = true;
        if self.kind == SwipeKind::Mode {
            return steps.signum();
        }
        self.anchor += f64::from(steps) * self.step;
        steps
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_swipe_steps_both_ways_without_drift() {
        let mut swipe = Swipe::new(SwipeKind::Volume, 1000.0, 40.0);
        assert_eq!(swipe.advance(1039.0), 0);
        assert_eq!(swipe.advance(1041.0), 1);
        assert_eq!(swipe.advance(1125.0), 2);
        assert_eq!(swipe.advance(1080.0), -1);
        assert_eq!(swipe.advance(1040.0), -1);
    }

    #[test]
    fn mode_swipe_fires_once() {
        let mut swipe = Swipe::new(SwipeKind::Mode, 500.0, 160.0);
        assert_eq!(swipe.advance(400.0), 0);
        assert_eq!(swipe.advance(300.0), -1);
        assert_eq!(swipe.advance(0.0), 0);
        assert_eq!(swipe.advance(900.0), 0);
    }
}
