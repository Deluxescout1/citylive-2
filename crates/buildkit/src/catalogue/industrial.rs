//! Industrial: Brick Factory with Smokestack, Logistics Warehouse, Silos, Power Substation.

use render::palette::rgb;
use crate::color::{self, pal, FACE_L, FACE_R, FACE_TOP};
use crate::geom::{self, box_faces, box_top, project};
use crate::parts::{self, WinOpts};
use crate::types::{BuildingSpec, Light, Sprite};

pub fn draw_brick_factory(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let (w, d) = (2.00f32, 1.80f32);
    let v = spec.variant % 4;
    let age = spec.age;

    // Structural variation across 4 variants
    let (z_wall, stack_h, twin_stacks, brick_c) = match v {
        0 => (28, 55, false, pal::BRICK_DARK_CLINKER),
        1 => (40, 65, true, pal::BRICK_RED), // 3 storeys (+1 storey!) + twin stacks
        2 => (28, 48, false, pal::STONE_DARK_SLATE), // dark slate factory + cyclone collector
        _ => (26, 50, false, pal::BRICK_BUFF), // steel frame + buff brick infill
    };

    parts::building_shadow(sprite, 0.0, 0.0, w, d, stack_h, ox, oy);

    let (lf, rf) = box_faces(0.0, 0.0, w, d, 0, z_wall, ox, oy);
    parts::paint_brick(&lf, sprite, light, brick_c, pal::BRICK_MORTAR, FACE_L, age);
    parts::paint_brick(&rf, sprite, light, brick_c, pal::BRICK_MORTAR, FACE_R, age);
    parts::corner_seam(sprite, light, w, d, 0, z_wall, pal::STONE_GRANITE, ox, oy);

    // Roof deck covering top of main walls
    let base_roof = box_top(0.0, 0.0, w, d, z_wall, ox, oy);
    base_roof.fill(sprite, color::surf(pal::ROOF_ASPHALT_DARK, light, FACE_TOP));

    // Sawtooth industrial roof with northlight glass
    let teeth = 3;
    let tooth_w = w / teeth as f32;
    for i in 0..teeth {
        let x_start = i as f32 * tooth_w;
        let x_end = (i + 1) as f32 * tooth_w;
        let (lf_tooth, rf_tooth) = box_faces(x_start, 0.0, x_end, d, z_wall, z_wall + 9, ox, oy);
        lf_tooth.fill(sprite, color::surf(pal::GLASS_BLUE_SKY, light, FACE_L));
        rf_tooth.fill(sprite, color::surf(pal::ROOF_ASPHALT_DARK, light, FACE_R));
    }

    // Cylindrical brick smokestack chimney
    let (sx0, sy0, sw, sd) = (w - 0.38, 0.15, 0.30, 0.30);
    let (s_lf, s_rf) = box_faces(sx0, sy0, sx0 + sw, sy0 + sd, 0, stack_h, ox, oy);
    parts::paint_brick(&s_lf, sprite, light, brick_c, pal::BRICK_MORTAR, FACE_L, age);
    parts::paint_brick(&s_rf, sprite, light, brick_c, pal::BRICK_MORTAR, FACE_R, age);
    let s_top = box_top(sx0, sy0, sx0 + sw, sy0 + sd, stack_h, ox, oy);
    s_top.fill(sprite, color::surf(pal::METAL_STEEL_DARK, light, FACE_TOP));

    if twin_stacks {
        let (sx1, sy1) = (w - 0.80, 0.15);
        let (s2_lf, s2_rf) = box_faces(sx1, sy1, sx1 + sw, sy1 + sd, 0, stack_h, ox, oy);
        parts::paint_brick(&s2_lf, sprite, light, brick_c, pal::BRICK_MORTAR, FACE_L, age);
        parts::paint_brick(&s2_rf, sprite, light, brick_c, pal::BRICK_MORTAR, FACE_R, age);
    }

    // Multi-pane industrial windows on both faces
    let len = rf.top_len();
    let wo = WinOpts {
        w: 8,
        h: 9,
        frame: pal::METAL_STEEL_DARK,
        sill: true,
        lintel: true,
        shutters: false,
        shutter_c: pal::SHUTTER_BLACK,
        seed: spec.seed,
        lit_p: 0.50,
    };
    parts::draw_window(&rf, sprite, light, 6, 4, wo, z_wall);
    parts::draw_window(&rf, sprite, light, 18, 4, wo, z_wall);

    let len_l = lf.top_len();
    parts::draw_window(&lf, sprite, light, 6, 4, wo, z_wall);
    parts::draw_window(&lf, sprite, light, len_l - 14, 4, wo, z_wall);

    // Industrial freight loading dock door
    let dq = rf.sub_px(len - 16, z_wall - 13, len - 2, z_wall, rf.rows() as f32);
    dq.paint(sprite, &mut |_u, v, _x, _y| {
        let is_rib = (v % 2) == 0;
        let c = if is_rib { pal::METAL_STEEL_DARK } else { color::shade(pal::METAL_STEEL_DARK, 1.25) };
        Some((color::surf(c, light, FACE_R), (z_wall - v) as u16))
    });
}

