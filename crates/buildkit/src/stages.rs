//! Construction stages rendering (B33d): Excavation, Foundation, Framing with tower crane, Cladding.

use render::palette::rgb;
use crate::color::{self, pal, FACE_L, FACE_R, FACE_TOP};
use crate::geom::{self, box_faces, box_top, project};
use crate::parts;
use crate::types::{BuildingSpec, Light, Sprite};

pub fn draw_construction_stage(
    spec: &BuildingSpec,
    light: &Light,
    sprite: &mut Sprite,
    ox: f32,
    oy: f32,
    target_height: f32,
) {
    let (w, d) = (
        spec.footprint.0.max(1) as f32,
        spec.footprint.1.max(1) as f32,
    );

    match spec.stage {
        crate::types::ConstructionStage::Excavation => {
            draw_excavation(w, d, light, sprite, ox, oy);
        }
        crate::types::ConstructionStage::Foundation => {
            draw_foundation(w, d, light, sprite, ox, oy);
        }
        crate::types::ConstructionStage::Framing => {
            draw_framing(w, d, target_height * 0.45, light, sprite, ox, oy);
        }
        crate::types::ConstructionStage::Cladding => {
            draw_cladding(spec, w, d, target_height * 0.85, light, sprite, ox, oy);
        }
        crate::types::ConstructionStage::Finished => {
            // Unreachable here: handled by catalogue dispatcher
        }
    }
}

/// 1. Excavation Pit: earth banks, shoring walls, yellow excavator, safety fence.
fn draw_excavation(w: f32, d: f32, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let pit_depth = 6;
    parts::building_shadow(sprite, 0.0, 0.0, w, d, 4, ox, oy);

    // Ground collar
    let collar = box_top(0.0, 0.0, w, d, 1, ox, oy);
    collar.fill(sprite, color::surf(pal::CONCRETE_GREY, light, FACE_TOP));

    // Excavation pit shoring walls
    let (lf, rf) = box_faces(0.12, 0.12, w - 0.12, d - 0.12, -pit_depth, 1, ox, oy);
    let steel_sheet = pal::METAL_STEEL_DARK;
    lf.paint(sprite, &mut |u, _v, _x, _y| {
        let is_rib = (u % 3) == 0;
        let c = if is_rib { steel_sheet } else { color::shade(steel_sheet, 1.2) };
        Some((color::surf(c, light, FACE_L), 1))
    });
    rf.paint(sprite, &mut |u, _v, _x, _y| {
        let is_rib = (u % 3) == 0;
        let c = if is_rib { color::shade(steel_sheet, 0.85) } else { color::shade(steel_sheet, 1.05) };
        Some((color::surf(c, light, FACE_R), 1))
    });

    // Soil pit floor
    let pit_floor = box_top(0.12, 0.12, w - 0.12, d - 0.12, -pit_depth, ox, oy);
    pit_floor.paint(sprite, &mut |_u, _v, x, y| {
        let grain = color::jcell(x, y, 3, 0.08);
        Some((color::surf(color::shade(pal::SOIL_GARDEN, grain), light, FACE_TOP), 1))
    });

    // Yellow hydraulic excavator crawler in the pit
    let ex_x = w * 0.55;
    let ex_y = d * 0.50;
    let yellow_c = rgb(0xf4, 0xc2, 0x24);
    // Tracks
    let (t_lf, t_rf) = box_faces(ex_x - 0.25, ex_y - 0.15, ex_x + 0.25, ex_y + 0.15, -pit_depth, -pit_depth + 4, ox, oy);
    t_lf.fill(sprite, color::surf(pal::METAL_STEEL_DARK, light, FACE_L));
    t_rf.fill(sprite, color::surf(pal::METAL_STEEL_DARK, light, FACE_R));
    // Cab & body
    let (c_lf, c_rf) = box_faces(ex_x - 0.18, ex_y - 0.12, ex_x + 0.18, ex_y + 0.12, -pit_depth + 4, -pit_depth + 10, ox, oy);
    c_lf.fill(sprite, color::surf(yellow_c, light, FACE_L));
    c_rf.fill(sprite, color::surf(yellow_c, light, FACE_R));
    let c_top = box_top(ex_x - 0.18, ex_y - 0.12, ex_x + 0.18, ex_y + 0.12, -pit_depth + 10, ox, oy);
    c_top.fill(sprite, color::surf(yellow_c, light, FACE_TOP));

    // Excavator boom & arm
    let (p0_x, p0_y) = project(ex_x + 0.15, ex_y, (-pit_depth + 8) as f32, ox, oy);
    let (p1_x, p1_y) = project(ex_x + 0.35, ex_y + 0.20, (-pit_depth + 14) as f32, ox, oy);
    let (p2_x, p2_y) = project(ex_x + 0.45, ex_y + 0.35, (-pit_depth + 2) as f32, ox, oy);
    geom::draw_line(sprite, p0_x.round() as i32, p0_y.round() as i32, p1_x.round() as i32, p1_y.round() as i32, yellow_c, 8);
    geom::draw_line(sprite, p1_x.round() as i32, p1_y.round() as i32, p2_x.round() as i32, p2_y.round() as i32, yellow_c, 8);

    // Perimeter orange safety fence
    for i in 0..6 {
        let px = i as f32 * w / 5.0;
        let (p0_x, p0_y) = project(px, d, 1.0, ox, oy);
        let (p1_x, p1_y) = project(px, d, 7.0, ox, oy);
        geom::draw_line(sprite, p0_x.round() as i32, p0_y.round() as i32, p1_x.round() as i32, p1_y.round() as i32, rgb(0xea, 0x5a, 0x24), 7);
    }
}

