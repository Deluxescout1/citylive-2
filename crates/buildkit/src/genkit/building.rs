//! B73c: drawing every low-rise and mid-rise family from a [`Design`].
//!
//! The kit's geometry is the same as the towers' — iso boxes, the shared facade
//! painters, the shared window/lit rules — but a house brings parts a tower never
//! has: a pitched roof with eaves, a porch, dormers, a bay, a chimney, a garage, a
//! garden and a fence; a shop brings a front, an awning and flats above; a works
//! brings a sawtooth, silos and a mill chimney; a civic building brings its
//! program (a school's yard and hall, a fire station's bays and tower, a
//! hospital's wings, a library's portico, a town hall's clock, a church's spire).
//!
//! Everything draws inside `d.bw × d.bd` — the lot — so the B74 overlap audit
//! stays at zero: the front garden, the drive and the fence are inside the lot
//! as much as the walls are.

use render::palette::{rgb, Color};

use super::paint::{self, mats};
use super::pick::paints;
use super::{roofs, CivicKind, Design, Facade, Family, Finish, Plan, RoofForm};
use crate::color::{self, pal, FACE_L, FACE_R, FACE_TOP};
use crate::geom::{self, box_faces, box_top, fill_polygon, project, Quad};
use crate::parts::{self, WinOpts};
use crate::types::{BuildingSpec, Light, Sprite};

/// The wall colour a design wears.
pub fn wall_colour(d: &Design) -> Color {
    let list = paints(d.finish);
    list[(d.paint as usize) % list.len()]
}

/// A volume of the building.
#[derive(Clone, Copy)]
struct Vol {
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    z0: i32,
    z1: i32,
    /// Draw a roof on this volume (a courtyard wall has none).
    roof: bool,
    /// The main mass (the balcony rows and the roof kit belong to it).
    main: bool,
    /// The volume on the street side: it takes the front door and the porch.
    front: bool,
}

pub fn draw(spec: &BuildingSpec, d: &Design, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let wall = wall_colour(d);
    let trim = trim_colour(d, wall);
    let (bw, bd) = (d.bw, d.bd);

    // Where the mass sits on the lot: a house stands back for its front garden,
    // a shop comes to the pavement.
    let front_frac = match d.family {
        Family::House => 0.30,
        Family::Townhouse => 0.10,
        Family::Shop => 0.04,
        Family::Apartment | Family::Office => 0.08,
        _ => 0.05,
    };
    let spare_x = (bw - d.w).max(0.0);
    let spare_y = (bd - d.d).max(0.0);
    let x0 = spare_x * (0.35 + 0.3 * color::hash01(d.seed as i32, 3));
    let y0 = spare_y * (1.0 - front_frac);
    let (x1, y1) = (x0 + d.w, y0 + d.d);

    site(d, light, sprite, ox, oy, wall);

    // The cast shadow the cache drops (transparent in the sprite: the engine
    // draws its own ground shadow for kit buildings).
    parts::building_shadow(sprite, x0, y0, x1, y1, d.z_crown, ox, oy);

    // A works' silos stand beside the shed, so they are drawn from the ground
    // up before the walls (the shed then hides their feet).
    if d.family == Family::Industrial {
        silos(d, light, sprite, ox, oy, x0, y0, y1);
    }

    let vols = volumes(d, x0, y0, x1, y1);

    // Painter's order: the far volume first, then the nearer ones over it.
    let mut order: Vec<usize> = (0..vols.len()).collect();
    order.sort_by(|&a, &b| {
        let ka = vols[a].y0 + vols[a].x0 * 0.35;
        let kb = vols[b].y0 + vols[b].x0 * 0.35;
        ka.partial_cmp(&kb).unwrap_or(std::cmp::Ordering::Equal)
    });
    for i in order {
        let v = vols[i];
        walls(d, &v, light, sprite, ox, oy, wall, trim);
        if v.roof {
            // A roof is a proportion of its wall, not only of its plan: a
            // cottage with a 60 % pitch still shows a storey of wall.
            let wall_h = (v.z1 - v.z0).max(6);
            let cap = if d.family == Family::House || d.family == Family::Townhouse {
                (wall_h as f32 * 0.75) as i32
            } else {
                (wall_h as f32 * 0.95) as i32
            };
            let rise = ((d.z_crown - v.z1) as i32).min(cap).max(4);
            if matches!(d.roof, RoofForm::Flat) {
                roofs::draw(d, light, sprite, ox, oy, v.x0, v.y0, v.x1, v.y1, v.z1, rise.min(4), wall);
                if !matches!(d.family, Family::House | Family::Townhouse) {
                    rooftop_kit(d, &v, light, sprite, ox, oy);
                }
            } else if matches!(d.roof, RoofForm::Shed) && d.family == Family::Industrial {
                sawtooth(d, &v, light, sprite, ox, oy, wall);
            } else {
                // A wing's roof is shallower than the main one.
                let rise = if v.main { rise } else { (rise as f32 * 0.6) as i32 };
                roofs::draw(d, light, sprite, ox, oy, v.x0, v.y0, v.x1, v.y1, v.z1, rise, wall);
            }
        }
    }

    features(spec, d, light, sprite, ox, oy, wall, trim, x0, y0, x1, y1);
    close_pinpricks(sprite);
}

/// A design's trim colour: white or cream on paint, the material's trim on a
/// commercial block.
fn trim_colour(d: &Design, wall: Color) -> Color {
    match d.family {
        Family::House | Family::Townhouse | Family::Shop | Family::Civic => {
            if d.finish == Finish::Brick || d.finish == Finish::Stone {
                pal::TRIM_CREAM
            } else {
                pal::TRIM_WHITE
            }
        }
        _ => {
            let m = mats(d.material);
            if m.trim == 0 {
                color::shade(wall, 1.2)
            } else {
                m.trim
            }
        }
    }
}

