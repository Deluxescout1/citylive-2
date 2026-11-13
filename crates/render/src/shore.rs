//! B79: life on the shore and the water's edge: beach umbrellas and towels,
//! sunbathers, swimmers, an angler on the pier, gulls.
//!
//! These are small hand-made sprites (like the people and the boat kit): built
//! once, drawn as movers with the painter's depth plane, so a towel hides
//! behind a house and a swimmer's head stays above its own ripple. The sim
//! decides who is out (`sim::isles`: the temperature, the hour, the week, the
//! storm); this file only knows how they look.

use crate::framebuffer::{Framebuffer, Sprite};
use crate::iso::Camera;
use crate::palette::{self, rgb, Color};
use map::TileMap;
use std::sync::OnceLock;

/// What a shore body is (the low four bits of its `set`; the variant is the top
/// four). Mirrors `sim::isles::ShoreKind`.
pub const UMBRELLA: u8 = 0;
pub const TOWEL: u8 = 1;
pub const SUNBATHER: u8 = 2;
pub const SWIMMER: u8 = 3;
pub const ANGLER: u8 = 4;
pub const GULL: u8 = 5;
pub const GULL_PERCHED: u8 = 6;
pub const CHILD: u8 = 7;
pub const WADER: u8 = 8;
pub const DEBRIS: u8 = 9;
pub const HAZARD: u8 = 10;

const SKIN: [Color; 5] = [
    rgb(0xf0, 0xc4, 0x9c),
    rgb(0xe0, 0xa8, 0x7c),
    rgb(0xc4, 0x8a, 0x60),
    rgb(0x9a, 0x66, 0x44),
    rgb(0xf4, 0xd2, 0xb0),
];
const HAIR: [Color; 5] = [
    rgb(0x2e, 0x22, 0x1a),
    rgb(0x6a, 0x42, 0x24),
    rgb(0xc8, 0x9a, 0x44),
    rgb(0x22, 0x1c, 0x18),
    rgb(0xa0, 0x58, 0x30),
];
/// Umbrella panel colours (the white panels are always white).
const BRELLA: [Color; 8] = [
    rgb(0xd8, 0x3c, 0x34),
    rgb(0x2e, 0x7a, 0xc8),
    rgb(0xf0, 0xb8, 0x28),
    rgb(0xee, 0x7a, 0x26),
    rgb(0x22, 0xa4, 0x9a),
    rgb(0xe8, 0x64, 0x9c),
    rgb(0x4c, 0xae, 0x48),
    rgb(0x8a, 0x56, 0xc0),
];
const SUIT: [Color; 8] = [
    rgb(0xd8, 0x3c, 0x34),
    rgb(0x2e, 0x7a, 0xc8),
    rgb(0xf0, 0xd0, 0x40),
    rgb(0x22, 0xa4, 0x9a),
    rgb(0xe8, 0x64, 0x9c),
    rgb(0xf4, 0xf0, 0xe6),
    rgb(0x3a, 0x44, 0x8a),
    rgb(0xee, 0x7a, 0x26),
];

struct Art {
    umbrella: Vec<Sprite>,
    towel: Vec<[Sprite; 2]>,
    sunbather: Vec<[Sprite; 2]>,
    swimmer: Vec<[Sprite; 2]>,
    wader: Vec<Sprite>,
    child: Vec<Sprite>,
    angler: Vec<[Sprite; 2]>,
    gull: [[Sprite; 2]; 2],
    perched: [Sprite; 2],
}

fn art() -> &'static Art {
    static ART: OnceLock<Art> = OnceLock::new();
    ART.get_or_init(build)
}

/// The bytes the art holds (the memory report).
pub fn bytes() -> usize {
    let a = art();
    let s2 = |v: &Vec<[Sprite; 2]>| v.iter().flatten().map(|s| s.bytes()).sum::<usize>();
    a.umbrella.iter().map(|s| s.bytes()).sum::<usize>()
        + s2(&a.towel)
        + s2(&a.sunbather)
        + s2(&a.swimmer)
        + a.wader.iter().map(|s| s.bytes()).sum::<usize>()
        + a.child.iter().map(|s| s.bytes()).sum::<usize>()
        + s2(&a.angler)
        + a.gull.iter().flatten().map(|s| s.bytes()).sum::<usize>()
        + a.perched.iter().map(|s| s.bytes()).sum::<usize>()
}

