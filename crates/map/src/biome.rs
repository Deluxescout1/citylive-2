//! Which land a world is (D66): one dominant biome per seed, with a neighbouring
//! biome blending in at one map edge (D64's horizon rule).
//!
//! The biome is a property of the *world*, not of a tile: the tile map remembers
//! which biome it was generated for plus which neighbour bleeds in and from which
//! edge, and [`Biome::blend_weight`] is the only place the mix is worked out. The
//! terrain generator, the renderer and the weather translation all read it, so a
//! desert's ground, its plants and its sandstorms can never disagree.
//!
//! The four lands the game ships today are [`Temperate`](Biome::Temperate),
//! [`Desert`](Biome::Desert), [`Tundra`](Biome::Tundra) and
//! [`Tropical`](Biome::Tropical). The rest of D66's list is here as stubs: they
//! can be named, saved and tested, but [`Biome::is_live`] is false, so no world
//! rolls them and the look tables fall back to temperate until they are built.

use crate::rng::{fbm, hash2};
use crate::types::Edge;

/// A world's land (D66).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Hash)]
pub enum Biome {
    /// Lush green, four seasons: the land every world had before D66.
    #[default]
    Temperate,
    /// Sand, dunes, red mesas, cacti and an oasis.
    Desert,
    /// Snow ground, spruces and firs, frozen lake edges, rock.
    Tundra,
    /// Palms, dense jungle, beaches and bright water.
    Tropical,

    // --- D66 stubs: named, saved, not yet generated -----------------------
    Alpine,
    Savanna,
    Swamp,
    Lagoon,
    Islands,
    Highlands,
    Volcanic,
    Mediterranean,
    Steppe,
    Bamboo,
}

impl Biome {
    /// Every biome, in `id()` order.
    pub const ALL: [Biome; 14] = [
        Biome::Temperate,
        Biome::Desert,
        Biome::Tundra,
        Biome::Tropical,
        Biome::Alpine,
        Biome::Savanna,
        Biome::Swamp,
        Biome::Lagoon,
        Biome::Islands,
        Biome::Highlands,
        Biome::Volcanic,
        Biome::Mediterranean,
        Biome::Steppe,
        Biome::Bamboo,
    ];

    /// The four lands a world can actually be (D68: desert, snow/tundra,
    /// tropical first, on a temperate base).
    pub const LIVE: [Biome; 10] = [
        Biome::Temperate,
        Biome::Desert,
        Biome::Tundra,
        Biome::Tropical,
        Biome::Alpine,
        Biome::Savanna,
        Biome::Swamp,
        Biome::Lagoon,
        Biome::Islands,
        Biome::Highlands,
    ];

    /// Does this land have a generator and a look yet?
    pub fn is_live(self) -> bool {
        matches!(
            self,
            Biome::Temperate | Biome::Desert | Biome::Tundra | Biome::Tropical
                | Biome::Alpine
                | Biome::Savanna
                | Biome::Swamp
                | Biome::Lagoon
                | Biome::Islands
                | Biome::Highlands
        )
    }

    pub fn id(self) -> u8 {
        match self {
            Biome::Temperate => 0,
            Biome::Desert => 1,
            Biome::Tundra => 2,
            Biome::Tropical => 3,
            Biome::Alpine => 4,
            Biome::Savanna => 5,
            Biome::Swamp => 6,
            Biome::Lagoon => 7,
            Biome::Islands => 8,
            Biome::Highlands => 9,
            Biome::Volcanic => 10,
            Biome::Mediterranean => 11,
            Biome::Steppe => 12,
            Biome::Bamboo => 13,
        }
    }

    pub fn from_id(id: u8) -> Biome {
        Biome::ALL[(id as usize) % Biome::ALL.len()]
    }