/// 2. Foundation: poured concrete footings, rebar grid, plywood formwork.
fn draw_foundation(w: f32, d: f32, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let f_h = 4;
    parts::building_shadow(sprite, 0.0, 0.0, w, d, f_h, ox, oy);

    // Concrete foundation perimeter footings
    let (lf, rf) = box_faces(0.0, 0.0, w, d, 0, f_h, ox, oy);
    lf.fill(sprite, color::surf(pal::CONCRETE_GREY, light, FACE_L));
    rf.fill(sprite, color::surf(pal::CONCRETE_GREY, light, FACE_R));
    parts::corner_seam(sprite, light, w, d, 0, f_h, pal::CONCRETE_LIGHT, ox, oy);

    // Slab with steel rebar grid
    let top = box_top(0.0, 0.0, w, d, f_h, ox, oy);
    top.paint(sprite, &mut |u, v, _x, _y| {
        let is_rebar = (u % 4 == 0) || (v % 4 == 0);
        let c = if is_rebar { pal::METAL_STEEL_DARK } else { pal::CONCRETE_LIGHT };
        Some((color::surf(c, light, FACE_TOP), f_h as u16))
    });

    // Wooden formwork panels around edges
    let form_c = rgb(0xb8, 0x86, 0x4e); // raw timber plywood
    for i in 0..4 {
        let px = 0.1 + i as f32 * (w - 0.2) / 3.0;
        let (p0_x, p0_y) = project(px, d, 0.0, ox, oy);
        let (p1_x, p1_y) = project(px, d, 6.0, ox, oy);
        geom::draw_line(sprite, p0_x.round() as i32, p0_y.round() as i32, p1_x.round() as i32, p1_y.round() as i32, form_c, 6);
    }
}

/// 3. Framing: structural steel skeleton / concrete frame rising with yellow tower crane.
fn draw_framing(w: f32, d: f32, frame_h: f32, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let z_top = frame_h.round() as i32;
    parts::building_shadow(sprite, 0.0, 0.0, w, d, z_top, ox, oy);

    // Concrete core
    let core_w = w * 0.45;
    let core_d = d * 0.45;
    let (c_lf, c_rf) = box_faces(w * 0.27, d * 0.27, w * 0.27 + core_w, d * 0.27 + core_d, 0, z_top + 4, ox, oy);
    c_lf.fill(sprite, color::surf(pal::CONCRETE_GREY, light, FACE_L));
    c_rf.fill(sprite, color::surf(color::shade(pal::CONCRETE_GREY, 0.85), light, FACE_R));
    let c_top = box_top(w * 0.27, d * 0.27, w * 0.27 + core_w, d * 0.27 + core_d, z_top + 4, ox, oy);
    c_top.fill(sprite, color::surf(pal::CONCRETE_LIGHT, light, FACE_TOP));

    // Floor slabs every 10 pixels
    let num_slabs = (z_top / 10).max(1);
    for s in 1..=num_slabs {
        let slab_z = s * 10;
        let slab = box_top(0.0, 0.0, w, d, slab_z, ox, oy);
        slab.fill(sprite, color::surf(pal::CONCRETE_LIGHT, light, FACE_TOP));

        // Perimeter orange safety netting
        let (n_lf, n_rf) = box_faces(0.0, 0.0, w, d, slab_z - 3, slab_z, ox, oy);
        n_lf.fill(sprite, color::surf(rgb(0xea, 0x60, 0x24), light, FACE_L));
        n_rf.fill(sprite, color::surf(rgb(0xd8, 0x54, 0x1e), light, FACE_R));
    }

    // Steel column grid on perimeter
    let steel_c = pal::METAL_STEEL_DARK;
    let num_cols = 4;
    for i in 0..num_cols {
        let px = i as f32 * w / (num_cols - 1) as f32;
        let (p0_x, p0_y) = project(px, d, 0.0, ox, oy);
        let (p1_x, p1_y) = project(px, d, z_top as f32, ox, oy);
        geom::draw_line(sprite, p0_x.round() as i32, p0_y.round() as i32, p1_x.round() as i32, p1_y.round() as i32, steel_c, z_top as u16);

        let py = i as f32 * d / (num_cols - 1) as f32;
        let (p0_x, p0_y) = project(w, py, 0.0, ox, oy);
        let (p1_x, p1_y) = project(w, py, z_top as f32, ox, oy);
        geom::draw_line(sprite, p0_x.round() as i32, p0_y.round() as i32, p1_x.round() as i32, p1_y.round() as i32, steel_c, z_top as u16);
    }

    // Yellow Tower Crane mounted in core
    draw_tower_crane(w * 0.5, d * 0.5, z_top + 4, light, sprite, ox, oy);
}