// -----------------------------------------------------------------------------
// The lot: garden, drive, path, fence, apron, yard
// -----------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn site(d: &Design, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32, wall: Color) {
    let (bw, bd) = (d.bw, d.bd);
    let ground = box_top(0.02, 0.02, bw - 0.02, bd - 0.02, 0, ox, oy);
    let lawn = pal::GRASS_LUSH;
    let flower = pal::FLOWER_RED;
    let soil = pal::SOIL_GARDEN;
    let pave = pal::CONCRETE_DRIVE;
    let seed = d.seed;
    match d.family {
        Family::House | Family::Townhouse => {
            ground.paint(sprite, &mut |u, v, x, y| {
                let c = match d.garden {
                    // Flowers: a border along the lot edge, lawn inside.
                    1 => {
                        let border = v < 3 || u < 3;
                        if border && (u + v) % 3 == 0 {
                            if color::hash01(x / 2, y / 2) > 0.6 {
                                flower
                            } else {
                                pal::FOLIAGE_GREEN
                            }
                        } else {
                            lawn
                        }
                    }
                    // Vegetables: rows of soil and green.
                    2 => {
                        if (u / 5) % 2 == 0 {
                            soil
                        } else {
                            pal::FOLIAGE_GREEN
                        }
                    }
                    // Gravel.
                    3 => pal::STONE_GRANITE,
                    // A pool: tiled blue with a deck.
                    4 => {
                        if u > 6 && u < bw as i32 * 32 - 10 && v > 4 && v < bd as i32 * 16 - 6 {
                            rgb(0x3c, 0x8c, 0xc0)
                        } else {
                            block_paving()
                        }
                    }
                    // A paved yard.
                    5 => block_paving(),
                    _ => lawn,
                };
                Some((color::surf(color::shade(c, color::jcell(x, y, 4, 0.05)), light, FACE_TOP), 0))
            });
            if d.family == Family::House {
                // A drive from the street to the garage/carport side.
                let (dx, dw) = if d.garage > 0 {
                    (bw * 0.08, 0.42)
                } else {
                    (bw * 0.62, 0.30)
                };
                let dr = box_top(dx, bd * 0.55, (dx + dw).min(bw - 0.04), bd - 0.03, 0, ox, oy);
                dr.paint(sprite, &mut |u, _v, x, y| {
                    let joint = u % 6 == 0;
                    let c = if joint {
                        color::shade(pave, 0.9)
                    } else {
                        color::shade(pave, color::jcell(x, y, 3, 0.04))
                    };
                    Some((color::surf(c, light, FACE_TOP), 0))
                });
                // The path from the pavement to the door.
                let px = bw * 0.5 + (color::hash01(seed as i32, 9) - 0.5) * 0.2;
                let path = box_top(px.min(bw - 0.3), bd * 0.42, (px + 0.22).min(bw - 0.04), bd - 0.02, 0, ox, oy);
                path.fill(sprite, color::surf(color::shade(pave, 1.04), light, FACE_TOP));
            }
            edge(d, light, sprite, ox, oy);
        }
        Family::Shop | Family::Apartment | Family::Office | Family::Civic => {
            // A paved forecourt to the pavement.
            ground.paint(sprite, &mut |_u, v, x, y| {
                let c = if v < 5 {
                    // The pavement in front.
                    color::shade(pal::CURB, color::jcell(x, y, 4, 0.03))
                } else if d.family == Family::Civic && d.garden == 0 {
                    // A civic lawn.
                    pal::GRASS_LUSH
                } else {
                    block_paving()
                };
                Some((color::surf(c, light, FACE_TOP), 0))
            });
            if matches!(d.family, Family::Apartment | Family::Office | Family::Shop) {
                parking(d, light, sprite, ox, oy);
                edge(d, light, sprite, ox, oy);
            }
        }
        Family::Industrial => {
            ground.paint(sprite, &mut |_u, v, x, y| {
                let c = color::shade(pal::ASPHALT, color::jcell(x, y, 5, 0.06));
                Some((color::surf(c, light, FACE_TOP), 0))
            });
            // Loading bay markings.
            let mark = color::surf(rgb(0xe8, 0xd0, 0x58), light, FACE_TOP);
            for k in 0..3 {
                let bx = 0.15 + 0.25 * k as f32;
                let q = box_top(bx * bw, bd * 0.72, (bx * bw) + 0.03, bd - 0.04, 0, ox, oy);
                q.fill(sprite, mark);
            }
        }
        _ => {
            ground.fill(sprite, color::surf(pal::CONCRETE_DRIVE, light, FACE_TOP));
        }
    }
    let _ = wall;
}

/// A block-paver surface: pavers, not a white slab (a bright yard read as a
/// hole in the street at 1×).
fn block_paving() -> Color {
    color::shade(pal::CONCRETE_DRIVE, 0.86)
}

/// The lot's edge: a low wall, a hedge or a picket fence along the front, with
/// short returns down both sides — a front garden's edge, not a wall around the
/// whole plot (which at 1× read as a floating band across the lot).
fn edge(d: &Design, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    if d.fence == 0 {
        return;
    }
    let (bw, bd) = (d.bw, d.bd);
    let (h, c) = match d.fence {
        1 => (3, pal::STONE_GRANITE),
        2 => (5, pal::FOLIAGE_GREEN),
        _ => (4, pal::TRIM_WHITE),
    };
    let runs = [
        // The front rail: (x0, y0, x1, y1).
        (0.05, bd - 0.10, bw - 0.05, bd - 0.06),
        // Two short returns.
        (0.05, bd - 0.34, 0.09, bd - 0.10),
        (bw - 0.09, bd - 0.34, bw - 0.05, bd - 0.10),
    ];
    for (i, &(x0, y0, x1, y1)) in runs.iter().enumerate() {
        let (lf, rf) = box_faces(x0, y0, x1, y1, 0, h, ox, oy);
        for (q, side) in [(lf, FACE_L), (rf, FACE_R)] {
            if d.fence == 3 {
                let rail = color::surf(c, light, side);
                q.paint(sprite, &mut |u, v, _x, _y| {
                    let post = u % 4 == 0;
                    let r = v == 0 || v == h - 1;
                    (post || r).then_some((rail, (h - v) as u16))
                });
            } else {
                let shade = if side == FACE_L { 1.0 } else { 0.82 };
                q.fill(sprite, color::surf(color::shade(c, shade * (1.0 - 0.03 * i as f32)), light, side));
            }
        }
    }
}

/// An apartment's or an office's car park: bays with cars and the drive to the
/// street (the owner: apartments must provide parking and reach a road).
fn parking(d: &Design, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let (bw, bd) = (d.bw, d.bd);
    let pave = pal::ASPHALT;
    // The apron occupies the front third of the lot, with a drive out.
    let y0 = (bd * 0.62).min(bd - 0.2);
    let apron = box_top(0.05, y0, bw - 0.05, bd - 0.03, 0, ox, oy);
    apron.paint(sprite, &mut |_u, _v, x, y| {
        let c = color::shade(pave, color::jcell(x, y, 4, 0.05));
        Some((color::surf(c, light, FACE_TOP), 0))
    });
    // Bays: white lines, and a car in some of them.
    let bays = ((bw - 0.1) / 0.30).floor().max(1.0) as i32;
    let line = color::surf(pal::TRIM_WHITE, light, FACE_TOP);
    for k in 0..=bays {
        let lx = 0.05 + k as f32 * 0.30;
        if lx >= bw - 0.06 {
            break;
        }
        let q = box_top(lx, y0 + 0.04, lx + 0.02, bd - 0.06, 0, ox, oy);
        q.fill(sprite, line);
    }
    for k in 0..bays {
        if color::hash_seed(d.seed, 0x200 + k as u32) % 3 == 0 {
            continue;
        }
        let cx = 0.05 + k as f32 * 0.30 + 0.05;
        let cy = y0 + 0.10;
        let car = match color::hash_seed(d.seed, 0x300 + k as u32) % 5 {
            0 => rgb(0x8c, 0x2e, 0x2e),
            1 => rgb(0x2e, 0x4e, 0x7c),
            2 => rgb(0xc8, 0xc8, 0xc0),
            3 => rgb(0x3a, 0x3e, 0x44),
            _ => rgb(0x6a, 0x6e, 0x74),
        };
        let (clf, crf) = box_faces(cx, cy, cx + 0.22, cy + 0.13, 0, 3, ox, oy);
        clf.fill(sprite, color::surf(car, light, FACE_L));
        crf.fill(sprite, color::surf(color::shade(car, 0.82), light, FACE_R));
        let top = box_top(cx + 0.03, cy + 0.03, cx + 0.19, cy + 0.10, 3, ox, oy);
        top.fill(sprite, color::surf(color::shade(car, 1.12), light, FACE_TOP));
    }
    // The drive: a paved strip from the bays to the street edge.
    let drive = box_top(bw * 0.55, bd * 0.55, (bw * 0.55 + 0.55).min(bw - 0.04), bd - 0.02, 0, ox, oy);
    drive.fill(sprite, color::surf(color::shade(pave, 1.08), light, FACE_TOP));
}

// -----------------------------------------------------------------------------
// Volumes and walls
// -----------------------------------------------------------------------------

