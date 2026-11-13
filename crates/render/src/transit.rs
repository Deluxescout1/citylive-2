//! The transport the city builds: the station, the bus and tram shelters, the
//! subway works, the airfield and the harbour (B58, D72).
//!
//! Everything here is **static** art: it is painted into the scene's static
//! layer through [`Fixtures`](crate::rail::Fixtures), so it costs nothing per
//! frame. What moves — a barrier arm, a crane's trolley — is a mover body in
//! `rail.rs` and gets a dirty rect like every other moving thing.
//!
//! The style follows B32/B54: 64-px tiles, real materials, one light direction
//! (the sun is up and to the +x side, as everywhere else), and every structure
//! anchored on its own ground so nothing floats.

use crate::framebuffer::Framebuffer;
use crate::iso::{Camera, HEIGHT_UNIT};
use crate::palette::{self, Color};
use map::TileMap;

// Materials.
const BRICK: Color = palette::rgb(0x8a, 0x4a, 0x3a);
const BRICK_DARK: Color = palette::rgb(0x6e, 0x3a, 0x2e);
const SLATE: Color = palette::rgb(0x4a, 0x4e, 0x58);
const SLATE_LIT: Color = palette::rgb(0x60, 0x65, 0x6f);
const PLATFORM: Color = palette::rgb(0xcf, 0xc9, 0xbb);
const PLATFORM_LIP: Color = palette::rgb(0xa8, 0xa2, 0x94);
const YELLOW_LINE: Color = palette::rgb(0xe8, 0xc0, 0x40);
const GLASS: Color = palette::rgb(0x5a, 0x7a, 0x96);
const GLASS_LIT: Color = palette::rgb(0x86, 0xa8, 0xc0);
const CANOPY_TOP: Color = palette::rgb(0x3e, 0x44, 0x50);
const CANOPY_UNDER: Color = palette::rgb(0x76, 0x7c, 0x88);
const STEEL: Color = palette::rgb(0x8a, 0x8e, 0x96);
const DARK_STEEL: Color = palette::rgb(0x3a, 0x3e, 0x44);
const ASPHALT: Color = palette::rgb(0x3c, 0x3e, 0x44);
const ASPHALT_LIT: Color = palette::rgb(0x4a, 0x4d, 0x54);
const PAINT: Color = palette::rgb(0xe8, 0xe4, 0xd8);
const GRASS_STRIP: Color = palette::rgb(0x5e, 0x8a, 0x3e);
const GRASS_MOWN: Color = palette::rgb(0x6e, 0x9a, 0x46);
const CONCRETE: Color = palette::rgb(0x9a, 0x9a, 0x94);
const CONCRETE_LIT: Color = palette::rgb(0xb0, 0xb0, 0xa8);
const TERMINAL: Color = palette::rgb(0xb0, 0xb4, 0xbc);
const TERMINAL_DARK: Color = palette::rgb(0x8a, 0x8e, 0x96);
const HANGAR: Color = palette::rgb(0x8a, 0x8e, 0x8a);
const HANGAR_ROOF: Color = palette::rgb(0xb0, 0xb4, 0xae);
const KIOSK: Color = palette::rgb(0x2e, 0x5a, 0x7a);
const KIOSK_LIT: Color = palette::rgb(0x4a, 0x7a, 0x9a);
const TRENCH: Color = palette::rgb(0x24, 0x1f, 0x1a);
const TRENCH_WALL: Color = palette::rgb(0x4a, 0x40, 0x38);
const SPOIL: Color = palette::rgb(0x7a, 0x6a, 0x52);
const TIMBER: Color = palette::rgb(0xa8, 0x88, 0x5e);
const TIMBER_DARK: Color = palette::rgb(0x5a, 0x46, 0x32);
const HELMET: Color = palette::rgb(0xf0, 0xa0, 0x20);
const VEST: Color = palette::rgb(0xf0, 0xd0, 0x40);
const RED_LIGHT: Color = palette::rgb(0xe0, 0x40, 0x2a);
const BEACON: Color = palette::rgb(0xff, 0x5a, 0x3a);

/// A light: the static-layer tint leaves pixels tagged 0xFE alone, which is how
/// every lamp in the world stays lit at night (B58c: an untagged lamp is a dull
/// brown pixel after the night multiply).
#[inline]
fn emissive(c: Color) -> Color {
    palette::with_alpha(c, 0xFE)
}

#[inline]
fn z_of(map: &TileMap, x: i32, y: i32) -> i32 {
    map.at(x, y).min_level() as i32 * HEIGHT_UNIT
}

/// The four screen corners of a tile's top face.
#[inline]
fn quad(cam: &Camera, map: &TileMap, x: i32, y: i32) -> [(f32, f32); 4] {
    let t = map.at(x, y);
    cam.corners(x, y, t.corner.map(|v| v as f32))
}

/// Fill a tile's top face, grown by a pixel so two hard surfaces side by side
/// (runway tiles, apron slabs, quay stones) meet without a hairline of grass
/// showing through the seam (B58c: a runway with a green grid is not a runway).
fn fill_tile_grown(
    fb: &mut Framebuffer,
    cam: &Camera,
    map: &TileMap,
    x: i32,
    y: i32,
    c: Color,
    grow: f32,
    lift: i32,
) {
    let q = quad(cam, map, x, y);
    let cx = (q[0].0 + q[1].0 + q[2].0 + q[3].0) * 0.25;
    let cy = (q[0].1 + q[1].1 + q[2].1 + q[3].1) * 0.25;
    let mut pts = [(0.0f32, 0.0f32); 4];
    for (i, p) in q.iter().enumerate() {
        let (dx, dy) = (p.0 - cx, p.1 - cy);
        let len = (dx * dx + dy * dy).sqrt().max(1e-3);
        pts[i] = (
            p.0 + dx / len * grow,
            p.1 + dy / len * grow - lift as f32,
        );
    }
    fb.fill_poly(&pts, c);
}

/// An axis-aligned box in tile space: top face plus the two camera-facing
/// sides. `(x, y)` is the box's own corner, in tiles from the tile's origin.
#[allow(clippy::too_many_arguments)]
pub fn box_at(
    fb: &mut Framebuffer,
    cam: &Camera,
    x: f32,
    y: f32,
    z: i32,
    w: f32,
    d: f32,
    h: i32,
    top: Color,
    left: Color,
    right: Color,
) {
    let p = |u: f32, v: f32, zz: i32| cam.project(u, v, zz);
    let (x0, y0, x1, y1) = (x, y, x + w, y + d);
    // The south-west and south-east faces are the ones the camera sees.
    fb.fill_poly(&[p(x0, y1, z + h), p(x1, y1, z + h), p(x1, y1, z), p(x0, y1, z)], left);
    fb.fill_poly(&[p(x1, y0, z + h), p(x1, y1, z + h), p(x1, y1, z), p(x1, y0, z)], right);
    fb.fill_poly(
        &[p(x0, y0, z + h), p(x1, y0, z + h), p(x1, y1, z + h), p(x0, y1, z + h)],
        top,
    );
}

/// A window: a lit or glass pane on a wall, in tile space.
#[allow(clippy::too_many_arguments)]
fn window_on(
    fb: &mut Framebuffer,
    cam: &Camera,
    x: f32,
    y: f32,
    z: i32,
    w: f32,
    h: i32,
    c: Color,
) {
    let p = |u: f32, v: f32, zz: i32| cam.project(u, v, zz);
    fb.fill_poly(&[p(x, y, z + h), p(x + w, y, z + h), p(x + w, y, z), p(x, y, z)], c);
}

// ------------------------------------------------------------ the station

/// One tile of the station hall (B58): brick walls on the tile, a slate roof
/// slab at a constant height above it, windows and a door on the camera side.
/// Drawn per tile so its own ground is painted first; on a flat site the slabs
/// line up into one roof.
#[allow(clippy::too_many_arguments)]
pub fn draw_station_tile(
    fb: &mut Framebuffer,
    cam: &Camera,
    map: &TileMap,
    x: i32,
    y: i32,
    zone: (i32, i32, i32, i32),
    kind: u8,
    glow: f32,
) {
    let (zx, zy, zw, zh) = zone;
    if x < zx || x >= zx + zw || y < zy || y >= zy + zh {
        return;
    }
    let z = z_of(map, x, y);
    // A storey is ~4 px here: a halt's hut is ~11 px, a town hall ~15, a
    // terminal ~19. (A 24-px hall read as a warehouse roof.)
    let h = if kind == 0 { 11 } else if kind == 1 { 15 } else { 19 };
    let (x0, y0) = (x as f32 + 0.04, y as f32 + 0.04);
    let (w, d) = (0.92f32, 0.92f32);
    let p = |u: f32, v: f32, zz: i32| cam.project(u, v, zz);
    // Plinth, then the brick body.
    box_at(fb, cam, x0, y0, z, w, d, 2, palette::shade(CONCRETE, 0.9), CONCRETE, palette::shade(CONCRETE, 0.8));
    box_at(fb, cam, x0, y0, z + 2, w, d, h - 6, palette::shade(SLATE, 0.9), BRICK, BRICK_DARK);
    // A course line and two window bays on the front face, lit after dark.
    let lit = glow > 0.35;
    let pane = if lit { GLASS_LIT } else { GLASS };
    let course = z + 2 + (h - 6) / 2;
    fb.line(p(x0, y0 + d, course), p(x0 + w, y0 + d, course), palette::shade(BRICK, 1.3));
    for k in 0..2 {
        let u = x0 + 0.14 + k as f32 * 0.44;
        window_on(fb, cam, u, y0 + d, z + 4, 0.24, 4, pane);
        if h > 12 {
            window_on(fb, cam, u, y0 + d, z + 9, 0.24, 3, pane);
        }
        let v = y0 + 0.14 + k as f32 * 0.44;
        fb.fill_poly(
            &[
                p(x0 + w, v, z + 7),
                p(x0 + w, v + 0.24, z + 7),
                p(x0 + w, v + 0.24, z + 4),
                p(x0 + w, v, z + 4),
            ],
            pane,
        );
    }
    // The door and its porch, on the near-left tile of the hall.
    let front = y == zy + zh - 1;
    if front && x == zx {
        fb.fill_poly(
            &[
                p(x0 + 0.36, y0 + d, z + 7),
                p(x0 + 0.62, y0 + d, z + 7),
                p(x0 + 0.62, y0 + d, z + 2),
                p(x0 + 0.36, y0 + d, z + 2),
            ],
            palette::rgb(0x3a, 0x2a, 0x22),
        );
        fb.fill_poly(
            &[
                p(x0 + 0.28, y0 + d + 0.05, z + 9),
                p(x0 + 0.7, y0 + d + 0.05, z + 9),
                p(x0 + 0.7, y0 + d + 0.05, z + 7),
                p(x0 + 0.28, y0 + d + 0.05, z + 7),
            ],
            SLATE,
        );
        // The name board over the door.
        let b = p(x0 + 0.16, y0 + d, z + h - 5);
        fb.fill_rect(b.0 as i32, b.1 as i32, 11, 2, PAINT);
        for k in 0..4 {
            fb.put(b.0 as i32 + 1 + k * 3, b.1 as i32, DARK_STEEL);
        }
    }
    // The clock, on the front face (a station has one).
    if kind >= 1 && front && x == zx + zw - 1 {
        let c = p(x0 + w * 0.5, y0 + d, z + h - 5);
        fb.fill_rect(c.0 as i32 - 1, c.1 as i32 - 1, 3, 3, PAINT);
        fb.put(c.0 as i32, c.1 as i32, DARK_STEEL);
    }
    // The roof slab at a constant height above this tile.
    let rh = h;
    box_at(
        fb,
        cam,
        x0 - 0.04,
        y0 - 0.04,
        z + rh - 4,
        w + 0.08,
        d + 0.08,
        4,
        SLATE_LIT,
        SLATE,
        palette::shade(SLATE, 0.72),
    );
    fb.line(
        p(x0, y0 + d * 0.5, z + rh),
        p(x0 + w, y0 + d * 0.5, z + rh),
        palette::shade(SLATE_LIT, 1.18),
    );
}