fn mirror(s: &Sprite) -> Sprite {
    let mut o = Sprite::new(s.w, s.h);
    o.set_anchor(s.w as i32 - 1 - s.ax, s.ay);
    for y in 0..s.h {
        for x in 0..s.w {
            o.px[y * s.w + (s.w - 1 - x)] = s.px[y * s.w + x];
        }
    }
    o
}

/// A sprite from rows of characters; `pal` maps a character to a colour.
fn from_rows(rows: &[&str], pal: &[(char, Color)], ax: i32, ay: i32) -> Sprite {
    let w = rows.iter().map(|r| r.chars().count()).max().unwrap_or(1);
    let mut s = Sprite::new(w, rows.len());
    s.set_anchor(ax, ay);
    for (y, r) in rows.iter().enumerate() {
        for (x, ch) in r.chars().enumerate() {
            if let Some(&(_, c)) = pal.iter().find(|(k, _)| *k == ch) {
                s.set(x, y, c);
            }
        }
    }
    s
}

/// Fill a convex quad on a sprite (scanlines).
fn fill_quad(s: &mut Sprite, p: [(f32, f32); 4], c: impl Fn(i32, i32) -> Color) {
    let (mut y0, mut y1) = (f32::MAX, f32::MIN);
    for q in p {
        y0 = y0.min(q.1);
        y1 = y1.max(q.1);
    }
    for y in y0.floor() as i32..=y1.ceil() as i32 {
        let fy = y as f32 + 0.5;
        let (mut xa, mut xb) = (f32::MAX, f32::MIN);
        for i in 0..4 {
            let (a, b) = (p[i], p[(i + 1) % 4]);
            if (a.1 <= fy && b.1 > fy) || (b.1 <= fy && a.1 > fy) {
                let t = (fy - a.1) / (b.1 - a.1);
                let x = a.0 + (b.0 - a.0) * t;
                xa = xa.min(x);
                xb = xb.max(x);
            }
        }
        if xa > xb {
            continue;
        }
        for x in xa.round() as i32..xb.round() as i32 {
            if x >= 0 && y >= 0 && (x as usize) < s.w && (y as usize) < s.h {
                s.set(x as usize, y as usize, c(x, y));
            }
        }
    }
}

fn umbrella(var: usize) -> Sprite {
    let col = BRELLA[var % 8];
    let white = rgb(0xf6, 0xf2, 0xe6);
    let mut s = Sprite::new(19, 17);
    s.set_anchor(9, 16);
    // The dome: a flattened half ellipse with six panels, lit from the right.
    let (cx, rx, ry) = (9.0f32, 9.0f32, 5.0f32);
    for y in 0..6i32 {
        for x in 0..19i32 {
            let dx = (x as f32 - cx) / rx;
            let dy = (5.0 - y as f32 - 0.2) / ry;
            if dx * dx + dy * dy > 1.0 {
                continue;
            }
            // Panels radiate from the top: width follows the dome.
            let panel = (((x as f32 - cx) / 1.7).floor() as i32).rem_euclid(2);
            let base = if panel == 0 { col } else { white };
            let lit = if x as f32 > cx { 1.08 } else { 0.9 };
            let top = if y <= 1 { 1.1 } else { 1.0 };
            s.set(x as usize, (y + 1) as usize, palette::shade(base, lit * top));
        }
    }
    // The scalloped hem: a dark edge under the rim.
    for x in 0..19usize {
        let y = if x % 3 == 1 { 7 } else { 6 };
        let dx = (x as f32 - cx) / rx;
        if dx.abs() < 1.0 {
            let panel = (((x as f32 - cx) / 1.7).floor() as i32).rem_euclid(2);
            let base = if panel == 0 { col } else { white };
            s.set(x, y, palette::shade(base, 0.7));
        }
    }
    // Finial and pole.
    s.set(9, 0, palette::shade(col, 0.8));
    for y in 7..16 {
        s.set(9, y, rgb(0xd8, 0xd0, 0xc0));
        s.set(10, y, rgb(0x9a, 0x92, 0x84));
    }
    // A little mound of sand at its foot.
    s.set(8, 16, rgb(0xd8, 0xc0, 0x88));
    s.set(11, 16, rgb(0xc8, 0xaa, 0x70));
    s
}

