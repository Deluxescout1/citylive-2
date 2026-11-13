//! B73c: isometric roofs for the low-rise families.
//!
//! One painter per roof form, all built from the same four-corner slope
//! primitive: a quad whose top edge is higher on screen than its bottom edge,
//! painted through [`Quad::paint`] so the courses run parallel to the eave.
//! The forms are the ones a real street has — gable, cross gable, hip, gambrel,
//! mansard, saltbox, shed, half-hip, pyramid, flat — and the *covering*
//! ([`RoofMat`]) decides the texture: shingle courses, fine slate, ribbed tile,
//! standing-seam metal, straw, or a bare membrane.
//!
//! Eaves overhang by the design's `eaves` (the land's weather: deep in the
//! alpine and tropical lands, thin in the desert), so every roof throws a gutter
//! line over its wall — which is what makes a house read as a house at 1×.

use render::palette::{rgb, Color};

use super::pick::ROOF_TINTS;
use super::{Design, RoofForm, RoofMat};
use crate::color::{self, pal, FACE_L, FACE_R, FACE_TOP};
use crate::geom::{self, box_faces, box_top, project, Quad};
use crate::parts;
use crate::types::{Light, Sprite};

/// The roof's base colour: the design's tint, restricted to what the covering
/// allows (a membrane is always the flat-roof gravel, thatch is always straw).
pub fn roof_colour(d: &Design) -> Color {
    let t = ROOF_TINTS[(d.roof_tint as usize) % ROOF_TINTS.len()];
    match d.roof_mat {
        RoofMat::Membrane => pal::ROOF_GRAVEL,
        RoofMat::Thatch => rgb(0x9a, 0x7c, 0x48),
        _ => t,
    }
}

/// B76 r3: which snow tag a slope of this roof takes. A roof's pitch is the
/// design's own number (a percentage of the roof's short side), so a steep
/// alpine gable keeps less than a shallow desert hip — no guessing from pixels.
pub fn slope_tag(d: &Design, rise: i32, x0: f32, y0: f32, x1: f32, y1: f32) -> u8 {
    let short = (x1 - x0).min(y1 - y0).max(0.2);
    let half_px = 0.5 * short * 32.0;
    let _ = d;
    render::snow::slope_tag(rise as f32, half_px)
}

