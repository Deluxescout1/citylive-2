//! Shared iso geometry. The renderer imports these, so the projection, the
//! world size and the coverage tests can never disagree.

/// Tile diamond width in pixels (D61: 64-px art shown at 1×).
pub const TILE_W: i32 = 64;
/// Tile diamond height in pixels (2:1 projection).
pub const TILE_H: i32 = 32;
pub const HALF_W: i32 = TILE_W / 2;
pub const HALF_H: i32 = TILE_H / 2;

/// Art detail multiplier: how many pixels one pixel of the original 32-px art
/// became in D61's 64-px art. Draw code multiplies hand-tuned pixel sizes by
/// this instead of carrying a second set of numbers.
pub const DETAIL: i32 = TILE_W / 32;

/// Pixels per terrain level. Deliberately smaller than `HALF_H`: with 16 px per
/// level a one-level ramp would rise exactly as much as the projection drops
/// across one tile, so the ramp would come out screen-horizontal and invisible.
/// At 12 px a ramp reads as a gentle slope and a step as a 12 px terrace.
pub const HEIGHT_UNIT: i32 = 12;