/// A towel lying on the sand, long side along +x (`along_y` swaps it).
fn towel(var: usize, along_y: bool) -> Sprite {
    let a = BRELLA[var % 8];
    let b = rgb(0xf6, 0xf2, 0xe6);
    let mut s = Sprite::new(18, 11);
    s.set_anchor(9, 6);
    let (ex, ey) = if along_y { ((6.4, 3.2), (-12.8, 6.4)) } else { ((12.8, 6.4), (-6.4, 3.2)) };
    let o = if along_y { (11.0, 0.5) } else { (7.0, 0.5) };
    let q = [
        o,
        (o.0 + ex.0, o.1 + ex.1),
        (o.0 + ex.0 + ey.0, o.1 + ex.1 + ey.1),
        (o.0 + ey.0, o.1 + ey.1),
    ];
    fill_quad(&mut s, q, |x, y| {
        // Stripes across the long side.
        let k = if along_y { y - x / 2 } else { x / 2 + y };
        let stripe = (k / 2).rem_euclid(2) == 0;
        if stripe { a } else { b }
    });
    // A shadow line along the front edge.
    for x in 0..s.w {
        for y in (1..s.h).rev() {
            if s.px[y * s.w + x] == 0 && s.px[(y - 1) * s.w + x] != 0 {
                s.set(x, y, rgb(0xb8, 0xa0, 0x6c));
                break;
            }
        }
    }
    s
}

/// A sunbather lying along +x on a towel.
fn sunbather(var: usize, along_y: bool) -> Sprite {
    let skin = SKIN[var % 5];
    let hair = HAIR[(var / 2) % 5];
    let suit = SUIT[var % 8];
    let mut s = Sprite::new(18, 11);
    s.set_anchor(9, 6);
    // Segments along the body, 14 px long down the iso axis (2 px across, 1 px
    // down); thickness 3 px.
    let seg: [(char, i32); 14] = [
        ('h', 0), ('h', 1), ('f', 2), ('t', 3), ('t', 4), ('t', 5), ('t', 6), ('b', 7), ('b', 8), ('l', 9), ('l', 10), ('l', 11), ('l', 12), ('l', 13),
    ];
    for (k, &(part, i)) in seg.iter().enumerate() {
        let (x, y) = (2 + i, 2 + i / 2);
        let _ = k;
        for t in 0..3i32 {
            let c = match part {
                'h' => if t == 0 { hair } else { skin },
                'f' => skin,
                't' => if t >= 1 { suit } else { skin },
                'b' => if t >= 1 { suit } else { skin },
                _ => if t == 2 { palette::shade(skin, 0.82) } else { skin },
            };
            s.set(x as usize, (y + t) as usize, c);
        }
    }
    // A hat's worth of shade over the face: a hair highlight.
    s.set(2, 2, palette::shade(hair, 1.3));
    if along_y {
        mirror(&s)
    } else {
        s
    }
}

fn swimmer(var: usize, frame: usize) -> Sprite {
    let skin = SKIN[var % 5];
    let hair = if var % 3 == 0 { SUIT[var % 8] } else { HAIR[var % 5] };
    let mut s = Sprite::new(13, 8);
    s.set_anchor(6, 6);
    // Head.
    s.set_rect(5, 0, 3, 1, hair);
    s.set_rect(5, 1, 3, 1, hair);
    s.set_rect(5, 2, 3, 2, skin);
    s.set(7, 2, palette::shade(skin, 0.8));
    // Shoulders, just above the water.
    s.set_rect(3, 4, 7, 2, skin);
    s.set_rect(4, 5, 5, 1, palette::shade(skin, 0.86));
    // The stroke: one arm reaching, the other pulling.
    if frame == 0 {
        s.set_rect(10, 3, 2, 1, skin);
        s.set(12, 2, skin);
        s.set_rect(1, 5, 2, 1, skin);
    } else {
        s.set_rect(1, 3, 2, 1, skin);
        s.set(0, 2, skin);
        s.set_rect(10, 5, 2, 1, skin);
    }
    s
}

