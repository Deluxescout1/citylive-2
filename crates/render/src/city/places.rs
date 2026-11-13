//! The places cars need (B59/B59b): car parks, the multi-storey, forecourts,
//! charging hubs, the drive-through, the wash, the dealership and the motel.
//!
//! Every place is laid out by `map::carplaces` (the same definition the sim
//! parks its cars by) in the plot's own frame: `t` along the street, `d` in
//! from the kerb. Painted lines, pumps and lanes are drawn *round the bays*, so a
//! car stands square in the white lines painted for it.
//!
//! Depth: the ground paints with a key just below its first tile, so every car
//! stands on it; each structure (canopy, kiosk, showroom, tunnel) stamps the key
//! of its own far tile, so a car in front of it is drawn over it and a car
//! behind it is hidden by it — the rule buildings already follow.

use super::{CityObj, ObjKind, Stage};
use crate::framebuffer::Framebuffer;
use crate::palette::{self, Color, rgb};
use crate::actors::static_key;
use map::carplaces::{self as cp, Frame, Nose, Rect, Shape};

type Pt = (f32, f32);

// ------------------------------------------------------------------ colours

const KERB: Color = rgb(0xb8, 0xb4, 0xa8);
const ASPHALT: Color = rgb(0x5a, 0x5f, 0x68);
const LINE: Color = rgb(0xf0, 0xee, 0xe2);
const LINE_DIM: Color = rgb(0xc8, 0xc8, 0xc0);
const YELLOW: Color = rgb(0xe6, 0xc2, 0x3a);
const CONC: Color = rgb(0xb4, 0xb0, 0xa6);
const CONC_DARK: Color = rgb(0x6c, 0x6a, 0x66);
const STEEL: Color = rgb(0x7a, 0x84, 0x8e);
const GLASS: Color = rgb(0x5c, 0x82, 0xa4);
/// A colour that keeps its brightness at night: the static layer's tint leaves
/// a pixel with alpha 0xFE alone (lit windows, lamps, signs).
const fn em(c: Color) -> Color {
    (c & 0x00FF_FFFF) | (0xFE << 24)
}
const GLASS_LIT: Color = em(rgb(0xff, 0xe2, 0x9c));
const WARM: Color = rgb(0xff, 0xd2, 0x8a);
const POST_WHITE: Color = rgb(0xe6, 0xe6, 0xe0);
const BOARD: Color = rgb(0x24, 0x2a, 0x36);
const GREEN_LED: Color = rgb(0x62, 0xe0, 0x84);
const AMBER_LED: Color = rgb(0xf0, 0xb0, 0x3c);
const RED_LED: Color = rgb(0xf2, 0x54, 0x48);
const GRASS: Color = rgb(0x5a, 0x86, 0x44);
const FUEL_FASCIA: Color = rgb(0xc8, 0x3c, 0x34);
const FUEL_STRIPE: Color = rgb(0xf0, 0xe8, 0xd0);
const CANOPY_TOP: Color = rgb(0xdc, 0xde, 0xdc);
const KIOSK_WALL: Color = rgb(0xe4, 0xdc, 0xc8);
const KIOSK_ROOF: Color = rgb(0xb0, 0x40, 0x38);
const SOLAR: Color = rgb(0x36, 0x4c, 0x74);
const SOLAR_CELL: Color = rgb(0x5a, 0x7a, 0xae);
const EATERY_WALL: Color = rgb(0xe8, 0xcc, 0x9c);
const EATERY_ROOF: Color = rgb(0xc0, 0x42, 0x38);
const WASH_WALL: Color = rgb(0x84, 0xb0, 0xd0);
const WASH_ROOF: Color = rgb(0x34, 0x64, 0x98);
const WASH_MOUTH: Color = rgb(0x1c, 0x24, 0x34);
const SHOW_ROOF: Color = rgb(0x3a, 0x3e, 0x4a);
const SHOW_FASCIA: Color = rgb(0x3a, 0x86, 0xc4);
const MOTEL_WALL: Color = rgb(0xec, 0xe0, 0xc4);
const MOTEL_ROOF: Color = rgb(0x4e, 0x52, 0x5a);
const MOTEL_DOOR: Color = rgb(0x9a, 0x3e, 0x32);
const MOTEL_RAIL: Color = rgb(0x5a, 0x4a, 0x42);
const NEON: Color = rgb(0xff, 0x5a, 0x88);
const NEON_B: Color = rgb(0x5a, 0xc8, 0xff);

/// Colours of the little cars seen through a deck's openings and a showroom's
/// glass (the sprites cannot stand under a slab).
const PAINT: [Color; 6] = [
    rgb(0xd6, 0x52, 0x4a),
    rgb(0xec, 0xc4, 0x54),
    rgb(0x60, 0xa4, 0xd6),
    rgb(0xee, 0xee, 0xe8),
    rgb(0x78, 0xbe, 0x82),
    rgb(0x5a, 0x5c, 0x64),
];

// -------------------------------------------------------------------- glyphs

/// A 3x5 pixel face for the few words the signs carry. `#` is ink.
fn glyph(c: char) -> [&'static str; 5] {
    match c {
        '0' => ["###", "#.#", "#.#", "#.#", "###"],
        '1' => [".#.", "##.", ".#.", ".#.", "###"],
        '2' => ["###", "..#", "###", "#..", "###"],
        '3' => ["###", "..#", "###", "..#", "###"],
        '4' => ["#.#", "#.#", "###", "..#", "..#"],
        '5' => ["###", "#..", "###", "..#", "###"],
        '6' => ["###", "#..", "###", "#.#", "###"],
        '7' => ["###", "..#", ".#.", ".#.", ".#."],
        '8' => ["###", "#.#", "###", "#.#", "###"],
        '9' => ["###", "#.#", "###", "..#", "###"],
        'P' => ["###", "#.#", "###", "#..", "#.."],
        'E' => ["###", "#..", "##.", "#..", "###"],
        'V' => ["#.#", "#.#", "#.#", "#.#", ".#."],
        'M' => ["#.#", "###", "###", "#.#", "#.#"],
        'O' => ["###", "#.#", "#.#", "#.#", "###"],
        'T' => ["###", ".#.", ".#.", ".#.", ".#."],
        'L' => ["#..", "#..", "#..", "#..", "###"],
        'W' => ["#.#", "#.#", "###", "###", "#.#"],
        'A' => [".#.", "#.#", "###", "#.#", "#.#"],
        'S' => ["###", "#..", "###", "..#", "###"],
        'H' => ["#.#", "#.#", "###", "#.#", "#.#"],
        'F' => ["###", "#..", "##.", "#..", "#.."],
        'U' => ["#.#", "#.#", "#.#", "#.#", "###"],
        'B' => ["##.", "#.#", "##.", "#.#", "##."],
        _ => ["...", "...", "...", "...", "..."],
    }
}

/// Paint `text` with its top-left at `(x, y)`; returns the width used.
fn text(fb: &mut Framebuffer, x: i32, y: i32, s: &str, c: Color) -> i32 {
    let mut cx = x;
    for ch in s.chars() {
        let g = glyph(ch);
        for (row, line) in g.iter().enumerate() {
            for (col, b) in line.bytes().enumerate() {
                if b == b'#' {
                    fb.put(cx + col as i32, y + row as i32, c);
                }
            }
        }
        cx += 4;
    }
    cx - x
}

// ------------------------------------------------------------------- context

struct Cx<'a, P: Fn(f32, f32, i32) -> Pt> {
    fb: &'a mut Framebuffer,
    p: &'a P,
    o: &'a CityObj,
    f: Frame,
    al: f32,
    dl: f32,
    glow: f32,
    hour: f32,
    built: bool,
    look: &'static crate::biome::Ground,
}

fn shape_of(k: ObjKind) -> Option<Shape> {
    Some(match k {
        ObjKind::CarPark => Shape::Park,
        ObjKind::CarParkDeck => Shape::Deck,
        ObjKind::GasStation => Shape::Gas,
        ObjKind::ChargingHub => Shape::Charge,
        ObjKind::DriveThrough => Shape::Drive,
        ObjKind::CarWash => Shape::Wash,
        ObjKind::Dealership => Shape::Dealer,
        ObjKind::Motel => Shape::Motel,
        _ => return None,
    })
}

/// The height and tile-space footprint of what stands on a place, for the
/// shadow field: `(x0, y0, x1, y1, top_px)`.
pub fn casters(o: &CityObj) -> Vec<(f32, f32, f32, f32, f32)> {
    let Some(shape) = shape_of(o.kind) else {
        return Vec::new();
    };
    if o.stage != Stage::Finished {
        return Vec::new();
    }
    let f = Frame::new(o.x, o.y, o.w, o.h, o.front);
    let (al, dl) = f.size();
    let rect = |r: Rect| {
        let a = f.at(r.t0, r.d0);
        let b = f.at(r.t1, r.d1);
        (a.0.min(b.0), a.1.min(b.1), a.0.max(b.0), a.1.max(b.1))
    };
    let mut v = Vec::new();
    if let Some(b) = cp::building_for(shape, al, dl, o.front >= 2) {
        let (x0, y0, x1, y1) = rect(b);
        let top = match shape {
            Shape::Deck => 12.0,
            Shape::Gas => 12.0,
            Shape::Charge => 10.0,
            Shape::Drive => 14.0,
            Shape::Wash => 14.0,
            Shape::Dealer => 13.0,
            Shape::Motel => 25.0,
            Shape::Park => 0.0,
        };
        v.push((x0, y0, x1, y1, top));
    }
    if shape == Shape::Deck {
        let (x0, y0, x1, y1) = (o.x as f32 + 0.12, o.y as f32 + 0.12, (o.x + o.w) as f32 - 0.12, (o.y + o.h) as f32 - 0.12);
        v.push((x0, y0, x1, y1, 33.0));
    }
    if let Some(c) = cp::canopy(shape, al, dl) {
        let (x0, y0, x1, y1) = rect(c);
        v.push((x0, y0, x1, y1, 19.0));
    }
    v
}

