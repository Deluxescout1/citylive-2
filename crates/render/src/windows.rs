//! B72 (D78): the windows follow the people inside.
//!
//! A lit window used to be a function of the building's *use* and the clock: one
//! share per facade, one hash per pane. Nobody switched a light on. Here every
//! window pane is a small record — where it is on screen, how to colour it, and
//! which of its building's 64 light slots it belongs to — and the sim says which
//! slots are lit (a household at home and awake, a cleaner's floor, a night
//! shift). The scene keeps the panes' current state and repaints only the ones
//! that change, so a whole facade never flips in one frame and a light costs a
//! few dozen pixels, not a static rebuild.
//!
//! [`pane_px`] is the single place a window pixel is coloured: the initial bake
//! calls it with `lit = false` (every pane starts dark), the repaint calls it
//! with the state it is switching to, and the colour it computes is also the
//! "old" value the repaint compares against before it writes a pixel — so a
//! nearer building that has been drawn over a further one's wall is never
//! clobbered.

use crate::framebuffer::Rect;
use crate::palette::{self, pal, Color};

/// The key a building's panes are recorded against: its lot's anchor tile.
/// (A lot's footprint does not move, and two lots never share an anchor.)
pub fn obj_key(x: i32, y: i32) -> u32 {
    ((x as u32 & 0xFFFF) << 16) | (y as u32 & 0xFFFF)
}

/// Light slots per building: one bit each in the sim's `u64` plan. A tower's
/// panes share these, so a floor switches as a floor (which is how offices and
/// apartments read at night).
pub const SLOTS: u32 = 64;

/// How one wall face maps screen pixels to wall coordinates. This mirrors
/// `tex::wall_quad_px` exactly (same rounding, same clamping), because a repaint
/// has to land on the same pixels the bake did.
#[derive(Clone, Copy, Debug)]
pub struct FaceMap {
    /// x of the normalised left edge (the quad's smaller-x corner).
    pub left: f32,
    /// Width of the face in pixels (>0).
    pub dx: f32,
    /// Top edge y at the left and right corners.
    pub top_l: f32,
    pub top_r: f32,
    /// Height of the face in pixels at the left and right corners.
    pub span_l: f32,
    pub span_r: f32,
    /// The quad was given right-to-left.
    pub flip: bool,
    pub wpx: f32,
    pub hpx: f32,
}

impl FaceMap {
    /// The face's own bounds, for a quick reject.
    pub fn bounds(&self) -> Rect {
        let x0 = self.left.floor() as i32 - 1;
        let x1 = (self.left + self.dx).ceil() as i32 + 1;
        let y0 = (self.top_l.min(self.top_r)).floor() as i32 - 1;
        let y1 = (self.top_l.max(self.top_r) + self.span_l.max(self.span_r)).ceil() as i32 + 1;
        Rect::new(x0, y0, x1, y1)
    }

    /// Wall pixel coordinates of screen pixel `(x, y)`, or `None` when the pixel
    /// is outside the face (as `wall_quad_px` would leave it unwritten).
    pub fn at(&self, x: i32, y: i32) -> Option<(f32, f32)> {
        if self.dx.abs() < 0.5 || self.span_l <= 0.0 {
            return None;
        }
        let t = (x as f32 + 0.5 - self.left) / self.dx;
        if !(0.0..=1.0).contains(&t) {
            return None;
        }
        let yt = self.top_l + (self.top_r - self.top_l) * t;
        let yb = yt + self.span_l + (self.span_r - self.span_l) * t;
        if (y as f32) < (yt - 0.5).ceil() || (y as f32) >= (yb - 0.5).ceil() {
            return None;
        }
        let u = if self.flip { 1.0 - t } else { t };
        let v = ((y as f32 + 0.5 - yt) / (yb - yt).max(0.001)).clamp(0.0, 1.0);
        Some((u * self.wpx, v * self.hpx))
    }