/// One platform tile: the raised slab, its edge line, a bench and a lamp.
pub fn draw_platform_tile(
    fb: &mut Framebuffer,
    cam: &Camera,
    map: &TileMap,
    x: i32,
    y: i32,
    side: u8,
    canopy: bool,
    glow: f32,
) {
    let z = z_of(map, x, y);
    // Full width along the track and half depth off it: a platform is one
    // continuous slab, not a row of paving stones (B58).
    let (a, b) = match side {
        0 => ((0.0, 0.10), (0.55, 0.90)),
        1 => ((0.45, 0.10), (1.0, 0.90)),
        2 => ((0.0, 0.05), (1.0, 0.55)),
        _ => ((0.0, 0.45), (1.0, 0.95)),
    };
    let top = [
        cam.project(x as f32 + a.0, y as f32 + a.1, z + 3),
        cam.project(x as f32 + b.0, y as f32 + a.1, z + 3),
        cam.project(x as f32 + b.0, y as f32 + b.1, z + 3),
        cam.project(x as f32 + a.0, y as f32 + b.1, z + 3),
    ];
    let fascia = [
        cam.project(x as f32 + a.0, y as f32 + b.1, z + 3),
        cam.project(x as f32 + b.0, y as f32 + b.1, z + 3),
        cam.project(x as f32 + b.0, y as f32 + b.1, z),
        cam.project(x as f32 + a.0, y as f32 + b.1, z),
    ];
    fb.fill_poly(&fascia, PLATFORM_LIP);
    fb.fill_poly(&top, PLATFORM);
    // The yellow safety line along the track edge.
    let (e0, e1) = match side {
        0 => ((0.0, 0.10), (0.0, 0.90)),
        1 => ((1.0, 0.10), (1.0, 0.90)),
        2 => ((0.0, 0.05), (1.0, 0.05)),
        _ => ((0.0, 0.95), (1.0, 0.95)),
    };
    fb.line(
        cam.project(x as f32 + e0.0, y as f32 + e0.1, z + 3),
        cam.project(x as f32 + e1.0, y as f32 + e1.1, z + 3),
        YELLOW_LINE,
    );
    // A bench against the back of the platform (a 6×1 px plank on two legs).
    let (bx, by) = match side {
        0 => (0.28, 0.72),
        1 => (0.72, 0.28),
        2 => (0.72, 0.28),
        _ => (0.28, 0.72),
    };
    let c = cam.project(x as f32 + bx, y as f32 + by, z + 3);
    fb.fill_rect(c.0 as i32 - 2, c.1 as i32 - 2, 5, 1, TIMBER);
    fb.put(c.0 as i32 - 1, c.1 as i32 - 1, DARK_STEEL);
    fb.put(c.0 as i32 + 1, c.1 as i32 - 1, DARK_STEEL);
    if canopy {
        // A canopy column with its girder, and the underside of the roof at the
        // back of the platform (the roof itself is drawn by the canopy pass so
        // it sits above the platform furniture).
        let col = cam.project(x as f32 + a.0 + 0.06, y as f32 + b.1 - 0.06, z + 3);
        fb.line(col, (col.0, col.1 - 8.0), STEEL);
        let tip = cam.project(x as f32 + b.0, y as f32 + a.1 + 0.06, z + 3);
        fb.line((col.0, col.1 - 8.0), (tip.0, tip.1 - 8.0), DARK_STEEL);
    }
    if glow > 0.35 {
        // The platform lamps are on after dark: a small pool of light.
        let l = cam.project(x as f32 + 0.5, y as f32 + 0.5, z + 12);
        fb.put_blended(l.0 as i32, l.1 as i32, palette::rgb(0xff, 0xe8, 0xb0), 0.85);
    }
}

/// The canopy roof over a platform tile: a slab on the columns, drawn after the
/// platform so it covers the bench and not the train.
#[allow(clippy::too_many_arguments)]
pub fn draw_canopy_tile(
    fb: &mut Framebuffer,
    cam: &Camera,
    map: &TileMap,
    x: i32,
    y: i32,
    side: u8,
    glow: f32,
) {
    let z = z_of(map, x, y);
    // One continuous roof along the platform: full width on the track axis, so
    // neighbouring canopy tiles join instead of reading as a row of huts (B58).
    let (a, b) = match side {
        0 => ((0.0, 0.05), (0.62, 0.95)),
        1 => ((0.38, 0.05), (1.0, 0.95)),
        2 => ((0.0, 0.02), (1.0, 0.62)),
        _ => ((0.0, 0.38), (1.0, 0.98)),
    };
    let lift = 10;
    let top = [
        cam.project(x as f32 + a.0, y as f32 + a.1, z + lift),
        cam.project(x as f32 + b.0, y as f32 + a.1, z + lift),
        cam.project(x as f32 + b.0, y as f32 + b.1, z + lift),
        cam.project(x as f32 + a.0, y as f32 + b.1, z + lift),
    ];
    let under = [
        cam.project(x as f32 + a.0, y as f32 + b.1, z + lift - 2),
        cam.project(x as f32 + b.0, y as f32 + b.1, z + lift - 2),
        cam.project(x as f32 + b.0, y as f32 + b.1, z + lift),
        cam.project(x as f32 + a.0, y as f32 + b.1, z + lift),
    ];
    fb.fill_poly(&under, CANOPY_UNDER);
    fb.fill_poly(&top, CANOPY_TOP);
    // Ribs across the canopy.
    for k in 1..4 {
        let t = k as f32 / 4.0;
        let p0 = (
            x as f32 + a.0 + (b.0 - a.0) * t,
            y as f32 + a.1,
        );
        let p1 = (
            x as f32 + a.0 + (b.0 - a.0) * t,
            y as f32 + b.1,
        );
        fb.line(
            cam.project(p0.0, p0.1, z + lift),
            cam.project(p1.0, p1.1, z + lift),
            palette::shade(CANOPY_TOP, 1.3),
        );
    }
    if glow > 0.35 {
        // Strip lights under the canopy.
        let c = cam.project(x as f32 + (a.0 + b.0) * 0.5, y as f32 + (a.1 + b.1) * 0.5, z + lift - 2);
        fb.put_blended(c.0 as i32, c.1 as i32, palette::rgb(0xff, 0xf0, 0xc8), 0.9);
    }
}

// ------------------------------------------------------- tram and bus stops

/// A stop shelter: a glass box with a roof, a bench and a sign pole (B58).
pub fn draw_shelter_tile(
    fb: &mut Framebuffer,
    cam: &Camera,
    map: &TileMap,
    x: i32,
    y: i32,
    side: u8,
    glow: f32,
) {
    let z = z_of(map, x, y);
    // The shelter faces the road (side is the pavement's side from the street).
    let (ox, oy, w, d) = match side {
        0 => (0.55, 0.18, 0.35, 0.64),
        1 => (0.10, 0.18, 0.35, 0.64),
        2 => (0.18, 0.55, 0.64, 0.35),
        _ => (0.18, 0.10, 0.64, 0.35),
    };
    // Back and side glass panels, then the roof.
    let p = |u: f32, v: f32, zz: i32| cam.project(u, v, zz);
    let (x0, y0, x1, y1) = (
        x as f32 + ox,
        y as f32 + oy,
        x as f32 + ox + w,
        y as f32 + oy + d,
    );
    let glass = if glow > 0.35 { GLASS_LIT } else { palette::shade(GLASS, 1.15) };
    fb.fill_poly(&[p(x0, y1, z + 9), p(x1, y1, z + 9), p(x1, y1, z + 1), p(x0, y1, z + 1)], glass);
    fb.fill_poly(&[p(x1, y0, z + 9), p(x1, y1, z + 9), p(x1, y1, z + 1), p(x1, y0, z + 1)], palette::shade(glass, 0.85));
    // Frame and roof.
    fb.line(p(x0, y1, z + 10), p(x1, y1, z + 10), STEEL);
    fb.line(p(x1, y0, z + 10), p(x1, y1, z + 10), STEEL);
    fb.fill_poly(
        &[
            p(x0 - 0.03, y0, z + 11),
            p(x1, y0, z + 11),
            p(x1, y1 + 0.03, z + 11),
            p(x0 - 0.03, y1 + 0.03, z + 11),
        ],
        palette::rgb(0xd8, 0xdc, 0xe4),
    );
    // Bench.
    let c = p(x0 + w * 0.5, y1 - 0.05, z + 4);
    fb.fill_rect(c.0 as i32 - 2, c.1 as i32, 5, 1, TIMBER);
    // The sign pole and its board.
    let pole = p(x0 + w + 0.05, y0 + d * 0.5, z + 1);
    fb.line(pole, (pole.0, pole.1 - 12.0), DARK_STEEL);
    fb.fill_rect(pole.0 as i32 - 1, pole.1 as i32 - 14, 4, 3, palette::rgb(0x2e, 0x6a, 0xa8));
    fb.put(pole.0 as i32, pole.1 as i32 - 13, PAINT);
}

/// A bus stop's kerb and its own sign where the shelter is on the other tile.
pub fn draw_bus_stop_tile(
    fb: &mut Framebuffer,
    cam: &Camera,
    map: &TileMap,
    x: i32,
    y: i32,
    side: u8,
    shelter: bool,
    glow: f32,
) {
    let z = z_of(map, x, y);
    if shelter {
        draw_shelter_tile(fb, cam, map, x, y, side, glow);
    } else {
        // The timetable board on its own, on the pavement edge.
        let (u, v) = match side {
            0 => (0.15, 0.5),
            1 => (0.85, 0.5),
            2 => (0.5, 0.15),
            _ => (0.5, 0.85),
        };
        let p = cam.project(x as f32 + u, y as f32 + v, z + 1);
        fb.line(p, (p.0, p.1 - 11.0), DARK_STEEL);
        fb.fill_rect(p.0 as i32 - 1, p.1 as i32 - 13, 4, 3, palette::rgb(0x2e, 0x6a, 0xa8));
        fb.put(p.0 as i32, p.1 as i32 - 12, PAINT);
    }
}

