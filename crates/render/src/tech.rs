//! Technology art (B49/D72): what the crews install once it is there —
//! utility poles and their overhead wires, the buried cable cabinets, cell
//! towers and the small cells on the lamp posts, rooftop solar and the solar
//! farm, wind turbines, charger bays, and the drones over a near-future city.
//!
//! Everything here is static art: the scene bakes it into the static layer once
//! and repaints only the tile a crew has just finished (B49 item 4). Only the
//! drones move, and they are drawn per frame like the other actors.
//!
//! The lamp head itself is street furniture (B41) and lives in `street.rs`; the
//! level (gas / sodium / LED) comes from the sim's technology path and is handed
//! to [`crate::street::draw_lamp`].

use std::collections::{BTreeMap, BTreeSet};

use crate::framebuffer::Framebuffer;
use crate::iso::Camera;
use crate::palette::{self, rgb};
use map::TileMap;

/// What the world has installed, in the form the renderer wants (B49).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TechLook {
    /// (x, y) of lamp posts with the head a crew fitted (0 gas, 1 sodium, 2 LED).
    pub lamps: Vec<(i32, i32, u8)>,
    /// The world's latest lamp level: what a lamp with no entry of its own wears.
    pub lamp_base: u8,
    pub poles: Vec<(i32, i32)>,
    pub buried: Vec<(i32, i32)>,
    pub towers: Vec<(i32, i32)>,
    pub cells: Vec<(i32, i32)>,
    /// Rooftop solar, keyed by the building's own anchor tile (x, y, style).
    pub solar: Vec<(i32, i32, u8)>,
    pub farm: Vec<(i32, i32)>,
    pub turbines: Vec<(i32, i32)>,
    pub chargers: Vec<(i32, i32)>,
    pub drones: bool,
}

impl TechLook {
    pub fn is_empty(&self) -> bool {
        self.lamps.is_empty()
            && self.poles.is_empty()
            && self.buried.is_empty()
            && self.towers.is_empty()
            && self.cells.is_empty()
            && self.solar.is_empty()
            && self.farm.is_empty()
            && self.turbines.is_empty()
            && self.chargers.is_empty()
    }
}

/// The same picture, indexed by tile, for the static pass (O(1) per tile).
#[derive(Clone, Debug, Default)]
pub struct TechKey {
    pub lamps: BTreeMap<(i32, i32), u8>,
    pub lamp_base: u8,
    pub poles: BTreeSet<(i32, i32)>,
    pub buried: BTreeSet<(i32, i32)>,
    pub towers: BTreeSet<(i32, i32)>,
    pub cells: BTreeSet<(i32, i32)>,
    pub solar: BTreeMap<(i32, i32), u8>,
    pub farm: BTreeSet<(i32, i32)>,
    pub turbines: BTreeSet<(i32, i32)>,
    pub chargers: BTreeSet<(i32, i32)>,
}

impl TechKey {
    pub fn from_look(look: &TechLook) -> TechKey {
        let mut k = TechKey {
            lamp_base: look.lamp_base,
            ..TechKey::default()
        };
        for &(x, y, l) in &look.lamps {
            k.lamps.insert((x, y), l);
        }
        for &(x, y) in &look.poles {
            k.poles.insert((x, y));
        }
        for &(x, y) in &look.buried {
            k.buried.insert((x, y));
        }
        for &(x, y) in &look.towers {
            k.towers.insert((x, y));
        }
        for &(x, y) in &look.cells {
            k.cells.insert((x, y));
        }
        for &(x, y, s) in &look.solar {
            k.solar.insert((x, y), s);
        }
        for &(x, y) in &look.farm {
            k.farm.insert((x, y));
        }
        for &(x, y) in &look.turbines {
            k.turbines.insert((x, y));
        }
        for &(x, y) in &look.chargers {
            k.chargers.insert((x, y));
        }
        k
    }

    /// The head a lamp tile wears: what a crew fitted, else the world's latest.
    pub fn lamp_level(&self, x: i32, y: i32) -> u8 {
        self.lamps.get(&(x, y)).copied().unwrap_or(self.lamp_base)
    }
}

// --- palettes --------------------------------------------------------------

/// Weathered timber, as a pole looks after a winter.
const POLE_WOOD: palette::Color = rgb(0x6a, 0x52, 0x38);
const POLE_WOOD_LIT: palette::Color = rgb(0x86, 0x6a, 0x4a);
const WIRE: palette::Color = rgb(0x3a, 0x3c, 0x42);
/// The steel of a cell tower or a turbine mast.
const STEEL: palette::Color = rgb(0xb4, 0xb8, 0xbe);
const STEEL_DARK: palette::Color = rgb(0x84, 0x88, 0x90);
/// Solar glass: deep blue with a pale sheen along the top edge.
const PANEL: palette::Color = rgb(0x22, 0x36, 0x58);
const PANEL_LIT: palette::Color = rgb(0x46, 0x66, 0x96);
const PANEL_FRAME: palette::Color = rgb(0x8a, 0x8e, 0x94);
/// The green cabinet that replaces a pole once the cable is underground.
const CABINET: palette::Color = rgb(0x3e, 0x5c, 0x42);
const CABINET_LIT: palette::Color = rgb(0x54, 0x76, 0x56);
/// A charger bay's post and its little green light.
const CHARGER: palette::Color = rgb(0xdc, 0xdf, 0xe2);
const CHARGER_DARK: palette::Color = rgb(0x7a, 0x80, 0x86);

