//! Civic art (B52, D67/D75): the government district's dressing, election
//! posters and the rally, a building fire and its engines, a blackout and its
//! crews, industrial smoke and smog, and the small street-life props (a cat on
//! a ridge, a lemonade stand, an angler's rod, a saxophone).
//!
//! Original pixel art in the world's 1x vocabulary (people 9x15, tiles 64x32).
//! The sim decides *what* is on and *where*; this module only paints one of
//! each into a framebuffer, clipped. Static pieces are baked into the scene's
//! static layer (the civic centre's flags and lamps, the rally square); the
//! rest is drawn per composite through the depth plane so a building in front
//! hides it, and nothing paints at all unless the scene asks.

use crate::framebuffer::{Framebuffer, Rect};
use crate::iso::Camera;
use crate::palette::{self, Color, rgb};
use crate::people::Figure;
use map::TileMap;

// ---------------------------------------------------------------------------
// What the scene is told
// ---------------------------------------------------------------------------

/// The part of the civic picture that is baked into the static layer. A change
/// re-bakes, so it only changes when something starts or stops (a rally, a
/// blackout's edge).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CivicStatic {
    /// The square wears the rally dress (bunting in the two slates' colours,
    /// the stage), from the rally to the end of election night.
    pub rally: bool,
    pub slates: [Color; 2],
    /// The dark zone of a blackout in tile space (centre x, y, radius),
    /// quantised by the glue so the bake does not run every step.
    pub dark: Option<(i32, i32, i32)>,
}

impl Default for CivicStatic {
    fn default() -> Self {
        CivicStatic {
            rally: false,
            slates: [rgb(0xe0, 0xa0, 0x20), rgb(0x28, 0x96, 0xa0)],
            dark: None,
        }
    }
}

impl CivicStatic {
    /// Is this tile inside the blackout? (Its lamps are dark.)
    pub fn is_dark(&self, x: i32, y: i32) -> bool {
        match self.dark {
            Some((cx, cy, r)) => {
                let (dx, dy) = ((x - cx) as f32, (y - cy) as f32);
                dx * dx + dy * dy <= (r * r) as f32
            }
            None => false,
        }
    }
}

/// A small thing standing in the street.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PropKind {
    /// An election poster on an A-frame; `v` is the slate (0/1).
    Poster,
    /// A cat on a roof ridge at the lot (`x`,`y` anchor, `w`,`h` size via
    /// `span`); `v` is its coat (0..3).
    Cat,
    /// The children's lemonade stand.
    Lemonade,
    /// An angler's rod and bucket, facing `dir`.
    Rod,
    /// The saxophone, its case and the notes.
    Sax,
    /// The street band's notes drifting over the row.
    Notes,
    /// A fire engine at the scene; `dir` along x (0/2) or y (1/3).
    Engine,
    /// A repair crew's truck (blackout, fire aftermath).
    Truck,
    /// A cone and barrier tape round a repair.
    Barrier,
    /// The results board on election night.
    Board,
    /// A police car parked at the station.
    PoliceCar,
    /// A post van.
    PostVan,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CivicProp {
    pub x: f32,
    pub y: f32,
    pub kind: PropKind,
    pub v: u8,
    pub dir: u8,
    /// Height above the ground in px (a cat on a roof).
    pub lift: i32,
}

/// A building on fire, in tile space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FireLook {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub level: u8,
    /// Wall height in px (from the scene's own object).
    pub wall: i32,
    /// 0 smoke only, 1 flames, 2 aftermath (the smoke has thinned, crews work).
    pub stage: u8,
    /// 0..1 through the stage.
    pub u: f32,
}

/// An industrial chimney's plume.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stack {
    pub x: f32,
    pub y: f32,
    pub level: u8,
    pub wall: i32,
    /// 0..1: how thick the smoke is (the smog; scrubbers thin it).
    pub strength: f32,
    /// Scrubbed: pale steam instead of brown smoke.
    pub scrubbed: bool,
}

/// Everything that is drawn per composite.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CivicDyn {
    pub figures: Vec<Figure>,
    pub props: Vec<CivicProp>,
    pub fires: Vec<FireLook>,
    pub stacks: Vec<Stack>,
    /// Smog over the town, 0 clear .. 1 thick, and the centre of the haze (the
    /// industrial ground) in tiles.
    pub smog: f32,
    pub smog_at: (f32, f32),
}

impl CivicDyn {
    pub fn is_empty(&self) -> bool {
        self.figures.is_empty()
            && self.props.is_empty()
            && self.fires.is_empty()
            && self.stacks.is_empty()
            && self.smog < 0.05
    }

    /// Does anything here move between frames?
    pub fn animated(&self) -> bool {
        !self.fires.is_empty()
            || !self.stacks.is_empty()
            || self.props.iter().any(|p| {
                matches!(p.kind, PropKind::Sax | PropKind::Notes | PropKind::Engine | PropKind::Truck | PropKind::Cat | PropKind::Rod)
            })
    }
}

/// The whole look, as the glue builds it from the sim.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CivicLook {
    pub stat: CivicStatic,
    pub dynamic: CivicDyn,
}

// ---------------------------------------------------------------------------
// Drawing helpers
// ---------------------------------------------------------------------------

/// A depth-tested pen: a pixel hidden by anything nearer is not drawn.
pub struct Pen<'a> {
    pub fb: &'a mut Framebuffer,
    pub depth: &'a [u16],
    pub key: u16,
}