// ------------------------------------------------------------- the subway

/// The subway works: hoardings, then the open cut with its crews and spoil,
/// then the deck going back on, then the street and its entrances (B58).
#[allow(clippy::too_many_arguments)]
pub fn draw_subway_tile(
    fb: &mut Framebuffer,
    cam: &Camera,
    map: &TileMap,
    x: i32,
    y: i32,
    along_x: bool,
    stage: u8,
    entrance: bool,
    grate: bool,
    glow: f32,
) {
    let z = z_of(map, x, y);
    let q = quad(cam, map, x, y);
    match stage {
        // Hoardings: a plywood fence along the kerb with a warning sign.
        0 => {
            let (a, b) = if along_x {
                (cam.project(x as f32, y as f32 + 0.62, z), cam.project(x as f32 + 1.0, y as f32 + 0.62, z))
            } else {
                (cam.project(x as f32 + 0.62, y as f32, z), cam.project(x as f32 + 0.62, y as f32 + 1.0, z))
            };
            fb.fill_poly(
                &[a, b, (b.0, b.1 - 7.0), (a.0, a.1 - 7.0)],
                palette::rgb(0x2e, 0x4a, 0x6a),
            );
            fb.line(a, b, palette::rgb(0x1a, 0x2e, 0x44));
            let s = cam.project(x as f32 + 0.5, y as f32 + 0.5, z);
            fb.fill_rect(s.0 as i32 - 1, s.1 as i32 - 9, 3, 3, palette::rgb(0xf0, 0xc8, 0x30));
        }
        // The open cut: excavation, shoring, a beam across every third tile,
        // spoil on the pavement and a worker's lamp.
        1 => {
            // The cut itself, inset so the trench kerbs read.
            let inner = [
                cam.project(x as f32 + 0.12, y as f32 + 0.12, z - 3),
                cam.project(x as f32 + 0.88, y as f32 + 0.12, z - 3),
                cam.project(x as f32 + 0.88, y as f32 + 0.88, z - 3),
                cam.project(x as f32 + 0.12, y as f32 + 0.88, z - 3),
            ];
            fb.fill_poly(&inner, TRENCH);
            // Shoring planks along both edges.
            let (e0, e1, e2, e3) = if along_x {
                (
                    cam.project(x as f32, y as f32 + 0.12, z),
                    cam.project(x as f32 + 1.0, y as f32 + 0.12, z),
                    cam.project(x as f32, y as f32 + 0.88, z),
                    cam.project(x as f32 + 1.0, y as f32 + 0.88, z),
                )
            } else {
                (
                    cam.project(x as f32 + 0.12, y as f32, z),
                    cam.project(x as f32 + 0.12, y as f32 + 1.0, z),
                    cam.project(x as f32 + 0.88, y as f32, z),
                    cam.project(x as f32 + 0.88, y as f32 + 1.0, z),
                )
            };
            fb.line(e0, e1, TIMBER_DARK);
            fb.line(e2, e3, TIMBER_DARK);
            // A steel beam across the cut every third tile.
            if (x + y) % 3 == 0 {
                fb.line(e0, e3, STEEL);
                fb.line(e0, (e0.0, e0.1 - 4.0), DARK_STEEL);
                fb.line(e3, (e3.0, e3.1 - 4.0), DARK_STEEL);
            }
            // Spoil heap on the pavement.
            let s = cam.project(x as f32 + 0.2, y as f32 + 0.5, z);
            for k in 0..4 {
                fb.put(s.0 as i32 + k, s.1 as i32 - k / 2, SPOIL);
            }
            if glow > 0.35 {
                let lamp = cam.project(x as f32 + 0.5, y as f32 + 0.5, z + 6);
                fb.put_blended(lamp.0 as i32, lamp.1 as i32, palette::rgb(0xff, 0xd0, 0x60), 0.9);
            }
        }
        // The deck going back on: the cut covered with a concrete slab, one
        // tile in three still open for the crane.
        2 => {
            let inner = [
                cam.project(x as f32 + 0.1, y as f32 + 0.1, z - 2),
                cam.project(x as f32 + 0.9, y as f32 + 0.1, z - 2),
                cam.project(x as f32 + 0.9, y as f32 + 0.9, z - 2),
                cam.project(x as f32 + 0.1, y as f32 + 0.9, z - 2),
            ];
            if (x + y) % 3 == 1 {
                fb.fill_poly(&inner, TRENCH);
            } else {
                let s = [
                    cam.project(x as f32 + 0.1, y as f32 + 0.1, z),
                    cam.project(x as f32 + 0.9, y as f32 + 0.1, z),
                    cam.project(x as f32 + 0.9, y as f32 + 0.9, z),
                    cam.project(x as f32 + 0.1, y as f32 + 0.9, z),
                ];
                fb.fill_poly(&s, CONCRETE);
                fb.line(s[0], s[1], palette::shade(CONCRETE, 0.8));
            }
        }
        // The street is back, and the entrances are being built.
        3 | _ => {
            if entrance {
                draw_subway_entrance(fb, cam, map, x, y, glow, stage >= 4);
            }
        }
    }
    if grate && stage >= 3 {
        // A ventilation grate in the pavement: three slots and a steel frame.
        let q = q;
        let c = cam.project(x as f32 + 0.5, y as f32 + 0.5, z + 1);
        let w = 5;
        fb.fill_rect(c.0 as i32 - w / 2, c.1 as i32 - 1, w, 3, DARK_STEEL);
        for k in 0..3 {
            fb.fill_rect(c.0 as i32 - w / 2 + 1, c.1 as i32 + k as i32, w - 2, 1, palette::rgb(0x1a, 0x1c, 0x20));
        }
        let _ = q;
    }
}

/// A subway entrance: stairs down into a kiosk with a sign, railings and a lamp
/// (only once the works are open, the kiosk is boarded).
pub fn draw_subway_entrance(
    fb: &mut Framebuffer,
    cam: &Camera,
    map: &TileMap,
    x: i32,
    y: i32,
    glow: f32,
    open: bool,
) {
    let z = z_of(map, x, y);
    let p = |u: f32, v: f32, zz: i32| cam.project(u, v, zz);
    // The stair mouth: a dark rectangle in the pavement.
    let mouth = [
        p(x as f32 + 0.2, y as f32 + 0.3, z + 1),
        p(x as f32 + 0.8, y as f32 + 0.3, z + 1),
        p(x as f32 + 0.8, y as f32 + 0.8, z + 1),
        p(x as f32 + 0.2, y as f32 + 0.8, z + 1),
    ];
    fb.fill_poly(&mouth, palette::rgb(0x14, 0x16, 0x1a));
    // Steps: three lit treads going down.
    for k in 0..3 {
        let f = 0.3 + k as f32 * 0.16;
        fb.line(
            p(x as f32 + 0.24, y as f32 + f, z + 1),
            p(x as f32 + 0.76, y as f32 + f, z + 1),
            palette::rgb(0x3a, 0x3e, 0x46),
        );
    }
    // Railing posts down both sides.
    for (u, v) in [(0.2, 0.3), (0.2, 0.8), (0.8, 0.3), (0.8, 0.8)] {
        let a = p(x as f32 + u, y as f32 + v, z + 1);
        fb.line(a, (a.0, a.1 - 5.0), DARK_STEEL);
    }
    // The kiosk: a green/blue box with a white name board and a lamp.
    if open {
        box_at(fb, cam, x as f32 + 0.56, y as f32 + 0.08, z, 0.36, 0.3, 10, KIOSK_LIT, KIOSK, palette::shade(KIOSK, 0.8));
        let s = p(x as f32 + 0.62, y as f32 + 0.38, z + 8);
        fb.fill_rect(s.0 as i32, s.1 as i32, 5, 2, PAINT);
        fb.put(s.0 as i32 + 1, s.1 as i32, KIOSK);
        fb.put(s.0 as i32 + 3, s.1 as i32, KIOSK);
        if glow > 0.35 {
            let l = p(x as f32 + 0.74, y as f32 + 0.38, z + 11);
            fb.put_blended(l.0 as i32, l.1 as i32, palette::rgb(0xff, 0xe0, 0xa0), 0.95);
        }
    } else {
        // Boarded up: the contractor's hoarding over the kiosk footprint.
        box_at(fb, cam, x as f32 + 0.56, y as f32 + 0.08, z, 0.36, 0.3, 8, palette::rgb(0x2e, 0x4a, 0x6a), palette::rgb(0x24, 0x3a, 0x54), palette::rgb(0x1a, 0x2e, 0x44));
    }
}

// ------------------------------------------------------------ the airfield

