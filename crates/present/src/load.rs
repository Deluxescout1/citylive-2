//! B101: be polite under load (D88: no caps, but no hogging either).
//!
//! The wallpaper runs at the display's rate. When another program takes the CPU
//! (a build, a game that is still showing the desktop, a video call) frames start
//! to miss their deadline, and the usual answer is that the frame rate drops for
//! everything at once. This governor looks for the *pattern* - several late or
//! over-budget frames in the last second - and sheds the cheapest-to-lose motion
//! first: the water's shimmer, the clouds' drift and the far traffic and
//! residents are stepped less often, while everything else (and the frame rate)
//! stays where it is. When the frames are comfortable again for a few seconds it
//! steps back, one level at a time. Every change is logged.

use std::time::Duration;

/// The deepest level.
pub const MAX_LEVEL: u8 = 3;

/// How the three motion classes are thinned at a level: one tick in `n`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Divisors {
    pub water: u32,
    pub clouds: u32,
    pub movers: u32,
}

#[derive(Clone, Debug)]
pub struct Governor {
    level: u8,
    /// One bit per recent frame, newest lowest: set = it missed.
    history: u32,
    /// Comfortable frames in a row.
    calm: u32,
    /// Frames to wait after a change before another.
    cooldown: u32,
    /// Counts animation ticks (the divisors act on it).
    tick: u64,
    /// Missed frames among the last 30 that raise the level.
    raise_at: u32,
    /// Comfortable frames in a row that lower it (about 5 s at 30 fps).
    calm_needed: u32,
}

impl Default for Governor {
    fn default() -> Self {
        Self {
            level: 0,
            history: 0,
            calm: 0,
            cooldown: 0,
            tick: 0,
            raise_at: 6,
            calm_needed: 150,
        }
    }
}

impl Governor {
    pub fn level(&self) -> u8 {
        self.level
    }

    /// The thinning at the current level.
    pub fn divisors(&self) -> Divisors {
        match self.level {
            0 => Divisors { water: 1, clouds: 1, movers: 1 },
            1 => Divisors { water: 2, clouds: 2, movers: 1 },
            2 => Divisors { water: 4, clouds: 3, movers: 2 },
            _ => Divisors { water: 8, clouds: 4, movers: 3 },
        }
    }

    /// Advance the tick counter (once per animation step) and return it.
    pub fn next_tick(&mut self) -> u64 {
        self.tick = self.tick.wrapping_add(1);
        self.tick
    }

    /// Is this the tick on which a class thinned by `n` should step?
    pub fn due(&self, n: u32) -> bool {
        n <= 1 || self.tick % n as u64 == 0
    }

    /// Feed one finished frame: how long its work took (animate and composite
    /// and present), how late it started against its deadline, and the frame
    /// interval. Returns `(from, to)` when the level changed.
    pub fn frame(&mut self, busy: Duration, late: Duration, interval: Duration) -> Option<(u8, u8)> {
        let missed = busy * 4 > interval * 3 || late * 2 > interval;
        self.history = (self.history << 1) | missed as u32;
        if missed {
            self.calm = 0;
        } else {
            self.calm = self.calm.saturating_add(1);
        }
        self.cooldown = self.cooldown.saturating_sub(1);
        if self.cooldown > 0 {
            return None;
        }
        let recent = (self.history & 0x3FFF_FFFF).count_ones();
        if self.level < MAX_LEVEL && recent >= self.raise_at {
            let from = self.level;
            self.level += 1;
            self.history = 0;
            self.cooldown = 45;
            return Some((from, self.level));
        }
        if self.level > 0 && self.calm >= self.calm_needed {
            let from = self.level;
            self.level -= 1;
            self.calm = 0;
            self.cooldown = 45;
            return Some((from, self.level));
        }
        None
    }

    /// A line for the log.
    pub fn describe(&self) -> String {
        let d = self.divisors();
        format!(
            "water 1/{}, clouds 1/{}, far movers 1/{}",
            d.water, d.clouds, d.movers
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const I: Duration = Duration::from_micros(33_333);

    fn run(g: &mut Governor, frames: usize, busy_ms: u64, late_ms: u64) -> Vec<(u8, u8)> {
        let mut changes = Vec::new();
        for _ in 0..frames {
            if let Some(c) = g.frame(Duration::from_millis(busy_ms), Duration::from_millis(late_ms), I) {
                changes.push(c);
            }
        }
        changes
    }

    #[test]
    fn comfortable_frames_never_degrade() {
        let mut g = Governor::default();
        assert!(run(&mut g, 3000, 12, 0).is_empty());
        assert_eq!(g.level(), 0);
        assert_eq!(g.divisors(), Divisors { water: 1, clouds: 1, movers: 1 });
    }

    #[test]
    fn a_few_odd_slow_frames_are_not_a_pattern() {
        let mut g = Governor::default();
        for i in 0..600 {
            let busy = if i % 15 == 0 { 40 } else { 10 };
            g.frame(Duration::from_millis(busy), Duration::ZERO, I);
        }
        assert_eq!(g.level(), 0);
    }

    #[test]
    fn repeated_misses_step_the_level_up_to_the_cap_and_back() {
        let mut g = Governor::default();
        let up = run(&mut g, 600, 40, 20);
        assert_eq!(up, vec![(0, 1), (1, 2), (2, 3)]);
        assert_eq!(g.level(), MAX_LEVEL);
        // The load goes away: one step down per calm stretch.
        let down = run(&mut g, 2000, 10, 0);
        assert_eq!(down, vec![(3, 2), (2, 1), (1, 0)]);
        assert_eq!(g.level(), 0);
    }

    #[test]
    fn lateness_alone_counts() {
        let mut g = Governor::default();
        let up = run(&mut g, 100, 5, 30);
        assert!(!up.is_empty());
    }

    #[test]
    fn the_divisors_thin_but_never_stop() {
        let mut g = Governor::default();
        g.level = MAX_LEVEL;
        let d = g.divisors();
        let mut water = 0;
        for _ in 0..80 {
            g.next_tick();
            if g.due(d.water) {
                water += 1;
            }
        }
        assert_eq!(water, 10);
        assert!(g.describe().contains("water 1/8"));
    }
}