/// Draw one place.
pub fn draw<P: Fn(f32, f32, i32) -> Pt>(
    o: &CityObj,
    fb: &mut Framebuffer,
    p: &P,
    glow: f32,
    hour: f32,
    look: &'static crate::biome::Ground,
) {
    let Some(shape) = shape_of(o.kind) else {
        return;
    };
    let f = Frame::new(o.x, o.y, o.w, o.h, o.front);
    let (al, dl) = f.size();
    let mut c = Cx {
        fb,
        p,
        o,
        f,
        al,
        dl,
        glow,
        hour,
        built: o.stage == Stage::Finished,
        look,
    };
    c.ground(shape);
    if !c.built {
        // The trade going in: the apron and the marked bays, a fence is the
        // site art's job elsewhere. Nothing stands on it yet.
        c.bays(shape);
        return;
    }
    match shape {
        Shape::Park => c.park(),
        Shape::Deck => c.deck(),
        Shape::Gas => c.gas(),
        Shape::Charge => c.charge(),
        Shape::Drive => c.drive(),
        Shape::Wash => c.wash(),
        Shape::Dealer => c.dealer(),
        Shape::Motel => c.motel(),
    }
}

impl<'a, P: Fn(f32, f32, i32) -> Pt> Cx<'a, P> {
    // ---------------------------------------------------------------- basics

    fn w(&self, t: f32, d: f32) -> Pt {
        self.f.at(t, d)
    }

    fn night(&self) -> bool {
        self.glow > 0.35
    }

    fn pt(&self, t: f32, d: f32, z: f32) -> Pt {
        let (x, y) = self.w(t, d);
        (self.p)(x, y, z.round() as i32)
    }

    fn wrect(&self, t0: f32, t1: f32, d0: f32, d1: f32) -> (f32, f32, f32, f32) {
        let a = self.w(t0, d0);
        let b = self.w(t1, d1);
        (a.0.min(b.0), a.1.min(b.1), a.0.max(b.0), a.1.max(b.1))
    }

    /// The painter key of a structure whose far tile is the one under `(x, y)`.
    fn key_at(x: f32, y: f32) -> u16 {
        let d = (x - 1e-3).floor() as i32 + (y - 1e-3).floor() as i32;
        static_key(d, true)
    }

    fn key_rect(&mut self, t0: f32, t1: f32, d0: f32, d1: f32) {
        let (_, _, x1, y1) = self.wrect(t0, t1, d0, d1);
        self.fb.set_depth_key(Self::key_at(x1, y1));
    }

    fn key_ground(&mut self) {
        let d = self.o.x + self.o.y - 1;
        self.fb.set_depth_key(static_key(d, true));
    }

    fn slab(&mut self, t0: f32, t1: f32, d0: f32, d1: f32, z: f32, col: Color) {
        let (x0, y0, x1, y1) = self.wrect(t0, t1, d0, d1);
        let zi = z.round() as i32;
        let pts = [(self.p)(x0, y0, zi), (self.p)(x1, y0, zi), (self.p)(x1, y1, zi), (self.p)(x0, y1, zi)];
        self.fb.fill_poly(&pts, col);
    }

    fn slab_blend(&mut self, t0: f32, t1: f32, d0: f32, d1: f32, z: f32, col: Color, cov: f32) {
        let (x0, y0, x1, y1) = self.wrect(t0, t1, d0, d1);
        let zi = z.round() as i32;
        let pts = [(self.p)(x0, y0, zi), (self.p)(x1, y0, zi), (self.p)(x1, y1, zi), (self.p)(x0, y1, zi)];
        self.fb.fill_poly_blended(&pts, col, cov);
    }

    /// A box on the plot: the two faces the camera sees and the top.
    fn boxx(&mut self, t0: f32, t1: f32, d0: f32, d1: f32, z0: f32, z1: f32, top: Color, side: Color) {
        let (x0, y0, x1, y1) = self.wrect(t0, t1, d0, d1);
        let (z0, z1) = (z0.round() as i32, z1.round() as i32);
        let p = self.p;
        self.fb.fill_poly(&[p(x0, y1, z1), p(x1, y1, z1), p(x1, y1, z0), p(x0, y1, z0)], side);
        self.fb.fill_poly(
            &[p(x1, y0, z1), p(x1, y1, z1), p(x1, y1, z0), p(x1, y0, z0)],
            palette::shade(side, 0.80),
        );
        self.fb.fill_poly(&[p(x0, y0, z1), p(x1, y0, z1), p(x1, y1, z1), p(x0, y1, z1)], top);
    }

    /// A point on one of the two camera-facing faces of a world rect:
    /// `left` is the +y face (along x), otherwise the +x face (along y).
    fn fpt(&self, r: (f32, f32, f32, f32), left: bool, u: f32, z: f32) -> Pt {
        let (x0, y0, x1, y1) = r;
        let zi = z.round() as i32;
        if left {
            (self.p)(x0 + (x1 - x0) * u, y1, zi)
        } else {
            (self.p)(x1, y0 + (y1 - y0) * u, zi)
        }
    }

    /// A quad on a camera-facing face, `u` along it, `z` up it.
    fn fquad(&mut self, r: (f32, f32, f32, f32), left: bool, (u0, u1): (f32, f32), (z0, z1): (f32, f32), col: Color) {
        let pts = [
            self.fpt(r, left, u0, z1),
            self.fpt(r, left, u1, z1),
            self.fpt(r, left, u1, z0),
            self.fpt(r, left, u0, z0),
        ];
        self.fb.fill_poly(&pts, col);
    }

    fn fline(&mut self, r: (f32, f32, f32, f32), left: bool, u: f32, (z0, z1): (f32, f32), col: Color) {
        let a = self.fpt(r, left, u, z0);
        let b = self.fpt(r, left, u, z1);
        self.fb.line(a, b, col);
    }

    fn line(&mut self, a: (f32, f32), b: (f32, f32), z: f32, col: Color) {
        let pa = self.pt(a.0, a.1, z);
        let pb = self.pt(b.0, b.1, z);
        self.fb.line(pa, pb, col);
    }

    fn pole(&mut self, t: f32, d: f32, z0: f32, h: f32, col: Color) {
        let a = self.pt(t, d, z0 + h);
        let b = self.pt(t, d, z0);
        self.fb.line(a, b, col);
        // A lit edge so a column reads as round steel, not a hairline.
        self.fb.line((a.0 + 1.0, a.1), (b.0 + 1.0, b.1), palette::shade(col, 0.72));
    }

    fn dot(&mut self, t: f32, d: f32, z: f32, col: Color) {
        let q = self.pt(t, d, z);
        self.fb.put(q.0 as i32, q.1 as i32, col);
    }

    fn glow_dot(&mut self, t: f32, d: f32, z: f32, col: Color) {
        let q = self.pt(t, d, z);
        let (x, y) = (q.0 as i32, q.1 as i32);
        self.fb.put(x, y, em(col));
        self.fb.put_blended(x + 1, y, col, 0.45);
        self.fb.put_blended(x - 1, y, col, 0.45);
        self.fb.put_blended(x, y + 1, col, 0.4);
        self.fb.put_blended(x, y - 1, col, 0.4);
    }

    fn tree(&mut self, t: f32, d: f32, v: u8) {
        let (x, y) = self.w(t, d);
        self.key_at_point(x, y);
        self.o.park_tree_big(self.fb, self.p, x, y, v, self.look);
    }

    fn key_at_point(&mut self, x: f32, y: f32) {
        self.fb.set_depth_key(Self::key_at(x + 0.5, y + 0.5));
    }

    fn lamp(&mut self, t: f32, d: f32) {
        let (x, y) = self.w(t, d);
        self.key_at_point(x, y);
        let top = (self.p)(x, y, 20);
        let base = (self.p)(x, y, 0);
        self.fb.line(top, base, STEEL);
        // The arm and the head, leaning toward the lot.
        self.fb.line(top, (top.0 + 2.0, top.1 + 1.0), STEEL);
        self.fb.put(top.0 as i32 + 2, top.1 as i32 + 2, WARM);
        self.fb.put(top.0 as i32 + 3, top.1 as i32 + 2, WARM);
        if self.night() {
            self.fb.put_blended(top.0 as i32 + 2, top.1 as i32 + 3, WARM, 0.5);
            self.fb.put_blended(top.0 as i32 + 3, top.1 as i32 + 3, WARM, 0.5);
            // The pool of light on the tarmac.
            let (gx, gy) = (base.0 as i32 + 2, base.1 as i32);
            for dy in -2..=2i32 {
                for dx in -5..=5i32 {
                    let r = (dx * dx) as f32 / 25.0 + (dy * dy) as f32 / 4.0;
                    if r < 1.0 {
                        self.fb.put_blended(gx + dx, gy + dy, WARM, 0.18 * (1.0 - r));
                    }
                }
            }
        }
    }

    // ---------------------------------------------------------------- ground