impl Pen<'_> {
    #[inline]
    pub fn put(&mut self, x: i32, y: i32, c: Color) {
        let clip = self.fb.clip();
        if x < clip.x0 || x >= clip.x1 || y < clip.y0 || y >= clip.y1 {
            return;
        }
        let i = y as usize * self.fb.w + x as usize;
        if self.depth.get(i).is_some_and(|d| *d > self.key) {
            return;
        }
        self.fb.put(x, y, c);
    }

    pub fn rect(&mut self, x: i32, y: i32, w: i32, h: i32, c: Color) {
        for yy in y..y + h {
            for xx in x..x + w {
                self.put(xx, yy, c);
            }
        }
    }

    /// A translucent pixel (smoke, glow).
    pub fn blend(&mut self, x: i32, y: i32, c: Color, cov: f32) {
        let clip = self.fb.clip();
        if x < clip.x0 || x >= clip.x1 || y < clip.y0 || y >= clip.y1 {
            return;
        }
        let i = y as usize * self.fb.w + x as usize;
        if self.depth.get(i).is_some_and(|d| *d > self.key) {
            return;
        }
        self.fb.put_blended(x, y, c, cov);
    }

    pub fn line(&mut self, a: (i32, i32), b: (i32, i32), c: Color) {
        let (mut x, mut y) = a;
        let (dx, dy) = ((b.0 - a.0).abs(), -(b.1 - a.1).abs());
        let (sx, sy) = (if a.0 < b.0 { 1 } else { -1 }, if a.1 < b.1 { 1 } else { -1 });
        let mut err = dx + dy;
        loop {
            self.put(x, y, c);
            if x == b.0 && y == b.1 {
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
}

/// The ground point of a tile-space position, in framebuffer pixels.
pub fn ground(cam: &Camera, map: &TileMap, x: f32, y: f32) -> (i32, i32) {
    let z = crate::iso::level_px(map.surface_level_at(x, y));
    let p = cam.project(x, y, z);
    (p.0 as i32, p.1 as i32)
}

fn h01(a: i32, b: i32, salt: u32) -> f32 {
    crate::tex::hash2(a, b, salt)
}

// ---------------------------------------------------------------------------
// The government district's dressing (static)
// ---------------------------------------------------------------------------

/// A flagpole with a flag, in screen space at a foot point.
fn flagpole(fb: &mut Framebuffer, foot: (i32, i32), h: i32, flag: Color, trim: Color, wave: i32) {
    let (x, y) = foot;
    let h = h * 3 / 2;
    for k in 0..h {
        fb.put(x, y - k, rgb(0xd8, 0xd8, 0xdc));
    }
    fb.put(x, y - h - 1, rgb(0xf0, 0xd0, 0x50));
    // The flag: 14 x 9, rippling a pixel.
    for i in 0..14 {
        let off = ((i as f32 * 0.7 + wave as f32).sin() * 1.2).round() as i32;
        for j in 0..9 {
            let c = if j == 0 || j == 8 { trim } else { flag };
            fb.put(x + 1 + i, y - h + j + off, c);
        }
    }
    fb.put(x, y, rgb(0x60, 0x60, 0x68));
}

fn lamp_blue(fb: &mut Framebuffer, x: i32, y: i32, lit: bool) {
    let c = if lit { rgb(0x50, 0xa8, 0xff) } else { rgb(0x2c, 0x58, 0x90) };
    fb.fill_rect(x - 1, y - 1, 3, 3, c);
    fb.put(x - 1, y - 1, palette::shade(c, 1.35));
    fb.put(x, y + 2, rgb(0x30, 0x32, 0x3a));
}

/// Dress a civic building (static layer). `o` is the scene's own object; the
/// pieces stand on the forecourt at the front of the lot.
pub fn civic_dressing(fb: &mut Framebuffer, cam: &Camera, o: &crate::city::CityObj, glow: f32) {
    use crate::city::{LandmarkKind as K, ObjKind};
    let kind = match o.kind {
        ObjKind::Hall => 0,
        ObjKind::Landmark(K::Library) => 4,
        ObjKind::Landmark(K::FireStation) => 5,
        _ => return,
    };
    let base = o.level as i32 * crate::iso::HEIGHT_UNIT;
    // The two faces the camera sees are +y (front 1) and +x (front 0). Put the
    // forecourt on the +y face unless the street is on +x.
    let on_x = o.front % 4 == 0;
    let (fx, fy) = if on_x {
        ((o.x + o.w) as f32 + 0.35, o.y as f32 + o.h as f32 * 0.5)
    } else {
        (o.x as f32 + o.w as f32 * 0.5, (o.y + o.h) as f32 + 0.35)
    };
    // Along the kerb, perpendicular to the front.
    let (ax, ay) = if on_x { (0.0, 1.0) } else { (1.0, 0.0) };
    let reach = if on_x { o.h as f32 } else { o.w as f32 };
    let at = |t: f32| cam.project(fx + ax * t * reach * 0.5, fy + ay * t * reach * 0.5, base);
    let pole = |fb: &mut Framebuffer, t: f32, h: i32, flag: Color, trim: Color| {
        let p = at(t);
        flagpole(fb, (p.0 as i32, p.1 as i32), h, flag, trim, (t * 3.0) as i32);
    };
    match kind {
        // The hall: three flagpoles, the middle one tallest.
        0 => {
            pole(fb, -0.9, 30, rgb(0x2c, 0x5c, 0xa8), rgb(0xf0, 0xf0, 0xf4));
            pole(fb, 0.0, 38, rgb(0xb8, 0x34, 0x34), rgb(0xf0, 0xf0, 0xf4));
            pole(fb, 0.9, 30, rgb(0xe0, 0xa0, 0x20), rgb(0x30, 0x34, 0x40));
        }
        // The courthouse: two flags and a pair of stone urns at the steps.
        1 => {
            pole(fb, -0.8, 34, rgb(0x2c, 0x5c, 0xa8), rgb(0xf0, 0xf0, 0xf4));
            pole(fb, 0.8, 34, rgb(0xb8, 0x34, 0x34), rgb(0xf0, 0xf0, 0xf4));
            for t in [-0.35, 0.35] {
                let p = at(t);
                let (x, y) = (p.0 as i32, p.1 as i32);
                fb.fill_rect(x - 2, y - 7, 5, 7, rgb(0xb8, 0xb4, 0xa8));
                fb.fill_rect(x - 3, y - 8, 7, 2, rgb(0xcc, 0xc8, 0xbc));
                fb.fill_rect(x - 2, y - 11, 5, 3, rgb(0x4c, 0x7a, 0x44));
            }
        }
        // The police station: a flag and the blue lamps either side of the door.
        2 => {
            pole(fb, -0.9, 30, rgb(0x2c, 0x5c, 0xa8), rgb(0xf0, 0xf0, 0xf4));
            let lit = glow > 0.35;
            for t in [-0.3, 0.3] {
                let p = at(t);
                let (x, y) = (p.0 as i32, p.1 as i32);
                fb.fill_rect(x, y - 13, 1, 13, rgb(0x30, 0x32, 0x3a));
                lamp_blue(fb, x, y - 15, lit);
            }
        }
        // The post office: a flag, and a pillar box at the kerb.
        3 => {
            pole(fb, -0.9, 30, rgb(0xb8, 0x34, 0x34), rgb(0xf0, 0xf0, 0xf4));
            let p = at(0.75);
            let (x, y) = (p.0 as i32, p.1 as i32);
            fb.fill_rect(x - 2, y - 9, 5, 9, rgb(0x2c, 0x5c, 0xa8));
            fb.fill_rect(x - 2, y - 10, 5, 2, rgb(0x3c, 0x70, 0xc4));
            fb.fill_rect(x - 1, y - 7, 3, 1, rgb(0x14, 0x20, 0x38));
            fb.put(x + 2, y - 4, rgb(0xf0, 0xd0, 0x50));
        }
        // The library: a single flag on the lawn.
        4 => pole(fb, -0.9, 26, rgb(0x5a, 0x8a, 0x4a), rgb(0xf0, 0xf0, 0xf4)),
        // The fire station: a flag and the red lamp above the engine doors.
        _ => {
            pole(fb, 0.9, 28, rgb(0xb8, 0x34, 0x34), rgb(0xf0, 0xf0, 0xf4));
            let p = at(-0.2);
            let (x, y) = (p.0 as i32, p.1 as i32);
            fb.fill_rect(x, y - 12, 1, 12, rgb(0x30, 0x32, 0x3a));
            let c = if glow > 0.35 { rgb(0xff, 0x60, 0x50) } else { rgb(0x90, 0x34, 0x30) };
            fb.fill_rect(x - 1, y - 14, 3, 3, c);
        }
    }
}

/// The square dressed for the rally and election night (static layer): bunting
/// in the two slates' colours round the edge and the stage at the back.
pub fn rally_static(fb: &mut Framebuffer, cam: &Camera, sq: &crate::spectacle::Square, slates: [Color; 2]) {
    crate::spectacle::banner_dress(fb, cam, sq, slates, false);
    crate::spectacle::stage(fb, cam, sq.stage_rect(), sq.level);
    // A slate-coloured banner on the stage's back wall, one each side.
    let (x0, y0, x1, _y1) = sq.stage_rect();
    let base = sq.level as i32 * crate::iso::HEIGHT_UNIT;
    for (i, c) in slates.iter().enumerate() {
        let xa = if i == 0 { x0 + 0.1 } else { x1 - 0.55 };
        let q = [
            cam.project(xa, y0 + 0.12, base + 18),
            cam.project(xa + 0.45, y0 + 0.12, base + 18),
            cam.project(xa + 0.45, y0 + 0.12, base + 36),
            cam.project(xa, y0 + 0.12, base + 36),
        ];
        fb.fill_poly(&q, *c);
        fb.line(q[0], q[1], palette::shade(*c, 1.3));
    }
}

// ---------------------------------------------------------------------------
// Props (per composite, depth-tested)
// ---------------------------------------------------------------------------

/// An election poster on an A-frame: the slate's colour on top, a pale board
/// with a candidate's head and shoulders, and a bar of print.
pub fn poster(p: &mut Pen, sx: i32, sy: i32, slate: Color) {
    // B52b: twice the size and a dark outline, so it reads at 1x.
    let wood = rgb(0x4a, 0x38, 0x28);
    let ink = rgb(0x20, 0x1c, 0x1c);
    p.rect(sx - 8, sy - 2, 2, 2, wood);
    p.rect(sx + 6, sy - 2, 2, 2, wood);
    p.rect(sx - 9, sy - 25, 18, 24, ink);
    p.rect(sx - 8, sy - 24, 16, 22, rgb(0xff, 0xfc, 0xf0));
    p.rect(sx - 8, sy - 24, 16, 6, slate);
    p.rect(sx - 8, sy - 24, 16, 1, palette::shade(slate, 1.3));
    p.rect(sx - 2, sy - 16, 5, 5, rgb(0xe0, 0xb4, 0x88));
    p.rect(sx - 2, sy - 16, 5, 2, ink);
    p.rect(sx - 6, sy - 11, 13, 4, palette::shade(slate, 0.75));
    p.rect(sx - 6, sy - 6, 13, 2, ink);
    p.rect(sx - 4, sy - 3, 9, 1, rgb(0x70, 0x6c, 0x68));
}

/// A cat sitting on a ridge, facing left or right; at night its eyes glow.
pub fn cat(p: &mut Pen, sx: i32, sy: i32, coat: u8, face_right: bool, glow: f32, blink: bool) {
    let fur = [rgb(0x24, 0x22, 0x26), rgb(0x8a, 0x86, 0x84), rgb(0xc8, 0x84, 0x3c), rgb(0xe6, 0xe2, 0xdc)][coat as usize % 4];
    let dark = palette::shade(fur, 0.7);
    let d = if face_right { 1 } else { -1 };
    // Body (a sitting loaf), tail, head with two ears.
    p.rect(sx - 2, sy - 4, 5, 4, fur);
    p.rect(sx - 2 + d, sy - 7, 3, 3, fur);
    p.put(sx - 2 + d, sy - 8, fur);
    p.put(sx + d, sy - 8, fur);
    // The tail curls round the foot, away from the face.
    p.put(sx - 3 * d, sy - 1, dark);
    p.put(sx - 4 * d, sy - 2, dark);
    p.put(sx - 4 * d, sy - 3, dark);
    // Eyes: pale by day, a glow at night.
    let eye = if glow > 0.4 { rgb(0xd8, 0xff, 0x70) } else { rgb(0x80, 0xa0, 0x40) };
    if !blink {
        p.put(sx - 1 + d, sy - 6, eye);
        p.put(sx + 1 + d, sy - 6, eye);
    }
    p.put(sx - 1, sy - 1, dark);
    p.put(sx + 1, sy - 1, dark);
}

/// The children's lemonade stand: a table, a striped awning and a pitcher.
pub fn lemonade(p: &mut Pen, sx: i32, sy: i32) {
    let wood = rgb(0x9a, 0x78, 0x52);
    p.rect(sx - 7, sy - 5, 15, 2, wood);
    p.rect(sx - 6, sy - 3, 2, 3, palette::shade(wood, 0.8));
    p.rect(sx + 5, sy - 3, 2, 3, palette::shade(wood, 0.8));
    // Awning stripes, yellow and white.
    for i in 0..15 {
        let c = if (i / 2) % 2 == 0 { rgb(0xf0, 0xd8, 0x40) } else { rgb(0xf4, 0xf0, 0xe4) };
        p.put(sx - 7 + i, sy - 13, c);
        p.put(sx - 7 + i, sy - 12, c);
        p.put(sx - 7 + i, sy - 11, palette::shade(c, 0.85));
    }
    p.rect(sx - 7, sy - 13, 1, 9, rgb(0xd8, 0xd4, 0xc8));
    p.rect(sx + 7, sy - 13, 1, 9, rgb(0xd8, 0xd4, 0xc8));
    // Pitcher and cups.
    p.rect(sx - 2, sy - 9, 3, 4, rgb(0xf0, 0xe0, 0x58));
    p.put(sx - 2, sy - 9, rgb(0xf8, 0xf4, 0xe0));
    p.rect(sx + 2, sy - 7, 2, 2, rgb(0xf0, 0xe0, 0x58));
    // The sign: a lemon on a card.
    p.rect(sx + 9, sy - 7, 5, 5, rgb(0xf4, 0xf0, 0xe4));
    p.rect(sx + 10, sy - 6, 3, 3, rgb(0xf0, 0xd8, 0x40));
    p.put(sx + 11, sy - 7, rgb(0x4c, 0x8a, 0x44));
    p.rect(sx + 11, sy - 2, 1, 2, wood);
}

/// An angler's rod and line, and the bucket. The angler stands at `(sx, sy)`,
/// facing `dir` (toward the water).
pub fn rod(p: &mut Pen, sx: i32, sy: i32, dir: u8, t: f32) {
    // Screen direction of each tile direction: +x down-right, +y down-left.
    let (dx, dy): (i32, i32) = match dir % 4 {
        0 => (1, 1),
        1 => (-1, 1),
        2 => (-1, -1),
        _ => (1, -1),
    };
    let hand = (sx + dx, sy - 7);
    let tip = (sx + dx * 11, sy - 15 + dy);
    p.line(hand, tip, rgb(0x6a, 0x52, 0x38));
    // The line drops to the water, the float bobbing.
    let bob = ((t * 2.2).sin() * 0.6).round() as i32;
    let fl = (tip.0 + dx * 3, sy - 1 + dy * 2 + bob);
    p.line(tip, fl, rgb(0xe8, 0xe8, 0xf0));
    p.put(fl.0, fl.1, rgb(0xe0, 0x40, 0x30));
    // A bucket behind.
    let (bx, by) = (sx - dx * 5, sy);
    p.rect(bx - 2, by - 3, 4, 3, rgb(0x70, 0x76, 0x80));
    p.put(bx - 2, by - 4, rgb(0x90, 0x96, 0xa0));
}

/// The saxophone across the player, and the open case.
pub fn sax(p: &mut Pen, sx: i32, sy: i32, t: f32, face_right: bool) {
    let d = if face_right { 1 } else { -1 };
    let brass = rgb(0xe0, 0xb0, 0x38);
    let hi = rgb(0xff, 0xe0, 0x80);
    // Neck at the mouth, the body down the chest, the bell turned up.
    p.line((sx + d, sy - 10), (sx + d * 3, sy - 8), brass);
    p.line((sx + d * 3, sy - 8), (sx + d * 3, sy - 3), brass);
    p.put(sx + d * 4, sy - 3, brass);
    p.put(sx + d * 4, sy - 4, hi);
    p.put(sx + d * 5, sy - 4, brass);
    p.put(sx + d * 3, sy - 6, hi);
    // The open case.
    p.rect(sx - d * 8, sy - 1, 6, 2, rgb(0x2c, 0x2a, 0x30));
    p.put(sx - d * 7, sy - 1, rgb(0x8a, 0x20, 0x30));
    p.put(sx - d * 5, sy - 1, rgb(0xe8, 0xc8, 0x50));
    notes(p, sx + d * 4, sy - 12, t);
}

/// Music notes drifting up and away.
pub fn notes(p: &mut Pen, x: i32, y: i32, t: f32) {
    for k in 0..3 {
        let ph = (t * 0.55 + k as f32 / 3.0).fract();
        let nx = x + (k as f32 * 5.0 - 5.0 + (ph * 6.0).sin() * 2.0) as i32;
        let ny = y - (ph * 18.0) as i32;
        let c = rgb(0xf8, 0xf0, 0xd0);
        let a = (1.0 - ph).clamp(0.0, 1.0);
        p.blend(nx, ny, c, a);
        p.blend(nx, ny + 1, c, a);
        p.blend(nx - 1, ny + 2, c, a);
        p.blend(nx, ny + 2, c, a);
        p.blend(nx + 1, ny - 1, c, a * 0.8);
    }
}

/// The results board on election night: two columns of the slates' colours.
pub fn board(p: &mut Pen, sx: i32, sy: i32, slates: [Color; 2], lead: f32) {
    let wood = rgb(0x4a, 0x3c, 0x30);
    p.rect(sx - 8, sy - 3, 2, 3, wood);
    p.rect(sx + 6, sy - 3, 2, 3, wood);
    p.rect(sx - 9, sy - 21, 18, 18, rgb(0x24, 0x28, 0x34));
    p.rect(sx - 9, sy - 21, 18, 1, rgb(0x70, 0x78, 0x90));
    // Two bars growing with the count.
    let a = (6.0 + 8.0 * lead.clamp(0.0, 1.0)) as i32;
    let b = (14.0 - 6.0 * lead.clamp(0.0, 1.0)) as i32;
    p.rect(sx - 7, sy - 6 - a, 5, a, slates[0]);
    p.rect(sx + 2, sy - 6 - b, 5, b, slates[1]);
    p.rect(sx - 7, sy - 6 - a, 5, 1, palette::shade(slates[0], 1.3));
    p.rect(sx + 2, sy - 6 - b, 5, 1, palette::shade(slates[1], 1.3));
}

/// A traffic cone with tape (repair works).
pub fn barrier(p: &mut Pen, sx: i32, sy: i32) {
    for dx in [-7, 7] {
        p.rect(sx + dx - 1, sy - 5, 3, 5, rgb(0xf0, 0x70, 0x20));
        p.put(sx + dx, sy - 3, rgb(0xf4, 0xf0, 0xe4));
        p.put(sx + dx, sy - 6, rgb(0xf0, 0x70, 0x20));
    }
    // Hazard tape between the cones.
    for i in -6..=6 {
        let c = if (i / 2) % 2 == 0 { rgb(0xf0, 0xd0, 0x30) } else { rgb(0x30, 0x2c, 0x28) };
        p.put(sx + i, sy - 4, c);
    }
}

/// A truck on the road: `kind` 0 repair crew (yellow, amber beacon), 1 post
/// van (white with a red band), 2 police car (white, blue lights).
pub fn vehicle(p: &mut Pen, sx: i32, sy: i32, kind: u8, along_x: bool, t: f32, glow: f32) {
    let (body, accent) = match kind {
        0 => (rgb(0xf0, 0xc0, 0x28), rgb(0x30, 0x34, 0x3c)),
        1 => (rgb(0xf0, 0xee, 0xe6), rgb(0xc0, 0x38, 0x34)),
        _ => (rgb(0xf0, 0xee, 0xe6), rgb(0x2c, 0x5c, 0xa8)),
    };
    // A 16 x 7 box on the road, seen from the front-left; mirrored by axis.
    let (bx, by) = (sx - 8, sy - 9);
    let flip = !along_x;
    p.rect(bx, by + 2, 16, 6, body);
    p.rect(bx, by + 2, 16, 1, palette::shade(body, 1.12));
    p.rect(bx, by + 7, 16, 1, palette::shade(body, 0.75));
    p.rect(bx + 3, by, 10, 3, palette::shade(body, 0.92));
    p.rect(bx + 4, by + 1, 3, 2, rgb(0x6a, 0x7a, 0x90));
    p.rect(bx + 9, by + 1, 3, 2, rgb(0x6a, 0x7a, 0x90));
    p.rect(bx, by + 5, 16, 1, accent);
    // Wheels.
    p.rect(bx + 2, by + 8, 3, 2, rgb(0x1c, 0x1c, 0x20));
    p.rect(bx + 11, by + 8, 3, 2, rgb(0x1c, 0x1c, 0x20));
    // The beacon.
    let on = ((t * 3.0) as i32) % 2 == 0;
    let (bc_a, bc_b) = if kind == 2 {
        (rgb(0x50, 0x90, 0xff), rgb(0xff, 0x50, 0x40))
    } else {
        (rgb(0xff, 0xb0, 0x30), rgb(0xff, 0xb0, 0x30))
    };
    let (la, lb) = if flip { (bx + 11, bx + 3) } else { (bx + 3, bx + 11) };
    p.put(la, by - 1, if on { bc_a } else { palette::shade(bc_a, 0.5) });
    p.put(lb, by - 1, if on { palette::shade(bc_b, 0.5) } else { bc_b });
    let _ = glow;
}

/// A fire engine: a red body with a white stripe, a ladder on the roof, two
/// flashing lights, and a hose to the building with a water arc.
pub fn engine(p: &mut Pen, sx: i32, sy: i32, t: f32, toward: (i32, i32)) {
    let red = rgb(0xc8, 0x28, 0x2c);
    let (bx, by) = (sx - 12, sy - 11);
    p.rect(bx, by + 3, 24, 7, red);
    p.rect(bx, by + 3, 24, 1, palette::shade(red, 1.25));
    p.rect(bx, by + 9, 24, 1, palette::shade(red, 0.7));
    // The cab and the locker doors.
    p.rect(bx + 1, by, 8, 4, palette::shade(red, 0.95));
    p.rect(bx + 2, by + 1, 6, 2, rgb(0x78, 0x8c, 0xa4));
    p.rect(bx, by + 6, 24, 1, rgb(0xf4, 0xf0, 0xe4));
    for k in 0..4 {
        p.rect(bx + 11 + k * 3, by + 4, 2, 2, palette::shade(red, 0.8));
    }
    // The ladder along the roof.
    p.rect(bx + 10, by + 1, 13, 1, rgb(0xd8, 0xd4, 0xc8));
    for k in 0..6 {
        p.put(bx + 11 + k * 2, by + 2, rgb(0xd8, 0xd4, 0xc8));
    }
    // Wheels.
    for wx in [3, 17] {
        p.rect(bx + wx, by + 10, 4, 3, rgb(0x1c, 0x1c, 0x20));
        p.put(bx + wx + 1, by + 11, rgb(0x90, 0x92, 0x98));
    }
    // Lights, alternating.
    let on = ((t * 4.0) as i32) % 2 == 0;
    p.put(bx + 2, by - 1, if on { rgb(0xff, 0x50, 0x40) } else { rgb(0x80, 0x30, 0x2c) });
    p.put(bx + 6, by - 1, if on { rgb(0x80, 0x30, 0x2c) } else { rgb(0xff, 0x50, 0x40) });
    // The hose: a sagging line to the wall, and the jet arcing over.
    let (tx, ty) = toward;
    p.line((sx + 6, sy - 2), (tx, ty + 4), rgb(0xd8, 0xc0, 0x40));
    for k in 0..14 {
        let f = (k as f32 + (t * 9.0).fract()) / 14.0;
        let x = sx as f32 + 10.0 + (tx - sx - 10) as f32 * f;
        let y = sy as f32 - 14.0 + (ty - sy + 6) as f32 * f - (f * (1.0 - f) * 4.0 * 14.0);
        p.blend(x as i32, y as i32, rgb(0xb0, 0xd8, 0xff), 0.85);
    }
}

// ---------------------------------------------------------------------------
// Fire, smoke, haze
// ---------------------------------------------------------------------------

/// A puff of smoke: a dithered disc fading outwards.
fn puff(p: &mut Pen, cx: f32, cy: f32, r: f32, col: Color, alpha: f32) {
    let (x0, x1) = ((cx - r).floor() as i32, (cx + r).ceil() as i32);
    let (y0, y1) = ((cy - r).floor() as i32, (cy + r).ceil() as i32);
    for y in y0..=y1 {
        for x in x0..=x1 {
            let (dx, dy) = (x as f32 - cx, y as f32 - cy);
            let d = (dx * dx + dy * dy).sqrt() / r;
            if d > 1.0 {
                continue;
            }
            // Ordered dither on the rim keeps the edge pixel-art, not a blur.
            let thr = ((x & 1) * 2 + (y & 1)) as f32 / 4.0;
            let a = alpha * (1.0 - d * d * 0.6);
            if a > thr * 0.25 {
                p.blend(x, y, col, (a * 1.5).clamp(0.0, 0.85));
            }
        }
    }
}

/// One flame: a licking tongue, red at the base, orange, then a yellow tip.
fn flame(p: &mut Pen, x: i32, y: i32, size: f32, t: f32, seed: f32) {
    let flick = ((t * 11.0 + seed * 7.0).sin() * 0.5 + 0.5) * 0.4 + 0.8;
    let h = (13.0 * size * flick) as i32;
    let w = 6.0 * size;
    let sway = ((t * 6.0 + seed * 3.0).sin() * 1.6) as i32;
    for k in 0..h {
        let f = k as f32 / h.max(1) as f32;
        // Bulges low, narrows to a point.
        let half = (w * 0.5 * (std::f32::consts::PI * f.powf(0.75)).sin().max(0.0) * (1.0 - f * 0.4)).round() as i32;
        let off = (sway as f32 * f * f) as i32;
        for dx in -half..=half {
            let core = dx.abs() * 3 <= half * 2;
            let c = if f < 0.3 {
                rgb(0xd0, 0x28, 0x1c)
            } else if f < 0.62 {
                if core { rgb(0xff, 0x90, 0x24) } else { rgb(0xe8, 0x50, 0x1c) }
            } else if core {
                rgb(0xff, 0xe0, 0x50)
            } else {
                rgb(0xff, 0x9c, 0x28)
            };
            p.put(x + dx + off, y - k, c);
        }
    }
}

/// A building on fire: windows ablaze on the two visible faces, flames over the
/// roof, and the smoke column leaning with the wind. `stage` 0 is the first
/// smoke only, 1 the blaze, 2 the aftermath (a few embers, thinning smoke).
pub fn fire(p: &mut Pen, cam: &Camera, f: &FireLook, t: f32, wind: (f32, f32), glow: f32) {
    let wall_key = p.key;
    let base = f.level as i32 * crate::iso::HEIGHT_UNIT;
    let wall = f.wall.max(16);
    let top = base + wall;
    let (cx, cy) = (f.x as f32 + f.w as f32 * 0.5, f.y as f32 + f.h as f32 * 0.5);
    let roof = cam.project(cx, cy, top);
    let (rx, ry) = (roof.0 as i32, roof.1 as i32);
    // Intensity of the blaze 0..1.
    let burn = match f.stage {
        0 => 0.0,
        1 => (f.u * 6.0).min(1.0) * (1.0 - ((f.u - 0.7).max(0.0) / 0.3) * 0.7),
        _ => 0.0,
    };
    if burn > 0.02 {
        // Window flames along the +y face and the +x face.
        let faces: [(f32, f32, f32, f32); 2] = [
            (f.x as f32 + 0.5, (f.y + f.h) as f32 - 0.5, (f.w as f32 - 1.0).max(0.4), 0.0),
            ((f.x + f.w) as f32 - 0.5, f.y as f32 + 0.5, 0.0, (f.h as f32 - 1.0).max(0.4)),
        ];
        let mut n = 0;
        for (fx0, fy0, dx, dy) in faces {
            let cols = ((dx + dy) * 1.2).round().max(2.0) as i32;
            for c in 0..cols {
                for row in 0..((wall / 20).clamp(1, 2)) {
                    let h = h01(c + n * 13, row, f.x as u32 * 31 + f.y as u32);
                    if h > burn * 0.95 {
                        continue;
                    }
                    let u = (c as f32 + 0.5) / cols as f32;
                    let z = base + wall * 45 / 100 + row * (wall / 3).max(8);
                    let q = cam.project(fx0 + dx * u, fy0 + dy * u, z);
                    flame(p, q.0 as i32, q.1 as i32 + 2, 0.7 + 0.5 * burn * h, t, h * 5.0 + c as f32);
                }
            }
            n += 1;
        }
        // The roof ablaze.
        for k in 0..4 {
            let h = h01(k, 5, f.x as u32);
            flame(p, rx - 8 + k * 5 + (h * 3.0) as i32, ry + 3, 1.0 + 0.8 * burn, t, h * 9.0 + k as f32);
        }
    }
    // Smoke: a column of puffs rising and leaning away from the wind. It hangs
    // in the air above the roofs, so nothing hides it.
    p.key = u16::MAX;
    let strength = match f.stage {
        0 => 0.25 + 0.5 * f.u,
        1 => 0.9,
        _ => 0.5 * (1.0 - f.u),
    };
    if strength > 0.03 {
        let n = (10.0 + 10.0 * strength) as i32;
        let lean = (wind.0 * 4.0 + 1.0).clamp(-6.0, 9.0);
        for k in 0..n {
            let ph = ((t * 0.12) + k as f32 / n as f32).fract();
            let rise = ph * (70.0 + 80.0 * strength);
            let drift = ph * ph * lean * 8.0 + ((t * 0.9 + k as f32).sin() * 2.0);
            let r = 3.0 + ph * (9.0 + 7.0 * strength);
            let dark = 0.30 + 0.35 * strength;
            let col = rgb((0x20 as f32 * (1.0 - dark) + 0x30 as f32) as u8, (0x1e as f32 * (1.0 - dark) + 0x2e as f32) as u8, (0x1c as f32 * (1.0 - dark) + 0x2c as f32) as u8);
            let a = (1.0 - ph) * (0.35 + 0.5 * strength);
            puff(p, rx as f32 + drift, ry as f32 - rise, r, col, a);
        }
    }
    p.key = wall_key;
    // Embers glow after dark.
    if burn > 0.05 && glow > 0.35 {
        for k in 0..6 {
            let ph = (t * 0.7 + k as f32 * 0.31).fract();
            let ex = rx as f32 + (k as f32 - 3.0) * 4.0 + (ph * 5.0).sin() * 3.0;
            p.blend(ex as i32, (ry as f32 - ph * 40.0) as i32, rgb(0xff, 0xa0, 0x40), 1.0 - ph);
        }
    }
}

/// A chimney plume from a works.
pub fn stack(p: &mut Pen, cam: &Camera, s: &Stack, t: f32, wind: (f32, f32)) {
    p.key = u16::MAX;
    let base = s.level as i32 * crate::iso::HEIGHT_UNIT;
    let q = cam.project(s.x, s.y, base + s.wall + 12);
    let (sx, sy) = (q.0, q.1);
    let n = 6 + (6.0 * s.strength) as i32;
    let lean = (wind.0 * 3.0 + 1.0).clamp(-5.0, 7.0);
    for k in 0..n {
        let ph = ((t * 0.07) + k as f32 / n as f32).fract();
        let r = 2.5 + ph * (5.0 + 6.0 * s.strength);
        let col = if s.scrubbed { rgb(0xe8, 0xea, 0xee) } else { rgb(0x6a, 0x62, 0x58) };
        let a = (1.0 - ph) * if s.scrubbed { 0.35 } else { 0.35 + 0.45 * s.strength };
        puff(p, sx as f32 + ph * ph * lean * 7.0, sy as f32 - ph * (34.0 + 40.0 * s.strength), r, col, a);
    }
}

/// The smog's veil over the land (B52, reshaped by B66): an **even** wash over
/// everything the air stands over, faded a little toward the near ground so it
/// still reads as depth, and dithered so it reads as air rather than a gradient
/// fill.
///
/// It used to be a brown-grey disc centred on the industrial ground, which on
/// any coast or plain with the works in a corner of the frame read as a blob
/// floating in that corner. Air is not a disc: it lies over the whole land.
///
/// `r` is the box being composited, `band` the land band `(top, bottom)` on
/// screen (the veil stops at the land line — the sky has its own haze), and
/// `strength` 0..1 the weather's and the sim's smog.
pub fn haze(fb: &mut Framebuffer, r: Rect, band: (i32, i32), strength: f32, tint: [f32; 3]) {
    if strength < 0.03 {
        return;
    }
    let r = r.intersect(&fb.clip()).intersect(&Rect::new(0, band.0, fb.w as i32, band.1));
    if r.is_empty() {
        return;
    }
    let col = rgb(
        (0x9c as f32 * tint[0]).min(255.0) as u8,
        (0x8a as f32 * tint[1]).min(255.0) as u8,
        (0x66 as f32 * tint[2]).min(255.0) as u8,
    );
    let (top, bottom) = band;
    let span = (bottom - top).max(1) as f32;
    for y in r.y0..r.y1 {
        // 0 at the near ground under your feet, 1 up at the land line: the air
        // is thickest looking across the city, thinnest straight down.
        let far = (((bottom - y) as f32) / span).clamp(0.0, 1.0);
        let a = strength * (0.28 + 0.26 * far);
        for x in r.x0..r.x1 {
            // A quarter-step dither breaks the banding a flat wash would show.
            let dith = (((x & 3) * 4 + (y & 3)) as f32 / 16.0 - 0.5) * 0.03;
            fb.put_blended(x, y, col, (a + dith).clamp(0.0, 0.7));
        }
    }
}

/// `(1 - d2) ^ 1.3` for d2 in 0..1, tabulated once (B61d).
fn falloff_table() -> &'static [f32; 1024] {
    use std::sync::OnceLock;
    static TABLE: OnceLock<[f32; 1024]> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut t = [0.0f32; 1024];
        for (i, slot) in t.iter_mut().enumerate() {
            let d2 = i as f32 / 1023.0;
            *slot = (1.0 - d2).powf(1.3);
        }
        t
    })
}