    /// Screen bbox of a grid cell, generous by a pixel on each side.
    fn cell_rect(&self, u0: f32, u1: f32, v0: f32, v1: f32) -> Rect {
        let (t0, t1) = if self.flip { (1.0 - u1, 1.0 - u0) } else { (u0, u1) };
        let x0 = (self.left + t0 * self.dx - 0.5).floor() as i32 - 1;
        let x1 = (self.left + t1 * self.dx - 0.5).ceil() as i32 + 1;
        let mut y0 = f32::MAX;
        let mut y1 = f32::MIN;
        for t in [t0, t1] {
            let yt = self.top_l + (self.top_r - self.top_l) * t;
            let span = self.span_l + (self.span_r - self.span_l) * t;
            y0 = y0.min(yt + v0 * span);
            y1 = y1.max(yt + v1 * span);
        }
        Rect::new(x0, (y0 - 1.0).floor() as i32, x1, (y1 + 1.0).ceil() as i32)
    }
}

/// The window grid on a face.
#[derive(Clone, Copy, Debug)]
pub enum Grid {
    /// Punched windows on a solid wall (houses, shops, offices' stone).
    Punched {
        floor: f32,
        ww: f32,
        wh: f32,
        top: f32,
        n: f32,
    },
    /// Curtain wall: bays across, floors down.
    Glass { bay: f32, floor: f32 },
}

/// One wall face with windows on it.
#[derive(Clone, Copy, Debug)]
pub struct Face {
    pub map: FaceMap,
    /// 0 = the sunlit side, 1 = the shaded one.
    pub side: u8,
    pub grid: Grid,
    /// The wall's own colour (curtain mullions and spandrels are shades of it).
    pub wall: Color,
    /// The building's colour seed, so two houses never share a pattern.
    pub seed: u32,
}

/// One switchable pane.
#[derive(Clone, Debug)]
pub struct Pane {
    pub face: u32,
    pub col: i16,
    pub row: i16,
    pub slot: u8,
    /// How dark it has to be before this one comes on (0..1 of full night):
    /// the dusk switch-on is staggered window by window, never a facade flip.
    pub at: u8,
    pub lit: bool,
    /// B72 round 2: the pixels this pane's light spills onto, with what was
    /// there before it was lit (so switching it off is exact). Empty while dark.
    pub spill: Vec<(u32, Color)>,
}

/// What one wall pixel is, as far as the window goes.
#[derive(Clone, Copy, Debug)]
pub enum Hit {
    /// Not part of a window: the wall texture shows.
    Wall,
    /// A window colour (frame, mullion, glass — lit or dark).
    Ink(Color),
    /// The ledge under a window.
    Sill,
    /// A shadow line around a window: the caller shades its own wall colour.
    Dim(f32),
}

/// The pane's hash: which slot it belongs to, when it comes on, and nothing the
/// city can drift from — same save, same hour, same windows.
pub fn pane_hash(f: &Face, col: i32, row: i32) -> u32 {
    let mut x = (col as u32).wrapping_mul(0x9E37_79B9)
        ^ (row as u32).wrapping_mul(0x85EB_CA6B)
        ^ (f.side as u32).wrapping_mul(0x1234_5677)
        ^ f.seed.wrapping_mul(0x27D4_EB2F);
    x ^= x >> 15;
    x = x.wrapping_mul(0x2545_F491);
    x ^= x >> 13;
    x
}

/// Which light slot of its building a pane answers to.
pub fn slot_of(f: &Face, col: i32, row: i32) -> u8 {
    (pane_hash(f, col, row) & (SLOTS - 1)) as u8
}

/// The night level (0..1) at which a pane's own light comes on: spread over the
/// dusk, with most of the city lit by the time it is properly dark.
pub fn turn_on(f: &Face, col: i32, row: i32) -> u8 {
    ((pane_hash(f, col, row) >> 8) & 0xFF) as u8
}

impl Face {
    /// Is the pane at this wall pixel lit, given the building's lights and the
    /// night's own darkness?
    pub fn pane_lit(&self, lights: u64, night: f32, col: i32, row: i32) -> bool {
        let slot = slot_of(self, col, row);
        if (lights >> slot) & 1 == 0 {
            return false;
        }
        let at = turn_on(self, col, row) as f32 / 255.0;
        night >= 0.02 + 0.62 * at
    }

    /// The screen bbox of one pane's grid cell.
    pub fn cell_rect(&self, col: i32, row: i32) -> Rect {
        let (u0, u1, v0, v1) = self.cell_span(col, row);
        self.map.cell_rect(u0, u1, v0, v1)
    }