/// Paint one slope. `top` is the higher edge on screen (the ridge or the far
/// eave), `bot` the lower one, in projected screen coordinates.
#[allow(clippy::too_many_arguments)]
fn slope(
    sprite: &mut Sprite,
    light: &Light,
    top: ((f32, f32), (f32, f32)),
    bot: ((f32, f32), (f32, f32)),
    z_top: i32,
    z_bot: i32,
    d: &Design,
    base: Color,
    shade: f32,
    side: f32,
    tag: u8,
) {
    // Keep the quad upright whatever order the caller's corners came in: a far
    // slope's eave is higher on screen than its ridge.
    let (a, b, (ta, ba)) = if (top.0 .1 + top.1 .1) <= (bot.0 .1 + bot.1 .1) {
        (top, bot, (top.0, top.1))
    } else {
        (bot, top, (top.0, top.1))
    };
    let (zt, zb) = if (top.0 .1 + top.1 .1) <= (bot.0 .1 + bot.1 .1) {
        (z_top, z_bot)
    } else {
        (z_bot, z_top)
    };
    let _ = (ta, ba);
    let q = Quad::new(a.0, a.1, b.1, b.0, zb, zt);
    let rows = q.rows().max(1);
    let col = color::surf(color::shade(base, shade), light, side);
    let dark = color::surf(color::shade(base, shade * 0.84), light, side);
    let hi = color::surf(color::shade(base, shade * 1.10), light, side);
    let age = d.age;
    sprite.on_surface(tag);
    q.paint(sprite, &mut |u, v, x, y| {
        let depth = (zt as f32 - (zt as f32 - zb as f32) * (v as f32 / rows as f32)) as u16;
        let c = match d.roof_mat {
            RoofMat::Shingle => {
                let course = v / 3;
                let stagger = (course + (u / 5)) % 2 == 0;
                if v % 3 == 0 {
                    dark
                } else if !stagger && v % 3 == 2 {
                    color::shade(col, 0.94)
                } else {
                    color::shade(col, color::jcell(x, y, 3, 0.05))
                }
            }
            RoofMat::Slate => {
                if v % 2 == 0 {
                    color::shade(col, 0.88)
                } else if (u + (v / 2) * 3) % 7 == 0 {
                    color::shade(col, 0.80)
                } else {
                    color::shade(col, color::jcell(x, y, 4, 0.04))
                }
            }
            RoofMat::Tile => {
                if v % 3 == 0 {
                    dark
                } else if u % 4 == 0 {
                    color::shade(col, 0.90)
                } else {
                    hi
                }
            }
            RoofMat::Metal => {
                if u % 5 == 0 {
                    color::shade(base, shade * 0.78)
                } else if u % 5 == 1 {
                    hi
                } else {
                    color::shade(col, color::jcell(x / 5, y, 6, 0.03))
                }
            }
            RoofMat::Thatch => {
                let straw = color::hash01(x / 2, y / 2);
                color::shade(col, 0.88 + 0.22 * straw)
            }
            RoofMat::Membrane => color::shade(col, color::jcell(x, y, 3, 0.05)),
        };
        Some((color::weather(c, x, y, age), depth))
    });
    // A gutter/edge line along the eave, so the roof's edge reads at 1x. Under
    // snow it is the edge the load hangs over, so it is tagged as a ledge.
    let e = color::surf(color::shade(base, shade * 0.66), light, side);
    sprite.on_surface(render::snow::LEDGE);
    geom::draw_line(
        sprite,
        b.0 .0.round() as i32,
        b.0 .1.round() as i32,
        b.1 .0.round() as i32,
        b.1 .1.round() as i32,
        e,
        zb as u16,
    );
    sprite.on_surface(render::snow::NONE);
}

/// Draw a roof over a volume. `z_eave` is the wall top; the roof reaches `rise`
/// pixels above it and overhangs the walls by the design's eaves.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    d: &Design,
    light: &Light,
    sprite: &mut Sprite,
    ox: f32,
    oy: f32,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    z_eave: i32,
    rise: i32,
    wall: Color,
) {
    let e = (d.eaves.max(2) as f32) / 100.0;
    let (ax0, ay0, ax1, ay1) = (x0 - e, y0 - e, x1 + e, y1 + e);
    let base = roof_colour(d);
    let zr = z_eave + rise;
    // The roof the renderer's shadow pass sees (B71/D74): one pitched form.
    let form = match d.roof {
        RoofForm::Hip | RoofForm::Pyramid => 2,
        RoofForm::Mansard => 3,
        RoofForm::Flat => 0,
        _ => 1,
    };
    if form != 0 {
        sprite.roof = Some(render::scene::RoofInfo {
            x0,
            y0,
            x1,
            y1,
            eave: z_eave as f32,
            ridge: zr as f32,
            form,
        });
    }
    let p = |x: f32, y: f32, z: i32| project(x, y, z as f32, ox, oy);
    // A thatched covering is its own geometry (a deep, ragged hip), whatever
    // form the land's table picked.
    if matches!(d.roof_mat, RoofMat::Thatch) && !matches!(d.roof, RoofForm::Flat) {
        thatch(d, light, sprite, ox, oy, ax0, ay0, ax1, ay1, z_eave, rise, base);
        return;
    }
    // Ridge along the longer side (a short frontage gets a gable end to the
    // street, a long one a long ridge).
    let along_x = (x1 - x0) >= (y1 - y0);

    match d.roof {
        RoofForm::Flat => flat(d, light, sprite, ox, oy, x0, y0, x1, y1, z_eave, rise, base, wall),
        RoofForm::Gable | RoofForm::Saltbox | RoofForm::HalfHip | RoofForm::GableCross => {
            let form = if matches!(d.roof, RoofForm::GableCross) {
                RoofForm::Gable
            } else {
                d.roof
            };
            gable(d, light, sprite, ox, oy, ax0, ay0, ax1, ay1, z_eave, rise, base, wall, form, along_x);
            if matches!(d.roof, RoofForm::GableCross) {
                // The front gable: a narrower roof standing over the entrance
                // side, its own ridge running to the street.
                let cx0 = x0 + (x1 - x0) * 0.18;
                let cx1 = x1 - (x1 - x0) * 0.18;
                gable(d, light, sprite, ox, oy, cx0, y1 - (y1 - y0) * 0.55, cx1, y1 + e, z_eave, (rise as f32 * 0.92) as i32, base, wall, RoofForm::Gable, !along_x);
            }
        }
        RoofForm::Hip => hip(d, light, sprite, ox, oy, ax0, ay0, ax1, ay1, z_eave, rise, base, along_x),
        RoofForm::Pyramid => pyramid(d, light, sprite, ox, oy, ax0, ay0, ax1, ay1, z_eave, rise, base),
        RoofForm::Gambrel => gambrel(d, light, sprite, ox, oy, ax0, ay0, ax1, ay1, z_eave, rise, base, along_x),
        RoofForm::Mansard => mansard(d, light, sprite, ox, oy, x0, y0, x1, y1, z_eave, rise, base, wall),
        RoofForm::Shed => shed(d, light, sprite, ox, oy, ax0, ay0, ax1, ay1, z_eave, rise, base, wall, along_x),
    }
    let _ = p;
}

