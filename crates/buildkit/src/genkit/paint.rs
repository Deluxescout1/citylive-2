//! B73b/B73c: facades, bases and the shared window painting.
//!
//! Every wall a design draws goes through [`paint`], which picks the facade
//! system's painter and paints the whole face: wall, windows, frames, mullions,
//! balconies, screens — and the evening, using the same lit-window rules as the
//! rest of the catalogue (`parts::draw_window`'s hash and `lit_p * night^2`
//! share, and `parts::paint_glass_curtain_tinted` for glass), so the window
//! lights stay B72/D78's business and a generated tower looks like the city.
//!
//! B73c reuses [`pane`], [`punch_grid`] and [`mats`] for its own families; the
//! facade systems themselves are material-driven and not tower-specific.

use render::palette::{Color, rgb};

use super::{Base, Design, Facade, Material};
use crate::color::{self, pal, FACE_TOP};
use crate::geom::Quad;
use crate::parts;
use crate::types::{Light, Sprite};

/// A material's palette: wall, trim, glass, metal and the mortar/panel tones the
/// facade systems need.
#[derive(Clone, Copy, Debug)]
pub struct Mats {
    pub wall: Color,
    pub trim: Color,
    pub glass: Color,
    pub glass_hi: Color,
    pub metal: Color,
    pub mortar: Color,
    pub panel: Color,
}

