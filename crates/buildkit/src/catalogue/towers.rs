//! Skyscraper Towers: 16 Slender, Varied Designs with distinct crowns, glass tints, and setbacks.

use crate::color::{self, pal, FACE_L, FACE_R, FACE_TOP};
use crate::geom::{box_faces, box_top};
use crate::parts::{self};
use crate::types::{BuildingSpec, Light, Sprite};

/// 1. Art Deco Spire Tower (stepped setbacks and a gilded spire)
pub fn draw_art_deco_spire(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let (w, d) = (1.50f32, 1.30f32);
    let v = spec.variant % 4;
    let (z1, z2, z3, spire_h) = match v {
        0 => (90, 138, 172, 38), // 45 storeys
        1 => (110, 166, 206, 44), // 54 storeys (+9 storeys!)
        2 => (78, 120, 150, 32), // 38 storeys
        _ => (100, 150, 186, 40), // 48 storeys
    };

    parts::building_shadow(sprite, 0.0, 0.0, w, d, z3 + spire_h, ox, oy);

    let stone_c = pal::STONE_LIMESTONE;

    // Base podium & shaft
    let (lf1, rf1) = box_faces(0.0, 0.0, w, d, 0, z1, ox, oy);
    parts::paint_stone_ashlar(&lf1, sprite, light, stone_c, FACE_L, spec.age);
    parts::paint_stone_ashlar(&rf1, sprite, light, stone_c, FACE_R, spec.age);
    parts::corner_seam(sprite, light, w, d, 0, z1, stone_c, ox, oy);
    let t1 = box_top(0.0, 0.0, w, d, z1, ox, oy);
    t1.fill(sprite, color::surf(pal::STONE_GRANITE, light, FACE_TOP));

    // Tier 2 setback
    let (lf2, rf2) = box_faces(0.18, 0.18, w - 0.18, d - 0.18, z1, z2, ox, oy);
    parts::paint_stone_ashlar(&lf2, sprite, light, stone_c, FACE_L, spec.age);
    parts::paint_stone_ashlar(&rf2, sprite, light, stone_c, FACE_R, spec.age);
    parts::corner_seam(sprite, light, w - 0.18, d - 0.18, z1, z2, stone_c, ox, oy);
    let t2 = box_top(0.18, 0.18, w - 0.18, d - 0.18, z2, ox, oy);
    t2.fill(sprite, color::surf(pal::STONE_GRANITE, light, FACE_TOP));

    // Tier 3 crown
    let (lf3, rf3) = box_faces(0.35, 0.35, w - 0.35, d - 0.35, z2, z3, ox, oy);
    rf3.paint(sprite, &mut |u, v_win, _x, _y| {
        let is_sunburst = (u + v_win) % 4 == 0;
        let c = if light.night > 0.05 {
            if is_sunburst { color::crown_lit(light) } else { pal::METAL_STEEL_DARK }
        } else {
            if is_sunburst { pal::METAL_CHAMPAGNE } else { stone_c }
        };
        Some((color::surf(c, light, FACE_R), (z3 - v_win) as u16))
    });
    lf3.paint(sprite, &mut |u, v_win, _x, _y| {
        let is_sunburst = (u + v_win) % 4 == 0;
        let c = if light.night > 0.05 {
            if is_sunburst { color::crown_lit(light) } else { pal::METAL_STEEL_DARK }
        } else {
            if is_sunburst { pal::METAL_CHAMPAGNE } else { stone_c }
        };
        Some((color::surf(c, light, FACE_L), (z3 - v_win) as u16))
    });
    parts::corner_seam(sprite, light, w - 0.35, d - 0.35, z2, z3, pal::METAL_CHAMPAGNE, ox, oy);

    // Tier 3 roof deck
    let t3 = box_top(0.35, 0.35, w - 0.35, d - 0.35, z3, ox, oy);
    t3.fill(sprite, color::surf(pal::METAL_CHAMPAGNE, light, FACE_TOP));

    // Closed 4-sided stainless steel pyramidal crown rising to spire apex
    let mx = w / 2.0;
    let my = d / 2.0;
    let z_pyr = z3 + 14;
    let (c_fl, c_fr) = (
        color::surf(pal::METAL_CHAMPAGNE, light, FACE_L),
        color::surf(color::shade(pal::METAL_CHAMPAGNE, 0.85), light, FACE_R),
    );
    let pyr_fl = [
        crate::geom::project(0.35, d - 0.35, z3 as f32, ox, oy),
        crate::geom::project(w - 0.35, d - 0.35, z3 as f32, ox, oy),
        crate::geom::project(mx, my, z_pyr as f32, ox, oy),
    ];
    sprite.on_surface(render::snow::SLOPE_MEDIUM);
    crate::geom::fill_polygon(sprite, &pyr_fl, c_fl, z_pyr as u16);
    let pyr_fr = [
        crate::geom::project(w - 0.35, 0.35, z3 as f32, ox, oy),
        crate::geom::project(w - 0.35, d - 0.35, z3 as f32, ox, oy),
        crate::geom::project(mx, my, z_pyr as f32, ox, oy),
    ];
    crate::geom::fill_polygon(sprite, &pyr_fr, c_fr, z_pyr as u16);
    sprite.on_surface(render::snow::NONE);

    // Stainless steel needle spire rising from pyramid apex
    parts::spire_needle(sprite, light, mx, my, z_pyr, spire_h, ox, oy);
}