#[allow(clippy::too_many_arguments)]
fn flat(
    d: &Design,
    light: &Light,
    sprite: &mut Sprite,
    ox: f32,
    oy: f32,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    z: i32,
    parapet: i32,
    base: Color,
    wall: Color,
) {
    let coping = if matches!(d.roof_mat, RoofMat::Membrane) {
        color::shade(wall, 1.02)
    } else {
        color::shade(base, 1.10)
    };
    parts::roof_flat(sprite, light, x0, y0, x1, y1, z, parapet.max(2), coping, ox, oy, d.age);
}

#[allow(clippy::too_many_arguments)]
fn gable(
    d: &Design,
    light: &Light,
    sprite: &mut Sprite,
    ox: f32,
    oy: f32,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    z: i32,
    rise: i32,
    base: Color,
    wall: Color,
    form: RoofForm,
    along_x: bool,
) {
    let p = |x: f32, y: f32, zz: i32| project(x, y, zz as f32, ox, oy);
    let zr = z + rise;
    let st = slope_tag(d, rise, x0, y0, x1, y1);
    if along_x {
        // Ridge along x. The saltbox pushes it towards the back.
        let my = match form {
            RoofForm::Saltbox => y0 + (y1 - y0) * 0.34,
            _ => (y0 + y1) / 2.0,
        };
        // The half-hip: the ridge stops short and the ends fall away.
        let (r0, r1) = if matches!(form, RoofForm::HalfHip) {
            (x0 + (x1 - x0) * 0.16, x1 - (x1 - x0) * 0.16)
        } else {
            (x0, x1)
        };
        // Gable-end walls first (the near end is the visible one).
        gable_tri(sprite, light, p(x1, y0, z), p(x1, y1, z), p(x1, my, zr), wall, FACE_R, zr);
        gable_tri(sprite, light, p(x0, y0, z), p(x0, y1, z), p(x0, my, zr), wall, FACE_L, zr);
        // The far (back) slope, then the near one over it.
        slope(sprite, light, (p(r0, my, zr), p(r1, my, zr)), (p(x0, y0, z), p(x1, y0, z)), zr, z, d, base, 1.0, FACE_TOP, st);
        slope(sprite, light, (p(r0, my, zr), p(r1, my, zr)), (p(x0, y1, z), p(x1, y1, z)), zr, z, d, base, 1.0, FACE_R, st);
        ridge_cap(sprite, light, p(r0, my, zr), p(r1, my, zr), base, zr);
    } else {
        let mx = match form {
            RoofForm::Saltbox => x0 + (x1 - x0) * 0.34,
            _ => (x0 + x1) / 2.0,
        };
        let (r0, r1) = if matches!(form, RoofForm::HalfHip) {
            (y0 + (y1 - y0) * 0.16, y1 - (y1 - y0) * 0.16)
        } else {
            (y0, y1)
        };
        gable_tri(sprite, light, p(x0, y1, z), p(x1, y1, z), p(mx, y1, zr), wall, FACE_L, zr);
        gable_tri(sprite, light, p(x0, y0, z), p(x1, y0, z), p(mx, y0, zr), wall, FACE_R, zr);
        // The far slope is the x0 side (up-screen on the left).
        slope(sprite, light, (p(mx, r0, zr), p(mx, r1, zr)), (p(x0, y0, z), p(x0, y1, z)), zr, z, d, base, 0.94, FACE_L, st);
        slope(sprite, light, (p(mx, r0, zr), p(mx, r1, zr)), (p(x1, y0, z), p(x1, y1, z)), zr, z, d, base, 1.0, FACE_R, st);
        ridge_cap(sprite, light, p(mx, r0, zr), p(mx, r1, zr), base, zr);
    }
}