    pub fn name(self) -> &'static str {
        match self {
            Biome::Temperate => "temperate",
            Biome::Desert => "desert",
            Biome::Tundra => "tundra",
            Biome::Tropical => "tropical",
            Biome::Alpine => "alpine",
            Biome::Savanna => "savanna",
            Biome::Swamp => "swamp",
            Biome::Lagoon => "lagoon",
            Biome::Islands => "islands",
            Biome::Highlands => "highlands",
            Biome::Volcanic => "volcanic",
            Biome::Mediterranean => "mediterranean",
            Biome::Steppe => "steppe",
            Biome::Bamboo => "bamboo",
        }
    }

    /// Parse a `--biome <name>` value. Synonyms keep the old CityLive's names
    /// working (`snow`, `jungle`, `dunes`).
    pub fn from_name(s: &str) -> Option<Biome> {
        let s = s.trim().to_ascii_lowercase();
        Some(match s.as_str() {
            "temperate" | "grass" | "green" => Biome::Temperate,
            "desert" | "dunes" | "mesa" | "sand" => Biome::Desert,
            "tundra" | "snow" | "arctic" | "ice" => Biome::Tundra,
            "tropical" | "jungle" | "rainforest" | "palm" => Biome::Tropical,
            "alpine" | "mountain" | "mountains" => Biome::Alpine,
            "savanna" | "savannah" | "steppe_grass" => Biome::Savanna,
            "swamp" | "bayou" | "wetland" => Biome::Swamp,
            "lagoon" | "lagoons" => Biome::Lagoon,
            "islands" | "archipelago" => Biome::Islands,
            "highlands" | "moors" => Biome::Highlands,
            "volcanic" | "volcano" => Biome::Volcanic,
            "mediterranean" | "med" => Biome::Mediterranean,
            "steppe" => Biome::Steppe,
            "bamboo" => Biome::Bamboo,
            _ => return None,
        })
    }

    /// Relative weight for the world roll. The four original lands keep the
    /// weights (and so the seeds) they always had; the lands added in B46e are
    /// weighted against each other in [`Biome::pick`]'s second roll.
    pub fn weight(self) -> u32 {
        match self {
            Biome::Temperate => 56,
            Biome::Desert => 14,
            Biome::Tundra => 13,
            Biome::Tropical => 17,
            Biome::Alpine => 18,
            Biome::Savanna => 16,
            Biome::Swamp => 12,
            Biome::Highlands => 16,
            Biome::Islands => 10,
            Biome::Lagoon => 8,
            _ => 0,
        }
    }

    /// The lands added after D68, in the order of the second roll.
    pub const NEWER: [Biome; 6] = [
        Biome::Alpine,
        Biome::Savanna,
        Biome::Swamp,
        Biome::Highlands,
        Biome::Islands,
        Biome::Lagoon,
    ];

    /// The land a world is, from its seed. Deterministic and stable: the same
    /// seed is the same land for ever, which is what lets a save store only the
    /// seed plus a small override.
    ///
    /// Two rolls: the first is the historical one over the four original lands,
    /// so a seed keeps its land; the second hands a share of them over to the
    /// newer lands (45 % of temperate worlds, 15 % of the others), which keeps
    /// temperate the commonest land by far (D66) while every new land turns up.
    pub fn pick(seed: u64) -> Biome {
        let roll = unit_hash(seed ^ 0x4269_6F6D_6500_0001) * 100.0;
        let mut acc = 0.0;
        let mut base = Biome::Temperate;
        for b in [Biome::Temperate, Biome::Desert, Biome::Tundra, Biome::Tropical] {
            acc += b.weight() as f32;
            if roll < acc {
                base = b;
                break;
            }
        }
        let hand_over = if base == Biome::Temperate { 0.45 } else { 0.15 };
        if unit_hash(seed ^ 0x4269_6F6D_6500_0002) >= hand_over {
            return base;
        }
        let total: u32 = Biome::NEWER.iter().map(|b| b.weight()).sum();
        let mut r = unit_hash(seed ^ 0x4269_6F6D_6500_0003) * total as f32;
        for b in Biome::NEWER {
            let w = b.weight() as f32;
            if r < w {
                return b;
            }
            r -= w;
        }
        base
    }

    /// The neighbour that blends in at one map edge (D66 "neighbouring biomes
    /// blending in at the edges"). `None` for a land with only one live
    /// neighbour, or when the world keeps entirely to itself.
    pub fn neighbour(self, seed: u64) -> Option<Biome> {
        // B46e: the edge neighbour stays one of the four original lands, so every
        // existing seed keeps its blend; the newer lands are reached by the
        // ending's morph instead.
        let others: Vec<Biome> = Biome::LIVE[..4].iter().copied().filter(|b| *b != self).collect();
        if others.is_empty() {
            return None;
        }
        // A quarter of worlds have no neighbour at all: one land all the way to
        // the horizon is its own strong look.
        if unit_hash(seed ^ 0x6E65_6967_6862_0001) < 0.22 {
            return None;
        }
        let i = (unit_hash(seed ^ 0x6E65_6967_6862_0002) * others.len() as f32) as usize;
        Some(others[i.min(others.len() - 1)])
    }

    /// Which map edge the neighbour bleeds in from.
    pub fn blend_edge(seed: u64) -> Edge {
        let i = (unit_hash(seed ^ 0x626C_656E_6400_0001) * 4.0) as usize;
        Edge::ALL[i.min(3)]
    }
}

