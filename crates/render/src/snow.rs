//! B76 round 3: snow as a property of the **surface it lies on** (D84, D85).
//!
//! Round 2 capped the top rows of a building sprite, which on a tower is its
//! crown: the crown's vertical faces turned white and read as a pale pyramid
//! floating over the roof while the roof deck stayed bare grey. The builder
//! knows every surface it paints, so it **tags** each pixel as it writes it
//! ([`Sprite::stamp`] in `buildkit`) and this module decides what snow does on
//! that kind of surface:
//!
//! * a flat deck takes an even layer, the steeper a slope the thinner the load,
//! * a ridge or hip crest keeps its own (darker) line, so the ridge reads,
//! * a coping, sill or awning rail gets a 1-px white lip,
//! * a chimney, tank or AC box pokes out dark with snow on its **top** face,
//! * a roof that is heated melts first, leaving wet dark patches.
//!
//! Nothing here is geometry above a roof: only pixels the builder already drew
//! can be painted, which is what keeps snow inside the building's silhouette.
//!
//! The tags are four bits wide and travel to the renderer packed two pixels to
//! a byte ([`pack`]), because the sprite cache is kept small (D14).

use crate::palette::{self, Color};

/// Not a surface that holds snow: a wall, a window, a fence, the ground.
pub const NONE: u8 = 0;
/// A horizontal roof deck or terrace.
pub const FLAT: u8 = 1;
/// A roof slope under ~20°: snow lies on it almost like on a deck.
pub const SLOPE_GENTLE: u8 = 2;
/// A roof slope around 20–40°.
pub const SLOPE_MEDIUM: u8 = 3;
/// A roof slope over ~40°: only a thin, broken load stays.
pub const SLOPE_STEEP: u8 = 4;
/// A ridge or hip crest: it keeps its own line so the roof still reads.
pub const RIDGE: u8 = 5;
/// A 1-px top edge of something a layer can lie on: a sill, an awning, a
/// balcony rail, a parapet coping, a sign's hood.
pub const LEDGE: u8 = 6;
/// The top face of a chimney, tank, AC box or stair head.
pub const EQUIP_TOP: u8 = 7;
/// The side of that equipment: snow does not stick to a vertical face, so a
/// stack pokes out of the load instead of wearing a hat.
pub const EQUIP_SIDE: u8 = 8;
/// A leafy top (a rooftop plant, a hedge in a plot): a light cap, not a slab.
pub const FOLIAGE: u8 = 9;

/// Cold white: the colour a full load takes in the open.
const SNOW: Color = 0xFFF4F8FF;
/// The shaded side of a drift (a lip's underside, an edge in shadow).
const SNOW_SHADE: Color = 0xFFDCE6F2;
/// Wet paving showing through a melted patch.
const WET: Color = 0xFF5C6166;

/// How much of the open load this kind of surface keeps.
#[inline]
fn share(tag: u8) -> f32 {
    match tag {
        FLAT => 1.0,
        SLOPE_GENTLE => 0.97,
        SLOPE_MEDIUM => 0.88,
        // Even a steep pitch carries a load in a real winter; it is the *ridges*
        // and the courses that show through, not the roof's own colour.
        SLOPE_STEEP => 0.72,
        RIDGE => 0.0,
        LEDGE => 1.0,
        EQUIP_TOP => 0.92,
        FOLIAGE => 0.80,
        _ => 0.0,
    }
}

/// Is this a surface snow can lie on at all?
#[inline]
pub fn holds_snow(tag: u8) -> bool {
    holds(tag)
}

#[inline]
fn holds(tag: u8) -> bool {
    matches!(
        tag,
        FLAT | SLOPE_GENTLE | SLOPE_MEDIUM | SLOPE_STEEP | RIDGE | LEDGE | EQUIP_TOP | FOLIAGE
    )
}

/// A cheap deterministic 0..1 from a pixel, so a load is mottled and a thin one
/// is patchy rather than a flat wash (no RNG: the same city always agrees).
///
/// The cell is 3 px wide: at 1× a per-pixel hash reads as static, while a load
/// that is uneven in ~3-px patches reads as snow.
#[inline]
fn grain(x: i32, y: i32) -> f32 {
    let (x, y) = (x.div_euclid(3), y.div_euclid(3));
    let mut h = (x as u32).wrapping_mul(0x9E37_79B1) ^ (y as u32).wrapping_mul(0x85EB_CA77);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2545_F491);
    h ^= h >> 13;
    (h & 0xFFFF) as f32 / 65535.0
}

/// The colour a tagged pixel takes under `cover` of lying snow, or `None` to
/// leave the pixel exactly as the builder drew it.
///
/// `heat` (0..1) is a roof that warms from inside: it thaws first, so its load
/// is thinner and wet paving shows through in patches.
pub fn on_tag(tag: u8, base: Color, cover: f32, heat: f32, x: i32, y: i32) -> Option<Color> {
    if cover <= 0.0 || !holds(tag) {
        return None;
    }
    let g = grain(x, y);
    let base = palette::with_alpha(base, 0xFF);
    match tag {
        LEDGE => {
            // A lip: it holds a white edge from the first flake, and it is the
            // brightest line on the building once the snow is deep.
            if cover < 0.12 {
                return None;
            }
            let t = (0.55 + 0.45 * cover).min(0.97);
            Some(palette::mix(base, SNOW, t))
        }
        RIDGE => {
            // The crest keeps its own line (a darker grey than the load around
            // it), so a gable still reads as a gable under snow.
            Some(palette::mix(base, SNOW, 0.30 + 0.25 * cover))
        }
        EQUIP_TOP => {
            let t = share(tag) * cover;
            Some(palette::mix(base, SNOW, (t * 0.96).min(0.96)))
        }
        _ => {
            // A thin load is patchy: right after it starts, the deck shows
            // through in patches and the load only builds where it is uneven.
            let mut c = cover * share(tag);
            if tag == FLAT {
                c *= 1.0 - 0.35 * heat;
                if heat > 0.35 && g > 0.62 {
                    // Melted back to wet paving.
                    return Some(palette::mix(base, WET, (0.30 + 0.35 * heat).min(0.6)));
                }
            }
            if c < 0.55 && g > 0.30 + 1.3 * c {
                // Bare ground patch: no paint, the surface's own colour stays.
                return None;
            }
            // The load is nearly even, with a cold-grey drift line here and
            // there, so it reads as snow rather than as a white wash or static.
            let shade = if g < 0.16 { SNOW_SHADE } else { SNOW };
            let t = (c * (0.96 + 0.04 * g)).clamp(0.0, 0.98);
            Some(palette::mix(base, shade, t))
        }
    }
}

