//! The land's own buildings (B46b item 3, D66 item 4).
//!
//! B46 shipped a palette shift over the catalogue's New England houses; the
//! review was right that a desert village of washed-out pitched cottages with
//! chimneys is not a desert village. So the three lands draw their *own*
//! low-rise buildings, out of the same parts system (`crate::parts`):
//!
//! * **desert** — adobe: a stuccoed cube, a flat roof behind a parapet with a
//!   roof terrace, protruding viga beams, a deep-set arched door, small windows
//!   and no chimney;
//! * **tundra** — timber and stone: a stone base, log walls, a steep roof under
//!   a thick snow load with icicles, a hearth plume in the cold;
//! * **tropical** — a stilt house: raised on posts over the wet ground, wide
//!   eaves, palm thatch early and bright painted board later, an open veranda.
//!
//! Taller buildings (towers, offices, hospitals) keep the catalogue's art and
//! take the gentler material shift in `crate::style`; a village is what these
//! functions are for.

use render::palette::rgb;

use crate::color::{self, pal, FACE_L, FACE_R, FACE_TOP};
use crate::geom::{box_faces, box_top, fill_polygon, project};
use crate::parts::{self, WinOpts};
use crate::style::BiomeStyle;
use crate::types::{BuildingSpec, BuildingType, BuildingStyle, Era, Light, Sprite};

/// Colours of one land's plaster, roof, timber and trim.
struct Palette {
    wall: [u32; 4],
    trim: u32,
    roof: u32,
    door: u32,
    /// A second accent: shutters, veranda rail, roof hatch.
    accent: u32,
}

fn desert_palette() -> Palette {
    // Whitewash and sun-baked clay, bleached timber, deep blue doors: the
    // Moroccan / Pueblo mix the land wants.
    Palette {
        wall: [
            pal::STUCCO_SAND,
            pal::STUCCO_OCHRE,
            pal::STUCCO_WHITE,
            pal::STUCCO_TERRACOTTA,
        ],
        trim: pal::STUCCO_WHITE,
        roof: rgb(0xc8, 0x94, 0x68),
        door: rgb(0x3e, 0x5a, 0x8c),
        accent: rgb(0x8a, 0x6a, 0x48),
    }
}

fn tropical_palette() -> Palette {
    // Bright painted board over a dark timber frame.
    Palette {
        wall: [
            rgb(0xf2, 0xc0, 0x88), // mango
            rgb(0x8c, 0xd8, 0xc8), // sea green
            rgb(0xf4, 0xec, 0xd2), // lime whitewash
            rgb(0xe8, 0x8e, 0x96), // frangipani pink
        ],
        trim: rgb(0xf8, 0xf4, 0xe8),
        roof: rgb(0x8a, 0x6c, 0x3e), // palm thatch
        door: rgb(0x3a, 0x6a, 0x50),
        accent: rgb(0x5a, 0x8a, 0x6c),
    }
}

fn tundra_palette() -> Palette {
    // Tarred timber, grey stone, dark slate.
    Palette {
        wall: [
            rgb(0x8c, 0x68, 0x4a),
            rgb(0x7a, 0x58, 0x40),
            rgb(0x6e, 0x76, 0x7e),
            rgb(0x9a, 0x7a, 0x58),
        ],
        trim: rgb(0xd8, 0xd2, 0xc4),
        roof: rgb(0x4e, 0x56, 0x60),
        door: rgb(0x4a, 0x3a, 0x2e),
        accent: rgb(0xa8, 0x4a, 0x3c),
    }
}

/// Draw the land's low-rise version of a catalogue type. Returns `true` when it
/// handled the spec.
pub fn dispatch(
    spec: &BuildingSpec,
    light: &Light,
    sprite: &mut Sprite,
    ox: f32,
    oy: f32,
) -> bool {
    let style = spec.biome;
    if style == BiomeStyle::Temperate {
        return false;
    }
    // A village is what this is for: houses and the small main-street shops.
    // A civic building or a tower keeps the catalogue's art.
    let low_rise = matches!(
        spec.resolved_type(),
        BuildingType::NewEnglandCape
            | BuildingType::NewEnglandColonial
            | BuildingType::NewEnglandSaltbox
            | BuildingType::NewEnglandFarmhouse
            | BuildingType::SuburbanRanch
            | BuildingType::SuburbanCraftsman
            | BuildingType::SuburbanSplitLevel
            | BuildingType::SuburbanContemporary
            | BuildingType::BrickShop2Storey
            | BuildingType::BrickBakeryCafe3Storey
            | BuildingType::BrickCornerStore4Storey
            | BuildingType::BrickBookstoreArcade
    );
    if !low_rise || spec.storeys > 4 {
        return false;
    }
    match style {
        BiomeStyle::Desert => draw_adobe(spec, light, sprite, ox, oy),
        BiomeStyle::Tundra => draw_cabin(spec, light, sprite, ox, oy),
        BiomeStyle::Tropical => draw_stilt(spec, light, sprite, ox, oy),
        BiomeStyle::Swamp | BiomeStyle::Coastal => draw_stilt(spec, light, sprite, ox, oy),
        BiomeStyle::Alpine => draw_chalet(spec, light, sprite, ox, oy),
        BiomeStyle::Savanna => draw_mudbrick(spec, light, sprite, ox, oy),
        BiomeStyle::Highlands => draw_cottage(spec, light, sprite, ox, oy),
        BiomeStyle::Temperate => return false,
    }
    true
}