/// The tile's screen centre and its two iso axes (a whole tile wide, so the
/// offsets below are in pixels, not in tiles — the lesson of B42g).
fn geom(cam: &Camera, map: &TileMap, x: i32, y: i32) -> ((f32, f32), (f32, f32), (f32, f32)) {
    let z = crate::iso::level_px(map.surface_level_at(x as f32 + 0.5, y as f32 + 0.5));
    let c = cam.project(x as f32 + 0.5, y as f32 + 0.5, z);
    let ax = cam.project(x as f32 + 1.5, y as f32 + 0.5, z);
    let ay = cam.project(x as f32 + 0.5, y as f32 + 1.5, z);
    (
        (c.0, c.1),
        (ax.0 - c.0, ax.1 - c.1),
        (ay.0 - c.0, ay.1 - c.1),
    )
}

fn runs_along_x(map: &TileMap, x: i32, y: i32) -> bool {
    map.at(x - 1, y).kind.is_lane() || map.at(x + 1, y).kind.is_lane()
}

fn unit(v: (f32, f32)) -> (f32, f32) {
    let l = (v.0 * v.0 + v.1 * v.1).sqrt();
    if l < 0.001 {
        (0.0, 0.0)
    } else {
        (v.0 / l, v.1 / l)
    }
}

/// Where a pole stands (at the kerb) and how tall it is: the foot, and the top
/// where the wires meet.
fn pole_geom(cam: &Camera, map: &TileMap, x: i32, y: i32) -> ((f32, f32), (f32, f32), f32) {
    let (c, ux, uy) = geom(cam, map, x, y);
    let along_x = runs_along_x(map, x, y);
    let (px, py) = if along_x { (0, 1) } else { (1, 0) };
    let side = if !map.at(x + px, y + py).kind.is_lane() {
        1.0
    } else {
        -1.0
    };
    let v = unit(if along_x { uy } else { ux });
    let foot = (c.0 + v.0 * 10.0 * side, c.1 + v.1 * 10.0 * side);
    let h = 26.0;
    ((foot.0, foot.1), (foot.0, foot.1 - h), h)
}

/// A utility pole: timber, two crossarms, and the insulators that hold the
/// wires (B49 item 2: poles and overhead wires up at the kerb).
pub fn draw_pole(fb: &mut Framebuffer, cam: &Camera, map: &TileMap, x: i32, y: i32) {
    let ((fx, fy), (tx, ty), h) = pole_geom(cam, map, x, y);
    let (bx, by) = (fx as i32, fy as i32);
    // The trunk, a lit edge on the sun side.
    fb.fill_rect(bx, by - h as i32, 2, h as i32, POLE_WOOD);
    fb.fill_rect(bx + 1, by - h as i32, 1, h as i32, POLE_WOOD_LIT);
    fb.fill_rect(bx - 1, by - 1, 4, 2, palette::rgb(0x4a, 0x3a, 0x2a));
    // Two crossarms near the top, with a ceramic on each end.
    for (dy, half) in [(-(h as i32) + 3, 4), (-(h as i32) + 8, 3)] {
        fb.fill_rect(bx - half, by + dy, half as i32 * 2 + 2, 1, POLE_WOOD);
        for ex in [bx - half, bx + half + 1] {
            fb.put(ex, by + dy - 1, palette::rgb(0xc8, 0xc4, 0xb4));
        }
    }
    let _ = ty;
    let _ = tx;
}

/// The wires between two poles, with a sag that grows with the span (B49b: the
/// poles stand every fourth tile, so a span is several tiles long and a single
/// short segment would read as a floating stick).
pub fn draw_wire(
    fb: &mut Framebuffer,
    cam: &Camera,
    map: &TileMap,
    ax: i32,
    ay: i32,
    bx: i32,
    by: i32,
    span: i32,
) {
    let (_, (tx0, ty0), _) = pole_geom(cam, map, ax, ay);
    let (_, (tx1, ty1), _) = pole_geom(cam, map, bx, by);
    let sag = 1.0 + span as f32 * 0.9;
    for off in [0.0f32, 3.0] {
        let a = (tx0, ty0 + off);
        let b = (tx1, ty1 + off);
        // Two spans and a midpoint drop: a catenary, cheap.
        let mid = ((a.0 + b.0) * 0.5, (a.1 + b.1) * 0.5 + sag);
        let q1 = ((a.0 + mid.0) * 0.5, (a.1 + mid.1) * 0.5 + sag * 0.25);
        let q2 = ((b.0 + mid.0) * 0.5, (b.1 + mid.1) * 0.5 + sag * 0.25);
        fb.line(a, q1, WIRE);
        fb.line(q1, mid, WIRE);
        fb.line(mid, q2, WIRE);
        fb.line(q2, b, WIRE);
    }
}

