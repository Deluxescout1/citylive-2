//! Generic Landmarks of D59: Stadium, Train Station, Museum, Town Hall, Cathedral,
//! Signature Tower, Lighthouse, Container Port, Ferris Wheel, Cable-Stay Bridge Pylon.

use render::palette::rgb;
use crate::color::{self, pal, FACE_L, FACE_R, FACE_TOP};
use crate::geom::{self, box_faces, box_top, project};
use crate::parts::{self, WinOpts};
use crate::types::{BuildingSpec, Light, Sprite};

/// 1. Landmark Stadium: full oval bowl with tiers of colored seats, roof canopy, floodlight masts, lit pitch.
pub fn draw_stadium(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let (w, d) = (3.20f32, 3.20f32);
    let v = spec.variant % 4;
    let z_rim = match v {
        0 => 40,
        1 => 46, // modern canopy stadium
        2 => 38, // brick arched arcade stadium
        _ => 48, // indoor covered arena dome
    };
    let flood_h = z_rim + 22;

    parts::building_shadow(sprite, 0.0, 0.0, w, d, z_rim, ox, oy);

    // Solid concrete ground base slab across the full 3.2x3.2 footprint
    let (s_lf, s_rf) = box_faces(0.0, 0.0, w, d, 0, 3, ox, oy);
    s_lf.fill(sprite, color::surf(pal::CONCRETE_GREY, light, FACE_L));
    s_rf.fill(sprite, color::surf(pal::CONCRETE_GREY, light, FACE_R));
    let base_slab = box_top(0.0, 0.0, w, d, 3, ox, oy);
    base_slab.fill(sprite, color::surf(pal::CONCRETE_LIGHT, light, FACE_TOP));

    // Outer architectural stadium bowl facade
    let (lf, rf) = box_faces(0.1, 0.1, w - 0.1, d - 0.1, 3, z_rim, ox, oy);
    let conc_c = match v {
        2 => pal::BRICK_RED,
        _ => pal::CONCRETE_LIGHT,
    };
    parts::paint_stucco(&lf, sprite, light, conc_c, FACE_L, spec.age);

    // Outer architectural rib colonnade with entrance portals
    rf.paint(sprite, &mut |u, v_win, x, y| {
        let is_rib = (u % 7) == 0;
        let is_portal = v_win >= z_rim - 8 && (u % 7 >= 2 && u % 7 <= 5);
        let depth = (z_rim - v_win) as u16;
        if is_portal {
            Some((color::surf(pal::METAL_STEEL_DARK, light, FACE_R), depth))
        } else if is_rib {
            Some((color::surf(pal::METAL_WHITE, light, 1.15), depth))
        } else {
            let grain = color::jcell(x, y, 4, 0.02);
            Some((color::surf(color::shade(conc_c, grain), light, FACE_R), depth))
        }
    });
    parts::corner_seam(sprite, light, w - 0.1, d - 0.1, 3, z_rim, pal::METAL_WHITE, ox, oy);

    if v == 3 {
        // Covered indoor arena dome roof
        let (d_lf, d_rf) = box_faces(0.25, 0.25, w - 0.25, d - 0.25, z_rim, z_rim + 10, ox, oy);
        d_lf.fill(sprite, color::surf(pal::METAL_STEEL_DARK, light, FACE_L));
        d_rf.fill(sprite, color::surf(pal::METAL_STEEL_DARK, light, FACE_R));
        let dome = box_top(0.25, 0.25, w - 0.25, d - 0.25, z_rim + 10, ox, oy);
        dome.fill(sprite, color::surf(pal::METAL_STEEL_DARK, light, FACE_TOP));
    } else {
        // Tiers of stadium seats in bowl
        // Upper tier: red seats
        let t_upper = box_top(0.35, 0.35, w - 0.35, d - 0.35, z_rim - 6, ox, oy);
        t_upper.paint(sprite, &mut |u, _v, _x, _y| {
            let is_aisle = (u % 8) == 0;
            let c = if is_aisle { pal::CONCRETE_LIGHT } else { rgb(0xc4, 0x32, 0x38) }; // red seats
            Some((color::surf(c, light, FACE_TOP), (z_rim - 6) as u16))
        });

        // Lower tier: blue seats
        let t_lower = box_top(0.55, 0.55, w - 0.55, d - 0.55, z_rim - 14, ox, oy);
        t_lower.paint(sprite, &mut |u, _v, _x, _y| {
            let is_aisle = (u % 8) == 0;
            let c = if is_aisle { pal::CONCRETE_LIGHT } else { rgb(0x28, 0x60, 0xb8) }; // blue seats
            Some((color::surf(c, light, FACE_TOP), (z_rim - 14) as u16))
        });

        // Green playing pitch in the open center
        let pitch = box_top(0.80, 0.80, w - 0.80, d - 0.80, 12, ox, oy);
        pitch.paint(sprite, &mut |u, v_pitch, _x, _y| {
            let is_stripe = (u / 5) % 2 == 0;
            let mut c = if is_stripe { pal::GRASS_LUSH } else { color::shade(pal::GRASS_LUSH, 1.12) };
            // White field markings: center circle and touchlines
            let is_field_line = (u == 2 || u == 22 || v_pitch == 2 || v_pitch == 16) || (u == 12);
            if is_field_line {
                c = pal::TRIM_WHITE;
            }
            if light.is_night {
                // Pitch brightly illuminated under night floodlights!
                c = color::shade(c, 1.35);
            }
            Some((color::surf(c, light, FACE_TOP), 12))
        });

        // Suspended cantilevered roof canopy covering the upper seats
        let (c_lf, c_rf) = box_faces(0.1, 0.1, w - 0.1, d - 0.1, z_rim, z_rim + 3, ox, oy);
        c_lf.fill(sprite, color::surf(pal::METAL_WHITE, light, FACE_L));
        c_rf.fill(sprite, color::surf(pal::METAL_WHITE, light, FACE_R));

        // 4 Corner floodlight masts
        let corners = [
            (0.15f32, 0.15f32),
            (w - 0.15f32, 0.15f32),
            (0.15f32, d - 0.15f32),
            (w - 0.15f32, d - 0.15f32),
        ];
        for &(cx, cy) in &corners {
            let (p0_x, p0_y) = project(cx, cy, z_rim as f32, ox, oy);
            let (p1_x, p1_y) = project(cx, cy, flood_h as f32, ox, oy);
            geom::draw_line(sprite, p0_x.round() as i32, p0_y.round() as i32, p1_x.round() as i32, p1_y.round() as i32, pal::METAL_STEEL_DARK, flood_h as u16);
            let light_c = if light.is_night { color::LAMP_AMBER } else { pal::METAL_WHITE };
            let fx = p1_x.round() as i32;
            let fy = p1_y.round() as i32;
            sprite.set_pixel(fx, fy, light_c, flood_h as u16);
            sprite.set_pixel(fx - 1, fy, light_c, flood_h as u16);
            sprite.set_pixel(fx + 1, fy, light_c, flood_h as u16);
            sprite.set_pixel(fx, fy - 1, light_c, flood_h as u16);
        }
    }
}