// ---------------------------------------------------------------------------
// desert: adobe
// ---------------------------------------------------------------------------

fn draw_adobe(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let p = desert_palette();
    let v = spec.variant % 4;
    let age = spec.age;
    // Two storeys for a colonial, a low shop for a storefront.
    let tall = spec.storeys >= 2 || matches!(spec.function, crate::BuildingFunction::Shop);
    let (w, d) = (1.15f32, 1.05f32);
    let z_wall = if tall { 34 } else { 22 } + (v as i32 % 2) * 2;
    let wall = p.wall[v as usize % 4];
    parts::building_shadow(sprite, 0.0, 0.0, w, d, z_wall + 6, ox, oy);

    let (lf, rf) = box_faces(0.0, 0.0, w, d, 0, z_wall, ox, oy);
    parts::paint_stucco(&lf, sprite, light, wall, FACE_L, age);
    parts::paint_stucco(&rf, sprite, light, color::shade(wall, 0.97), FACE_R, age);
    // Rounded corners: knock the two exposed wall corners back a pixel, so the
    // cube reads as hand-built rather than extruded.
    for (cx, cy) in [project(w, d, z_wall as f32, ox, oy), project(0.0, d, 0.0, ox, oy)] {
        sprite.set_pixel(
            cx.round() as i32,
            cy.round() as i32,
            color::surf(color::shade(wall, 0.86), light, FACE_R),
            z_wall as u16,
        );
    }
    // Dado: a darker band at the foot, where the dust settles.
    let (df, dr) = box_faces(0.0, 0.0, w, d, 0, 3, ox, oy);
    df.fill(sprite, color::surf(color::shade(wall, 0.82), light, FACE_L));
    dr.fill(sprite, color::surf(color::shade(wall, 0.76), light, FACE_R));

    parts::roof_flat_parapet(
        sprite,
        light,
        -0.03,
        -0.03,
        w + 0.03,
        d + 0.03,
        z_wall,
        5,
        p.roof,
        wall,
        ox,
        oy,
        age,
    );

    // The viga ends: they stick out of the wall *under* the parapet, so they go
    // on after the roof or the parapet paints over them.
    parts::vigas(sprite, light, &lf, z_wall - 1, 4, p.accent, 4);
    parts::vigas(sprite, light, &rf, z_wall - 1, 4, p.accent, 4);

    // The roof terrace: a hatch to the stairs, and a water jar on half the
    // houses. Small, but it is what tells you the roof is used.
    let hatch = box_top(w * 0.66, d * 0.24, w * 0.82, d * 0.42, z_wall + 6, ox, oy);
    hatch.fill(sprite, color::surf(color::shade(p.accent, 0.82), light, FACE_TOP));
    if v % 2 == 0 {
        let jar = project(w * 0.22, d * 0.30, (z_wall + 5) as f32, ox, oy);
        for k in 0..4 {
            sprite.set_pixel(
                jar.0.round() as i32,
                jar.1.round() as i32 - k,
                color::surf(color::shade(p.roof, 0.92), light, FACE_TOP),
                (z_wall + 6) as u16,
            );
        }
    }

    // Windows: small, deep-set, no shutters (the sun is the enemy, not the rain).
    let wo = WinOpts {
        w: 6,
        h: 6,
        frame: rgb(0xe8, 0xdc, 0xc4),
        sill: false,
        lintel: false,
        shutters: false,
        shutter_c: p.accent,
        seed: spec.seed,
        lit_p: 0.62,
    };
    let len = rf.top_len();
    let steps = if tall { 2 } else { 1 };
    for s in 0..steps {
        let y = 6 + s * (z_wall / 2);
        parts::draw_window(&rf, sprite, light, 4, y, wo, z_wall);
        parts::draw_window(&rf, sprite, light, len - 10, y, wo, z_wall);
        let len_l = lf.top_len();
        parts::draw_window(&lf, sprite, light, 4, y, wo, z_wall);
        parts::draw_window(&lf, sprite, light, len_l - 10, y, wo, z_wall);
    }
    parts::draw_arch_door(
        sprite,
        light,
        &rf,
        len / 2 - 3,
        z_wall - 11,
        6,
        11,
        p.door,
        p.trim,
        z_wall,
    );
    // A shop gets an awning: a stall of striped cloth off the front wall.
    if matches!(spec.function, crate::BuildingFunction::Shop) {
        awning(sprite, light, &rf, len / 2 - 14, z_wall - 14, 28, p.trim, color::shade(p.door, 1.1));
    }
}

