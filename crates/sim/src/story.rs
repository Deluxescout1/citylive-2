//! The city's name and its story feed (B48a/D65/D68).
//!
//! Two things the city *says* about itself: its name, chosen by the founders
//! when the camp is struck, from a table that fits the land (D66/D68), and a
//! bounded ring of short human lines, each with its sim time — the founding,
//! the first cabin, the first paved road, the population milestones, the new
//! landmarks, the street crews, the families moving in. The tray reads the
//! ring newest first (D65); nothing here draws, and nothing here costs per
//! frame.
//!
//! The name is a pure function of `(seed, biome)`, so a save keeps it, a
//! reload regenerates the same one, and two machines playing the same seed
//! settle on the same place.

use map::Biome;
use std::collections::VecDeque;

/// How many story lines the ring holds (D65: the *recent* story, bounded).
pub const STORY_CAP: usize = 200;

/// One line of the feed.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    /// Sim seconds since the founding.
    pub time: f64,
    /// The line, in the voice of the place ("the first street is paved").
    pub text: String,
}

/// How many milestone lines the feed keeps for good (B48 follow-up).
pub const KEPT_CAP: usize = 120;

/// The feed: two bounded lists. The *kept* list holds the milestones and
/// firsts (the founding, the naming, the first road, the population marks) and
/// is never pushed out by gossip; the *ring* holds the rolling life lines
/// (births, weddings, funerals, moves), the 200 most recent. The tray reads
/// both, newest first, so the mix is by time.
#[derive(Clone, Debug, Default)]
pub struct Story {
    lines: VecDeque<Line>,
    kept: Vec<Line>,
}

impl Story {
    /// B84: the bytes this feed holds (the rolling ring and the kept lines).
    pub fn bytes(&self) -> usize {
        let line = std::mem::size_of::<Line>();
        self.lines.capacity() * line
            + self.kept.capacity() * line
            + self
                .lines
                .iter()
                .chain(self.kept.iter())
                .map(|l| l.text.capacity())
                .sum::<usize>()
    }

    /// A rolling line (life events): the ring keeps the newest `STORY_CAP`.
    pub fn push(&mut self, time: f64, text: impl Into<String>) {
        self.lines.push_back(Line {
            time,
            text: text.into(),
        });
        if self.lines.len() > STORY_CAP {
            self.lines.pop_front();
        }
    }

    /// A milestone or a first: kept until the list passes `KEPT_CAP`, which a
    /// world's worth of firsts never does.
    pub fn push_kept(&mut self, time: f64, text: impl Into<String>) {
        self.kept.push(Line {
            time,
            text: text.into(),
        });
        if self.kept.len() > KEPT_CAP {
            self.kept.remove(0);
        }
    }

    pub fn len(&self) -> usize {
        self.lines.len() + self.kept.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty() && self.kept.is_empty()
    }

    /// The `n` newest lines of either kind, newest first — the tray's order.
    pub fn latest(&self, n: usize) -> Vec<Line> {
        let mut a = self.lines.iter().rev().peekable();
        let mut b = self.kept.iter().rev().peekable();
        let mut out = Vec::with_capacity(n);
        while out.len() < n {
            let take_a = match (a.peek(), b.peek()) {
                (Some(x), Some(y)) => x.time >= y.time,
                (Some(_), None) => true,
                (None, Some(_)) => false,
                (None, None) => break,
            };
            out.push(if take_a { a.next() } else { b.next() }.unwrap().clone());
        }
        out
    }

    /// Every line oldest first, each with whether it is a kept one (the save).
    /// A merge by time that never reorders lines *within* either list: a line
    /// may be told with an earlier time than the one before it (an ending's
    /// warning is dated to its start), and a save must give back each list in
    /// the order it was told, or a reload changes the world's hash.
    pub fn iter_flagged(&self) -> Vec<(&Line, bool)> {
        let mut out = Vec::with_capacity(self.len());
        let mut a = self.kept.iter().peekable();
        let mut b = self.lines.iter().peekable();
        loop {
            let take_kept = match (a.peek(), b.peek()) {
                (Some(x), Some(y)) => x.time <= y.time,
                (Some(_), None) => true,
                (None, Some(_)) => false,
                (None, None) => break,
            };
            if take_kept {
                out.push((a.next().unwrap(), true));
            } else {
                out.push((b.next().unwrap(), false));
            }
        }
        out
    }