/// 2. Landmark Train Station: grand Beaux-Arts terminal concourse + vaulted iron-and-glass platform shed.
pub fn draw_train_station(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let (w, d) = (3.00f32, 2.00f32);
    let v = spec.variant % 4;
    let z_facade = match v {
        0 => 48,
        1 => 58, // Beaux-Arts grand terminal with twin towers
        2 => 50, // Victorian Gothic railway station
        _ => 42, // modern glass canopy terminal
    };
    let shed_d = 1.15f32;

    parts::building_shadow(sprite, 0.0, 0.0, w, d, z_facade, ox, oy);

    // Rear vaulted train shed (glass and steel arch) spanning y=0..shed_d
    let shed_top = box_top(0.2, 0.0, w - 0.2, shed_d, 32, ox, oy);
    shed_top.paint(sprite, &mut |u, _v, _x, _y| {
        let is_arch_rib = (u % 5) == 0;
        let c = if is_arch_rib { pal::METAL_STEEL_DARK } else { pal::GLASS_BLUE_SKY };
        Some((color::surf(c, light, FACE_TOP), 32))
    });

    // Monumental front station concourse spanning the full footprint (0,0) to (w,d)
    let (lf, rf) = box_faces(0.0, 0.0, w, d, 0, z_facade, ox, oy);
    let stone_c = match v {
        2 => pal::BRICK_RED,
        _ => pal::STONE_LIMESTONE,
    };
    parts::paint_stone_ashlar(&lf, sprite, light, stone_c, FACE_L, spec.age);
    parts::paint_stone_ashlar(&rf, sprite, light, stone_c, FACE_R, spec.age);
    parts::corner_seam(sprite, light, w, d, 0, z_facade, pal::STONE_GRANITE, ox, oy);

    // Concourse solid flat roof deck (closes the entire concourse volume)
    parts::roof_flat(sprite, light, 0.0, 0.0, w, d, z_facade, 5, pal::STONE_GRANITE, ox, oy, spec.age);

    // Massive arched thermal glass window in center of front facade
    let len = rf.top_len();
    let arch_q = rf.sub_px(len / 2 - 16, 4, len / 2 + 16, 32, rf.rows() as f32);
    arch_q.paint(sprite, &mut |u, v_arch, _x, _y| {
        let is_mullion = (u % 4) == 0 || (v_arch % 5) == 0;
        let c = if is_mullion {
            pal::METAL_STEEL_DARK
        } else if light.is_night {
            color::WIN_WARM
        } else {
            pal::GLASS_REFLECTIVE
        };
        Some((color::surf(c, light, FACE_R), (z_facade - v_arch) as u16))
    });

    // Central clock above arched entrance
    let (cx, cy) = project(w / 2.0, d, (z_facade - 2) as f32, ox, oy);
    let clock_c = if light.is_night { color::WIN_WARM } else { pal::TRIM_WHITE };
    for dy in -2..=2 {
        for dx in -2..=2 {
            if dx * dx + dy * dy <= 4 {
                sprite.set_pixel(cx.round() as i32 + dx, cy.round() as i32 + dy, clock_c, z_facade as u16);
            }
        }
    }

    // Grand entrance arcade with taxi canopy
    let can = box_top(0.3, d, w - 0.3, d + 0.35, 14, ox, oy);
    can.fill(sprite, color::surf(pal::METAL_STEEL_DARK, light, FACE_TOP));

    // Left face arched windows
    let len_l = lf.top_len();
    let wo = WinOpts {
        w: 6,
        h: 9,
        frame: pal::STONE_LIMESTONE,
        sill: true,
        lintel: true,
        shutters: false,
        shutter_c: pal::SHUTTER_BLACK,
        seed: spec.seed,
        lit_p: 0.70,
    };
    parts::draw_window(&lf, sprite, light, 6, 8, wo, z_facade);
    parts::draw_window(&lf, sprite, light, len_l - 14, 8, wo, z_facade);
}