// ---------------------------------------------------------------------------
// tundra: timber and stone
// ---------------------------------------------------------------------------

fn draw_cabin(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let p = tundra_palette();
    let v = spec.variant % 4;
    let age = spec.age;
    let storeys = spec.storeys.max(1) as i32;
    let (w, d) = (1.15f32, 1.05f32);
    let z_base = 4;
    let z_wall = z_base + 14 * storeys.min(2) + (v as i32 % 2) * 2;
    let rise = 14 + (v as i32 % 3) * 3; // steep enough to shed the snow
    let wall = p.wall[v as usize % 4];
    parts::building_shadow(sprite, 0.0, 0.0, w, d, z_wall + rise, ox, oy);

    // A stone plinth, so the timber sits out of the meltwater.
    let (sf, sr) = box_faces(0.0, 0.0, w, d, 0, z_base, ox, oy);
    sf.fill(sprite, color::surf(pal::STONE_GRANITE, light, FACE_L));
    sr.fill(sprite, color::surf(color::shade(pal::STONE_GRANITE, 0.84), light, FACE_R));

    let (lf, rf) = box_faces(0.0, 0.0, w, d, z_base, z_wall, ox, oy);
    parts::paint_log(&lf, sprite, light, wall, FACE_L, age);
    parts::paint_log(&rf, sprite, light, color::shade(wall, 0.96), FACE_R, age);

    // Gable ends and a steep roof under its snow load.
    let my = d / 2.0;
    parts::draw_gable_end(
        sprite,
        light,
        project(0.0, 0.0, z_wall as f32, ox, oy),
        project(0.0, d, z_wall as f32, ox, oy),
        project(0.0, my, (z_wall + rise) as f32, ox, oy),
        wall,
        FACE_L,
        z_wall,
        z_wall + rise,
    );
    parts::draw_gable_end(
        sprite,
        light,
        project(w, 0.0, z_wall as f32, ox, oy),
        project(w, d, z_wall as f32, ox, oy),
        project(w, my, (z_wall + rise) as f32, ox, oy),
        wall,
        FACE_R,
        z_wall,
        z_wall + rise,
    );
    parts::roof_snow_steep(
        sprite,
        light,
        -0.06,
        -0.06,
        w + 0.06,
        d + 0.06,
        z_wall,
        rise,
        p.roof,
        rgb(0xdc, 0xe6, 0xf2),
        ox,
        oy,
        age,
    );

    // The hearth: a stone chimney on the ridge, with a plume in the cold.
    let (cx0, cy0) = (w * 0.42, d * 0.40);
    let (cf, cr) = box_faces(cx0, cy0, cx0 + 0.18, cy0 + 0.18, z_wall - 2, z_wall + rise + 8, ox, oy);
    cf.fill(sprite, color::surf(pal::STONE_GRANITE, light, FACE_L));
    cr.fill(sprite, color::surf(color::shade(pal::STONE_GRANITE, 0.82), light, FACE_R));
    let top = project(cx0 + 0.09, cy0 + 0.09, (z_wall + rise + 9) as f32, ox, oy);
    parts::smoke_plume(sprite, light, top.0.round() as i32 + 1, top.1.round() as i32 - 1, (z_wall + rise + 10) as u16);

    // Small deep-set windows with bright shutters; a stout door with a porch.
    let wo = WinOpts {
        w: 7,
        h: 7,
        frame: p.trim,
        sill: true,
        lintel: true,
        shutters: true,
        shutter_c: p.accent,
        seed: spec.seed,
        lit_p: 0.72,
    };
    let len = rf.top_len();
    parts::draw_window(&rf, sprite, light, 4, z_base + 4, wo, z_wall);
    parts::draw_window(&rf, sprite, light, len - 11, z_base + 4, wo, z_wall);
    parts::draw_window(&lf, sprite, light, lf.top_len() / 2 - 3, z_base + 4, wo, z_wall);
    if storeys >= 2 {
        parts::draw_window(&rf, sprite, light, 7, z_base + 16, wo, z_wall);
    }
    parts::draw_door(&rf, sprite, light, len / 2 - 3, z_wall - z_base - 11, 6, 11, p.door, p.trim, z_wall);
    // The porch canopy over the door.
    let pw = 0.30;
    let (pf, pr) = box_faces(
        (w - pw) * 0.5,
        d - 0.02,
        (w + pw) * 0.5,
        d + 0.22,
        z_base + 12,
        z_base + 13,
        ox,
        oy,
    );
    pf.fill(sprite, color::surf(rgb(0xe8, 0xf0, 0xf8), light, FACE_TOP));
    pr.fill(sprite, color::surf(rgb(0xd0, 0xda, 0xe6), light, FACE_TOP));
}