    /// Which grid cell a wall pixel belongs to.
    pub fn cell_of(&self, up: f32, vp: f32) -> (i32, i32) {
        self.cell_at(up, vp)
    }

    /// The cell's grid index at a wall pixel.
    fn cell_at(&self, up: f32, vp: f32) -> (i32, i32) {
        match self.grid {
            Grid::Punched { floor, .. } => {
                let col_pitch = self.map.wpx / self.grid_cols();
                ((up / col_pitch).floor() as i32, (vp / floor).floor() as i32)
            }
            Grid::Glass { bay, floor } => ((up / bay).floor() as i32, (vp / floor).floor() as i32),
        }
    }

    fn grid_cols(&self) -> f32 {
        match self.grid {
            Grid::Punched { n, .. } => n.max(1.0),
            Grid::Glass { .. } => 1.0,
        }
    }

    /// The u/v extents of one cell, for the repaint's iteration.
    fn cell_span(&self, col: i32, row: i32) -> (f32, f32, f32, f32) {
        match self.grid {
            Grid::Punched { floor, ww, wh, top, .. } => {
                let col_pitch = self.map.wpx / self.grid_cols();
                let u0 = col as f32 * col_pitch;
                let v0 = row as f32 * floor;
                (u0 / self.map.wpx, (u0 + ww) / self.map.wpx, (v0 + top) / self.map.hpx, (v0 + top + wh) / self.map.hpx)
            }
            Grid::Glass { bay, floor } => {
                let u0 = col as f32 * bay;
                let v0 = row as f32 * floor;
                (u0 / self.map.wpx, (u0 + bay) / self.map.wpx, v0 / self.map.hpx, (v0 + floor) / self.map.hpx)
            }
        }
    }

    /// Every pane on this face.
    pub fn panes(&self) -> Vec<(i16, i16)> {
        let mut out = Vec::new();
        match self.grid {
            Grid::Punched { floor, wh, top, .. } => {
                let cols = self.grid_cols() as i32;
                let rows = (self.map.hpx / floor) as i32;
                for row in 0..rows {
                    // A row whose window starts past the top of the wall is not
                    // drawn at all; one that runs past it is cut by the wall
                    // (which is what the bake does).
                    let v0 = row as f32 * floor + top;
                    if v0 >= self.map.hpx {
                        continue;
                    }
                    for col in 0..cols {
                        if self.map.wpx / self.grid_cols() < 6.0 {
                            continue;
                        }
                        out.push((col as i16, row as i16));
                    }
                }
            }
            Grid::Glass { bay, floor } => {
                let cols = (self.map.wpx / bay) as i32;
                let rows = (self.map.hpx / floor) as i32;
                for row in 0..rows {
                    for col in 0..cols {
                        out.push((col as i16, row as i16));
                    }
                }
            }
        }
        out
    }

    /// Repaint one pane: `put` is called for every screen pixel the pane owns,
    /// with the colour it should have. Returns the pane's screen bounds.
    pub fn paint_pane<F>(&self, col: i32, row: i32, lit: bool, mut put: F) -> Rect
    where
        F: FnMut(i32, i32, Color),
    {
        let (u0, u1, v0, v1) = self.cell_span(col, row);
        let r = self.map.cell_rect(u0, u1, v0, v1).intersect(&self.map.bounds());
        if r.is_empty() {
            return r;
        }
        for y in r.y0..r.y1 {
            for x in r.x0..r.x1 {
                let Some((up, vp)) = self.map.at(x, y) else {
                    continue;
                };
                if self.cell_at(up, vp) != (col, row) {
                    continue;
                }
                if let Hit::Ink(c) = self.pane_px(up, vp, lit) {
                    put(x, y, c);
                }
            }
        }
        r
    }