/// 3. Landmark Museum: neoclassical temple with 3D projecting portico, 6 freestanding columns, steps, and dome.
pub fn draw_museum(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let (w, d) = (3.00f32, 2.00f32);
    let v = spec.variant % 4;
    let (z_wall, dome_h) = match v {
        0 => (44, 24),
        1 => (50, 28), // grand double-wing museum + tall dome
        2 => (38, 18), // modern cubic wing
        _ => (46, 22),
    };

    parts::building_shadow(sprite, 0.0, 0.0, w, d, z_wall + dome_h, ox, oy);

    // Monumental stone entrance stylobate staircase on the ground
    let (s_lf, s_rf) = box_faces(0.0, 0.0, w, d, 0, 4, ox, oy);
    s_lf.fill(sprite, color::surf(pal::STONE_GRANITE, light, FACE_L));
    s_rf.fill(sprite, color::surf(pal::STONE_GRANITE, light, FACE_R));
    let terrace = box_top(0.0, 0.0, w, d, 4, ox, oy);
    terrace.fill(sprite, color::surf(pal::STONE_LIMESTONE, light, FACE_TOP));

    // Main museum hall walls (from 0,0 to w, d-0.35)
    let stone_c = pal::STONE_LIMESTONE;
    let (lf, rf) = box_faces(0.1, 0.1, w - 0.1, d - 0.35, 4, z_wall, ox, oy);
    parts::paint_stone_ashlar(&lf, sprite, light, stone_c, FACE_L, spec.age);
    parts::paint_stone_ashlar(&rf, sprite, light, stone_c, FACE_R, spec.age);
    parts::corner_seam(sprite, light, w - 0.1, d - 0.35, 4, z_wall, pal::STONE_GRANITE, ox, oy);

    // Solid closed roof deck over entire museum hall
    let museum_roof = box_top(0.1, 0.1, w - 0.1, d - 0.35, z_wall, ox, oy);
    museum_roof.fill(sprite, color::surf(pal::STONE_GRANITE, light, FACE_TOP));

    // 3D Projecting Temple Portico extending forward by 0.35 tiles
    let (p_x0, p_x1) = (w * 0.20, w * 0.80);
    let (p_y0, p_y1) = (d - 0.35, d);

    // 6 Freestanding 3D stone columns standing in front of the recessed entrance wall
    for col in 0..6 {
        let cx = p_x0 + 0.04 + col as f32 * (p_x1 - p_x0 - 0.22) / 5.0;
        let cy = p_y1 - 0.15;
        let (col_lf, col_rf) = box_faces(cx, cy, cx + 0.14, cy + 0.14, 4, z_wall - 4, ox, oy);
        col_lf.fill(sprite, color::surf(pal::STONE_GRANITE, light, FACE_L));
        col_rf.fill(sprite, color::surf(color::shade(pal::STONE_GRANITE, 1.15), light, FACE_R));
        parts::corner_seam(sprite, light, cx + 0.14, cy + 0.14, 4, z_wall - 4, pal::STONE_LIMESTONE, ox, oy);
    }

    // Heavy classical entablature beam resting ON TOP of the 6 columns
    let (ent_lf, ent_rf) = box_faces(p_x0, p_y0, p_x1, p_y1, z_wall - 4, z_wall, ox, oy);
    ent_lf.fill(sprite, color::surf(stone_c, light, FACE_L));
    ent_rf.fill(sprite, color::surf(stone_c, light, FACE_R));
    let ent_top = box_top(p_x0, p_y0, p_x1, p_y1, z_wall, ox, oy);
    ent_top.fill(sprite, color::surf(pal::STONE_GRANITE, light, FACE_TOP));

    // Projecting triangular pediment with relief sculptures
    let ped_pts = [
        project(p_x0, p_y1, z_wall as f32, ox, oy),
        project(p_x1, p_y1, z_wall as f32, ox, oy),
        project((p_x0 + p_x1) / 2.0, p_y1, (z_wall + 14) as f32, ox, oy),
    ];
    geom::fill_polygon(sprite, &ped_pts, color::surf(pal::STONE_LIMESTONE, light, FACE_R), (z_wall + 14) as u16);

    // Central classical octagonal drum and copper/glass dome
    let (dome_x, dome_y, dw, dd) = (w / 2.0 - 0.50, d / 2.0 - 0.50, 1.00, 1.00);
    let (d_lf, d_rf) = box_faces(dome_x, dome_y, dome_x + dw, dome_y + dd, z_wall, z_wall + 8, ox, oy);
    d_lf.fill(sprite, color::surf(pal::STONE_LIMESTONE, light, FACE_L));
    d_rf.fill(sprite, color::surf(pal::STONE_LIMESTONE, light, FACE_R));
    let drum_top = box_top(dome_x, dome_y, dome_x + dw, dome_y + dd, z_wall + 8, ox, oy);
    drum_top.fill(sprite, color::surf(pal::ROOF_COPPER_PATINA, light, FACE_TOP));

    // Copper dome cap
    let mx = w / 2.0;
    let my = d / 2.0;
    let z_dome_apex = z_wall + dome_h;
    let dome_l = [
        project(dome_x, dome_y + dd, (z_wall + 8) as f32, ox, oy),
        project(dome_x + dw, dome_y + dd, (z_wall + 8) as f32, ox, oy),
        project(mx, my, z_dome_apex as f32, ox, oy),
    ];
    let dome_r = [
        project(dome_x + dw, dome_y, (z_wall + 8) as f32, ox, oy),
        project(dome_x + dw, dome_y + dd, (z_wall + 8) as f32, ox, oy),
        project(mx, my, z_dome_apex as f32, ox, oy),
    ];
    sprite.on_surface(render::snow::SLOPE_MEDIUM);
    geom::fill_polygon(sprite, &dome_l, color::surf(pal::ROOF_COPPER_PATINA, light, FACE_L), z_dome_apex as u16);
    geom::fill_polygon(sprite, &dome_r, color::surf(color::shade(pal::ROOF_COPPER_PATINA, 0.82), light, FACE_R), z_dome_apex as u16);
    sprite.on_surface(render::snow::NONE);
    parts::spire_needle(sprite, light, mx, my, z_dome_apex, 10, ox, oy);

    // Grand entrance portal with bronze double doors on recessed wall
    parts::draw_door(&rf, sprite, light, rf.top_len() / 2 - 5, z_wall - 14, 10, 14, pal::METAL_CHAMPAGNE, pal::STONE_GRANITE, 4);
}

/// 4. Landmark Town Hall (D59)
pub fn draw_town_hall(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    crate::catalogue::civic::draw_town_hall(spec, light, sprite, ox, oy);
}

