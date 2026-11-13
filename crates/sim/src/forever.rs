//! Growth that never stops (D6, B26c item 1).
//!
//! The planned grid fills the land in a few weeks; after that the city can only
//! go **up**. This module is the second engine:
//!
//! * `redevelop()` — one teardown-and-rebuild per decision: a cottage into a
//!   terrace, a terrace into apartments, an office into a tower, and a tower
//!   that gains floors as the decades pass (D6: "teardown → taller rebuilds,
//!   districts mature"; the tall-tower cap rises with the city's age);
//! * `reclaim()` — once the town is mature, the cells that were skipped to
//!   protect the old village are taken back, so the historic core is finally
//!   replaced block by block (D39: "earlier buildings are gradually replaced,
//!   though some historic ones survive").
//!
//! Both are rate-limited and staged, so the frames change slowly and everything
//! is visible as construction (D33).

use crate::{District, Era, LotKind, Sim, Stage, HOUSE_SECONDS};
use map::rng::Rng;

/// How tall a tower may get at this age of the city (B26c: the downtown keeps
/// rising — 20 storeys at day 30, ~40 by day 180).
// B39/D62: density is gated by **population**, not by the calendar, so a slow
// seed does not get towers before it has the people for them. One place for the
// constants.
/// A mid-rise (4-8 storeys) needs this many residents.
pub const MID_RISE_MIN_POP: u32 = 800;
/// The first tower needs this many.
pub const TOWER_MIN_POP: u32 = 2_500;
/// The signature tower (a landmark) needs this many.
pub const SIGNATURE_MIN_POP: u32 = 4_000;
/// Floors a tower may reach at this population (D62: height follows people).
pub fn tower_cap(pop: u32) -> u8 {
    // 22 floors at the first tower, one more per 1 000 residents, capped at 40 so
    // the skyline still fits the frame (B26d).
    let extra = pop.saturating_sub(TOWER_MIN_POP) / 1_000;
    (22 + extra.min(18)) as u8
}

impl Sim {
    /// The next denser thing this plot could become, or a taller rebuild.
    /// `None` means the plot is at its ceiling.
    fn rebuild_target(&self, i: usize, days: f64, lvl: u8) -> Option<(LotKind, u8)> {
        let l = &self.lots[i];
        let d = self.district_at(l.x, l.y);
        // B99: and never taller than the room under the land line.
        let cap = tower_cap(self.residents()).min(self.headroom_storeys(l.x, l.y));
        match (l.kind, d) {
            // Estates and the high-end fringe stay low (D46).
            (LotKind::House, District::HighEnd) => None,
            // The old village keeps its cottages for a while, then follows the
            // town up (D39).
            (LotKind::House, District::Core) if days < 30.0 => None,
            (LotKind::House, District::Core) => Some((LotKind::Townhouse, 0)),
            (LotKind::House, District::Suburb) if lvl < 2 || days < 25.0 => None,
            (LotKind::House, _) => Some((LotKind::Townhouse, 0)),
            (LotKind::Townhouse, District::Suburb) if days < 40.0 => None,
            (LotKind::Townhouse, _) if lvl >= 2 && self.residents() >= MID_RISE_MIN_POP => {
                Some((LotKind::Apartment, 0))
            }
            (LotKind::Townhouse, _) => None,
            (LotKind::Shop | LotKind::Store, _)
                if days >= 35.0 && self.residents() >= MID_RISE_MIN_POP =>
            {
                Some((LotKind::Apartment, 0))
            }
            (LotKind::Shop | LotKind::Store, _) => None,
            // D52/B26d: the tower cluster grows outward as the city matures, so
            // downtown upgrades ripple into the ring of apartments round it.
            (LotKind::Apartment, _)
                if days >= 45.0
                    && self.residents() >= TOWER_MIN_POP
                    && self.in_downtown_ripple(l.x, l.y, days) =>
            {
                if cap >= 14 { Some((LotKind::Tower, 0)) } else { None }
            }
            (LotKind::Apartment, _) => None,
            // Only the downtown grows towers (B26b item 4).
            (LotKind::Office, District::Downtown) if self.residents() >= TOWER_MIN_POP && cap >= 14 => {
                Some((LotKind::Tower, 0))
            }
            (LotKind::Office, _) if self.residents() >= MID_RISE_MIN_POP => None,
            (LotKind::Office, _) => None,
            // A tower gains floors for as long as the city ages, up to the cap.
            (LotKind::Tower, _) if l.storeys + 1 < cap => Some((LotKind::Tower, 3)),
            _ => None,
        }
    }

