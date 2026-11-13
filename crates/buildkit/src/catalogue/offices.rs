//! Offices: 8 visibly distinct designs (tints, mullion grids, setbacks, podiums, roof equipment).

use crate::color::{self, pal, FACE_L, FACE_R, FACE_TOP};
use crate::geom::{box_faces, box_top};
use crate::parts::{self, WinOpts};
use crate::types::{BuildingSpec, Light, Sprite};

/// 1. Classic Stone Bank / Office Building
pub fn draw_classic_stone(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let (w, d) = (2.05f32, 1.85f32);
    let v = spec.variant % 4;
    let z_wall = match v {
        0 => 48,
        1 => 60, // 5 storeys (+1 storey!)
        2 => 48, // 4 storeys temple front
        _ => 38, // 3 storeys corner bank
    };
    let age = spec.age;

    parts::building_shadow(sprite, 0.0, 0.0, w, d, z_wall, ox, oy);

    let stone_c = pal::STONE_LIMESTONE;
    let (lf, rf) = box_faces(0.0, 0.0, w, d, 0, z_wall, ox, oy);
    parts::paint_stone_ashlar(&lf, sprite, light, stone_c, FACE_L, age);
    parts::paint_stone_ashlar(&rf, sprite, light, stone_c, FACE_R, age);
    parts::corner_seam(sprite, light, w, d, 0, z_wall, pal::STONE_GRANITE, ox, oy);

    // Monumental classical pilasters on right face
    let len = rf.top_len();
    for col in 0..5 {
        let u = col * (len - 4) / 4;
        let col_q = rf.sub_px(u, 2, u + 3, z_wall - 10, rf.rows() as f32);
        col_q.fill(sprite, color::surf(color::shade(stone_c, 1.12), light, FACE_R));
    }

    let len_l = lf.top_len();
    for col in 0..4 {
        let u = col * (len_l - 4) / 3;
        let col_q = lf.sub_px(u, 2, u + 3, z_wall - 10, lf.rows() as f32);
        col_q.fill(sprite, color::surf(color::shade(stone_c, 1.12), light, FACE_L));
    }

    parts::roof_flat(sprite, light, 0.0, 0.0, w, d, z_wall, 6, pal::STONE_GRANITE, ox, oy, age);
    parts::draw_seed_rooftop_elements(sprite, light, w, d, z_wall, spec.seed, ox, oy);

    // Windows on both faces
    let wo = WinOpts {
        w: 6,
        h: 9,
        frame: pal::METAL_CHAMPAGNE,
        sill: true,
        lintel: true,
        shutters: false,
        shutter_c: pal::SHUTTER_BLACK,
        seed: spec.seed,
        lit_p: 0.60,
    };
    let num_floors = z_wall / 12;
    for floor in 0..num_floors - 1 {
        let v_win = 4 + floor * 11;
        for bay in 0..4 {
            let u = 4 + bay * (len - 4) / 4;
            parts::draw_window(&rf, sprite, light, u, v_win, wo, z_wall - floor * 11);
        }
        for bay in 0..3 {
            let u = 4 + bay * (len_l - 4) / 3;
            parts::draw_window(&lf, sprite, light, u, v_win, wo, z_wall - floor * 11);
        }
    }

    parts::draw_door(&rf, sprite, light, len / 2 - 5, z_wall - 14, 10, 14, pal::METAL_CHAMPAGNE, pal::STONE_GRANITE, 0);
}