/// The cabinet left where the overhead lines went underground (B49: the poles
/// in the core come down and the wires are buried).
pub fn draw_cabinet(fb: &mut Framebuffer, cam: &Camera, map: &TileMap, x: i32, y: i32) {
    let (c, ux, uy) = geom(cam, map, x, y);
    let along_x = runs_along_x(map, x, y);
    let (px, py) = if along_x { (0, 1) } else { (1, 0) };
    let side = if !map.at(x + px, y + py).kind.is_lane() {
        1.0
    } else {
        -1.0
    };
    let v = unit(if along_x { uy } else { ux });
    let (fx, fy) = (c.0 + v.0 * 11.0 * side, c.1 + v.1 * 11.0 * side);
    let (bx, by) = (fx as i32, fy as i32);
    // A box on a concrete pad, with a vent slot and a pale lid.
    fb.fill_rect(bx - 1, by - 1, 5, 1, palette::rgb(0x9a, 0x9a, 0x9c));
    fb.fill_rect(bx, by - 7, 4, 6, CABINET);
    fb.fill_rect(bx + 1, by - 7, 2, 6, CABINET_LIT);
    fb.fill_rect(bx, by - 8, 4, 1, palette::rgb(0x8e, 0x9c, 0x8e));
    fb.fill_rect(bx + 1, by - 5, 2, 1, palette::rgb(0x28, 0x3a, 0x2c));
    let _ = ux;
}

/// A cell tower: a lattice mast with a dish and a whip aerial on a small fenced
/// pad (B49 item 2: the towers appear).
pub fn draw_tower(fb: &mut Framebuffer, cam: &Camera, map: &TileMap, x: i32, y: i32) {
    let (c, _ux, _uy) = geom(cam, map, x, y);
    let (bx, by) = (c.0 as i32, c.1 as i32);
    let h = 56i32;
    // The pad and its fence.
    fb.fill_rect(bx - 7, by - 1, 15, 2, palette::rgb(0x9c, 0x9c, 0x98));
    for k in 0..3 {
        fb.put(bx - 7 + k * 6, by - 3, palette::rgb(0x6c, 0x6c, 0x68));
    }
    // The mast: two legs tapering to a point, with a cross-brace every few px.
    for i in 0..h {
        let t = i as f32 / h as f32;
        let half = (6.0 * (1.0 - t * 0.82)).round() as i32;
        let y = by - 2 - i;
        fb.put(bx - half, y, STEEL);
        fb.put(bx + half, y, STEEL_DARK);
        if i % 6 == 0 {
            fb.line(
                (bx as f32 - half as f32, y as f32),
                (bx as f32 + half as f32, y as f32),
                STEEL_DARK,
            );
        }
    }
    let top = by - 2 - h;
    // The platform: three panel antennas and a dish.
    fb.fill_rect(bx - 3, top + 2, 7, 5, palette::rgb(0xe0, 0xdc, 0xd0));
    fb.fill_rect(bx - 3, top + 2, 7, 1, palette::rgb(0xf2, 0xf0, 0xe8));
    // A microwave dish hanging off one side.
    fb.fill_rect(bx + 4, top + 6, 3, 3, palette::rgb(0xf0, 0xee, 0xe4));
    fb.fill_rect(bx + 5, top + 7, 1, 1, palette::rgb(0x8a, 0x8e, 0x94));
    // The whip aerial, and the red light on top of a real mast.
    fb.fill_rect(bx, top - 5, 1, 6, STEEL);
    fb.put(bx, top - 6, palette::rgb(0xd8, 0x3a, 0x30));
}

/// A 5G small cell: a slim box and its antenna bolted to a lamp post (B49).
pub fn draw_small_cell(fb: &mut Framebuffer, cam: &Camera, map: &TileMap, x: i32, y: i32) {
    let (c, ux, uy) = geom(cam, map, x, y);
    let along_x = runs_along_x(map, x, y);
    let (px, py) = if along_x { (0, 1) } else { (1, 0) };
    let side = if !map.at(x + px, y + py).kind.is_lane() {
        1.0
    } else {
        -1.0
    };
    let v = unit(if along_x { uy } else { ux });
    let (fx, fy) = (c.0 + v.0 * 11.0 * side, c.1 + v.1 * 11.0 * side);
    let (bx, by) = (fx as i32, fy as i32);
    // Up the post, below the head: a grey box with two pale aerials.
    fb.fill_rect(bx, by - 15, 3, 4, palette::rgb(0x5a, 0x5e, 0x64));
    fb.fill_rect(bx + 1, by - 15, 1, 4, palette::rgb(0x7a, 0x7e, 0x86));
    fb.put(bx, by - 17, palette::rgb(0xc8, 0xd0, 0xd8));
    fb.put(bx + 2, by - 17, palette::rgb(0xc8, 0xd0, 0xd8));
    let _ = ux;
}