/// SplitMix64 finaliser, so neighbouring seeds land far apart.
fn unit_hash(seed: u64) -> f32 {
    let mut x = seed.wrapping_add(0x9E37_79B9_7F4A_7C15).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 30;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^= x >> 31;
    (x >> 40) as f32 / (1u64 << 24) as f32
}

/// How much of the neighbour's land is at tile `(x, y)`: 0 = the dominant biome
/// only, 1 = fully the neighbour. The mix lives in a band against one map edge
/// (a third of the short side), broken up by two scales of noise so the border
/// is a ragged tongue of land rather than a ruler line (D64: never a hard cut).
///
/// This is a pure function of `(seed, size, x, y)` — no storage — so the tile
/// generator, the renderer and the tests all get exactly the same answer.
pub fn blend_weight(seed: u64, edge: Edge, w: i32, h: i32, x: i32, y: i32) -> f32 {
    if w < 4 || h < 4 {
        return 0.0;
    }
    let band = (0.28 * w.min(h) as f32).max(4.0);
    let d = match edge {
        Edge::Left => x as f32,
        Edge::Right => (w - 1 - x) as f32,
        Edge::Top => y as f32,
        Edge::Bottom => (h - 1 - y) as f32,
    };
    if d >= band {
        return 0.0;
    }
    // 0 at the band's inner edge, 1 at the map edge.
    let t = 1.0 - (d / band).clamp(0.0, 1.0);
    let t = t * t * (3.0 - 2.0 * t);
    // Tongues of land: patchy at wood scale, ragged at the fine scale.
    let broad = fbm(x as f32 / 7.5, y as f32 / 7.5, seed ^ 0x0B1E_4D00, 2);
    let fine = fbm(x as f32 / 2.2, y as f32 / 2.2, seed ^ 0x0B1E_4D01, 2);
    let patch = ((broad * 0.7 + fine * 0.3) - 0.34).clamp(0.0, 1.0) / 0.5;
    (t * (0.35 + 1.05 * patch.min(1.0))).clamp(0.0, 1.0)
}

/// Does the neighbour's *species* grow here, as opposed to a colour blend?
/// Above this weight a plant is the neighbour's (a palm among the desert scrub
/// at the jungle end of a world) rather than the host's.
pub const SPECIES_MIX: f32 = 0.55;