/// Split the design's plan into its volumes. The volumes tile the lot: no two
/// overlap, so a wing is a real wing beside the main mass and never a box drawn
/// through it.
fn volumes(d: &Design, x0: f32, y0: f32, x1: f32, y1: f32) -> Vec<Vol> {
    let (w, dd) = (x1 - x0, y1 - y0);
    let z = d.z_top;
    let raise = if matches!(d.plan, Plan::Stilts) { 7 } else { 0 };
    let main = Vol { x0, y0, x1, y1, z0: raise, z1: z, roof: true, main: true, front: true };
    let mut out = vec![main];
    match d.plan {
        Plan::Box | Plan::Narrow | Plan::Arcade | Plan::Stilts => {}
        Plan::L => {
            // A bar at the back and a wing down one side: the court in the
            // front corner is the L's own yard.
            out[0] = Vol { y1: y0 + dd * 0.66, main: true, front: false, ..main };
            out.push(Vol {
                x0,
                y0: y0 + dd * 0.65,
                x1: x0 + w * 0.52,
                y1,
                z0: 0,
                z1: (z as f32 * 0.74) as i32,
                roof: true,
                main: false,
                front: true,
            });
        }
        Plan::T | Plan::Cross => {
            // A back bar and a projecting centre bay: the T.
            let low = !matches!(d.plan, Plan::Cross);
            out[0] = Vol { y1: y0 + dd * 0.62, main: true, front: false, ..main };
            out.push(Vol {
                x0: x0 + w * 0.28,
                y0: y0 + dd * 0.61,
                x1: x0 + w * 0.74,
                y1,
                z0: 0,
                z1: if low { (z as f32 * 0.8) as i32 } else { z },
                roof: true,
                main: low,
                front: true,
            });
        }
        Plan::Split => {
            // Two halves of different heights: the high one takes the door.
            out[0] = Vol { x1: x0 + w * 0.47, z1: (z as f32 * 0.64) as i32, main: false, front: false, ..main };
            out.push(Vol {
                x0: x0 + w * 0.47,
                y0,
                x1,
                y1,
                z0: 0,
                z1: z,
                roof: true,
                main: true,
                front: true,
            });
        }
        Plan::Wing => {
            // The main mass on one side, a low hall/drive bay on the other.
            out[0] = Vol { x0: x0 + w * 0.40, main: true, front: false, ..main };
            out.push(Vol {
                x0,
                y0: y0 + dd * 0.12,
                x1: x0 + w * 0.38,
                y1,
                z0: 0,
                z1: (z as f32 * 0.56) as i32,
                roof: true,
                main: false,
                front: true,
            });
        }
        Plan::Courtyard => {
            // A bar at the back and two arms: the court opens to the street.
            out[0] = Vol { y1: y0 + dd * 0.58, main: true, front: false, ..main };
            for (a, b) in [(0.0f32, 0.17f32), (0.83, 1.0)] {
                out.push(Vol {
                    x0: x0 + w * a,
                    y0: y0 + dd * 0.57,
                    x1: x0 + w * b,
                    y1,
                    z0: 0,
                    z1: (z as f32 * 0.72) as i32,
                    roof: true,
                    main: false,
                    front: false,
                });
            }
        }
    }
    // Program volumes (B73c §2: a civic type keeps its program). They are
    // volumes, not decorations: the nave's tower stands beside the nave, the
    // fire station's drill tower beside the bays, the school's hall beside the
    // teaching block — drawn in the same painter's order as everything else.
    match (d.family, d.civic) {
        (Family::Civic, CivicKind::Church | CivicKind::FireStation) => {
            let tw = (w * 0.30).clamp(0.42, 0.95);
            // Give the tower its own strip: shrink every volume that starts at
            // the left edge.
            for v in out.iter_mut() {
                if v.x0 <= x0 + 0.02 {
                    v.x0 = x0 + tw + 0.05;
                }
            }
            out.push(Vol {
                x0: x0 + 0.02,
                y0: y0 + 0.02,
                x1: x0 + tw,
                y1: y0 + tw,
                z0: 0,
                z1: z + if d.civic == CivicKind::Church { 18 } else { 20 },
                roof: true,
                main: false,
                front: false,
            });
        }
        (Family::Civic, CivicKind::School) => {
            let hw = (w * 0.34).clamp(0.5, 1.4);
            for v in out.iter_mut() {
                if v.x0 <= x0 + 0.02 {
                    v.x0 = x0 + hw + 0.04;
                }
            }
            out.push(Vol {
                x0,
                y0: y0 + dd * 0.16,
                x1: x0 + hw,
                y1,
                z0: 0,
                z1: (z as f32 * 0.66) as i32,
                roof: true,
                main: false,
                front: false,
            });
        }
        (Family::Civic, CivicKind::Hospital) => {
            // Two ward wings across the back, each lower than the main block.
            for k in 0..2 {
                let wy = y0 + dd * (0.05 + 0.24 * k as f32);
                out.push(Vol {
                    x0: x0 - 0.02,
                    y0: wy,
                    x1: (x1 + w * 0.16).min(d.bw - 0.02),
                    y1: wy + dd * 0.18,
                    z0: 0,
                    z1: ((z as f32) * 0.62) as i32,
                    roof: true,
                    main: false,
                    front: false,
                });
            }
        }
        _ => {}
    }
    // The street side is whichever volume reaches furthest forward.
    let front = out
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.y1.partial_cmp(&b.1.y1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(i, _)| i)
        .unwrap_or(0);
    for (i, v) in out.iter_mut().enumerate() {
        v.front = i == front;
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn walls(
    d: &Design,
    v: &Vol,
    light: &Light,
    sprite: &mut Sprite,
    ox: f32,
    oy: f32,
    wall: Color,
    trim: Color,
) {
    sprite.on_surface(render::snow::NONE);
    if v.z0 > 0 {
        // A raised floor: posts under the volume (the stilts' idiom).
        parts::stilts(sprite, light, v.x0, v.y0, v.x1, v.y1, v.z0, color::shade(wall, 0.7), ox, oy);
    }
    let (lf, rf) = box_faces(v.x0, v.y0, v.x1, v.y1, v.z0, v.z1, ox, oy);
    if matches!(d.family, Family::House | Family::Townhouse | Family::Shop)
        || d.finish == Finish::Timber
        || d.civic == CivicKind::Church
    {
        house_wall(d, &lf, sprite, light, FACE_L, wall, trim, v);
        house_wall(d, &rf, sprite, light, FACE_R, wall, trim, v);
    } else {
        let salt = (v.z0 as u32).wrapping_mul(7) ^ ((v.x0 * 10.0) as u32);
        paint::paint(d.facade, &lf, sprite, light, d, salt, FACE_L);
        paint::paint(d.facade, &rf, sprite, light, d, salt + 1, FACE_R);
        // The ground floor: the base's own front (a shop front, a colonnade).
        if d.z_podium > 0 {
            let (blf, brf) = box_faces(v.x0, v.y0, v.x1, v.y1, v.z0, v.z0 + d.z_podium, ox, oy);
            paint::paint_base(d.base, &blf, sprite, light, d, 11, FACE_L);
            paint::paint_base(d.base, &brf, sprite, light, d, 12, FACE_R);
        }
    }
    parts::corner_seam(sprite, light, v.x1, v.y1, v.z0, v.z1, trim, ox, oy);
}

/// A cladding wall with punched windows and a door on the front.
#[allow(clippy::too_many_arguments)]
fn house_wall(
    d: &Design,
    q: &Quad,
    sprite: &mut Sprite,
    light: &Light,
    side: f32,
    wall: Color,
    trim: Color,
    v: &Vol,
) {
    clad(d, q, sprite, light, wall, side);
    let rows = q.rows();
    let floor = d.floor.max(6);
    let floors = ((v.z1 - v.z0) / floor).max(1);
    let len = q.top_len();
    let shutters = matches!(d.finish, Finish::Clapboard | Finish::Shingle | Finish::Log)
        && color::hash_seed(d.seed, 0x401) % 100 < 45;
    let shutter = {
        let c = match color::hash_seed(d.seed, 0x402) % 4 {
            0 => pal::SHUTTER_GREEN,
            1 => pal::SHUTTER_BURGUNDY,
            2 => pal::SHUTTER_BLACK,
            _ => rgb(0x3a, 0x5a, 0x78),
        };
        c
    };
    let win_w = if shutters { 6 } else { 4 };
    let win_h = (floor - 5).clamp(4, 10);
    let bay = if len > 40 { 11 } else { 8 };
    // The door: on the front face of the main volume, at a seeded place.
    let door_u = {
        let spans = ((len - win_w - 4) / bay).max(1);
        let k = (color::hash_seed(d.seed, 0x403) as i32) % spans;
        2 + k * bay
    };
    for f in 0..floors {
        let v_top = rows - 2 - f * floor;
        let v_px = v_top - win_h;
        if v_px < 2 {
            break;
        }
        let mut u = 3;
        while u + win_w + 1 < len - 1 {
            let is_door = side == FACE_R && v.front && f == 0 && (u - door_u).abs() <= 1;
            if is_door {
                let dh = (floor - 2).min(rows - v_px - 1);
                parts::draw_door(q, sprite, light, u, v_top - dh, win_w.max(5), dh, door_colour(d, trim), trim, 0);
                u += win_w + 2;
                continue;
            }
            // A window whose light follows the household's own schedule.
            let wo = WinOpts {
                w: win_w,
                h: win_h,
                frame: trim,
                sill: true,
                lintel: d.finish == Finish::Brick || d.finish == Finish::Stone,
                shutters,
                shutter_c: shutter,
                seed: d.seed ^ (f as u64 * 97),
                lit_p: 0.72,
            };
            parts::draw_window(q, sprite, light, u, v_px, wo, v_top);
            u += bay;
        }
    }
    let _ = wall;
}

/// The cladding itself.
fn clad(d: &Design, q: &Quad, sprite: &mut Sprite, light: &Light, wall: Color, side: f32) {
    match d.finish {
        Finish::Clapboard => parts::paint_clapboard(q, sprite, light, wall, side, d.age),
        Finish::Shingle => shingle_wall(q, sprite, light, wall, side, d.age),
        Finish::Brick => parts::paint_brick(q, sprite, light, wall, pal::BRICK_MORTAR, side, d.age),
        Finish::Stone => parts::paint_stone_ashlar(q, sprite, light, wall, side, d.age),
        Finish::Stucco | Finish::Adobe | Finish::Concrete => {
            parts::paint_stucco(q, sprite, light, wall, side, d.age)
        }
        Finish::Timber => half_timber(q, sprite, light, wall, side, d.age),
        Finish::Log => parts::paint_log(q, sprite, light, wall, side, d.age),
        Finish::Corrugated => parts::paint_corrugated(q, sprite, light, wall, side, d.age),
        Finish::Panel => panel_wall(q, sprite, light, wall, side, d.age),
        Finish::Glass => parts::paint_glass_curtain(q, sprite, light, d.floor.max(4), 6, side, d.seed),
    }
}

/// Wall shingles: staggered courses, each a shade of the one above.
fn shingle_wall(q: &Quad, sprite: &mut Sprite, light: &Light, wall: Color, side: f32, age: f32) {
    let rows = q.rows().max(1);
    let len = q.top_len();
    q.paint(sprite, &mut |u, v, x, y| {
        let course = v / 3;
        let joint = u % 5 == 0 || (v % 3 == 0);
        let c = if joint {
            color::shade(wall, 0.86)
        } else {
            color::shade(wall, 0.96 + 0.08 * color::hash01(course, (u / 5) as i32))
        };
        let depth = (q.z_top as f32 + (q.z_base as f32 - q.z_top as f32) * (v as f32 / rows as f32)) as u16;
        Some((color::surf(color::weather(c, x, y, age), light, side), depth))
    });
    let _ = len;
}

/// Flat panels with joint lines: the modern low-rise's wall.
fn panel_wall(q: &Quad, sprite: &mut Sprite, light: &Light, wall: Color, side: f32, age: f32) {
    let rows = q.rows().max(1);
    q.paint(sprite, &mut |u, v, x, y| {
        let joint = u % 9 == 0 || v % 8 == 0;
        let c = if joint {
            color::shade(wall, 0.84)
        } else {
            color::shade(wall, 0.98 + 0.04 * color::hash01(u / 9, v / 8))
        };
        let depth = (q.z_top as f32 + (q.z_base as f32 - q.z_top as f32) * (v as f32 / rows as f32)) as u16;
        Some((color::surf(color::weather(c, x, y, age), light, side), depth))
    });
}

/// Plaster between dark timber posts — the fachwerk / chalet wall.
fn half_timber(q: &Quad, sprite: &mut Sprite, light: &Light, wall: Color, side: f32, age: f32) {
    let rows = q.rows().max(1);
    let len = q.top_len();
    let beam = color::surf(rgb(0x4a, 0x36, 0x2a), light, side);
    q.paint(sprite, &mut |u, v, x, y| {
        let depth = (q.z_top as f32 + (q.z_base as f32 - q.z_top as f32) * (v as f32 / rows as f32)) as u16;
        // Posts every 9 px, a rail top and bottom, and a diagonal brace.
        let floor = 12;
        let post = u % 9 == 0;
        let rail = v % floor == 0 || v % floor == floor - 1;
        let brace = (u + v) % 23 == 0;
        if post || rail || brace {
            return Some((beam, depth));
        }
        Some((color::surf(color::weather(color::shade(wall, color::jcell(x, y, 3, 0.03)), x, y, age), light, side), depth))
    });
    let _ = len;
}

/// The front door's colour: seeded, in the palette a real street uses.
fn door_colour(d: &Design, trim: Color) -> Color {
    match color::hash_seed(d.seed, 0x404) % 7 {
        0 => pal::CLAP_BARN_RED,
        1 => rgb(0x2e, 0x4a, 0x6a), // navy
        2 => rgb(0x2e, 0x5a, 0x42), // forest
        3 => rgb(0x8a, 0x6a, 0x30), // brass/ochre
        4 => rgb(0x3a, 0x3a, 0x3e), // black
        5 => rgb(0x6a, 0x3a, 0x5a), // plum
        _ => trim,
    }
}

// -----------------------------------------------------------------------------
// Roofs and rooftop kit
// -----------------------------------------------------------------------------

fn rooftop_kit(d: &Design, v: &Vol, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    if !v.main {
        return;
    }
    let (w, dd) = (v.x1 - v.x0, v.y1 - v.y0);
    let z = v.z1 + 2;
    let metal = color::surf(pal::METAL_STEEL_DARK, light, FACE_L);
    match d.top {
        super::Top::WaterTank => {
            // A timber tank on a steel stand, the old masonry idiom.
            let (tx0, ty0) = (v.x0 + w * 0.55, v.y0 + dd * 0.2);
            let (tx1, ty1) = (tx0 + w * 0.3, ty0 + dd * 0.34);
            for &(lx, ly) in &[(tx0, ty0), (tx1, ty0), (tx0, ty1), (tx1, ty1)] {
                let (ax, ay) = project(lx, ly, z as f32, ox, oy);
                let (bx, by) = project(lx, ly, (z + 4) as f32, ox, oy);
                geom::draw_line(sprite, ax as i32, ay as i32, bx as i32, by as i32, metal, (z + 4) as u16);
            }
            sprite.on_surface(render::snow::EQUIP_SIDE);
            let (lf, rf) = box_faces(tx0, ty0, tx1, ty1, z + 4, z + 10, ox, oy);
            let cedar = color::surf(pal::ROOF_CEDAR_SHAKE, light, FACE_L);
            lf.fill(sprite, cedar);
            rf.fill(sprite, color::surf(color::shade(pal::ROOF_CEDAR_SHAKE, 0.8), light, FACE_R));
            let lid = box_top(tx0, ty0, tx1, ty1, z + 11, ox, oy);
            sprite.on_surface(render::snow::EQUIP_TOP);
            lid.fill(sprite, color::surf(color::shade(pal::ROOF_CEDAR_SHAKE, 1.1), light, FACE_TOP));
            sprite.on_surface(render::snow::NONE);
        }
        super::Top::RoofGarden => {
            let g = box_top(v.x0 + w * 0.15, v.y0 + dd * 0.2, v.x1 - w * 0.15, v.y1 - dd * 0.3, z + 1, ox, oy);
            sprite.on_surface(render::snow::FOLIAGE);
            g.fill(sprite, color::surf(pal::FOLIAGE_GREEN, light, FACE_TOP));
            sprite.on_surface(render::snow::NONE);
            for k in 0..3 {
                let fx = v.x0 + w * (0.2 + 0.25 * k as f32);
                let fy = v.y0 + dd * 0.4;
                let (px, py) = project(fx, fy, (z + 3) as f32, ox, oy);
                sprite.set_pixel(px as i32, py as i32, color::surf(pal::FOLIAGE_GREEN, light, FACE_L), (z + 3) as u16);
                sprite.set_pixel(px as i32, py as i32 - 1, color::surf(pal::FOLIAGE_GREEN, light, FACE_L), (z + 3) as u16);
            }
        }
        super::Top::AntennaCluster => {
            for (k, fx) in [0.3f32, 0.62].iter().enumerate() {
                parts::spire_needle(sprite, light, v.x0 + w * fx, v.y0 + dd * 0.35, z, 8 + 4 * k as i32, ox, oy);
            }
        }
        super::Top::Helipad => {
            let pad = box_top(v.x0 + w * 0.12, v.y0 + dd * 0.15, v.x1 - w * 0.12, v.y1 - dd * 0.2, z + 1, ox, oy);
            pad.paint(sprite, &mut |u, v2, _x, _y| {
                let border = u <= 1 || v2 <= 1;
                let c = if border { pal::METAL_WHITE } else { pal::ASPHALT };
                Some((color::surf(c, light, FACE_TOP), (z + 1) as u16))
            });
        }
        super::Top::LightBox => {
            let (lf, rf) = box_faces(v.x0 + w * 0.2, v.y0 + dd * 0.2, v.x0 + w * 0.62, v.y0 + dd * 0.45, z, z + 7, ox, oy);
            lf.fill(sprite, color::surf(pal::METAL_STEEL_DARK, light, FACE_L));
            rf.fill(sprite, color::surf(color::shade(pal::METAL_STEEL_DARK, 0.84), light, FACE_R));
            if light.night > 0.1 {
                let glow = parts::emissive(color::shade(rgb(0xff, 0xf0, 0xdc), 0.55 + 0.45 * light.night));
                lf.paint(sprite, &mut |u, v2, _x, _y| (u > 1 && v2 > 1).then_some((glow, (z + 7) as u16)));
            }
        }
        _ => {
            // A plant box, a stair head and a vent: what a flat roof carries.
            let sy = v.y0 + dd * 0.35;
            let (sl, sr) = box_faces(v.x0 + w * 0.45, sy, v.x0 + w * 0.72, sy + dd * 0.28, z, z + 5, ox, oy);
            sl.fill(sprite, color::surf(pal::CONCRETE_LIGHT, light, FACE_L));
            sr.fill(sprite, color::surf(pal::CONCRETE_GREY, light, FACE_R));
            let vent = box_top(v.x0 + w * 0.18, v.y0 + dd * 0.3, v.x0 + w * 0.32, v.y0 + dd * 0.42, z + 2, ox, oy);
            vent.fill(sprite, color::surf(pal::ROOF_GRAVEL, light, FACE_TOP));
        }
    }
    // A civic building may keep a small flag (D59: invented, no real flag).
    if d.family == Family::Civic {
        let fx = v.x0 + w * 0.82;
        let fy = v.y0 + dd * 0.2;
        let (px, py) = project(fx, fy, (v.z1 + 2) as f32, ox, oy);
        let pole = color::surf(pal::TRIM_WHITE, light, FACE_L);
        geom::draw_line(sprite, px as i32, py as i32, px as i32, py as i32 - 13, pole, (v.z1 + 15) as u16);
        let flag = color::surf(rgb(0x9c, 0x3a, 0x32), light, FACE_TOP);
        for k in 0..4 {
            for j in 0..3 {
                sprite.set_pixel(px as i32 + 1 + k, py as i32 - 13 + j, flag, (v.z1 + 15) as u16);
            }
        }
    }
}

/// A works' sawtooth: the shed roof repeated across the span, with a glazed
/// clerestory on the tall face — the shape every mill and shed has.
#[allow(clippy::too_many_arguments)]
fn sawtooth(d: &Design, v: &Vol, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32, wall: Color) {
    let n = 3;
    let depth = (v.y1 - v.y0) / n as f32;
    let rise = (d.z_crown - v.z1).max(3);
    let metal = roofs::roof_colour(d);
    for k in 0..n {
        let y0 = v.y0 + depth * k as f32;
        let y1 = y0 + depth;
        let z = v.z1;
        let zr = z + rise;
        let p = |x: f32, y: f32, zz: i32| project(x, y, zz as f32, ox, oy);
        // The slope rises to the back of each tooth. B76 r3: a steep sawtooth
        // sheds almost all of its snow.
        sprite.on_surface(render::snow::SLOPE_STEEP);
        let sl = [p(v.x0, y1, zr), p(v.x1, y1, zr), p(v.x1, y0, z), p(v.x0, y0, z)];
        fill_polygon(sprite, &sl, color::surf(color::shade(metal, 1.0), light, FACE_TOP), zr as u16);
        sprite.on_surface(render::snow::NONE);
        // The clerestory: a glazed strip on the tall face.
        let band = [p(v.x0, y1, z + rise - 2), p(v.x1, y1, z + rise - 2), p(v.x1, y1, z + 1), p(v.x0, y1, z + 1)];
        let glass = color::surf(color::mix(pal::GLASS_DARK, pal::GLASS_REFLECTIVE, if light.is_night { 0.2 } else { 0.6 }), light, FACE_L);
        fill_polygon(sprite, &band, glass, (z + rise) as u16);
        if k == 0 {
            // The gable end of the works, so the teeth do not float.
            let tri = [p(v.x1, v.y0, z), p(v.x1, v.y1, z), p(v.x1, y1, zr)];
            fill_polygon(sprite, &tri, color::surf(color::shade(wall, 0.9), light, FACE_R), zr as u16);
        }
    }
}

// -----------------------------------------------------------------------------
// Features
// -----------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn features(
    spec: &BuildingSpec,
    d: &Design,
    light: &Light,
    sprite: &mut Sprite,
    ox: f32,
    oy: f32,
    wall: Color,
    trim: Color,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
) {
    // The front volume: the porch, the bay and the chimney belong to it.
    let fronts = volumes(d, x0, y0, x1, y1);
    let fv = fronts.iter().find(|v| v.front).copied().unwrap_or(fronts[0]);
    // The porch / stoop / veranda, on the front face of the street volume.
    if d.porch > 0 && d.family == Family::House {
        porch(d, light, sprite, ox, oy, fv.x0, fv.y1, fv.x1, wall, trim);
    }
    // A garage beside the house, in the lot's own margin (never through the
    // walls): an attached bay if there is room, a carport if there is less.
    if d.family == Family::House && d.garage > 0 {
        let room = x0 - 0.05;
        let gw = (d.w * 0.44).clamp(0.34, 0.95).min(room);
        if gw >= 0.32 {
            let gx = (x0 - gw - 0.04).max(0.03);
            let gy = (fv.y1 - d.d * 0.55).max(y0);
            garage(d, light, sprite, ox, oy, gx, gy, gw, d.floor, wall, trim, d.garage == 1);
        }
    }
    // A bay window on the front.
    if d.bay == 1 {
        bay_window(d, light, sprite, ox, oy, fv.x0, fv.y1, fv.x1, wall, trim);
    }
    // Dormers in the near roof slope.
    if d.dormers > 0 && !matches!(d.roof, RoofForm::Flat | RoofForm::Shed) {
        let h = (d.floor - 3).clamp(5, 9);
        let n = d.dormers as i32;
        for k in 0..n {
            let f = (k as f32 + 0.5) / n as f32;
            let cx = fv.x0 + (fv.x1 - fv.x0) * (0.28 + 0.44 * f);
            let z_eave = fv.z1;
            roofs::dormer(d, light, sprite, ox, oy, cx, fv.y1 - 0.06, z_eave, h, wall);
        }
    }
    // A works' chimney is a tall thin stack, not a house chimney.
    if d.family == Family::Industrial && d.chimney > 0 {
        let sx = fv.x1 - (fv.x1 - fv.x0) * 0.18;
        let sy = fv.y0 + (fv.y1 - fv.y0) * 0.16;
        let z = fv.z1;
        let h = 30;
        // B76 r3: a stack is a vertical face — no snow lies on it, only on its
        // cap, and it pokes out of the load instead of wearing a hat.
        sprite.on_surface(render::snow::EQUIP_SIDE);
        let (lf, rf) = box_faces(sx - 0.10, sy, sx + 0.10, sy + 0.20, z - 6, z + h, ox, oy);
        let brick = color::surf(pal::BRICK_RED, light, FACE_L);
        lf.fill(sprite, brick);
        rf.fill(sprite, color::surf(color::shade(pal::BRICK_RED, 0.8), light, FACE_R));
        for k in [8, 20] {
            let (cl, cr) = box_faces(sx - 0.12, sy - 0.01, sx + 0.12, sy + 0.21, z + k, z + k + 2, ox, oy);
            cl.fill(sprite, color::surf(pal::BRICK_DARK_CLINKER, light, FACE_L));
            cr.fill(sprite, color::surf(color::shade(pal::BRICK_DARK_CLINKER, 0.8), light, FACE_R));
        }
        let cap = box_top(sx - 0.13, sy - 0.02, sx + 0.13, sy + 0.22, z + h + 1, ox, oy);
        sprite.on_surface(render::snow::EQUIP_TOP);
        cap.fill(sprite, color::surf(pal::BRICK_DARK_CLINKER, light, FACE_TOP));
        sprite.on_surface(render::snow::NONE);
        if light.is_night {
            let (px, py) = project(sx, sy + 0.1, (z + h) as f32, ox, oy);
            parts::smoke_plume(sprite, light, px as i32, py as i32 - 2, (z + h) as u16);
        }
    }
    // The chimney: on the roof's own ridge, so it pierces the slope.
    if d.chimney > 0 && d.family != Family::Industrial {
        let h = if d.chimney == 2 { 13 } else { 7 };
        let cx = fv.x0 + (fv.x1 - fv.x0) * (0.55 + 0.2 * color::hash01(d.seed as i32, 0x71));
        let cy = fv.y0 + (fv.y1 - fv.y0) * 0.5;
        let z = fv.z1 + ((d.z_crown - fv.z1) as f32 * 0.5) as i32;
        let brick = if d.finish == Finish::Stone { pal::STONE_GRANITE } else { pal::BRICK_RED };
        roofs::chimney(d, light, sprite, ox, oy, cx, cy, z, h, d.chimney == 2, brick);
    }
    // A shop's awning and sign band over the front.
    if matches!(d.family, Family::Shop) && d.base == super::Base::ShopFront {
        awning(d, light, sprite, ox, oy, x0, y1, x1, trim);
    }
    match d.family {
        Family::Industrial => works(spec, d, light, sprite, ox, oy, x0, y0, x1, y1, wall),
        Family::Civic => civic(d, light, sprite, ox, oy, x0, y0, x1, y1, wall, trim),
        _ => {}
    }
}

#[allow(clippy::too_many_arguments)]
fn porch(d: &Design, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32, x0: f32, y1: f32, x1: f32, _wall: Color, trim: Color) {
    let wide = d.porch >= 3;
    let w = if wide { (x1 - x0) * 0.9 } else { (x1 - x0) * 0.40 };
    let px0 = x0 + ((x1 - x0) - w) * 0.5;
    let px1 = px0 + w;
    let depth = if wide { 0.30 } else { 0.22 };
    let ph = if d.porch == 1 { 1 } else { 5 };
    let py0 = y1 - 0.02;
    let py1 = y1 + depth;
    if d.porch >= 2 {
        let deck = box_top(px0, py0, px1, py1, 1, ox, oy);
        deck.fill(sprite, color::surf(color::shade(pal::CONCRETE_DRIVE, 0.96), light, FACE_TOP));
    }
    // Steps for a stoop, posts and a little roof for a porch or veranda.
    let post = color::surf(trim, light, FACE_L);
    for &lx in &[px0 + 0.04, px1 - 0.04] {
        let (ax, ay) = project(lx, py1 - 0.04, 1.0, ox, oy);
        let (bx, by) = project(lx, py1 - 0.04, (1 + ph) as f32, ox, oy);
        geom::draw_line(sprite, ax as i32, ay as i32, bx as i32, by as i32, post, (1 + ph) as u16);
    }
    if d.porch >= 2 {
        let roof_c = color::surf(color::shade(roofs::roof_colour(d), 0.96), light, FACE_TOP);
        let (rl, _rr) = box_faces(px0 - 0.02, py0, px1 + 0.02, py1, 1 + ph, 2 + ph, ox, oy);
        rl.fill(sprite, roof_c);
        let top = box_top(px0 - 0.02, py0, px1 + 0.02, py1, 2 + ph, ox, oy);
        top.fill(sprite, roof_c);
    }
    if d.porch >= 3 {
        // A veranda rail along the deck's edge.
        let rail = color::surf(trim, light, FACE_R);
        let (rl, rr) = box_faces(px0, py1 - 0.03, px1, py1, 1, 1 + ph.min(2), ox, oy);
        for q in [rl, rr] {
            q.paint(sprite, &mut |u, v, _x, _y| {
                (u % 2 == 0 || v == 0).then_some((rail, (1 + ph.min(2) - v) as u16))
            });
        }
    }
}

fn garage(d: &Design, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32, gx: f32, gy: f32, gw: f32, floor: i32, wall: Color, trim: Color, attached: bool) {
    let gh = (floor + 2).min(15);
    let gd = (gw * 0.9).min(0.9);
    let (lf, rf) = box_faces(gx, gy, gx + gw, gy + gd, 0, gh, ox, oy);
    clad(d, &lf, sprite, light, wall, FACE_L);
    clad(d, &rf, sprite, light, wall, FACE_R);
    // The door: a wide roller door or two leaves, facing the drive.
    let door = color::surf(if attached { pal::TRIM_WHITE } else { pal::METAL_STEEL_DARK }, light, FACE_R);
    let len = rf.top_len();
    rf.paint(sprite, &mut |u, v, _x, _y| {
        if v >= gh - floor - 2 && u > 2 && u < len - 3 {
            let panel = v % 3 == 0;
            Some((if panel { color::shade(door, 0.86) } else { door }, (gh - v) as u16))
        } else {
            None
        }
    });
    // Its roof: a shallow gable in the house's own covering.
    let rise = (gh as f32 * 0.35) as i32;
    roofs::draw(d, light, sprite, ox, oy, gx, gy, gx + gw, gy + gd, gh, rise, wall);
    parts::corner_seam(sprite, light, gx + gw, gy + gd, 0, gh, trim, ox, oy);
}

#[allow(clippy::too_many_arguments)]
fn bay_window(d: &Design, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32, x0: f32, y1: f32, x1: f32, wall: Color, trim: Color) {
    let w = ((x1 - x0) * 0.28).clamp(0.3, 0.7);
    let bx = x0 + (x1 - x0) * 0.5 - w / 2.0;
    let z0 = d.floor - 3;
    let z1 = d.z_top - 2;
    let (lf, rf) = box_faces(bx, y1 - 0.04, bx + w, y1 + 0.14, z0, z1, ox, oy);
    clad(d, &lf, sprite, light, wall, FACE_L);
    clad(d, &rf, sprite, light, wall, FACE_R);
    let wo = WinOpts {
        w: (lf.top_len() - 3).clamp(3, 8),
        h: (z1 - z0 - 4).clamp(3, 8),
        frame: trim,
        sill: false,
        lintel: false,
        shutters: false,
        shutter_c: pal::SHUTTER_BLACK,
        seed: d.seed ^ 0x55,
        lit_p: 0.7,
    };
    parts::draw_window(&lf, sprite, light, 2, 2, wo, z1);
    let top = box_top(bx - 0.02, y1 - 0.04, bx + w + 0.02, y1 + 0.14, z1, ox, oy);
    top.fill(sprite, color::surf(color::shade(trim, 0.98), light, FACE_TOP));
    let _ = rf;
}

#[allow(clippy::too_many_arguments)]
fn awning(d: &Design, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32, x0: f32, y1: f32, x1: f32, trim: Color) {
    // A canvas awning over part of the shop front: a muted pair of colours, a
    // short valance and a shadow under it, so it reads as cloth, not paint.
    if color::hash_seed(d.seed, 0x500) % 100 >= 60 {
        return;
    }
    let (a, b) = match color::hash_seed(d.seed, 0x501) % 5 {
        0 => (rgb(0x9c, 0x3a, 0x36), rgb(0xc8, 0x5a, 0x50)),
        1 => (rgb(0x2e, 0x52, 0x7c), rgb(0x46, 0x6e, 0x9c)),
        2 => (rgb(0x2e, 0x6a, 0x50), rgb(0x46, 0x86, 0x68)),
        3 => (rgb(0x6a, 0x4a, 0x2e), rgb(0x8a, 0x66, 0x44)),
        _ => (rgb(0x53, 0x3a, 0x6a), rgb(0x6e, 0x52, 0x8a)),
    };
    let span = (x1 - x0) * 0.66;
    let ax0 = x0 + ((x1 - x0) - span) * 0.5;
    let ax1 = ax0 + span;
    let z_top = d.z_podium.max(8) + 1;
    let p = |x: f32, y: f32, z: i32| project(x, y, z as f32, ox, oy);
    let e = 0.20;
    let pts = [p(ax0, y1, z_top), p(ax1, y1, z_top), p(ax1, y1 + e, z_top - 4), p(ax0, y1 + e, z_top - 4)];
    let quad = Quad::new(pts[0], pts[1], pts[2], pts[3], z_top - 4, z_top);
    quad.paint(sprite, &mut |u, _v, _x, _y| {
        let c = if (u / 4) % 2 == 0 { a } else { b };
        Some((color::surf(c, light, FACE_TOP), z_top as u16))
    });
    // The valance and its shadow line.
    let (vl, vr) = box_faces(ax0, y1 + e - 0.02, ax1, y1 + e, z_top - 5, z_top - 4, ox, oy);
    let v = color::surf(color::shade(a, 0.9), light, FACE_L);
    vl.fill(sprite, v);
    vr.fill(sprite, color::surf(color::shade(a, 0.78), light, FACE_R));
    let _ = trim;
}

#[allow(clippy::too_many_arguments)]
fn works(
    spec: &BuildingSpec,
    d: &Design,
    light: &Light,
    sprite: &mut Sprite,
    ox: f32,
    oy: f32,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    wall: Color,
) {
    // Roller doors on the loading side.
    let doors = if d.w > 1.6 { 2 } else { 1 };
    for k in 0..doors {
        let fx = 0.18 + 0.42 * k as f32;
        let (lf, _rf) = box_faces(x0 + (x1 - x0) * fx, y1 - 0.02, x0 + (x1 - x0) * (fx + 0.2), y1 + 0.05, 0, d.floor + 2, ox, oy);
        lf.paint(sprite, &mut |_u, v, _x, _y| {
            let rib = v % 2 == 0;
            Some((color::surf(if rib { rgb(0x5a, 0x5e, 0x64) } else { rgb(0x6e, 0x72, 0x78) }, light, FACE_L), (d.floor + 2 - v) as u16))
        });
    }
    let _ = (spec, wall);
}

/// The works' silos: two cylinders on a plinth, with a domed cap and a seam band.
#[allow(clippy::too_many_arguments)]
fn silos(d: &Design, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32, x0: f32, y0: f32, y1: f32) {
    if color::hash_seed(d.seed, 0x601) % 100 >= 55 {
        return;
    }
    let r = 0.17f32;
    let h = (d.z_top + 12).clamp(20, 40);
    let mut cx = 0.08f32;
    for k in 0..2 {
        let cy = y0 + (y1 - y0) * (0.30 + 0.34 * k as f32);
        let (cxs, cys) = project(x0.max(0.0) + cx, cy, h as f32, ox, oy);
        let a = r * 32.0 * std::f32::consts::SQRT_2;
        let b = r * 16.0 * std::f32::consts::SQRT_2;
        for dx in -(a.ceil() as i32)..=(a.ceil() as i32) {
            let t = dx as f32 / a.max(1.0);
            let dy = b * (1.0 - (t * t).min(1.0)).sqrt();
            let shade = 0.76 + 0.30 * (-t * 0.5 + 0.5);
            for kk in 0..h {
                let seam = kk % 9 == 0;
                let c = if seam {
                    color::shade(pal::METAL_WHITE, shade * 0.82)
                } else {
                    color::shade(pal::METAL_WHITE, shade)
                };
                sprite.set_pixel(
                    cxs as i32 + dx,
                    (cys + dy) as i32 + kk,
                    color::surf(c, light, 1.0),
                    (h - kk) as u16,
                );
            }
            // The dome cap: a shallow arc over the cylinder's top.
            if dx.abs() < (a.ceil() as i32) - 1 {
                let cap = color::shade(pal::METAL_WHITE, 1.06);
                sprite.set_pixel(cxs as i32 + dx, (cys + dy) as i32 - 1, color::surf(cap, light, FACE_TOP), (h + 1) as u16);
            }
        }
        cx += 0.44;
    }
}

#[allow(clippy::too_many_arguments)]
fn civic(
    d: &Design,
    light: &Light,
    sprite: &mut Sprite,
    ox: f32,
    oy: f32,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    _wall: Color,
    _trim: Color,
) {
    match d.civic {
        CivicKind::School => {
            // The hard court in front of the block: tarmac with painted lines,
            // inside a low fence.
            let y_top = y1 + 0.02;
            let y_bot = (d.bd - 0.03).min(y_top + (d.bd - y1) * 0.9);
            if y_bot - y_top > 0.3 {
                let yard = box_top(x0 + 0.04, y_top, x1 - 0.04, y_bot, 1, ox, oy);
                yard.fill(sprite, color::surf(rgb(0x6c, 0x70, 0x76), light, FACE_TOP));
                let line = color::surf(pal::TRIM_WHITE, light, FACE_TOP);
                for k in 0..3 {
                    let lx = x0 + (x1 - x0) * (0.22 + 0.28 * k as f32);
                    let q = box_top(lx, y_top + 0.04, lx + 0.02, y_bot - 0.04, 1, ox, oy);
                    q.fill(sprite, line);
                }
            }
        }
        CivicKind::FireStation => {
            // Two apparatus bays and the drill tower.
            for k in 0..2 {
                let bx = x0 + 0.12 + 0.36 * k as f32;
                let (lf, _) = box_faces(bx, y1 - 0.02, bx + 0.28, y1 + 0.05, 0, d.floor + 3, ox, oy);
                lf.paint(sprite, &mut |u, v, _x, _y| {
                    let frame = u < 1 || u > 12 || v == 0;
                    let c = if frame { rgb(0xa8, 0x2e, 0x2e) } else { rgb(0x4a, 0x4e, 0x54) };
                    Some((color::surf(c, light, FACE_L), (d.floor + 3 - v) as u16))
                });
            }
            // (The drill tower is a volume of its own - see `volumes`.)
        }
        CivicKind::Library | CivicKind::TownHall => {
            // A portico of columns, steps, a pediment, and (a town hall) a clock.
            let len = x1 - x0;
            let cx = x0 + len * 0.5;
            let pw = (len * 0.5).min(1.4);
            let (px0, px1) = (cx - pw / 2.0, cx + pw / 2.0);
            let ph = d.z_top;
            // Steps.
            for k in 0..3 {
                let sy = y1 + 0.05 + 0.06 * k as f32;
                let q = box_top(px0 - 0.1 + 0.03 * k as f32, sy, px1 + 0.1 - 0.03 * k as f32, sy + 0.06, 1 + k, ox, oy);
                q.fill(sprite, color::surf(color::shade(pal::STONE_LIMESTONE, 0.96), light, FACE_TOP));
            }
            // Columns and an entablature.
            let n = ((px1 - px0) / 0.24).floor().max(2.0) as i32;
            for k in 0..=n {
                let cxp = px0 + (px1 - px0) * (k as f32 / n as f32);
                let (a0, b0) = project(cxp, y1 + 0.16, 3.0, ox, oy);
                let (a1, b1) = project(cxp, y1 + 0.16, (ph - 2) as f32, ox, oy);
                geom::draw_line(sprite, a0 as i32, b0 as i32, a1 as i32, b1 as i32, color::surf(pal::STONE_LIMESTONE, light, FACE_L), ph as u16);
                let (c0, d0) = project(cxp + 0.02, y1 + 0.18, 3.0, ox, oy);
                geom::draw_line(sprite, c0 as i32, d0 as i32, a1 as i32, b1 as i32, color::surf(color::shade(pal::STONE_LIMESTONE, 0.86), light, FACE_R), ph as u16);
            }
            let (el, er) = box_faces(px0 - 0.06, y1 + 0.04, px1 + 0.06, y1 + 0.2, ph - 2, ph + 1, ox, oy);
            let stone = color::surf(pal::STONE_LIMESTONE, light, FACE_L);
            el.fill(sprite, stone);
            er.fill(sprite, color::surf(color::shade(pal::STONE_LIMESTONE, 0.86), light, FACE_R));
            // The pediment.
            let p = |x: f32, y: f32, z: i32| project(x, y, z as f32, ox, oy);
            let ap = (ph as f32 + (pw * 0.35) as f32) as i32;
            let tri = [p(px0 - 0.06, y1 + 0.2, ph + 1), p(px1 + 0.06, y1 + 0.2, ph + 1), p(cx, y1 + 0.2, ap)];
            fill_polygon(sprite, &tri, color::surf(color::shade(pal::STONE_LIMESTONE, 1.05), light, FACE_TOP), ap as u16);
            if d.civic == CivicKind::TownHall {
                // A clock face on the pediment.
                let (ccx, ccy) = project(cx, y1 + 0.2, (ph + 3) as f32, ox, oy);
                for dy in -2..=2 {
                    for dx in -2..=2 {
                        if dx * dx + dy * dy <= 4 {
                            sprite.set_pixel(ccx as i32 + dx, ccy as i32 + dy, color::surf(rgb(0xf4, 0xf0, 0xdc), light, FACE_TOP), (ph + 5) as u16);
                        }
                    }
                }
            }
        }
        CivicKind::Hospital => {
            // The canopy over the ambulance bay.
            let cq = box_top(x0 + (x1 - x0) * 0.3, y1, x0 + (x1 - x0) * 0.7, y1 + 0.34, d.z_podium.max(9), ox, oy);
            cq.fill(sprite, color::surf(pal::METAL_WHITE, light, FACE_TOP));
        }
        CivicKind::Church => {
            // The spire on the tower volume (the tower itself is a volume).
            let tw = (d.w * 0.30).clamp(0.42, 0.95);
            let tx = x0 + 0.02;
            let ty = y0 + 0.02;
            let tz = d.z_top + 20;
            let p = |x: f32, y: f32, z: i32| project(x, y, z as f32, ox, oy);
            let apex = tz + 20;
            let t = [p(tx, ty, tz), p(tx + tw, ty, tz), p(tx + tw, ty + tw, tz), p(tx, ty + tw, tz)];
            fill_polygon(sprite, &[t[0], t[1], p(tx + tw / 2.0, ty + tw / 2.0, apex)], color::surf(color::shade(roofs::roof_colour(d), 1.0), light, FACE_TOP), apex as u16);
            fill_polygon(sprite, &[t[1], t[2], p(tx + tw / 2.0, ty + tw / 2.0, apex)], color::surf(color::shade(roofs::roof_colour(d), 0.84), light, FACE_R), apex as u16);
            fill_polygon(sprite, &[t[3], t[2], p(tx + tw / 2.0, ty + tw / 2.0, apex)], color::surf(color::shade(roofs::roof_colour(d), 0.72), light, FACE_L), apex as u16);
            // A cross.
            let (kx, ky) = project(tx + tw / 2.0, ty + tw / 2.0, (apex + 1) as f32, ox, oy);
            let cross = color::surf(pal::TRIM_WHITE, light, FACE_TOP);
            sprite.set_pixel(kx as i32, ky as i32 - 3, cross, (apex + 4) as u16);
            sprite.set_pixel(kx as i32, ky as i32 - 2, cross, (apex + 3) as u16);
            sprite.set_pixel(kx as i32 - 1, ky as i32 - 2, cross, (apex + 3) as u16);
            sprite.set_pixel(kx as i32 + 1, ky as i32 - 2, cross, (apex + 3) as u16);
        }
        CivicKind::None => {}
    }
}

/// Close single-pixel pinholes inside a drawn silhouette (H32's rule, and the
/// catalogue's own "no dropout" test).
pub(crate) fn close_pinpricks(sprite: &mut Sprite) {
    if sprite.w < 3 || sprite.h < 3 {
        return;
    }
    let (w, h) = (sprite.w, sprite.h);
    let drawn = |c: u32| c != 0 && !((c >> 24) < 0x80 && (c & 0xFF_FFFF) == 0x10_141c);
    for _ in 0..2 {
        let mut fix: Vec<(usize, u32, u16)> = Vec::new();
        for x in 0..w {
            for y in 1..h - 1 {
                let i = y * w + x;
                if sprite.px[i] != 0 {
                    continue;
                }
                let up = sprite.px[i - w];
                let down = sprite.px[i + w];
                if drawn(up) && drawn(down) {
                    let c = crate::color::mix(up | 0xFF00_0000, down | 0xFF00_0000, 0.5);
                    let depth = sprite.depth[i - w].max(sprite.depth[i + w]);
                    fix.push((i, c, depth));
                }
            }
        }
        if fix.is_empty() {
            break;
        }
        for (i, c, depth) in fix {
            sprite.px[i] = c;
            sprite.depth[i] = depth;
        }
    }
}

/// The building's own name for the report (the family, so a report can group).
pub fn family_name(d: &Design) -> &'static str {
    match d.family {
        Family::Tower => "tower",
        Family::House => "house",
        Family::Townhouse => "townhouse",
        Family::Shop => "shop",
        Family::Apartment => "apartment",
        Family::Office => "office",
        Family::Industrial => "industrial",
        Family::Civic => match d.civic {
            CivicKind::School => "school",
            CivicKind::FireStation => "fire station",
            CivicKind::Library => "library",
            CivicKind::TownHall => "town hall",
            CivicKind::Hospital => "hospital",
            CivicKind::Church => "church",
            CivicKind::None => "civic",
        },
    }
}

/// Unused today, kept so the module reads the same as the tower's: the body a
/// family's block is cut from.
pub fn _body(f: Family) -> super::Body {
    match f {
        Family::Office | Family::Apartment | Family::Shop => super::Body::Slab,
        _ => super::Body::Square,
    }
}

/// `Facade` is only used for the block path; name it so the import is not a
/// mystery in a house-only build.
const _: Option<Facade> = None;