// ---------------------------------------------------------------------------
// tropical: stilt house
// ---------------------------------------------------------------------------

fn swamp_palette() -> Palette {
    // Weathered cypress boards, grey-green with damp, under rusted tin.
    Palette {
        wall: [
            rgb(0x86, 0x78, 0x5e),
            rgb(0x72, 0x82, 0x6c),
            rgb(0x94, 0x86, 0x68),
            rgb(0x6a, 0x72, 0x66),
        ],
        trim: rgb(0xd8, 0xd2, 0xc0),
        roof: rgb(0x8a, 0x6a, 0x4a),
        door: rgb(0x4a, 0x62, 0x58),
        accent: rgb(0x5a, 0x72, 0x62),
    }
}

fn coastal_palette() -> Palette {
    // Sea-washed pastels with white trim and bright shutters.
    Palette {
        wall: [
            rgb(0x9e, 0xd4, 0xe8), // sky blue
            rgb(0xf6, 0xf0, 0xe4), // white
            rgb(0xf4, 0xdc, 0x8c), // sun yellow
            rgb(0xf0, 0xa8, 0x9a), // coral
        ],
        trim: rgb(0xfa, 0xf8, 0xf0),
        roof: rgb(0x8a, 0x6c, 0x3e),
        door: rgb(0x2e, 0x7a, 0xa8),
        accent: rgb(0x2e, 0x9a, 0xb4),
    }
}

fn alpine_palette() -> Palette {
    Palette {
        wall: [
            rgb(0xf0, 0xe8, 0xd6), // limewash
            rgb(0xe8, 0xdc, 0xc4),
            rgb(0xf4, 0xee, 0xe2),
            rgb(0xe2, 0xd6, 0xbe),
        ],
        trim: rgb(0xf6, 0xf0, 0xe4),
        roof: rgb(0x6a, 0x48, 0x38), // dark shingle
        door: rgb(0x5a, 0x3a, 0x2a),
        accent: rgb(0xa8, 0x3c, 0x34), // red shutters
    }
}

fn savanna_palette() -> Palette {
    Palette {
        wall: [
            rgb(0xc4, 0x80, 0x58), // red earth
            rgb(0xb4, 0x72, 0x4e),
            rgb(0xd0, 0x92, 0x66),
            rgb(0xe4, 0xc8, 0x9c), // limewashed
        ],
        trim: rgb(0xf0, 0xe4, 0xc8),
        roof: rgb(0xb8, 0x9a, 0x50), // golden thatch
        door: rgb(0x4a, 0x7a, 0x8a),
        accent: rgb(0x8a, 0x52, 0x3a),
    }
}

fn highland_palette() -> Palette {
    Palette {
        wall: [
            rgb(0x8c, 0x8a, 0x84), // grey stone
            rgb(0x7a, 0x7a, 0x76),
            rgb(0x9a, 0x94, 0x88),
            rgb(0xf0, 0xec, 0xe0), // whitewashed
        ],
        trim: rgb(0xe8, 0xe4, 0xd8),
        roof: rgb(0x68, 0x70, 0x7c), // slate
        door: rgb(0x3a, 0x5a, 0x4a),
        accent: rgb(0x8a, 0x3a, 0x34),
    }
}