/// The triangle of wall a gable leaves above the eave, drawn in the wall's own
/// cladding colour (so a brick gable is a brick gable). It is a wall, so no snow
/// lies on it (B76 r3).
fn gable_tri(sprite: &mut Sprite, light: &Light, a: (f32, f32), b: (f32, f32), apex: (f32, f32), wall: Color, side: f32, zr: i32) {
    sprite.on_surface(render::snow::NONE);
    geom::fill_polygon(
        sprite,
        &[a, b, apex],
        color::surf(color::shade(wall, 0.96), light, side),
        zr as u16,
    );
}

#[allow(clippy::too_many_arguments)]
fn hip(
    d: &Design,
    light: &Light,
    sprite: &mut Sprite,
    ox: f32,
    oy: f32,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    z: i32,
    rise: i32,
    base: Color,
    along_x: bool,
) {
    let p = |x: f32, y: f32, zz: i32| project(x, y, zz as f32, ox, oy);
    let zr = z + rise;
    let st = slope_tag(d, rise, x0, y0, x1, y1);
    let (mx, my) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    if along_x {
        let inset = ((y1 - y0) * 0.5).min((x1 - x0) * 0.4);
        let (r0, r1) = (x0 + inset, x1 - inset);
        // Far slope, then the near slope, then the two end triangles.
        slope(sprite, light, (p(r0, my, zr), p(r1, my, zr)), (p(x0, y0, z), p(x1, y0, z)), zr, z, d, base, 0.98, FACE_TOP, st);
        slope(sprite, light, (p(r0, my, zr), p(r1, my, zr)), (p(x0, y1, z), p(x1, y1, z)), zr, z, d, base, 1.0, FACE_R, st);
        slope(sprite, light, (p(r0, my, zr), p(r0, my, zr)), (p(x0, y0, z), p(x0, y1, z)), zr, z, d, base, 0.90, FACE_L, st);
        slope(sprite, light, (p(r1, my, zr), p(r1, my, zr)), (p(x1, y0, z), p(x1, y1, z)), zr, z, d, base, 0.96, FACE_R, st);
        ridge_cap(sprite, light, p(r0, my, zr), p(r1, my, zr), base, zr);
    } else {
        let inset = ((x1 - x0) * 0.5).min((y1 - y0) * 0.4);
        let (r0, r1) = (y0 + inset, y1 - inset);
        slope(sprite, light, (p(mx, r0, zr), p(mx, r1, zr)), (p(x0, y0, z), p(x0, y1, z)), zr, z, d, base, 0.98, FACE_L, st);
        slope(sprite, light, (p(mx, r0, zr), p(mx, r1, zr)), (p(x1, y0, z), p(x1, y1, z)), zr, z, d, base, 1.0, FACE_R, st);
        slope(sprite, light, (p(mx, r0, zr), p(mx, r0, zr)), (p(x0, y0, z), p(x1, y0, z)), zr, z, d, base, 0.94, FACE_TOP, st);
        slope(sprite, light, (p(mx, r1, zr), p(mx, r1, zr)), (p(x0, y1, z), p(x1, y1, z)), zr, z, d, base, 0.96, FACE_R, st);
        ridge_cap(sprite, light, p(mx, r0, zr), p(mx, r1, zr), base, zr);
    }
}