    fn ground(&mut self, shape: Shape) {
        self.key_ground();
        let (al, dl) = (self.al, self.dl);
        let seed = self.o.seed;
        // The kerb ring (raised a pixel), then the tarmac inside it. The
        // street side is left flush: that is where the cars come in.
        let m = 0.04;
        let (x0, y0, x1, y1) = (self.o.x as f32, self.o.y as f32, (self.o.x + self.o.w) as f32, (self.o.y + self.o.h) as f32);
        let p = self.p;
        let kerb_top = [p(x0 + m, y0 + m, 1), p(x1 - m, y0 + m, 1), p(x1 - m, y1 - m, 1), p(x0 + m, y1 - m, 1)];
        self.fb.fill_poly(&kerb_top, KERB);
        // Kerb faces on the two near sides, so the pad stands off the grass.
        self.fb.fill_poly(
            &[p(x0 + m, y1 - m, 1), p(x1 - m, y1 - m, 1), p(x1 - m, y1 - m, 0), p(x0 + m, y1 - m, 0)],
            palette::shade(KERB, 0.82),
        );
        self.fb.fill_poly(
            &[p(x1 - m, y0 + m, 1), p(x1 - m, y1 - m, 1), p(x1 - m, y1 - m, 0), p(x1 - m, y0 + m, 0)],
            palette::shade(KERB, 0.66),
        );
        let n = 0.12;
        let inner = [p(x0 + n, y0 + n, 1), p(x1 - n, y0 + n, 1), p(x1 - n, y1 - n, 1), p(x0 + n, y1 - n, 1)];
        let base = palette::jitter(ASPHALT, self.o.h(0x59), 0.04);
        crate::tex::fill_poly_px(self.fb, &inner, |px, py, _, _| {
            let h = crate::tex::hash2(px / 2, py / 2, seed ^ 0x59);
            if h > 0.88 {
                palette::shade(base, 1.06)
            } else if h < 0.10 {
                palette::shade(base, 0.94)
            } else {
                base
            }
        });
        // The street side: a dropped kerb, the tarmac runs out to the road.
        let tar = palette::shade(base, 1.03);
        let (s0, s1) = (self.w(0.0, 0.0), self.w(al, 0.0));
        let (s2, s3) = (self.w(al, 0.16), self.w(0.0, 0.16));
        self.fb.fill_poly(&[p(s0.0, s0.1, 1), p(s1.0, s1.1, 1), p(s2.0, s2.1, 1), p(s3.0, s3.1, 1)], tar);
        self.fb.fill_poly(&[p(s0.0, s0.1, 1), p(s1.0, s1.1, 1), p(s1.0, s1.1, 0), p(s0.0, s0.1, 0)], tar);
        let _ = (dl, shape);
    }

    /// Grass verges and a few trees round the edges of a plot (not the street
    /// side), so a pad of tarmac is not a grey card on the lawn.
    fn verges(&mut self, tree_spots: &[(f32, f32)]) {
        let (al, dl) = (self.al, self.dl);
        self.key_ground();
        let g = palette::shade(GRASS, 0.96);
        // Back verge and the two side verges, 0.14 wide inside the kerb ring.
        self.slab(0.12, al - 0.12, dl - 0.24, dl - 0.12, 1.0, g);
        self.slab(0.12, 0.24, 0.16, dl - 0.12, 1.0, g);
        self.slab(al - 0.24, al - 0.12, 0.16, dl - 0.12, 1.0, g);
        for (i, &(t, d)) in tree_spots.iter().enumerate() {
            self.tree(t, d, (self.o.seed as usize + i) as u8);
        }
    }

    // ------------------------------------------------------------------ bays

    /// Painted bays for every bank the layout has, and aisle arrows.
    fn bays(&mut self, shape: Shape) {
        self.key_ground();
        let (al, dl) = (self.al, self.dl);
        match shape {
            Shape::Park | Shape::Deck => self.park_lines(0.0, 0, true),
            Shape::Dealer => {
                // The lot is a display forecourt: two rows of stock marked with
                // short bay ticks.
                let b = cp::building(Shape::Dealer, al, dl).unwrap();
                for &d in &[0.5f32, 1.3] {
                    if d + 0.225 > b.d0 - 0.1 {
                        continue;
                    }
                    let ts = bay_ts(al, 0.35, 0.5);
                    for &t in &ts {
                        self.line((t - 0.25, d - 0.22), (t - 0.25, d + 0.22), 0.0, LINE_DIM);
                    }
                    if let Some(&t) = ts.last() {
                        self.line((t + 0.25, d - 0.22), (t + 0.25, d + 0.22), 0.0, LINE_DIM);
                    }
                }
            }
            Shape::Motel => {
                let far = self.o.front >= 2;
                let b = cp::building_for(Shape::Motel, al, dl, far).unwrap();
                let d = cp::motel_car_d(b, far);
                for &t in &cp::motel_doors(al) {
                    self.line((t - 0.31, d - 0.22), (t - 0.31, d + 0.22), 0.0, LINE);
                }
                if let Some(&t) = cp::motel_doors(al).last() {
                    self.line((t + 0.31, d - 0.22), (t + 0.31, d + 0.22), 0.0, LINE);
                }
            }
            Shape::Charge => {
                let pc = cp::charger_line(dl);
                for &t in &bay_ts(al, 0.4, 0.5) {
                    for s in [-1.0f32, 1.0] {
                        let d = pc + s * 0.3;
                        self.line((t - 0.25, d - 0.22), (t - 0.25, d + 0.22), 0.0, GREEN_LED);
                    }
                }
                if let Some(&t) = bay_ts(al, 0.4, 0.5).last() {
                    for s in [-1.0f32, 1.0] {
                        let d = pc + s * 0.3;
                        self.line((t + 0.25, d - 0.22), (t + 0.25, d + 0.22), 0.0, GREEN_LED);
                    }
                }
            }
            _ => {}
        }
    }

    /// The lines of a car park: bank edges, a line between every two bays,
    /// arrows in the aisles. `lift` is the deck height they are painted at.
    fn park_lines(&mut self, lift: f32, level: u8, arrows: bool) {
        let (al, dl) = (self.al, self.dl);
        let rows = cp::bank_rows(dl);
        let ts = bay_ts(al, 0.2, cp::BAY_PITCH);
        let step = cp::BAY_PITCH;
        let (tl, tr) = (0.2f32, al - 0.2);
        let _ = (tl, tr, step);
        let core = if self.o.kind == ObjKind::CarParkDeck { cp::building(Shape::Deck, al, dl) } else { None };
        let cut = |t: f32, d: f32| core.is_some_and(|c| t > c.t0 - 0.35 && t < c.t1 + 0.35 && d > c.d0 - 0.35 && d < c.d1 + 0.35);
        let mut edges: Vec<f32> = Vec::new();
        for &(d, _) in &rows {
            for e in [d - 0.225, d + 0.225] {
                if !edges.iter().any(|x| (x - e).abs() < 0.01) {
                    edges.push(e);
                }
            }
        }
        let first = ts.first().copied().unwrap_or(0.4) - cp::BAY_PITCH * 0.5;
        let last = ts.last().copied().unwrap_or(al - 0.4) + cp::BAY_PITCH * 0.5;
        for &e in &edges {
            // A bank's long line, broken where the stair core stands.
            let mut t = first;
            while t < last - 0.01 {
                let t2 = (t + 0.5).min(last);
                if !cut((t + t2) * 0.5, e) {
                    self.line((t, e), (t2, e), lift, LINE);
                }
                t = t2;
            }
        }
        for &(d, _) in &rows {
            for (i, &t) in ts.iter().enumerate() {
                let tt = t - cp::BAY_PITCH * 0.5;
                if cut(tt, d) {
                    continue;
                }
                self.line((tt, d - 0.225), (tt, d + 0.225), lift, LINE);
                if i == ts.len() - 1 {
                    self.line((tt + cp::BAY_PITCH, d - 0.225), (tt + cp::BAY_PITCH, d + 0.225), lift, LINE);
                }
            }
        }
        if !arrows {
            return;
        }
        // Aisle arrows: between two banks, one-way, alternating.
        let mut aisles: Vec<f32> = Vec::new();
        for w in rows.windows(2) {
            let (a, b) = (w[0].0, w[1].0);
            if b - a > 0.6 {
                aisles.push((a + b) * 0.5);
            }
        }
        for (k, &dd) in aisles.iter().enumerate() {
            for j in 0..3 {
                let t = al * (0.25 + 0.25 * j as f32);
                if cut(t, dd) {
                    continue;
                }
                let dir = if (k + level as usize) % 2 == 0 { 1.0 } else { -1.0 };
                self.arrow(t, dd, dir, lift);
            }
        }
    }

    /// A small chevron arrow along the frontage at `(t, d)`.
    fn arrow(&mut self, t: f32, d: f32, dir: f32, z: f32) {
        let h = 0.09;
        self.line((t - 0.1 * dir, d), (t + 0.12 * dir, d), z, LINE_DIM);
        self.line((t + 0.12 * dir, d), (t + 0.02 * dir, d - h), z, LINE_DIM);
        self.line((t + 0.12 * dir, d), (t + 0.02 * dir, d + h), z, LINE_DIM);
    }

    /// The blue board with its white P.
    fn p_sign(&mut self, t: f32, d: f32) {
        let (x, y) = self.w(t, d);
        self.key_at_point(x, y);
        let base = self.pt(t, d, 0.0);
        let top = self.pt(t, d, 17.0);
        self.fb.line(base, top, STEEL);
        let (bx, by) = (top.0 as i32 - 3, top.1 as i32 - 6);
        self.fb.fill_rect(bx, by, 7, 8, rgb(0x2c, 0x62, 0xc0));
        self.fb.fill_rect(bx, by, 7, 1, rgb(0x6a, 0x9c, 0xe8));
        text(self.fb, bx + 2, by + 2, "P", rgb(0xf4, 0xf4, 0xf0));
        if self.night() {
            self.fb.put_blended(bx + 3, by + 9, rgb(0x6a, 0x9c, 0xe8), 0.5);
        }
    }

    // ----------------------------------------------------------------- places

    fn park(&mut self) {
        let (al, dl) = (self.al, self.dl);
        self.verges(&[(0.16, dl - 0.18), (al - 0.16, dl - 0.18), (0.16, dl * 0.5), (al - 0.16, dl * 0.5)]);
        self.key_ground();
        self.park_lines(0.0, 0, true);
        // A disabled bay by the entrance: the blue square.
        let ts = bay_ts(al, 0.2, cp::BAY_PITCH);
        if let Some(&t) = ts.get(ts.len() / 2) {
            let d = cp::bank_rows(dl)[0].0;
            self.slab(t - 0.07, t + 0.07, d - 0.08, d + 0.08, 1.0, rgb(0x34, 0x6c, 0xc4));
        }
        // The pay-and-display booth and its barrier by the kerb.
        self.booth();
        // Lamps on the ends of the middle bank and along the edges.
        for &(t, d) in &[(0.1, dl * 0.5), (al - 0.1, dl * 0.5), (al * 0.5, dl - 0.2), (0.2, 0.3), (al - 0.2, 0.3)] {
            self.lamp(t, d);
        }
        self.p_sign(al - 0.3, 0.2);
    }