    /// One window pixel: the single source of truth for a pane's colour.
    pub fn pane_px(&self, up: f32, vp: f32, lit: bool) -> Hit {
        let shade = if self.side == 0 { 0.98 } else { 0.74 };
        match self.grid {
            Grid::Punched { floor, ww, wh, top, .. } => {
                let col_pitch = self.map.wpx / self.grid_cols();
                let col = (up / col_pitch).floor();
                let cu = up - col * col_pitch;
                let x0 = (col_pitch - ww) * 0.5;
                let row = (vp / floor).floor();
                let cv = vp - row * floor;
                let in_x = cu >= x0 && cu < x0 + ww;
                let in_y = cv >= top && cv < top + wh;
                if in_x && in_y {
                    let fx = cu - x0;
                    let fy = cv - top;
                    let frame = fx < 1.0 || fx > ww - 2.0 || fy < 1.0 || fy > wh - 2.0;
                    if frame {
                        return Hit::Ink(palette::shade(pal::DOOR, shade * 0.9));
                    }
                    if (fx - ww * 0.5).abs() < 0.6 {
                        return Hit::Ink(palette::shade(pal::DOOR, shade * 0.85));
                    }
                    let c = if lit {
                        palette::mix(pal::WINDOW_LIT, pal::WINDOW_HALO, (fy / wh) * 0.35)
                    } else if fy < wh * 0.35 && fx > ww * 0.25 {
                        palette::mix(pal::WINDOW_DARK, pal::STYLE_GLASS, 0.55)
                    } else {
                        palette::shade(pal::WINDOW_DARK, shade + 0.06)
                    };
                    return Hit::Ink(c);
                }
                if in_x {
                    if (cv - (top + wh)).abs() < 1.0 {
                        return Hit::Sill;
                    }
                    if (cv - (top + wh + 1.0)).abs() < 0.6 {
                        return Hit::Dim(0.80);
                    }
                    if (cv - (top - 1.0)).abs() < 0.6 {
                        return Hit::Dim(0.86);
                    }
                }
                Hit::Wall
            }
            Grid::Glass { bay, floor } => {
                let cu = up % bay;
                let cv = vp % floor;
                let base = self.wall;
                if cu < 1.0 || cv < 1.0 {
                    return Hit::Ink(palette::shade(base, 0.62));
                }
                if cv > floor - 2.0 {
                    return Hit::Ink(palette::shade(base, 0.80));
                }
                let t = cv / floor;
                let streak = ((up * 0.5 + vp * 1.6) % 34.0) < 3.0;
                let mut pane = palette::mix(
                    palette::shade(base, 0.92),
                    palette::mix(palette::shade(base, 1.12), pal::SKY_HORIZON, 0.26),
                    t * 0.9,
                );
                if streak {
                    pane = palette::mix(pane, pal::SKY_HORIZON, 0.22);
                }
                if lit {
                    let bay_i = (up / bay).floor() as i32;
                    let floor_i = (vp / floor).floor() as i32;
                    // Warm offices and cool ones; a pane off the corner of the
                    // bay (a reveal) is a touch dimmer.
                    let warm = (pane_hash(self, bay_i, floor_i) >> 20) & 1 == 0;
                    let reveal = cu < 1.6 || cv < 2.0;
                    let base_lit = if warm { pal::WINDOW_LIT } else { pal::WINDOW_LIT_COOL };
                    let c = palette::mix(base_lit, pal::WINDOW_HALO, if warm { 0.30 } else { 0.40 });
                    return Hit::Ink(if reveal { palette::shade(c, 0.88) } else { c });
                }
                Hit::Ink(pane)
            }
        }
    }
}

/// A window pane on a pre-rendered kit sprite, in screen space.
#[derive(Clone, Debug, Default)]
pub struct KitPane {
    /// Index into the scene's sprite list (to restore the dark pane).
    pub spr: u32,
    /// Where the sprite's pixel (0, 0) sits on screen.
    pub ox: i16,
    pub oy: i16,
    /// The pane's glass pixels, in sprite coordinates.
    pub px: Vec<(i16, i16)>,
    /// Warm tungsten or cool LED.
    pub warm: bool,
    pub slot: u8,
    /// Night level at which it comes on (0..255).
    pub at: u8,
    pub lit: bool,
    /// See [`Pane::spill`].
    pub spill: Vec<(u32, Color)>,
}