#[allow(clippy::too_many_arguments)]
fn pyramid(
    d: &Design,
    light: &Light,
    sprite: &mut Sprite,
    ox: f32,
    oy: f32,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    z: i32,
    rise: i32,
    base: Color,
) {
    let p = |x: f32, y: f32, zz: i32| project(x, y, zz as f32, ox, oy);
    let zr = z + rise;
    let st = slope_tag(d, rise, x0, y0, x1, y1);
    let (mx, my) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    let apex = p(mx, my, zr);
    // Far slopes first, then the two near ones.
    slope(sprite, light, (apex, apex), (p(x0, y0, z), p(x1, y0, z)), zr, z, d, base, 0.98, FACE_TOP, st);
    slope(sprite, light, (apex, apex), (p(x0, y0, z), p(x0, y1, z)), zr, z, d, base, 0.92, FACE_L, st);
    slope(sprite, light, (apex, apex), (p(x1, y0, z), p(x1, y1, z)), zr, z, d, base, 1.0, FACE_R, st);
    slope(sprite, light, (apex, apex), (p(x0, y1, z), p(x1, y1, z)), zr, z, d, base, 0.96, FACE_R, st);
}

#[allow(clippy::too_many_arguments)]
fn gambrel(
    d: &Design,
    light: &Light,
    sprite: &mut Sprite,
    ox: f32,
    oy: f32,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    z: i32,
    rise: i32,
    base: Color,
    along_x: bool,
) {
    let p = |x: f32, y: f32, zz: i32| project(x, y, zz as f32, ox, oy);
    // Two pitches a side: a steep lower slope to the break, a shallow upper.
    let zb = z + (rise as f32 * 0.62) as i32;
    let zr = z + rise;
    let st = slope_tag(d, rise, x0, y0, x1, y1);
    if along_x {
        let my = (y0 + y1) / 2.0;
        let b0 = y0 + (my - y0) * 0.45;
        let b1 = y1 - (y1 - my) * 0.45;
        slope(sprite, light, (p(x0, b0, zb), p(x1, b0, zb)), (p(x0, y0, z), p(x1, y0, z)), zb, z, d, base, 1.0, FACE_TOP, st);
        slope(sprite, light, (p(x0, my, zr), p(x1, my, zr)), (p(x0, b0, zb), p(x1, b0, zb)), zr, zb, d, base, 1.0, FACE_TOP, st);
        slope(sprite, light, (p(x0, my, zr), p(x1, my, zr)), (p(x0, b1, zb), p(x1, b1, zb)), zr, zb, d, base, 0.94, FACE_R, st);
        slope(sprite, light, (p(x0, b1, zb), p(x1, b1, zb)), (p(x0, y1, z), p(x1, y1, z)), zb, z, d, base, 1.0, FACE_R, st);
        // The big gable ends: a pair of wall triangles each.
        for (plane, side) in [(x1, FACE_R), (x0, FACE_L)] {
            let tri = [
                if side == FACE_L { p(plane, y0, z) } else { p(plane, y0, z) },
                if side == FACE_L { p(plane, y1, z) } else { p(plane, y1, z) },
                if side == FACE_L { p(plane, (y0 + y1) / 2.0, zr) } else { p(plane, (y0 + y1) / 2.0, zr) },
            ];
            let _ = plane;
            sprite.on_surface(render::snow::NONE);
            geom::fill_polygon(sprite, &tri, color::surf(color::shade(base, 0.98), light, side), zr as u16);
        }
        ridge_cap(sprite, light, p(x0, my, zr), p(x1, my, zr), base, zr);
    } else {
        let mx = (x0 + x1) / 2.0;
        let b0 = x0 + (mx - x0) * 0.45;
        let b1 = x1 - (x1 - mx) * 0.45;
        slope(sprite, light, (p(b0, y0, zb), p(b0, y1, zb)), (p(x0, y0, z), p(x0, y1, z)), zb, z, d, base, 1.0, FACE_L, st);
        slope(sprite, light, (p(mx, y0, zr), p(mx, y1, zr)), (p(b0, y0, zb), p(b0, y1, zb)), zr, zb, d, base, 1.0, FACE_L, st);
        slope(sprite, light, (p(mx, y0, zr), p(mx, y1, zr)), (p(b1, y0, zb), p(b1, y1, zb)), zr, zb, d, base, 0.94, FACE_R, st);
        slope(sprite, light, (p(b1, y0, zb), p(b1, y1, zb)), (p(x1, y0, z), p(x1, y1, z)), zb, z, d, base, 1.0, FACE_R, st);
        for side in [FACE_L, FACE_R] {
            let plane = if side == FACE_L { y0 } else { y1 };
            let tri = [p(x0, plane, z), p(x1, plane, z), p((x0 + x1) / 2.0, plane, zr)];
            geom::fill_polygon(sprite, &tri, color::surf(color::shade(base, 0.98), light, side), zr as u16);
        }
        ridge_cap(sprite, light, p(mx, y0, zr), p(mx, y1, zr), base, zr);
    }
}