/// 5. Landmark Cathedral: cruciform nave + transept, twin Gothic western bell towers, flying buttresses, rose window, soaring steeple spire.
pub fn draw_cathedral(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let (w, d) = (3.20f32, 2.20f32);
    let v = spec.variant % 4;
    let (z_nave, tower_h, steeple_h) = match v {
        0 => (44, 74, 50), // twin 74px towers + 94px crossing spire
        1 => (50, 84, 60), // twin 84px towers + 110px spire
        2 => (40, 68, 44),
        _ => (46, 78, 54),
    };

    parts::building_shadow(sprite, 0.0, 0.0, w, d, z_nave + steeple_h, ox, oy);

    // Stone foundation base
    let (b_lf, b_rf) = box_faces(0.0, 0.0, w, d, 0, 3, ox, oy);
    b_lf.fill(sprite, color::surf(pal::STONE_GRANITE, light, FACE_L));
    b_rf.fill(sprite, color::surf(pal::STONE_GRANITE, light, FACE_R));
    let base_pad = box_top(0.0, 0.0, w, d, 3, ox, oy);
    base_pad.fill(sprite, color::surf(pal::STONE_LIMESTONE, light, FACE_TOP));

    // Main cruciform nave walls (from 0.0 to w - 0.45)
    let nave_w = w - 0.45;
    let stone_c = pal::STONE_LIMESTONE;
    let (lf, rf) = box_faces(0.1, 0.1, nave_w, d - 0.1, 3, z_nave, ox, oy);
    parts::paint_stone_ashlar(&lf, sprite, light, stone_c, FACE_L, spec.age);
    parts::paint_stone_ashlar(&rf, sprite, light, stone_c, FACE_R, spec.age);
    parts::corner_seam(sprite, light, nave_w, d - 0.1, 3, z_nave, pal::STONE_GRANITE, ox, oy);

    // Steep pitched slate roof over nave
    let my = d / 2.0;
    parts::draw_gable_end(sprite, light, project(0.1, 0.1, z_nave as f32, ox, oy), project(0.1, d - 0.1, z_nave as f32, ox, oy), project(0.1, my, (z_nave + 18) as f32, ox, oy), stone_c, FACE_L, z_nave, z_nave + 18);
    parts::draw_gable_end(sprite, light, project(nave_w, 0.1, z_nave as f32, ox, oy), project(nave_w, d - 0.1, z_nave as f32, ox, oy), project(nave_w, my, (z_nave + 18) as f32, ox, oy), stone_c, FACE_R, z_nave, z_nave + 18);
    parts::roof_gable(sprite, light, 0.05, 0.05, nave_w + 0.05, d - 0.05, z_nave, 18, pal::ROOF_SLATE_GREY, ox, oy, spec.age);

    // 3D Flying buttresses with stone pinnacle piers along the side nave walls
    for col in 0..4 {
        let px = 0.3 + col as f32 * (nave_w - 0.6) / 3.0;
        let py = d + 0.10;
        // Buttress ground pier
        let (p_lf, p_rf) = box_faces(px, py - 0.15, px + 0.15, py, 3, z_nave - 12, ox, oy);
        p_lf.fill(sprite, color::surf(pal::STONE_GRANITE, light, FACE_L));
        p_rf.fill(sprite, color::surf(pal::STONE_GRANITE, light, FACE_R));
        // Sloped flying arch connecting to clerestory wall
        let (a0_x, a0_y) = project(px + 0.07, py - 0.07, (z_nave - 12) as f32, ox, oy);
        let (a1_x, a1_y) = project(px + 0.07, d - 0.10, (z_nave - 4) as f32, ox, oy);
        geom::draw_line(sprite, a0_x.round() as i32, a0_y.round() as i32, a1_x.round() as i32, a1_y.round() as i32, color::surf(stone_c, light, 1.1), (z_nave - 4) as u16);
    }

    // TWIN GOTHIC WESTERN BELL TOWERS on the front facade!
    // Tower 1 (Front corner)
    let (t1_lf, t1_rf) = box_faces(w - 0.50, d - 0.55, w, d, 3, tower_h, ox, oy);
    parts::paint_stone_ashlar(&t1_lf, sprite, light, stone_c, FACE_L, spec.age);
    parts::paint_stone_ashlar(&t1_rf, sprite, light, stone_c, FACE_R, spec.age);
    parts::corner_seam(sprite, light, w, d, 3, tower_h, pal::STONE_GRANITE, ox, oy);
    let t1_top = box_top(w - 0.50, d - 0.55, w, d, tower_h, ox, oy);
    t1_top.fill(sprite, color::surf(pal::ROOF_SLATE_GREY, light, FACE_TOP));
    parts::spire_needle(sprite, light, w - 0.25, d - 0.27, tower_h, 14, ox, oy);

    // Tower 2 (Rear corner)
    let (t2_lf, t2_rf) = box_faces(w - 0.50, 0.0, w, 0.55, 3, tower_h, ox, oy);
    parts::paint_stone_ashlar(&t2_lf, sprite, light, stone_c, FACE_L, spec.age);
    parts::paint_stone_ashlar(&t2_rf, sprite, light, stone_c, FACE_R, spec.age);
    parts::corner_seam(sprite, light, w, 0.55, 3, tower_h, pal::STONE_GRANITE, ox, oy);
    let t2_top = box_top(w - 0.50, 0.0, w, 0.55, tower_h, ox, oy);
    t2_top.fill(sprite, color::surf(pal::ROOF_SLATE_GREY, light, FACE_TOP));
    parts::spire_needle(sprite, light, w - 0.25, 0.27, tower_h, 14, ox, oy);

    // Central facade between the twin towers with grand Gothic portal and Rose Window
    let (cf_lf, cf_rf) = box_faces(w - 0.45, 0.55, w - 0.05, d - 0.55, 3, z_nave + 4, ox, oy);
    parts::paint_stone_ashlar(&cf_lf, sprite, light, stone_c, FACE_L, spec.age);
    parts::paint_stone_ashlar(&cf_rf, sprite, light, stone_c, FACE_R, spec.age);

    // Stained glass Rose Window in center
    let (rose_x, rose_y) = project(w - 0.05, my, (z_nave - 8) as f32, ox, oy);
    let rose_c = if light.is_night { color::WIN_WARM } else { rgb(0x40, 0x86, 0xd4) };
    for dy in -4..=4 {
        for dx in -4..=4 {
            if dx * dx + dy * dy <= 12 {
                sprite.set_pixel(rose_x.round() as i32 + dx, rose_y.round() as i32 + dy, rose_c, (z_nave - 8) as u16);
            }
        }
    }

    // Triple pointed-arch Gothic portal entrance
    parts::draw_door(&cf_rf, sprite, light, cf_rf.top_len() / 2 - 5, z_nave - 14, 10, 14, pal::STONE_DARK_SLATE, pal::STONE_GRANITE, 3);

    // Soaring central crossing steeple tower and needle spire
    let (st_x, st_y) = (nave_w * 0.50, d * 0.35);
    let (s_lf, s_rf) = box_faces(st_x, st_y, st_x + 0.60, st_y + 0.60, z_nave + 10, z_nave + 28, ox, oy);
    s_lf.fill(sprite, color::surf(stone_c, light, FACE_L));
    s_rf.fill(sprite, color::surf(stone_c, light, FACE_R));
    let s_top = box_top(st_x, st_y, st_x + 0.60, st_y + 0.60, z_nave + 28, ox, oy);
    s_top.fill(sprite, color::surf(pal::ROOF_SLATE_GREY, light, FACE_TOP));
    parts::spire_needle(sprite, light, st_x + 0.30, st_y + 0.30, z_nave + 28, steeple_h, ox, oy);
}

