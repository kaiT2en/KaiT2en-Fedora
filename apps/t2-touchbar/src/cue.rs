// SPDX-License-Identifier: GPL-3.0-or-later

//! The play cue on the dark bar: the edges of a play triangle appear one at
//! a time until it closes, then leave again in reverse. Every change is a
//! single frame, so the cue costs about one wakeup per step and lights only
//! a few thin lines, never the same ones for long.

/// Edges of the triangle pointing right, clockwise from the left one:
/// 0 is the vertical edge, 1 the upper and 2 the lower slope.
pub const EDGES: u8 = 3;

const STEP_MS: u64 = 800;
/// The closed triangle stays a little longer, so it reads as play.
const CLOSED_MS: u64 = 1_600;

pub struct Cue {
    /// The order in which this round adds its edges.
    order: [u8; 3],
    /// Position in the round: one, two, three, two, one edges.
    step: u8,
    rng: u32,
}

impl Cue {
    /// The first round starts with the vertical edge and runs clockwise.
    /// Later rounds pick start and direction at random.
    pub fn new(seed: u32) -> Self {
        Self {
            order: [0, 1, 2],
            step: 0,
            rng: seed | 1,
        }
    }

    /// The lit edges as a bit mask.
    pub fn edges(&self) -> u8 {
        let count = match self.step {
            0 | 4 => 1,
            1 | 3 => 2,
            _ => 3,
        };
        self.order[..count]
            .iter()
            .fold(0, |mask, edge| mask | 1 << edge)
    }

    /// How long the current state stays.
    pub fn hold_ms(&self) -> u64 {
        if self.step == 2 { CLOSED_MS } else { STEP_MS }
    }

    pub fn advance(&mut self) {
        self.step = (self.step + 1) % 5;
        if self.step == 0 {
            let start = (self.random() % u32::from(EDGES)) as u8;
            let turn = if self.random() & 1 == 0 { 1 } else { 2 };
            self.order = [start, (start + turn) % EDGES, (start + 2 * turn) % EDGES];
        }
    }

    /// xorshift32: varied enough to look random, no dependency.
    fn random(&mut self) -> u32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        self.rng
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_round_builds_clockwise_from_the_vertical_edge_and_back() {
        let mut cue = Cue::new(7);
        let mut seen = vec![cue.edges()];
        for _ in 0..4 {
            cue.advance();
            seen.push(cue.edges());
        }
        assert_eq!(seen, [0b001, 0b011, 0b111, 0b011, 0b001]);
    }

    #[test]
    fn never_dark_and_one_edge_per_step_within_a_round() {
        let mut cue = Cue::new(12_345);
        for _ in 0..500 {
            let before = cue.edges();
            let round_ends = cue.step == 4;
            cue.advance();
            let after = cue.edges();
            assert_ne!(after, 0);
            if !round_ends {
                assert_eq!((before ^ after).count_ones(), 1);
            }
        }
    }

    #[test]
    fn later_rounds_start_on_every_edge() {
        let mut cue = Cue::new(99);
        let mut starts = 0u8;
        for _ in 0..200 {
            cue.advance();
            if cue.step == 0 {
                starts |= cue.edges();
            }
        }
        assert_eq!(starts, 0b111);
    }
}