/// The palette of a material.
pub fn mats(m: Material) -> Mats {
    match m {
        Material::Limestone => Mats {
            wall: pal::STONE_LIMESTONE,
            trim: pal::TRIM_CREAM,
            glass: pal::GLASS_DARK,
            glass_hi: pal::GLASS_REFLECTIVE,
            metal: pal::METAL_STEEL_DARK,
            mortar: pal::STONE_GRANITE,
            panel: pal::CONCRETE_LIGHT,
        },
        Material::Sandstone => Mats {
            wall: rgb(0xd8, 0xc2, 0xa0),
            trim: pal::TRIM_CREAM,
            glass: pal::GLASS_BRONZE,
            glass_hi: rgb(0xa8, 0x92, 0x74),
            metal: pal::ROOF_COPPER_NEW,
            mortar: rgb(0xb8, 0xa2, 0x84),
            panel: pal::STUCCO_SAND,
        },
        Material::RedBrick => Mats {
            wall: pal::BRICK_RED,
            trim: pal::STONE_LIMESTONE,
            glass: pal::GLASS_DARK,
            glass_hi: pal::GLASS_REFLECTIVE,
            metal: pal::METAL_STEEL_DARK,
            mortar: pal::BRICK_MORTAR,
            panel: pal::STONE_GRANITE,
        },
        Material::BuffBrick => Mats {
            wall: pal::BRICK_BUFF,
            trim: pal::TRIM_CREAM,
            glass: pal::GLASS_DARK,
            glass_hi: pal::GLASS_REFLECTIVE,
            metal: pal::METAL_STEEL_DARK,
            mortar: pal::BRICK_LIGHT_MORTAR,
            panel: pal::STUCCO_OCHRE,
        },
        Material::Brownstone => Mats {
            wall: pal::STONE_BROWNSTONE,
            trim: pal::STONE_LIMESTONE,
            glass: pal::GLASS_DARK,
            glass_hi: pal::GLASS_REFLECTIVE,
            metal: pal::METAL_STEEL_DARK,
            mortar: rgb(0x5a, 0x3a, 0x30),
            panel: pal::STONE_GRANITE,
        },
        Material::Concrete => Mats {
            wall: pal::CONCRETE_GREY,
            trim: pal::CONCRETE_LIGHT,
            glass: pal::GLASS_DARK,
            glass_hi: pal::GLASS_REFLECTIVE,
            metal: pal::METAL_STEEL_DARK,
            mortar: pal::CONCRETE_GREY,
            panel: pal::CONCRETE_LIGHT,
        },
        Material::WhitePanel => Mats {
            wall: pal::METAL_WHITE,
            trim: pal::TRIM_WHITE,
            glass: pal::GLASS_BLUE_SKY,
            glass_hi: pal::GLASS_REFLECTIVE,
            metal: pal::METAL_STEEL_DARK,
            mortar: pal::CONCRETE_LIGHT,
            panel: pal::CONCRETE_LIGHT,
        },
        Material::SilverSteel => Mats {
            // Cool and metallic: at 1x a warm grey stone and a steel tower look
            // the same, and a downtown is mostly glass and steel.
            wall: rgb(0xa8, 0xb2, 0xc0),
            trim: rgb(0xe0, 0xe6, 0xee),
            glass: pal::GLASS_REFLECTIVE,
            glass_hi: rgb(0xd8, 0xe6, 0xf4),
            metal: rgb(0x4a, 0x52, 0x5e),
            mortar: rgb(0x8a, 0x94, 0xa2),
            panel: rgb(0xc0, 0xc8, 0xd2),
        },
        Material::Champagne => Mats {
            wall: pal::METAL_CHAMPAGNE,
            trim: rgb(0xe8, 0xdc, 0xc6),
            glass: pal::GLASS_BRONZE,
            glass_hi: rgb(0xb0, 0x9c, 0x80),
            metal: rgb(0x8a, 0x7c, 0x64),
            mortar: rgb(0xa8, 0x9c, 0x88),
            panel: pal::CONCRETE_LIGHT,
        },
        Material::Bronze => Mats {
            wall: pal::ROOF_COPPER_NEW,
            trim: rgb(0xd0, 0x9a, 0x70),
            glass: pal::GLASS_BRONZE,
            glass_hi: rgb(0xa8, 0x8e, 0x74),
            metal: rgb(0x6a, 0x4c, 0x38),
            mortar: rgb(0x8a, 0x5a, 0x3c),
            panel: pal::CONCRETE_LIGHT,
        },
        Material::BlackGranite => Mats {
            wall: pal::STONE_DARK_SLATE,
            trim: rgb(0x7a, 0x80, 0x8a),
            glass: pal::GLASS_DARK,
            glass_hi: pal::GLASS_REFLECTIVE,
            metal: pal::METAL_STEEL_DARK,
            mortar: rgb(0x3e, 0x42, 0x4a),
            panel: rgb(0x6a, 0x70, 0x78),
        },
        Material::Copper => Mats {
            wall: pal::ROOF_COPPER_PATINA,
            trim: pal::ROOF_COPPER_NEW,
            glass: rgb(0x5c, 0x9e, 0x94),
            glass_hi: rgb(0xa0, 0xd0, 0xc4),
            metal: rgb(0x8a, 0x62, 0x44),
            mortar: rgb(0x7a, 0x7a, 0x58),
            panel: pal::CONCRETE_LIGHT,
        },
        Material::GlassBlue => Mats {
            wall: rgb(0x74, 0xa0, 0xc8),
            trim: pal::METAL_WHITE,
            glass: pal::GLASS_BLUE_SKY,
            glass_hi: pal::GLASS_REFLECTIVE,
            metal: pal::METAL_STEEL_DARK,
            mortar: rgb(0x8a, 0xa8, 0xc4),
            panel: pal::METAL_WHITE,
        },
        Material::GlassGreen => Mats {
            wall: rgb(0x6c, 0xa8, 0x9c),
            trim: pal::METAL_WHITE,
            glass: rgb(0x5c, 0x96, 0x8c),
            glass_hi: rgb(0x9c, 0xd0, 0xc4),
            metal: pal::METAL_STEEL_DARK,
            mortar: rgb(0x84, 0xb0, 0xa6),
            panel: pal::METAL_WHITE,
        },
        Material::GlassBronze => Mats {
            wall: rgb(0x86, 0x74, 0x64),
            trim: rgb(0xd8, 0xc8, 0xb0),
            glass: pal::GLASS_BRONZE,
            glass_hi: rgb(0x94, 0x80, 0x6a),
            metal: rgb(0x5a, 0x4c, 0x40),
            mortar: rgb(0x9a, 0x88, 0x76),
            panel: pal::CONCRETE_LIGHT,
        },
        Material::GlassGrey => Mats {
            wall: rgb(0x74, 0x80, 0x8c),
            trim: pal::METAL_WHITE,
            glass: pal::GLASS_DARK,
            glass_hi: rgb(0x8c, 0x9c, 0xac),
            metal: pal::METAL_STEEL_DARK,
            mortar: rgb(0x8a, 0x92, 0x9c),
            panel: pal::CONCRETE_LIGHT,
        },
    }
}