/// 2. Modern Glass Curtain Wall Skyscraper (Slender 1×1, blue reflective glass)
pub fn draw_glass_curtain_wall(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let (w, d) = (1.15f32, 1.05f32);
    let v = spec.variant % 4;
    let z_top = match v {
        0 => 160, // 38 storeys
        1 => 195, // 46 storeys (+8 storeys!)
        2 => 140, // 32 storeys
        _ => 175, // 42 storeys
    };

    parts::building_shadow(sprite, 0.0, 0.0, w, d, z_top, ox, oy);

    let (lf, rf) = box_faces(0.0, 0.0, w, d, 0, z_top, ox, oy);
    parts::paint_glass_curtain(&lf, sprite, light, 7, 6, FACE_L, spec.seed + 19);
    parts::paint_glass_curtain(&rf, sprite, light, 7, 6, FACE_R, spec.seed);
    parts::corner_seam(sprite, light, w, d, 0, z_top, pal::METAL_STEEL_DARK, ox, oy);

    let shaft_top = box_top(0.0, 0.0, w, d, z_top, ox, oy);
    shaft_top.fill(sprite, color::surf(pal::METAL_STEEL_DARK, light, FACE_TOP));

    // A glass crown over the shaft with a lit accent line at its top edge in the
    // evening (B99d): architecture by day, a lit crown at night, never a solid
    // band of saturated colour across the parapet.
    let (c_lf, c_rf) = box_faces(0.05, 0.05, w - 0.05, d - 0.05, z_top, z_top + 10, ox, oy);
    let n = light.night.clamp(0.0, 1.0);
    let glass_r = color::surf(pal::GLASS_REFLECTIVE, light, FACE_R);
    let glass_l = color::surf(pal::GLASS_REFLECTIVE, light, FACE_L);
    c_rf.fill(sprite, color::mix(glass_r, color::crown_metal(light, FACE_R), n));
    c_lf.fill(sprite, color::mix(glass_l, color::crown_metal(light, FACE_L), n));
    if n > 0.05 {
        // Only the top row of the crown carries the accent light.
        let lit = color::crown_lit(light);
        let (t_lf, t_rf) = box_faces(0.05, 0.05, w - 0.05, d - 0.05, z_top + 9, z_top + 10, ox, oy);
        t_rf.fill(sprite, lit);
        t_lf.fill(sprite, color::shade(lit, 0.88));
    }

    parts::spire_needle(sprite, light, w / 2.0, d / 2.0, z_top + 10, 16, ox, oy);
}

/// 3. Tiered Terraces Skyscraper (2×2, warm champagne metal, sky gardens)
pub fn draw_tiered_terraces(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let (w, d) = (2.00f32, 1.80f32);
    let v = spec.variant % 4;
    let (z1, z2, z3) = match v {
        0 => (65, 110, 146), // 34 storeys
        1 => (80, 134, 178), // 42 storeys (+8 storeys!)
        2 => (56, 96, 128),  // 28 storeys
        _ => (72, 122, 160), // 38 storeys
    };

    parts::building_shadow(sprite, 0.0, 0.0, w, d, z3, ox, oy);

    // Tier 1
    let (lf1, rf1) = box_faces(0.0, 0.0, w, d, 0, z1, ox, oy);
    parts::paint_glass_curtain(&rf1, sprite, light, 8, 7, FACE_R, spec.seed);
    parts::paint_glass_curtain(&lf1, sprite, light, 8, 7, FACE_L, spec.seed + 10);
    parts::corner_seam(sprite, light, w, d, 0, z1, pal::METAL_CHAMPAGNE, ox, oy);
    let t1 = box_top(0.0, 0.0, w, d, z1, ox, oy);
    t1.fill(sprite, color::surf(pal::FOLIAGE_GREEN, light, FACE_TOP));

    // Tier 2
    let (lf2, rf2) = box_faces(0.25, 0.25, w - 0.25, d - 0.25, z1, z2, ox, oy);
    parts::paint_glass_curtain(&rf2, sprite, light, 8, 7, FACE_R, spec.seed + 1);
    parts::paint_glass_curtain(&lf2, sprite, light, 8, 7, FACE_L, spec.seed + 11);
    parts::corner_seam(sprite, light, w - 0.25, d - 0.25, z1, z2, pal::METAL_CHAMPAGNE, ox, oy);
    let t2 = box_top(0.25, 0.25, w - 0.25, d - 0.25, z2, ox, oy);
    t2.fill(sprite, color::surf(pal::FOLIAGE_GREEN, light, FACE_TOP));

    // Tier 3 Penthouse
    let (lf3, rf3) = box_faces(0.5, 0.5, w - 0.5, d - 0.5, z2, z3, ox, oy);
    parts::paint_glass_curtain(&rf3, sprite, light, 8, 7, FACE_R, spec.seed + 2);
    parts::paint_glass_curtain(&lf3, sprite, light, 8, 7, FACE_L, spec.seed + 12);
    parts::corner_seam(sprite, light, w - 0.5, d - 0.5, z2, z3, pal::METAL_CHAMPAGNE, ox, oy);

    let t3 = box_top(0.5, 0.5, w - 0.5, d - 0.5, z3, ox, oy);
    t3.fill(sprite, color::surf(pal::METAL_STEEL_DARK, light, FACE_TOP));
    parts::spire_needle(sprite, light, w / 2.0, d / 2.0, z3, 14, ox, oy);
}