#[allow(clippy::too_many_arguments)]
fn mansard(
    d: &Design,
    light: &Light,
    sprite: &mut Sprite,
    ox: f32,
    oy: f32,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    z: i32,
    rise: i32,
    base: Color,
    wall: Color,
) {
    // The catalogue's own mansard geometry: steep faces, an inset deck and a
    // cresting line, so a Paris block reads at 1x.
    parts::roof_mansard(sprite, light, x0, y0, x1, y1, z, rise, base, ox, oy, d.age);
    // A dormer in the near slope, so the roof is lived-in (brief §2).
    let _ = (wall, light);
}

#[allow(clippy::too_many_arguments)]
fn shed(
    d: &Design,
    light: &Light,
    sprite: &mut Sprite,
    ox: f32,
    oy: f32,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    z: i32,
    rise: i32,
    base: Color,
    wall: Color,
    along_x: bool,
) {
    let p = |x: f32, y: f32, zz: i32| project(x, y, zz as f32, ox, oy);
    let zr = z + rise;
    let st = slope_tag(d, rise, x0, y0, x1, y1);
    if along_x {
        // High at the back (y0), falling to the street.
        slope(sprite, light, (p(x0, y0, zr), p(x1, y0, zr)), (p(x0, y1, z), p(x1, y1, z)), zr, z, d, base, 1.0, FACE_TOP, st);
        // The end walls are triangles: the back is taller than the front.
        for side in [FACE_L, FACE_R] {
            let plane = if side == FACE_L { x0 } else { x1 };
            let tri = [p(plane, y0, z), p(plane, y1, z), p(plane, y0, zr)];
            sprite.on_surface(render::snow::NONE);
            geom::fill_polygon(sprite, &tri, color::surf(color::shade(wall, 0.96), light, side), zr as u16);
        }
    } else {
        slope(sprite, light, (p(x0, y0, zr), p(x0, y1, zr)), (p(x1, y0, z), p(x1, y1, z)), zr, z, d, base, 1.0, FACE_L, st);
        for side in [FACE_L, FACE_R] {
            let plane = if side == FACE_L { y0 } else { y1 };
            let tri = [p(x0, plane, z), p(x1, plane, z), p(x0, plane, zr)];
            sprite.on_surface(render::snow::NONE);
            geom::fill_polygon(sprite, &tri, color::surf(color::shade(wall, 0.96), light, side), zr as u16);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn thatch(
    d: &Design,
    light: &Light,
    sprite: &mut Sprite,
    ox: f32,
    oy: f32,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    z: i32,
    rise: i32,
    base: Color,
) {
    parts::roof_thatch(sprite, light, x0, y0, x1, y1, z, rise, base, ox, oy, d.age);
}

fn ridge_cap(sprite: &mut Sprite, light: &Light, a: (f32, f32), b: (f32, f32), base: Color, z: i32) {
    // B76 r3: the crest keeps its own line under snow, so a gable still reads.
    sprite.on_surface(render::snow::RIDGE);
    geom::draw_line(
        sprite,
        a.0.round() as i32,
        a.1.round() as i32,
        b.0.round() as i32,
        b.1.round() as i32,
        color::surf(color::shade(base, 1.14), light, FACE_TOP),
        z as u16,
    );
    sprite.on_surface(render::snow::NONE);
}

/// A dormer in a roof slope: a small gabled box with a window, sitting on the
/// eave and leaning up the slope.
#[allow(clippy::too_many_arguments)]
pub fn dormer(
    d: &Design,
    light: &Light,
    sprite: &mut Sprite,
    ox: f32,
    oy: f32,
    cx: f32,
    y_face: f32,
    z_eave: i32,
    h: i32,
    wall: Color,
) {
    let w = 0.26;
    let depth = 0.26;
    let y0 = y_face - depth;
    let (lf, rf) = box_faces(cx - w / 2.0, y0, cx + w / 2.0, y_face, z_eave, z_eave + h, ox, oy);
    lf.fill(sprite, color::surf(color::shade(wall, 0.98), light, FACE_L));
    rf.fill(sprite, color::surf(color::shade(wall, 0.82), light, FACE_R));
    let wo = parts::WinOpts {
        w: 5,
        h: 6,
        frame: pal::TRIM_WHITE,
        sill: true,
        lintel: false,
        shutters: false,
        shutter_c: pal::SHUTTER_BLACK,
        seed: d.seed ^ 0x77,
        lit_p: 0.75,
    };
    parts::draw_window(&lf, sprite, light, 2, 2, wo, z_eave + h);
    // Its little roof: a gable whose ridge runs out of the slope.
    let base = roof_colour(d);
    let p = |x: f32, y: f32, zz: i32| project(x, y, zz as f32, ox, oy);
    let apex_z = z_eave + h + (h / 2).max(3);
    let top = color::surf(color::shade(base, 1.02), light, FACE_TOP);
    let shaded = color::surf(color::shade(base, 0.86), light, FACE_R);
    // B76 r3: a dormer's own little roof holds snow like any other slope.
    sprite.on_surface(render::snow::SLOPE_MEDIUM);
    geom::fill_polygon(sprite, &[p(cx - w / 2.0, y0, z_eave + h), p(cx - w / 2.0, y_face, z_eave + h), p(cx, y_face, apex_z)], top, apex_z as u16);
    geom::fill_polygon(sprite, &[p(cx, y_face, apex_z), p(cx + w / 2.0, y_face, z_eave + h), p(cx + w / 2.0, y0, z_eave + h)], shaded, apex_z as u16);
    sprite.on_surface(render::snow::NONE);
}

/// A chimney stack: brick or stone, with a cap and (at night) a plume.
#[allow(clippy::too_many_arguments)]
pub fn chimney(
    d: &Design,
    light: &Light,
    sprite: &mut Sprite,
    ox: f32,
    oy: f32,
    x: f32,
    y: f32,
    z: i32,
    h: i32,
    tall: bool,
    brick: Color,
) {
    let (w, dd) = if tall { (0.30, 0.30) } else { (0.20, 0.20) };
    let (lf, rf) = box_faces(x, y, x + w, y + dd, z, z + h, ox, oy);
    // B76 r3: a stack pokes out of the load dark, with snow only on its cap.
    sprite.on_surface(render::snow::EQUIP_SIDE);
    lf.fill(sprite, color::surf(brick, light, FACE_L));
    rf.fill(sprite, color::surf(color::shade(brick, 0.80), light, FACE_R));
    let cap = box_top(x - 0.02, y - 0.02, x + w + 0.02, y + dd + 0.02, z + h, ox, oy);
    sprite.on_surface(render::snow::EQUIP_TOP);
    cap.fill(sprite, color::surf(color::shade(brick, 0.72), light, FACE_TOP));
    sprite.on_surface(render::snow::NONE);
    if light.is_night {
        let (px, py) = project(x + w / 2.0, y + dd / 2.0, (z + h) as f32, ox, oy);
        parts::smoke_plume(sprite, light, px as i32, py as i32 - 1, (z + h) as u16);
    }
    let _ = d;
}
