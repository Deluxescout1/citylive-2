//! B73b: drawing towers from a [`Design`].
//!
//! The geometry is deliberately plain — iso boxes, one cylinder, a few slanted
//! quads — because at 64-pixel tiles a tower is 60-130 px wide and 40-250 px
//! tall: silhouettes, floor rhythm and colour read; anything finer does not.
//! What matters is that the *combination* is different per lot: a podium, a
//! stepping profile, a facade rhythm, a crown and a material, with the lot's own
//! height driving the whole thing.
//!
//! B73c's bodies and tops go next to these (same [`Band`] model, same facade
//! painter), which is why the band/deck machinery is public inside the crate.

use render::palette::rgb;

use super::paint::{self, mats};
use super::{Body, Design, Massing, Top};
use crate::color::{self, pal, FACE_L, FACE_R, FACE_TOP};
use crate::geom::{self, box_faces, box_top, fill_polygon, project, Quad};
use crate::parts;
use crate::types::{Light, Sprite};

/// One rectangular mass of a band: a footprint at a height range.
#[derive(Clone, Copy, Debug)]
struct Band {
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    z0: i32,
    z1: i32,
    /// An open planted floor (a sky garden), drawn without a wall.
    garden: bool,
    /// This is the band the crown sits on.
    crown_band: bool,
}

/// Draw a generated tower.
pub fn draw(_spec: &crate::types::BuildingSpec, d: &Design, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    // Q4 (round 2): a slim tower on a big lot stands on a small paved plaza —
    // pavers, planters and a path out to the pavement — instead of open grass.
    plaza(d, light, sprite, ox, oy);

    if d.led_edge {
        // A thin LED line up the tower's two visible corners (round 2 item 3),
        // lit with the evening like the crown.
        let c = d.crown_lamp(light, false);
        for (lx, ly) in [(d.bw * 0.5 + d.w / 2.0, d.bd * 0.5 + d.d / 2.0), (d.bw * 0.5 + d.w / 2.0, d.bd * 0.5 - d.d / 2.0)] {
            let (x0, y0) = project(lx, ly, d.z_podium as f32, ox, oy);
            let (x1, y1) = project(lx, ly, d.z_top as f32, ox, oy);
            if light.night > 0.05 {
                geom::draw_line(sprite, x0 as i32, y0 as i32, x1 as i32, y1 as i32, c, d.z_top as u16);
            }
        }
    }

    // The cast shadow the cache drops (it is transparent in the sprite, so the
    // engine never blits it, but the silhouette still reads from above).
    parts::building_shadow(sprite, 0.0, 0.0, d.bw, d.bd, d.z_crown, ox, oy);

    if d.z_podium > 0 {
        let (lf, rf) = box_faces(0.0, 0.0, d.bw, d.bd, 0, d.z_podium, ox, oy);
        paint::paint_base(d.base, &lf, sprite, light, d, 1, FACE_L);
        paint::paint_base(d.base, &rf, sprite, light, d, 2, FACE_R);
        parts::corner_seam(sprite, light, d.bw, d.bd, 0, d.z_podium, mats(d.material).metal, ox, oy);
        // The terrace beside the shaft, where the body stands back: a paved
        // deck with a parapet and planting, never a blank white slab.
        let m = mats(d.material);
        parts::roof_flat(sprite, light, 0.0, 0.0, d.bw, d.bd, d.z_podium, 2, m.metal, ox, oy, d.age);
        let plant = color::surf(pal::FOLIAGE_GREEN, light, FACE_TOP);
        let paving = color::surf(color::shade(pal::CONCRETE_DRIVE, 0.92), light, FACE_TOP);
        for k in 0..3 {
            let fx = 0.14 + 0.32 * k as f32;
            let (px, py) = project(d.bw * fx, d.bd * (0.2 + 0.28 * ((k * 7) % 3) as f32), (d.z_podium + 1) as f32, ox, oy);
            sprite.set_pixel(px as i32, py as i32, paving, (d.z_podium + 1) as u16);
            sprite.set_pixel(px as i32, py as i32 - 1, plant, (d.z_podium + 2) as u16);
        }
    }

    let bands = bands(d);
    for b in &bands {
        draw_band(d, b, light, sprite, ox, oy);
    }

    if d.z_crown > d.z_top {
        if let Some(top) = bands.iter().rev().find(|b| b.crown_band).copied() {
            crown(d, &top, light, sprite, ox, oy);
        }
    }
    // A diagonal edge (a roof coping, a chamfer, a slanted quad) can leave a
    // single unpainted pixel inside the silhouette where two rasterised lines
    // meet. The catalogue's own test forbids those dropouts, and at 1x they read
    // as a pinhole in the wall, so close them: a pixel with a drawn neighbour
    // straight above and straight below is inside the building.
    close_pinpricks(sprite);
}