/// 4. Cross-Braced Mega Tower (Slender 1×2, exposed X-braced frame)
pub fn draw_cross_braced(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let (w, d) = (1.40f32, 1.85f32);
    let v = spec.variant % 4;
    let z_top = match v {
        0 => 205, // 50 storeys
        1 => 240, // 58 storeys (+8 storeys!)
        2 => 175, // 42 storeys
        _ => 215, // 52 storeys
    };

    parts::building_shadow(sprite, 0.0, 0.0, w, d, z_top, ox, oy);

    let (lf, rf) = box_faces(0.0, 0.0, w, d, 0, z_top, ox, oy);

    // B99: the glass between the braces is a real curtain wall (floors, mullions,
    // night-lit panes) like every other tower; it used to be one flat dark colour,
    // so these 50-storey towers read as blank slabs by day and never lit at night.
    parts::paint_glass_curtain(&lf, sprite, light, 7, 6, FACE_L, spec.seed + 19);
    parts::paint_glass_curtain(&rf, sprite, light, 7, 6, FACE_R, spec.seed);
    // Matte black structural steel with massive X-bracing truss on both faces.
    for face in [&rf, &lf] {
        face.paint(sprite, &mut |u, v_win, _x, _y| {
            // `u` counts half-pixel steps along the face (Quad::paint samples
            // twice per pixel), so the face is 2 * top_len() steps wide. The
            // old code used the pixel length: past the first half every pixel
            // counted as the corner column, a flat dark band down each face.
            let len = (face.top_len() * 2).max(1);
            let depth = (z_top - v_win) as u16;
            let u_rel = u as f32 / len as f32;
            let v_rel = (v_win % 32) as f32 / 32.0;
            let is_diag_a = (u_rel - v_rel).abs() < 0.05;
            let is_diag_b = (u_rel - (1.0 - v_rel)).abs() < 0.05;
            let is_belt_truss = (v_win % 32) <= 2;
            let is_corner_col = u <= 3 || u >= len - 4;
            if is_diag_a || is_diag_b || is_belt_truss || is_corner_col {
                Some((color::surf(pal::METAL_STEEL_DARK, light, 1.15), depth))
            } else {
                None
            }
        });
    }
    parts::corner_seam(sprite, light, w, d, 0, z_top, pal::METAL_STEEL_DARK, ox, oy);

    parts::roof_flat(sprite, light, 0.0, 0.0, w, d, z_top, 4, pal::METAL_STEEL_DARK, ox, oy, 0.0);

    parts::spire_needle(sprite, light, w * 0.35, d * 0.5, z_top, 24, ox, oy);
    parts::spire_needle(sprite, light, w * 0.65, d * 0.5, z_top, 24, ox, oy);
}

/// 5. Slanted Modern Glass Tower (Slender 1×1, leaning slab)
pub fn draw_slanted_glass(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let (w, d) = (1.20f32, 1.10f32);
    let v = spec.variant % 4;
    let z_top = match v {
        0 => 135, // 30 storeys
        1 => 165, // 38 storeys (+8 storeys!)
        2 => 120, // 26 storeys
        _ => 150, // 34 storeys
    };

    parts::building_shadow(sprite, 0.0, 0.0, w, d, z_top, ox, oy);

    let (lf, rf) = box_faces(0.0, 0.0, w, d, 0, z_top, ox, oy);
    parts::paint_glass_curtain(&lf, sprite, light, 7, 6, FACE_L, spec.seed + 31);
    parts::paint_glass_curtain(&rf, sprite, light, 7, 6, FACE_R, spec.seed);
    parts::corner_seam(sprite, light, w, d, 0, z_top, pal::METAL_WHITE, ox, oy);

    let top = box_top(0.0, 0.0, w, d, z_top, ox, oy);
    top.paint(sprite, &mut |u, v_top, _x, _y| {
        let is_border = u == 0 || v_top == 0 || u == 15 || v_top == 15;
        let is_h = (u >= 6 && u <= 9 && v_top >= 4 && v_top <= 11) && (u == 6 || u == 9 || v_top == 7 || v_top == 8);
        let c = if is_border {
            pal::METAL_WHITE
        } else if is_h {
            color::GOLD_SIGN
        } else {
            pal::ASPHALT
        };
        Some((color::surf(c, light, FACE_TOP), z_top as u16))
    });
}