/// The whole airfield, in one pass (B58c): the pavement first (runway, taxiway,
/// apron, access drive), then its buildings from far to near. Drawing it per
/// tile left a green seam between every pair of asphalt tiles and turned the
/// terminal into a field of slabs.
pub fn draw_airport(
    fb: &mut Framebuffer,
    cam: &Camera,
    map: &TileMap,
    ap: &crate::rail::AirportLook,
    glow: f32,
) {
    // --- the ground: one grown quad per tile, in diagonal order ---
    let mut ground: Vec<(i32, i32)> = ap.paved.clone();
    ground.extend(ap.taxiway.iter().copied());
    ground.extend(ap.access.iter().copied());
    let mut zones = vec![ap.apron, ap.terminal];
    zones.extend(ap.hangar);
    for z in zones {
        for ty in z.1..z.1 + z.3 {
            for tx in z.0..z.0 + z.2 {
                ground.push((tx, ty));
            }
        }
    }
    ground.sort_by_key(|&(x, y)| (x + y, x));
    ground.dedup();
    let (dx, dy) = if ap.runway.len() > 1 {
        let a = ap.runway[0];
        let b = *ap.runway.last().unwrap();
        (b.0 - a.0, b.1 - a.1)
    } else {
        (1, 0)
    };
    let along_x = dx.abs() >= dy.abs();
    let first = ap.runway.first().copied().unwrap_or((0, 0));
    let last_i = ap.runway.len().saturating_sub(1) as i32;
    let in_zone = |z: (i32, i32, i32, i32), x: i32, y: i32| {
        x >= z.0 && x < z.0 + z.2 && y >= z.1 && y < z.1 + z.3
    };
    for &(x, y) in &ground {
        if ap.paved.contains(&(x, y)) {
            let (along, across) = if along_x {
                (x - first.0, y - first.1)
            } else {
                (y - first.1, x - first.0)
            };
            draw_runway_tile(
                fb,
                cam,
                map,
                x,
                y,
                ap.stage >= 1,
                along_x,
                along.clamp(0, last_i),
                last_i,
                across,
                ap.width.max(1),
                glow,
            );
        } else if ap.taxiway.contains(&(x, y)) {
            // A grass field has no paved taxiway: the strip's aircraft taxy on
            // the mown grass (B58c).
            if ap.stage >= 1 {
                let along_x = ap.taxiway.len() > 1 && ap.taxiway[0].1 == ap.taxiway[1].1;
                draw_taxiway_tile(fb, cam, map, x, y, along_x);
            }
        } else if in_zone(ap.apron, x, y)
            || in_zone(ap.terminal, x, y)
            || ap.hangar.is_some_and(|h| in_zone(h, x, y))
        {
            // Paving and apron arrive with the first upgrade: a strip parks its
            // aircraft on the grass (B58c).
            if ap.stage >= 1 {
                let stand = ap.parked.contains(&(x, y));
                draw_apron_tile(fb, cam, map, x, y, stand);
            }
        } else {
            let along_x = ap.access.windows(2).any(|w| w[0].1 == w[1].1);
            draw_access_tile(fb, cam, map, x, y, along_x);
        }
    }
    // --- the runway's lights, after every pavement tile (B58c) ---
    if glow > 0.35 {
        for &(x, y) in &ap.paved {
            let (along, across) = if along_x {
                (x - first.0, y - first.1)
            } else {
                (y - first.1, x - first.0)
            };
            draw_runway_lights(
                fb,
                cam,
                map,
                x,
                y,
                along_x,
                along.clamp(0, last_i),
                last_i,
                across,
                ap.width.max(1),
            );
        }
    }
    // --- the buildings, far to near ---
    // The terminal and the apron arrive with the paving; a strip has a hangar,
    // a windsock and grass (B58c).
    let mut blocks: Vec<(i32, u8)> = vec![(ap.windsock.0 + ap.windsock.1, 4)];
    if ap.stage >= 1 {
        blocks.push((ap.terminal.0 + ap.terminal.1, 0));
    }
    if let Some(h) = ap.hangar {
        blocks.push((h.0 + h.1, 1));
    }
    if let Some((tx, ty, _)) = ap.tower {
        blocks.push((tx + ty, 2));
    }
    for (i, &(px, py)) in ap.parked.iter().enumerate() {
        if i > 0 {
            blocks.push((px + py, 3));
            let _ = px;
            let _ = py;
        }
    }
    blocks.sort_by_key(|(k, _)| *k);
    for (_, what) in blocks {
        match what {
            0 => draw_terminal(fb, cam, map, ap.terminal, glow),
            1 => {
                if let Some(h) = ap.hangar {
                    draw_hangar(fb, cam, map, h);
                }
            }
            2 => {
                if let Some((tx, ty, _)) = ap.tower {
                    draw_tower(fb, cam, map, tx, ty, glow);
                }
            }
            3 => {
                for (i, &(px, py)) in ap.parked.iter().enumerate() {
                    if i > 0 {
                        draw_parked_plane(fb, cam, map, px, py, i as u8);
                    }
                }
            }
            _ => draw_windsock(fb, cam, map, ap.windsock.0, ap.windsock.1),
        }
    }
}

/// A runway tile (B58c): mown grass while the field is a strip, asphalt after,
/// with a white centre line on the seam between the pavement's rows, white edge
/// stripes on the outer rows, threshold bars at both ends, and the edge lights
/// lit after dark.
#[allow(clippy::too_many_arguments)]
pub fn draw_runway_tile(
    fb: &mut Framebuffer,
    cam: &Camera,
    map: &TileMap,
    x: i32,
    y: i32,
    paved: bool,
    along_x: bool,
    i: i32,
    last_i: i32,
    row: i32,
    width: i32,
    glow: f32,
) {
    let z = z_of(map, x, y);
    let q = quad(cam, map, x, y);
    let p = |u: f32, v: f32| cam.project(u, v, z);
    // The strip's own axis: `u` runs along the runway, `v` across it.
    let (u0, u1) = if along_x {
        (x as f32, x as f32 + 1.0)
    } else {
        (y as f32, y as f32 + 1.0)
    };
    let (v0, v1) = if along_x {
        (y as f32, y as f32 + 1.0)
    } else {
        (x as f32, x as f32 + 1.0)
    };
    let at = |u: f32, v: f32| if along_x { p(u, v) } else { p(v, u) };
    let _ = q;
    if !paved {
        // Mown grass, striped across the strip so it reads as a runway.
        let c = if (u0 as i32 + v0 as i32) % 2 == 0 { GRASS_MOWN } else { GRASS_STRIP };
        fill_tile_grown(fb, cam, map, x, y, c, 0.8, 0);
        // A mown strip still has its edges marked: a pale mown line down each
        // side, so the strip is a strip and not a meadow.
        if row == 0 {
            fb.line(at(u0, v0 + 0.1), at(u1, v0 + 0.1), palette::rgb(0x8a, 0xb0, 0x5a));
        }
        if row == width.max(1) - 1 {
            fb.line(at(u0, v1 - 0.1), at(u1, v1 - 0.1), palette::rgb(0x8a, 0xb0, 0x5a));
        }
        return;
    }
    fill_tile_grown(fb, cam, map, x, y, if i == 0 || i == last_i { ASPHALT_LIT } else { ASPHALT }, 1.2, 0);
    // Outer edge stripes: white, continuous along the strip (each tile draws a
    // little past its own ends so the dashes join).
    let (e0, e1) = (u0 - 0.02, u1 + 0.02);
    if row == 0 {
        fb.line(at(e0, v0 + 0.03), at(e1, v0 + 0.03), palette::shade(PAINT, 0.85));
    }
    if row == width.max(1) - 1 {
        fb.line(at(e0, v1 - 0.03), at(e1, v1 - 0.03), palette::shade(PAINT, 0.85));
    }
    // The centre line: a dashed band on the seam between the pavement rows.
    // Both rows draw their side of it, so whichever the painter reaches last,
    // the line stays (the seam is where two tiles' fills meet).
    if width >= 2 {
        let (b0, b1) = if row == 0 { (v1 - 0.05, v1) } else { (v0, v0 + 0.05) };
        if i % 2 == 0 {
            fb.line(at(u0 + 0.08, b0), at(u0 + 0.92, b0), PAINT);
            fb.line(at(u0 + 0.08, b1), at(u0 + 0.92, b1), PAINT);
        }
    }
    // Threshold markings: bars across the pavement at both ends.
    if i == 0 || i == last_i {
        for k in 0..3 {
            let ua = u0 + 0.18 + k as f32 * 0.24;
            fb.line(at(ua, v0 + 0.2), at(ua, v1 - 0.2), PAINT);
        }
    }
}

/// The runway's lights, in their own pass (B58c): the edge lamps and the
/// threshold lights are drawn *after* every pavement tile, or the next tile's
/// asphalt (grown by a pixel to kill the seams) paints over them.
pub fn draw_runway_lights(
    fb: &mut Framebuffer,
    cam: &Camera,
    map: &TileMap,
    x: i32,
    y: i32,
    along_x: bool,
    i: i32,
    last_i: i32,
    row: i32,
    width: i32,
) {
    let z = z_of(map, x, y);
    let p = |u: f32, v: f32| cam.project(u, v, z);
    let (u0, u1) = if along_x {
        (x as f32, x as f32 + 1.0)
    } else {
        (y as f32, y as f32 + 1.0)
    };
    let (v0, v1) = if along_x {
        (y as f32, y as f32 + 1.0)
    } else {
        (x as f32, x as f32 + 1.0)
    };
    let at = |u: f32, v: f32| if along_x { p(u, v) } else { p(v, u) };
    let lamp = palette::rgb(0xff, 0xe0, 0x9a);
    let mut dot = |pt: (f32, f32), c: Color, halo: f32| {
        let (lx, ly) = (pt.0 as i32, pt.1 as i32);
        // A 3-px lamp head with a soft halo, the size of a street lamp's (B58c:
        // a 1-px light on a runway at 1× is invisible).
        fb.fill_rect(lx - 1, ly - 1, 3, 3, emissive(c));
        let halo_c = emissive(palette::mix(c, ASPHALT, 0.6));
        for (dx, dy, a) in [
            (-2, 0, halo),
            (2, 0, halo),
            (0, -2, halo),
            (0, 2, halo),
            (-2, -1, halo * 0.6),
            (2, 1, halo * 0.6),
        ] {
            if a > 0.0 {
                fb.put(lx + dx, ly + dy, halo_c);
            }
        }
    };
    if row == 0 {
        dot(at(u0 + 0.5, v0 + 0.06), lamp, 0.35);
    }
    if row == width.max(1) - 1 {
        dot(at(u0 + 0.5, v1 - 0.06), lamp, 0.35);
    }
    if i == 0 {
        let c = at(u0 + 0.06, v0 + 0.5);
        dot(c, palette::rgb(0x70, 0xff, 0x80), 0.3);
        dot(at(u0 + 0.06, v0 + 1.2), palette::rgb(0x70, 0xff, 0x80), 0.25);
    }
    if i == last_i {
        let c = at(u1 - 0.06, v0 + 0.5);
        dot(c, palette::rgb(0xff, 0x5a, 0x48), 0.3);
        dot(at(u1 - 0.06, v0 + 1.2), palette::rgb(0xff, 0x5a, 0x48), 0.25);
    }
}

/// The taxiway: narrower asphalt with a yellow line.
pub fn draw_taxiway_tile(fb: &mut Framebuffer, cam: &Camera, map: &TileMap, x: i32, y: i32, along_x: bool) {
    let z = z_of(map, x, y);
    let (a, b) = if along_x {
        ((x as f32, y as f32 + 0.25), (x as f32 + 1.0, y as f32 + 0.75))
    } else {
        ((x as f32 + 0.25, y as f32), (x as f32 + 0.75, y as f32 + 1.0))
    };
    fill_tile_grown(fb, cam, map, x, y, ASPHALT_LIT, 1.0, 0);
    let mid0 = if along_x { (x as f32 - 0.02, y as f32 + 0.5) } else { (x as f32 + 0.5, y as f32 - 0.02) };
    let mid1 = if along_x { (x as f32 + 1.02, y as f32 + 0.5) } else { (x as f32 + 0.5, y as f32 + 1.02) };
    fb.line(cam.project(mid0.0, mid0.1, z), cam.project(mid1.0, mid1.1, z), YELLOW_LINE);
}

/// The apron: concrete, with a stand box where an aircraft parks.
pub fn draw_apron_tile(fb: &mut Framebuffer, cam: &Camera, map: &TileMap, x: i32, y: i32, stand: bool) {
    let z = z_of(map, x, y);
    fill_tile_grown(fb, cam, map, x, y, CONCRETE, 1.2, 0);
    if stand {
        let p = |u: f32, v: f32| cam.project(u, v, z);
        fb.line(p(x as f32 + 0.2, y as f32 + 0.2), p(x as f32 + 0.8, y as f32 + 0.2), palette::shade(PAINT, 0.9));
        fb.line(p(x as f32 + 0.2, y as f32 + 0.8), p(x as f32 + 0.8, y as f32 + 0.8), palette::shade(PAINT, 0.9));
    }
}