    /// A ticket booth with a striped barrier arm.
    fn booth(&mut self) {
        let al = self.al;
        let (t, d) = (al * 0.5 - 0.55, 0.14);
        self.key_rect(t - 0.14, t + 0.14, d - 0.1, d + 0.1);
        self.boxx(t - 0.14, t + 0.14, d - 0.1, d + 0.1, 1.0, 11.0, rgb(0x4a, 0x58, 0x6c), rgb(0xd8, 0xd4, 0xc4));
        let r = self.wrect(t - 0.14, t + 0.14, d - 0.1, d + 0.1);
        self.fquad(r, true, (0.2, 0.8), (5.0, 9.0), if self.night() { GLASS_LIT } else { GLASS });
        self.fquad(r, false, (0.2, 0.8), (5.0, 9.0), if self.night() { GLASS_LIT } else { GLASS });
        // The arm, red and white, across the entrance lane.
        for k in 0..6 {
            let tt = t + 0.14 + k as f32 * 0.06;
            let c = if k % 2 == 0 { rgb(0xe8, 0x4a, 0x40) } else { rgb(0xf0, 0xf0, 0xea) };
            let a = self.pt(tt, d, 8.0);
            let b = self.pt(tt + 0.06, d, 8.0);
            self.fb.line(a, b, c);
            self.fb.line((a.0, a.1 + 1.0), (b.0, b.1 + 1.0), c);
        }
        self.pole(t + 0.14 + 6.0 * 0.06, d, 1.0, 8.0, STEEL);
    }

    fn deck(&mut self) {
        let (al, dl) = (self.al, self.dl);
        // The structure's footprint: the plot inside its kerb.
        let (m0, m1, n0, n1) = (0.12f32, al - 0.12, 0.12f32, dl - 0.12);
        let r = self.wrect(m0, m1, n0, n1);
        self.key_ground();
        let tone = palette::jitter(CONC, self.o.h(0x5a), 0.05);
        let top = palette::shade(tone, 1.04);
        let dark = palette::shade(tone, 0.34);
        let slab_edge = palette::shade(tone, 0.96);
        let floors = [2.0f32, 16.0, 30.0];
        // The two faces the camera sees, bottom to top: the plinth, then two
        // covered storeys (each an open mouth with its lights, columns and
        // cars, under a slab edge), then the roof.
        for left in [true, false] {
            let shade = if left { 1.0 } else { 0.84 };
            self.fquad(r, left, (0.0, 1.0), (0.0, floors[0]), palette::shade(slab_edge, shade * 0.9));
            for s in 0..2usize {
                let open_bot = floors[s];
                let open_top = floors[s + 1] - 3.0;
                self.fquad(r, left, (0.0, 1.0), (open_bot, open_top), palette::shade(dark, shade));
                // The ceiling's lights, a thin line that glows at night.
                let lamp = if self.night() { em(WARM) } else { palette::shade(tone, 0.6) };
                for j in 0..6 {
                    let u0 = 0.06 + j as f32 * 0.16;
                    let a = self.fpt(r, left, u0, open_top - 1.0);
                    let b = self.fpt(r, left, u0 + 0.08, open_top - 1.0);
                    self.fb.line(a, b, lamp);
                }
                // Cars inside, painted: sprites stand on the roof deck only.
                let fill = self.deck_fill(s);
                for j in 0..9 {
                    let h = crate::tex::hash2(j * 7 + s as i32 * 13, left as i32 + 1, self.o.seed);
                    if h > fill {
                        continue;
                    }
                    let u = 0.07 + j as f32 * 0.105;
                    let col = PAINT[(h * 97.0) as usize % PAINT.len()];
                    let a = self.fpt(r, left, u, open_bot + 1.0);
                    let b = self.fpt(r, left, u + 0.065, open_bot + 1.0);
                    let (x0, y0) = (a.0.min(b.0) as i32, a.1.min(b.1) as i32);
                    let wdt = ((a.0 - b.0).abs() as i32).max(3);
                    self.fb.fill_rect(x0, y0 - 3, wdt, 3, palette::shade(col, if left { 1.0 } else { 0.8 }));
                    self.fb.fill_rect(x0 + 1, y0 - 4, (wdt - 2).max(1), 1, palette::shade(col, 1.15));
                }
                for j in 0..=4 {
                    self.fline(r, left, j as f32 * 0.25, (open_bot, open_top), palette::shade(tone, 0.82 * shade));
                }
                // The slab edge above the opening.
                self.fquad(r, left, (0.0, 1.0), (open_top, floors[s + 1]), palette::shade(slab_edge, shade));
                self.fquad(r, left, (0.0, 1.0), (floors[s + 1] - 1.0, floors[s + 1]), palette::shade(slab_edge, shade * 1.08));
            }
        }
        // The roof deck: tarmac, painted bays, a parapet round it.
        let z = floors[2].round() as i32;
        let p = self.p;
        let roof = [p(r.0, r.1, z), p(r.2, r.1, z), p(r.2, r.3, z), p(r.0, r.3, z)];
        let base = palette::shade(ASPHALT, 1.08);
        self.fb.fill_poly(&roof, base);
        self.park_lines(floors[2], 2, true);
        // Parapet: a wall 3 px tall on the four edges, the near two with faces.
        let pz = z + 3;
        for left in [true, false] {
            let shade = if left { 1.0 } else { 0.84 };
            self.fquad(r, left, (0.0, 1.0), (floors[2], floors[2] + 3.0), palette::shade(tone, shade * 0.98));
            self.fquad(r, left, (0.0, 1.0), (floors[2] + 2.0, floors[2] + 3.0), palette::shade(tone, shade * 1.1));
        }
        let back = [(r.0, r.1), (r.2, r.1), (r.0, r.3)];
        let _ = (pz, back);
        for &(a, b) in &[((r.0, r.1), (r.2, r.1)), ((r.0, r.1), (r.0, r.3))] {
            let pa = p(a.0, a.1, z + 3);
            let pb = p(b.0, b.1, z + 3);
            self.fb.line(pa, pb, palette::shade(tone, 1.1));
        }
        // The stair and lift core: a tower at the back corner.
        let core = cp::building(Shape::Deck, al, dl).unwrap();
        self.key_rect(core.t0, core.t1, core.d0, core.d1);
        self.boxx(core.t0, core.t1, core.d0, core.d1, floors[2], floors[2] + 14.0, palette::shade(tone, 0.9), palette::shade(tone, 0.94));
        let cr = self.wrect(core.t0, core.t1, core.d0, core.d1);
        for left in [true, false] {
            self.fquad(cr, left, (0.2, 0.8), (floors[2] + 3.0, floors[2] + 11.0), if self.night() { GLASS_LIT } else { palette::shade(GLASS, if left { 1.0 } else { 0.8 }) });
            self.fline(cr, left, 0.5, (floors[2] + 3.0, floors[2] + 11.0), palette::shade(tone, 0.8));
        }
        // The P board on the core's roof, and a flag.
        let cc = ((core.t0 + core.t1) * 0.5, (core.d0 + core.d1) * 0.5);
        let q = self.pt(cc.0, cc.1, floors[2] + 14.0);
        let (bx, by) = (q.0 as i32 - 3, q.1 as i32 - 9);
        self.fb.line(q, (q.0, q.1 - 3.0), STEEL);
        self.fb.fill_rect(bx, by, 7, 8, rgb(0x2c, 0x62, 0xc0));
        text(self.fb, bx + 2, by + 2, "P", rgb(0xf4, 0xf4, 0xf0));
        // Lamps on the roof deck.
        for &(t, d) in &[(0.25, dl * 0.5), (al - 0.25, dl * 0.5)] {
            self.lamp_at(t, d, floors[2]);
        }
        self.p_sign(al - 0.25, 0.1);
    }

    /// How full a covered deck looks at this hour, 0..1: the working day.
    fn deck_fill(&self, level: usize) -> f32 {
        let h = self.hour;
        let day = if (7.0..9.5).contains(&h) {
            0.55 + (h - 7.0) * 0.18
        } else if (9.5..17.5).contains(&h) {
            0.95
        } else if (17.5..21.0).contains(&h) {
            0.95 - (h - 17.5) * 0.2
        } else {
            0.18
        };
        (day - level as f32 * 0.08).clamp(0.05, 0.95)
    }

    fn lamp_at(&mut self, t: f32, d: f32, z: f32) {
        let (x, y) = self.w(t, d);
        self.key_at_point(x, y);
        let zi = z.round() as i32;
        let top = (self.p)(x, y, zi + 16);
        let base = (self.p)(x, y, zi);
        self.fb.line(top, base, STEEL);
        self.fb.put(top.0 as i32, top.1 as i32, WARM);
        self.fb.put(top.0 as i32 + 1, top.1 as i32, WARM);
        if self.night() {
            self.fb.put_blended(top.0 as i32, top.1 as i32 + 1, WARM, 0.55);
            self.fb.put_blended(top.0 as i32 + 1, top.1 as i32 + 1, WARM, 0.45);
        }
    }

    // ------------------------------------------------------------------ gas

