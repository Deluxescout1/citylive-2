//! B74: the small tree species and the town's planting rhythm, shared by the
//! sim's audit and the renderer's draw-time planting (the render crate cannot
//! depend on the sim, so the rules live here, next to the species they name).

use crate::types::TreeSpecies;

/// The species whose crown is narrow enough to live in a wall's margin: the
/// brief's list (fruit trees, birch, small maples, ornamental conifers), measured
/// from the crown table in `render::terrain::crown_for` (all at or under 18 px
/// before the individual's jitter).
pub fn is_small_species(s: TreeSpecies) -> bool {
    matches!(
        s,
        TreeSpecies::Apple
            | TreeSpecies::Cherry
            | TreeSpecies::Birch
            | TreeSpecies::JapaneseMaple
            | TreeSpecies::Olive
            | TreeSpecies::Cypress
    )
}

/// The small species a garden tree or a street tree may be, per land (a palm on
/// a temperate street would look like a mistake).
pub const GARDEN: [TreeSpecies; 4] = [
    TreeSpecies::Apple,
    TreeSpecies::Cherry,
    TreeSpecies::Birch,
    TreeSpecies::JapaneseMaple,
];

/// One street tree every this many tiles along a street (the brief's "regular
/// spacing").
pub const STREET_PITCH: i32 = 3;