/// The vertical profile of a design: its bands, bottom to top.
fn bands(d: &Design) -> Vec<Band> {
    let bx = ((d.bw - d.w) / 2.0).max(0.0);
    let by = ((d.bd - d.d) / 2.0).max(0.0);
    let (w, dd) = (d.w, d.d);
    let (zb, zt) = (d.z_podium, d.z_top);
    let h = (zt - zb).max(1);
    let floor = d.floor.max(3);

    // The massing rule's steps, as (top fraction, cumulative inset).
    let steps: Vec<(f32, f32)> = if matches!(d.body, Body::Wedge) {
        // A wedge leans back over its top third: the slope is a crown, not the
        // whole face (a full-height slope read as a sail).
        vec![(0.68, 0.0), (1.0, 0.0)]
    } else {
        match d.massing {
        Massing::None => vec![(1.0, 0.0)],
        Massing::OneStep => vec![(0.74, 0.0), (1.0, 0.085)],
        Massing::WeddingCake => vec![(0.48, 0.0), (0.72, 0.07), (0.88, 0.14), (1.0, 0.21)],
        Massing::Tapering => (1..=6).map(|k| (k as f32 / 6.0, 0.022 * k as f32)).collect(),
        // The sky garden is handled below: a gap band, no wall.
        Massing::SkyGarden | Massing::Staggered => vec![(1.0, 0.0)],
        }
    };

    let mut out: Vec<Band> = Vec::new();
    let mut prev = zb;
    for (i, (frac, inset)) in steps.iter().enumerate() {
        let z1 = if (*frac - 1.0).abs() < 0.001 {
            zt
        } else {
            zb + (h as f32 * frac) as i32
        };
        let (ix, iy) = match d.massing {
            // Staggered: the first step comes in along x only, the second along
            // y too, so the corners step rather than the whole box.
            Massing::Staggered if i == 1 => (0.07, 0.0),
            Massing::Staggered if i >= 2 => (0.07, 0.07),
            _ => (*inset, *inset),
        };
        let band = Band {
            x0: bx + ix,
            y0: by + iy,
            x1: bx + w - ix,
            y1: by + dd - iy,
            z0: prev,
            z1,
            garden: false,
            crown_band: i + 1 == steps.len(),
        };
        push_plan(d, band, i as u32, &mut out);
        prev = z1;
    }

    // The sky garden: an open planted floor set between two masses.
    if matches!(d.massing, Massing::SkyGarden) {
        let gap = (floor * 2).min(h / 3).max(3);
        let mid = zb + (h as f32 * 0.55) as i32;
        let mut angled: Vec<Band> = Vec::new();
        for b in out.drain(..) {
            let split = b.z0 < mid && mid < b.z1;
            if split {
                let mut lower = b;
                lower.z1 = mid;
                lower.crown_band = false;
                angled.push(lower);
                let mut gap_band = b;
                gap_band.z0 = mid;
                gap_band.z1 = (mid + gap).min(zt);
                gap_band.garden = true;
                gap_band.crown_band = false;
                angled.push(gap_band);
                let mut upper = b;
                upper.z0 = (mid + gap).min(zt);
                upper.x0 += 0.05;
                upper.x1 -= 0.05;
                upper.y0 += 0.05;
                upper.y1 -= 0.05;
                upper.crown_band = true;
                angled.push(upper);
            } else {
                angled.push(b);
            }
        }
        out = angled;
    }

    // Bodies that carry their own massing, whatever rule was picked.
    out = match d.body {
        Body::Twisting => twist_bands(d),
        Body::Stacked => stacked_bands(d),
        Body::Twin => twin_bands(d),
        Body::Tapered => taper_bands(d),
        _ => out,
    };

    // Slabs are long and thin; the plan already narrows the footprint.
    out
}

/// An L, cross or plain plan: split the band into its volumes.
fn push_plan(d: &Design, b: Band, idx: u32, out: &mut Vec<Band>) {
    let (w, dd) = (b.x1 - b.x0, b.y1 - b.y0);
    match d.body {
        Body::LPlan => {
            // A main volume with a lower wing along one side. The wing runs the
            // full height of the bands it occupies and stops at a band edge, so
            // its roof is a terrace beside the tower, never a slab hanging over
            // the gap below it.
            let cap = d.z_podium + ((d.z_top - d.z_podium) as f32 * 0.55) as i32;
            let mut main = b;
            main.y1 = b.y0 + dd * 0.62;
            out.push(main);
            if b.z0 < cap {
                let mut side = b;
                side.x1 = b.x0 + (w * 0.42).max(0.3);
                side.z1 = b.z1.min(cap);
                side.crown_band = false;
                out.push(side);
            }
        }
        Body::Cross => {
            // A main slab with a perpendicular bar through it.
            let mut main = b;
            main.y0 = b.y0 + dd * 0.34;
            main.y1 = b.y0 + dd * 0.66;
            out.push(main);
            let mut bar = b;
            bar.x0 = b.x0 + w * 0.34;
            bar.x1 = b.x0 + w * 0.66;
            out.push(bar);
        }
        _ => {
            let _ = idx;
            out.push(b)
        }
    }
}

/// A twisting body: many thin slices whose insets pulse as they rise, so the
/// shaft's edges wave instead of running straight — and each slice stays
/// inside the one below, so nothing overhangs into thin air.
fn twist_bands(d: &Design) -> Vec<Band> {
    let bx = ((d.bw - d.w) / 2.0).max(0.0);
    let by = ((d.bd - d.d) / 2.0).max(0.0);
    let (w, dd) = (d.w, d.d);
    let (zb, zt) = (d.z_podium, d.z_top);
    let h = (zt - zb).max(1);
    let n = 7;
    let mut out = Vec::new();
    let (mut ix, mut iy) = (0.0f32, 0.0f32);
    for k in 0..n {
        let f = (k + 1) as f32 / n as f32;
        let wave = 0.045 * ((k as f32) * 0.9).sin();
        // Monotone: the slice never comes back out past the one below it.
        ix = (ix.max(0.035 + 0.055 * f + wave)).min(w * 0.35);
        iy = (iy.max(0.030 + 0.050 * f - wave)).min(dd * 0.35);
        out.push(Band {
            x0: bx + ix,
            y0: by + iy,
            x1: bx + w - ix,
            y1: by + dd - iy,
            z0: zb + (h as f32 * (k as f32 / n as f32)) as i32,
            z1: if k + 1 == n { zt } else { zb + (h as f32 * ((k + 1) as f32 / n as f32)) as i32 },
            garden: false,
            crown_band: k + 1 == n,
        });
    }
    out
}

/// A tapering body: the plan narrows with height.
fn taper_bands(d: &Design) -> Vec<Band> {
    let bx = ((d.bw - d.w) / 2.0).max(0.0);
    let by = ((d.bd - d.d) / 2.0).max(0.0);
    let (w, dd) = (d.w, d.d);
    let (zb, zt) = (d.z_podium, d.z_top);
    let h = (zt - zb).max(1);
    let n = 7;
    let mut out = Vec::new();
    for k in 0..n {
        let f = k as f32 / (n - 1) as f32;
        let inset = 0.075 * f;
        out.push(Band {
            x0: bx + inset,
            y0: by + inset,
            x1: bx + w - inset,
            y1: by + dd - inset,
            z0: zb + (h as f32 * (k as f32 / n as f32)) as i32,
            z1: if k + 1 == n { zt } else { zb + (h as f32 * ((k + 1) as f32 / n as f32)) as i32 },
            garden: false,
            crown_band: k + 1 == n,
        });
    }
    out
}