/// 2. Art Deco Commercial Office Building
pub fn draw_art_deco(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let (w, d) = (2.05f32, 1.85f32);
    let v = spec.variant % 4;
    let (z_base, z_top) = match v {
        0 => (60, 78), // 7 storeys
        1 => (72, 96), // 9 storeys (+2 storeys!)
        2 => (50, 68), // 6 storeys
        _ => (68, 88), // 8 storeys
    };
    let age = spec.age;

    parts::building_shadow(sprite, 0.0, 0.0, w, d, z_top, ox, oy);

    let stone_c = pal::STONE_LIMESTONE;
    let (lf, rf) = box_faces(0.0, 0.0, w, d, 0, z_base, ox, oy);
    parts::paint_stone_ashlar(&lf, sprite, light, stone_c, FACE_L, age);
    parts::paint_stone_ashlar(&rf, sprite, light, stone_c, FACE_R, age);
    parts::corner_seam(sprite, light, w, d, 0, z_base, stone_c, ox, oy);

    let (lf_top, rf_top) = box_faces(0.2, 0.2, w - 0.2, d - 0.2, z_base, z_top, ox, oy);
    parts::paint_stone_ashlar(&lf_top, sprite, light, stone_c, FACE_L, age);
    parts::paint_stone_ashlar(&rf_top, sprite, light, stone_c, FACE_R, age);
    parts::corner_seam(sprite, light, w - 0.2, d - 0.2, z_base, z_top, stone_c, ox, oy);

    parts::roof_flat(sprite, light, 0.2, 0.2, w - 0.2, d - 0.2, z_top, 4, stone_c, ox, oy, age);
    parts::draw_seed_rooftop_elements(sprite, light, w, d, z_top, spec.seed + 1, ox, oy);

    let len = rf.top_len();
    for col in 0..4 {
        let u = 3 + col * (len - 6) / 3;
        let pier_q = rf.sub_px(u, 0, u + 2, z_base - 10, rf.rows() as f32);
        pier_q.fill(sprite, color::surf(color::shade(stone_c, 1.15), light, FACE_R));
    }

    let len_l = lf.top_len();
    for col in 0..3 {
        let u = 3 + col * (len_l - 6) / 2;
        let pier_q = lf.sub_px(u, 0, u + 2, z_base - 10, lf.rows() as f32);
        pier_q.fill(sprite, color::surf(color::shade(stone_c, 1.15), light, FACE_L));
    }

    let wo = WinOpts {
        w: 6,
        h: 7,
        frame: pal::METAL_CHAMPAGNE,
        sill: true,
        lintel: true,
        shutters: false,
        shutter_c: pal::SHUTTER_BLACK,
        seed: spec.seed,
        lit_p: 0.65,
    };
    let num_floors = z_base / 10;
    for floor in 0..num_floors - 1 {
        let v_win = 2 + floor * 9;
        for bay in 0..3 {
            let u = 6 + bay * (len - 6) / 3;
            parts::draw_window(&rf, sprite, light, u, v_win, wo, z_base - floor * 9);
        }
        for bay in 0..2 {
            let u = 6 + bay * (len_l - 6) / 2;
            parts::draw_window(&lf, sprite, light, u, v_win, wo, z_base - floor * 9);
        }
    }

    parts::draw_door(&rf, sprite, light, len / 2 - 4, z_base - 13, 8, 13, pal::METAL_CHAMPAGNE, pal::STONE_GRANITE, 0);
}

/// 3. Modern Glass Curtain Wall Office Building
pub fn draw_modern_glass(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let (w, d) = (2.05f32, 1.85f32);
    let v = spec.variant % 4;
    let z_wall = match v {
        0 => 84, // 8 storeys
        1 => 106, // 10 storeys (+2 storeys!)
        2 => 84,
        _ => 96,
    };
    let age = spec.age;

    parts::building_shadow(sprite, 0.0, 0.0, w, d, z_wall, ox, oy);

    let (lf, rf) = box_faces(0.0, 0.0, w, d, 0, z_wall, ox, oy);
    parts::paint_glass_curtain(&lf, sprite, light, 9, 8, FACE_L, spec.seed + 11);
    parts::paint_glass_curtain(&rf, sprite, light, 9, 8, FACE_R, spec.seed);
    parts::corner_seam(sprite, light, w, d, 0, z_wall, pal::METAL_STEEL_DARK, ox, oy);

    parts::roof_flat(sprite, light, 0.0, 0.0, w, d, z_wall, 4, pal::METAL_STEEL_DARK, ox, oy, age);
    parts::draw_seed_rooftop_elements(sprite, light, w, d, z_wall, spec.seed + 2, ox, oy);

    let can = box_top(w * 0.25, d, w * 0.75, d + 0.30, 14, ox, oy);
    can.fill(sprite, color::surf(pal::METAL_STEEL_DARK, light, FACE_TOP));
}