    fn gas(&mut self) {
        let (al, dl) = (self.al, self.dl);
        self.verges(&[(0.17, dl - 0.18), (al - 0.17, dl - 0.18), (al - 0.17, dl * 0.5)]);
        self.key_ground();
        let (i0, i1, dc) = cp::island(al);
        // The painted pump lanes beside the island and the island itself.
        for s in [-1.0f32, 1.0] {
            self.line((i0 - 0.2, dc + s * 0.62), (i1 + 0.2, dc + s * 0.62), 0.0, LINE_DIM);
        }
        // The island: a raised kerb, two bollards at the ends.
        self.slab(i0, i1, dc - 0.15, dc + 0.15, 3.0, rgb(0xc4, 0xc0, 0xb4));
        self.key_rect(i0, i1, dc - 0.15, dc + 0.15);
        self.boxx(i0, i1, dc - 0.15, dc + 0.15, 0.0, 3.0, rgb(0xc4, 0xc0, 0xb4), rgb(0xa0, 0x9c, 0x92));
        for &t in &[i0 + 0.04, i1 - 0.04] {
            self.boxx(t - 0.04, t + 0.04, dc - 0.04, dc + 0.04, 3.0, 8.0, YELLOW, palette::shade(YELLOW, 0.8));
        }
        // The pumps: a white housing with a red head and a dark display.
        let mid = (i0 + i1) * 0.5;
        for k in [-1.0f32, 1.0] {
            let t = mid + k * 0.34;
            self.key_rect(t - 0.1, t + 0.1, dc - 0.08, dc + 0.08);
            self.boxx(t - 0.1, t + 0.1, dc - 0.08, dc + 0.08, 3.0, 13.0, rgb(0xe8, 0xe8, 0xe2), POST_WHITE);
            let r = self.wrect(t - 0.1, t + 0.1, dc - 0.08, dc + 0.08);
            self.fquad(r, true, (0.15, 0.85), (9.0, 12.0), FUEL_FASCIA);
            self.fquad(r, true, (0.3, 0.7), (6.0, 9.0), BOARD);
            self.fquad(r, false, (0.15, 0.85), (9.0, 12.0), palette::shade(FUEL_FASCIA, 0.8));
            self.fquad(r, false, (0.3, 0.7), (6.0, 9.0), palette::shade(BOARD, 1.2));
            if self.night() {
                self.fquad(r, true, (0.3, 0.7), (7.0, 9.0), em(GREEN_LED));
            }
        }
        // The kiosk at the back.
        let k = cp::building(Shape::Gas, al, dl).unwrap();
        self.shop_box(k, 12.0, KIOSK_WALL, KIOSK_ROOF, true);
        // The canopy over the pumps: columns, a slab and a fascia band.
        let c = cp::canopy(Shape::Gas, al, dl).unwrap();
        self.canopy(c, 18.0, CANOPY_TOP, true);
        // The price pylon at the kerb corner.
        self.price_pylon(0.22, 0.22);
    }

    /// A small shop with glazing on both camera faces and a lit sign line.
    fn shop_box(&mut self, r: Rect, h: f32, wall: Color, roof: Color, sign: bool) {
        self.key_rect(r.t0, r.t1, r.d0, r.d1);
        self.boxx(r.t0, r.t1, r.d0, r.d1, 0.0, h, roof, wall);
        let wr = self.wrect(r.t0, r.t1, r.d0, r.d1);
        for left in [true, false] {
            let shade = if left { 1.0 } else { 0.8 };
            let lit = self.night();
            let glass = if lit { GLASS_LIT } else { palette::shade(GLASS, shade) };
            self.fquad(wr, left, (0.1, 0.9), (2.5, h - 3.5), glass);
            for j in 1..4 {
                self.fline(wr, left, 0.1 + 0.8 * j as f32 / 4.0, (2.5, h - 3.5), palette::shade(wall, 0.8 * shade));
            }
            // The roof's fascia band, in the trade's colour.
            self.fquad(wr, left, (0.0, 1.0), (h - 2.0, h), palette::shade(roof, 0.9 * shade));
        }
        if sign && self.night() {
            let q = self.pt((r.t0 + r.t1) * 0.5, (r.d0 + r.d1) * 0.5, h + 1.0);
            self.fb.put_blended(q.0 as i32, q.1 as i32, WARM, 0.9);
        }
    }

    /// A flat canopy on columns: slab, thick fascia round the edge, lights
    /// that glow at night. Columns stand at the corners of `r` and inside it.
    fn canopy(&mut self, r: Rect, z: f32, top: Color, columns_mid: bool) {
        // The columns first (they are behind and under the slab).
        let cols: Vec<(f32, f32)> = if columns_mid {
            let (i0, i1, dc) = cp::island(self.al);
            vec![(i0 + 0.12, dc), (i1 - 0.12, dc)]
        } else {
            vec![(r.t0 + 0.12, r.d0 + 0.12), (r.t1 - 0.12, r.d0 + 0.12), (r.t0 + 0.12, r.d1 - 0.12), (r.t1 - 0.12, r.d1 - 0.12)]
        };
        for &(t, d) in &cols {
            let (x, y) = self.w(t, d);
            self.key_at_point(x, y);
            let a = self.pt(t, d, z);
            let b = self.pt(t, d, 3.0);
            self.fb.fill_rect(a.0 as i32 - 1, a.1 as i32, 3, (b.1 - a.1) as i32 + 1, STEEL);
            self.fb.line((a.0 + 1.0, a.1), (b.0 + 1.0, b.1), palette::shade(STEEL, 0.7));
        }
        // The slab: top, then the fascia faces on the two near sides.
        self.key_rect(r.t0, r.t1, r.d0, r.d1);
        let wr = self.wrect(r.t0, r.t1, r.d0, r.d1);
        let p = self.p;
        let zi = z.round() as i32;
        // The underside shows nothing; the fascia is a 3 px band.
        for left in [true, false] {
            let shade = if left { 1.0 } else { 0.8 };
            self.fquad(wr, left, (0.0, 1.0), (z - 4.0, z), palette::shade(FUEL_FASCIA, shade));
            self.fquad(wr, left, (0.0, 1.0), (z - 3.0, z - 2.0), palette::shade(FUEL_STRIPE, shade));
        }
        // The roof is a frosted sheet: the island, the pumps and the cars under
        // it stay readable through it (and are drawn over it), the rim and the
        // fascia are solid.
        self.key_ground();
        let pts = [p(wr.0, wr.1, zi), p(wr.2, wr.1, zi), p(wr.2, wr.3, zi), p(wr.0, wr.3, zi)];
        self.fb.fill_poly_blended(&pts, top, 0.58);
        let rim = palette::shade(top, 1.02);
        for (a, b) in [
            ((wr.0, wr.1), (wr.2, wr.1)),
            ((wr.2, wr.1), (wr.2, wr.3)),
            ((wr.2, wr.3), (wr.0, wr.3)),
            ((wr.0, wr.3), (wr.0, wr.1)),
        ] {
            let (pa, pb) = (p(a.0, a.1, zi), p(b.0, b.1, zi));
            self.fb.line(pa, pb, rim);
            self.fb.line((pa.0, pa.1 + 1.0), (pb.0, pb.1 + 1.0), rim);
        }
        // A ridge of two skylight strips along the long way.
        let (cx, cy) = ((wr.0 + wr.2) * 0.5, (wr.1 + wr.3) * 0.5);
        let (ex, ey) = ((wr.2 - wr.0) * 0.5, (wr.3 - wr.1) * 0.5);
        let along_x = ex >= ey;
        let (ax, ay) = if along_x { (ex - 0.1, 0.0) } else { (0.0, ey - 0.1) };
        self.fb.line(p(cx - ax, cy - ay, zi), p(cx + ax, cy + ay, zi), palette::shade(top, 0.9));
        self.key_rect(r.t0, r.t1, r.d0, r.d1);
        if self.night() {
            // The lit underside spills onto the forecourt.
            self.key_ground();
            self.slab_blend(r.t0 + 0.05, r.t1 - 0.05, r.d0 + 0.05, r.d1 - 0.05, 1.0, WARM, 0.22);
            self.key_rect(r.t0, r.t1, r.d0, r.d1);
            for j in 0..6 {
                let t = r.t0 + (r.t1 - r.t0) * (j as f32 + 0.5) / 6.0;
                self.glow_dot(t, r.d1 - 0.03, z - 3.0, WARM);
            }
        }
    }

    /// The price pylon: a pole and a board of lit prices.
    fn price_pylon(&mut self, t: f32, d: f32) {
        let (x, y) = self.w(t, d);
        self.key_at_point(x, y);
        let base = self.pt(t, d, 0.0);
        let top = self.pt(t, d, 20.0);
        self.fb.line(base, top, STEEL);
        self.fb.line((base.0 + 1.0, base.1), (top.0 + 1.0, top.1), palette::shade(STEEL, 0.7));
        let (bx, by) = (top.0 as i32 - 7, top.1 as i32 - 18);
        self.fb.fill_rect(bx - 1, by - 1, 15, 20, rgb(0x2c, 0x30, 0x3a));
        self.fb.fill_rect(bx, by, 13, 4, FUEL_FASCIA);
        self.fb.fill_rect(bx, by + 4, 13, 14, BOARD);
        let ink1 = if self.night() { em(GREEN_LED) } else { rgb(0x58, 0xb4, 0x70) };
        let ink2 = if self.night() { em(AMBER_LED) } else { rgb(0xc8, 0x98, 0x38) };
        text(self.fb, bx + 1, by + 5, "189", ink1);
        text(self.fb, bx + 1, by + 12, "205", ink2);
        self.fb.put(bx + 4, by + 10, ink1);
        self.fb.put(bx + 4, by + 17, ink2);
    }

    // --------------------------------------------------------------- charging