/// Rooftop solar on a finished building: a shallow array of panels lying on the
/// roof, drawn in the building's own footprint so it never floats (B49 item 2).
///
/// The panel plane sits at the wall head plus the roof's own rise, and is inset
/// from the walls, which reads as "on the roof" from this camera without a
/// bespoke roof map per building kind.
pub fn draw_solar_roof(
    fb: &mut Framebuffer,
    cam: &Camera,
    o: &crate::city::CityObj,
    style: u8,
    top_px: i32,
) {
    let base = o.level as i32 * crate::iso::HEIGHT_UNIT;
    let top = base + top_px - 2;
    // The glass, in this roof's own tone, so a street of panels does not read as
    // one big blue sheet.
    let tone = match style % 3 {
        0 => PANEL,
        1 => palette::mix(PANEL, rgb(0x2a, 0x2c, 0x40), 0.35),
        _ => palette::mix(PANEL, rgb(0x1c, 0x40, 0x40), 0.30),
    };
    let mut quad =
        |a: (f32, f32, i32), b: (f32, f32, i32), c2: (f32, f32, i32), d: (f32, f32, i32)| {
            let p = [
                cam.project(a.0, a.1, a.2),
                cam.project(b.0, b.1, b.2),
                cam.project(c2.0, c2.1, c2.2),
                cam.project(d.0, d.1, d.2),
            ];
            for i in 0..4 {
                fb.line(p[i], p[(i + 1) % 4], PANEL_FRAME);
            }
            fb.fill_poly(&p, tone);
            let hi = p
                .iter()
                .fold(p[0], |acc, q| if q.1 < acc.1 { *q } else { acc });
            fb.put(hi.0 as i32, hi.1 as i32, PANEL_LIT);
        };
    // The roof is smaller than the lot (the art insets the walls from the plot),
    // so the array is laid over the *middle* of the roof, not the plot edge.
    let inset = 0.34f32;
    let (x0, y0, x1, y1) = (
        o.x as f32 + inset,
        o.y as f32 + inset,
        (o.x + o.w) as f32 - inset,
        (o.y + o.h) as f32 - inset,
    );
    if x1 - x0 < 0.35 || y1 - y0 < 0.35 {
        return;
    }
    if o.kind.is_massing() {
        // A flat roof: the array lies on the deck, inset from the parapet, in one
        // or two rows so a wide tower reads as a row of panels.
        let (dx0, dy0, dx1, dy1) = (x0 + 0.10, y0 + 0.10, x1 - 0.10, y1 - 0.10);
        let cols = if o.w.min(o.h) >= 3 { 2 } else { 1 };
        for c in 0..cols {
            let ax = dx0 + (dx1 - dx0) * (c as f32 / cols as f32) + 0.05;
            let bx = dx0 + (dx1 - dx0) * ((c + 1) as f32 / cols as f32) - 0.05;
            quad(
                (ax, dy0, top),
                (bx, dy0, top),
                (bx, dy1, top),
                (ax, dy1, top),
            );
        }
    } else {
        // A pitched roof: one row of panels on the slope the camera sees, from
        // the eave up toward the ridge, so it lies *on* the roof instead of
        // floating over it.
        let pitch = if o.w.min(o.h) >= 3 { 4 } else { 6 };
        let eave = top - pitch;
        let ymid = y0 + (y1 - y0) * 0.55;
        let cols = if o.w.min(o.h) >= 3 { 2 } else { 1 };
        for c in 0..cols {
            let ax = x0 + (x1 - x0) * (c as f32 / cols as f32) + 0.06;
            let bx = x0 + (x1 - x0) * ((c + 1) as f32 / cols as f32) - 0.06;
            quad(
                (ax, ymid, top),
                (bx, ymid, top),
                (bx, y1, eave),
                (ax, y1, eave),
            );
        }
    }
}

