//! Which horizon a world gets (D64).
//!
//! The map is finite: its far edge lands on the bottom of the sky band, and
//! what stands *beyond* it is painted into the sky layer. The treatment is
//! picked from the seed, so a reset city gets a different edge of the world:
//!
//! * [`Horizon::Forest`] — a dense treeline thinning into hazy blue-grey hills;
//! * [`Horizon::Fields`] — farmland and meadow receding into haze, low hills;
//! * [`Horizon::Coast`] — the sea runs to the horizon (only on a map with a sea).
//!
//! `render` owns this because both the sky painter (`sky`, which depends on
//! `render`) and the scene need the same answer for one world.

/// A horizon treatment (D64), plus the three the biome overhaul adds (D66):
/// dunes on a desert, snowy ridges on a tundra, a jungle canopy on a tropical
/// world.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Horizon {
    Forest,
    Fields,
    Coast,
    /// Rolling dunes fading into a bleached dust haze.
    Dunes,
    /// Snow-covered ridges, pale and cold.
    SnowRidges,
    /// A dense canopy wall with palms standing out of it.
    JungleCanopy,
    /// B46e: sharp snow-capped peaks over a dark conifer foot.
    Alpine,
    /// B46e: low golden swells under a pale haze, flat-topped trees on them.
    Savanna,
    /// B46e: a low cypress line lost in mist.
    Swamp,
    /// B46e: rolling heather hills, purple-grey in the haze.
    Highlands,
}

impl Default for Horizon {
    /// The historical look: a treeline on the horizon.
    fn default() -> Self {
        Horizon::Forest
    }
}

impl Horizon {
    /// Deterministic choice for a world. `has_sea` gates the coast: a landlocked
    /// map falls back to fields (a sea horizon over dry land reads as a bug).
    pub fn pick(seed: u64, has_sea: bool) -> Self {
        // SplitMix64 finaliser, so neighbouring seeds land on different edges.
        let mut x = seed
            .wrapping_add(0x9E37_79B9_7F4A_7C15)
            .wrapping_mul(0xBF58_476D_1CE4_E5B9);
        x ^= x >> 30;
        x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
        x ^= x >> 31;
        match x % 3 {
            0 => Horizon::Forest,
            1 => Horizon::Fields,
            _ => {
                if has_sea {
                    Horizon::Coast
                } else {
                    Horizon::Fields
                }
            }
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Horizon::Forest => "forest",
            Horizon::Fields => "fields",
            Horizon::Coast => "coast",
            Horizon::Dunes => "dunes",
            Horizon::SnowRidges => "snow ridges",
            Horizon::JungleCanopy => "jungle canopy",
            Horizon::Alpine => "alpine peaks",
            Horizon::Savanna => "savanna",
            Horizon::Swamp => "swamp",
            Horizon::Highlands => "highlands",
        }
    }

    /// The horizon a biome world reads best against (D66). The biome's own
    /// treatment mostly wins; a temperate world keeps D64's per-seed roll, and a
    /// desert with a sea still gets dunes (a beach is a beach, but the land
    /// behind it is still desert).
    pub fn for_biome(biome: map::Biome, seed: u64, has_sea: bool) -> Horizon {
        match biome {
            map::Biome::Desert => Horizon::Dunes,
            map::Biome::Tundra => Horizon::SnowRidges,
            map::Biome::Alpine => Horizon::Alpine,
            map::Biome::Savanna | map::Biome::Steppe => Horizon::Savanna,
            map::Biome::Swamp => Horizon::Swamp,
            map::Biome::Highlands => Horizon::Highlands,
            map::Biome::Islands | map::Biome::Lagoon => {
                if has_sea {
                    Horizon::Coast
                } else {
                    Horizon::pick(seed, false)
                }
            }
            map::Biome::Tropical => {
                if has_sea {
                    Horizon::Coast
                } else {
                    Horizon::JungleCanopy
                }
            }
            _ => Horizon::pick(seed, has_sea),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_treatment_appears_over_the_seeds() {
        let mut f = 0;
        let mut g = 0;
        let mut c = 0;
        for seed in 1..=90u64 {
            match Horizon::pick(seed, true) {
                Horizon::Forest => f += 1,
                Horizon::Fields => g += 1,
                Horizon::Coast => c += 1,
                // A temperate world's roll never picks a biome horizon (D66).
                _ => unreachable!("temperate roll returned a biome horizon"),
            }
        }
        assert!(f > 10 && g > 10 && c > 10, "lopsided pick: {f}/{g}/{c}");
    }

    #[test]
    fn a_landlocked_map_never_gets_coast() {
        for seed in 1..=200u64 {
            assert_ne!(Horizon::pick(seed, false), Horizon::Coast);
        }
    }

    #[test]
    fn the_pick_is_stable() {
        assert_eq!(Horizon::pick(42, true), Horizon::pick(42, true));
    }

    #[test]
    fn each_biome_gets_its_own_horizon() {
        assert_eq!(Horizon::for_biome(map::Biome::Desert, 3, true), Horizon::Dunes);
        assert_eq!(Horizon::for_biome(map::Biome::Tundra, 3, false), Horizon::SnowRidges);
        assert_eq!(
            Horizon::for_biome(map::Biome::Tropical, 3, true),
            Horizon::Coast
        );
        assert_eq!(
            Horizon::for_biome(map::Biome::Tropical, 3, false),
            Horizon::JungleCanopy
        );
    }
}