    fn charge(&mut self) {
        let (al, dl) = (self.al, self.dl);
        self.verges(&[(0.17, dl - 0.18), (al - 0.17, dl - 0.18), (al - 0.17, dl * 0.5)]);
        self.key_ground();
        self.bays(Shape::Charge);
        let pc = cp::charger_line(dl);
        // The charger posts down the middle of the canopy: a white pillar,
        // a green light, a cable.
        for &t in &bay_ts(al, 0.4, 0.5) {
            let (x, y) = self.w(t - 0.25, pc);
            self.key_at_point(x, y);
            let t0 = t - 0.25;
            self.boxx(t0 - 0.05, t0 + 0.05, pc - 0.05, pc + 0.05, 0.0, 9.0, POST_WHITE, rgb(0xd4, 0xd6, 0xd0));
            let r = self.wrect(t0 - 0.05, t0 + 0.05, pc - 0.05, pc + 0.05);
            self.fquad(r, true, (0.2, 0.8), (5.0, 8.0), if self.night() { em(GREEN_LED) } else { palette::shade(GREEN_LED, 0.75) });
            self.fquad(r, false, (0.2, 0.8), (5.0, 8.0), palette::shade(GREEN_LED, 0.6));
        }
        // The hut.
        let hut = cp::building(Shape::Charge, al, dl).unwrap();
        self.shop_box(hut, 10.0, rgb(0xd6, 0xd8, 0xd4), rgb(0x3a, 0x4a, 0x62), false);
        // The solar canopy.
        let c = cp::canopy(Shape::Charge, al, dl).unwrap();
        self.solar_canopy(c, 17.0);
        // A lightning-bolt pole sign at the kerb corner.
        self.bolt_sign(al - 0.2, 0.2);
    }

    fn solar_canopy(&mut self, r: Rect, z: f32) {
        // Columns at the four corners and the middle of the long sides.
        for &(t, d) in &[(r.t0 + 0.1, r.d0 + 0.1), (r.t1 - 0.1, r.d0 + 0.1), (r.t0 + 0.1, r.d1 - 0.1), (r.t1 - 0.1, r.d1 - 0.1), ((r.t0 + r.t1) * 0.5, r.d1 - 0.1)] {
            let (x, y) = self.w(t, d);
            self.key_at_point(x, y);
            let a = self.pt(t, d, z);
            let b = self.pt(t, d, 0.0);
            self.fb.fill_rect(a.0 as i32 - 1, a.1 as i32, 3, (b.1 - a.1) as i32 + 1, STEEL);
        }
        self.key_rect(r.t0, r.t1, r.d0, r.d1);
        let wr = self.wrect(r.t0, r.t1, r.d0, r.d1);
        let zi = z.round() as i32;
        let p = self.p;
        for left in [true, false] {
            let shade = if left { 1.0 } else { 0.8 };
            self.fquad(wr, left, (0.0, 1.0), (z - 3.0, z), palette::shade(rgb(0xc8, 0xcc, 0xcc), shade));
            self.fquad(wr, left, (0.0, 1.0), (z - 1.0, z), palette::shade(rgb(0x7c, 0xc8, 0x98), shade));
        }
        // Panels in rows with gaps between them: the chargers and the cars
        // under them show through the gaps and are drawn over the roof.
        self.key_ground();
        let rows = 5;
        let along_x = (wr.2 - wr.0) >= (wr.3 - wr.1);
        for k in 0..rows {
            let f0 = (k as f32 + 0.10) / rows as f32;
            let f1 = (k as f32 + 0.62) / rows as f32;
            let (a0, a1, b0, b1) = if along_x {
                (wr.0, wr.2, wr.1 + (wr.3 - wr.1) * f0, wr.1 + (wr.3 - wr.1) * f1)
            } else {
                (wr.0 + (wr.2 - wr.0) * f0, wr.0 + (wr.2 - wr.0) * f1, wr.1, wr.3)
            };
            let q = [p(a0, b0, zi), p(a1, b0, zi), p(a1, b1, zi), p(a0, b1, zi)];
            self.fb.fill_poly(&q, SOLAR);
            self.fb.line(p(a0, b0, zi), p(a1, b0, zi), palette::shade(SOLAR_CELL, 1.25));
            // Cell joints.
            for c in 1..6 {
                let f = c as f32 / 6.0;
                let (x, y) = (a0 + (a1 - a0) * f, b0 + (b1 - b0) * f);
                if along_x {
                    self.fb.line(p(x, b0, zi), p(x, b1, zi), SOLAR_CELL);
                } else {
                    self.fb.line(p(a0, y, zi), p(a1, y, zi), SOLAR_CELL);
                }
            }
        }
        self.key_rect(r.t0, r.t1, r.d0, r.d1);
        if self.night() {
            self.key_ground();
            self.slab_blend(r.t0 + 0.05, r.t1 - 0.05, r.d0 + 0.05, r.d1 - 0.05, 1.0, GREEN_LED, 0.10);
        }
    }

    fn bolt_sign(&mut self, t: f32, d: f32) {
        let (x, y) = self.w(t, d);
        self.key_at_point(x, y);
        let base = self.pt(t, d, 0.0);
        let top = self.pt(t, d, 18.0);
        self.fb.line(base, top, STEEL);
        let (bx, by) = (top.0 as i32 - 4, top.1 as i32 - 10);
        self.fb.fill_rect(bx, by, 9, 11, rgb(0x20, 0x48, 0x38));
        self.fb.fill_rect(bx, by, 9, 1, GREEN_LED);
        let ink = if self.night() { em(rgb(0xff, 0xf0, 0x70)) } else { rgb(0xe8, 0xd8, 0x58) };
        for &(dx, dy) in &[(5, 2), (4, 3), (3, 4), (4, 5), (3, 5), (5, 5), (4, 6), (3, 7), (3, 8)] {
            self.fb.put(bx + dx, by + dy, ink);
        }
        for &(dx, dy) in &[(4, 4), (5, 4), (4, 6)] {
            self.fb.put(bx + dx, by + dy, ink);
        }
    }

    // ------------------------------------------------------------- drive-through

    fn drive(&mut self) {
        let (al, dl) = (self.al, self.dl);
        self.verges(&[(0.17, dl - 0.18), (al - 0.17, dl - 0.18)]);
        self.key_ground();
        let (pts, s_win) = cp::drive_lane(al, dl);
        // The lane: a lighter band of tarmac with white edge lines, arrows and
        // the order point.
        let half = 0.21;
        let lane_c = palette::shade(ASPHALT, 1.14);
        for w in pts.windows(2) {
            let (a, b) = (w[0], w[1]);
            let (t0, t1) = (a.0.min(b.0) - half, a.0.max(b.0) + half);
            let (d0, d1) = (a.1.min(b.1), a.1.max(b.1));
            if (a.0 - b.0).abs() < 1e-4 {
                self.slab(a.0 - half, a.0 + half, d0 + 0.0, d1 + half, 1.0, lane_c);
            } else {
                self.slab(t0, t1, a.1 - half, a.1 + half, 1.0, lane_c);
            }
        }
        // Edge lines: outer and inner, dashed on the inner.
        for w in pts.windows(2) {
            let (a, b) = (w[0], w[1]);
            if (a.0 - b.0).abs() < 1e-4 {
                let (d0, d1) = (a.1.min(b.1), a.1.max(b.1) + half);
                self.line((a.0 - half, d0), (a.0 - half, d1), 0.0, LINE_DIM);
                self.line((a.0 + half, d0), (a.0 + half, d1), 0.0, LINE_DIM);
            } else {
                let (t0, t1) = (a.0.min(b.0) - half, a.0.max(b.0) + half);
                self.line((t0, a.1 - half), (t1, a.1 - half), 0.0, LINE_DIM);
                self.line((t0, a.1 + half), (t1, a.1 + half), 0.0, LINE_DIM);
            }
        }
        // Arrows along the lane.
        for k in 0..6 {
            let s = 0.6 + k as f32 * 0.95;
            if s > s_win + 0.6 {
                break;
            }
            let ((t, d), nose) = cp::along_lane(&pts, s);
            self.lane_arrow(t, d, nose);
        }
        // The building, inside the loop.
        let b = cp::building(Shape::Drive, al, dl).unwrap();
        self.key_rect(b.t0, b.t1, b.d0, b.d1);
        self.boxx(b.t0, b.t1, b.d0, b.d1, 0.0, 10.0, EATERY_ROOF, EATERY_WALL);
        let wr = self.wrect(b.t0, b.t1, b.d0, b.d1);
        for left in [true, false] {
            let shade = if left { 1.0 } else { 0.8 };
            // A band of windows and a red roof fascia.
            let glass = if self.night() { GLASS_LIT } else { palette::shade(GLASS, shade) };
            self.fquad(wr, left, (0.08, 0.92), (2.5, 6.5), glass);
            for j in 1..5 {
                self.fline(wr, left, 0.08 + 0.84 * j as f32 / 5.0, (2.5, 6.5), palette::shade(EATERY_WALL, 0.78 * shade));
            }
            self.fquad(wr, left, (0.0, 1.0), (7.0, 10.0), palette::shade(EATERY_ROOF, shade));
            self.fquad(wr, left, (0.0, 1.0), (8.0, 9.0), palette::shade(FUEL_STRIPE, shade));
        }
        // A parapet ring and two rooftop units, so the roof is not a red field.
        {
            let p = self.p;
            let zi = 10;
            let ring = palette::shade(EATERY_ROOF, 1.18);
            let (ix0, iy0, ix1, iy1) = (wr.0 + 0.07, wr.1 + 0.07, wr.2 - 0.07, wr.3 - 0.07);
            for (a, c) in [((ix0, iy0), (ix1, iy0)), ((ix1, iy0), (ix1, iy1)), ((ix1, iy1), (ix0, iy1)), ((ix0, iy1), (ix0, iy0))] {
                self.fb.line(p(a.0, a.1, zi), p(c.0, c.1, zi), ring);
            }
            let (cx, cy) = ((wr.0 + wr.2) * 0.5, (wr.1 + wr.3) * 0.5);
            for &(ox, oy) in &[(-0.28f32, -0.2f32), (0.25, 0.18)] {
                let (ux, uy) = (cx + ox, cy + oy);
                let (x0, y0, x1, y1) = (ux - 0.1, uy - 0.09, ux + 0.1, uy + 0.09);
                self.fb.fill_poly(&[p(x0, y1, 14), p(x1, y1, 14), p(x1, y1, 10), p(x0, y1, 10)], rgb(0xa8, 0xa8, 0xa4));
                self.fb.fill_poly(&[p(x1, y0, 14), p(x1, y1, 14), p(x1, y1, 10), p(x1, y0, 10)], rgb(0x86, 0x86, 0x84));
                self.fb.fill_poly(&[p(x0, y0, 14), p(x1, y0, 14), p(x1, y1, 14), p(x0, y1, 14)], rgb(0xc4, 0xc4, 0xc0));
            }
        }
        // The pickup window's awning over the lane, at the queue's head.
        let ((pt_, pd), _) = cp::along_lane(&pts, s_win);
        let wt = b.t1;
        self.key_rect(wt, wt + 0.3, pd - 0.2, pd + 0.2);
        self.boxx(wt, wt + 0.3, pd - 0.2, pd + 0.2, 7.0, 9.0, rgb(0xd4, 0x58, 0x4a), rgb(0xf0, 0xe8, 0xd0));
        self.pole(wt + 0.28, pd - 0.18, 0.0, 7.0, STEEL);
        self.pole(wt + 0.28, pd + 0.18, 0.0, 7.0, STEEL);
        let _ = pt_;
        // The menu board at the start of the loop, facing the order lane.
        let mb = (0.58, 1.3);
        self.key_rect(mb.0 - 0.08, mb.0 + 0.08, mb.1 - 0.15, mb.1 + 0.15);
        self.boxx(mb.0 - 0.07, mb.0 + 0.07, mb.1 - 0.17, mb.1 + 0.17, 0.0, 10.0, BOARD, rgb(0x34, 0x3a, 0x48));
        let mr = self.wrect(mb.0 - 0.07, mb.0 + 0.07, mb.1 - 0.17, mb.1 + 0.17);
        for left in [true, false] {
            let shade = if left { 1.0 } else { 0.8 };
            self.fquad(mr, left, (0.12, 0.88), (3.0, 9.0), if self.night() { em(WARM) } else { palette::shade(rgb(0xe8, 0xc8, 0x6c), shade) });
        }
        // The pole sign: a round board on a tall pole at the kerb.
        self.drive_sign(al - 0.12, 0.12);
        // A lamp over the order lane.
        self.lamp(0.1, dl * 0.5);
        self.lamp(al - 0.1, dl * 0.5);
    }