/// The terminal as one building: a glazed hall with a flat roof, a parapet, a
/// canopy over the apron side and a sign (B58c: per-tile boxes read as a field
/// of slabs).
pub fn draw_terminal(
    fb: &mut Framebuffer,
    cam: &Camera,
    map: &TileMap,
    zone: (i32, i32, i32, i32),
    glow: f32,
) {
    let (zx, zy, zw, zh) = zone;
    let z = z_of(map, zx, zy);
    let h = 15;
    let (x0, y0) = (zx as f32 + 0.03, zy as f32 + 0.03);
    let (w, d) = (zw as f32 - 0.06, zh as f32 - 0.06);
    let p = |u: f32, v: f32, zz: i32| cam.project(u, v, zz);
    // The hall: a pale steel and glass box.
    box_at(
        fb,
        cam,
        x0,
        y0,
        z,
        w,
        d,
        h,
        palette::rgb(0xd8, 0xdc, 0xe0),
        TERMINAL,
        TERMINAL_DARK,
    );
    // A curtain wall on both camera-facing sides, with mullions, lit after dark.
    let lit = glow > 0.35;
    let pane = if lit {
        emissive(palette::rgb(0xd8, 0xe8, 0xf0))
    } else {
        GLASS
    };
    let bays = ((w + d) * 2.0) as i32;
    for k in 0..bays.max(2) {
        let t = (k as f32 + 0.5) / bays.max(2) as f32;
        let ua = x0 + t * w;
        fb.fill_poly(
            &[
                p(ua - 0.07, y0 + d, z + 11),
                p(ua + 0.07, y0 + d, z + 11),
                p(ua + 0.07, y0 + d, z + 3),
                p(ua - 0.07, y0 + d, z + 3),
            ],
            pane,
        );
        let va = y0 + t * d;
        fb.fill_poly(
            &[
                p(x0 + w, va - 0.07, z + 11),
                p(x0 + w, va + 0.07, z + 11),
                p(x0 + w, va + 0.07, z + 3),
                p(x0 + w, va - 0.07, z + 3),
            ],
            palette::shade(pane, 0.85),
        );
    }
    // A bright entrance bay at the front centre.
    let du = x0 + w * 0.5;
    fb.fill_poly(
        &[
            p(du - 0.18, y0 + d, z + 9),
            p(du + 0.18, y0 + d, z + 9),
            p(du + 0.18, y0 + d, z),
            p(du - 0.18, y0 + d, z),
        ],
        if lit {
            palette::rgb(0xf0, 0xf4, 0xf8)
        } else {
            palette::rgb(0x9c, 0xb8, 0xc8)
        },
    );
    // The roof: a pale slab with a parapet, and the sign on the front.
    box_at(
        fb,
        cam,
        x0 - 0.03,
        y0 - 0.03,
        z + h,
        w + 0.06,
        d + 0.06,
        2,
        palette::rgb(0xc4, 0xc8, 0xcc),
        palette::rgb(0xa8, 0xac, 0xb0),
        palette::rgb(0x96, 0x9a, 0x9e),
    );
    // Skylights along the roof, and the airport's name on the roof edge.
    for k in 0..3 {
        let t = 0.2 + k as f32 * 0.28;
        let (u0, v0) = (x0 + t * w, y0 + d * 0.25);
        let sky = [
            p(u0, v0, z + h + 2),
            p(u0 + 0.18, v0, z + h + 2),
            p(u0 + 0.18, v0 + d * 0.5, z + h + 2),
            p(u0, v0 + d * 0.5, z + h + 2),
        ];
        fb.fill_poly(&sky, palette::rgb(0x8e, 0xb4, 0xc8));
    }
    let s = p(du - 0.22, y0 + d, z + 13);
    fb.fill_rect(s.0 as i32, s.1 as i32, 9, 2, palette::rgb(0x2e, 0x5a, 0x8a));
    for k in 0..3 {
        fb.put(s.0 as i32 + 1 + k * 3, s.1 as i32, PAINT);
    }
    // The apron-side canopy.
    let c = [
        p(x0 + 0.1, y0 + d - 0.02, z + 10),
        p(x0 + w - 0.1, y0 + d - 0.02, z + 10),
        p(x0 + w - 0.1, y0 + d + 0.45, z + 10),
        p(x0 + w * 0.5, y0 + d + 0.5, z + 10),
    ];
    fb.fill_poly(&c, palette::rgb(0xc0, 0xc4, 0xc8));
    // Rooftop plant, so the roof is not a blank plane.
    for k in 0..2 {
        let (ux, uy) = (x0 + 0.2 + k as f32 * (w - 0.6), y0 + 0.2);
        box_at(
            fb,
            cam,
            ux,
            uy,
            z + h + 2,
            0.3,
            0.3,
            3,
            palette::rgb(0x8a, 0x8e, 0x92),
            palette::rgb(0x6e, 0x72, 0x76),
            palette::rgb(0x5a, 0x5e, 0x62),
        );
    }
}

/// The hangar: one shed with a curved roof, ribs and two open bays on the
/// camera side (B58c).
pub fn draw_hangar(fb: &mut Framebuffer, cam: &Camera, map: &TileMap, zone: (i32, i32, i32, i32)) {
    let (zx, zy, zw, zh) = zone;
    let z = z_of(map, zx, zy);
    let h = 13;
    let (x0, y0) = (zx as f32 + 0.02, zy as f32 + 0.02);
    let (w, d) = (zw as f32 - 0.04, zh as f32 - 0.04);
    let p = |u: f32, v: f32, zz: i32| cam.project(u, v, zz);
    box_at(
        fb,
        cam,
        x0,
        y0,
        z,
        w,
        d,
        h,
        palette::rgb(0xb4, 0xb8, 0xb4),
        HANGAR,
        palette::shade(HANGAR, 0.8),
    );
    // The barrel roof: a lighter cap with a ridge.
    let roof = [
        p(x0 - 0.02, y0 - 0.02, z + h + 3),
        p(x0 + w + 0.02, y0 - 0.02, z + h + 3),
        p(x0 + w + 0.02, y0 + d + 0.02, z + h + 3),
        p(x0 - 0.02, y0 + d + 0.02, z + h + 3),
    ];
    fb.fill_poly(&roof, HANGAR_ROOF);
    fb.line(
        p(x0, y0 + d * 0.5, z + h + 3),
        p(x0 + w, y0 + d * 0.5, z + h + 3),
        palette::shade(HANGAR_ROOF, 1.1),
    );
    // Corrugated ribs down both camera-facing sides.
    for k in 0..5 {
        let t = 0.1 + k as f32 * 0.2;
        fb.line(
            p(x0 + t * w, y0 + d, z + h),
            p(x0 + t * w, y0 + d, z),
            palette::shade(HANGAR, 0.7),
        );
        fb.line(
            p(x0 + w, y0 + t * d, z + h),
            p(x0 + w, y0 + t * d, z),
            palette::shade(HANGAR, 0.62),
        );
    }
    // Two open bays with a lintel on *both* camera-facing sides: an airfield
    // can run either way (B58c: on a north-south strip the doors were on the
    // hidden side and the hangar read as a blank box).
    for k in 0..2 {
        let ua = x0 + 0.12 + k as f32 * (w * 0.5);
        fb.fill_poly(
            &[
                p(ua, y0 + d, z + 9),
                p(ua + w * 0.34, y0 + d, z + 9),
                p(ua + w * 0.34, y0 + d, z),
                p(ua, y0 + d, z),
            ],
            palette::rgb(0x2a, 0x2c, 0x30),
        );
        fb.line(
            p(ua, y0 + d, z + 9),
            p(ua + w * 0.34, y0 + d, z + 9),
            palette::rgb(0xe0, 0xc0, 0x40),
        );
        let va = y0 + 0.12 + k as f32 * (d * 0.5);
        fb.fill_poly(
            &[
                p(x0 + w, va, z + 9),
                p(x0 + w, va + d * 0.34, z + 9),
                p(x0 + w, va + d * 0.34, z),
                p(x0 + w, va, z),
            ],
            palette::rgb(0x24, 0x26, 0x2a),
        );
        fb.line(
            p(x0 + w, va, z + 9),
            p(x0 + w, va + d * 0.34, z + 9),
            palette::rgb(0xd0, 0xb0, 0x38),
        );
    }
}

/// The control tower with its beacon (B58c: tall enough to read over the
/// terminal, with a glazed cab and a red beacon at night).
pub fn draw_tower(fb: &mut Framebuffer, cam: &Camera, map: &TileMap, x: i32, y: i32, glow: f32) {
    let z = z_of(map, x, y);
    let p = |u: f32, v: f32, zz: i32| cam.project(u, v, zz);
    let base = p(x as f32 + 0.5, y as f32 + 0.5, z);
    // A plinth and a shaft: darker than the terminal, so the tower reads as a
    // tower and not as a chimney on the hall (B58c).
    fb.fill_rect(base.0 as i32 - 3, base.1 as i32 - 2, 7, 2, palette::rgb(0x8a, 0x8e, 0x92));
    for k in 0..34 {
        let c = if k % 8 < 4 {
            palette::rgb(0x9a, 0x9e, 0xa2)
        } else {
            palette::rgb(0x8a, 0x8e, 0x92)
        };
        fb.put(base.0 as i32, base.1 as i32 - 2 - k, c);
        fb.put(base.0 as i32 + 1, base.1 as i32 - 2 - k, palette::shade(c, 0.82));
        if k % 8 == 0 {
            fb.put(base.0 as i32 - 1, base.1 as i32 - 2 - k, palette::shade(c, 0.9));
        }
    }
    // The glazed cab: wider than the shaft, with a roof lip.
    let cab = (base.0 - 5.0, base.1 - 44.0);
    fb.fill_rect(cab.0 as i32, cab.1 as i32, 12, 7, palette::rgb(0x6a, 0x8a, 0xa2));
    for k in 0..6 {
        fb.put(cab.0 as i32 + 1 + k * 2, cab.1 as i32, palette::rgb(0xa8, 0xd0, 0xe8));
        fb.put(cab.0 as i32 + 1 + k * 2, cab.1 as i32 + 1, palette::rgb(0x8e, 0xb4, 0xcc));
    }
    fb.fill_rect(cab.0 as i32, cab.1 as i32 + 7, 12, 1, palette::rgb(0xd8, 0xd8, 0xd0));
    fb.fill_rect(cab.0 as i32 - 1, cab.1 as i32 - 1, 14, 1, palette::rgb(0xf0, 0xf0, 0xea));
    // The beacon: red, lit at night with a halo.
    let b = (cab.0 as i32 + 4, cab.1 as i32 - 2);
    if glow > 0.35 {
        fb.fill_rect(b.0 - 1, b.1 - 1, 3, 3, emissive(BEACON));
        fb.put(b.0 - 2, b.1, emissive(palette::mix(BEACON, palette::rgb(0x1a, 0x1c, 0x20), 0.5)));
        fb.put(b.0 + 2, b.1, emissive(palette::mix(BEACON, palette::rgb(0x1a, 0x1c, 0x20), 0.5)));
    } else {
        fb.put(b.0, b.1, palette::shade(BEACON, 0.75));
    }
}