/// 6. Signature Observation Tower: the tallest skyscraper (65+ storeys, twisting crystalline glass, sky lounge, spire).
pub fn draw_signature_tower(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let (w, d) = (2.00f32, 1.80f32);
    let v = spec.variant % 4;
    let (z1, z2, z3, spire_h) = match v {
        0 => (105, 190, 245, 36), // 65 storeys (281px total!)
        1 => (120, 215, 275, 42), // 72 storeys mega-tall (+30px!)
        2 => (90, 165, 215, 32),
        _ => (110, 200, 255, 38),
    };

    parts::building_shadow(sprite, 0.0, 0.0, w, d, z3 + spire_h, ox, oy);

    // Tier 1: Double-height crystalline glass podium lobby (floors 1-25)
    let (lf1, rf1) = box_faces(0.0, 0.0, w, d, 0, z1, ox, oy);
    parts::paint_glass_curtain(&lf1, sprite, light, 7, 6, FACE_L, spec.seed);
    parts::paint_glass_curtain(&rf1, sprite, light, 7, 6, FACE_R, spec.seed + 1);
    parts::corner_seam(sprite, light, w, d, 0, z1, pal::METAL_WHITE, ox, oy);
    let t1 = box_top(0.0, 0.0, w, d, z1, ox, oy);
    t1.fill(sprite, color::surf(pal::METAL_STEEL_DARK, light, FACE_TOP));

    // Tier 2: Tapering faceted glass shaft (floors 25-50)
    let (lf2, rf2) = box_faces(0.20, 0.20, w - 0.20, d - 0.20, z1, z2, ox, oy);
    parts::paint_glass_curtain(&lf2, sprite, light, 7, 6, FACE_L, spec.seed + 2);
    parts::paint_glass_curtain(&rf2, sprite, light, 7, 6, FACE_R, spec.seed + 3);
    parts::corner_seam(sprite, light, w - 0.20, d - 0.20, z1, z2, pal::METAL_WHITE, ox, oy);
    let t2 = box_top(0.20, 0.20, w - 0.20, d - 0.20, z2, ox, oy);
    t2.fill(sprite, color::surf(pal::METAL_STEEL_DARK, light, FACE_TOP));

    // Tier 3: Skybridge observation deck & revolving sky lounge (floors 50-60)
    let (lf3, rf3) = box_faces(0.38, 0.38, w - 0.38, d - 0.38, z2, z3, ox, oy);
    // Illuminated observation disc windows
    rf3.paint(sprite, &mut |u, v_win, _x, _y| {
        let is_window = v_win >= 4 && v_win <= 14;
        let depth = (z3 - v_win) as u16;
        if is_window {
            if light.is_night {
                Some((color::WIN_WARM, depth)) // glowing golden sky lounge
            } else {
                let glint = (u % 4) == 0;
                let c = if glint { pal::GLASS_REFLECTIVE } else { pal::GLASS_DARK };
                Some((color::surf(c, light, FACE_R), depth))
            }
        } else {
            Some((color::surf(pal::METAL_WHITE, light, FACE_R), depth))
        }
    });
    lf3.paint(sprite, &mut |u, v_win, _x, _y| {
        let is_window = v_win >= 4 && v_win <= 14;
        let depth = (z3 - v_win) as u16;
        if is_window {
            if light.is_night {
                Some((color::WIN_WARM, depth))
            } else {
                let glint = (u % 4) == 0;
                let c = if glint { pal::GLASS_REFLECTIVE } else { pal::GLASS_DARK };
                Some((color::surf(c, light, FACE_L), depth))
            }
        } else {
            Some((color::surf(pal::METAL_WHITE, light, FACE_L), depth))
        }
    });
    parts::corner_seam(sprite, light, w - 0.38, d - 0.38, z2, z3, pal::METAL_WHITE, ox, oy);

    let t3 = box_top(0.38, 0.38, w - 0.38, d - 0.38, z3, ox, oy);
    t3.fill(sprite, color::surf(pal::METAL_WHITE, light, FACE_TOP));

    // Illuminated crown fins and soaring needle spire
    let mx = w / 2.0;
    let my = d / 2.0;
    parts::spire_needle(sprite, light, mx, my, z3, spire_h, ox, oy);
}