/// Offset stacked boxes: the upper mass cantilevers over the lower one.
fn stacked_bands(d: &Design) -> Vec<Band> {
    let bx = ((d.bw - d.w) / 2.0).max(0.0);
    let by = ((d.bd - d.d) / 2.0).max(0.0);
    let (w, dd) = (d.w, d.d);
    let (zb, zt) = (d.z_podium, d.z_top);
    let h = (zt - zb).max(1);
    let mid = zb + (h as f32 * 0.58) as i32;
    // The upper mass is set back inside the lower one on two sides, so the
    // overhang reads as a stacked box without cantilevering into thin air.
    let sx = (w * 0.16).max(0.10);
    let sy = (dd * 0.12).max(0.08);
    vec![
        Band { x0: bx, y0: by, x1: bx + w, y1: by + dd, z0: zb, z1: mid, garden: false, crown_band: false },
        Band { x0: bx, y0: by + sy, x1: bx + w - sx, y1: by + dd, z0: mid, z1: zt, garden: false, crown_band: true },
    ]
}

/// Twin volumes with a sky bridge between them.
fn twin_bands(d: &Design) -> Vec<Band> {
    let bx = ((d.bw - d.w) / 2.0).max(0.0);
    let by = ((d.bd - d.d) / 2.0).max(0.0);
    let (w, dd) = (d.w, d.d);
    let (zb, zt) = (d.z_podium, d.z_top);
    let gap = (w * 0.13).clamp(0.12, 0.30) / 2.0;
    let mid = zb + ((zt - zb) as f32 * 0.62) as i32;
    let mut out = Vec::new();
    for (x0, x1) in [(0.0, w / 2.0 - gap), (w / 2.0 + gap, w)] {
        out.push(Band { x0: bx + x0, y0: by, x1: bx + x1, y1: by + dd, z0: zb, z1: zt, garden: false, crown_band: true });
    }
    // The bridge, between the two towers.
    out.push(Band {
        x0: bx + w / 2.0 - gap,
        y0: by + dd * 0.3,
        x1: bx + w / 2.0 + gap,
        y1: by + dd * 0.7,
        z0: mid,
        z1: mid + d.floor * 2,
        garden: false,
        crown_band: false,
    });
    out
}