    fn lane_arrow(&mut self, t: f32, d: f32, nose: Nose) {
        let (dt, dd) = match nose {
            Nose::Fwd => (1.0, 0.0),
            Nose::Back => (-1.0, 0.0),
            Nose::In => (0.0, 1.0),
            Nose::Out => (0.0, -1.0),
        };
        let (px, py) = (-dd, dt);
        let tip = (t + dt * 0.14, d + dd * 0.14);
        let tail = (t - dt * 0.12, d - dd * 0.12);
        self.line(tail, tip, 0.0, LINE_DIM);
        self.line(tip, (t + px * 0.08 - dt * 0.0, d + py * 0.08 - dd * 0.0), 0.0, LINE_DIM);
        self.line(tip, (t - px * 0.08, d - py * 0.08), 0.0, LINE_DIM);
    }

    fn drive_sign(&mut self, t: f32, d: f32) {
        let (x, y) = self.w(t, d);
        self.key_at_point(x, y);
        let base = self.pt(t, d, 0.0);
        let top = self.pt(t, d, 24.0);
        self.fb.line(base, top, STEEL);
        let (cx, cy) = (top.0 as i32, top.1 as i32 - 4);
        for dy in -5..=5i32 {
            for dx in -6..=6i32 {
                let r = (dx * dx) as f32 / 36.0 + (dy * dy) as f32 / 25.0;
                if r <= 1.0 {
                    let c = if r > 0.72 { rgb(0xf0, 0xe0, 0xa0) } else { EATERY_ROOF };
                    self.fb.put(cx + dx, cy + dy, c);
                }
            }
        }
        // A burger in the disc: bun, patty, leaf, bun.
        let bun = rgb(0xec, 0xb4, 0x5c);
        for (dy, half, col) in [(-3, 2, bun), (-2, 3, bun), (-1, 3, bun), (0, 3, rgb(0x5a, 0x34, 0x20)), (1, 3, rgb(0x6c, 0xb8, 0x4c)), (2, 3, bun), (3, 2, palette::shade(bun, 0.85))] {
            self.fb.fill_rect(cx - half, cy + dy, half * 2 + 1, 1, col);
        }
        if self.night() {
            self.fb.put_blended(cx, cy + 6, WARM, 0.6);
        }
    }

    // ------------------------------------------------------------------ wash

    fn wash(&mut self) {
        let (al, dl) = (self.al, self.dl);
        self.verges(&[(0.17, dl - 0.18), (al - 0.17, dl - 0.18)]);
        self.key_ground();
        let b = cp::building(Shape::Wash, al, dl).unwrap();
        let dc = (b.d0 + b.d1) * 0.5;
        // The lane through the tunnel, painted, with arrows pointing in.
        self.slab(0.14, al - 0.14, dc - 0.2, dc + 0.2, 1.0, palette::shade(ASPHALT, 1.12));
        self.line((0.14, dc - 0.2), (al - 0.14, dc - 0.2), 0.0, LINE_DIM);
        self.line((0.14, dc + 0.2), (al - 0.14, dc + 0.2), 0.0, LINE_DIM);
        self.lane_arrow((b.t0 - 0.2).max(0.3), dc, Nose::Fwd);
        // The tunnel: a long blue shell.
        self.key_rect(b.t0, b.t1, b.d0, b.d1);
        self.boxx(b.t0, b.t1, b.d0, b.d1, 0.0, 13.0, WASH_ROOF, WASH_WALL);
        let wr = self.wrect(b.t0, b.t1, b.d0, b.d1);
        let p = self.p;
        for left in [true, false] {
            let shade = if left { 1.0 } else { 0.8 };
            // A white stripe and a viewing window along the side.
            self.fquad(wr, left, (0.0, 1.0), (9.0, 11.0), palette::shade(rgb(0xe8, 0xf0, 0xf4), shade));
            let glass = if self.night() { GLASS_LIT } else { palette::shade(rgb(0x42, 0x6c, 0x98), shade) };
            self.fquad(wr, left, (0.25, 0.75), (3.0, 8.0), glass);
        }
        // The roof's edge band and a ridge of white.
        let zi = 13;
        let (cx, cy) = ((wr.0 + wr.2) * 0.5, (wr.1 + wr.3) * 0.5);
        let ridge = if b.t1 - b.t0 > b.d1 - b.d0 {
            // Runs along t.
            let a = self.w(b.t0 + 0.1, dc);
            let c = self.w(b.t1 - 0.1, dc);
            (p(a.0, a.1, zi), p(c.0, c.1, zi))
        } else {
            (p(cx, cy, zi), p(cx, cy, zi))
        };
        self.fb.line(ridge.0, ridge.1, rgb(0xe8, 0xf0, 0xf4));
        self.mouth(b, 13.0);
        // The gantry over the entrance lane and the pay terminal.
        let gt = (b.t1 + 0.16).min(al - 0.5);
        self.key_rect(gt - 0.04, gt + 0.04, b.d0 - 0.02, b.d1 + 0.02);
        self.pole(gt, dc - 0.28, 0.0, 14.0, rgb(0x34, 0x64, 0x98));
        self.pole(gt, dc + 0.28, 0.0, 14.0, rgb(0x34, 0x64, 0x98));
        let a = self.pt(gt, dc - 0.28, 14.0);
        let c = self.pt(gt, dc + 0.28, 14.0);
        self.fb.line(a, c, rgb(0x34, 0x64, 0x98));
        self.fb.line((a.0, a.1 + 1.0), (c.0, c.1 + 1.0), rgb(0x34, 0x64, 0x98));
        let g = if self.night() { em(rgb(0xff, 0xf0, 0x90)) } else { rgb(0x9c, 0xd8, 0xf0) };
        self.fb.put(((a.0 + c.0) * 0.5) as i32, ((a.1 + c.1) * 0.5) as i32 + 2, g);
        // The sign.
        let (x, y) = self.w(0.25, 0.2);
        self.key_at_point(x, y);
        let q = self.pt(0.25, 0.2, 18.0);
        self.fb.line(self.pt(0.25, 0.2, 0.0), q, STEEL);
        let (bx, by) = (q.0 as i32 - 9, q.1 as i32 - 8);
        self.fb.fill_rect(bx, by, 19, 8, rgb(0x2a, 0x58, 0x90));
        self.fb.fill_rect(bx, by, 19, 1, rgb(0x7c, 0xc0, 0xe8));
        text(self.fb, bx + 1, by + 2, "WASH", if self.night() { em(rgb(0xff, 0xff, 0xff)) } else { rgb(0xe4, 0xf0, 0xf8) });
        // Vacuum posts beside the exit.
        for &t in &[b.t1 - 0.1, b.t1 + 0.1] {
            let d = b.d0 - 0.2;
            let (x, y) = self.w(t, d);
            self.key_at_point(x, y);
            self.boxx(t - 0.04, t + 0.04, d - 0.04, d + 0.04, 0.0, 9.0, rgb(0xdc, 0xc8, 0x3c), rgb(0xe8, 0xe0, 0xc0));
        }
    }