#[inline]
fn depth_at(q: &Quad, v: i32) -> u16 {
    let rows = q.rows().max(1);
    (q.z_top as f32 + (q.z_base as f32 - q.z_top as f32) * (v as f32 / rows as f32)) as u16
}

/// A sign's own colour, from the lot seed: invented, never a brand (D75).
fn sign_color(seed: u64) -> Color {
    match seed % 6 {
        0 => rgb(0xc8, 0x3a, 0x3a),
        1 => rgb(0x2e, 0x6a, 0xb8),
        2 => rgb(0x2e, 0x8a, 0x5c),
        3 => rgb(0xb8, 0x7c, 0x24),
        4 => rgb(0x7a, 0x3c, 0xa8),
        _ => rgb(0x2e, 0x8a, 0x9a),
    }
}

/// One window in a wall, painted with the catalogue's own window-light rule:
/// the hash is `hash_seed(seed, u * 13 + v * 37)` and the lit share is
/// `lit_p * night^2`, exactly as `parts::draw_window` chooses it (B72/D78), so a
/// generated tower's lights come on with the rest of the city and never all at
/// once.
#[allow(clippy::too_many_arguments)]
pub fn pane(
    q: &Quad,
    sprite: &mut Sprite,
    light: &Light,
    u0: i32,
    v0: i32,
    w: i32,
    h: i32,
    frame: Color,
    m: Mats,
    lit_p: f32,
    seed: u64,
    side: f32,
) {
    if u0 < 1 || v0 < 1 || u0 + w + 1 > q.top_len() || v0 + h + 1 > q.rows() || w < 3 || h < 3 {
        return;
    }
    let night = light.night.clamp(0.0, 1.0);
    let lit = if night > 0.0 {
        let hash = color::hash_seed(seed, (u0 * 13 + v0 * 37) as u32);
        ((hash % 100) as f32) < lit_p * 100.0 * night * night
    } else {
        false
    };
    let glow = if lit {
        let warm = (color::hash_seed(seed, u0 as u32) % 3) != 0;
        let c = if warm { color::WIN_WARM } else { color::WIN_COOL };
        parts::emissive(color::shade(c, 0.55 + 0.45 * night))
    } else {
        0
    };
    // The pane is glass whatever the wall is: on a bronze or a buff-brick wall a
    // pane painted in the material's own glass would vanish into it.
    let glass = if lit {
        0
    } else {
        color::mix(
            color::mix(m.glass, color::WIN_OFF_DAY, 0.55),
            color::WIN_OFF_NIGHT,
            night,
        )
    };
    let frame_c = color::surf(color::shade(frame, 0.88), light, side);
    let nq = q.sub_px(u0, v0, u0 + w, v0 + h, q.rows() as f32);
    let z = q.z_top;
    nq.paint(sprite, &mut |u, v, _x, _y| {
        let depth = (z - v0 - v).max(0) as u16;
        let border = u == 0 || u >= w - 1 || v == 0 || v >= h - 1;
        if border {
            return Some((frame_c, depth));
        }
        if lit {
            Some((glow, depth))
        } else {
            // A sliver of sky reflected in the pane, as the catalogue's own
            // windows have.
            let c = if (u + v) % 5 == 0 { m.glass_hi } else { glass };
            Some((color::surf(c, light, side), depth))
        }
    });
}