/// 4. Corporate Plaza with precast concrete grid facade
pub fn draw_corporate_plaza(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let (w, d) = (2.05f32, 1.85f32);
    let v = spec.variant % 4;
    let z_wall = match v {
        0 => 102, // 10 storeys
        1 => 124, // 12 storeys (+2 storeys!)
        2 => 102,
        _ => 114,
    };
    let age = spec.age;

    parts::building_shadow(sprite, 0.0, 0.0, w, d, z_wall, ox, oy);

    let (lf, rf) = box_faces(0.0, 0.0, w, d, 0, z_wall, ox, oy);
    let grid_c = if v == 2 { pal::METAL_CHAMPAGNE } else { pal::CONCRETE_LIGHT };

    rf.paint(sprite, &mut |u, v_win, x, y| {
        let is_floor_band = (v_win % 9) == 0 || (v_win % 9) == 1;
        let is_col_pier = (u % 8) == 0 || (u % 8) == 1;
        let depth = (z_wall - v_win) as u16;

        if is_floor_band || is_col_pier {
            return Some((color::surf(grid_c, light, FACE_R), depth));
        }

        if light.is_night {
            let lit = (color::hash_seed(spec.seed, (u / 8 + v_win / 9 * 31) as u32) % 100) < 45;
            if lit {
                Some((color::WIN_COOL, depth))
            } else {
                Some((color::WIN_OFF_NIGHT, depth))
            }
        } else {
            let diag = (x * 2 - y) / 8;
            let glint = (diag % 6) == 0;
            let c = if glint { pal::GLASS_REFLECTIVE } else { pal::GLASS_DARK };
            Some((color::surf(c, light, FACE_R), depth))
        }
    });

    lf.paint(sprite, &mut |u, v_win, x, y| {
        let is_floor_band = (v_win % 9) == 0 || (v_win % 9) == 1;
        let is_col_pier = (u % 8) == 0 || (u % 8) == 1;
        let depth = (z_wall - v_win) as u16;

        if is_floor_band || is_col_pier {
            return Some((color::surf(grid_c, light, FACE_L), depth));
        }

        if light.is_night {
            let lit = (color::hash_seed(spec.seed, (u / 8 + v_win / 9 * 31 + 7) as u32) % 100) < 45;
            if lit {
                Some((color::WIN_COOL, depth))
            } else {
                Some((color::WIN_OFF_NIGHT, depth))
            }
        } else {
            let diag = (x * 2 - y) / 8;
            let glint = (diag % 6) == 0;
            let c = if glint { pal::GLASS_REFLECTIVE } else { pal::GLASS_DARK };
            Some((color::surf(c, light, FACE_L), depth))
        }
    });
    parts::corner_seam(sprite, light, w, d, 0, z_wall, grid_c, ox, oy);

    parts::roof_flat(sprite, light, 0.0, 0.0, w, d, z_wall, 4, pal::METAL_STEEL_DARK, ox, oy, age);
    parts::draw_seed_rooftop_elements(sprite, light, w, d, z_wall, spec.seed + 3, ox, oy);
}

/// 5. Teal Glass Curtain Wall Office with white vertical mullion fins
pub fn draw_teal_curtain(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let (w, d) = (2.05f32, 1.85f32);
    let v = spec.variant % 4;
    let z_wall = match v {
        0 => 84, // 8 storeys
        1 => 106, // 10 storeys
        2 => 84,
        _ => 96,
    };
    let age = spec.age;

    parts::building_shadow(sprite, 0.0, 0.0, w, d, z_wall, ox, oy);

    let (lf, rf) = box_faces(0.0, 0.0, w, d, 0, z_wall, ox, oy);
    let teal_c = pal::GLASS_TEAL;

    rf.paint(sprite, &mut |u, v_win, _x, _y| {
        let is_mullion = (u % 6) == 0;
        let depth = (z_wall - v_win) as u16;
        if is_mullion {
            return Some((color::surf(pal::METAL_WHITE, light, 1.15), depth));
        }
        if light.is_night {
            let lit = (color::hash_seed(spec.seed, (u / 6 + v_win / 7 * 31) as u32) % 100) < 45;
            let c = if lit { color::WIN_COOL } else { color::WIN_OFF_NIGHT };
            Some((c, depth))
        } else {
            Some((color::surf(teal_c, light, FACE_R), depth))
        }
    });

    lf.paint(sprite, &mut |u, v_win, _x, _y| {
        let is_mullion = (u % 6) == 0;
        let depth = (z_wall - v_win) as u16;
        if is_mullion {
            return Some((color::surf(pal::METAL_WHITE, light, 1.15), depth));
        }
        if light.is_night {
            let lit = (color::hash_seed(spec.seed, (u / 6 + v_win / 7 * 31 + 7) as u32) % 100) < 45;
            let c = if lit { color::WIN_COOL } else { color::WIN_OFF_NIGHT };
            Some((c, depth))
        } else {
            Some((color::surf(teal_c, light, FACE_L), depth))
        }
    });
    parts::corner_seam(sprite, light, w, d, 0, z_wall, pal::METAL_WHITE, ox, oy);

    parts::roof_flat(sprite, light, 0.0, 0.0, w, d, z_wall, 4, pal::METAL_STEEL_DARK, ox, oy, age);
    parts::draw_seed_rooftop_elements(sprite, light, w, d, z_wall, spec.seed + 4, ox, oy);
}