    /// The tunnel's mouth, on the end face the camera sees (the `t1` end: `t`
    /// runs along +x or +y, so that end is the +x or the +y face): a dark
    /// opening with the brush rollers, spray and the curtain.
    fn mouth(&mut self, b: Rect, h: f32) {
        let (along, _) = self.f.axes();
        let left = along.0 == 0.0;
        let r = self.wrect(b.t0, b.t1, b.d0, b.d1);
        self.fquad(r, left, (0.0, 1.0), (0.0, h), palette::shade(WASH_WALL, 0.9));
        self.fquad(r, left, (0.14, 0.86), (1.0, h - 3.0), WASH_MOUTH);
        for j in 0..4 {
            let u = 0.24 + 0.17 * j as f32;
            self.fquad(r, left, (u - 0.035, u + 0.035), (2.0, h - 4.0), rgb(0xe8, 0x7c, 0xb0));
        }
        // Spray: a few pale pixels in the opening.
        for j in 0..5 {
            let a = self.fpt(r, left, 0.2 + 0.15 * j as f32, 3.0 + (j % 3) as f32 * 2.0);
            self.fb.put(a.0 as i32, a.1 as i32, rgb(0xd8, 0xf0, 0xf8));
        }
        self.fquad(r, left, (0.0, 1.0), (h - 3.0, h - 1.0), rgb(0xe8, 0xf0, 0xf4));
        self.fquad(r, left, (0.0, 1.0), (h - 2.0, h - 1.0), palette::shade(WASH_ROOF, 0.9));
    }

    // ----------------------------------------------------------------- dealer

    fn dealer(&mut self) {
        let (al, dl) = (self.al, self.dl);
        self.verges(&[(0.17, dl - 0.18), (al - 0.17, dl - 0.18)]);
        self.key_ground();
        self.bays(Shape::Dealer);
        // The showroom: glass walls on a low plinth, a dark roof, a bright fascia.
        let b = cp::building(Shape::Dealer, al, dl).unwrap();
        self.key_rect(b.t0, b.t1, b.d0, b.d1);
        self.boxx(b.t0, b.t1, b.d0, b.d1, 0.0, 13.0, SHOW_ROOF, rgb(0xc8, 0xd2, 0xd8));
        let wr = self.wrect(b.t0, b.t1, b.d0, b.d1);
        for left in [true, false] {
            let shade = if left { 1.0 } else { 0.8 };
            let glass = if self.night() { GLASS_LIT } else { palette::shade(rgb(0x6c, 0x98, 0xb8), shade) };
            self.fquad(wr, left, (0.02, 0.98), (2.0, 10.0), glass);
            // The cars on show behind the glass.
            for j in 0..5 {
                let u = 0.12 + 0.18 * j as f32;
                let col = PAINT[(self.o.seed as usize + j * 3 + left as usize) % PAINT.len()];
                self.fquad(wr, left, (u, u + 0.1), (2.0, 5.0), palette::shade(col, shade));
                self.fquad(wr, left, (u + 0.02, u + 0.08), (5.0, 7.0), palette::shade(col, shade * 1.15));
            }
            for j in 0..=6 {
                self.fline(wr, left, j as f32 / 6.0, (2.0, 10.0), palette::shade(rgb(0xe8, 0xec, 0xee), shade));
            }
            // The fascia: a band of brand-free blue.
            self.fquad(wr, left, (0.0, 1.0), (10.0, 13.0), palette::shade(SHOW_FASCIA, shade));
            self.fquad(wr, left, (0.0, 1.0), (11.0, 12.0), palette::shade(rgb(0xe8, 0xf0, 0xf8), shade));
        }
        // Flag poles with pennants at the corners, bunting along the kerb.
        let pl = (0.22, 0.2);
        let pr = (al - 0.22, 0.2);
        for &(t, d) in &[pl, pr] {
            let (x, y) = self.w(t, d);
            self.key_at_point(x, y);
            self.pole(t, d, 0.0, 30.0, STEEL);
        }
        for &(t, d, c) in &[(pl.0, pl.1, rgb(0xe8, 0x5a, 0x4a)), (pr.0, pr.1, rgb(0xf0, 0xd0, 0x58))] {
            let top = self.pt(t, d, 30.0);
            self.fb.fill_rect(top.0 as i32 + 1, top.1 as i32, 6, 4, c);
            self.fb.put(top.0 as i32 + 7, top.1 as i32 + 1, c);
        }
        self.key_ground();
        let n = 16;
        let a = self.pt(pl.0, pl.1, 27.0);
        let c = self.pt(pr.0, pr.1, 27.0);
        let sag = |f: f32| (f * std::f32::consts::PI).sin() * 3.0;
        let mut prev = a;
        for k in 1..=n {
            let f = k as f32 / n as f32;
            let cur = (a.0 + (c.0 - a.0) * f, a.1 + (c.1 - a.1) * f + sag(f));
            self.fb.line(prev, cur, rgb(0xd8, 0xd8, 0xd0));
            prev = cur;
        }
        for k in 1..n {
            let f = k as f32 / n as f32;
            let (x, y) = (a.0 + (c.0 - a.0) * f, a.1 + (c.1 - a.1) * f + sag(f));
            let col = [rgb(0xe8, 0x5a, 0x4a), rgb(0xf0, 0xd0, 0x58), rgb(0x58, 0x9c, 0xe0), rgb(0xf4, 0xf0, 0xe4)][k % 4];
            let (xi, yi) = (x.round() as i32, y.round() as i32);
            self.fb.fill_rect(xi - 1, yi + 1, 3, 1, col);
            self.fb.put(xi, yi + 2, col);
        }
        self.lamp(al * 0.5, 0.15);
    }

    // ----------------------------------------------------------------- motel

    fn motel(&mut self) {
        let (al, dl) = (self.al, self.dl);
        self.verges(&[(0.17, dl - 0.18), (al - 0.17, dl - 0.18)]);
        self.key_ground();
        self.bays(Shape::Motel);
        let b = cp::building_for(Shape::Motel, al, dl, self.o.front >= 2).unwrap();
        self.key_rect(b.t0, b.t1, b.d0, b.d1);
        let h = 24.0;
        self.boxx(b.t0, b.t1, b.d0, b.d1, 0.0, h, MOTEL_ROOF, MOTEL_WALL);
        let wr = self.wrect(b.t0, b.t1, b.d0, b.d1);
        let doors = cp::motel_doors(al);
        let n = doors.len().max(1);
        for left in [true, false] {
            let shade = if left { 1.0 } else { 0.8 };
            // The balcony slab and its rail at mid height.
            self.fquad(wr, left, (0.0, 1.0), (11.0, 12.5), palette::shade(rgb(0xc8, 0xbc, 0xa0), shade));
            self.fquad(wr, left, (0.0, 1.0), (12.5, 14.5), palette::shade(MOTEL_RAIL, shade));
            self.fquad(wr, left, (0.0, 1.0), (13.5, 14.5), palette::shade(rgb(0xb8, 0xa8, 0x90), shade));
            for (k, _) in doors.iter().enumerate() {
                let u0 = k as f32 / n as f32;
                let u1 = (k + 1) as f32 / n as f32;
                let w = u1 - u0;
                // Ground floor: a door and a window.
                let lit = self.night() && (crate::tex::hash2(k as i32, 3, self.o.seed) > 0.4);
                let wcol = if lit { GLASS_LIT } else { GLASS };
                self.fquad(wr, left, (u0 + w * 0.12, u0 + w * 0.34), (1.0, 8.0), palette::shade(MOTEL_DOOR, shade));
                self.fquad(wr, left, (u0 + w * 0.48, u0 + w * 0.86), (3.0, 8.0), palette::shade(wcol, shade));
                // Upper floor, the same, behind the rail.
                self.fquad(wr, left, (u0 + w * 0.12, u0 + w * 0.34), (13.0, 20.0), palette::shade(MOTEL_DOOR, shade));
                self.fquad(wr, left, (u0 + w * 0.48, u0 + w * 0.86), (15.0, 20.0), palette::shade(wcol, shade));
                // A rail post between rooms.
                self.fline(wr, left, u0, (11.0, 14.5), palette::shade(MOTEL_RAIL, shade));
            }
            // The roof's fascia.
            self.fquad(wr, left, (0.0, 1.0), (h - 2.5, h), palette::shade(rgb(0x58, 0x5c, 0x66), shade));
        }
        // The neon pylon at the kerb end.
        let far = self.o.front >= 2;
        let (pt_, pd) = (if far { 0.17 } else { al - 0.25 }, 0.22);
        let (x, y) = self.w(pt_, pd);
        self.key_at_point(x, y);
        let base = self.pt(pt_, pd, 0.0);
        let top = self.pt(pt_, pd, 26.0);
        self.fb.line(base, top, STEEL);
        self.fb.line((base.0 + 1.0, base.1), (top.0 + 1.0, top.1), palette::shade(STEEL, 0.7));
        let (bx, by) = (top.0 as i32 - 11, top.1 as i32 - 11);
        self.fb.fill_rect(bx - 1, by - 1, 24, 14, rgb(0x2a, 0x1c, 0x34));
        self.fb.fill_rect(bx, by, 22, 12, rgb(0x34, 0x24, 0x40));
        let neon = if self.night() { em(NEON) } else { palette::shade(NEON, 0.7) };
        text(self.fb, bx + 1, by + 1, "MOTEL", neon);
        let vac = if self.night() { em(NEON_B) } else { palette::shade(NEON_B, 0.7) };
        self.fb.fill_rect(bx + 2, by + 8, 18, 1, vac);
        self.fb.fill_rect(bx + 5, by + 10, 12, 1, vac);
        if self.night() {
            self.fb.put_blended(bx + 10, by + 14, NEON, 0.6);
            self.fb.put_blended(bx + 11, by + 14, NEON, 0.4);
        }
        let ld = if far { 1.25 } else { 0.3 };
        self.lamp(0.2, ld + if far { 0.0 } else { 0.0 });
        self.lamp(al * 0.5, ld);
    }
}

fn bay_ts(len: f32, margin: f32, pitch: f32) -> Vec<f32> {
    let n = ((len - 2.0 * margin) / pitch).floor().max(1.0) as i32;
    let step = (len - 2.0 * margin) / n as f32;
    (0..n).map(|i| margin + (i as f32 + 0.5) * step).collect()
}