/// Is this tile the neighbour's land rather than the host's, at `weight`?
///
/// The decision is made on the smooth field, nudged by a *low-frequency* noise
/// so the border is a ragged coastline of tongues rather than a dither of
/// single tiles — a speckled border reads as a bug, not a blend. This is what
/// picks a palm among the scrub and a spruce on the sand.
#[inline]
pub fn has_neighbour(weight: f32, x: i32, y: i32, seed: u64) -> bool {
    if weight <= 0.0 {
        return false;
    }
    let nudge = (fbm(x as f32 / 3.5, y as f32 / 3.5, seed ^ 0x0B1E_4D02, 2) - 0.5) * 0.45;
    weight + nudge > SPECIES_MIX
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn ids_and_names_round_trip() {
        for b in Biome::ALL {
            assert_eq!(Biome::from_id(b.id()), b, "{}", b.name());
            assert_eq!(Biome::from_name(b.name()), Some(b));
        }
        assert_eq!(Biome::from_name("DUNES"), Some(Biome::Desert));
        assert_eq!(Biome::from_name("snow"), Some(Biome::Tundra));
        assert_eq!(Biome::from_name("nope"), None);
    }

    /// D66: one dominant biome per world, temperate the most common.
    #[test]
    fn temperate_dominates_the_roll() {
        let mut counts: HashMap<Biome, usize> = HashMap::new();
        let n = 4000u64;
        for seed in 0..n {
            let b = Biome::pick(seed);
            assert!(b.is_live(), "seed {seed} rolled a stub: {b:?}");
            *counts.entry(b).or_default() += 1;
        }
        let t = counts[&Biome::Temperate];
        assert!(t as f32 / n as f32 > 0.28, "temperate only {t}/{n}");
        for (b, &c) in &counts {
            if *b != Biome::Temperate {
                assert!(c < t, "{b:?} ({c}) outnumbers temperate ({t})");
            }
        }
        // Every live land actually appears.
        for b in Biome::LIVE {
            assert!(*counts.get(&b).unwrap_or(&0) > 40, "{b:?} is too rare");
        }
    }

    #[test]
    fn roll_is_stable_and_seed_specific() {
        assert_eq!(Biome::pick(12345), Biome::pick(12345));
        let mut differed = false;
        for s in 0..200u64 {
            if Biome::pick(s) != Biome::pick(s + 1) {
                differed = true;
            }
        }
        assert!(differed);
    }

    #[test]
    fn neighbour_is_a_different_live_land() {
        for seed in 0..400u64 {
            let host = Biome::pick(seed);
            match host.neighbour(seed) {
                Some(n) => {
                    assert_ne!(n, host, "seed {seed}");
                    assert!(n.is_live());
                }
                None => {}
            }
        }
        // Both kinds of world exist: with a neighbour and without.
        let with = (0..200u64).filter(|s| Biome::pick(*s).neighbour(*s).is_some()).count();
        assert!(with > 60 && with < 190, "neighbour spread {with}/200");
    }

    #[test]
    fn blend_lives_at_one_edge_only() {
        let (w, h) = (120, 120);
        let seed = 99;
        let edge = Biome::blend_edge(seed);
        let mut touched = 0;
        for y in 0..h {
            for x in 0..w {
                let b = blend_weight(seed, edge, w, h, x, y);
                assert!((0.0..=1.0).contains(&b));
                if b > 0.0 {
                    touched += 1;
                    // Only ever inside the band at `edge`.
                    match edge {
                        Edge::Left => assert!(x < 41),
                        Edge::Right => assert!(x > w - 42),
                        Edge::Top => assert!(y < 41),
                        Edge::Bottom => assert!(y > h - 42),
                    }
                }
            }
        }
        assert!(touched > 500, "blend band is empty ({touched} tiles)");
        // The middle of the map is always pure host.
        assert_eq!(blend_weight(seed, edge, w, h, w / 2, h / 2), 0.0);
        // ...and there is real neighbour land somewhere in the band.
        let strong = (0..w)
            .flat_map(|x| (0..h).map(move |y| (x, y)))
            .filter(|&(x, y)| blend_weight(seed, edge, w, h, x, y) > 0.75)
            .count();
        assert!(strong > 40, "no tile is mostly the neighbour ({strong})");
    }

    #[test]
    fn blend_is_smooth_and_patchy() {
        let (w, h) = (140, 140);
        let seed = 7;
        let edge = Biome::blend_edge(seed);
        // Two neighbouring tiles never jump from nothing to everything.
        let mut jumps = 0;
        for y in 0..h {
            for x in 1..w {
                let a = blend_weight(seed, edge, w, h, x - 1, y);
                let b = blend_weight(seed, edge, w, h, x, y);
                if (a - b).abs() > 0.35 {
                    jumps += 1;
                }
            }
        }
        assert!(jumps < 80, "{jumps} hard edges in the blend band");
    }
}