/// The windsock at the threshold: a pole and an orange sock.
pub fn draw_windsock(fb: &mut Framebuffer, cam: &Camera, map: &TileMap, x: i32, y: i32) {
    let z = z_of(map, x, y);
    let p = cam.project(x as f32 + 0.5, y as f32 + 0.5, z);
    fb.line(p, (p.0, p.1 - 13.0), palette::rgb(0xd8, 0xd8, 0xd0));
    for k in 0..4 {
        let w = 4 - k;
        fb.fill_rect(
            p.0 as i32 + 1 + k as i32,
            p.1 as i32 - 13 + k as i32,
            w,
            2,
            palette::rgb(0xf0, 0x8a, 0x30),
        );
        if k % 2 == 1 {
            fb.fill_rect(
                p.0 as i32 + 1 + k as i32,
                p.1 as i32 - 13 + k as i32,
                w,
                1,
                palette::rgb(0xf4, 0xf0, 0xe8),
            );
        }
    }
}

/// A parked aircraft on the apron: a fuselage with windows and a cheat line,
/// swept wings, engines, a tail fin and its shadow on the concrete.
pub fn draw_parked_plane(fb: &mut Framebuffer, cam: &Camera, map: &TileMap, x: i32, y: i32, variant: u8) {
    let z = z_of(map, x, y);
    let c = cam.project(x as f32 + 0.5, y as f32 + 0.5, z);
    let cx = c.0 as i32;
    let cy = c.1 as i32;
    let body = if variant % 2 == 0 {
        palette::rgb(0xf0, 0xf2, 0xf6)
    } else {
        palette::rgb(0xe4, 0xd8, 0xb8)
    };
    let shade = palette::shade(body, 0.8);
    let win = palette::rgb(0x3a, 0x4a, 0x62);
    let trim = palette::rgb(0x2e, 0x5a, 0x9a);
    for i in 0..16 {
        fb.put(cx - 7 + i, cy, body);
        fb.put(cx - 7 + i, cy + 1, shade);
        if (3..13).contains(&i) && i % 2 == 0 {
            fb.put(cx - 7 + i, cy, win);
        }
        if i % 4 == 0 {
            fb.put(cx - 7 + i, cy + 1, trim);
        }
    }
    for i in 0..5 {
        fb.put(cx - 2 - i, cy - 1 - i / 2, shade);
        fb.put(cx + 2 + i, cy - 1 - i / 2, shade);
        fb.put(cx - 2 - i, cy + 2 + i / 2, shade);
        fb.put(cx + 2 + i, cy + 2 + i / 2, shade);
    }
    fb.fill_rect(cx - 3, cy + 2, 2, 2, palette::rgb(0x9a, 0xa0, 0xaa));
    fb.fill_rect(cx + 3, cy + 2, 2, 2, palette::rgb(0x9a, 0xa0, 0xaa));
    for k in 0..4 {
        fb.put(cx - 8, cy - 1 - k, trim);
        fb.put(cx - 9, cy - k, trim);
    }
    fb.put(cx - 8, cy - 1, body);
    for i in 0..14 {
        fb.put_blended(cx - 6 + i, cy + 3, palette::rgb(0x20, 0x22, 0x26), 0.18);
    }
}

/// The access drive from the terminal to the street: asphalt with a white edge
/// line, drawn as one grown quad per tile so it has no seams.
pub fn draw_access_tile(fb: &mut Framebuffer, cam: &Camera, map: &TileMap, x: i32, y: i32, along_x: bool) {
    let z = z_of(map, x, y);
    fill_tile_grown(fb, cam, map, x, y, ASPHALT_LIT, 0.8, 0);
    let (m0, m1) = if along_x {
        ((x as f32 - 0.02, y as f32 + 0.5), (x as f32 + 1.02, y as f32 + 0.5))
    } else {
        ((x as f32 + 0.5, y as f32 - 0.02), (x as f32 + 0.5, y as f32 + 1.02))
    };
    fb.line(
        cam.project(m0.0, m0.1, z),
        cam.project(m1.0, m1.1, z),
        palette::shade(PAINT, 0.9),
    );
}

// ------------------------------------------------------------- the harbour

/// The whole harbour in one pass (B58c): the quay as a straight stone edge with
/// a continuous wall down to the water, bollards, small cranes, a fish market
/// shed, the piers and the boats alongside. Per-tile drawing made the quay a row
/// of diamond teeth and the pier a line of loose planks.
pub fn draw_harbour(
    fb: &mut Framebuffer,
    cam: &Camera,
    map: &TileMap,
    h: &crate::rail::HarbourLook,
    glow: f32,
) {
    let stone = palette::rgb(0x9c, 0x98, 0x8e);
    let stone_lit = palette::rgb(0xb4, 0xb0, 0xa4);
    let stone_dark = palette::rgb(0x6e, 0x6a, 0x62);
    let (dx, dy) = h.water_dir;
    // The water's own surface, per tile, so the wall knows how deep to go.
    let water_z = |x: i32, y: i32| -> i32 {
        (map.surface_level_at(x as f32, y as f32) * HEIGHT_UNIT as f32).round() as i32
    };
    // 1. The quay's stone paving, one grown quad per tile.
    for &(x, y) in &h.quay {
        let z = z_of(map, x, y);
        fill_tile_grown(fb, cam, map, x, y, stone, 1.2, 0);
        // Coping stones along the water edge, and the wall face below them.
        let (u0, u1) = if dx != 0 {
            (y as f32, y as f32 + 1.0)
        } else {
            (x as f32, x as f32 + 1.0)
        };
        let along_x = dx != 0;
        let at = |u: f32, v: f32, zz: i32| {
            if along_x {
                cam.project(v, u, zz)
            } else {
                cam.project(u, v, zz)
            }
        };
        // The edge line faces the water: v = 1 when the water is at +y/+x.
        let wet_high = if along_x { dy > 0 } else { dx > 0 };
        let v = if wet_high { 0.94 } else { 0.06 };
        let top = at(u0, v, z + 1);
        let top2 = at(u1, v, z + 1);
        // The coping: a pale band along the edge.
        fb.line(at(u0, v, z + 2), at(u1, v, z + 2), stone_lit);
        // The wall: from the coping down to the water, a continuous face.
        let (wx, wy) = (x + dx, y + dy);
        // A quay stands proud of the water: the wall is at least four pixels of
        // masonry whatever the tide line is, with a pale coping on top (B58c:
        // the first quay had no face at all on a flat shore).
        let wz = (water_z(wx, wy) - 1).min(z - 5);
        let bot = at(u0, v, wz);
        let bot2 = at(u1, v, wz);
        fb.fill_poly(
            &[top, top2, bot2, bot],
            if wet_high { stone_dark } else { palette::shade(stone_dark, 0.92) },
        );
        // The coping stone: a pale band along the edge.
        fb.line(at(u0, v, z + 1), at(u1, v, z + 1), stone_lit);
        fb.line(at(u0, v, z), at(u1, v, z), palette::shade(stone_lit, 0.8));
        // Course lines in the wall so it reads as masonry, and a dark waterline.
        let courses = ((z + 1 - wz) / 3).max(1);
        for k in 1..courses {
            let zz = z + 1 - k * 3;
            if zz <= wz {
                break;
            }
            fb.line(at(u0, v, zz), at(u1, v, zz), palette::shade(stone_dark, 0.8));
        }
        fb.line(at(u0, v, wz + 1), at(u1, v, wz + 1), palette::rgb(0x2a, 0x3a, 0x42));
    }
    // 1b. The ends: a revetment corner, so the stone does not stop dead against
    // a ragged shore (B58d: the zig-zag teeth beside the quay). At each end the
    // quay turns inland for two tiles with the same coping and wall.
    for end in [0usize, h.quay.len().saturating_sub(1)] {
        let Some(&(x, y)) = h.quay.get(end) else { continue };
        // Inland is away from the water; along the quay is towards the middle.
        let (sx, sy) = (-dx, -dy);
        let (ax, ay) = if h.quay.len() > 1 {
            let other = h.quay[if end == 0 { 1 } else { h.quay.len() - 2 }];
            ((other.0 - x).signum(), (other.1 - y).signum())
        } else {
            (0, 0)
        };
        let _ = (ax, ay);
        for k in 1..=2 {
            let (wx, wy) = (x + sx * k, y + sy * k);
            if !map.in_bounds(wx, wy) || map.at(wx, wy).is_water() {
                break;
            }
            let wz = z_of(map, wx, wy);
            fill_tile_grown(fb, cam, map, wx, wy, stone, 1.2, 0);
            // The coping along its water-facing edge, and a return wall.
            let along_x = dx != 0;
            let (u0, u1) = if along_x {
                (wy as f32, wy as f32 + 1.0)
            } else {
                (wx as f32, wx as f32 + 1.0)
            };
            let at = |u: f32, v: f32, zz: i32| {
                if along_x {
                    cam.project(wx as f32 + v, u, zz)
                } else {
                    cam.project(u, wy as f32 + v, zz)
                }
            };
            fb.line(at(u0, 0.03, wz + 1), at(u1, 0.03, wz + 1), stone_lit);
            let mut wtop = at(u0, 0.03, wz + 1);
            let mut wbot = at(u0, 0.03, wz - 4);
            fb.line(wtop, wbot, stone_dark);
            wtop = at(u1, 0.03, wz + 1);
            wbot = at(u1, 0.03, wz - 4);
            fb.line(wtop, wbot, stone_dark);
        }
    }
    // 2. Bollards every third quay tile, on the water edge.
    for (i, &(x, y)) in h.quay.iter().enumerate() {
        if i % 2 != 1 {
            continue;
        }
        let z = z_of(map, x, y);
        let along_x = dx != 0;
        let wet_high = if along_x { dy > 0 } else { dx > 0 };
        let (bx, by) = if along_x {
            (x as f32 + 0.5, y as f32 + if wet_high { 0.86 } else { 0.14 })
        } else {
            (x as f32 + if wet_high { 0.86 } else { 0.14 }, y as f32 + 0.5)
        };
        let b = cam.project(bx, by, z + 2);
        let (bx0, by0) = (b.0 as i32, b.1 as i32);
        // A bollard: a dark waist with a pale cap, on a base plate.
        fb.fill_rect(bx0 - 1, by0 - 4, 3, 4, palette::rgb(0x3a, 0x3c, 0x40));
        fb.fill_rect(bx0 - 1, by0 - 5, 3, 1, palette::rgb(0x7a, 0x7c, 0x80));
        fb.fill_rect(bx0 - 2, by0, 5, 1, palette::rgb(0x5a, 0x5c, 0x60));
        // A coil of rope at its foot.
        fb.put(bx0 + 2, by0 - 1, palette::rgb(0xc8, 0xb4, 0x88));
        fb.put(bx0 + 3, by0 - 1, palette::rgb(0xa8, 0x94, 0x68));
    }
    // 3. The piers: a continuous timber deck on posts.
    for pier in &h.piers {
        for &(x, y) in pier {
            let z = (map.surface_level_at(x as f32, y as f32) * HEIGHT_UNIT as f32).round() as i32 + 3;
            fill_tile_grown(fb, cam, map, x, y, TIMBER, 1.0, 3);
            // Plank lines across the deck.
            let along_x = dx != 0;
            for k in 1..4 {
                let t = k as f32 / 4.0;
                let (a, b) = if along_x {
                    (cam.project(x as f32 + t, y as f32, z), cam.project(x as f32 + t, y as f32 + 1.0, z))
                } else {
                    (cam.project(x as f32, y as f32 + t, z), cam.project(x as f32 + 1.0, y as f32 + t, z))
                };
                fb.line(a, b, TIMBER_DARK);
            }
            // Posts down to the water.
            for (u, v) in [(0.15f32, 0.85f32), (0.85, 0.15)] {
                let top = cam.project(x as f32 + u, y as f32 + v, z);
                let bot = cam.project(x as f32 + u, y as f32 + v, z - 6);
                fb.line(top, bot, TIMBER_DARK);
            }
        }
        // A bollard at the pier's end.
        if let Some(&(x, y)) = pier.last() {
            let z = (map.surface_level_at(x as f32, y as f32) * HEIGHT_UNIT as f32).round() as i32 + 3;
            let b = cam.project(x as f32 + 0.5, y as f32 + 0.5, z);
            fb.fill_rect(b.0 as i32 - 1, b.1 as i32 - 3, 2, 3, palette::rgb(0x3a, 0x3c, 0x40));
        }
    }
    // 4. The fish market shed: a timber and stone building with a dark roof,
    //    a sign and a lamp over the door.
    let (sx, sy, sw, sh) = h.shed;
    if sw > 0 && sh > 0 {
        let z = z_of(map, sx, sy);
        let (x0, y0) = (sx as f32 + 0.04, sy as f32 + 0.04);
        let (w, d) = (sw as f32 - 0.08, sh as f32 - 0.08);
        let p = |u: f32, v: f32, zz: i32| cam.project(u, v, zz);
        box_at(fb, cam, x0, y0, z, w, d, 9, SLATE_LIT, palette::rgb(0xb0, 0x9a, 0x7c), palette::rgb(0x8a, 0x76, 0x5c));
        // A slate roof with a ridge.
        box_at(
            fb,
            cam,
            x0 - 0.06,
            y0 - 0.06,
            z + 9,
            w + 0.12,
            d + 0.12,
            3,
            SLATE_LIT,
            SLATE,
            palette::shade(SLATE, 0.75),
        );
        fb.line(
            p(x0, y0 + d * 0.5, z + 12),
            p(x0 + w, y0 + d * 0.5, z + 12),
            palette::shade(SLATE_LIT, 1.15),
        );
        // The sign board across the front, and the lamp over the door.
        let s = p(x0 + 0.12, y0 + d, z + 7);
        fb.fill_rect(s.0 as i32, s.1 as i32, 9, 2, palette::rgb(0x2e, 0x4a, 0x6a));
        for k in 0..3 {
            fb.put(s.0 as i32 + 1 + k * 3, s.1 as i32, palette::rgb(0xf0, 0xec, 0xd8));
        }
        let d2 = p(x0 + w * 0.5, y0 + d, z + 3);
        fb.fill_rect(d2.0 as i32 - 2, d2.1 as i32, 5, 3, palette::rgb(0x4a, 0x3a, 0x2c));
        if glow > 0.35 {
            let l = p(x0 + w * 0.5, y0 + d + 0.05, z + 8);
            fb.put_blended(l.0 as i32, l.1 as i32, palette::rgb(0xff, 0xe0, 0xa0), 0.9);
        }
    }
    // 5. The quay cranes: a small jib on a plinth, with its hook over the water.
    for &(x, y) in &h.cranes {
        let z = z_of(map, x, y);
        let base = cam.project(x as f32 + 0.35, y as f32 + 0.65, z + 1);
        let top = (base.0, base.1 - 16.0);
        fb.fill_rect(base.0 as i32 - 2, base.1 as i32 - 1, 5, 2, palette::rgb(0x5a, 0x5c, 0x60));
        fb.line(base, top, palette::rgb(0xc8, 0x9a, 0x32));
        // The jib leans out over the water.
        let tip = (
            top.0 + if dx != 0 { -dx as f32 * 12.0 } else { -dy as f32 * 6.0 },
            top.1 + if dx != 0 { -dy as f32 * 6.0 } else { -dx as f32 * 12.0 },
        );
        fb.line((base.0, base.1 - 6.0), tip, palette::rgb(0xe8, 0xba, 0x42));
        // The hook and a crate on the end of it.
        fb.put(tip.0 as i32, tip.1 as i32 + 2, palette::rgb(0x3a, 0x3c, 0x40));
        fb.fill_rect(tip.0 as i32 - 1, tip.1 as i32 + 4, 3, 3, palette::rgb(0x8a, 0x6a, 0x3a));
    }
    // 6. The boats alongside.
    for (i, &(x, y)) in h.moored.iter().enumerate() {
        draw_moored_boat(fb, cam, map, x, y, i as u8, h.water_dir);
    }
    let _ = stone;
}

