//! B76: the season on the built city — snow lying on a roof, flowers on a
//! windowsill, a parasol over a café table.
//!
//! The terrain and the trees are season-dressed in `terrain.rs` (they are the
//! ground). A *building* is mostly drawn from a pre-rendered kit sprite
//! (`buildkit`), so its roof cannot re-colour itself; this module draws the
//! seasonal dressing on top, in the static layer, from the lot's own geometry.
//! Everything here is a pure function of the lot, the season and the snow, so a
//! dirty-rect repaint draws the same pixels.

use crate::city::{CityObj, ObjKind};
use crate::framebuffer::Framebuffer;
use crate::iso::{Camera, HEIGHT_UNIT};
use crate::palette::{self, Color};
use crate::terrain::SeasonLook;

/// The snow colour roofs and ledges take.
const SNOW: (u8, u8, u8) = (0xec, 0xf3, 0xfa);

/// The face a decoration is seen on. The camera looks at the +x and +y faces
/// only, so a lot whose door is on the far side still has its flowers and its
/// lights on a face the eye can find (B76).
fn visible_face(o: &CityObj) -> u8 {
    match o.front % 4 {
        0 | 1 => o.front % 4,
        _ => (o.seed % 2) as u8,
    }
}

/// The lot's built footprint and eave height, the same inset the engine uses.
fn footprint(o: &CityObj) -> (f32, f32, f32, f32) {
    let i = 0.13;
    (
        o.x as f32 + i,
        o.y as f32 + i,
        (o.x + o.w) as f32 - i,
        (o.y + o.h) as f32 - i,
    )
}

/// Is this a lot a person lives in (so spring flowers and winter smoke belong on
/// it)?
fn is_dwelling(k: ObjKind) -> bool {
    matches!(
        k,
        ObjKind::House | ObjKind::Townhouse | ObjKind::Apartment | ObjKind::LogCabin
    )
}

/// A shop front on a street: parasols and café tables come out in summer.
fn is_hospitality(k: ObjKind) -> bool {
    matches!(k, ObjKind::Store | ObjKind::Shop | ObjKind::Landmark(_))
}

/// The season on the frontage of one lot.
pub fn building(
    fb: &mut Framebuffer,
    cam: &Camera,
    o: &CityObj,
    season: &SeasonLook,
    hour: f32,
    hol: &crate::holiday::HolidayLook,
) {
    if !matches!(o.stage, crate::city::Stage::Finished) {
        return;
    }
    // Street side: the front face's two outer corners, so anything put on the
    // wall lands on the wall and not through it.
    if season.blossom > 0.2 || (season.fresh > 0.2 && season.blossom < 0.2) {
        if is_dwelling(o.kind) {
            window_boxes(fb, cam, o, season);
        }
    }
    if season.grass_dry < 0.1 && season.blossom < 0.05 && season.autumn < 0.05 {
        // Summer: café tables and parasols outside a shop.
        if is_hospitality(o.kind) {
            cafe(fb, cam, o, hour);
        }
    }
    // B76: the holiday lights — a string along the eave of the street face, on
    // the share of houses whose residents have got round to it.
    if hol.is_lit() {
        holiday_lights(fb, cam, o, hol);
    }
    // Autumn: pumpkins on doorsteps come from `holiday` (Halloween), not here.
}