/// 6. Cylindrical Rounded Crown Tower (Slender 1×1)
pub fn draw_cylindrical_crown(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let (w, d) = (1.20f32, 1.10f32);
    let v = spec.variant % 4;
    let (z_body, z_crown) = match v {
        0 => (130, 150), // 34 storeys
        1 => (160, 182), // 42 storeys (+8 storeys!)
        2 => (115, 134), // 28 storeys
        _ => (145, 166), // 38 storeys
    };

    parts::building_shadow(sprite, 0.0, 0.0, w, d, z_crown, ox, oy);

    let (lf, rf) = box_faces(0.0, 0.0, w, d, 0, z_body, ox, oy);
    parts::paint_glass_curtain(&lf, sprite, light, 7, 6, FACE_L, spec.seed);
    parts::paint_glass_curtain(&rf, sprite, light, 7, 6, FACE_R, spec.seed + 5);
    parts::corner_seam(sprite, light, w, d, 0, z_body, pal::METAL_WHITE, ox, oy);

    let (c_lf, c_rf) = box_faces(0.12, 0.12, w - 0.12, d - 0.12, z_body, z_crown, ox, oy);
    c_rf.paint(sprite, &mut |_u, v_win, _x, _y| {
        let is_louver = (v_win % 3) == 0;
        let c = if light.night > 0.05 {
            if is_louver { color::crown_lit(light) } else { pal::METAL_STEEL_DARK }
        } else {
            if is_louver { pal::METAL_WHITE } else { pal::GLASS_DARK }
        };
        Some((color::surf(c, light, FACE_R), (z_crown - v_win) as u16))
    });
    c_lf.paint(sprite, &mut |_u, v_win, _x, _y| {
        let is_louver = (v_win % 3) == 0;
        let c = if light.night > 0.05 {
            if is_louver { color::crown_lit(light) } else { pal::METAL_STEEL_DARK }
        } else {
            if is_louver { pal::METAL_WHITE } else { pal::GLASS_DARK }
        };
        Some((color::surf(c, light, FACE_L), (z_crown - v_win) as u16))
    });
    parts::corner_seam(sprite, light, w - 0.12, d - 0.12, z_body, z_crown, pal::METAL_WHITE, ox, oy);

    let top = box_top(0.12, 0.12, w - 0.12, d - 0.12, z_crown, ox, oy);
    top.fill(sprite, color::surf(pal::METAL_WHITE, light, FACE_TOP));

    parts::spire_needle(sprite, light, w / 2.0, d / 2.0, z_crown, 18, ox, oy);
}

/// 7. Reflective Bands Corporate Headquarters Tower (Slender 1×2)
pub fn draw_reflective_bands(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let (w, d) = (1.35f32, 1.75f32);
    let v = spec.variant % 4;
    let z_top = match v {
        0 => 135, // 30 storeys
        1 => 165, // 38 storeys (+8 storeys!)
        2 => 120, // 26 storeys
        _ => 150, // 34 storeys
    };

    parts::building_shadow(sprite, 0.0, 0.0, w, d, z_top, ox, oy);

    let (lf, rf) = box_faces(0.0, 0.0, w, d, 0, z_top, ox, oy);

    rf.paint(sprite, &mut |_u, v_win, x, y| {
        let is_granite_band = (v_win % 10) < 4;
        let depth = (z_top - v_win) as u16;
        if is_granite_band {
            let c = pal::STONE_GRANITE;
            Some((color::surf(c, light, FACE_R), depth))
        } else if light.is_night {
            let lit = (color::hash_seed(spec.seed, (v_win / 10 * 37) as u32) % 100) < 50;
            let c = if lit { color::WIN_WARM } else { color::WIN_OFF_NIGHT };
            Some((c, depth))
        } else {
            let diag = (x * 2 - y) / 8;
            let glint = (diag % 5) == 0;
            let c = if glint { pal::GLASS_REFLECTIVE } else { pal::GLASS_DARK };
            Some((color::surf(c, light, FACE_R), depth))
        }
    });

    lf.paint(sprite, &mut |_u, v_win, x, y| {
        let is_granite_band = (v_win % 10) < 4;
        let depth = (z_top - v_win) as u16;
        if is_granite_band {
            let c = pal::STONE_GRANITE;
            Some((color::surf(c, light, FACE_L), depth))
        } else if light.is_night {
            let lit = (color::hash_seed(spec.seed, (v_win / 10 * 37 + 13) as u32) % 100) < 50;
            let c = if lit { color::WIN_WARM } else { color::WIN_OFF_NIGHT };
            Some((c, depth))
        } else {
            let diag = (x * 2 - y) / 8;
            let glint = (diag % 5) == 0;
            let c = if glint { pal::GLASS_REFLECTIVE } else { pal::GLASS_DARK };
            Some((color::surf(c, light, FACE_L), depth))
        }
    });
    parts::corner_seam(sprite, light, w, d, 0, z_top, pal::STONE_GRANITE, ox, oy);

    parts::roof_flat(sprite, light, 0.0, 0.0, w, d, z_top, 4, pal::STONE_GRANITE, ox, oy, 0.0);
    parts::rooftop_hvac(sprite, light, 0.35, 0.45, z_top, ox, oy);
}