    /// Is this plot inside the downtown's ripple? It starts at the district
    /// itself and widens by ~1.5 tiles a week, so the skyline spreads outward
    /// over the months instead of sitting in one fixed circle (B26d item 2).
    fn in_downtown_ripple(&self, x: i32, y: i32, days: f64) -> bool {
        let c = self.downtown_centre();
        let dist = ((x - c.0).abs() + (y - c.1).abs()) as f32;
        // No hard edge: the ripple creeps out ~1 tile a fortnight, so the city
        // keeps densifying for years (D6) without a cliff in the numbers.
        let r = ((0.20 + (days - 45.0).max(0.0) * 0.0009) * self.span() as f64) as f32;
        if dist <= r {
            return true;
        }
        self.district_at(x, y) == District::Downtown
    }

    /// D6: one rebuild a decision — the best-placed plot that can go denser.
    pub(crate) fn redevelop(&mut self, rng: &mut Rng) -> bool {
        let days = self.days();
        let lvl = self.density_level();
        if days < 12.0 {
            return false;
        }
        let mut cand: Option<(usize, f32)> = None;
        for (i, l) in self.lots.iter().enumerate() {
            if !l.finished() || matches!(l.kind, LotKind::Park | LotKind::LogCabin) {
                continue;
            }
            // B39: a plot that fronts only a worn path is the settlement, not a
            // block — it does not densify into a mid-rise.
            if !self.has_lane_near(l.x, l.y, l.w, l.h) {
                continue;
            }
            if self.rebuild_target(i, days, lvl).is_none() {
                continue;
            }
            let v = self.land_value(l.x, l.y) + self.district_at(l.x, l.y).density_bonus();
            if cand.map(|(_, bv)| v > bv).unwrap_or(true) {
                cand = Some((i, v));
            }
        }
        let Some((i, _v)) = cand else { return false };
        let Some((next, lift)) = self.rebuild_target(i, days, lvl) else {
            return false;
        };
        let (x, y, w, h, k) = {
            let l = &self.lots[i];
            (l.x, l.y, l.w, l.h, l.kind)
        };
        let d = self.district_at(x, y);
        let storeys = if lift > 0 {
            (self.lots[i].storeys + lift).min(tower_cap(self.residents()))
        } else {
            (self.storeys_for(next, d, x, y) + rng.usize(3) as u8)
                .min(self.headroom_storeys(x, y).max(2))
        };
        let l = &mut self.lots[i];
        l.kind = next;
        l.storeys = storeys;
        l.style = d.style();
        // Teardown and rebuild: the plot goes back to the staked stage, so the
        // 7 stages are visible (D33).
        l.stage = Stage::Staked;
        l.stage_end = self.time + HOUSE_SECONDS / 6.0;
        // A new building has no age: the ageing pass reckons grime, boards and
        // shutters from `started`, so a tower put up today must not wear the
        // years of the cottage it replaced (the ageing rebuild resets it too).
        l.started = self.time;
        for ty in y..y + h {
            for tx in x..x + w {
                self.changes.insert((tx, ty));
            }
        }
        self.events.push(crate::Event {
            time: self.time,
            text: if lift > 0 {
                format!("{k:?} gains floors: now {} storeys", storeys)
            } else {
                format!("{k:?} rebuilt as {}", crate::next_name(next))
            },
        });
        true
    }

    /// B26c item 1: a mature city takes back the cells the planner skipped to
    /// keep the old village intact, so the core is finally redeveloped (D39).
    pub(crate) fn reclaim(&mut self) -> bool {
        if self.era() < Era::City || self.days() < 32.0 {
            return false;
        }
        // Nearest first, and only cells whose **sole** objection is the old
        // core's lanes: anything else would come straight back as a skip, and
        // the pick would then be stuck on that cell for ever.
        let mut cand: Vec<((i32, i32), i32)> = self
            .blocks_skipped()
            .into_iter()
            .map(|c| {
                let r = self.grid_rect(c.0, c.1);
                let dx = r.sx - self.founding.0;
                let dy = r.sy - self.founding.1;
                (c, dx * dx + dy * dy)
            })
            .collect();
        cand.sort_unstable();
        for (c, _) in cand {
            if self.unskip_cell(c) {
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use crate::{Pace, Sim, Stage};

    #[test]
    fn a_rebuilt_plot_starts_its_life_again() {
        // Watch the city hour by hour: any plot sent back to the staked stage
        // (a rebuild) must have been started just now, not at its old founding.
        let mut s = Sim::new(5, Pace::Month, 107, 107);
        s.ending.world_days = 1e9;
        let mut seen = 0;
        let mut t = 0.0;
        while t < 45.0 * 86400.0 {
            t += 3600.0;
            s.run_to(t);
            let backdated = s
                .lots
                .iter()
                .filter(|l| l.stage == Stage::Staked && s.time - l.started > 3.0 * 3600.0)
                .count();
            assert_eq!(backdated, 0, "at day {:.1}: {backdated} staked plots kept an old start", s.days());
            seen += s.lots.iter().filter(|l| l.stage == Stage::Staked).count();
        }
        assert!(seen > 0, "no plot was ever staked");
    }
}