fn wader(var: usize) -> Sprite {
    let skin = SKIN[var % 5];
    let hair = HAIR[(var + 1) % 5];
    let suit = SUIT[(var * 3) % 8];
    let mut s = Sprite::new(9, 13);
    s.set_anchor(4, 12);
    s.set_rect(3, 0, 3, 1, hair);
    s.set_rect(3, 1, 3, 2, skin);
    s.set_rect(2, 3, 5, 4, suit);
    s.set_rect(3, 3, 3, 1, palette::shade(suit, 1.15));
    s.set_rect(1, 3, 1, 4, skin);
    s.set_rect(7, 3, 1, 4, skin);
    s.set_rect(3, 7, 3, 3, palette::shade(skin, 0.9));
    s
}

fn child(var: usize) -> Sprite {
    let skin = SKIN[var % 5];
    let hair = HAIR[(var + 2) % 5];
    let shirt = SUIT[(var * 5 + 1) % 8];
    let mut s = Sprite::new(13, 10);
    s.set_anchor(4, 9);
    s.set_rect(2, 0, 3, 1, hair);
    s.set_rect(2, 1, 3, 2, skin);
    s.set_rect(1, 3, 5, 3, shirt);
    s.set_rect(0, 4, 1, 2, skin);
    s.set_rect(6, 4, 1, 2, skin);
    s.set(2, 6, rgb(0x2e, 0x5a, 0x9a));
    s.set(4, 6, rgb(0x2e, 0x5a, 0x9a));
    s.set(2, 7, skin);
    s.set(4, 7, skin);
    s.set(2, 8, skin);
    s.set(4, 8, skin);
    // The beach ball.
    let ball = [rgb(0xe8, 0x40, 0x34), rgb(0xf6, 0xf2, 0xe6), rgb(0x2e, 0x7a, 0xc8)];
    s.set_rect(9, 5, 4, 4, ball[0]);
    s.set_rect(10, 5, 2, 4, ball[1]);
    s.set_rect(9, 6, 4, 1, ball[2]);
    s.set(9, 5, 0);
    s.set(12, 5, 0);
    s.set(9, 8, 0);
    s.set(12, 8, 0);
    s
}

/// An angler sitting on the deck's edge, facing +x or +y (the rod and the line
/// are drawn per frame).
fn angler(var: usize, face_y: bool) -> Sprite {
    let skin = SKIN[var % 5];
    let shirt = [rgb(0x3a, 0x6a, 0x9a), rgb(0xc8, 0x5a, 0x3a), rgb(0x4a, 0x8a, 0x4a), rgb(0xd8, 0xc0, 0x50)][var % 4];
    let hat = [rgb(0xe0, 0xc4, 0x70), rgb(0x2e, 0x3a, 0x5a), rgb(0xc8, 0x3c, 0x34)][var % 3];
    let mut s = Sprite::new(11, 14);
    s.set_anchor(5, 13);
    // Hat with a brim, face, body, trousers, boots dangling.
    s.set_rect(2, 0, 5, 1, hat);
    s.set_rect(1, 1, 7, 1, palette::shade(hat, 0.82));
    s.set_rect(3, 2, 3, 2, skin);
    s.set_rect(2, 4, 5, 4, shirt);
    s.set_rect(3, 4, 3, 1, palette::shade(shirt, 1.15));
    s.set_rect(6, 5, 2, 1, skin);
    s.set_rect(2, 8, 5, 2, rgb(0x4a, 0x50, 0x68));
    s.set_rect(2, 10, 2, 3, rgb(0x4a, 0x50, 0x68));
    s.set_rect(5, 10, 2, 3, rgb(0x4a, 0x50, 0x68));
    s.set_rect(2, 12, 2, 1, rgb(0x2a, 0x26, 0x22));
    s.set_rect(5, 12, 2, 1, rgb(0x2a, 0x26, 0x22));
    if face_y {
        mirror(&s)
    } else {
        s
    }
}