/// 8. Slender Luxury Pinnacle Pencil Tower (Ultra-slender 1×1, 56 storeys)
pub fn draw_slender_pinnacle(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let (w, d) = (1.05f32, 0.95f32);
    let v = spec.variant % 4;
    let z_top = match v {
        0 => 235, // 56 storeys
        1 => 265, // 64 storeys (+8 storeys!)
        2 => 210, // 48 storeys
        _ => 245, // 58 storeys
    };

    parts::building_shadow(sprite, 0.0, 0.0, w, d, z_top + 25, ox, oy);

    let (lf, rf) = box_faces(0.0, 0.0, w, d, 0, z_top, ox, oy);

    rf.paint(sprite, &mut |u, v_win, _x, _y| {
        let depth = (z_top - v_win) as u16;
        if v_win >= 70 && v_win <= 78 && u > 2 && u < rf.top_len() - 3 {
            return None; // open wind pass-through sky notch!
        }
        let is_mullion = (u % 5) == 0;
        if is_mullion {
            return Some((color::surf(pal::METAL_CHAMPAGNE, light, 1.1), depth));
        }
        if light.is_night {
            let lit = (color::hash_seed(spec.seed, (u / 5 + v_win / 8 * 29) as u32) % 100) < 55;
            let c = if lit { color::WIN_WARM } else { color::WIN_OFF_NIGHT };
            Some((c, depth))
        } else {
            Some((color::surf(pal::GLASS_BRONZE, light, FACE_R), depth))
        }
    });

    lf.paint(sprite, &mut |u, v_win, _x, _y| {
        let depth = (z_top - v_win) as u16;
        if v_win >= 70 && v_win <= 78 && u > 2 && u < lf.top_len() - 3 {
            return None;
        }
        let is_mullion = (u % 5) == 0;
        if is_mullion {
            return Some((color::surf(pal::METAL_CHAMPAGNE, light, 1.1), depth));
        }
        if light.is_night {
            let lit = (color::hash_seed(spec.seed, (u / 5 + v_win / 8 * 29 + 17) as u32) % 100) < 55;
            let c = if lit { color::WIN_WARM } else { color::WIN_OFF_NIGHT };
            Some((c, depth))
        } else {
            Some((color::surf(pal::GLASS_BRONZE, light, FACE_L), depth))
        }
    });
    parts::corner_seam(sprite, light, w, d, 0, z_top, pal::METAL_CHAMPAGNE, ox, oy);

    let top = box_top(0.0, 0.0, w, d, z_top, ox, oy);
    top.fill(sprite, color::surf(pal::METAL_CHAMPAGNE, light, FACE_TOP));

    parts::spire_needle(sprite, light, w / 2.0, d / 2.0, z_top, 28, ox, oy);
}