/// Rooftop solar on a **kit** building (B49, redone for B49b item 1).
///
/// The array is a **rectangular grid laid along the roof's iso axes**: the two
/// edge directions are the tile axes projected (a step along `+x` is `(+32,+16)`
/// on screen, along `+y` it is `(-32,+16)`), so the cells are parallelograms
/// that follow the roof edges — rows parallel to the ridge, columns down the
/// slope — and never fan out from a point.
///
/// The kit's buildings are art, not geometry, so the roof is found by reading
/// the sprite: the opaque rows just under its top are the roof band, its
/// silhouette bounds the grid, and a cell whose centre lands on a **rooftop
/// box** (a pixel much brighter than the deck) or off the roof is skipped —
/// which is the margin the planner asked for around parapets and plant.
pub fn draw_solar_on_sprite(
    fb: &mut Framebuffer,
    k: &crate::scene::KitSprite,
    sx: i32,
    sy: i32,
    style: u8,
) {
    if k.h < 10 || k.oy < 14 || k.w < 12 {
        return;
    }
    let hh = k.oy as f32;
    // A tall block has a flat roof with a parapet; a cottage has a pitched roof
    // whose near slope is the lower half of its top.
    let flat = k.oy >= 36;
    let (band_a, band_b) = if flat {
        (0.04f32, 0.18f32)
    } else {
        (0.11f32, 0.36f32)
    };
    let ra = ((hh * band_a) as usize).max(1);
    let rb = ((hh * band_b) as usize).max(ra + 3).min(k.h - 1);
    let mut x_lo = i32::MAX;
    let mut x_hi = i32::MIN;
    let mut deck_lum: Vec<f32> = Vec::new();
    for row in ra..=rb {
        let mut lo = i32::MAX;
        let mut hi = i32::MIN;
        for x in 0..k.w {
            let c = k.px[row * k.w + x];
            if c == palette::TRANSPARENT {
                continue;
            }
            lo = lo.min(x as i32);
            hi = hi.max(x as i32);
            deck_lum.push(palette::luma(c) as f32);
        }
        if hi >= lo {
            x_lo = x_lo.min(lo);
            x_hi = x_hi.max(hi);
        }
    }
    if x_hi - x_lo < 10 || deck_lum.is_empty() {
        return;
    }
    deck_lum.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let deck = deck_lum[deck_lum.len() / 2];
    // Inset from the silhouette: the array keeps a clear margin from the parapet
    // and the eaves.
    let inset_x = ((x_hi - x_lo) as f32 * 0.16) as i32 + 2;
    let (bx, by) = (sx - k.ox, sy - k.oy);
    // The two roof axes on screen, and one cell's step along each (in tiles).
    let (ax, ay) = (32.0f32, 16.0f32);
    let cell_u = 0.28f32; // along +x (the ridge on a pitched roof)
    let cell_v = 0.22f32; // down the near slope (+y)
    let (du, dv) = ((ax * cell_u, ay * cell_u), (-ax * cell_v, ay * cell_v));
    let tone = match style % 3 {
        0 => PANEL,
        1 => palette::mix(PANEL, rgb(0x2a, 0x2c, 0x40), 0.35),
        _ => palette::mix(PANEL, rgb(0x1c, 0x40, 0x40), 0.30),
    };
    // The grid's origin: the roof band's top-left, stepped in by the margin.
    let origin = (
        (bx + x_lo + inset_x) as f32,
        (by + ra as i32) as f32 + (hh * 0.02),
    );
    let span_x = (x_hi - x_lo - inset_x * 2) as f32;
    let span_y = (rb.saturating_sub(ra + 1)) as f32;
    let n_u = ((span_x / (ax * cell_u)) as i32 + 1).clamp(1, 5);
    let n_v = ((span_y / (ay * cell_v)) as i32 + 1).clamp(1, 3);
    let opaque_at = |px: i32, py: i32| -> Option<palette::Color> {
        if px < 0 || py < 0 || px >= k.w as i32 || py >= k.h as i32 {
            return None;
        }
        let c = k.px[py as usize * k.w + px as usize];
        (c != palette::TRANSPARENT).then_some(c)
    };
    for iv in 0..n_v {
        for iu in 0..n_u {
            let (ox, oy) = (
                origin.0 + du.0 * iu as f32 + dv.0 * iv as f32,
                origin.1 + du.1 * iu as f32 + dv.1 * iv as f32,
            );
            // Cell corners, shrunk a pixel so the roof shows between rows.
            // A two-pixel gap on every side, so the roof shows between the rows
            // (and between the columns) at 1×.
            let quad = [
                (ox + 0.5, oy + 1.0),
                (ox + du.0 - 1.5, oy + du.1),
                (ox + du.0 + dv.0 - 1.5, oy + du.1 + dv.1 - 1.5),
                (ox + dv.0 + 0.5, oy + dv.1 - 1.5),
            ];
            // Off the roof, or on a rooftop box (much brighter than the deck):
            // no panel there. **All four corners** are tested, so a panel never
            // hangs over an eave or half onto a plant room.
            let mut fits = true;
            for (qx, qy) in quad {
                let Some(px) = opaque_at((qx - bx as f32) as i32, (qy - by as f32) as i32) else {
                    fits = false;
                    break;
                };
                if palette::luma(px) as f32 > deck + 46.0 {
                    fits = false;
                    break;
                }
            }
            if !fits {
                continue;
            }
            // A light frame, the glass, and the cell rows a shade darker.
            for i in 0..4 {
                fb.line(quad[i], quad[(i + 1) % 4], PANEL_FRAME);
            }
            fb.fill_poly(&quad, tone);
            let mid = (
                (quad[0].0 + quad[1].0) * 0.5 + (quad[3].0 - quad[0].0) * 0.5,
                (quad[0].1 + quad[1].1) * 0.5 + (quad[3].1 - quad[0].1) * 0.5,
            );
            let a = (
                quad[0].0 + (mid.0 - quad[0].0) * 0.55,
                quad[0].1 + (mid.1 - quad[0].1) * 0.55,
            );
            let b = (
                quad[1].0 + (mid.0 - quad[1].0) * 0.55,
                quad[1].1 + (mid.1 - quad[1].1) * 0.55,
            );
            fb.line(a, b, palette::shade(tone, 0.80));
            // A sheen along the highest edge: glass catching the light.
            fb.line(quad[0], quad[1], PANEL_LIT);
        }
    }
}

/// A row of solar panels on the ground: the solar farm at the edge of the city
/// (B49 item 2). Each tile carries a tilted panel and its frame, all facing the
/// same way, so the farm reads as one installation.
pub fn draw_farm(fb: &mut Framebuffer, cam: &Camera, map: &TileMap, x: i32, y: i32) {
    let (c, ux, uy) = geom(cam, map, x, y);
    // Two rows of panels per tile, tilted toward +x, standing off the ground.
    for r in 0..2 {
        let off = (r as f32 - 1.0) * 7.0;
        let y0 = c.1 + off;
        let w = 13.0f32;
        let a = (c.0 - w, y0 + 3.0);
        let b = (c.0 + w, y0 + 3.0);
        let c2 = (c.0 + w - 3.0, y0 - 5.0);
        let d = (c.0 - w + 3.0, y0 - 5.0);
        fb.line(a, d, PANEL_FRAME);
        fb.line(b, c2, PANEL_FRAME);
        fb.line(a, b, PANEL_FRAME);
        fb.line(d, c2, PANEL_FRAME);
        fb.fill_poly(&[a, b, c2, d], PANEL);
        // The legs, and a sheen along the top edge.
        fb.fill_rect(a.0 as i32, a.1 as i32, 1, 3, STEEL_DARK);
        fb.fill_rect(b.0 as i32, b.1 as i32, 1, 3, STEEL_DARK);
        fb.line(d, c2, PANEL_LIT);
    }
    let _ = (ux, uy);
}