/// The bounds a prop may touch, for damage.
pub fn prop_bounds(cam: &Camera, map: &TileMap, pr: &CivicProp) -> Rect {
    let (sx, sy) = ground(cam, map, pr.x, pr.y);
    let sy = sy - pr.lift;
    match pr.kind {
        PropKind::Engine => Rect::new(sx - 30, sy - 60, sx + 60, sy + 12),
        PropKind::Truck | PropKind::PoliceCar | PropKind::PostVan => Rect::new(sx - 12, sy - 14, sx + 12, sy + 6),
        PropKind::Sax | PropKind::Notes => Rect::new(sx - 14, sy - 34, sx + 16, sy + 6),
        PropKind::Rod => Rect::new(sx - 12, sy - 22, sx + 24, sy + 6),
        PropKind::Lemonade => Rect::new(sx - 10, sy - 16, sx + 18, sy + 4),
        PropKind::Board => Rect::new(sx - 11, sy - 24, sx + 11, sy + 2),
        _ => Rect::new(sx - 10, sy - 16, sx + 10, sy + 4),
    }
}

pub fn fire_bounds(cam: &Camera, f: &FireLook) -> Rect {
    let base = f.level as i32 * crate::iso::HEIGHT_UNIT;
    let top = base + f.wall.max(16);
    let (cx, cy) = (f.x as f32 + f.w as f32 * 0.5, f.y as f32 + f.h as f32 * 0.5);
    let roof = cam.project(cx, cy, top);
    let lo = cam.project((f.x + f.w) as f32, (f.y + f.h) as f32, base);
    let body = Rect::new(roof.0 as i32 - 70, roof.1 as i32 - 190, roof.0 as i32 + 130, lo.1 as i32 + 8);
    // B93: after dark the flames light the walls and the street round them with
    // a 110 px glow (`Scene::draw_civic_over`); it flickers every frame and must
    // be repainted away when the fire goes out.
    let c = cam.project(f.x as f32 + f.w as f32 * 0.5, (f.y + f.h) as f32, base + 10);
    let glow = crate::spectacle::Light { x: c.0, y: c.1, r: 110.0, rgb: [0.0; 3], k: 1.0 }.bounds();
    body.union(&glow)
}

pub fn stack_bounds(cam: &Camera, s: &Stack) -> Rect {
    let base = s.level as i32 * crate::iso::HEIGHT_UNIT;
    let q = cam.project(s.x, s.y, base + s.wall + 12);
    Rect::new(q.0 as i32 - 26, q.1 as i32 - 110, q.0 as i32 + 60, q.1 as i32 + 6)
}