fn gull(wings_up: bool) -> Sprite {
    let w = rgb(0xf6, 0xf6, 0xf2);
    let g = rgb(0xb4, 0xba, 0xc4);
    let k = rgb(0x34, 0x38, 0x42);
    let y = rgb(0xf0, 0xb0, 0x30);
    let rows: &[&str] = if wings_up {
        &[
            "k.........k.",
            "gk.......kg.",
            ".gw.....wg..",
            "..gwwwwwg...",
            "...wwwwwwwy.",
            "....wwwwww..",
            ".....g......",
        ]
    } else {
        &[
            "............",
            "............",
            "....wwwww...",
            "gggwwwwwwwy.",
            ".gkwwwwwww..",
            "..k.wwww....",
            "......g.....",
        ]
    };
    from_rows(rows, &[('w', w), ('g', g), ('k', k), ('y', y)], 5, 3)
}

fn perched() -> Sprite {
    let w = rgb(0xf6, 0xf6, 0xf2);
    let g = rgb(0xa8, 0xb0, 0xba);
    let y = rgb(0xf0, 0xb0, 0x30);
    let k = rgb(0x34, 0x38, 0x42);
    from_rows(
        &[
            "...ww..",
            "..wwwy.",
            ".gwwww.",
            "ggwwww.",
            "gkwww..",
            ".g.o.o.",
        ],
        &[('w', w), ('g', g), ('y', y), ('k', k), ('o', rgb(0xe0, 0x8a, 0x34))],
        3,
        5,
    )
}

fn build() -> Art {
    Art {
        umbrella: (0..8).map(umbrella).collect(),
        towel: (0..8).map(|v| [towel(v, false), towel(v, true)]).collect(),
        sunbather: (0..8).map(|v| [sunbather(v, false), sunbather(v, true)]).collect(),
        swimmer: (0..8).map(|v| [swimmer(v, 0), swimmer(v, 1)]).collect(),
        wader: (0..8).map(wader).collect(),
        child: (0..8).map(child).collect(),
        angler: (0..8).map(|v| [angler(v, false), angler(v, true)]).collect(),
        gull: [[gull(true), gull(false)], [mirror(&gull(true)), mirror(&gull(false))]],
        perched: [perched(), mirror(&perched())],
    }
}

/// How the shore body's `set` byte splits.
pub fn decode(set: u8) -> (u8, usize) {
    (set & 0x0f, (set >> 4) as usize & 7)
}

/// The screen rect (centre-relative extents) a shore body touches: half width,
/// rows up and down. Used for the dirty rect.
pub fn extent(set: u8, lift: i32) -> (i32, i32, i32) {
    match decode(set).0 {
        UMBRELLA => (11, 20, 4),
        TOWEL | SUNBATHER => (11, 8, 6),
        SWIMMER | WADER => (11, 16, 5),
        ANGLER => (24, 40, 20),
        GULL => (9, 10 + lift, 6 + lift),
        GULL_PERCHED => (6, 10, 3),
        DEBRIS => (9, 6, 5),
        HAZARD => (8, 22, 4),
        _ => (9, 14, 4),
    }
}