/// 6. Bronze Pavilion Office with copper sunshades and retail lobby
pub fn draw_bronze_pavilion(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let (w, d) = (2.05f32, 1.85f32);
    let v = spec.variant % 4;
    let z_wall = match v {
        0 => 74, // 7 storeys
        1 => 86, // 8 storeys
        2 => 62, // 6 storeys
        _ => 74,
    };
    let age = spec.age;

    parts::building_shadow(sprite, 0.0, 0.0, w, d, z_wall, ox, oy);

    let (lf, rf) = box_faces(0.0, 0.0, w, d, 0, z_wall, ox, oy);
    let copper_c = pal::ROOF_COPPER_NEW;

    rf.paint(sprite, &mut |u, v_win, _x, _y| {
        let is_fin = (u % 5) == 0;
        let depth = (z_wall - v_win) as u16;
        if is_fin {
            return Some((color::surf(copper_c, light, 1.20), depth));
        }
        if light.is_night {
            let lit = (color::hash_seed(spec.seed, (u / 5 + v_win / 8 * 27) as u32) % 100) < 50;
            let c = if lit { color::WIN_WARM } else { color::WIN_OFF_NIGHT };
            Some((c, depth))
        } else {
            Some((color::surf(pal::GLASS_BRONZE, light, FACE_R), depth))
        }
    });

    lf.paint(sprite, &mut |u, v_win, _x, _y| {
        let is_fin = (u % 5) == 0;
        let depth = (z_wall - v_win) as u16;
        if is_fin {
            return Some((color::surf(copper_c, light, 1.20), depth));
        }
        if light.is_night {
            let lit = (color::hash_seed(spec.seed, (u / 5 + v_win / 8 * 27 + 5) as u32) % 100) < 50;
            let c = if lit { color::WIN_WARM } else { color::WIN_OFF_NIGHT };
            Some((c, depth))
        } else {
            Some((color::surf(pal::GLASS_BRONZE, light, FACE_L), depth))
        }
    });
    parts::corner_seam(sprite, light, w, d, 0, z_wall, copper_c, ox, oy);

    parts::roof_flat(sprite, light, 0.0, 0.0, w, d, z_wall, 4, pal::METAL_STEEL_DARK, ox, oy, age);
    parts::draw_seed_rooftop_elements(sprite, light, w, d, z_wall, spec.seed + 5, ox, oy);
}

/// 7. Brutalist Concrete Office with deep recessed ribbon windows
pub fn draw_brutalist_concrete(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let (w, d) = (2.05f32, 1.85f32);
    let v = spec.variant % 4;
    let z_wall = match v {
        0 => 84, // 8 storeys
        1 => 96, // 9 storeys
        2 => 74, // 7 storeys
        _ => 84,
    };
    let age = spec.age;

    parts::building_shadow(sprite, 0.0, 0.0, w, d, z_wall, ox, oy);

    let (lf, rf) = box_faces(0.0, 0.0, w, d, 0, z_wall, ox, oy);
    let conc_c = pal::CONCRETE_GREY;

    // Deep recessed ribbon windows between cantilevered concrete spandrels
    rf.paint(sprite, &mut |u, v_win, _x, _y| {
        let is_spandrel = (v_win % 10) < 4;
        let depth = (z_wall - v_win) as u16;
        if is_spandrel {
            return Some((color::surf(color::shade(conc_c, 1.15), light, FACE_R), depth));
        }
        let is_mullion = (u % 7) == 0;
        if is_mullion {
            return Some((color::surf(pal::METAL_STEEL_DARK, light, 1.0), depth));
        }
        if light.is_night {
            let lit = (color::hash_seed(spec.seed, (u / 7 + v_win / 10 * 31) as u32) % 100) < 40;
            let c = if lit { color::WIN_COOL } else { color::WIN_OFF_NIGHT };
            Some((c, depth))
        } else {
            Some((color::surf(pal::GLASS_DARK, light, FACE_R), depth))
        }
    });

    lf.paint(sprite, &mut |u, v_win, _x, _y| {
        let is_spandrel = (v_win % 10) < 4;
        let depth = (z_wall - v_win) as u16;
        if is_spandrel {
            return Some((color::surf(color::shade(conc_c, 1.15), light, FACE_L), depth));
        }
        let is_mullion = (u % 7) == 0;
        if is_mullion {
            return Some((color::surf(pal::METAL_STEEL_DARK, light, 1.0), depth));
        }
        if light.is_night {
            let lit = (color::hash_seed(spec.seed, (u / 7 + v_win / 10 * 31 + 7) as u32) % 100) < 40;
            let c = if lit { color::WIN_COOL } else { color::WIN_OFF_NIGHT };
            Some((c, depth))
        } else {
            Some((color::surf(pal::GLASS_DARK, light, FACE_L), depth))
        }
    });
    parts::corner_seam(sprite, light, w, d, 0, z_wall, conc_c, ox, oy);

    parts::roof_flat(sprite, light, 0.0, 0.0, w, d, z_wall, 4, conc_c, ox, oy, age);
    parts::draw_seed_rooftop_elements(sprite, light, w, d, z_wall, spec.seed, ox, oy);
}