/// 7. Landmark Coastal Lighthouse: round tapering conical tower, gallery deck, rotating lantern beam.
pub fn draw_lighthouse(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let (w, d) = (2.00f32, 1.80f32);
    let v = spec.variant % 4;
    let (z_tower, lantern_h) = match v {
        0 => (68, 14), // conical red/white spiral
        1 => (78, 16), // tall black/white banded (+10px!)
        2 => (58, 14), // granite offshore caisson
        _ => (64, 14), // octagonal brick tower
    };

    parts::building_shadow(sprite, 0.0, 0.0, w, d, z_tower + lantern_h, ox, oy);

    // Solid ground rock / foundation slab (firmly grounded!)
    let (b_lf, b_rf) = box_faces(0.0, 0.0, w, d, 0, 3, ox, oy);
    b_lf.fill(sprite, color::surf(pal::STONE_DARK_SLATE, light, FACE_L));
    b_rf.fill(sprite, color::surf(pal::STONE_DARK_SLATE, light, FACE_R));
    let base_pad = box_top(0.0, 0.0, w, d, 3, ox, oy);
    base_pad.fill(sprite, color::surf(pal::CONCRETE_GREY, light, FACE_TOP));

    // Attached keeper's cottage at the base (left side)
    let (c_lf, c_rf) = box_faces(0.1, 0.6, 0.95, 1.5, 3, 20, ox, oy);
    parts::paint_clapboard(&c_lf, sprite, light, pal::CLAP_WHITE, FACE_L, spec.age);
    parts::paint_clapboard(&c_rf, sprite, light, pal::CLAP_WHITE, FACE_R, spec.age);
    parts::roof_gable(sprite, light, 0.05, 0.55, 1.00, 1.55, 20, 10, pal::ROOF_CEDAR_SHAKE, ox, oy, spec.age);

    // Round tapering conical lighthouse tower
    let (cx, cy) = (1.40f32, 0.80f32);
    let r_base = 0.44f32;
    let r_top = 0.28f32;

    let band_c = match v {
        0 => rgb(0xd4, 0x2e, 0x2e), // red spiral
        1 => pal::SHUTTER_BLACK, // black spiral
        2 => pal::STONE_GRANITE, // granite ashlar
        _ => pal::BRICK_RED, // red brick
    };

    // Render tapering conical tower scanlines from z=3 to z=z_tower
    for z in 3..z_tower {
        let t = (z - 3) as f32 / (z_tower - 3) as f32;
        let r = r_base + (r_top - r_base) * t;

        let center_sx = ox + (cx - cy) * 32.0;
        let center_sy = oy + (cx + cy) * 16.0 - z as f32;
        let rx_px = r * 32.0 * 1.4142;
        let ry_px = r * 16.0 * 1.4142;

        let x0 = (center_sx - rx_px).floor() as i32;
        let x1 = (center_sx + rx_px).ceil() as i32;

        for x in x0..=x1 {
            let u_norm = ((x as f32 - center_sx) / rx_px).clamp(-1.0, 1.0);
            let curve_y = (1.0 - u_norm * u_norm).max(0.0).sqrt() * ry_px;
            let y_start = (center_sy - curve_y * 0.4).round() as i32;
            let y_end = (center_sy + curve_y).round() as i32;

            let diffuse = (0.90 - 0.32 * u_norm).clamp(0.55, 1.25);
            let spiral_phase = u_norm * 0.9 + (z as f32 / 8.0);
            let is_band = (spiral_phase.floor() as i32 % 2) == 0;
            let base_c = if is_band { band_c } else { pal::CLAP_WHITE };
            let pixel_c = color::surf(color::shade(base_c, diffuse), light, 1.0);

            for y in y_start..=y_end {
                sprite.set_pixel(x, y, pixel_c, z as u16);
            }
        }
    }

    // Circular gallery walkway deck atop the tower
    let deck_r = r_top + 0.08;
    let deck_lf = box_faces(cx - deck_r, cy - deck_r, cx + deck_r, cy + deck_r, z_tower - 2, z_tower, ox, oy);
    deck_lf.0.fill(sprite, color::surf(pal::STONE_DARK_SLATE, light, FACE_L));
    deck_lf.1.fill(sprite, color::surf(pal::STONE_DARK_SLATE, light, FACE_R));
    let gallery_deck = box_top(cx - deck_r, cy - deck_r, cx + deck_r, cy + deck_r, z_tower, ox, oy);
    gallery_deck.fill(sprite, color::surf(pal::STONE_DARK_SLATE, light, FACE_TOP));

    // Gallery railing
    let (center_sx, center_sy) = (ox + (cx - cy) * 32.0, oy + (cx + cy) * 16.0 - z_tower as f32);
    let rail_c = color::surf(pal::WROUGHT_IRON, light, 1.0);
    for dx in -10..=10 {
        let x = (center_sx + dx as f32).round() as i32;
        let y = (center_sy + 4.0).round() as i32;
        sprite.set_pixel(x, y - 4, rail_c, z_tower as u16);
    }

    // Glass lantern room
    let l_r = r_top - 0.04;
    let (l_lf, l_rf) = box_faces(cx - l_r, cy - l_r, cx + l_r, cy + l_r, z_tower, z_tower + lantern_h, ox, oy);
    let lantern_c = if light.is_night { color::WIN_WARM } else { pal::GLASS_DARK };
    l_rf.fill(sprite, lantern_c);
    l_lf.fill(sprite, lantern_c);

    // Conical copper dome roof atop lantern
    let lantern_top = box_top(cx - l_r, cy - l_r, cx + l_r, cy + l_r, z_tower + lantern_h, ox, oy);
    lantern_top.fill(sprite, color::surf(pal::ROOF_COPPER_PATINA, light, FACE_TOP));
    parts::spire_needle(sprite, light, cx, cy, z_tower + lantern_h, 8, ox, oy);

    // Glowing rotating lantern beam at night
    if light.is_night {
        let (lx, ly) = project(cx, cy, (z_tower + lantern_h / 2) as f32, ox, oy);
        let beam_c = color::WIN_WARM;
        for dx in 1..=32 {
            let dy = -dx / 4;
            sprite.blend_pixel(lx.round() as i32 + dx, ly.round() as i32 + dy, beam_c, (z_tower + lantern_h) as u16, 0.55);
        }
    }
}