/// Punch a grid of windows through a wall that has already been painted.
#[allow(clippy::too_many_arguments)]
pub fn punch_grid(
    q: &Quad,
    sprite: &mut Sprite,
    light: &Light,
    m: Mats,
    seed: u64,
    win: (i32, i32),
    bay: i32,
    floor_h: i32,
    frame: Color,
    lit_p: f32,
    side: f32,
    sill: bool,
) {
    let rows = q.rows();
    let len = q.top_len();
    let mut v = 3;
    while v + win.1 + 2 < rows {
        let mut u = 2;
        while u + win.0 + 1 < len {
            pane(q, sprite, light, u, v, win.0, win.1, frame, m, lit_p, seed, side);
            if sill {
                let sq = q.sub_px(u - 1, v + win.1, u + win.0, v + win.1 + 1, rows as f32);
                sq.fill(sprite, color::surf(color::shade(frame, 1.08), light, FACE_TOP));
            }
            u += bay;
        }
        v += floor_h;
    }
}

/// Paint one whole face in the design's facade system.
pub fn paint(f: Facade, q: &Quad, sprite: &mut Sprite, light: &Light, d: &Design, salt: u32, side: f32) {
    // B76 r3: a facade is a wall: whatever surface was tagged before (a roof
    // plane, a terrace) does not carry on into it.
    sprite.on_surface(render::snow::NONE);
    let mut m = mats(d.material);
    // B73c: a painted wall wears the design's own colour over the material's.
    if d.wall != 0 {
        m.wall = d.wall as Color;
    }
    let seed = d.seed ^ (salt as u64).wrapping_mul(0x9E37_79B9);
    let rows = q.rows();
    let len = q.top_len();
    let floor_h = d.floor.max(3);
    match f {
        Facade::Curtain => {
            parts::paint_glass_curtain_tinted(q, sprite, light, floor_h, 6, side, seed, m.glass, m.glass_hi)
        }
        Facade::PunchedStone => {
            parts::paint_stone_ashlar(q, sprite, light, m.wall, side, d.age);
            punch_grid(q, sprite, light, m, seed, (4, 6), 9, floor_h.max(5), m.trim, 0.55, side, true);
        }
        Facade::BrickTrim => {
            parts::paint_brick(q, sprite, light, m.wall, m.mortar, side, d.age);
            // A stone belt course every fourth floor.
            let mut v = 0;
            while v + 2 < rows {
                if (v / floor_h.max(1)) % 4 == 3 {
                    let bq = q.sub_px(0, v, len, v + 2, rows as f32);
                    bq.fill(sprite, color::surf(m.trim, light, side));
                }
                v += floor_h;
            }
            punch_grid(q, sprite, light, m, seed, (3, 5), 7, floor_h.max(4), m.trim, 0.55, side, true);
        }
        Facade::StripWindows => {
            let night = light.night.clamp(0.0, 1.0);
            q.paint(sprite, &mut |u, v, x, y| {
                let depth = depth_at(q, v);
                let spandrel = (v % floor_h) < 2;
                let mull = (u % 6) == 0;
                if spandrel {
                    return Some((color::surf(m.panel, light, side), depth));
                }
                if mull {
                    return Some((color::surf(m.metal, light, side), depth));
                }
                let fl = v / floor_h;
                let px = u / 6;
                let hash = color::hash_seed(seed, (px * 31 + fl * 57) as u32);
                if ((hash % 100) as f32) < 36.0 * night {
                    let warm = (hash % 3) != 0;
                    let c = if warm { color::WIN_WARM } else { color::WIN_COOL };
                    return Some((parts::emissive(color::shade(c, 0.55 + 0.45 * night)), depth));
                }
                let diag = (x * 2 - y) / 7;
                let c = if diag.rem_euclid(5) == 0 { m.glass_hi } else { m.glass };
                Some((color::surf(color::mix(c, color::WIN_OFF_NIGHT, night * night * 0.5), light, side), depth))
            });
        }
        Facade::VerticalFins => {
            parts::paint_glass_curtain_tinted(q, sprite, light, floor_h, 8, side, seed, m.glass, m.glass_hi);
            let fin = color::surf(m.metal, light, side);
            let hi = color::surf(color::shade(m.metal, 1.35), light, side);
            q.paint(sprite, &mut |u, v, _x, _y| {
                let depth = depth_at(q, v);
                if u % 7 == 0 {
                    Some((hi, depth))
                } else if u % 7 == 1 {
                    Some((fin, depth))
                } else {
                    None
                }
            });
        }
        Facade::Louvres => {
            parts::paint_glass_curtain_tinted(q, sprite, light, floor_h, 7, side, seed, m.glass, m.glass_hi);
            let lo = color::surf(m.metal, light, side);
            let hi = color::surf(color::shade(m.metal, 1.3), light, side);
            q.paint(sprite, &mut |_u, v, _x, _y| {
                let depth = depth_at(q, v);
                match v % 3 {
                    0 => Some((hi, depth)),
                    1 => Some((lo, depth)),
                    _ => None,
                }
            });
        }
        Facade::Balconies => {
            parts::paint_stucco(q, sprite, light, m.wall, side, d.age);
            punch_grid(q, sprite, light, m, seed, (3, 4), 7, floor_h.max(4), m.metal, 0.65, side, false);
            // A balcony slab and rail under every floor's windows.
            let rail = color::surf(m.metal, light, side);
            let slab = color::surf(color::shade(m.panel, 0.9), light, FACE_TOP);
            q.paint(sprite, &mut |u, v, _x, _y| {
                let depth = depth_at(q, v);
                if v % floor_h == floor_h - 3 {
                    return Some((slab, depth));
                }
                if v % floor_h == floor_h - 2 {
                    return Some((rail, depth));
                }
                if v % floor_h == floor_h - 1 && u % 2 == 0 {
                    return Some((rail, depth));
                }
                None
            });
        }
        Facade::Diagrid => {
            parts::paint_glass_curtain_tinted(q, sprite, light, floor_h, 6, side, seed, m.glass, m.glass_hi);
            let steel = color::surf(color::shade(m.metal, 1.65), light, side);
            q.paint(sprite, &mut |u, v, _x, _y| {
                let depth = depth_at(q, v);
                if ((u + v) % 14) <= 1 || ((u as i32 - v).rem_euclid(14)) <= 1 {
                    Some((steel, depth))
                } else {
                    None
                }
            });
        }
        Facade::Exoskeleton => {
            parts::paint_glass_curtain_tinted(q, sprite, light, floor_h, 6, side, seed, m.glass, m.glass_hi);
            let steel = color::surf(color::shade(m.metal, 1.6), light, side);
            let bright = color::surf(color::shade(m.metal, 1.85), light, side);
            let len2 = (len * 2).max(1) as f32;
            q.paint(sprite, &mut |u, v, _x, _y| {
                let depth = depth_at(q, v);
                // Columns every 9 pixels, and an X per three floors.
                if u % 18 <= 1 {
                    return Some((bright, depth));
                }
                let u_rel = u as f32 / len2;
                let v_rel = (v % (floor_h * 3)) as f32 / (floor_h * 3) as f32;
                if (u_rel - v_rel).abs() < 0.035 || (u_rel - (1.0 - v_rel)).abs() < 0.035 {
                    return Some((steel, depth));
                }
                None
            });
        }
        Facade::Brutalist => {
            parts::paint_stucco(q, sprite, light, m.wall, side, d.age);
            let dark = color::surf(rgb(0x2a, 0x30, 0x38), light, side);
            let pier = color::surf(color::shade(m.wall, 1.14), light, side);
            let night = light.night.clamp(0.0, 1.0);
            q.paint(sprite, &mut |u, v, _x, _y| {
                let depth = depth_at(q, v);
                let band = v % floor_h;
                let pierpos = u % 10;
                if pierpos <= 1 {
                    return Some((pier, depth));
                }
                // A deep reveal: the top of the opening is in shadow, the sill
                // catches the sky.
                if band == 1 {
                    return Some((dark, depth));
                }
                if band == 2 || band == 3 || band == 4 {
                    let fl = v / floor_h;
                    let hash = color::hash_seed(seed, (u / 10 * 29 + fl * 71) as u32);
                    if ((hash % 100) as f32) < 30.0 * night {
                        let warm = (hash % 4) != 0;
                        let c = if warm { color::WIN_WARM } else { color::WIN_COOL };
                        return Some((parts::emissive(color::shade(c, 0.5 + 0.4 * night)), depth));
                    }
                    return Some((color::surf(color::mix(m.glass, color::WIN_OFF_NIGHT, night), light, side), depth));
                }
                Some((pier, depth))
            });
        }
        Facade::Sawtooth => {
            parts::paint_stucco(q, sprite, light, m.wall, side, d.age);
            let night = light.night.clamp(0.0, 1.0);
            q.paint(sprite, &mut |u, v, _x, _y| {
                let depth = depth_at(q, v);
                // Vertical saw teeth: alternate faces catch and lose the light.
                let tooth = u % 8;
                let lit_face = tooth < 4;
                if (v % floor_h) >= floor_h - 2 {
                    return None; // the spandrel stays wall
                }
                let fl = v / floor_h;
                let hash = color::hash_seed(seed, (u / 8 * 23 + fl * 47) as u32);
                if lit_face && ((hash % 100) as f32) < 28.0 * night {
                    let c = if hash % 3 != 0 { color::WIN_WARM } else { color::WIN_COOL };
                    return Some((parts::emissive(color::shade(c, 0.55 + 0.4 * night)), depth));
                }
                let c = if lit_face {
                    color::shade(m.panel, 1.02)
                } else {
                    color::shade(m.panel, 0.78)
                };
                Some((color::surf(color::mix(c, color::WIN_OFF_NIGHT, night * 0.6), light, side), depth))
            });
        }
        Facade::Terracotta => {
            parts::paint_stucco(q, sprite, light, m.wall, side, d.age);
            let joint = color::surf(color::shade(m.mortar, 0.86), light, side);
            q.paint(sprite, &mut |u, v, _x, _y| {
                let depth = depth_at(q, v);
                // Baguette joints every 4 pixels and a floor band.
                if u % 4 == 0 || v % floor_h == 0 {
                    return Some((joint, depth));
                }
                None
            });
            // Small square windows between the baguettes.
            punch_grid(q, sprite, light, m, seed, (2, 3), 8, floor_h.max(5), m.trim, 0.5, side, false);
        }
        Facade::Perforated => {
            parts::paint_stucco(q, sprite, light, color::shade(m.wall, 0.9), side, d.age);
            // The screen is the material's own metal lightened towards the wall,
            // so a silver tower does not read as a black slab behind a mesh.
            let screen = color::surf(color::mix(m.metal, m.wall, 0.45), light, side);
            let hl = color::surf(color::mix(m.metal, m.wall, 0.25), light, side);
            let night = light.night.clamp(0.0, 1.0);
            q.paint(sprite, &mut |u, v, _x, _y| {
                let depth = depth_at(q, v);
                // A grid of perforations; behind them, the floor's light.
                if u % 3 == 1 && v % 3 == 1 {
                    let fl = v / floor_h;
                    let hash = color::hash_seed(seed, (u / 3 * 17 + fl * 43) as u32);
                    if ((hash % 100) as f32) < 30.0 * night {
                        let c = if hash % 3 != 0 { color::WIN_WARM } else { color::WIN_COOL };
                        return Some((parts::emissive(color::shade(c, 0.5 + 0.4 * night)), depth));
                    }
                    return Some((hl, depth));
                }
                Some((screen, depth))
            });
        }
        Facade::Checker => {
            let night = light.night.clamp(0.0, 1.0);
            let panel = color::surf(m.panel, light, side);
            let glass = color::surf(color::mix(m.glass, color::WIN_OFF_NIGHT, night * 0.7), light, side);
            q.paint(sprite, &mut |u, v, _x, _y| {
                let depth = depth_at(q, v);
                if u % 6 == 0 || v % floor_h == 0 {
                    return Some((color::surf(m.metal, light, side), depth));
                }
                let cell = (u / 6) + (v / floor_h);
                if cell % 2 == 0 {
                    let fl = v / floor_h;
                    let hash = color::hash_seed(seed, (u / 6 * 19 + fl * 53) as u32);
                    if ((hash % 100) as f32) < 34.0 * night {
                        let warm = (hash % 3) != 0;
                        let c = if warm { color::WIN_WARM } else { color::WIN_COOL };
                        return Some((parts::emissive(color::shade(c, 0.55 + 0.45 * night)), depth));
                    }
                    Some((glass, depth))
                } else {
                    Some((panel, depth))
                }
            });
        }
    }
}