/// 8. Post-Modern Clock Office Building with striped masonry and pediment clock
pub fn draw_postmodern_clock(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let (w, d) = (2.05f32, 1.85f32);
    let v = spec.variant % 4;
    let z_wall = match v {
        0 => 94, // 9 storeys
        1 => 106, // 10 storeys
        2 => 84, // 8 storeys
        _ => 94,
    };
    let age = spec.age;

    parts::building_shadow(sprite, 0.0, 0.0, w, d, z_wall + 12, ox, oy);

    let (lf, rf) = box_faces(0.0, 0.0, w, d, 0, z_wall, ox, oy);

    // Alternating horizontal stripes of granite and red sandstone
    rf.paint(sprite, &mut |_u, v_win, _x, _y| {
        let is_stripe = (v_win % 8) < 4;
        let depth = (z_wall - v_win) as u16;
        let c = if is_stripe { pal::STONE_GRANITE } else { pal::STONE_BROWNSTONE };
        Some((color::surf(c, light, FACE_R), depth))
    });
    lf.paint(sprite, &mut |_u, v_win, _x, _y| {
        let is_stripe = (v_win % 8) < 4;
        let depth = (z_wall - v_win) as u16;
        let c = if is_stripe { pal::STONE_GRANITE } else { pal::STONE_BROWNSTONE };
        Some((color::surf(c, light, FACE_L), depth))
    });
    parts::corner_seam(sprite, light, w, d, 0, z_wall, pal::STONE_GRANITE, ox, oy);

    // Windows
    let len = rf.top_len();
    let wo = WinOpts {
        w: 6,
        h: 8,
        frame: pal::METAL_CHAMPAGNE,
        sill: true,
        lintel: true,
        shutters: false,
        shutter_c: pal::SHUTTER_BLACK,
        seed: spec.seed,
        lit_p: 0.65,
    };
    let num_floors = z_wall / 11;
    for floor in 0..num_floors - 1 {
        let v_win = 3 + floor * 11;
        for bay in 0..3 {
            let u = 5 + bay * (len - 8) / 2;
            parts::draw_window(&rf, sprite, light, u, v_win, wo, z_wall - floor * 11);
        }
    }

    // Triangular pediment with clock atop facade
    let ped_pts = [
        crate::geom::project(0.2, d, z_wall as f32, ox, oy),
        crate::geom::project(w - 0.2, d, z_wall as f32, ox, oy),
        crate::geom::project(w / 2.0, d, (z_wall + 12) as f32, ox, oy),
    ];
    crate::geom::fill_polygon(sprite, &ped_pts, color::surf(pal::STONE_GRANITE, light, FACE_R), (z_wall + 12) as u16);

    let (cx, cy) = crate::geom::project(w / 2.0, d, (z_wall + 4) as f32, ox, oy);
    let clock_c = if light.is_night { color::WIN_WARM } else { pal::TRIM_WHITE };
    for dy in -2..=2 {
        for dx in -2..=2 {
            if dx * dx + dy * dy <= 4 {
                sprite.set_pixel(cx.round() as i32 + dx, cy.round() as i32 + dy, clock_c, (z_wall + 6) as u16);
            }
        }
    }

    parts::roof_flat(sprite, light, 0.0, 0.0, w, d, z_wall, 4, pal::STONE_GRANITE, ox, oy, age);
    parts::draw_seed_rooftop_elements(sprite, light, w, d, z_wall, spec.seed + 1, ox, oy);
}