/// One house's holiday lights (B76): a warm string along the eave and a lit
/// shrub by the door. Which houses are lit is the lot's own hash against the
/// town's share, so the town fills up house by house over weeks and empties the
/// same way.
fn holiday_lights(fb: &mut Framebuffer, cam: &Camera, o: &CityObj, hol: &crate::holiday::HolidayLook) {
    // A dwelling or a shop, never a tower or a park.
    if !matches!(
        o.kind,
        ObjKind::House
            | ObjKind::Townhouse
            | ObjKind::Apartment
            | ObjKind::Store
            | ObjKind::Shop
            | ObjKind::LogCabin
    ) {
        return;
    }
    let h01 = (o.seed as f32 * 0.618_034).fract();
    if h01 > hol.lights {
        return;
    }
    // The eave of the street face.
    let base = o.level as i32 * HEIGHT_UNIT;
    let (x0, y0, x1, y1) = footprint(o);
    let (ex0, ey0, ex1, ey1) = (x0 - 0.04, y0 - 0.04, x1 + 0.04, y1 + 0.04);
    let z = o.wall_height();
    let (a, b) = match visible_face(o) {
        0 => (cam.project(ex1, ey0, base + z), cam.project(ex1, ey1, base + z)),
        1 => (cam.project(ex0, ey1, base + z), cam.project(ex1, ey1, base + z)),
        2 => (cam.project(ex0, ey1, base + z), cam.project(ex0, ey0, base + z)),
        _ => (cam.project(ex0, ey0, base + z), cam.project(ex1, ey0, base + z)),
    };
    let len = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
    let n = (len / 4.0).floor().max(2.0) as i32;
    // The strand itself, then a bulb every third pixel.
    fb.line(a, b, palette::rgb(0x2c, 0x2c, 0x30));
    let bulbs: [Color; 4] = if hol.holiday == crate::holiday::Holiday::Xmas && hol.bulbs > 0 {
        hol.garland
    } else {
        // The ordinary string: warm white and gold.
        [
            palette::rgb(0xff, 0xec, 0xc0),
            palette::rgb(0xff, 0xd0, 0x70),
            palette::rgb(0xff, 0xf4, 0xd8),
            palette::rgb(0xff, 0xc0, 0x58),
        ]
    };
    for i in 0..=n {
        let t = i as f32 / n as f32;
        let x = a.0 + (b.0 - a.0) * t;
        let y = a.1 + (b.1 - a.1) * t;
        let c = bulbs[(i as usize + o.seed as usize) % 4];
        // 0xFE is the "keeps its brightness at night" tag the window lights use;
        // without it the static-layer tint dims the bulbs to nothing. A bulb is
        // 2x2 so a lit eave still reads when the whole town is at phone size.
        let lit = palette::with_alpha(c, 0xFE);
        let hi = palette::with_alpha(palette::shade(c, 1.12), 0xFE);
        fb.fill_rect(x as i32, y as i32, 2, 2, lit);
        fb.put(x as i32, y as i32 - 1, hi);
    }
    // A lit shrub or a lit tree by the door: one bright clump.
    if h01 * 3.0 < hol.lights * 3.0 {
        let (dx, dy) = match visible_face(o) {
            0 => (ex1 + 0.35, (ey0 + ey1) * 0.5),
            1 => ((ex0 + ex1) * 0.5, ey1 + 0.35),
            2 => (ex0 - 0.35, (ey0 + ey1) * 0.5),
            _ => ((ex0 + ex1) * 0.5, ey0 - 0.35),
        };
        let (px, py) = cam.project(dx, dy, base);
        let c = palette::rgb(0xff, 0xe0, 0x90);
        fb.put(px as i32, py as i32 - 1, palette::with_alpha(c, 0xFE));
        fb.put(
            px as i32 - 1,
            py as i32,
            palette::with_alpha(palette::shade(c, 0.85), 0xFE),
        );
        fb.put(
            px as i32 + 1,
            py as i32,
            palette::with_alpha(palette::shade(c, 0.85), 0xFE),
        );
    }
}