/// A moored boat (B91): one of the boat kit's working boats, tied alongside the
/// quay with its length along the wall. `water_dir` is the way the water lies.
pub fn draw_moored_boat(fb: &mut Framebuffer, cam: &Camera, map: &TileMap, x: i32, y: i32, variant: u8, water_dir: (i32, i32)) {
    use crate::boats::{self, BoatKind, Pose};
    let level = map.surface_level_at(x as f32, y as f32);
    let z = (level * HEIGHT_UNIT as f32).round() as i32;
    let c = cam.project(x as f32 + 0.5, y as f32 + 0.5, z);
    let kinds = [BoatKind::Fishing, BoatKind::Lobster, BoatKind::Dinghy, BoatKind::Launch, BoatKind::Fishing, BoatKind::Rowboat];
    let kind = kinds[(variant as usize * 5 + x as usize + y as usize) % kinds.len()];
    // Along the quay: across the direction the water lies, bow either way.
    let along = if water_dir.0 != 0 { (0.0, 1.0) } else { (1.0, 0.0) };
    let sign = if (x + y + variant as i32) % 2 == 0 { 1.0 } else { -1.0 };
    boats::draw(
        fb,
        &Pose {
            kind,
            variant: (variant as usize * 3 + x as usize) as u8 % boats::VARIANTS as u8,
            dir: (along.0 * sign, along.1 * sign),
            sx: c.0 as i32,
            sy: c.1 as i32,
            night: false,
            age: 0.0,
            moving: false,
            depth: None,
            haze: None,
            haze_to: 0,
        },
    );
}

// ------------------------------------------------------------- dynamic art

/// A level crossing with its barriers and lights: `phase` 0 = open (arms up),
/// 1 = shut (arms across the road). The sim decides the phase from the train's
/// position along the line (B58).
pub fn draw_crossing(
    fb: &mut Framebuffer,
    cam: &Camera,
    map: &TileMap,
    x: i32,
    y: i32,
    dir: (f32, f32),
    phase: f32,
    blink: f32,
) {
    let z = z_of(map, x, y);
    let phase = phase.clamp(0.0, 1.0);
    // `dir` is the direction the *track* runs; the road crosses it, so the
    // barriers stand at the road's two edges and the arms swing across the road.
    let along_x = dir.0.abs() >= dir.1.abs();
    // The posts: on both sides of the road (a tile step across it).
    let posts: [(f32, f32); 2] = if along_x {
        // Track runs along +x: the road runs along y; posts at either y edge.
        [(x as f32 + 0.5, y as f32 + 0.12), (x as f32 + 0.5, y as f32 + 0.88)]
    } else {
        [(x as f32 + 0.12, y as f32 + 0.5), (x as f32 + 0.88, y as f32 + 0.5)]
    };
    let white = palette::rgb(0xe8, 0xe4, 0xd8);
    let red = palette::rgb(0xc0, 0x3a, 0x2a);
    // The arm swings from vertical (up) to a quarter tile across the road.
    // A road is one tile wide, which is 32 px across the screen: the arm has to
    // reach most of the way over it (B58: the first attempt was half a lane).
    let across_screen = if along_x { (-26.0f32, 13.0) } else { (26.0, 13.0) };
    for (i, &(px, py)) in posts.iter().enumerate() {
        let base = cam.project(px, py, z);
        // The post: nine pixels of red/white banding.
        for k in 0..9 {
            let c = if (k / 3) % 2 == 0 { white } else { red };
            fb.put(base.0 as i32, base.1 as i32 - k, c);
        }
        let top = (base.0, base.1 - 9.0);
        let side = if i == 0 { -1.0 } else { 1.0 };
        // Phase 0: straight up. Phase 1: flat across the road.
        let end = (
            top.0 + across_screen.0 * side * phase,
            top.1 - 11.0 * (1.0 - phase),
        );
        // The arm, striped: three white/red segments.
        for seg in 0..3 {
            let t0 = seg as f32 / 3.0;
            let t1 = (seg as f32 + 1.0) / 3.0;
            let c = if seg % 2 == 0 { white } else { red };
            fb.line(
                (top.0 + (end.0 - top.0) * t0, top.1 + (end.1 - top.1) * t0),
                (top.0 + (end.0 - top.0) * t1, top.1 + (end.1 - top.1) * t1),
                c,
            );
        }
        // The light on the post: red and flashing while the crossing is shut.
        let on = phase > 0.02 && ((blink * 1.6) as i32) % 2 == 0;
        let lc = if on { palette::rgb(0xff, 0x4a, 0x30) } else { palette::rgb(0x5a, 0x1a, 0x14) };
        fb.put(base.0 as i32, base.1 as i32 - 11, lc);
        if on {
            fb.put_blended(base.0 as i32 + 1, base.1 as i32 - 10, palette::rgb(0xff, 0x9a, 0x70), 0.45);
        }
    }
}