/// 4. Cladding: lower 50% finished facade, upper 50% exposed frame with tower crane.
fn draw_cladding(spec: &BuildingSpec, w: f32, d: f32, total_h: f32, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let z_total = total_h.round() as i32;
    let z_clad = (z_total * 55 / 100).max(12);
    parts::building_shadow(sprite, 0.0, 0.0, w, d, z_total, ox, oy);

    // Clad lower portion: finished curtain wall or stone facade
    let (lf, rf) = box_faces(0.0, 0.0, w, d, 0, z_clad, ox, oy);
    parts::paint_glass_curtain(&lf, sprite, light, 8, 7, FACE_L, spec.seed + 7);
    parts::paint_glass_curtain(&rf, sprite, light, 8, 7, FACE_R, spec.seed);
    parts::corner_seam(sprite, light, w, d, 0, z_clad, pal::METAL_STEEL_DARK, ox, oy);

    // Upper framing portion
    draw_framing(w, d, (z_total - z_clad) as f32, light, sprite, ox, oy - z_clad as f32);
}

/// Yellow Tower Crane: vertical lattice mast, horizontal slewing jib, counterweight, operator cab, trolley, hook line.
pub fn draw_tower_crane(cx: f32, cy: f32, z_base: i32, _light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let crane_c = rgb(0xf4, 0xc4, 0x24); // bright construction yellow
    let cable_c = rgb(0x28, 0x28, 0x28);
    let mast_h = 24;
    let z_jib = z_base + mast_h;

    // Vertical lattice mast
    let (m0_x, m0_y) = project(cx, cy, z_base as f32, ox, oy);
    let (m1_x, m1_y) = project(cx, cy, z_jib as f32, ox, oy);
    let mx0 = m0_x.round() as i32;
    let my0 = m0_y.round() as i32;
    let mx1 = m1_x.round() as i32;
    let my1 = m1_y.round() as i32;
    geom::draw_line(sprite, mx0, my0, mx1, my1, crane_c, z_jib as u16);
    geom::draw_line(sprite, mx0 + 1, my0, mx1 + 1, my1, crane_c, z_jib as u16);

    // Horizontal slewing jib (extends front-left by 26px, counter-jib extends right by 10px)
    let jib_len = 28;
    let counter_len = 10;
    let j_left = mx1 - jib_len;
    let j_right = mx1 + counter_len;
    for dx in j_left..=j_right {
        sprite.set_pixel(dx, my1, crane_c, z_jib as u16);
    }
    // A-frame tower peak above slewing ring
    let peak_y = my1 - 7;
    geom::draw_line(sprite, mx1, my1, mx1, peak_y, crane_c, (z_jib + 7) as u16);
    // Tie cables from peak to jib tip and counterweight
    geom::draw_line(sprite, mx1, peak_y, j_left, my1, cable_c, z_jib as u16);
    geom::draw_line(sprite, mx1, peak_y, j_right, my1, cable_c, z_jib as u16);

    // Counterweight concrete blocks
    for dy in 0..3 {
        for dx in 0..4 {
            sprite.set_pixel(j_right - dx, my1 + dy, pal::CONCRETE_GREY, z_jib as u16);
        }
    }

    // Trolley & suspended hoist cable with hook
    let trolley_x = mx1 - 16;
    sprite.set_pixel(trolley_x, my1, rgb(0x18, 0x18, 0x18), z_jib as u16);
    for dy in 1..=12 {
        sprite.set_pixel(trolley_x, my1 + dy, cable_c, (z_jib - dy) as u16);
    }
    // Crane hook lifting load
    sprite.set_pixel(trolley_x, my1 + 13, rgb(0xdc, 0x24, 0x24), (z_jib - 13) as u16);
}