impl KitPane {
    /// The pane's screen bounds.
    pub fn rect(&self) -> Rect {
        let mut r = Rect::new(i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for (x, y) in &self.px {
            let sx = self.ox as i32 + *x as i32;
            let sy = self.oy as i32 + *y as i32;
            r = r.union(&Rect::new(sx, sy, sx + 1, sy + 1));
        }
        if r.x1 <= r.x0 {
            Rect::new(0, 0, 0, 0)
        } else {
            r
        }
    }

    /// Is it lit now? `lights` is its building's slot mask, `night` the sky's
    /// own darkness.
    pub fn want(&self, lights: u64, night: f32) -> bool {
        if (lights >> self.slot) & 1 == 0 {
            return false;
        }
        night >= 0.02 + 0.62 * (self.at as f32 / 255.0)
    }
}

/// One building's windows: the faces (engine art), the panes on them, and the
/// panes of its kit sprite.
#[derive(Default, Debug)]
pub struct ObjWin {
    pub key: u32,
    /// Which of the 64 slots are lit (from the people inside).
    pub lights: u64,
    pub faces: Vec<Face>,
    pub panes: Vec<Pane>,
    pub kit: Vec<KitPane>,
    /// Repaint every pane of this building on the next refresh (its art was
    /// just re-drawn and the panes' recorded state cannot be trusted).
    pub force: bool,
    /// Did the sim send lights for this building? (The horizon belt has no lots,
    /// so it gets a synthetic plan instead.)
    pub planned: bool,
}

/// Every building's windows, keyed by lot anchor.
#[derive(Default, Debug)]
pub struct Field {
    pub objs: Vec<ObjWin>,
    index: std::collections::HashMap<u32, u32>,
    /// B72: panes actually switched since the scene was built (tools).
    pub switches: u64,
}

impl Field {
    /// B84: the bytes this field holds (its buildings, their faces, panes and
    /// kit panes, and the key index).
    pub fn bytes(&self) -> usize {
        let mut n = self.objs.capacity() * std::mem::size_of::<ObjWin>();
        n += self.index.capacity() * (std::mem::size_of::<u32>() * 2 + 1);
        for o in &self.objs {
            n += o.faces.capacity() * std::mem::size_of::<Face>()
                + o.panes.capacity() * std::mem::size_of::<Pane>()
                + o.kit.capacity() * std::mem::size_of::<KitPane>();
            for p in &o.panes {
                n += p.spill.capacity() * std::mem::size_of::<(u32, Color)>();
            }
            for k in &o.kit {
                n += k.px.capacity() * std::mem::size_of::<(i16, i16)>()
                    + k.spill.capacity() * std::mem::size_of::<(u32, Color)>();
            }
        }
        n
    }

    pub fn clear(&mut self) {
        self.objs.clear();
        self.index.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.objs.is_empty()
    }

    fn slot_for(&mut self, key: u32) -> usize {
        if let Some(&i) = self.index.get(&key) {
            return i as usize;
        }
        let i = self.objs.len() as u32;
        self.objs.push(ObjWin {
            key,
            ..Default::default()
        });
        self.index.insert(key, i);
        i as usize
    }

    /// Forget one building's windows (its art is about to be redrawn).
    pub fn forget(&mut self, key: u32) {
        let Some(&i) = self.index.get(&key) else {
            return;
        };
        let o = &mut self.objs[i as usize];
        o.faces.clear();
        o.panes.clear();
        o.kit.clear();
        o.force = true;
    }

    pub fn lights(&self, key: u32) -> u64 {
        self.index
            .get(&key)
            .map(|&i| self.objs[i as usize].lights)
            .unwrap_or(0)
    }

    /// Add the faces a redraw recorded, with one pane per window.
    ///
    /// The slot is the pane's **position in the face's grid**, not a hash of it:
    /// panes are walked row by row, so consecutive slots are a row of windows —
    /// a storey. The sim's offices and flats let whole floors, and this is what
    /// makes that read as a floor of light rather than confetti.
    pub fn add_faces(&mut self, faces: Vec<(u32, Face)>) {
        for (key, face) in faces {
            let i = self.slot_for(key);
            let o = &mut self.objs[i];
            let fi = o.faces.len() as u32;
            for (col, row) in face.panes() {
                let (c, r) = (col as i32, row as i32);
                let n = o.panes.len() as u32;
                o.panes.push(Pane {
                    face: fi,
                    col,
                    row,
                    slot: (n % SLOTS) as u8,
                    at: turn_on(&face, c, r),
                    lit: false,
                    spill: Vec::new(),
                });
            }
            o.faces.push(face);
        }
    }