/// How tall a turbine stands, in pixels above its pad (B49b item 2: tall enough
/// to read against the town — a 6-storey building is 52 px).
pub const TURBINE_H: i32 = 118;

/// Where a turbine's hub is on screen (the rotor turns about it).
pub fn turbine_hub(cam: &Camera, map: &TileMap, x: i32, y: i32) -> (f32, f32, i32) {
    let (c, _ux, _uy) = geom(cam, map, x, y);
    (c.0, c.1 - TURBINE_H as f32, c.1 as i32)
}

/// The **static** part of a turbine: the concrete pad, the tapered white tower
/// and the nacelle. The blades are drawn every frame (see
/// [`draw_turbine_rotor`]) so they turn at the configured fps (D71).
pub fn draw_turbine_mast(fb: &mut Framebuffer, cam: &Camera, map: &TileMap, x: i32, y: i32) {
    let (c, _ux, _uy) = geom(cam, map, x, y);
    let (bx, by) = (c.0 as i32, c.1 as i32);
    // The footing: a pale concrete pad, with a hard standing around it.
    fb.fill_rect(bx - 5, by - 1, 11, 2, palette::rgb(0xb0, 0xae, 0xa6));
    fb.fill_rect(bx - 3, by - 3, 7, 2, palette::rgb(0x9c, 0x9a, 0x94));
    // The tower: a slender taper, lit on the sun side.
    for i in 0..TURBINE_H {
        let t = i as f32 / TURBINE_H as f32;
        let half = (2.4 * (1.0 - t * 0.62)).round() as i32;
        let yy = by - 3 - i;
        fb.fill_rect(
            bx - half,
            yy,
            half * 2 + 1,
            1,
            if i % 3 == 0 { STEEL } else { STEEL_DARK },
        );
        fb.fill_rect(bx + half, yy, 1, 1, STEEL);
    }
    // The nacelle: a rounded housing behind the hub, and the red light on top.
    let hub_y = by - 3 - TURBINE_H;
    fb.fill_rect(bx - 5, hub_y - 2, 11, 5, palette::rgb(0xee, 0xf0, 0xf2));
    fb.fill_rect(bx - 5, hub_y + 2, 11, 1, palette::rgb(0xb8, 0xbc, 0xc2));
    fb.put(bx, hub_y - 3, palette::rgb(0xd8, 0x3a, 0x30));
}

/// The rotor: hub and three blades, turning with the clock. Drawn in the dynamic
/// pass, so the scene damages only the rotor's own rectangle each frame.
pub fn draw_turbine_rotor(
    fb: &mut Framebuffer,
    cam: &Camera,
    map: &TileMap,
    x: i32,
    y: i32,
    t: f32,
) {
    let (hx, hy, _) = turbine_hub(cam, map, x, y);
    // One turn every ~14 s: calm, and it never strobes at 30 fps.
    let phase = t * (std::f32::consts::TAU / 14.0) + (x * 7 + y * 13) as f32 * 0.11;
    let len = 26.0f32;
    for k in 0..3 {
        let a = phase + k as f32 * std::f32::consts::TAU / 3.0;
        let (dx, dy) = (a.cos(), a.sin() * 0.86);
        let tip = (hx + dx * len, hy + dy * len);
        fb.line((hx, hy), tip, palette::rgb(0xf2, 0xf4, 0xf6));
        // The blade is widest near the hub: a second pixel row for the body.
        let mid = (hx + dx * len * 0.45, hy + dy * len * 0.45);
        fb.line((hx, hy), mid, palette::rgb(0xdc, 0xe0, 0xe6));
        // A pale tip, so a blade against the sky still reads.
        fb.put(tip.0 as i32, tip.1 as i32, palette::rgb(0xfa, 0xfc, 0xff));
    }
    // The hub cap, over the blade roots.
    fb.fill_rect(
        hx as i32 - 2,
        hy as i32 - 2,
        4,
        4,
        palette::rgb(0xf4, 0xf6, 0xf8),
    );
    fb.put(hx as i32 - 1, hy as i32 - 1, palette::rgb(0xc8, 0xcc, 0xd2));
}

/// The screen rect a rotor sweeps (for the scene's damage bookkeeping).
pub fn rotor_rect(cam: &Camera, map: &TileMap, x: i32, y: i32) -> crate::framebuffer::Rect {
    let (hx, hy, _) = turbine_hub(cam, map, x, y);
    crate::framebuffer::Rect::new(
        hx as i32 - 32,
        hy as i32 - 30,
        hx as i32 + 33,
        hy as i32 + 31,
    )
}