fn draw_stilt(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let p = match spec.biome {
        BiomeStyle::Swamp => swamp_palette(),
        BiomeStyle::Coastal => coastal_palette(),
        _ => tropical_palette(),
    };
    let v = spec.variant % 4;
    let age = spec.age;
    let early = matches!(spec.era, Era::Settlement | Era::Village);
    let (w, d) = (1.15f32, 1.05f32);
    // Raised clear of the wet ground; the taller the house, the higher it sits.
    let z_floor = 8 + (v as i32 % 2) * 3;
    let z_wall = 14 + (spec.storeys.max(1) as i32 - 1) * 12;
    let rise = 14 + (v as i32 % 3) * 3;
    let wall = p.wall[v as usize % 4];
    parts::building_shadow(sprite, 0.0, 0.0, w, d, z_floor + z_wall + rise, ox, oy);
    parts::stilts(sprite, light, 0.0, 0.0, w, d, z_floor, rgb(0x6a, 0x52, 0x40), ox, oy);

    let (lf, rf) = box_faces(0.0, 0.0, w, d, z_floor, z_floor + z_wall, ox, oy);
    if early {
        // Split-bamboo walls, woven.
        parts::paint_clapboard(&lf, sprite, light, color::shade(wall, 0.94), FACE_L, age);
        parts::paint_clapboard(&rf, sprite, light, wall, FACE_R, age);
    } else {
        // Painted board with a white trim band at the top.
        parts::paint_clapboard(&lf, sprite, light, wall, FACE_L, age);
        parts::paint_clapboard(&rf, sprite, light, color::shade(wall, 0.97), FACE_R, age);
    }
    // Wide eaves: the roof apron is the whole point of the form.
    if early || v == 3 {
        parts::roof_thatch(
            sprite,
            light,
            -0.16,
            -0.16,
            w + 0.16,
            d + 0.16,
            z_floor + z_wall,
            rise,
            p.roof,
            ox,
            oy,
            age,
        );
    } else {
        parts::roof_hip_tile(
            sprite,
            light,
            -0.14,
            -0.14,
            w + 0.14,
            d + 0.14,
            z_floor + z_wall,
            rise,
            match (spec.biome, v % 2) {
                (BiomeStyle::Swamp, 0) => rgb(0x9a, 0x62, 0x48), // rusted tin
                (BiomeStyle::Swamp, _) => rgb(0x72, 0x7c, 0x78), // grey tin
                (BiomeStyle::Coastal, 0) => rgb(0xc8, 0x54, 0x40), // red tile
                (BiomeStyle::Coastal, _) => rgb(0x2e, 0x86, 0xa6), // blue tin
                (_, 0) => rgb(0xc8, 0x5a, 0x46), // fired clay
                _ => rgb(0x62, 0x8a, 0x92), // painted tin
            },
            ox,
            oy,
            age,
        );
    }

    // The veranda: a rail along the front, and two posts out to the eave.
    let rail_z = z_floor + 5;
    let (rf2, _) = box_faces(0.0, d - 0.02, w, d + 0.30, z_floor, rail_z, ox, oy);
    let rail = color::surf(p.accent, light, FACE_L);
    for x in 0..rf2.top_len() {
        let q = rf2.sub_px(x, 0, x + 1, 1, rf2.rows() as f32);
        sprite.set_pixel(q.tl.0.round() as i32, q.tl.1.round() as i32, rail, rail_z as u16);
    }
    for f in [0.08f32, 0.92] {
        let (px, py) = project(w * f, d + 0.30, (z_floor + z_wall) as f32, ox, oy);
        let (bx, by) = project(w * f, d + 0.30, z_floor as f32, ox, oy);
        crate::geom::draw_line(
            sprite,
            px.round() as i32,
            py.round() as i32,
            bx.round() as i32,
            by.round() as i32,
            color::surf(rgb(0x6a, 0x52, 0x40), light, FACE_L),
            (z_floor + z_wall) as u16,
        );
    }

    // Shuttered windows either side of a wide front door.
    let wo = WinOpts {
        w: 7,
        h: 8,
        frame: p.trim,
        sill: true,
        lintel: false,
        shutters: true,
        shutter_c: color::shade(p.accent, 0.92),
        seed: spec.seed,
        lit_p: 0.66,
    };
    let len = rf.top_len();
    parts::draw_window(&rf, sprite, light, 4, z_floor + 4, wo, z_floor + z_wall);
    parts::draw_window(&rf, sprite, light, len - 11, z_floor + 4, wo, z_floor + z_wall);
    parts::draw_window(&lf, sprite, light, lf.top_len() / 2 - 3, z_floor + 4, wo, z_floor + z_wall);
    parts::draw_door(
        &rf,
        sprite,
        light,
        len / 2 - 3,
        z_floor + z_wall - z_floor - 11,
        6,
        11,
        p.door,
        p.trim,
        z_floor + z_wall,
    );
    if matches!(spec.function, crate::BuildingFunction::Shop) {
        awning(sprite, light, &rf, len / 2 - 13, z_floor + 4, 26, p.trim, color::shade(p.door, 1.15));
    }
}


// ---------------------------------------------------------------------------
// alpine: chalet
// ---------------------------------------------------------------------------