    /// Record one kit pane against its building.
    pub fn add_kit(&mut self, key: u32, p: KitPane) {
        let i = self.slot_for(key);
        self.objs[i].kit.push(p);
    }

    /// Set a building's lights; true when they changed (a building that was
    /// running on the belt's synthetic plan counts as changed even when the sim
    /// says "nobody in": it has to stop pretending to be lit).
    pub fn set_lights(&mut self, key: u32, lights: u64) -> bool {
        let i = self.slot_for(key);
        let o = &mut self.objs[i];
        let changed = !o.planned || o.lights != lights;
        o.planned = true;
        o.lights = lights;
        changed
    }

    /// Number of switchable panes (tools, handbacks).
    /// One more pane switched (a light on or off).
    pub fn count_switch(&mut self) {
        self.switches += 1;
    }

    pub fn pane_count(&self) -> usize {
        self.objs
            .iter()
            .map(|o| o.panes.len() + o.kit.len())
            .sum()
    }

    /// Pixels a pane's cell covers, for the handbacks' "cheap" claim.
    pub fn lit_count(&self) -> usize {
        self.objs
            .iter()
            .map(|o| {
                o.panes.iter().filter(|p| p.lit).count() + o.kit.iter().filter(|p| p.lit).count()
            })
            .sum()
    }
}

/// B72 round 2: the horizon belt's own lights. The far towers are not lots and
/// have nobody in them, but a city's edge still glows in the evening and thins
/// out overnight — so each of their slots comes on at its own hour after work
/// and goes off at its own hour after that, a few panes at a time.
pub fn belt_lights(key: u32, hour: f32) -> u64 {
    let h = if hour < 12.0 { hour + 24.0 } else { hour };
    let mut bits = 0u64;
    for j in 0..SLOTS {
        let a = hash01(key ^ 0x51FE, j);
        let on = 16.4 + 4.2 * a;
        let len = 1.4 + 6.0 * hash01(key ^ 0x9A31, j);
        if (h as f64) >= on as f64 && (h as f64) < (on + len) as f64 {
            bits |= 1 << j;
        }
    }
    bits
}

fn hash01(a: u32, b: u32) -> f32 {
    let mut x = a ^ b.wrapping_mul(0x9E37_79B9);
    x ^= x >> 15;
    x = x.wrapping_mul(0x2545_F491);
    x ^= x >> 13;
    (x & 0xFFFF) as f32 / 65535.0
}

/// The sky's own darkness as a 0..1 switch-on scale (B58b's own curve).
pub fn night_of(glow: f32) -> f32 {
    ((glow - 0.10) / 0.55).clamp(0.0, 1.0)
}

/// A kit window pane lit: warm at the top, dimmer at the sill (a flat fill read
/// as a printed rectangle at 1x).
pub fn kit_lit_colour(warm: bool, night: f32, v: f32) -> Color {
    let base = if warm { pal::WINDOW_LIT } else { pal::WINDOW_LIT_COOL };
    let level = 0.55 + 0.45 * night;
    let shade = 1.06 - 0.16 * v;
    palette::with_alpha(palette::shade(base, level * shade), 0xFE)
}

/// The slot a kit pane answers to: its position in the sprite's pane list, which
/// is the order the art paints them in (row by row), so a row of windows shares
/// a slot — a storey, as the sim's own floors are.
pub fn kit_slot(index: u32, _key: u32) -> u8 {
    (index % SLOTS) as u8
}

/// When a kit pane comes on, in the same 0..255 scale as an engine pane.
pub fn kit_at(key: u32, x: i32, y: i32) -> u8 {
    let mut h = (x as u32).wrapping_mul(0x85EB_CA6B)
        ^ (y as u32).wrapping_mul(0x27D4_EB2F)
        ^ key.wrapping_mul(0x9E37_79B9);
    h ^= h >> 13;
    h = h.wrapping_mul(0x2545_F491);
    h ^= h >> 15;
    ((h >> 8) & 0xFF) as u8
}