/// 8. Container Port Gantry Crane with tall yellow legs, overhead boom, container stacks 3-4 high.
pub fn draw_container_port(_spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let (w, d) = (3.00f32, 2.00f32);
    let gantry_h = 65;

    parts::building_shadow(sprite, 0.0, 0.0, w, d, gantry_h, ox, oy);

    // Concrete shipping wharf quay edge on the ground
    let (b_lf, b_rf) = box_faces(0.0, 0.0, w, d, 0, 4, ox, oy);
    b_lf.fill(sprite, color::surf(pal::CONCRETE_GREY, light, FACE_L));
    b_rf.fill(sprite, color::surf(color::shade(pal::CONCRETE_GREY, 0.85), light, FACE_R));
    let wharf = box_top(0.0, 0.0, w, d, 4, ox, oy);
    wharf.paint(sprite, &mut |_u, _v, x, y| {
        let grain = color::jcell(x, y, 3, 0.04);
        Some((color::surf(color::shade(pal::CONCRETE_LIGHT, grain), light, FACE_TOP), 4))
    });

    // Stacks of shipping containers 3-4 high in rows with aisles
    let container_colors = [
        rgb(0xdc, 0x28, 0x28), // K-Line red
        rgb(0x28, 0x6e, 0xba), // Maersk blue
        rgb(0x28, 0x8e, 0x44), // Evergreen green
        rgb(0xea, 0x6e, 0x22), // Hapag-Lloyd orange
        rgb(0x38, 0x9e, 0xba), // cyan
    ];

    let stacks = [
        (0.2f32, 0.2f32, 0, 3),
        (0.2f32, 0.6f32, 1, 4),
        (0.2f32, 1.0f32, 2, 3),
        (0.8f32, 0.2f32, 3, 4),
        (0.8f32, 0.6f32, 4, 3),
        (0.8f32, 1.0f32, 0, 4),
    ];

    for &(sx, sy, col_idx, height_tiers) in &stacks {
        for tier in 0..height_tiers {
            let z0 = 4 + tier * 7;
            let z1 = z0 + 7;
            let c = container_colors[(col_idx + tier as usize) % container_colors.len()];
            let (c_lf, c_rf) = box_faces(sx, sy, sx + 0.50, sy + 0.35, z0, z1, ox, oy);
            c_lf.fill(sprite, color::surf(c, light, FACE_L));
            c_rf.fill(sprite, color::surf(c, light, FACE_R));
            let c_top = box_top(sx, sy, sx + 0.50, sy + 0.35, z1, ox, oy);
            c_top.fill(sprite, color::surf(c, light, FACE_TOP));
        }
    }

    // Heavy industrial container gantry crane with structural box legs, deck, cabin, and boom
    let crane_x = w * 0.72;
    let crane_y = d * 0.50;
    let yellow_c = rgb(0xf4, 0xc4, 0x24);

    // 4 Structural box gantry legs
    let leg_pts = [
        (crane_x - 0.35, crane_y - 0.35),
        (crane_x + 0.35, crane_y - 0.35),
        (crane_x - 0.35, crane_y + 0.35),
        (crane_x + 0.35, crane_y + 0.35),
    ];
    for &(lx, ly) in &leg_pts {
        let (leg_lf, leg_rf) = box_faces(lx - 0.08, ly - 0.08, lx + 0.08, ly + 0.08, 4, 46, ox, oy);
        leg_lf.fill(sprite, color::surf(yellow_c, light, FACE_L));
        leg_rf.fill(sprite, color::surf(color::shade(yellow_c, 0.85), light, FACE_R));
        parts::corner_seam(sprite, light, lx + 0.08, ly + 0.08, 4, 46, yellow_c, ox, oy);
    }

    // Heavy machinery deck platform spanning the portal legs at z=46..52
    let (d_lf, d_rf) = box_faces(crane_x - 0.45, crane_y - 0.45, crane_x + 0.45, crane_y + 0.45, 46, 52, ox, oy);
    d_lf.fill(sprite, color::surf(yellow_c, light, FACE_L));
    d_rf.fill(sprite, color::surf(color::shade(yellow_c, 0.85), light, FACE_R));
    let d_top = box_top(crane_x - 0.45, crane_y - 0.45, crane_x + 0.45, crane_y + 0.45, 52, ox, oy);
    d_top.fill(sprite, color::surf(yellow_c, light, FACE_TOP));

    // Operator cabin suspended beneath the boom at z=40..46
    let (cab_lf, cab_rf) = box_faces(crane_x + 0.05, crane_y + 0.10, crane_x + 0.35, crane_y + 0.35, 40, 46, ox, oy);
    cab_lf.fill(sprite, color::surf(pal::GLASS_REFLECTIVE, light, FACE_L));
    cab_rf.fill(sprite, color::surf(pal::GLASS_REFLECTIVE, light, FACE_R));

    // Heavy 3D structural lattice boom girder extending over the water quay at z=52..58
    let (b_lf, b_rf) = box_faces(crane_x - 0.80, crane_y - 0.14, crane_x + 1.05, crane_y + 0.14, 52, 58, ox, oy);
    b_lf.fill(sprite, color::surf(yellow_c, light, FACE_L));
    b_rf.fill(sprite, color::surf(color::shade(yellow_c, 0.85), light, FACE_R));
    let b_top = box_top(crane_x - 0.80, crane_y - 0.14, crane_x + 1.05, crane_y + 0.14, 58, ox, oy);
    b_top.fill(sprite, color::surf(yellow_c, light, FACE_TOP));

    // A-frame tower peak above trolley deck with tension cables
    let (peak_x, peak_y) = project(crane_x, crane_y, (gantry_h + 8) as f32, ox, oy);
    let (boom_tip_x, boom_tip_y) = project(crane_x + 1.05, crane_y, 58.0, ox, oy);
    let (boom_back_x, boom_back_y) = project(crane_x - 0.80, crane_y, 58.0, ox, oy);
    geom::draw_line(sprite, peak_x.round() as i32, peak_y.round() as i32, boom_tip_x.round() as i32, boom_tip_y.round() as i32, pal::METAL_STEEL_DARK, (gantry_h + 8) as u16);
    geom::draw_line(sprite, peak_x.round() as i32, peak_y.round() as i32, boom_back_x.round() as i32, boom_back_y.round() as i32, pal::METAL_STEEL_DARK, (gantry_h + 8) as u16);

    // Spreader lifting a suspended 40ft container
    let (sp_lf, sp_rf) = box_faces(crane_x + 0.40, crane_y - 0.18, crane_x + 0.90, crane_y + 0.18, 30, 37, ox, oy);
    sp_lf.fill(sprite, color::surf(rgb(0xdc, 0x28, 0x28), light, FACE_L));
    sp_rf.fill(sprite, color::surf(color::shade(rgb(0xdc, 0x28, 0x28), 0.85), light, FACE_R));
    let sp_top = box_top(crane_x + 0.40, crane_y - 0.18, crane_x + 0.90, crane_y + 0.18, 37, ox, oy);
    sp_top.fill(sprite, color::surf(rgb(0xdc, 0x28, 0x28), light, FACE_TOP));

    // Hoist cables
    let (c0_x, c0_y) = project(crane_x + 0.65, crane_y, 52.0, ox, oy);
    let (c1_x, c1_y) = project(crane_x + 0.65, crane_y, 37.0, ox, oy);
    geom::draw_line(sprite, c0_x.round() as i32, c0_y.round() as i32, c1_x.round() as i32, c1_y.round() as i32, pal::METAL_STEEL_DARK, 52);
}