/// A charger bay: a white post with a cable and a green light, beside the kerb
/// (B49 item 2: the filling stations and car parks gain charger bays).
pub fn draw_charger(fb: &mut Framebuffer, cam: &Camera, map: &TileMap, x: i32, y: i32) {
    let (c, ux, uy) = geom(cam, map, x, y);
    let along_x = runs_along_x(map, x, y);
    let (px, py) = if along_x { (0, 1) } else { (1, 0) };
    let side = if !map.at(x + px, y + py).kind.is_lane() {
        1.0
    } else {
        -1.0
    };
    let v = unit(if along_x { uy } else { ux });
    let (fx, fy) = (c.0 + v.0 * 12.0 * side, c.1 + v.1 * 12.0 * side);
    let (bx, by) = (fx as i32, fy as i32);
    // The post, its screen, and the cable looping to the ground.
    fb.fill_rect(bx, by - 9, 3, 8, CHARGER);
    fb.fill_rect(bx + 1, by - 9, 1, 8, CHARGER_DARK);
    fb.fill_rect(bx, by - 8, 3, 3, palette::rgb(0x30, 0x38, 0x40));
    fb.put(bx + 1, by - 7, palette::rgb(0x64, 0xe0, 0x8c));
    fb.line(
        (bx as f32 + 3.0, by as f32 - 4.0),
        (bx as f32 + 5.0, by as f32 - 1.0),
        CHARGER_DARK,
    );
    // A painted bay beside it: two white lines and a little bolt glyph.
    fb.fill_rect(bx - 4, by, 10, 1, palette::rgb(0xe0, 0xdc, 0xd0));
    let _ = ux;
}

// --- the crews on the ground (B49b item 3) ---------------------------------

/// One pixel through the static layer's depth plane: a crew is a *thing in the
/// world*, so a house in front of it hides it (the van used to be painted over
/// the roofs).
fn put_d(fb: &mut Framebuffer, depth: &[u16], key: u16, x: i32, y: i32, c: palette::Color) {
    let clip = fb.clip();
    if x < clip.x0 || x >= clip.x1 || y < clip.y0 || y >= clip.y1 {
        return;
    }
    let i = y as usize * fb.w + x as usize;
    if depth.get(i).is_some_and(|d| *d > key) {
        return;
    }
    fb.put(x, y, c);
}

fn rect_d(
    fb: &mut Framebuffer,
    depth: &[u16],
    key: u16,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    c: palette::Color,
) {
    for yy in y..y + h {
        for xx in x..x + w {
            put_d(fb, depth, key, xx, yy, c);
        }
    }
}

fn line_d(
    fb: &mut Framebuffer,
    depth: &[u16],
    key: u16,
    a: (f32, f32),
    b: (f32, f32),
    c: palette::Color,
) {
    let (x0, y0) = (a.0.round() as i32, a.1.round() as i32);
    let (x1, y1) = (b.0.round() as i32, b.1.round() as i32);
    let (dx, dy) = ((x1 - x0).abs(), -(y1 - y0).abs());
    let (sx, sy) = (if x0 < x1 { 1 } else { -1 }, if y0 < y1 { 1 } else { -1 });
    let (mut x, mut y) = (x0, y0);
    let mut err = dx + dy;
    loop {
        put_d(fb, depth, key, x, y, c);
        if x == x1 && y == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
    }
}