    /// All lines, oldest first.
    pub fn iter(&self) -> std::vec::IntoIter<&Line> {
        self.iter_flagged().into_iter().map(|(l, _)| l).collect::<Vec<_>>().into_iter()
    }

    /// Rebuild from saved rows and their kept flags.
    pub fn from_flagged(rows: Vec<(Line, bool)>) -> Story {
        let mut s = Story::default();
        for (l, kept) in rows {
            if kept {
                s.push_kept(l.time, l.text);
            } else {
                s.push(l.time, l.text);
            }
        }
        s
    }

    /// The feed as part of the sim's deterministic hash (B45-style FNV mix).
    pub fn hash(&self) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let mut mix = |v: u64| {
            h ^= v;
            h = h.wrapping_mul(0x1000_0000_01b3);
        };
        mix(self.lines.len() as u64);
        mix(self.kept.len() as u64);
        for l in self.lines.iter().chain(self.kept.iter()) {
            mix(l.time.to_bits());
            for b in l.text.as_bytes() {
                mix(*b as u64);
            }
        }
        h
    }
}

/// A city name, from what the land calls it and what it has. The first part
/// is the thing, the second the place: `Mill` + `brook`, `Frost` + `holm`,
/// `Palm` + `Haven` (D68: the name fits the biome).
pub fn city_name(seed: u64, biome: Biome) -> String {
    let (things, places) = tables(biome);
    // `Rng::new` mixes its state, so the seed and the land can never collide
    // into one stream.
    let mut rng = map::rng::Rng::new(seed ^ (0xC177_8A9B_E76C_B48A | biome.id() as u64));
    let a = things[rng.usize(things.len())];
    let b = places[rng.usize(places.len())];
    format!("{a}{b}")
}

/// One line of the feed, formatted for the tray ("day 3 — the first cabin is
/// staked") (B48a item 3).
///
/// B65: the day is `chronicle::day_number`'s, the same one the chronicle prints,
/// so the menu and the history never disagree about what day it is.
pub fn tray_line(l: &Line) -> String {
    format!("day {} — {}", crate::chronicle::day_number(l.time), l.text)
}

/// What the tray shows about the city (B48a): the name, the headcount and era
/// line, and the latest feed lines in the order the menu paints them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TrayStatus {
    /// The city's name (the tray's title).
    pub city: String,
    /// "N people · era".
    pub line: String,
    /// The latest feed lines, newest first (`just_watch` gives none).
    pub feed: Vec<String>,
    /// B53: the Bills gameday line for the tray ("Bills 14 - 7 NE · 2nd 7:12").
    /// Empty when there is no game to show.
    pub gameday: String,
    /// B52: the mayor and the economy's mood in a few words ("Mayor Ada Fenn ·
    /// good times"). Empty before the town has either.
    pub civic: String,
}

impl TrayStatus {
    /// The menu model as plain lines, the way the tray would paint it (for the
    /// handback and the tests — the tray itself is not a terminal, B48a item 4).
    pub fn as_menu(&self) -> Vec<String> {
        let mut out = vec![self.city.clone(), self.line.clone()];
        if !self.civic.is_empty() {
            out.push(self.civic.clone());
        }
        if !self.civic.is_empty() {
            out.push(self.civic.clone());
        }
        if !self.gameday.is_empty() {
            out.push(self.gameday.clone());
        }
        out.extend(self.feed.iter().cloned());
        out
    }
}