/// 9. Slender Teal Obelisk Tower (1×1, 42 storeys, faceted corner chamfers)
pub fn draw_teal_obelisk(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let (w, d) = (1.15f32, 1.05f32);
    let v = spec.variant % 4;
    let z_top = match v {
        0 => 175, // 42 storeys
        1 => 210, // 50 storeys (+8 storeys!)
        2 => 155, // 36 storeys
        _ => 190, // 46 storeys
    };

    parts::building_shadow(sprite, 0.0, 0.0, w, d, z_top + 14, ox, oy);

    let (lf, rf) = box_faces(0.0, 0.0, w, d, 0, z_top, ox, oy);
    let teal_c = pal::GLASS_TEAL;
    rf.paint(sprite, &mut |u, v_win, _x, _y| {
        let is_mullion = (u % 6) == 0;
        let depth = (z_top - v_win) as u16;
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
        let depth = (z_top - v_win) as u16;
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
    parts::corner_seam(sprite, light, w, d, 0, z_top, pal::METAL_WHITE, ox, oy);

    // Pyramid lantern cap at crown
    let mx = w / 2.0;
    let my = d / 2.0;
    let pyr_pts = [
        crate::geom::project(0.0, d, z_top as f32, ox, oy),
        crate::geom::project(w, d, z_top as f32, ox, oy),
        crate::geom::project(mx, my, (z_top + 14) as f32, ox, oy),
    ];
    let cap_c = color::mix(
        color::surf(pal::METAL_WHITE, light, FACE_R),
        color::crown_lit(light),
        light.night.clamp(0.0, 1.0) * 0.75,
    );
    sprite.on_surface(render::snow::SLOPE_MEDIUM);
    crate::geom::fill_polygon(sprite, &pyr_pts, cap_c, (z_top + 14) as u16);
    sprite.on_surface(render::snow::NONE);
    parts::spire_needle(sprite, light, mx, my, z_top + 14, 16, ox, oy);
}

/// 10. Dark Bronze Corporate Tower (1×1, 36 storeys, vertical copper fins, retail lobby)
pub fn draw_dark_bronze_corporate(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let (w, d) = (1.20f32, 1.10f32);
    let v = spec.variant % 4;
    let z_top = match v {
        0 => 150, // 36 storeys
        1 => 185, // 44 storeys (+8 storeys!)
        2 => 135, // 32 storeys
        _ => 165, // 40 storeys
    };

    parts::building_shadow(sprite, 0.0, 0.0, w, d, z_top, ox, oy);

    let (lf, rf) = box_faces(0.0, 0.0, w, d, 0, z_top, ox, oy);
    let copper_c = pal::ROOF_COPPER_NEW;

    // B99: panes with floors and night-lit windows under the copper fins; the
    // faces were one flat bronze, a blank slab by day and dark at night.
    parts::paint_glass_curtain(&lf, sprite, light, 8, 5, FACE_L, spec.seed + 5);
    parts::paint_glass_curtain(&rf, sprite, light, 8, 5, FACE_R, spec.seed);
    for face in [&rf, &lf] {
        face.paint(sprite, &mut |u, v_win, _x, _y| {
            if (u % 5) == 0 {
                Some((color::surf(copper_c, light, 1.20), (z_top - v_win) as u16))
            } else {
                None
            }
        });
    }
    parts::corner_seam(sprite, light, w, d, 0, z_top, copper_c, ox, oy);

    parts::roof_flat(sprite, light, 0.0, 0.0, w, d, z_top, 4, pal::METAL_STEEL_DARK, ox, oy, 0.0);
    parts::rooftop_hvac(sprite, light, 0.3, 0.3, z_top, ox, oy);
}

/// 11. White Lattice Crown Tower (2×2, 50 storeys, circular open lattice crown)
pub fn draw_white_lattice_crown(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let (w, d) = (2.00f32, 1.80f32);
    let v = spec.variant % 4;
    let (z_shaft, z_crown) = match v {
        0 => (190, 215), // 50 storeys
        1 => (220, 248), // 58 storeys (+8 storeys!)
        2 => (160, 182), // 42 storeys
        _ => (200, 226), // 52 storeys
    };

    parts::building_shadow(sprite, 0.0, 0.0, w, d, z_crown, ox, oy);

    let (lf, rf) = box_faces(0.0, 0.0, w, d, 0, z_shaft, ox, oy);
    let marble_c = pal::STONE_LIMESTONE;
    parts::paint_stone_ashlar(&lf, sprite, light, marble_c, FACE_L, spec.age);
    parts::paint_stone_ashlar(&rf, sprite, light, marble_c, FACE_R, spec.age);
    parts::corner_seam(sprite, light, w, d, 0, z_shaft, marble_c, ox, oy);

    let shaft_top = box_top(0.0, 0.0, w, d, z_shaft, ox, oy);
    shaft_top.fill(sprite, color::surf(pal::STONE_GRANITE, light, FACE_TOP));

    // Circular open-lattice architectural crown
    let (c_lf, c_rf) = box_faces(0.2, 0.2, w - 0.2, d - 0.2, z_shaft, z_crown, ox, oy);
    c_rf.paint(sprite, &mut |u, v_win, _x, _y| {
        let is_lattice = (u % 4 == 0) || (v_win % 4 == 0);
        let depth = (z_crown - v_win) as u16;
        if is_lattice {
            let c = if light.night > 0.05 {
                color::mix(color::surf(pal::METAL_WHITE, light, FACE_R), color::crown_lit(light), light.night)
            } else {
                color::surf(pal::METAL_WHITE, light, FACE_R)
            };
            Some((c, depth))
        } else {
            None // Open lattice cutout!
        }
    });
    c_lf.paint(sprite, &mut |u, v_win, _x, _y| {
        let is_lattice = (u % 4 == 0) || (v_win % 4 == 0);
        let depth = (z_crown - v_win) as u16;
        if is_lattice {
            let c = if light.night > 0.05 {
                color::mix(color::surf(pal::METAL_WHITE, light, FACE_R), color::crown_lit(light), light.night)
            } else {
                color::surf(pal::METAL_WHITE, light, FACE_R)
            };
            Some((c, depth))
        } else {
            None
        }
    });

    let crown_top = box_top(0.2, 0.2, w - 0.2, d - 0.2, z_crown, ox, oy);
    crown_top.fill(sprite, color::surf(pal::METAL_WHITE, light, FACE_TOP));
    parts::spire_needle(sprite, light, w / 2.0, d / 2.0, z_crown, 18, ox, oy);
}

/// 12. Silver Mirror Blade Tower (Slender 1×2 slab, 44 storeys, illuminated blue LED lines)
pub fn draw_silver_mirror_blade(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let (w, d) = (1.10f32, 1.85f32);
    let v = spec.variant % 4;
    let z_top = match v {
        0 => 180, // 44 storeys
        1 => 215, // 52 storeys (+8 storeys!)
        2 => 155, // 38 storeys
        _ => 195, // 48 storeys
    };

    parts::building_shadow(sprite, 0.0, 0.0, w, d, z_top, ox, oy);

    let (lf, rf) = box_faces(0.0, 0.0, w, d, 0, z_top, ox, oy);
    parts::paint_glass_curtain(&lf, sprite, light, 7, 6, FACE_L, spec.seed + 33);
    parts::paint_glass_curtain(&rf, sprite, light, 7, 6, FACE_R, spec.seed);
    parts::corner_seam(sprite, light, w, d, 0, z_top, pal::GLASS_REFLECTIVE, ox, oy);

    // Accent light up the building's corner, brought up with the evening (B99d)
    if light.night > 0.05 {
        let (e0_x, e0_y) = crate::geom::project(w, d, 0.0, ox, oy);
        let (e1_x, e1_y) = crate::geom::project(w, d, z_top as f32, ox, oy);
        crate::geom::draw_line(sprite, e0_x.round() as i32, e0_y.round() as i32, e1_x.round() as i32, e1_y.round() as i32, color::crown_lit(light), z_top as u16);
    }

    parts::roof_flat(sprite, light, 0.0, 0.0, w, d, z_top, 4, pal::METAL_STEEL_DARK, ox, oy, 0.0);
    parts::spire_needle(sprite, light, w / 2.0, d / 2.0, z_top, 20, ox, oy);
}

/// 13. Black Granite Spire (Slender 1×1, 40 storeys, black granite + gilded gold pinnacles)
pub fn draw_black_granite_spire(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let (w, d) = (1.20f32, 1.10f32);
    let v = spec.variant % 4;
    let (z_shaft, z_crown) = match v {
        0 => (145, 168), // 40 storeys
        1 => (175, 202), // 48 storeys (+8 storeys!)
        2 => (130, 150), // 36 storeys
        _ => (160, 184), // 44 storeys
    };

    parts::building_shadow(sprite, 0.0, 0.0, w, d, z_crown, ox, oy);

    let (lf, rf) = box_faces(0.0, 0.0, w, d, 0, z_shaft, ox, oy);
    let dark_stone = pal::STONE_DARK_SLATE;
    parts::paint_stone_ashlar(&lf, sprite, light, dark_stone, FACE_L, spec.age);
    parts::paint_stone_ashlar(&rf, sprite, light, dark_stone, FACE_R, spec.age);
    parts::corner_seam(sprite, light, w, d, 0, z_shaft, color::GOLD_SIGN, ox, oy);

    // Gilded gold window tracery and spandrels
    let len = rf.top_len();
    for floor in 0..(z_shaft / 12) {
        let v_win = 4 + floor * 11;
        let gq = rf.sub_px(4, v_win, len - 4, v_win + 2, rf.rows() as f32);
        gq.fill(sprite, color::surf(color::GOLD_SIGN, light, 1.15));
    }

    // Gothic lantern crown with corner pinnacles
    let (c_lf, c_rf) = box_faces(0.15, 0.15, w - 0.15, d - 0.15, z_shaft, z_crown, ox, oy);
    c_rf.fill(sprite, color::surf(dark_stone, light, FACE_R));
    c_lf.fill(sprite, color::surf(dark_stone, light, FACE_L));
    parts::corner_seam(sprite, light, w - 0.15, d - 0.15, z_shaft, z_crown, color::GOLD_SIGN, ox, oy);

    let top = box_top(0.15, 0.15, w - 0.15, d - 0.15, z_crown, ox, oy);
    top.fill(sprite, color::surf(pal::ROOF_COPPER_PATINA, light, FACE_TOP));

    // Corner gold pinnacles
    parts::spire_needle(sprite, light, 0.15, d - 0.15, z_crown, 10, ox, oy);
    parts::spire_needle(sprite, light, w - 0.15, d - 0.15, z_crown, 10, ox, oy);
    parts::spire_needle(sprite, light, w / 2.0, d / 2.0, z_crown, 18, ox, oy);
}

/// 14. Curved Glass Sail Tower (Slender 1×1, 35 storeys, sculptural sail silhouette)
pub fn draw_curved_glass_sail(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let (w, d) = (1.20f32, 1.10f32);
    let v = spec.variant % 4;
    let z_top = match v {
        0 => 145, // 35 storeys
        1 => 178, // 43 storeys (+8 storeys!)
        2 => 125, // 30 storeys
        _ => 158, // 38 storeys
    };

    parts::building_shadow(sprite, 0.0, 0.0, w, d, z_top, ox, oy);

    let (lf, rf) = box_faces(0.0, 0.0, w, d, 0, z_top, ox, oy);
    parts::paint_glass_curtain(&lf, sprite, light, 7, 6, FACE_L, spec.seed);
    parts::paint_glass_curtain(&rf, sprite, light, 7, 6, FACE_R, spec.seed + 11);
    parts::corner_seam(sprite, light, w, d, 0, z_top, pal::METAL_WHITE, ox, oy);

    // Convex arch crown curving upward like a billowing sail
    let mx = w / 2.0;
    let my = d / 2.0;
    let arch_pts = [
        crate::geom::project(0.0, d, z_top as f32, ox, oy),
        crate::geom::project(w, d, z_top as f32, ox, oy),
        crate::geom::project(mx, my, (z_top + 16) as f32, ox, oy),
    ];
    let crown_c = color::mix(
        color::surf(pal::METAL_WHITE, light, FACE_R),
        color::crown_lit(light),
        light.night.clamp(0.0, 1.0) * 0.75,
    );
    sprite.on_surface(render::snow::SLOPE_MEDIUM);
    crate::geom::fill_polygon(sprite, &arch_pts, crown_c, (z_top + 16) as u16);
    sprite.on_surface(render::snow::NONE);

    let top = box_top(0.0, 0.0, w, d, z_top, ox, oy);
    top.fill(sprite, color::surf(pal::METAL_STEEL_DARK, light, FACE_TOP));
    parts::spire_needle(sprite, light, mx, my, z_top + 16, 14, ox, oy);
}

/// 15. Hex Diagrid Tower (Slender 1×1, 36 storeys, diagrid skeleton)
pub fn draw_diagrid_hex(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let (w, d) = (1.20f32, 1.10f32);
    let v = spec.variant % 4;
    let z_top = match v {
        0 => 150, // 36 storeys
        1 => 182, // 44 storeys (+8 storeys!)
        2 => 130, // 31 storeys
        _ => 162, // 39 storeys
    };

    parts::building_shadow(sprite, 0.0, 0.0, w, d, z_top + 10, ox, oy);

    let (lf, rf) = box_faces(0.0, 0.0, w, d, 0, z_top, ox, oy);

    // Diamond diagrid exterior structural skeleton on both faces
    let steel_diag_c = pal::METAL_STEEL_DARK;
    rf.paint(sprite, &mut |u, v_win, _x, _y| {
        let is_diag_a = ((u + v_win) % 12) <= 1;
        let is_diag_b = ((u as i32 - v_win as i32).abs() % 12) <= 1;
        let depth = (z_top - v_win) as u16;
        if is_diag_a || is_diag_b {
            return Some((color::surf(steel_diag_c, light, 1.25), depth));
        }
        if light.is_night {
            let lit = (color::hash_seed(spec.seed, (u / 6 + v_win / 6 * 31) as u32) % 100) < 40;
            let c = if lit { color::WIN_COOL } else { color::WIN_OFF_NIGHT };
            Some((c, depth))
        } else {
            Some((color::surf(pal::GLASS_BLUE_SKY, light, FACE_R), depth))
        }
    });

    lf.paint(sprite, &mut |u, v_win, _x, _y| {
        let is_diag_a = ((u + v_win) % 12) <= 1;
        let is_diag_b = ((u as i32 - v_win as i32).abs() % 12) <= 1;
        let depth = (z_top - v_win) as u16;
        if is_diag_a || is_diag_b {
            return Some((color::surf(steel_diag_c, light, 1.25), depth));
        }
        if light.is_night {
            let lit = (color::hash_seed(spec.seed, (u / 6 + v_win / 6 * 31 + 9) as u32) % 100) < 40;
            let c = if lit { color::WIN_COOL } else { color::WIN_OFF_NIGHT };
            Some((c, depth))
        } else {
            Some((color::surf(pal::GLASS_BLUE_SKY, light, FACE_L), depth))
        }
    });
    parts::corner_seam(sprite, light, w, d, 0, z_top, steel_diag_c, ox, oy);

    // Rounded skylight dome top
    let top = box_top(0.1, 0.1, w - 0.1, d - 0.1, z_top, ox, oy);
    top.fill(sprite, color::surf(pal::GLASS_REFLECTIVE, light, FACE_TOP));
    parts::spire_needle(sprite, light, w / 2.0, d / 2.0, z_top, 16, ox, oy);
}

/// 16. Helipad Headquarters Tower (Slender 1×2, 32 storeys, 3-storey granite podium, rooftop helipad)
pub fn draw_helipad_headquarters(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let (w, d) = (1.35f32, 1.85f32);
    let v = spec.variant % 4;
    let (z_podium, z_top) = match v {
        0 => (24, 140), // 32 storeys
        1 => (30, 172), // 40 storeys (+8 storeys!)
        2 => (20, 122), // 28 storeys
        _ => (26, 154), // 36 storeys
    };

    parts::building_shadow(sprite, 0.0, 0.0, w, d, z_top, ox, oy);

    // 3-storey granite retail podium
    let (p_lf, p_rf) = box_faces(0.0, 0.0, w, d, 0, z_podium, ox, oy);
    parts::paint_stone_ashlar(&p_lf, sprite, light, pal::STONE_GRANITE, FACE_L, spec.age);
    parts::paint_stone_ashlar(&p_rf, sprite, light, pal::STONE_GRANITE, FACE_R, spec.age);
    parts::corner_seam(sprite, light, w, d, 0, z_podium, pal::STONE_GRANITE, ox, oy);
    let p_top = box_top(0.0, 0.0, w, d, z_podium, ox, oy);
    p_top.fill(sprite, color::surf(pal::STONE_GRANITE, light, FACE_TOP));

    // Glass tower shaft inset slightly
    let (lf, rf) = box_faces(0.08, 0.08, w - 0.08, d - 0.08, z_podium, z_top, ox, oy);
    parts::paint_glass_curtain(&lf, sprite, light, 7, 6, FACE_L, spec.seed);
    parts::paint_glass_curtain(&rf, sprite, light, 7, 6, FACE_R, spec.seed + 15);
    parts::corner_seam(sprite, light, w - 0.08, d - 0.08, z_podium, z_top, pal::METAL_WHITE, ox, oy);

    // Rooftop helipad
    let top = box_top(0.08, 0.08, w - 0.08, d - 0.08, z_top, ox, oy);
    top.paint(sprite, &mut |u, v_top, _x, _y| {
        let is_border = u == 0 || v_top == 0 || u == 15 || v_top == 15;
        let is_h = (u >= 6 && u <= 9 && v_top >= 4 && v_top <= 11) && (u == 6 || u == 9 || v_top == 7 || v_top == 8);
        let c = if is_border {
            pal::METAL_WHITE
        } else if is_h {
            color::GOLD_SIGN
        } else {
            pal::ASPHALT
        };
        Some((color::surf(c, light, FACE_TOP), z_top as u16))
    });
}