fn draw_chalet(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let p = alpine_palette();
    let v = spec.variant % 4;
    let age = spec.age;
    let (w, d) = (1.18f32, 1.04f32);
    let z_base = 3;
    let z_mid = z_base + 15; // limewashed ground floor
    let z_wall = z_mid + 14; // timber upper floor
    let rise = 11 + (v as i32 % 2) * 2; // a low, wide pitch: it carries the snow
    let wall = p.wall[v as usize % 4];
    let timber = rgb(0x7a, 0x52, 0x38);
    parts::building_shadow(sprite, 0.0, 0.0, w, d, z_wall + rise + 2, ox, oy);

    // Stone plinth, limewashed ground floor.
    let (sf, sr) = box_faces(0.0, 0.0, w, d, 0, z_base, ox, oy);
    sf.fill(sprite, color::surf(pal::STONE_GRANITE, light, FACE_L));
    sr.fill(sprite, color::surf(color::shade(pal::STONE_GRANITE, 0.84), light, FACE_R));
    let (lf0, rf0) = box_faces(0.0, 0.0, w, d, z_base, z_mid, ox, oy);
    parts::paint_stucco(&lf0, sprite, light, wall, FACE_L, age);
    parts::paint_stucco(&rf0, sprite, light, color::shade(wall, 0.97), FACE_R, age);
    // The upper floor, timber, jettied out a hand's width over the first.
    let j = 0.06;
    let (lf, rf) = box_faces(-j, -j, w + j, d + j, z_mid, z_wall, ox, oy);
    parts::paint_log(&lf, sprite, light, timber, FACE_L, age);
    parts::paint_log(&rf, sprite, light, color::shade(timber, 0.95), FACE_R, age);
    // Gable ends in timber, and a deep-eaved roof.
    let my = d / 2.0;
    parts::draw_gable_end(
        sprite,
        light,
        project(-j, -j, z_wall as f32, ox, oy),
        project(-j, d + j, z_wall as f32, ox, oy),
        project(-j, my, (z_wall + rise) as f32, ox, oy),
        color::shade(timber, 0.92),
        FACE_L,
        z_wall,
        z_wall + rise,
    );
    parts::draw_gable_end(
        sprite,
        light,
        project(w + j, -j, z_wall as f32, ox, oy),
        project(w + j, d + j, z_wall as f32, ox, oy),
        project(w + j, my, (z_wall + rise) as f32, ox, oy),
        color::shade(timber, 0.92),
        FACE_R,
        z_wall,
        z_wall + rise,
    );
    parts::roof_gable(
        sprite,
        light,
        -0.26,
        -0.22,
        w + 0.26,
        d + 0.22,
        z_wall,
        rise,
        p.roof,
        ox,
        oy,
        age,
    );
    // A stone chimney.
    let (cx0, cy0) = (w * 0.62, d * 0.30);
    let (cf, cr) = box_faces(cx0, cy0, cx0 + 0.16, cy0 + 0.16, z_wall, z_wall + rise + 6, ox, oy);
    cf.fill(sprite, color::surf(pal::STONE_GRANITE, light, FACE_L));
    cr.fill(sprite, color::surf(color::shade(pal::STONE_GRANITE, 0.82), light, FACE_R));
    if v % 2 == 0 {
        let top = project(cx0 + 0.08, cy0 + 0.08, (z_wall + rise + 7) as f32, ox, oy);
        parts::smoke_plume(sprite, light, top.0.round() as i32 + 1, top.1.round() as i32 - 1, (z_wall + rise + 8) as u16);
    }

    // The balcony runs along the front of the upper floor, flower boxes and all.
    let len = rf.top_len();
    parts::draw_balcony(&rf, sprite, light, 3, 4, len - 6, 6, z_wall);
    // Shuttered windows; a heavy door with a stone step.
    let wo = WinOpts {
        w: 6,
        h: 7,
        frame: p.trim,
        sill: true,
        lintel: false,
        shutters: true,
        shutter_c: p.accent,
        seed: spec.seed,
        lit_p: 0.68,
    };
    parts::draw_window(&rf0, sprite, light, 4, 5, wo, z_mid);
    parts::draw_window(&rf0, sprite, light, len - 11, 5, wo, z_mid);
    parts::draw_window(&lf0, sprite, light, lf0.top_len() / 2 - 3, 5, wo, z_mid);
    parts::draw_window(&rf, sprite, light, 6, 12, wo, z_wall);
    parts::draw_window(&rf, sprite, light, len - 13, 12, wo, z_wall);
    parts::draw_window(&lf, sprite, light, lf.top_len() / 2 - 3, 10, wo, z_wall);
    parts::draw_door(&rf0, sprite, light, len / 2 - 3, z_mid - z_base - 10, 6, 10, p.door, p.trim, z_mid);
    if matches!(spec.function, crate::BuildingFunction::Shop) {
        awning(sprite, light, &rf0, len / 2 - 13, z_mid - z_base - 12, 26, p.trim, p.accent);
    }
}

// ---------------------------------------------------------------------------
// savanna: mud-brick and thatch, then tin
// ---------------------------------------------------------------------------