/// Fill single-pixel pinholes inside a drawn silhouette.
fn close_pinpricks(sprite: &mut Sprite) {
    if sprite.w < 3 || sprite.h < 3 {
        return;
    }
    let (w, h) = (sprite.w, sprite.h);
    // "Drawn" is any non-zero pixel except the cast shadow, which the cache
    // drops (alpha < 0x80) but which reaches outside the lot: bridging it would
    // punch the shadow into the silhouette.
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

/// The tower's lot, paved: a forecourt in front of the entrance, with planters
/// and a path out to the two near edges (the sides the isometric camera sees).
/// The far half of the lot stays as it was — nothing is drawn there, so a slim
/// tower on a big plot no longer stands in a square of bare grass, and the
/// sprite does not have to carry the whole lot.
fn plaza(d: &Design, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let room = (d.bw - d.w).max(d.bd - d.d);
    if room < 0.16 {
        return;
    }
    let (bw, bd) = (d.bw, d.bd);
    let base = pal::CONCRETE_DRIVE;
    let joint = color::shade(base, 0.86);
    let edge = color::shade(base, 0.74);
    // The whole lot, so the tower always stands on its own pavement (a clipped
    // band leaves the tower's far corner hanging over nothing).
    let far = 0.03;
    let ground = box_top(0.03, far, bw - 0.03, bd - 0.03, 0, ox, oy);
    ground.paint(sprite, &mut |u, v, x, y| {
        let on_edge = u < 2 || v < 2;
        let c = if on_edge {
            edge
        } else if u % 7 == 0 || v % 7 == 0 {
            joint
        } else {
            color::shade(base, 0.94 + 0.10 * color::hash01(x / 3, y / 3))
        };
        Some((color::surf(c, light, FACE_TOP), 0))
    });
    // The path, from under the entrance out to both near edges.
    let path_c = color::shade(base, 1.08);
    let p1 = box_top(
        (bw * 0.5 + d.w * 0.05).min(bw - 0.28),
        (far + 0.05).max(bd - d.d * 0.5),
        bw - 0.04,
        bd - 0.04,
        0,
        ox,
        oy,
    );
    p1.fill(sprite, color::surf(path_c, light, FACE_TOP));
    let p2 = box_top(
        (bw * 0.5 + d.w * 0.05).min(bw - 0.28),
        (far + 0.05).max(bd - d.d * 0.5),
        bw - 0.04,
        bd - 0.04,
        0,
        ox,
        oy,
    );
    p2.fill(sprite, color::surf(path_c, light, FACE_TOP));

    // Planters along the near corners, in stone with a clipped green top.
    let planter = color::surf(color::shade(pal::STONE_GRANITE, 0.96), light, FACE_L);
    let leaf = color::surf(pal::FOLIAGE_GREEN, light, FACE_TOP);
    for (px, py) in [(0.08f32, far + 0.06), (bw - 0.26, far + 0.06), (bw - 0.26, bd - 0.30)] {
        let (lf, _rf) = box_faces(px, py, px + 0.17, py + 0.15, 0, 3, ox, oy);
        lf.fill(sprite, planter);
        let top = box_top(px, py, px + 0.17, py + 0.15, 3, ox, oy);
        top.fill(sprite, leaf);
    }
}

fn z_top_of(b: &Band) -> i32 {
    b.z1
}

/// Draw one band: its walls in the facade system, plus any body-specific
/// silhouette work (a chamfer, a drum, a wedge) and its roof deck.
fn draw_band(d: &Design, b: &Band, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let m = mats(d.material);
    // B76 r3: a band's walls are not a surface snow lies on; only the pieces
    // below that say so are tagged.
    sprite.on_surface(render::snow::NONE);
    if b.garden {
        // A planted floor: recessed, dark and green behind a rail — a sky garden
        // you can see the planting in, not a hole through the tower.
        let leaf = color::surf(pal::FOLIAGE_GREEN, light, FACE_L);
        let shade = color::surf(color::shade(pal::FOLIAGE_GREEN, 0.55), light, FACE_R);
        let (lf, rf) = box_faces(b.x0, b.y0, b.x1, b.y1, b.z0, b.z1, ox, oy);
        lf.paint(sprite, &mut |u, v, _x, _y| {
            let depth = (b.z1 - v).max(0) as u16;
            Some((if (u / 3 + v) % 3 == 0 { shade } else { leaf }, depth))
        });
        rf.paint(sprite, &mut |u, v, _x, _y| {
            let depth = (b.z1 - v).max(0) as u16;
            Some((if (u / 3 + v) % 3 == 0 { leaf } else { shade }, depth))
        });
        parts::corner_seam(sprite, light, b.x1, b.y1, b.z0, b.z1, m.metal, ox, oy);
        // The slab at its foot and the rail at its head. A planted terrace takes
        // a light, patchy cap rather than a slab; the rail is a ledge (B76 r3).
        let slab = box_top(b.x0, b.y0, b.x1, b.y1, b.z1, ox, oy);
        sprite.on_surface(render::snow::FOLIAGE);
        slab.fill(sprite, color::surf(pal::FOLIAGE_GREEN, light, FACE_TOP));
        sprite.on_surface(render::snow::LEDGE);
        let rail = color::surf(m.metal, light, FACE_L);
        let (glf, grf) = box_faces(b.x0, b.y0, b.x1, b.y1, b.z1, b.z1 + 1, ox, oy);
        glf.paint(sprite, &mut |u, _v, _x, _y| (u % 2 == 0).then_some((rail, (b.z1 + 1) as u16)));
        grf.paint(sprite, &mut |u, _v, _x, _y| (u % 2 == 0).then_some((rail, (b.z1 + 1) as u16)));
        sprite.on_surface(render::snow::NONE);
        return;
    }

    if matches!(d.body, Body::Drum) {
        drum_band(d, b, light, sprite, ox, oy);
        return;
    }
    if matches!(d.body, Body::Wedge) && b.crown_band {
        wedge_band(d, b, light, sprite, ox, oy);
        return;
    }

    let (lf, rf) = box_faces(b.x0, b.y0, b.x1, b.y1, b.z0, b.z1, ox, oy);
    let salt = (b.z0 as u32).wrapping_mul(7) ^ ((b.x0 * 10.0) as u32);
    paint::paint(d.facade, &lf, sprite, light, d, salt, FACE_L);
    paint::paint(d.facade, &rf, sprite, light, d, salt + 1, FACE_R);
    parts::corner_seam(sprite, light, b.x1, b.y1, b.z0, b.z1, m.metal, ox, oy);

    if matches!(d.body, Body::Chamfered | Body::Octagon) {
        chamfer(d, b, light, sprite, ox, oy);
    }
    if b.crown_band {
        deck(d, b, light, sprite, ox, oy);
    } else {
        // Every setback is a terrace you can see down onto: fill it, or a
        // stepped or cantilevered body reads as a hole between its two masses.
        // B76 r3: a terrace is a horizontal surface, so it holds the load.
        let top = box_top(b.x0, b.y0, b.x1, b.y1, b.z1, ox, oy);
        sprite.on_surface(render::snow::FLAT);
        top.fill(sprite, color::surf(pal::ROOF_GRAVEL, light, FACE_TOP));
    }
}

/// The roof deck: gravel, a parapet ring and a little plant, so a tower's top
/// reads as a finished roof rather than a cut-off box.
fn deck(d: &Design, b: &Band, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let m = mats(d.material);
    parts::roof_flat(sprite, light, b.x0, b.y0, b.x1, b.y1, b.z1, 2, m.metal, ox, oy, d.age);
}

/// A cut corner: two narrow planes where the sharp corner would be.
fn chamfer(d: &Design, b: &Band, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let m = mats(d.material);
    let wide = if matches!(d.body, Body::Octagon) { 0.10 } else { 0.06 };
    let (x1, y1) = (b.x1, b.y1);
    // The plane on the left wall's end…
    let a = Quad::new(
        project(x1 - wide, y1, b.z1 as f32, ox, oy),
        project(x1, y1, b.z1 as f32, ox, oy),
        project(x1, y1, b.z0 as f32, ox, oy),
        project(x1 - wide, y1, b.z0 as f32, ox, oy),
        b.z0,
        b.z1,
    );
    a.fill(sprite, color::surf(color::shade(m.wall, 1.10), light, FACE_L));
    // …and the plane on the right wall's end.
    let r = Quad::new(
        project(x1, y1 - wide, b.z1 as f32, ox, oy),
        project(x1, y1, b.z1 as f32, ox, oy),
        project(x1, y1, b.z0 as f32, ox, oy),
        project(x1, y1 - wide, b.z0 as f32, ox, oy),
        b.z0,
        b.z1,
    );
    r.fill(sprite, color::surf(color::shade(m.wall, 0.86), light, FACE_R));
}

/// A round drum: a real cylinder, shaded across its width.
fn drum_band(d: &Design, b: &Band, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let m = mats(d.material);
    let cx = (b.x0 + b.x1) / 2.0;
    let cy = (b.y0 + b.y1) / 2.0;
    let radius = ((b.x1 - b.x0).min(b.y1 - b.y0)) / 2.0;
    let (cxs, cys) = project(cx, cy, b.z1 as f32, ox, oy);
    let a = radius * 32.0 * std::f32::consts::SQRT_2;
    let bb = radius * 16.0 * std::f32::consts::SQRT_2;
    let height = (b.z1 - b.z0).max(1);
    let night = light.night.clamp(0.0, 1.0);
    let floor = d.floor.max(3);
    let x_lo = -a.ceil() as i32;
    let x_hi = a.ceil() as i32;
    for dx in x_lo..=x_hi {
        let t = dx as f32 / a.max(1.0);
        let dy = bb * (1.0 - (t * t).min(1.0)).sqrt();
        let x = cxs as i32 + dx;
        // Light from the left: the drum's left is bright, its right in shade.
        let shade = 0.74 + 0.34 * (-t * 0.5 + 0.5);
        let rim = cys + dy;
        // Vertical mullions every ~5 px of width.
        let mullion = dx.rem_euclid(6) == 0;
        for k in 0..height {
            let y = (rim + k as f32) as i32;
            let depth = (b.z1 - k).max(0) as u16;
            let spandrel = k % floor < 2;
            let c = if mullion || spandrel {
                color::shade(m.metal, shade)
            } else {
                // A lit pane now and then, chosen like every other window.
                let col = (dx + 1000) / 6;
                let fl = k / floor;
                let hash = color::hash_seed(d.seed, (col as u32 * 13 + fl as u32 * 37) ^ 5);
                if ((hash % 100) as f32) < 34.0 * night {
                    let warm = hash % 3 != 0;
                    let lit = if warm { color::WIN_WARM } else { color::WIN_COOL };
                    sprite.set_pixel(x, y, parts::emissive(color::shade(lit, 0.55 + 0.45 * night)), depth);
                    continue;
                }
                color::shade(color::mix(m.glass, color::WIN_OFF_NIGHT, night * night * 0.55), shade)
            };
            sprite.set_pixel(x, y, color::surf(c, light, 1.0), depth);
        }
    }
    if b.crown_band {
        // The drum's roof: an ellipse.
        let y_lo = -bb.ceil() as i32;
        let y_hi = bb.ceil() as i32;
        for dy in y_lo..=y_hi {
            let t = dy as f32 / bb.max(1.0);
            let half = a * (1.0 - (t * t).min(1.0)).sqrt();
            let y = cys as i32 + dy;
            let x0 = (cxs - half) as i32;
            let x1 = (cxs + half) as i32;
            sprite.on_surface(render::snow::FLAT);
            for x in x0..=x1 {
                sprite.set_pixel(x, y, color::surf(pal::ROOF_GRAVEL, light, FACE_TOP), b.z1 as u16);
            }
        }
    }
}

/// A wedge: the top band's left face leans back to a sloped top.
fn wedge_band(d: &Design, b: &Band, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let m = mats(d.material);
    let z_low = b.z0 + ((b.z1 - b.z0) as f32 * 0.45) as i32;
    // The tall wall at the back edge: with the roof sloping up towards it, it is
    // the silhouette under the high corner of the roof, so it must be drawn or
    // the slope reads as a slab hanging in the air.
    let back = Quad::new(
        project(b.x0, b.y0, b.z1 as f32, ox, oy),
        project(b.x1, b.y0, b.z1 as f32, ox, oy),
        project(b.x1, b.y0, b.z0 as f32, ox, oy),
        project(b.x0, b.y0, b.z0 as f32, ox, oy),
        b.z0,
        b.z1,
    );
    paint::paint(d.facade, &back, sprite, light, d, 9, FACE_L);
    // Left face: a trapezoid whose top edge rises from x0 to x1.
    let lf = Quad::new(
        project(b.x0, b.y1, z_low as f32, ox, oy),
        project(b.x1, b.y1, b.z1 as f32, ox, oy),
        project(b.x1, b.y1, b.z0 as f32, ox, oy),
        project(b.x0, b.y1, b.z0 as f32, ox, oy),
        b.z0,
        b.z1,
    );
    paint::paint(d.facade, &lf, sprite, light, d, 3, FACE_L);
    // Right face: a plain rectangle up to the high edge.
    let (_, rf) = box_faces(b.x0, b.y0, b.x1, b.y1, b.z0, b.z1, ox, oy);
    paint::paint(d.facade, &rf, sprite, light, d, 4, FACE_R);
    // The sloped roof plane.
    let top = Quad::new(
        project(b.x0, b.y0, z_low as f32, ox, oy),
        project(b.x1, b.y0, b.z1 as f32, ox, oy),
        project(b.x1, b.y1, b.z1 as f32, ox, oy),
        project(b.x0, b.y1, z_low as f32, ox, oy),
        z_low,
        b.z1,
    );
    // B76 r3: a leaning roof plane holds a medium load (it sheds the rest).
    sprite.on_surface(render::snow::slope_tag(
        (b.z1 - z_low) as f32,
        0.5 * (b.x1 - b.x0).max(b.y1 - b.y0) * 32.0,
    ));
    top.fill(sprite, color::surf(color::shade(m.panel, 0.96), light, FACE_TOP));
    sprite.on_surface(render::snow::LEDGE);
    // Edge line along the high edge, so the wedge reads at night too.
    let (ex0, ey0) = project(b.x1, b.y1, b.z1 as f32, ox, oy);
    let (ex1, ey1) = project(b.x1, b.y0, b.z1 as f32, ox, oy);
    geom::draw_line(
        sprite,
        ex0.round() as i32,
        ey0.round() as i32,
        ex1.round() as i32,
        ey1.round() as i32,
        color::surf(m.trim, light, FACE_TOP),
        b.z1 as u16,
    );
}

// -----------------------------------------------------------------------------
// Crowns
// -----------------------------------------------------------------------------

fn crown(d: &Design, b: &Band, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let m = mats(d.material);
    sprite.on_surface(render::snow::NONE);
    if matches!(d.body, Body::Drum) {
        // A round tower finishes with a mast from the middle of its roof: a box
        // crown on a circle leaves corners with nothing under them.
        let (cx, cy) = ((b.x0 + b.x1) / 2.0, (b.y0 + b.y1) / 2.0);
        parts::spire_needle(sprite, light, cx, cy, z_top_of(b), (d.z_crown - z_top_of(b)).max(4), ox, oy);
        if d.z_crown >= 140 {
            beacon(sprite, light, (cx, cy, d.z_crown), d.z_crown, ox, oy);
        }
        return;
    }
    let (mut x0, mut y0, mut x1, mut y1) = (b.x0, b.y0, b.x1, b.y1);
    if matches!(d.body, Body::Drum) {
        // A crown sits on the drum's roof, inside the circle: the ellipse's
        // silhouette falls away at the edge, so a full-width crown would hang
        // over the rim on both sides.
        let (cx, cy) = ((b.x0 + b.x1) / 2.0, (b.y0 + b.y1) / 2.0);
        let half = (b.x1 - b.x0).min(b.y1 - b.y0) * 0.31;
        x0 = cx - half;
        x1 = cx + half;
        y0 = cy - half;
        y1 = cy + half;
    }
    let z = b.z1;
    let w = x1 - x0;
    let dd = y1 - y0;
    let night = light.night.clamp(0.0, 1.0);
    // Round 2 item 3: a tower's crown lights in its own colour (warm white,
    // gold, red, green, blue, two-tone, silver) — and most towers are windows
    // only, so an unlit design shows bare metal after dark.
    let lit = if d.crown_lights {
        d.crown_lamp(light, false)
    } else {
        color::surf(color::shade(m.metal, 1.1), light, FACE_L)
    };
    let lit_accent = if d.crown_lights {
        d.crown_lamp(light, true)
    } else {
        color::surf(m.metal, light, FACE_L)
    };
    let metal = color::surf(m.metal, light, FACE_L);
    let mut apex = ((x0 + x1) / 2.0, (y0 + y1) / 2.0, z);

    match d.top {
        Top::FlatPlant => {
            parts::roof_flat(sprite, light, x0, y0, x1, y1, z, 2, m.metal, ox, oy, d.age);
            if d.crown_lights && night > 0.2 {
                // A lamp along the parapet coping, in the design's own colour.
                let (_, par) = box_faces(x0, y0, x1, y1, z + 2, z + 3, ox, oy);
                par.paint(sprite, &mut |u, _v, _x, _y| {
                    (u % 3 != 0).then_some((if u % 6 < 3 { lit } else { lit_accent }, (z + 3) as u16))
                });
            }
            // A stair head and two vent boxes, low enough to stay under the
            // design's crown height.
            let hgt = (d.z_crown - z - 2).clamp(2, 6);
            let stair = box_top(x0 + w * 0.55, y0 + dd * 0.45, x0 + w * 0.8, y0 + dd * 0.75, z + hgt, ox, oy);
            // B76 r3: rooftop equipment pokes out of the load with snow on top.
            sprite.on_surface(render::snow::EQUIP_TOP);
            stair.fill(sprite, color::surf(m.panel, light, FACE_TOP));
            sprite.on_surface(render::snow::EQUIP_SIDE);
            let (slf, srf) = box_faces(x0 + w * 0.3, y0 + dd * 0.3, x0 + w * 0.45, y0 + dd * 0.5, z + 2, z + hgt, ox, oy);
            slf.fill(sprite, color::surf(pal::CONCRETE_GREY, light, FACE_L));
            srf.fill(sprite, color::surf(color::shade(pal::CONCRETE_GREY, 0.82), light, FACE_R));
            sprite.on_surface(render::snow::NONE);
        }
        Top::WaterTank => {
            // Four legs, the tank, its conical lid: the old masonry idiom.
            let tx0 = x0 + w * 0.3;
            let tx1 = x1 - w * 0.3;
            let ty0 = y0 + dd * 0.3;
            let ty1 = y1 - dd * 0.3;
            for &(lx, ly) in &[(tx0, ty0), (tx1, ty0), (tx0, ty1), (tx1, ty1)] {
                let (px0, py0) = project(lx, ly, z as f32, ox, oy);
                let (px1, py1) = project(lx, ly, (z + 5) as f32, ox, oy);
                geom::draw_line(sprite, px0 as i32, py0 as i32, px1 as i32, py1 as i32, metal, (z + 5) as u16);
            }
            sprite.on_surface(render::snow::EQUIP_SIDE);
            let (tlf, trf) = box_faces(tx0, ty0, tx1, ty1, z + 5, z + 9, ox, oy);
            tlf.fill(sprite, color::surf(color::shade(m.wall, 0.9), light, FACE_L));
            trf.fill(sprite, color::surf(color::shade(m.wall, 0.75), light, FACE_R));
            sprite.on_surface(render::snow::SLOPE_GENTLE);
            let lid = [
                project(tx0, ty1, (z + 9) as f32, ox, oy),
                project(tx1, ty1, (z + 9) as f32, ox, oy),
                project((tx0 + tx1) / 2.0, (ty0 + ty1) / 2.0, (z + 11) as f32, ox, oy),
            ];
            fill_polygon(sprite, &lid, color::surf(m.metal, light, FACE_TOP), (z + 11) as u16);
            apex = ((tx0 + tx1) / 2.0, (ty0 + ty1) / 2.0, z + 11);
        }
        Top::DecoSpire => {
            let steps = [(0.12, 5), (0.24, 9)];
            let mut zz = z;
            for (inset, up) in steps {
                let (slf, srf) = box_faces(x0 + w * inset, y0 + dd * inset, x1 - w * inset, y1 - dd * inset, zz, zz + up, ox, oy);
                slf.fill(sprite, color::surf(color::shade(m.wall, 1.02), light, FACE_L));
                srf.fill(sprite, color::surf(color::shade(m.wall, 0.84), light, FACE_R));
                zz += up;
            }
            let spire = (d.z_crown - zz).max(3);
            parts::spire_needle(sprite, light, (x0 + x1) / 2.0, (y0 + y1) / 2.0, zz, spire, ox, oy);
            apex = ((x0 + x1) / 2.0, (y0 + y1) / 2.0, zz + spire);
        }
        Top::PyramidCap => {
            // Inset from the parapet and rising with the roof's own width, so it
            // reads as a cap rather than a lid laid over the building.
            let inset = 0.12;
            let (px0, py0, px1, py1) = (x0 + w * inset, y0 + dd * inset, x1 - w * inset, y1 - dd * inset);
            let rise = (d.z_crown - z).max(4);
            let (cx, cy) = ((px0 + px1) / 2.0, (py0 + py1) / 2.0);
            let apex_z = z + rise;
            let left = [
                project(px0, py1, z as f32, ox, oy),
                project(px1, py1, z as f32, ox, oy),
                project(cx, cy, apex_z as f32, ox, oy),
            ];
            sprite.on_surface(render::snow::slope_tag(rise as f32, 0.5 * w.min(dd) * 32.0));
            fill_polygon(sprite, &left, color::surf(color::shade(m.trim, 1.06), light, FACE_L), apex_z as u16);
            let right = [
                project(px1, py0, z as f32, ox, oy),
                project(px1, py1, z as f32, ox, oy),
                project(cx, cy, apex_z as f32, ox, oy),
            ];
            fill_polygon(sprite, &right, color::surf(color::shade(m.trim, 0.86), light, FACE_R), apex_z as u16);
            apex = (cx, cy, apex_z);
        }
        Top::NeedleMast => {
            let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
            let (slf, srf) = box_faces(x0 + w * 0.3, y0 + dd * 0.3, x1 - w * 0.3, y1 - dd * 0.3, z, z + 4, ox, oy);
            slf.fill(sprite, color::surf(m.metal, light, FACE_L));
            srf.fill(sprite, color::surf(color::shade(m.metal, 0.82), light, FACE_R));
            let mast = (d.z_crown - z - 4).max(4);
            parts::spire_needle(sprite, light, cx, cy, z + 4, mast, ox, oy);
            apex = (cx, cy, z + 4 + mast);
        }
        Top::Helipad => {
            let pad = box_top(x0 + w * 0.1, y0 + dd * 0.1, x1 - w * 0.1, y1 - dd * 0.1, z + 2, ox, oy);
            sprite.on_surface(render::snow::FLAT);
            pad.paint(sprite, &mut |u, v, _x, _y| {
                let border = u <= 1 || v <= 1;
                let is_h = (u >= 6 && u <= 10 && v >= 5 && v <= 11)
                    && (u == 6 || u == 10 || v == 7 || v == 8);
                let c = if border {
                    pal::METAL_WHITE
                } else if is_h {
                    color::GOLD_SIGN
                } else {
                    pal::ASPHALT
                };
                Some((color::surf(c, light, FACE_TOP), (z + 2) as u16))
            });
            // The parapet under it.
            sprite.on_surface(render::snow::EQUIP_SIDE);
            let (plf, prf) = box_faces(x0, y0, x1, y1, z, z + 2, ox, oy);
            plf.fill(sprite, color::surf(m.panel, light, FACE_L));
            prf.fill(sprite, color::surf(color::shade(m.panel, 0.8), light, FACE_R));
        }
        Top::Lantern => {
            let (clf, crf) = box_faces(x0 + w * 0.16, y0 + dd * 0.16, x1 - w * 0.16, y1 - dd * 0.16, z, z + 8, ox, oy);
            let glow = if night > 0.05 && d.crown_lights {
                lit_accent
            } else {
                color::surf(m.glass_hi, light, FACE_L)
            };
            clf.fill(sprite, color::surf(m.metal, light, FACE_L));
            crf.fill(sprite, color::surf(color::shade(m.metal, 0.82), light, FACE_R));
            // Lit glazing between the frame's mullions.
            let glow_l = if night > 0.05 { glow } else { color::surf(m.glass, light, FACE_L) };
            let glow_r = if night > 0.05 { glow } else { color::surf(m.glass, light, FACE_R) };
            clf.paint(sprite, &mut |u, v, _x, _y| {
                (u % 4 != 0 && v % 4 != 0).then_some((glow_l, (z + 8) as u16))
            });
            crf.paint(sprite, &mut |u, v, _x, _y| {
                (u % 4 != 0 && v % 4 != 0).then_some((glow_r, (z + 8) as u16))
            });
            apex = ((x0 + x1) / 2.0, (y0 + y1) / 2.0, z + 8);
        }
        Top::OpenLattice => {
            let inset = 0.14 * w.min(dd);
            let (ax0, ay0, ax1, ay1) = (x0 + inset, y0 + inset, x1 - inset, y1 - inset);
            // Four corner columns and three ring beams: nothing else.
            let top_z = d.z_crown;
            for &(lx, ly) in &[(ax0, ay1), (ax1, ay1), (ax1, ay0), (ax0, ay0)] {
                let (px0, py0) = project(lx, ly, z as f32, ox, oy);
                let (px1, py1) = project(lx, ly, top_z as f32, ox, oy);
                geom::draw_line(sprite, px0 as i32, py0 as i32, px1 as i32, py1 as i32, metal, top_z as u16);
            }
            for k in 1..=3 {
                let zz = z + (top_z - z) * k / 4;
                let (blf, brf) = box_faces(ax0, ay0, ax1, ay1, zz, zz + 1, ox, oy);
                blf.fill(sprite, metal);
                brf.fill(sprite, color::surf(color::shade(m.metal, 0.82), light, FACE_R));
            }
            apex = ((ax0 + ax1) / 2.0, (ay0 + ay1) / 2.0, d.z_crown);
        }
        Top::SlopedGlass => {
            let inset = 0.10;
            let (px0, py0, px1, py1) = (x0 + w * inset, y0 + dd * inset, x1 - w * inset, y1 - dd * inset);
            let (cx, cy) = ((px0 + px1) / 2.0, (py0 + py1) / 2.0);
            let apex_z = d.z_crown;
            let glass_c = color::surf(m.glass, light, FACE_L);
            sprite.on_surface(render::snow::slope_tag(
                (apex_z - z) as f32,
                0.5 * (px1 - px0).max(py1 - py0) * 32.0,
            ));
            let left = [
                project(px0, py1, z as f32, ox, oy),
                project(px1, py1, z as f32, ox, oy),
                project(cx, cy, apex_z as f32, ox, oy),
            ];
            fill_polygon(sprite, &left, glass_c, apex_z as u16);
            let right = [
                project(px1, py0, z as f32, ox, oy),
                project(px1, py1, z as f32, ox, oy),
                project(cx, cy, apex_z as f32, ox, oy),
            ];
            fill_polygon(sprite, &right, color::surf(m.glass, light, FACE_R), apex_z as u16);
            // A lit ridge line, so the cap reads as a crown at night.
            let (r0x, r0y) = project(px0, py1, z as f32, ox, oy);
            let (r1x, r1y) = project(cx, cy, apex_z as f32, ox, oy);
            let ridge = if night > 0.05 { lit } else { color::surf(m.trim, light, FACE_TOP) };
            geom::draw_line(sprite, r0x as i32, r0y as i32, r1x as i32, r1y as i32, ridge, apex_z as u16);
            apex = (cx, cy, apex_z);
        }
        Top::Ziggurat => {
            let mut zz = z;
            let step = ((d.z_crown - z) / 3).max(3);
            for (k, inset) in [0.10f32, 0.22, 0.34].iter().enumerate() {
                let (slf, srf) = box_faces(x0 + w * inset, y0 + dd * inset, x1 - w * inset, y1 - dd * inset, zz, zz + step, ox, oy);
                slf.fill(sprite, color::surf(color::shade(m.wall, 1.0 - 0.03 * k as f32), light, FACE_L));
                srf.fill(sprite, color::surf(color::shade(m.wall, 0.84 - 0.03 * k as f32), light, FACE_R));
                zz += step;
            }
            apex = ((x0 + x1) / 2.0, (y0 + y1) / 2.0, zz);
        }
        Top::RoofGarden => {
            parts::roof_flat(sprite, light, x0, y0, x1, y1, z, 2, m.metal, ox, oy, d.age);
            // The planting sits inside the parapet, not over it.
            let g = box_top(x0 + w * 0.14, y0 + dd * 0.14, x1 - w * 0.14, y1 - dd * 0.14, z + 1, ox, oy);
            sprite.on_surface(render::snow::FOLIAGE);
            g.fill(sprite, color::surf(pal::FOLIAGE_GREEN, light, FACE_TOP));
            // A few shrubs: two pixels of leaf over one of trunk.
            sprite.on_surface(render::snow::NONE);
            for k in 0..4 {
                let h = color::hash_seed(d.seed, 900 + k);
                let fx = x0 + w * (0.18 + 0.2 * (k as f32));
                let fy = y0 + dd * (0.3 + 0.15 * ((h % 3) as f32));
                let (px, py) = project(fx, fy, (z + 3) as f32, ox, oy);
                sprite.set_pixel(px as i32, py as i32, color::surf(pal::FOLIAGE_GREEN, light, FACE_L), (z + 3) as u16);
                sprite.set_pixel(px as i32, py as i32 + 1, color::surf(pal::FOLIAGE_GREEN, light, FACE_L), (z + 3) as u16);
                sprite.set_pixel(px as i32 + 1, py as i32, color::surf(pal::FOLIAGE_GREEN, light, FACE_R), (z + 3) as u16);
            }
        }
        Top::CoolingTowers => {
            // Two tapered stacks on the roof.
            for (k, fx) in [0.32f32, 0.68].iter().enumerate() {
                let cx = x0 + w * fx;
                let cy = y0 + dd * 0.5;
                let r = (w * 0.11).clamp(0.05, 0.16);
                let zz = z + 3 + k as i32;
                let base_l = project(cx - r, cy + r, zz as f32, ox, oy);
                let base_r = project(cx + r, cy + r, zz as f32, ox, oy);
                let top_l = project(cx - r * 0.7, cy + r * 0.7, (zz + 8) as f32, ox, oy);
                let top_r = project(cx + r * 0.7, cy + r * 0.7, (zz + 8) as f32, ox, oy);
                let quad = [base_l, base_r, top_r, top_l];
                fill_polygon(sprite, &quad, color::surf(color::shade(m.panel, 0.92), light, FACE_L), (zz + 8) as u16);
                let rim = [top_l, top_r, project(cx, cy, (zz + 8) as f32, ox, oy)];
                fill_polygon(sprite, &rim, color::surf(rgb(0xb0, 0xb4, 0xba), light, FACE_TOP), (zz + 8) as u16);
            }
            apex = ((x0 + x1) / 2.0, (y0 + y1) / 2.0, z + 11);
        }
        Top::CrownRing => {
            // A ring of lamps around the top edge (bright at night, metal by day).
            let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
            let a = (x1 - x0) / 2.0 * 32.0 * std::f32::consts::SQRT_2;
            let bb = (y1 - y0) / 2.0 * 16.0 * std::f32::consts::SQRT_2;
            let (cxs, cys) = project(cx, cy, (z + 3) as f32, ox, oy);
            let c = if night > 0.05 && d.crown_lights { lit } else { metal };
            // Four posts hold the ring above the parapet.
            for k in [0.0f32, 0.25, 0.5, 0.75] {
                let th = k * std::f32::consts::TAU;
                let x = cxs + a * th.cos();
                let y = cys + bb * th.sin();
                for dz in 0..=3 {
                    sprite.set_pixel(x as i32, y as i32 - dz, metal, (z + dz) as u16);
                }
            }
            for k in 0..32 {
                let th = k as f32 / 32.0 * std::f32::consts::TAU;
                let x = cxs + a * th.cos();
                let y = cys + bb * th.sin();
                // Two-tone crowns alternate their lamps round the ring.
                let cc = if night > 0.05 && d.crown_pal == 5 && k % 2 == 1 {
                    lit_accent
                } else {
                    c
                };
                sprite.set_pixel(x as i32, y as i32, cc, (z + 3) as u16);
            }
            apex = (cx, cy, z + 3);
        }
        Top::AntennaCluster => {
            for (k, (fx, fy, hh)) in
                [(0.3f32, 0.35f32, 0.72f32), (0.62, 0.45, 1.0), (0.45, 0.68, 0.55)].iter().enumerate()
            {
                let px = x0 + w * fx;
                let py = y0 + dd * fy;
                let h = (((d.z_crown - z) as f32) * hh) as i32;
                parts::spire_needle(sprite, light, px, py, z, h, ox, oy);
                if k == 1 {
                    apex = (px, py, z + h);
                }
            }
        }
        Top::LightBox => {
            // A lit sign box with invented glyphs (B73's signs, D75: no words).
            let bx0 = x0 + w * 0.14;
            let bx1 = x1 - w * 0.14;
            let by0 = y0 + dd * 0.14;
            let by1 = y1 - dd * 0.14;
            let (blf, brf) = box_faces(bx0, by0, bx1, by1, z, z + 10, ox, oy);
            blf.fill(sprite, color::surf(pal::METAL_STEEL_DARK, light, FACE_L));
            brf.fill(sprite, color::surf(color::shade(pal::METAL_STEEL_DARK, 0.84), light, FACE_R));
            let panel = if night > 0.1 {
                parts::emissive(color::shade(rgb(0xff, 0xf0, 0xdc), 0.55 + 0.45 * night))
            } else {
                color::surf(rgb(0x3a, 0x3e, 0x46), light, FACE_L)
            };
            let glyph = if night > 0.1 { lit } else { color::surf(m.trim, light, FACE_L) };
            for (face, salt) in [(&blf, 0u32), (&brf, 7u32)] {
                face.paint(sprite, &mut |u, v, _x, _y| {
                    if u < 2 || v < 2 || v > 8 {
                        return None;
                    }
                    let block = (u / 4 + salt as i32) % 3 != 0;
                    Some((if block { glyph } else { panel }, (z + 10) as u16))
                });
            }
            apex = ((bx0 + bx1) / 2.0, (by0 + by1) / 2.0, z + 10);
        }
    }

    sprite.on_surface(render::snow::NONE);
    // Aviation lights: the tallest towers carry one, and it burns at night.
    if d.z_crown >= 140 {
        beacon(sprite, light, apex, d.z_crown.max(apex.2), ox, oy);
    }
}

fn beacon(sprite: &mut Sprite, light: &Light, apex: (f32, f32, i32), z: i32, ox: f32, oy: f32) {
    let (px, py) = project(apex.0, apex.1, z as f32, ox, oy);
    let (px, py) = (px as i32, py as i32);
    sprite.set_pixel(px, py, crate::color::BEACON_RED, z as u16);
    if light.is_night {
        for (dx, dy) in [(-1, 0), (1, 0), (0, -1)] {
            sprite.set_pixel(px + dx, py + dy, color::shade(crate::color::BEACON_RED, 0.6), z as u16);
        }
    }
}