/// Draw one shore body. `(x, y)` is the tile position, `dir` the way it faces,
/// `lift` the height in px (gulls), `phase` 0..1 for the animation.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    fb: &mut Framebuffer,
    cam: &Camera,
    map: &TileMap,
    x: f32,
    y: f32,
    dir: (f32, f32),
    set: u8,
    lift: i32,
    phase: f32,
    night: bool,
    depth: &[u16],
    haze: Option<&[f32]>,
    under: &[Color],
) {
    let a = art();
    let (kind, var) = decode(set);
    let z = map.ground_px(x, y);
    let (sx, sy) = cam.project(x, y, z);
    let (sx, sy) = (sx as i32, sy as i32);
    let key = crate::actors::actor_key(x.floor() as i32, y.floor() as i32);
    let along_y = dir.0.abs() > dir.1.abs();
    let hz = |row: i32| haze.and_then(|h| h.get(row.max(0) as usize).copied()).unwrap_or(0.0);
    if hz(sy) > 0.8 {
        return;
    }
    // Draw a sprite with the painter's test and the distance haze.
    let blit = |fb: &mut Framebuffer, s: &Sprite, px: i32, py: i32, depth_test: bool| {
        // Fade towards the static layer's colour under it, not whatever the
        // frame holds (a damaged part of the frame may hold stale pixels).
        let (qx, qy) = ((px + s.ax).clamp(0, fb.w as i32 - 1) as usize, (py + s.ay).clamp(0, fb.h as i32 - 1) as usize);
        let bg = under.get(qy * fb.w + qx).copied().filter(|&c| c != 0).unwrap_or_else(|| fb.px()[qy * fb.w + qx]);
        let rows: Vec<f32>;
        let rows_ref: &[f32] = match haze {
            Some(h) => h,
            None => {
                rows = Vec::new();
                &rows
            }
        };
        let d = if depth_test { Some((key, depth)) } else { None };
        crate::boats::blit_hazed(fb, s, px, py, d, rows_ref, bg);
    };
    let dim = |c: Color| if night { palette::shade(c, 0.55) } else { c };
    let _ = dim;
    match kind {
        UMBRELLA => {
            // The shade under it: a soft dark oval on the sand.
            for (dx, dy, cov) in [(-6, 0, 0.12), (-4, 1, 0.2), (-2, 1, 0.22), (0, 1, 0.22), (2, 1, 0.2), (4, 1, 0.14), (-3, 2, 0.12), (1, 2, 0.12), (5, 0, 0.08)] {
                fb.put_blended(sx + dx, sy + dy, rgb(0x2a, 0x20, 0x18), cov);
            }
            let s = &a.umbrella[var];
            blit(fb, s, sx - s.ax, sy - s.ay, true);
        }
        TOWEL => {
            let s = &a.towel[var][along_y as usize];
            blit(fb, s, sx - s.ax, sy - s.ay, true);
        }
        SUNBATHER => {
            let s = &a.towel[var][along_y as usize];
            blit(fb, s, sx - s.ax, sy - s.ay, true);
            let b = &a.sunbather[var][along_y as usize];
            blit(fb, b, sx - b.ax, sy - b.ay - 1, true);
        }
        CHILD => {
            let s = &a.child[var];
            blit(fb, s, sx - s.ax, sy - s.ay, true);
        }
        SWIMMER | WADER => {
            // The ripple ring and its dark underside, then the bather.
            let t = phase * std::f32::consts::TAU;
            let r = 6 + ((t.sin() + 1.0) * 0.8) as i32;
            for i in -r..=r {
                let h = ((1.0 - (i as f32 / r as f32).powi(2)).max(0.0).sqrt() * 2.0) as i32;
                fb.put_blended(sx + i, sy + h, rgb(0xf0, 0xf8, 0xfc), 0.5);
                fb.put_blended(sx + i, sy - h, rgb(0xf0, 0xf8, 0xfc), 0.28);
                fb.put_blended(sx + i, sy + h + 1, rgb(0x10, 0x34, 0x44), 0.22);
            }
            if kind == SWIMMER {
                let s = &a.swimmer[var][((phase * 2.0) as usize) & 1];
                let bob = if (phase * 4.0) as i32 % 2 == 0 { 0 } else { 1 };
                blit(fb, s, sx - s.ax, sy - s.ay + bob, false);
            } else {
                let s = &a.wader[var];
                blit(fb, s, sx - s.ax, sy - s.ay, false);
            }
        }
        ANGLER => {
            let s = &a.angler[var][along_y as usize];
            // Sat on the deck: feet hang.
            let (px, py) = (sx - s.ax, sy - s.ay);
            blit(fb, s, px, py, true);
            // The rod from the hand, out over the water; the line down to the bobber.
            let step = if along_y { (-1.0f32, 0.5f32) } else { (1.0, 0.5) };
            let hand = ((sx + if along_y { -3 } else { 3 }) as f32, (sy - 8) as f32);
            let tip = (hand.0 + step.0 * 13.0, hand.1 - 6.0 + step.1 * 4.0);
            fb.line(hand, tip, rgb(0x5a, 0x42, 0x2a));
            let bob = (tip.0 + step.0 * 7.0, tip.1 + 14.0 + ((phase * std::f32::consts::TAU).sin()) * 0.6);
            fb.line(tip, bob, rgb(0xe4, 0xec, 0xf0));
            fb.put(bob.0 as i32, bob.1 as i32, rgb(0xe8, 0x40, 0x34));
            fb.put(bob.0 as i32, bob.1 as i32 + 1, rgb(0xf6, 0xf2, 0xe6));
            for i in -2..=2 {
                fb.put_blended(bob.0 as i32 + i, bob.1 as i32 + 2, rgb(0xf0, 0xf8, 0xfc), 0.35);
            }
        }
        GULL => {
            let right = dir.0 >= 0.0;
            let s = &a.gull[(!right) as usize][((phase * 2.0) as usize) & 1];
            // Its shadow on the water below.
            for i in -2..=2 {
                fb.put_blended(sx + i, sy + 1, rgb(0x10, 0x30, 0x40), 0.16);
            }
            blit(fb, s, sx - s.ax, sy - lift - s.ay, false);
        }
        GULL_PERCHED => {
            let s = &a.perched[(dir.0 < 0.0) as usize];
            blit(fb, s, sx - s.ax, sy - lift - s.ay + 6, true);
        }
        DEBRIS => {
            // Planks and a lifebuoy floating where the boats met.
            let plank = rgb(0x8a, 0x68, 0x3e);
            let dark = rgb(0x4a, 0x38, 0x24);
            for (dx, dy, w) in [(-5, 0, 4), (2, 1, 5), (-1, -2, 3), (4, -1, 3), (-3, 2, 3)] {
                fb.fill_rect(sx + dx, sy + dy, w, 1, plank);
                fb.fill_rect(sx + dx, sy + dy + 1, w, 1, dark);
            }
            for (dx, c) in [(0, rgb(0xe8, 0x40, 0x34)), (1, rgb(0xf6, 0xf2, 0xe6)), (2, rgb(0xe8, 0x40, 0x34))] {
                fb.put(sx + 1 + dx, sy - 1, c);
                fb.put(sx + 1 + dx, sy + 1, c);
            }
            fb.put(sx + 1, sy, rgb(0xf6, 0xf2, 0xe6));
            fb.put(sx + 3, sy, rgb(0xf6, 0xf2, 0xe6));
            for i in -7..=7 {
                fb.put_blended(sx + i, sy + 3, rgb(0xf0, 0xf8, 0xfc), 0.25);
            }
        }
        HAZARD => {
            // A buoy with a pole and flag; at night an amber light that blinks.
            fb.fill_rect(sx - 1, sy - 2, 3, 3, rgb(0xe8, 0x60, 0x20));
            fb.fill_rect(sx - 1, sy - 1, 3, 1, rgb(0xf6, 0xf2, 0xe6));
            fb.line((sx as f32, (sy - 2) as f32), (sx as f32, (sy - 14) as f32), rgb(0x3a, 0x3c, 0x40));
            fb.fill_rect(sx + 1, sy - 14, 4, 3, rgb(0xf0, 0x8a, 0x20));
            if night && phase < 0.5 {
                for (dx, dy, c) in [(0, 0, 0.95), (-1, 0, 0.5), (1, 0, 0.5), (0, -1, 0.5), (0, 1, 0.5), (-2, 0, 0.2), (2, 0, 0.2), (0, -2, 0.2), (0, 2, 0.2)] {
                    fb.put_blended(sx + dx, sy - 15 + dy, rgb(0xff, 0xb0, 0x30), c);
                }
            }
            for i in -3..=3 {
                fb.put_blended(sx + i, sy + 1, rgb(0xf0, 0xf8, 0xfc), 0.3);
            }
        }
        _ => {}
    }
}