/// Paint a base band (from the pavement to the podium's top) in the design's
/// base kind. Called once per face, with the face's own side shading.
pub fn paint_base(
    base: Base,
    q: &Quad,
    sprite: &mut Sprite,
    light: &Light,
    d: &Design,
    salt: u32,
    side: f32,
) {
    let m = mats(d.material);
    let seed = d.seed ^ (salt as u64).wrapping_mul(0x517C_C1B7);
    let rows = q.rows();
    let len = q.top_len();
    let floor_h = d.floor.max(3);
    let night = light.night.clamp(0.0, 1.0);
    match base {
        Base::None => paint(d.facade, q, sprite, light, d, salt, side),
        Base::Colonnade => {
            parts::paint_stone_ashlar(q, sprite, light, m.wall, side, d.age);
            let col = color::surf(m.trim, light, side);
            let shade = color::surf(color::shade(m.trim, 0.8), light, side);
            let recess = color::surf(rgb(0x1e, 0x22, 0x2a), light, side);
            q.paint(sprite, &mut |u, v, _x, _y| {
                let depth = depth_at(q, v);
                // An entablature over the columns, columns below it.
                if v < 3 {
                    return Some((col, depth));
                }
                match u % 7 {
                    0 => Some((shade, depth)),
                    1 | 2 => Some((col, depth)),
                    _ => Some((recess, depth)),
                }
            });
        }
        Base::ShopFront => {
            parts::paint_stucco(q, sprite, light, m.wall, side, d.age);
            // Ground-floor glazing with a warm shop glow at night.
            let frame = color::surf(m.trim, light, side);
            let fascia_c = color::surf(sign_color(d.seed), light, side);
            q.paint(sprite, &mut |u, v, _x, _y| {
                let depth = depth_at(q, v);
                if v < 2 {
                    return Some((frame, depth));
                }
                if v < floor_h {
                    // The fascia band with a row of invented letter blocks.
                    if v < floor_h - 1 {
                        let glyph = (u / 3 + (d.seed % 5) as i32) % 4 != 0;
                        let c = if glyph { color::shade(fascia_c, 1.0) } else { color::shade(fascia_c, 0.8) };
                        return Some((c, depth));
                    }
                    return Some((frame, depth));
                }
                if u % 8 == 0 {
                    return Some((frame, depth));
                }
                if night > 0.15 {
                    let c = color::shade(color::WIN_WARM, 0.55 + 0.45 * night);
                    return Some((parts::emissive(c), depth));
                }
                Some((color::surf(color::mix(m.glass, color::WIN_OFF_NIGHT, night), light, side), depth))
            });
        }
        Base::GlassLobby => {
            parts::paint_glass_curtain_tinted(q, sprite, light, floor_h, 5, side, seed, m.glass, m.glass_hi);
            // The lobby itself: tall panes that glow warm once it is dark.
            if night > 0.15 {
                let glow = parts::emissive(color::shade(color::WIN_WARM, 0.5 + 0.5 * night));
                q.paint(sprite, &mut |u, v, _x, _y| {
                    let depth = depth_at(q, v);
                    if v >= rows - floor_h * 2 && u % 5 != 0 {
                        Some((glow, depth))
                    } else {
                        None
                    }
                });
            }
        }
        Base::StonePlinth => {
            parts::paint_stone_ashlar(q, sprite, light, color::shade(m.wall, 0.94), side, d.age);
            let base_c = color::surf(color::shade(m.wall, 0.82), light, side);
            let sq = q.sub_px(0, rows - 2, len, rows, rows as f32);
            sq.fill(sprite, base_c);
        }
        Base::Parking => {
            parts::paint_stucco(q, sprite, light, color::shade(m.wall, 0.92), side, d.age);
            let slab = color::surf(color::mix(m.panel, m.wall, 0.55), light, side);
            let dark = color::surf(rgb(0x33, 0x38, 0x40), light, side);
            q.paint(sprite, &mut |u, v, _x, _y| {
                let depth = depth_at(q, v);
                let band = v % floor_h;
                if band < 2 {
                    return Some((slab, depth));
                }
                // An open deck: dark inside, a car or two parked.
                let car = (u / 11) % 3 == 1 && band >= 3 && band <= 6;
                if car {
                    let hue = color::hash_seed(seed, (u as u32 / 11) + (v as u32 / floor_h as u32) * 7) % 5;
                    let c = match hue {
                        0 => rgb(0x8c, 0x2e, 0x2e),
                        1 => rgb(0x2e, 0x4e, 0x7c),
                        2 => rgb(0xc8, 0xc8, 0xc0),
                        3 => rgb(0x3a, 0x3e, 0x44),
                        _ => rgb(0x6a, 0x6e, 0x74),
                    };
                    return Some((color::surf(c, light, side), depth));
                }
                if band == floor_h - 1 {
                    return Some((slab, depth));
                }
                Some((dark, depth))
            });
        }
        Base::MallScreen => {
            parts::paint_glass_curtain_tinted(q, sprite, light, floor_h, 6, side, seed, m.glass, m.glass_hi);
            // The D80 bright square: a big screen on the middle of the face.
            let u0 = (len / 2 - len / 5).max(2);
            let u1 = (len / 2 + len / 5).min(len - 2);
            let v0 = 4.min(rows / 3);
            let v1 = (rows - 4).max(v0 + 4);
            q.paint(sprite, &mut |u, v, _x, _y| {
                let depth = depth_at(q, v);
                if u < u0 || u > u1 || v < v0 || v > v1 {
                    return None;
                }
                let border = u == u0 || u == u1 || v == v0 || v == v1;
                if border {
                    return Some((color::surf(pal::METAL_STEEL_DARK, light, side), depth));
                }
                // An invented ad: blocks of colour, brighter at night.
                let cell = ((u - u0) / 4) + ((v - v0) / 4) * 3;
                let h = color::hash_seed(seed, cell as u32 * 7 + 3);
                let c = match h % 6 {
                    0 => rgb(0xd8, 0x4c, 0x6a),
                    1 => rgb(0x3c, 0x88, 0xd0),
                    2 => rgb(0xe0, 0xb0, 0x3c),
                    3 => rgb(0x4c, 0xc8, 0x98),
                    4 => rgb(0xc8, 0xc8, 0xd8),
                    _ => rgb(0x7a, 0x3c, 0xb0),
                };
                if night > 0.1 {
                    return Some((parts::emissive(color::shade(c, 0.55 + 0.45 * night)), depth));
                }
                Some((color::surf(color::shade(c, 0.5), light, side), depth))
            });
        }
        Base::RaisedPlaza => {
            // Two steps of stone with planting, then the shaft's own wall.
            parts::paint_stone_ashlar(q, sprite, light, m.wall, side, d.age);
            let step = color::surf(color::shade(m.trim, 1.02), light, side);
            let plant = color::surf(pal::FOLIAGE_GREEN, light, side);
            q.paint(sprite, &mut |u, v, _x, _y| {
                let depth = depth_at(q, v);
                if v >= rows - 2 {
                    return Some((step, depth));
                }
                if v >= rows - 4 && (u / 5) % 3 == 1 {
                    return Some((plant, depth));
                }
                None
            });
        }
    }
}