pub fn draw_warehouse(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let (w, d) = (2.90f32, 1.90f32);
    let v = spec.variant % 4;
    let age = spec.age;

    // Structural variation across 4 variants
    let (z_wall, metal_c, stripe_c, num_bays) = match v {
        0 => (26, pal::METAL_WHITE, rgb(0x28, 0x6e, 0xba), 3),
        1 => (34, pal::CONCRETE_LIGHT, rgb(0xdc, 0x6e, 0x24), 4), // 2 storeys (+higher clearance) + 4 bays
        2 => (24, pal::METAL_STEEL_DARK, rgb(0x38, 0x9e, 0x42), 3), // dark ribbed siding + green stripe
        _ => (28, pal::STONE_LIMESTONE, rgb(0xdc, 0x32, 0x32), 3), // concrete tilt-up + glass office
    };

    parts::building_shadow(sprite, 0.0, 0.0, w, d, z_wall, ox, oy);

    let (lf, rf) = box_faces(0.0, 0.0, w, d, 0, z_wall, ox, oy);
    parts::paint_corrugated(&lf, sprite, light, metal_c, FACE_L, age);
    parts::paint_corrugated(&rf, sprite, light, metal_c, FACE_R, age);
    parts::corner_seam(sprite, light, w, d, 0, z_wall, pal::METAL_STEEL_DARK, ox, oy);

    // Accent company stripe
    let len = rf.top_len();
    let sq = rf.sub_px(0, 3, len, 6, rf.rows() as f32);
    sq.fill(sprite, color::surf(stripe_c, light, FACE_R));
    let len_l = lf.top_len();
    let sq_l = lf.sub_px(0, 3, len_l, 6, lf.rows() as f32);
    sq_l.fill(sprite, color::surf(stripe_c, light, FACE_L));

    // Flat roof with exhaust vents
    parts::roof_flat(sprite, light, 0.0, 0.0, w, d, z_wall, 3, pal::METAL_STEEL_DARK, ox, oy, age);
    parts::rooftop_hvac(sprite, light, 0.6, 0.6, z_wall, ox, oy);
    parts::rooftop_hvac(sprite, light, 1.8, 0.6, z_wall, ox, oy);

    // Roll-up truck loading bays
    for bay in 0..num_bays {
        let u = 8 + bay * (len - 12) / num_bays;
        let bq = rf.sub_px(u, z_wall - 14, u + 12, z_wall, rf.rows() as f32);
        bq.paint(sprite, &mut |_u, v, _x, _y| {
            let is_rib = (v % 2) == 0;
            let c = if is_rib { pal::METAL_STEEL_DARK } else { pal::CONCRETE_LIGHT };
            Some((color::surf(c, light, FACE_R), (z_wall - v) as u16))
        });
    }
}