/// A row of flower boxes on the windowsills of a dwelling in spring (B76).
fn window_boxes(fb: &mut Framebuffer, cam: &Camera, o: &CityObj, season: &SeasonLook) {
    let base = o.level as i32 * HEIGHT_UNIT;
    let (x0, y0, x1, y1) = footprint(o);
    let h = o.wall_height().min(26);
    // Two sills up the street face, three boxes across it.
    let faces = match visible_face(o) {
        0 => (x1, y0, y1, true),  // +x face, sills run along y
        1 => (y1, x0, x1, false), // +y face, sills run along x
        2 => (x0, y0, y1, true),
        _ => (y0, x0, x1, false),
    };
    let (fixed, a0, a1, _along_y) = faces;
    let n = ((a1 - a0) / 0.55).floor().max(1.0) as i32;
    let bloom = [
        palette::rgb(0xe8, 0x6a, 0x8a),
        palette::rgb(0xf0, 0xc8, 0x40),
        palette::rgb(0xf4, 0xf0, 0xea),
        palette::rgb(0xa0, 0x6a, 0xc8),
    ];
    let mix = season.blossom.max(season.fresh);
    for i in 0..n.min(4) {
        let t = (i as f32 + 0.5) / n as f32;
        let along = a0 + (a1 - a0) * t;
        for (row, frac) in [(0, 0.62), (1, 0.42)] {
            let z = (h as f32 * frac) as i32;
            let (px, py) = if matches!(visible_face(o), 0) {
                cam.project(fixed, along, base + z)
            } else {
                cam.project(along, fixed, base + z)
            };
            // A dark window box with a bright head of flowers.
            let c = palette::shade(palette::rgb(0x4a, 0x3a, 0x2c), 1.0);
            fb.fill_rect(px as i32 - 2, py as i32, 5, 1, c);
            let f = bloom[(i as usize + row * 2 + (o.seed as usize & 3)) % bloom.len()];
            let f = palette::mix(palette::rgb(0x4a, 0x6a, 0x30), f, mix);
            fb.fill_rect(px as i32 - 2, py as i32 - 1, 5, 1, f);
        }
    }
}

/// A café's pavement: two small tables and their parasols, only in daylight
/// (B76: "summer: parasols and café tables out").
fn cafe(fb: &mut Framebuffer, cam: &Camera, o: &CityObj, hour: f32) {
    if !(8.0..21.0).contains(&hour) {
        return;
    }
    let base = o.level as i32 * HEIGHT_UNIT;
    let (x0, y0, x1, y1) = footprint(o);
    // The pavement in front of the door.
    let (fx, fy) = match o.front % 4 {
        0 => (x1 + 0.55, (y0 + y1) * 0.5),
        1 => ((x0 + x1) * 0.5, y1 + 0.55),
        2 => (x0 - 0.55, (y0 + y1) * 0.5),
        _ => ((x0 + x1) * 0.5, y0 - 0.55),
    };
    let along = if matches!(o.front % 4, 0) || matches!(o.front % 4, 2) {
        (0.0, 1.0)
    } else {
        (1.0, 0.0)
    };
    let cream = palette::rgb(0xf0, 0xe4, 0xc8);
    let pole = palette::rgb(0x6a, 0x5a, 0x44);
    let awning = [
        palette::rgb(0xc8, 0x3c, 0x38),
        palette::rgb(0x2e, 0x6a, 0x48),
        palette::rgb(0x2e, 0x50, 0x8a),
    ][(o.seed as usize / 7) % 3];
    for i in 0..2 {
        let d = (i as f32 - 0.5) * 1.1;
        let tx = fx + along.0 * d;
        let ty = fy + along.1 * d;
        let (sx, sy) = cam.project(tx, ty, base);
        let (sx, sy) = (sx as i32, sy as i32);
        // A round table (a 3-px disc) with two chairs.
        fb.fill_rect(sx - 1, sy - 2, 3, 1, cream);
        fb.fill_rect(sx, sy - 3, 1, 2, palette::shade(cream, 0.85));
        fb.put(sx - 3, sy - 1, palette::shade(cream, 0.8));
        fb.put(sx + 3, sy - 1, palette::shade(cream, 0.8));
        // The parasol: a pole and a striped canopy.
        for k in 1..7 {
            fb.put(sx, sy - 2 - k, pole);
        }
        let top = sy - 9;
        for (j, w) in [(0, 7), (1, 5), (2, 3)] {
            let c = if j % 2 == 0 { awning } else { cream };
            fb.fill_rect(sx - w / 2, top + j, w, 1, c);
        }
        fb.put(sx, top - 1, palette::shade(awning, 1.2));
    }
}
