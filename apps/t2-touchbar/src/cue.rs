// SPDX-License-Identifier: GPL-3.0-or-later

//! The play cue on the dark bar: a light running clockwise around a play
//! triangle. The next edge appears until the triangle closes, then the
//! oldest one leaves until a single edge is left, and it grows again from
//! there. Every change is a single frame, so the cue costs one wakeup per
//! step and lights only a few thin lines, each for the same share of time.

/// Edges of the triangle pointing right, clockwise from the left one:
/// 0 is the vertical edge, 1 the upper and 2 the lower slope.
pub const EDGES: u8 = 3;

const STEP_MS: u64 = 1_000;

pub struct Cue {
    /// The oldest lit edge.
    tail: u8,
    /// Lit edges from `tail` on, clockwise.
    length: u8,
    growing: bool,
}

impl Cue {
    /// Starts as a forward arrow, the two slopes, which the vertical edge
    /// closes next. After the pause bars two strokes stay two strokes.
    pub fn new() -> Self {
        Self {
            tail: 1,
            length: 2,
            growing: true,
        }
    }

    /// The lit edges as a bit mask.
    pub fn edges(&self) -> u8 {
        (0..self.length).fold(0, |mask, offset| mask | 1 << ((self.tail + offset) % EDGES))
    }

    /// How long the current state stays.
    pub fn hold_ms(&self) -> u64 {
        STEP_MS
    }

    pub fn advance(&mut self) {
        if self.growing {
            self.length += 1;
            self.growing = self.length < EDGES;
        } else {
            self.tail = (self.tail + 1) % EDGES;
            self.length -= 1;
            self.growing = self.length == 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_clockwise_from_the_forward_arrow() {
        let mut cue = Cue::new();
        let mut seen = vec![cue.edges()];
        for _ in 0..8 {
            cue.advance();
            seen.push(cue.edges());
        }
        assert_eq!(
            seen,
            [
                0b110, 0b111, 0b101, 0b001, 0b011, 0b111, 0b110, 0b100, 0b101
            ]
        );
    }

    #[test]
    fn never_dark_one_edge_per_step_and_every_edge_lit_alike() {
        let mut cue = Cue::new();
        let mut lit = [0u32; EDGES as usize];
        for _ in 0..12 {
            let before = cue.edges();
            cue.advance();
            let after = cue.edges();
            assert_ne!(after, 0);
            assert_eq!((before ^ after).count_ones(), 1);
            for (edge, count) in lit.iter_mut().enumerate() {
                *count += u32::from(after >> edge & 1);
            }
        }
        assert!(lit.iter().all(|&count| count == lit[0]));
    }
}