pub fn draw_silos(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let (w, d) = (2.00f32, 1.80f32);
    let v = spec.variant % 4;
    let silo_h = match v {
        0 => 62,
        1 => 74, // tall grain elevator (+12px!)
        2 => 54, // cement silos
        _ => 58,
    };
    let _age = spec.age;

    parts::building_shadow(sprite, 0.0, 0.0, w, d, silo_h, ox, oy);

    // Solid concrete ground base slab spanning the footprint
    let (s_lf, s_rf) = box_faces(0.0, 0.0, w, d, 0, 3, ox, oy);
    s_lf.fill(sprite, color::surf(pal::CONCRETE_GREY, light, FACE_L));
    s_rf.fill(sprite, color::surf(pal::CONCRETE_GREY, light, FACE_R));
    let slab = box_top(0.0, 0.0, w, d, 3, ox, oy);
    slab.fill(sprite, color::surf(pal::CONCRETE_LIGHT, light, FACE_TOP));

    // 4 Cylindrical concrete silos drawn in strict back-to-front depth order
    let silo_r = 0.40f32;
    let centers = [
        (0.50f32, 0.45f32), // Back left (depth = 0.95)
        (1.40f32, 0.45f32), // Back right (depth = 1.85)
        (0.50f32, 1.25f32), // Front left (depth = 1.75)
        (1.40f32, 1.25f32), // Front right (depth = 2.65)
    ];

    for &(cx, cy) in &centers {
        let (lf, rf) = box_faces(cx - silo_r, cy - silo_r, cx + silo_r, cy + silo_r, 3, silo_h, ox, oy);
        // Cylindrical curved shading on both faces
        rf.paint(sprite, &mut |u, _v, x, y| {
            let len = rf.top_len();
            let t = u as f32 / len as f32;
            let curve = (1.0 - (t - 0.5).abs() * 0.4).clamp(0.65, 1.15);
            let grain = color::jcell(x, y, 3, 0.02);
            let c = color::shade(pal::CONCRETE_LIGHT, curve * grain);
            Some((color::surf(c, light, FACE_R), silo_h as u16))
        });
        lf.paint(sprite, &mut |u, _v, x, y| {
            let len = lf.top_len();
            let t = u as f32 / len as f32;
            let curve = (1.0 - (t - 0.5).abs() * 0.4).clamp(0.65, 1.15);
            let grain = color::jcell(x, y, 3, 0.02);
            let c = color::shade(pal::CONCRETE_GREY, curve * grain);
            Some((color::surf(c, light, FACE_L), silo_h as u16))
        });
        parts::corner_seam(sprite, light, cx + silo_r, cy + silo_r, 3, silo_h, pal::CONCRETE_LIGHT, ox, oy);

        // Circular dome top
        let top = box_top(cx - silo_r, cy - silo_r, cx + silo_r, cy + silo_r, silo_h, ox, oy);
        top.fill(sprite, color::surf(pal::CONCRETE_LIGHT, light, FACE_TOP));
    }

    // Steel conveyor gallery bridge connecting the silo tops
    let (b_lf, b_rf) = box_faces(0.2, 0.7, w - 0.2, 1.0, silo_h, silo_h + 8, ox, oy);
    b_lf.fill(sprite, color::surf(pal::METAL_STEEL_DARK, light, FACE_L));
    b_rf.fill(sprite, color::surf(color::shade(pal::METAL_STEEL_DARK, 1.2), light, FACE_R));
    let b_top = box_top(0.2, 0.7, w - 0.2, 1.0, silo_h + 8, ox, oy);
    b_top.fill(sprite, color::surf(pal::CONCRETE_LIGHT, light, FACE_TOP));
}