/// Which slope tag a roof of this pitch takes: `rise` pixels over a half-span
/// of `half_px` pixels. A steep gable keeps a thin load, a shallow hip nearly
/// as much as a deck — the roof's own numbers decide, not a guess.
pub fn slope_tag(rise: f32, half_px: f32) -> u8 {
    let pitch = (rise / half_px.max(1.0)).max(0.0);
    // 35 % pitch lands near 0.7 here, 105 % near 2.1.
    if pitch < 0.85 {
        SLOPE_GENTLE
    } else if pitch < 1.5 {
        SLOPE_MEDIUM
    } else {
        SLOPE_STEEP
    }
}

/// How warm a building's roof runs from inside (0..1): the object's own rule
/// (`CityObj::heat`), so the kit path and the engine path never disagree.
pub fn heat_of(o: &crate::city::CityObj) -> f32 {
    o.heat()
}

/// A drift: the extra load that piles against a parapet, a wall or a rooftop
/// box. It is painted on the snow that is already there, so it only deepens an
/// edge that reads as snow itself.
pub fn drift(base: Color, cover: f32, heat: f32) -> Color {
    let t = (0.20 + 0.28 * cover) * (1.0 - 0.5 * heat);
    palette::mix(palette::with_alpha(base, 0xFF), SNOW, t.clamp(0.0, 0.55))
}

/// Pack a tag plane two pixels to a byte (four bits each).
pub fn pack(tags: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; (tags.len() + 1) / 2];
    for (i, &t) in tags.iter().enumerate() {
        let t = t & 0x0F;
        if i % 2 == 0 {
            out[i / 2] |= t;
        } else {
            out[i / 2] |= t << 4;
        }
    }
    out
}

/// Read one tag out of a packed plane.
#[inline]
pub fn get(packed: &[u8], i: usize) -> u8 {
    match packed.get(i / 2) {
        Some(b) => {
            if i % 2 == 0 {
                b & 0x0F
            } else {
                b >> 4
            }
        }
        None => NONE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_deck_takes_the_load_and_a_chimney_wall_does_not() {
        let grey = palette::rgb(0x70, 0x74, 0x7a);
        assert!(on_tag(FLAT, grey, 0.9, 0.0, 3, 4).is_some());
        assert!(on_tag(NONE, grey, 0.9, 0.0, 3, 4).is_none());
        assert!(on_tag(EQUIP_SIDE, grey, 0.9, 0.0, 3, 4).is_none());
    }

    #[test]
    fn a_steep_slope_keeps_less_than_a_gentle_one() {
        let grey = palette::rgb(0x70, 0x74, 0x7a);
        let light_load = |t| on_tag(t, grey, 0.8, 0.0, 9, 9).unwrap_or(grey);
        let lum = |c: Color| (0.30 * ((c >> 16) & 0xFF) as f32 + 0.59 * ((c >> 8) & 0xFF) as f32 + 0.11 * (c & 0xFF) as f32);
        assert!(lum(light_load(SLOPE_GENTLE)) > lum(light_load(SLOPE_STEEP)));
    }

    #[test]
    fn a_thin_load_is_patchy_and_a_deep_one_is_not() {
        let grey = palette::rgb(0x70, 0x74, 0x7a);
        let mut bare = 0;
        let mut covered = 0;
        for y in 0..32 {
            for x in 0..32 {
                if on_tag(FLAT, grey, 0.15, 0.0, x, y).is_none() {
                    bare += 1;
                }
                if on_tag(FLAT, grey, 1.0, 0.0, x, y).is_some() {
                    covered += 1;
                }
            }
        }
        assert!(bare > 250, "a 15% load should leave most of the deck showing, {bare}");
        assert_eq!(covered, 1024, "a full load covers every deck pixel");
    }

    #[test]
    fn a_heated_roof_melts_and_shows_wet_patches() {
        let grey = palette::rgb(0x70, 0x74, 0x7a);
        let cold = on_tag(FLAT, grey, 0.9, 0.0, 5, 5).unwrap();
        let warm = on_tag(FLAT, grey, 0.9, 1.0, 5, 5).unwrap();
        let lum = |c: Color| 0.30 * ((c >> 16) & 0xFF) as f32 + 0.59 * ((c >> 8) & 0xFF) as f32 + 0.11 * (c & 0xFF) as f32;
        assert!(lum(cold) > lum(warm));
    }

    #[test]
    fn the_tag_plane_survives_packing() {
        let tags = [FLAT, RIDGE, LEDGE, EQUIP_TOP, NONE, FOLIAGE, SLOPE_STEEP, SLOPE_MEDIUM, FLAT];
        let p = pack(&tags);
        let back: Vec<u8> = (0..tags.len()).map(|i| get(&p, i)).collect();
        assert_eq!(back, tags);
        assert_eq!(p.len(), 5);
    }
}