/// The name tables per land (D66). The stub biomes have no name of their own
/// yet and borrow the temperate one, the same way their look borrows it.
fn tables(biome: Biome) -> (&'static [&'static str], &'static [&'static str]) {
    match biome {
        Biome::Temperate => (
            &[
                "Mill", "Elm", "Hawthorn", "Cedar", "Willow", "Oak", "Aspen", "Hazel", "Bramble",
                "Lark", "Heron", "Fox", "Stone", "Green", "Hill", "Aldermoor",
            ],
            &[
                "brook", "field", "ford", "grove", "creek", "hall", "mead", "ridge", "hollow",
                "springs", "cote", "haven",
            ],
        ),
        Biome::Desert => (
            &[
                "Red", "Sun", "Wind", "Cactus", "Sand", "Dune", "Arid", "Alamo", "Canyon",
                "Mesquite", "Sage", "Caliche", "Vega", "Rio", "Amber", "Caldera",
            ],
            &[
                "mesa", "dunes", "springs", "plains", "pass", "hollow", "rocks", "bend",
                "flats", "canyon", "wash", "bluffs",
            ],
        ),
        Biome::Tundra => (
            &[
                "Frost", "Snow", "Polar", "Kettle", "Aurora", "Birch", "Pine", "Ivory", "North",
                "Sable", "Hawk", "Iron", "Ice", "Winter", "Tundra", "Ember",
            ],
            &[
                "holm", "fjord", "bark", "pines", "falls", "gate", "cross", "hollow", "peak",
                "stead", "harbour", "sound",
            ],
        ),
        Biome::Tropical => (
            &[
                "Palm", "Coral", "Lagoon", "Ivy", "Monsoon", "Rainbow", "Coconut", "Parrot",
                "Isle", "Tropic", "Mango", "Sable", "Fern", "Sunbird", "Drift", "Cobalt",
            ],
            &[
                "haven", "cove", "reef", "bay", "isles", "shore", "water", "keys", "strand",
                "bloom", "roost", "falls",
            ],
        ),
        _ => (
            &[
                "Mill", "Elm", "Hawthorn", "Cedar", "Willow", "Oak", "Aspen", "Hazel", "Bramble",
                "Lark", "Heron", "Fox", "Stone", "Green", "Hill", "Aldermoor",
            ],
            &[
                "brook", "field", "ford", "grove", "creek", "hall", "mead", "ridge", "hollow",
                "springs", "cote", "haven",
            ],
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// B48a item 4: the same seed and biome give the same name, for everyone.
    #[test]
    fn name_is_deterministic_per_seed_and_biome() {
        for seed in 0..8u64 {
            for b in Biome::ALL {
                assert_eq!(city_name(seed, b), city_name(seed, b));
            }
        }
    }

    /// Different seeds mostly give different names (a place is a place).
    #[test]
    fn seeds_mostly_give_different_names() {
        let names: Vec<String> = (0..16u64).map(|s| city_name(s, Biome::Temperate)).collect();
        let distinct = std::collections::BTreeSet::from_iter(names.iter());
        assert!(distinct.len() >= 12, "only {} distinct names in 16 seeds", distinct.len());
    }

    /// The name reads like a name, not like code: capital first part, lowercase
    /// place, both non-empty.
    #[test]
    fn names_read_like_names() {
        for b in Biome::ALL {
            for seed in 0..4u64 {
                let n = city_name(seed, b);
                let (a, rest) = n.split_at(1);
                assert!(!a.is_empty() && a.chars().next().unwrap().is_ascii_uppercase());
                assert!(!rest.is_empty() && rest.chars().next().unwrap().is_ascii_lowercase());
                assert!((3..=24).contains(&n.len()), "{n}");
            }
        }
    }

    /// B48a item 4: the ring is bounded and the tray's order is newest first.
    #[test]
    fn buffer_is_bounded_newest_first() {
        let mut s = Story::default();
        for i in 0..300u64 {
            s.push(i as f64, format!("line {i}"));
        }
        assert_eq!(s.lines.len(), STORY_CAP, "the ring must hold exactly {STORY_CAP}");
        let latest = s.latest(8);
        assert_eq!(latest.len(), 8);
        assert_eq!(latest[0].text, "line 299");
        assert_eq!(latest[7].text, "line 292");
        // The oldest kept line is the 101st.
        assert_eq!(s.lines.front().unwrap().text, "line 100");
    }
}