fn draw_mudbrick(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let p = savanna_palette();
    let v = spec.variant % 4;
    let age = spec.age;
    let early = matches!(spec.era, Era::Settlement | Era::Village);
    let (w, d) = (1.08f32, 1.0f32);
    let z_wall = 20 + (spec.storeys.max(1) as i32 - 1) * 12 + (v as i32 % 2) * 2;
    let rise = 14 + (v as i32 % 3) * 2;
    // Later houses are limewashed (the fourth wall colour); the early ones are
    // the raw red earth.
    let wall = if early { p.wall[(v as usize) % 3] } else { p.wall[(v as usize + 1) % 4] };
    parts::building_shadow(sprite, 0.0, 0.0, w, d, z_wall + rise, ox, oy);
    let (lf, rf) = box_faces(0.0, 0.0, w, d, 0, z_wall, ox, oy);
    parts::paint_stucco(&lf, sprite, light, wall, FACE_L, age);
    parts::paint_stucco(&rf, sprite, light, color::shade(wall, 0.96), FACE_R, age);
    // Dust splashed up the foot of the wall.
    let (df, dr) = box_faces(0.0, 0.0, w, d, 0, 3, ox, oy);
    df.fill(sprite, color::surf(color::shade(wall, 0.80), light, FACE_L));
    dr.fill(sprite, color::surf(color::shade(wall, 0.74), light, FACE_R));
    if early {
        // A deep hip of golden thatch, the eaves reaching past the walls.
        parts::roof_thatch(sprite, light, -0.18, -0.18, w + 0.18, d + 0.18, z_wall, rise + 2, p.roof, ox, oy, age);
    } else {
        // Corrugated tin on a low gable, red-oxide or galvanised.
        let tin = if v % 2 == 0 { rgb(0x9a, 0x5a, 0x44) } else { rgb(0xa4, 0xac, 0xae) };
        parts::roof_gable(sprite, light, -0.10, -0.10, w + 0.10, d + 0.10, z_wall, rise - 3, tin, ox, oy, age);
        let my = d / 2.0;
        for (x, sh) in [(0.0, FACE_L), (w, FACE_R)] {
            parts::draw_gable_end(
                sprite,
                light,
                project(x, 0.0, z_wall as f32, ox, oy),
                project(x, d, z_wall as f32, ox, oy),
                project(x, my, (z_wall + rise - 3) as f32, ox, oy),
                wall,
                sh,
                z_wall,
                z_wall + rise - 3,
            );
        }
    }
    let wo = WinOpts {
        w: 5,
        h: 6,
        frame: p.trim,
        sill: false,
        lintel: false,
        shutters: !early,
        shutter_c: p.accent,
        seed: spec.seed,
        lit_p: 0.6,
    };
    let len = rf.top_len();
    parts::draw_window(&rf, sprite, light, 4, 7, wo, z_wall);
    parts::draw_window(&rf, sprite, light, len - 10, 7, wo, z_wall);
    parts::draw_window(&lf, sprite, light, lf.top_len() / 2 - 3, 7, wo, z_wall);
    parts::draw_arch_door(sprite, light, &rf, len / 2 - 3, z_wall - 10, 6, 10, p.door, p.trim, z_wall);
    if matches!(spec.function, crate::BuildingFunction::Shop) {
        awning(sprite, light, &rf, len / 2 - 13, z_wall - 14, 26, p.trim, p.accent);
    }
}

// ---------------------------------------------------------------------------
// highlands: stone cottage under slate
// ---------------------------------------------------------------------------