/// A technology crew at work, drawn per frame and **depth-tested** (B49b item 3).
///
/// * The **van** stands on the road, *on the tile's lane*, never on a roof: it
///   is anchored at the tile's surface and hidden by anything in front.
/// * The **worker** stands on the pavement (the kerb side) with a ladder up to
///   the lamp head, so the job reads as work on the street furniture.
///
/// `kind` is the sim's crew kind: 4 = fitting crew (van + ladder + worker),
/// 3 = pole crew (van + two workers), anything else = a worker on the pavement.
pub fn draw_crew(
    fb: &mut Framebuffer,
    cam: &Camera,
    map: &TileMap,
    x: i32,
    y: i32,
    kind: u8,
    depth: &[u16],
    key: u16,
) {
    let (c, ux, uy) = geom(cam, map, x, y);
    let along_x = runs_along_x(map, x, y);
    // Across the street: the kerb side is the one that is not a lane.
    let (px, py) = if along_x { (0, 1) } else { (1, 0) };
    let side = if !map.at(x + px, y + py).kind.is_lane() {
        1.0
    } else {
        -1.0
    };
    let v = unit(if along_x { uy } else { ux });
    let u = unit(if along_x { ux } else { uy });
    // The pavement point, and the road point (the van).
    let (wx, wy) = (c.0 + v.0 * 11.0 * side, c.1 + v.1 * 11.0 * side);
    let (vx, vy) = (
        c.0 - u.0 * 2.0 + v.0 * 3.0 * side,
        c.1 - u.1 * 2.0 + v.1 * 3.0 * side,
    );
    let van = kind >= 3;
    if van {
        let (bx, by) = (vx as i32 - 8, vy as i32 - 9);
        // Body, cabin windows, a beacon on the roof, four wheels.
        rect_d(
            fb,
            depth,
            key,
            bx,
            by,
            16,
            7,
            palette::rgb(0xf2, 0xf0, 0xe8),
        );
        rect_d(
            fb,
            depth,
            key,
            bx,
            by,
            16,
            1,
            palette::rgb(0xd8, 0xd4, 0xca),
        );
        rect_d(
            fb,
            depth,
            key,
            bx + 2,
            by + 2,
            5,
            3,
            palette::rgb(0x8a, 0x96, 0xa4),
        );
        rect_d(
            fb,
            depth,
            key,
            bx + 9,
            by + 2,
            5,
            3,
            palette::rgb(0x9a, 0xa6, 0xb4),
        );
        rect_d(
            fb,
            depth,
            key,
            bx + 5,
            by + 2,
            3,
            3,
            palette::rgb(0xe6, 0xe2, 0xd6),
        );
        if kind >= 4 {
            // The fitting crew's beacon and its roof ladder rack.
            rect_d(
                fb,
                depth,
                key,
                bx + 2,
                by - 1,
                4,
                2,
                palette::rgb(0xe0, 0x7a, 0x28),
            );
            put_d(
                fb,
                depth,
                key,
                bx + 3,
                by - 2,
                palette::rgb(0xf4, 0xd0, 0x40),
            );
        } else {
            rect_d(
                fb,
                depth,
                key,
                bx + 2,
                by - 1,
                5,
                1,
                palette::rgb(0x6a, 0x6e, 0x76),
            );
        }
        for wx2 in [bx + 2, bx + 11] {
            rect_d(
                fb,
                depth,
                key,
                wx2,
                by + 7,
                3,
                2,
                palette::rgb(0x28, 0x28, 0x2e),
            );
        }
    }
    // The worker: orange vest, helmet, boots — on the pavement.
    let (people_px, people_py) = (wx as i32, wy as i32);
    let px0 = if van { people_px + 6 } else { people_px };
    let py0 = people_py;
    rect_d(fb, depth, key, px0, py0 - 4, 3, 5, rgb(0xf0, 0x8a, 0x2a));
    rect_d(
        fb,
        depth,
        key,
        px0,
        py0 - 7,
        3,
        3,
        palette::rgb(0xe8, 0xc4, 0xa0),
    );
    rect_d(fb, depth, key, px0, py0 - 7, 3, 1, rgb(0xf4, 0xd0, 0x40));
    put_d(fb, depth, key, px0, py0, palette::rgb(0x3a, 0x3a, 0x44));
    put_d(fb, depth, key, px0 + 2, py0, palette::rgb(0x3a, 0x3a, 0x44));
    if kind >= 4 {
        // A ladder up to the lamp head: two rails and rungs, depth-tested.
        let top = (px0 as f32 + 1.0, (py0 - 22) as f32);
        line_d(fb, depth, key, (px0 as f32, py0 as f32 - 1.0), top, STEEL);
        line_d(
            fb,
            depth,
            key,
            (px0 as f32 + 2.0, py0 as f32 - 1.0),
            (top.0 + 2.0, top.1),
            STEEL_DARK,
        );
        for r in 0..5 {
            let t = r as f32 / 5.0;
            let yy = py0 as f32 - 1.0 - 21.0 * t;
            line_d(
                fb,
                depth,
                key,
                (px0 as f32, yy),
                (px0 as f32 + 2.0, yy),
                STEEL_DARK,
            );
        }
    } else {
        // A tool over the shoulder.
        line_d(
            fb,
            depth,
            key,
            (px0 as f32 + 4.0, py0 as f32 - 3.0),
            (px0 as f32 + 4.0, py0 as f32 - 15.0),
            rgb(0x6a, 0x6e, 0x76),
        );
    }
}

/// A delivery drone (B49/D72, near-future worlds): a tiny body with two rotors,
/// seen from above, drawn per frame while it drifts over the city by day.
pub fn draw_drone(
    fb: &mut Framebuffer,
    cam: &Camera,
    map: &TileMap,
    x: f32,
    y: f32,
    z: f32,
    t: f32,
) {
    let zz = crate::iso::level_px(map.surface_level_at(x, y));
    // B49b item 5: a quadcopter at roof height (a house is ~24 px, a tower 100).
    let hover = 58.0 + (t * 0.7 + x).sin() * 2.5 + z;
    let p = cam.project(x, y, zz + hover as i32);
    let (px, py) = (p.0 as i32, p.1 as i32);
    // The rotors: four blurred arms, flickering with the clock.
    let spin = ((t * 11.0).sin() * 0.5 + 0.5) > 0.5;
    let rotor = if spin {
        palette::rgb(0xd8, 0xdc, 0xe2)
    } else {
        palette::rgb(0xa8, 0xac, 0xb4)
    };
    fb.fill_rect(px - 4, py - 1, 3, 1, rotor);
    fb.fill_rect(px + 2, py - 1, 3, 1, rotor);
    fb.put(px - 4, py, palette::rgb(0x6a, 0x6e, 0x76));
    fb.put(px + 4, py, palette::rgb(0x6a, 0x6e, 0x76));
    // The body, a camera pod, and a green running light.
    fb.fill_rect(px - 1, py, 3, 2, palette::rgb(0xe4, 0xe8, 0xee));
    fb.fill_rect(px - 1, py + 1, 3, 1, palette::rgb(0x9a, 0xa0, 0xa8));
    fb.put(px, py + 2, palette::rgb(0x60, 0xe0, 0x90));
    // The parcel box slung under it.
    fb.fill_rect(px - 1, py + 3, 3, 2, palette::rgb(0xc8, 0x9a, 0x50));
    fb.put(px, py + 3, palette::rgb(0xe0, 0xc0, 0x88));
    // Its soft shadow on the ground far below (offset by the sun, faint).
    let g = cam.project(x, y, zz + 1);
    for (dx, dy, a) in [(0, 0, 0.18), (1, 0, 0.12), (-1, 0, 0.12), (0, 1, 0.10)] {
        fb.put_blended(
            g.0 as i32 + dx,
            g.1 as i32 + dy,
            palette::rgb(0x14, 0x16, 0x1c),
            a,
        );
    }
}