/// 9. Waterfront / City Landmark Ferris Wheel: large 3-4 tiles high observation wheel with gondolas and neon rim.
pub fn draw_ferris_wheel(_spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let (w, d) = (2.20f32, 2.00f32);
    let wheel_h = 110;

    parts::building_shadow(sprite, 0.0, 0.0, w, d, wheel_h, ox, oy);

    // Ground plaza base
    let (b_lf, b_rf) = box_faces(0.0, 0.0, w, d, 0, 3, ox, oy);
    b_lf.fill(sprite, color::surf(pal::CONCRETE_GREY, light, FACE_L));
    b_rf.fill(sprite, color::surf(pal::CONCRETE_GREY, light, FACE_R));
    let plaza = box_top(0.0, 0.0, w, d, 3, ox, oy);
    plaza.fill(sprite, color::surf(pal::CONCRETE_LIGHT, light, FACE_TOP));

    let hub_x = w / 2.0;
    let hub_y = d / 2.0;
    let hub_z = 58;
    let radius = 44.0f32;

    // Structural A-frame tubular steel support legs
    let leg_c = color::surf(pal::METAL_WHITE, light, 1.0);
    for &(lx, ly) in &[(hub_x - 0.55, hub_y), (hub_x + 0.55, hub_y)] {
        let (p0_x, p0_y) = project(lx, ly, 3.0, ox, oy);
        let (p1_x, p1_y) = project(hub_x, hub_y, hub_z as f32, ox, oy);
        geom::draw_line(sprite, p0_x.round() as i32, p0_y.round() as i32, p1_x.round() as i32, p1_y.round() as i32, leg_c, hub_z as u16);
        geom::draw_line(sprite, p0_x.round() as i32 + 1, p0_y.round() as i32, p1_x.round() as i32 + 1, p1_y.round() as i32, leg_c, hub_z as u16);
    }

    let (center_sx, center_sy) = project(hub_x, hub_y, hub_z as f32, ox, oy);
    let rim_c = if light.is_night { color::NEON_PINK } else { pal::METAL_WHITE };
    let spoke_c = pal::METAL_STEEL_DARK;

    // 24 Radial spokes and passenger gondolas
    let num_spokes = 24;
    for i in 0..num_spokes {
        let angle = i as f32 * std::f32::consts::TAU / num_spokes as f32;
        let px = center_sx + angle.cos() * radius;
        let py = center_sy + angle.sin() * radius;
        geom::draw_line(sprite, center_sx.round() as i32, center_sy.round() as i32, px.round() as i32, py.round() as i32, spoke_c, hub_z as u16);

        // Passenger gondola capsule in bright colors
        let gondola_c = if light.is_night {
            if i % 2 == 0 { color::NEON_CYAN } else { color::NEON_AMBER }
        } else {
            match i % 3 {
                0 => rgb(0xdc, 0x32, 0x32),
                1 => rgb(0x28, 0x6e, 0xba),
                _ => rgb(0xf2, 0xc2, 0x2e),
            }
        };
        let gx = px.round() as i32;
        let gy = py.round() as i32;
        sprite.set_pixel(gx, gy, gondola_c, (hub_z + 2) as u16);
        sprite.set_pixel(gx, gy + 1, gondola_c, (hub_z + 2) as u16);
        sprite.set_pixel(gx + 1, gy, gondola_c, (hub_z + 2) as u16);
    }

    // Outer circular rim ring
    for deg in 0..180 {
        let angle = deg as f32 * std::f32::consts::TAU / 180.0;
        let px = center_sx + angle.cos() * radius;
        let py = center_sy + angle.sin() * radius;
        sprite.set_pixel(px.round() as i32, py.round() as i32, rim_c, (hub_z + 2) as u16);
    }
}

/// 10. Cable-Stay Bridge Pylon: soaring concrete H-pylon with sweeping diagonal stay cables and roadway deck.
pub fn draw_bridge_pylon(_spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let (w, d) = (2.20f32, 2.00f32);
    let pylon_h = 100;

    parts::building_shadow(sprite, 0.0, 0.0, w, d, pylon_h, ox, oy);

    // Concrete bridge roadway deck at z=24
    let deck_z = 24;
    let (d_lf, d_rf) = box_faces(0.0, 0.0, w, d, deck_z - 4, deck_z, ox, oy);
    d_lf.fill(sprite, color::surf(pal::CONCRETE_GREY, light, FACE_L));
    d_rf.fill(sprite, color::surf(pal::CONCRETE_GREY, light, FACE_R));
    let roadway = box_top(0.0, 0.0, w, d, deck_z, ox, oy);
    roadway.paint(sprite, &mut |_u, _v, x, y| {
        let is_center_line = (x % 16 < 10) && (y % 8 == 4);
        let c = if is_center_line { rgb(0xe8, 0xc8, 0x44) } else { pal::ASPHALT };
        Some((color::surf(c, light, FACE_TOP), deck_z as u16))
    });

    // Monumental H-frame concrete bridge pylon tower
    let conc_c = pal::CONCRETE_LIGHT;
    let (p1_lf, p1_rf) = box_faces(0.75, 0.20, 1.15, 0.50, 0, pylon_h, ox, oy);
    p1_lf.fill(sprite, color::surf(conc_c, light, FACE_L));
    p1_rf.fill(sprite, color::surf(conc_c, light, FACE_R));

    let (p2_lf, p2_rf) = box_faces(0.75, d - 0.50, 1.15, d - 0.20, 0, pylon_h, ox, oy);
    p2_lf.fill(sprite, color::surf(conc_c, light, FACE_L));
    p2_rf.fill(sprite, color::surf(conc_c, light, FACE_R));

    // Cross-beam between pylon legs at top
    let (b_lf, b_rf) = box_faces(0.75, 0.20, 1.15, d - 0.20, pylon_h - 10, pylon_h, ox, oy);
    b_lf.fill(sprite, color::surf(conc_c, light, FACE_L));
    b_rf.fill(sprite, color::surf(conc_c, light, FACE_R));
    let b_top = box_top(0.75, 0.20, 1.15, d - 0.20, pylon_h, ox, oy);
    b_top.fill(sprite, color::surf(conc_c, light, FACE_TOP));

    // Sweeping diagonal cable-stay steel cables fanning out down to the bridge deck
    let cable_c = if light.is_night { color::NEON_CYAN } else { pal::METAL_WHITE };
    for i in 0..6 {
        let deck_x = i as f32 * w / 5.0;
        let cable_anchor_z = pylon_h - 8 - i * 4;
        let (p0_x, p0_y) = project(0.95, 0.35, cable_anchor_z as f32, ox, oy);
        let (p1_x, p1_y) = project(deck_x, 0.10, deck_z as f32, ox, oy);
        geom::draw_line(sprite, p0_x.round() as i32, p0_y.round() as i32, p1_x.round() as i32, p1_y.round() as i32, cable_c, pylon_h as u16);

        let (p2_x, p2_y) = project(0.95, d - 0.35, cable_anchor_z as f32, ox, oy);
        let (p3_x, p3_y) = project(deck_x, d - 0.10, deck_z as f32, ox, oy);
        geom::draw_line(sprite, p2_x.round() as i32, p2_y.round() as i32, p3_x.round() as i32, p3_y.round() as i32, cable_c, pylon_h as u16);
    }

    // Red aircraft warning beacons at pylon summits
    parts::spire_needle(sprite, light, 0.95, 0.35, pylon_h, 8, ox, oy);
    parts::spire_needle(sprite, light, 0.95, d - 0.35, pylon_h, 8, ox, oy);
}