fn draw_cottage(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let p = highland_palette();
    let v = spec.variant % 4;
    let age = spec.age;
    let (w, d) = (1.2f32, 1.0f32);
    let z_wall = 22 + (spec.storeys.max(1) as i32 - 1).min(1) * 12;
    let rise = 15 + (v as i32 % 2) * 2;
    let wall = p.wall[v as usize % 4];
    parts::building_shadow(sprite, 0.0, 0.0, w, d, z_wall + rise, ox, oy);
    let (lf, rf) = box_faces(0.0, 0.0, w, d, 0, z_wall, ox, oy);
    if v == 3 {
        // Whitewashed croft.
        parts::paint_stucco(&lf, sprite, light, wall, FACE_L, age);
        parts::paint_stucco(&rf, sprite, light, color::shade(wall, 0.96), FACE_R, age);
    } else {
        parts::paint_stone_ashlar(&lf, sprite, light, wall, FACE_L, age);
        parts::paint_stone_ashlar(&rf, sprite, light, color::shade(wall, 0.94), FACE_R, age);
    }
    let my = d / 2.0;
    for (x, sh) in [(0.0, FACE_L), (w, FACE_R)] {
        parts::draw_gable_end(
            sprite,
            light,
            project(x, 0.0, z_wall as f32, ox, oy),
            project(x, d, z_wall as f32, ox, oy),
            project(x, my, (z_wall + rise) as f32, ox, oy),
            if v == 3 { wall } else { color::shade(wall, 0.96) },
            sh,
            z_wall,
            z_wall + rise,
        );
    }
    parts::roof_gable(sprite, light, -0.05, -0.05, w + 0.05, d + 0.05, z_wall, rise, p.roof, ox, oy, age);
    // A chimney at each gable, and a peat smoke from one.
    for (k, cx0) in [0.04f32, w - 0.20].iter().enumerate() {
        let cy0 = d * 0.36;
        let (cf, cr) = box_faces(*cx0, cy0, cx0 + 0.16, cy0 + 0.16, z_wall + 2, z_wall + rise + 5, ox, oy);
        cf.fill(sprite, color::surf(color::shade(wall, 0.9), light, FACE_L));
        cr.fill(sprite, color::surf(color::shade(wall, 0.72), light, FACE_R));
        if k == 0 {
            let top = project(cx0 + 0.08, cy0 + 0.08, (z_wall + rise + 6) as f32, ox, oy);
            parts::smoke_plume(sprite, light, top.0.round() as i32 + 1, top.1.round() as i32 - 1, (z_wall + rise + 7) as u16);
        }
    }
    let wo = WinOpts {
        w: 5,
        h: 6,
        frame: p.trim,
        sill: true,
        lintel: true,
        shutters: false,
        shutter_c: p.accent,
        seed: spec.seed,
        lit_p: 0.7,
    };
    let len = rf.top_len();
    parts::draw_window(&rf, sprite, light, 5, 8, wo, z_wall);
    parts::draw_window(&rf, sprite, light, len - 11, 8, wo, z_wall);
    parts::draw_window(&lf, sprite, light, lf.top_len() / 2 - 3, 8, wo, z_wall);
    if spec.storeys >= 2 {
        parts::draw_window(&rf, sprite, light, len / 2 - 3, 20, wo, z_wall);
    }
    parts::draw_door(&rf, sprite, light, len / 2 - 3, z_wall - 11, 6, 11, p.door, p.trim, z_wall);
    if matches!(spec.function, crate::BuildingFunction::Shop) {
        awning(sprite, light, &rf, len / 2 - 13, z_wall - 14, 26, p.trim, p.accent);
    }
}

/// A striped canvas awning over a shop front: two or three bands off the wall,
/// with the shadow they cast.
fn awning(
    sprite: &mut Sprite,
    light: &Light,
    q: &crate::geom::Quad,
    u: i32,
    v: i32,
    w: i32,
    trim: u32,
    stripe: u32,
) {
    let rows = q.rows() as f32;
    let aq = q.sub_px(u, v, u + w, v + 4, rows);
    let len = aq.top_len() as i32;
    let out = 5;
    for k in 0..out {
        let p0 = (aq.tl.0 + k as f32, aq.tl.1 + k as f32 * 0.55);
        let p1 = (aq.tr.0 + k as f32, aq.tr.1 + k as f32 * 0.55);
        let c = if k == out - 1 {
            color::shade(color::surf(trim, light, FACE_TOP), 0.80) // the shadowed hem
        } else if (k / 2) % 2 == 0 {
            color::surf(trim, light, FACE_TOP)
        } else {
            color::surf(stripe, light, FACE_TOP)
        };
        crate::geom::draw_line(
            sprite,
            p0.0.round() as i32,
            p0.1.round() as i32,
            p1.0.round() as i32,
            p1.1.round() as i32,
            c,
            (v + 4 + k) as u16,
        );
    }
    let _ = len;
    // Scalloped hem: a pixel dropped every third column.
    for k in (0..w).step_by(3) {
        let p = (aq.tl.0 + k as f32 + out as f32, aq.tl.1 + out as f32 * 0.55 + 1.0);
        sprite.set_pixel(
            p.0.round() as i32,
            p.1.round() as i32,
            color::surf(color::shade(trim, 0.9), light, FACE_L),
            (v + 4) as u16,
        );
    }
}

/// The palette a land's *tall* buildings take in `crate::style` (kept here so
/// the vernacular and the tint agree on the walls).
pub fn wall_tint(style: BiomeStyle, wall: u32) -> u32 {
    let p = match style {
        BiomeStyle::Desert => desert_palette(),
        BiomeStyle::Tropical => tropical_palette(),
        BiomeStyle::Tundra => tundra_palette(),
        BiomeStyle::Alpine => alpine_palette(),
        BiomeStyle::Savanna => savanna_palette(),
        BiomeStyle::Swamp => swamp_palette(),
        BiomeStyle::Coastal => coastal_palette(),
        BiomeStyle::Highlands => highland_palette(),
        BiomeStyle::Temperate => return wall,
    };
    color::mix(wall, p.wall[0], 0.25)
}

/// The new style's mark: the palette shift in `crate::style` is for buildings
/// this module does not draw.
pub fn draws_own(spec: &BuildingSpec) -> bool {
    let _ = spec.style;
    !matches!(spec.biome, BiomeStyle::Temperate)
        && spec.storeys <= 4
        && !matches!(spec.style, BuildingStyle::ModernGlass)
}