pub fn draw_power_substation(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let (w, d) = (2.00f32, 1.80f32);
    let v = spec.variant % 4;
    let z_wall = 18;
    let age = spec.age;

    parts::building_shadow(sprite, 0.0, 0.0, w, d, z_wall, ox, oy);

    // Solid gravel yard & concrete curb spanning the full lot footprint (so it never floats!)
    let (s_lf, s_rf) = box_faces(0.0, 0.0, w, d, 0, 2, ox, oy);
    s_lf.fill(sprite, color::surf(pal::CONCRETE_GREY, light, FACE_L));
    s_rf.fill(sprite, color::surf(pal::CONCRETE_GREY, light, FACE_R));
    let yard = box_top(0.0, 0.0, w, d, 2, ox, oy);
    yard.paint(sprite, &mut |_u, _v, x, y| {
        let grain = color::jcell(x, y, 2, 0.05);
        Some((color::surf(color::shade(pal::ROOF_GRAVEL, grain), light, FACE_TOP), 2))
    });

    // Brick control house on rear half sitting on ground slab
    let (lf, rf) = box_faces(0.1, 0.1, 1.0, 0.9, 2, 2 + z_wall, ox, oy);
    parts::paint_brick(&lf, sprite, light, pal::BRICK_RED, pal::BRICK_MORTAR, FACE_L, age);
    parts::paint_brick(&rf, sprite, light, pal::BRICK_RED, pal::BRICK_MORTAR, FACE_R, age);
    parts::corner_seam(sprite, light, 1.0, 0.9, 2, 2 + z_wall, pal::STONE_LIMESTONE, ox, oy);
    parts::roof_flat(sprite, light, 0.1, 0.1, 1.0, 0.9, 2 + z_wall, 3, pal::CONCRETE_GREY, ox, oy, age);

    // High clerestory windows on control house
    let wo = WinOpts {
        w: 5,
        h: 6,
        frame: pal::METAL_STEEL_DARK,
        sill: true,
        lintel: true,
        shutters: false,
        shutter_c: pal::SHUTTER_BLACK,
        seed: spec.seed,
        lit_p: 0.80,
    };
    parts::draw_window(&rf, sprite, light, 4, 3, wo, 2 + z_wall);
    parts::draw_door(&rf, sprite, light, 14, z_wall - 10, 6, 10, pal::METAL_STEEL_DARK, pal::TRIM_WHITE, 2);

    // Exterior electrical transformers in front yard sitting on concrete pads
    let num_transformers = match v {
        0 => 2,
        1 => 3, // large substation (+1 transformer)
        2 => 2,
        _ => 1,
    };

    for i in 0..num_transformers {
        let tx = 1.15f32;
        let ty = 0.25f32 + i as f32 * 0.45f32;
        // Concrete transformer containment pad
        let (p_lf, p_rf) = box_faces(tx - 0.05, ty - 0.05, tx + 0.45, ty + 0.40, 2, 4, ox, oy);
        p_lf.fill(sprite, color::surf(pal::CONCRETE_GREY, light, FACE_L));
        p_rf.fill(sprite, color::surf(pal::CONCRETE_GREY, light, FACE_R));

        // Transformer body with cooling fins
        let (t_lf, t_rf) = box_faces(tx, ty, tx + 0.40, ty + 0.35, 4, 14, ox, oy);
        t_lf.fill(sprite, color::surf(pal::CONCRETE_GREY, light, FACE_L));
        t_rf.paint(sprite, &mut |u, _v, _x, _y| {
            let is_fin = (u % 2) == 0;
            let c = if is_fin { pal::METAL_STEEL_DARK } else { color::shade(pal::CONCRETE_LIGHT, 1.1) };
            Some((color::surf(c, light, FACE_R), 14))
        });
        let t_top = box_top(tx, ty, tx + 0.40, ty + 0.35, 14, ox, oy);
        t_top.fill(sprite, color::surf(pal::METAL_STEEL_DARK, light, FACE_TOP));

        // Ceramic insulator bushings
        parts::spire_needle(sprite, light, tx + 0.20, ty + 0.17, 14, 7, ox, oy);
    }

    // Perimeter security chainlink fence around front edges
    for i in 0..6 {
        let px = i as f32 * w / 5.0;
        let (p0_x, p0_y) = project(px, d, 2.0, ox, oy);
        let (p1_x, p1_y) = project(px, d, 8.0, ox, oy);
        geom::draw_line(sprite, p0_x.round() as i32, p0_y.round() as i32, p1_x.round() as i32, p1_y.round() as i32, color::surf(pal::METAL_STEEL_DARK, light, 1.0), 8);
    }
}