/// A container crane's trolley and its swinging container (B58): the mast and
/// boom are baked with the port, the load is a mover so it can move.
pub fn draw_crane_trolley(
    fb: &mut Framebuffer,
    cam: &Camera,
    map: &TileMap,
    x: i32,
    y: i32,
    dir: (f32, f32),
    phase: f32,
) {
    let z = z_of(map, x, y);
    let base = cam.project(x as f32 + 0.5, y as f32 + 0.5, z + 2);
    let top = (base.0, base.1 - 20.0);
    // Out and back: 0..0.5 out over the water, 0.5..1 back.
    let out = if phase < 0.5 { phase * 2.0 } else { 2.0 - phase * 2.0 };
    let reach = (dir.0 * 10.0, dir.1 * 5.0);
    let tx = top.0 + reach.0 * out;
    let ty = top.1 + reach.1 * out;
    // The trolley.
    fb.fill_rect(tx as i32 - 2, ty as i32 - 1, 5, 2, palette::rgb(0xf2, 0xc8, 0x58));
    // The spreader and its container, hanging 6 px below.
    fb.line((tx, ty + 1.0), (tx, ty + 6.0), DARK_STEEL);
    let col = if (phase * 4.0) as i32 % 3 == 0 {
        palette::rgb(0xc0, 0x4a, 0x3a)
    } else if (phase * 4.0) as i32 % 3 == 1 {
        palette::rgb(0x3a, 0x6a, 0xa8)
    } else {
        palette::rgb(0x3a, 0x8a, 0x5a)
    };
    fb.fill_rect(tx as i32 - 3, ty as i32 + 6, 7, 4, col);
    fb.fill_rect(tx as i32 - 3, ty as i32 + 9, 7, 1, palette::shade(col, 0.7));
    fb.fill_rect(tx as i32 - 3, ty as i32 + 6, 7, 1, palette::shade(col, 1.2));
}

/// A worker on the cut-and-cover: helmet, vest, a shovel (B58: the crews).
pub fn draw_crew(fb: &mut Framebuffer, cam: &Camera, map: &TileMap, x: f32, y: f32, frame: bool) {
    let z = (map.surface_level_at(x, y) * HEIGHT_UNIT as f32).round() as i32;
    let c = cam.project(x, y, z + 1);
    let (cx, cy) = (c.0 as i32, c.1 as i32);
    fb.put(cx, cy - 1, VEST);
    fb.put(cx, cy, VEST);
    fb.put(cx, cy - 2, HELMET);
    // Legs alternate with the walk frame.
    fb.put(cx, cy + 1, palette::rgb(0x3a, 0x3e, 0x4a));
    fb.put(if frame { cx + 1 } else { cx - 1 }, cy + 1, palette::rgb(0x3a, 0x3e, 0x4a));
    // The shovel, over the shoulder.
    fb.put(cx + 1, cy - 2, palette::rgb(0x8a, 0x8e, 0x92));
    fb.put(cx + 2, cy - 3, palette::rgb(0x6a, 0x6e, 0x72));
}

/// Draw the track itself on a tile (the line, the second track, a siding).
/// Shared by every rail surface so the three read as one railway.
pub fn draw_track_tile(
    fb: &mut Framebuffer,
    cam: &Camera,
    map: &TileMap,
    x: i32,
    y: i32,
    track: &[(i32, i32)],
) {
    let z = z_of(map, x, y);
    let c = quad(cam, map, x, y);
    let ballast = palette::rgb(0x6b, 0x5c, 0x4a);
    let salt = ((x as u32) << 8) ^ (y as u32);
    crate::tex::diamond_tri_px(fb, &c, (0, 1, 2), |px, py, _u, _v| {
        crate::tex::grain_px(ballast, px, py, salt)
    });
    crate::tex::diamond_tri_px(fb, &c, (0, 2, 3), |px, py, _u, _v| {
        crate::tex::grain_px(palette::shade(ballast, 0.95), px, py, salt ^ 1)
    });
    let along_x = track.contains(&(x + 1, y)) || track.contains(&(x - 1, y));
    let along_y = track.contains(&(x, y - 1)) || track.contains(&(x, y + 1));
    let (a, b, perp) = if along_x && !along_y {
        (
            cam.project(x as f32 + 0.5, y as f32 + 0.18, z),
            cam.project(x as f32 + 0.5, y as f32 + 0.82, z),
            (0.0f32, -1.0f32),
        )
    } else if along_y && !along_x {
        (
            cam.project(x as f32 + 0.18, y as f32 + 0.5, z),
            cam.project(x as f32 + 0.82, y as f32 + 0.5, z),
            (-1.0f32, 0.0f32),
        )
    } else {
        (
            cam.project(x as f32 + 0.2, y as f32 + 0.2, z),
            cam.project(x as f32 + 0.8, y as f32 + 0.8, z),
            (0.0f32, 0.0f32),
        )
    };
    let steps = 5;
    let sleeper = palette::rgb(0x3a, 0x2e, 0x24);
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let px = a.0 + (b.0 - a.0) * t;
        let py = a.1 + (b.1 - a.1) * t;
        fb.line((px - 5.0, py + 2.5), (px + 5.0, py - 2.5), sleeper);
        fb.line((px - 4.0, py + 1.5), (px + 4.0, py - 1.5), palette::shade(sleeper, 1.35));
    }
    let steel = palette::rgb(0xd8, 0xd4, 0xc8);
    let dark = palette::rgb(0x8a, 0x86, 0x7c);
    for off in [2.6f32, -2.6f32] {
        let (a0, a1) = (
            (a.0 + perp.0 * off, a.1 + perp.1 * off),
            (b.0 + perp.0 * off, b.1 + perp.1 * off),
        );
        fb.line(a0, a1, dark);
        fb.line((a0.0, a0.1 - 1.0), (a1.0, a1.1 - 1.0), steel);
    }
}

/// A catenary mast on this track tile and the wire back to the previous mast
/// (B58: electrification is a visible upgrade).
pub fn draw_catenary_tile(
    fb: &mut Framebuffer,
    cam: &Camera,
    map: &TileMap,
    x: i32,
    y: i32,
    track: &[(i32, i32)],
    along_x: bool,
) {
    let z = z_of(map, x, y);
    // The mast stands on the far side of the track from the camera.
    let (mx, my) = if along_x {
        (x as f32 + 0.5, y as f32 + 0.02)
    } else {
        (x as f32 + 0.02, y as f32 + 0.5)
    };
    let base = cam.project(mx, my, z);
    let top = (base.0, base.1 - 24.0);
    fb.line(base, top, palette::rgb(0x5a, 0x5e, 0x64));
    // The cantilever out over the track.
    let over = if along_x {
        cam.project(mx, my + 0.46, z + 24)
    } else {
        cam.project(mx + 0.46, my, z + 24)
    };
    fb.line(top, over, palette::rgb(0x5a, 0x5e, 0x64));
    fb.put(over.0 as i32, over.1 as i32 + 1, palette::rgb(0x3a, 0x3e, 0x44));
    // The wire: forward to the next mast on the same track.
    let (nx, ny) = if along_x { (x + 1, y) } else { (x, y + 1) };
    if track.contains(&(nx, ny)) {
        let next = if along_x {
            cam.project(nx as f32, my, z + 24)
        } else {
            cam.project(mx, ny as f32, z + 24)
        };
        fb.line(over, next, palette::rgb(0x2e, 0x32, 0x38));
    }
}

/// A wagon standing in the yard (B58): a box van, a tank, a hopper or a loaded
/// flat, according to `kind`.
pub fn draw_parked_wagon(fb: &mut Framebuffer, cam: &Camera, map: &TileMap, x: i32, y: i32, kind: u8) {
    let z = z_of(map, x, y);
    let (body, top) = match kind % 4 {
        0 => (palette::rgb(0x7a, 0x3a, 0x2e), palette::rgb(0x9a, 0x4a, 0x38)),
        1 => (palette::rgb(0x8a, 0x8e, 0x92), palette::rgb(0xa8, 0xac, 0xb0)),
        2 => (palette::rgb(0x5a, 0x6a, 0x3a), palette::rgb(0x76, 0x86, 0x4e)),
        _ => (palette::rgb(0x8a, 0x6a, 0x3a), palette::rgb(0xa8, 0x86, 0x4a)),
    };
    box_at(
        fb,
        cam,
        x as f32 + 0.12,
        y as f32 + 0.28,
        z,
        0.76,
        0.44,
        9,
        top,
        body,
        palette::shade(body, 0.75),
    );
    // A wheel pair at each end.
    let p = |u: f32, v: f32| cam.project(u, v, z);
    for u in [0.2f32, 0.8] {
        let c = p(x as f32 + u, y as f32 + 0.72);
        fb.put(c.0 as i32, c.1 as i32, DARK_STEEL);
    }
}

/// A bus standing at the depot.
pub fn draw_parked_bus(fb: &mut Framebuffer, cam: &Camera, map: &TileMap, x: i32, y: i32) {
    let z = z_of(map, x, y);
    let body = palette::rgb(0xc8, 0x8a, 0x2a);
    box_at(
        fb,
        cam,
        x as f32 + 0.1,
        y as f32 + 0.3,
        z,
        0.8,
        0.42,
        11,
        palette::shade(body, 1.15),
        body,
        palette::shade(body, 0.75),
    );
    let p = |u: f32, v: f32, zz: i32| cam.project(u, v, zz);
    // A window strip down the visible side.
    fb.fill_poly(
        &[
            p(x as f32 + 0.16, y as f32 + 0.72, z + 9),
            p(x as f32 + 0.84, y as f32 + 0.72, z + 9),
            p(x as f32 + 0.84, y as f32 + 0.72, z + 6),
            p(x as f32 + 0.16, y as f32 + 0.72, z + 6),
        ],
        palette::rgb(0x28, 0x32, 0x3c),
    );
}

/// The bus depot's shed: an open-fronted garage with a corrugated roof.
pub fn draw_depot_shed(fb: &mut Framebuffer, cam: &Camera, map: &TileMap, x: i32, y: i32) {
    let z = z_of(map, x, y);
    box_at(
        fb,
        cam,
        x as f32 + 0.05,
        y as f32 + 0.05,
        z,
        0.9,
        0.9,
        12,
        palette::rgb(0xb8, 0xbc, 0xc0),
        palette::rgb(0x8a, 0x8e, 0x92),
        palette::rgb(0x6a, 0x6e, 0x72),
    );
    let p = |u: f32, v: f32, zz: i32| cam.project(u, v, zz);
    // Two open bays, dark inside.
    for k in 0..2 {
        let u = x as f32 + 0.16 + k as f32 * 0.4;
        fb.fill_poly(
            &[
                p(u, y as f32 + 0.95, z + 9),
                p(u + 0.24, y as f32 + 0.95, z + 9),
                p(u + 0.24, y as f32 + 0.95, z + 1),
                p(u, y as f32 + 0.95, z + 1),
            ],
            palette::rgb(0x2a, 0x2c, 0x30),
        );
    }
}
