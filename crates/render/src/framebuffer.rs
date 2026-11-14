//! Framebuffer, clipping, sprite blits and polygon filling.
//!
//! Everything draws through a clip rectangle so that a cached layer can be
//! partially re-rendered (dirty-rect rebuilds) without any per-draw special
//! cases: redraw *all* geometry, clipped to the damaged rectangle.

use crate::palette::{self, Color, TRANSPARENT};

/// Half-open rectangle: `x0..x1`, `y0..y1`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

impl Rect {
    pub const EMPTY: Rect = Rect {
        x0: 0,
        y0: 0,
        x1: 0,
        y1: 0,
    };

    pub const fn new(x0: i32, y0: i32, x1: i32, y1: i32) -> Self {
        Self { x0, y0, x1, y1 }
    }

    pub fn from_size(w: usize, h: usize) -> Self {
        Self::new(0, 0, w as i32, h as i32)
    }

    /// Bounding box of a point cloud.
    pub fn from_points(pts: &[(f32, f32)]) -> Self {
        if pts.is_empty() {
            return Self::EMPTY;
        }
        let mut r = Self::new(i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for &(x, y) in pts {
            r.x0 = r.x0.min(x.floor() as i32);
            r.y0 = r.y0.min(y.floor() as i32);
            r.x1 = r.x1.max(x.ceil() as i32);
            r.y1 = r.y1.max(y.ceil() as i32);
        }
        r
    }

    pub fn is_empty(&self) -> bool {
        self.x1 <= self.x0 || self.y1 <= self.y0
    }

    pub fn width(&self) -> i32 {
        (self.x1 - self.x0).max(0)
    }

    pub fn height(&self) -> i32 {
        (self.y1 - self.y0).max(0)
    }

    pub fn area(&self) -> i64 {
        self.width() as i64 * self.height() as i64
    }

    pub fn expanded(&self, n: i32) -> Self {
        Self::new(self.x0 - n, self.y0 - n, self.x1 + n, self.y1 + n)
    }

    pub fn intersect(&self, o: &Rect) -> Rect {
        Rect::new(
            self.x0.max(o.x0),
            self.y0.max(o.y0),
            self.x1.min(o.x1),
            self.y1.min(o.y1),
        )
    }

    pub fn intersects(&self, o: &Rect) -> bool {
        !self.intersect(o).is_empty()
    }

    pub fn union(&self, o: &Rect) -> Rect {
        Rect::new(
            self.x0.min(o.x0),
            self.y0.min(o.y0),
            self.x1.max(o.x1),
            self.y1.max(o.y1),
        )
    }
}

/// Small 1-bit-alpha sprite, generated in code.
#[derive(Clone, Debug)]
pub struct Sprite {
    pub w: usize,
    pub h: usize,
    pub px: Vec<Color>,
    /// Where the sprite's ground point sits inside it: sprites are blitted at
    /// `(x - ax, y - ay)`. Defaults to the bottom centre.
    pub ax: i32,
    pub ay: i32,
}

impl Sprite {
    pub fn new(w: usize, h: usize) -> Self {
        Self {
            w,
            h,
            px: vec![TRANSPARENT; w * h],
            ax: w as i32 / 2,
            ay: h as i32,
        }
    }

    /// B61f: the bytes this sprite's pixels hold.
    pub fn bytes(&self) -> usize {
        self.px.capacity() * std::mem::size_of::<Color>()
    }

    /// Move the ground point (the pixel the sprite is anchored on).
    pub fn set_anchor(&mut self, ax: i32, ay: i32) {
        self.ax = ax;
        self.ay = ay;
    }

    pub fn set(&mut self, x: usize, y: usize, c: Color) {
        if x < self.w && y < self.h {
            self.px[y * self.w + x] = c;
        }
    }

    /// Fill a rectangle inside the sprite (used by composite sprites, B49's
    /// horse and cart).
    pub fn set_rect(&mut self, x: i32, y: i32, w: i32, h: i32, c: Color) {
        for yy in y..y + h {
            for xx in x..x + w {
                if xx >= 0 && yy >= 0 {
                    self.set(xx as usize, yy as usize, c);
                }
            }
        }
    }

    /// Copy another sprite in at `(dx, dy)` (its own top-left), keeping this
    /// sprite's transparent pixels.
    pub fn blit(&mut self, src: &Sprite, dx: i32, dy: i32) {
        for sy in 0..src.h as i32 {
            for sx in 0..src.w as i32 {
                let c = src.get(sx as usize, sy as usize);
                if c == TRANSPARENT {
                    continue;
                }
                let (x, y) = (dx + sx, dy + sy);
                if x >= 0 && y >= 0 {
                    self.set(x as usize, y as usize, c);
                }
            }
        }
    }

    pub fn get(&self, x: usize, y: usize) -> Color {
        if x < self.w && y < self.h {
            self.px[y * self.w + x]
        } else {
            TRANSPARENT
        }
    }
}

/// A pixel buffer in `0xAARRGGBB`, with a clip rectangle and an optional
/// per-pixel depth plane.
///
/// The depth plane stores a "painter key" (see [`crate::scene`]) for every pixel
/// that something was drawn into. Actors test against it so they can disappear
/// behind buildings and hills (P4).
#[derive(Clone)]
pub struct Framebuffer {
    pub w: usize,
    pub h: usize,
    px: Vec<Color>,
    clip: Rect,
    depth: Option<Vec<u16>>,
    depth_key: u16,
    /// Where the buffer's top-left sits in drawing coordinates (0,0 for every
    /// frame; the sway layer's small scratch borrows the frame's coordinates).
    origin: (i32, i32),
    /// While non-zero, every pixel written is tagged with this alpha value (see
    /// [`TAG_PROP`] / [`TAG_BUILT`]): something standing on the ground, which the
    /// horizon haze must never dissolve (B42b). Cleared again after the item.
    pub solid: u8,
    /// B72: the window faces drawn into this buffer, with the object each
    /// belongs to. Only the static layer collects them (sprites are not
    /// framebuffers); the scene takes them after the rebuild.
    pub windows: Vec<(u32, crate::windows::Face)>,
    /// B72: the kit-sprite window panes blitted here (sprite panes, already in
    /// screen coordinates).
    pub kit_panes: Vec<(u32, crate::windows::KitPane)>,
    /// B76 r2: record the faces and panes (true) or not. A paint-over static
    /// pass draws the same walls in the same places, so its records would be a
    /// duplicate set to be paid for and thrown away (8 ms of the frame budget).
    pub record_windows: bool,
    /// B100: the painter diagonal a column slice of a city object paints at,
    /// for engine art that stamps its own depth keys (`i32::MIN` = none, use
    /// the object's far corner).
    slice_diag: i32,
}

/// Tag for a *prop* (a tree, a bush, a stone): never hazed, and allowed to rise
/// past the horizon so a back-row canopy is not cut off.
pub const TAG_PROP: u8 = 0xFD;

/// Tag for something the city *built* (a wall, a roof, a lane, a lamp post):
/// never hazed either, but hidden once it is beyond the horizon, so a far tower
/// cannot float in the sky band on a patch of masked-away ground.
pub const TAG_BUILT: u8 = 0xFC;

/// A tube of the old name: either tag.
pub const SOLID_ALPHA: u8 = TAG_PROP;

impl Framebuffer {
    pub fn new(w: usize, h: usize) -> Self {
        Self {
            w,
            h,
            px: vec![TRANSPARENT; w * h],
            clip: Rect::from_size(w, h),
            depth: None,
            depth_key: 0,
            slice_diag: i32::MIN,
            origin: (0, 0),
            solid: 0,
            windows: Vec::new(),
            kit_panes: Vec::new(),
            record_windows: true,
        }
    }

    /// B72: a wall face with windows was just drawn here — keep it, and the
    /// object it belongs to, for the scene's window pass.
    pub fn push_face(&mut self, obj: u32, face: crate::windows::Face) {
        if self.record_windows {
            self.windows.push((obj, face));
        }
    }

    /// Take the recorded faces (the buffer stops collecting).
    pub fn take_faces(&mut self) -> Vec<(u32, crate::windows::Face)> {
        std::mem::take(&mut self.windows)
    }

    /// The transparent pixel (fully clear), for comparisons.
    pub const fn transparent() -> Color {
        TRANSPARENT
    }

    /// B72: a kit sprite's window panes were just blitted here. `panes` are in
    /// the sprite's own pixel space; `(ox, oy)` is where its pixel (0, 0) sits
    /// on screen.
    #[inline]
    pub fn push_kit_panes(
        &mut self,
        key: u32,
        panes: &[crate::scene::KitSpritePane],
        ox: i32,
        oy: i32,
        spr: u32,
    ) {
        if !self.record_windows {
            return;
        }
        for (index, p) in panes.iter().enumerate() {
            // The pane's own first pixel locates it for its switch-on time; its
            // place in the list gives its floor.
            let (px, py) = p.px.first().copied().unwrap_or((0, 0));
            self.kit_panes.push((
                key,
                crate::windows::KitPane {
                    spr,
                    ox: ox as i16,
                    oy: oy as i16,
                    px: p.px.clone(),
                    warm: p.warm,
                    slot: crate::windows::kit_slot(index as u32, key),
                    at: crate::windows::kit_at(key, ox + px as i32, oy + py as i32),
                    lit: false,
                    spill: Vec::new(),
                },
            ));
        }
    }

    /// Take the recorded kit panes.
    pub fn take_kit_panes(&mut self) -> Vec<(u32, crate::windows::KitPane)> {
        std::mem::take(&mut self.kit_panes)
    }

    /// Draw as if the buffer's top-left were `(x, y)` (B69b). Clip and bounds
    /// follow.
    pub fn set_origin(&mut self, x: i32, y: i32) {
        self.origin = (x, y);
        self.clip = self.bounds();
    }

    /// A framebuffer that also records a painter key per pixel.
    pub fn with_depth(w: usize, h: usize) -> Self {
        let mut fb = Self::new(w, h);
        fb.depth = Some(vec![0; w * h]);
        fb
    }

    /// Key stamped into the depth plane by every subsequent draw.
    #[inline]
    pub fn set_slice_diag(&mut self, d: Option<i32>) {
        self.slice_diag = d.unwrap_or(i32::MIN);
    }

    /// The diagonal engine art should key itself with: the slice's, if a
    /// column slice is being painted, else `own`.
    #[inline]
    pub fn slice_diag_or(&self, own: i32) -> i32 {
        if self.slice_diag == i32::MIN { own } else { self.slice_diag }
    }

    #[inline]
    pub fn set_depth_key(&mut self, key: u16) {
        self.depth_key = key;
    }

    #[inline]
    pub fn depth_key(&self) -> u16 {
        self.depth_key
    }

    #[inline]
    pub fn depth(&self) -> Option<&[u16]> {
        self.depth.as_deref()
    }

    #[inline]
    pub fn depth_mut(&mut self) -> Option<&mut [u16]> {
        self.depth.as_mut().map(|d| d.as_mut_slice())
    }

    /// Reset the depth plane to `key` over `r` (used by dirty-rect repaints).
    pub fn clear_depth_rect(&mut self, r: Rect, key: u16) {
        let r = r.intersect(&self.bounds());
        if r.is_empty() {
            return;
        }
        let w = self.w;
        let n = r.width() as usize;
        if let Some(d) = &mut self.depth {
            for y in r.y0..r.y1 {
                let i = y as usize * w + r.x0 as usize;
                d[i..i + n].fill(key);
            }
        }
    }

    /// B61f: the bytes this buffer holds (colour plane + depth plane), so the
    /// memory report names the buffers that are big rather than guessing.
    pub fn bytes(&self) -> usize {
        self.colour_bytes() + self.depth_bytes()
    }

    /// B84: the colour plane alone, so a report can tell a buffer's two planes
    /// apart (they are optimised differently).
    pub fn colour_bytes(&self) -> usize {
        self.px.capacity() * std::mem::size_of::<Color>()
    }

    /// B84: the depth plane alone (0 when the buffer has none).
    pub fn depth_bytes(&self) -> usize {
        self.depth
            .as_ref()
            .map_or(0, |d| d.capacity() * std::mem::size_of::<u16>())
    }

    #[inline]
    pub fn px(&self) -> &[Color] {
        &self.px
    }

    #[inline]
    pub fn px_mut(&mut self) -> &mut [Color] {
        &mut self.px
    }

    #[inline]
    pub fn index(&self, x: i32, y: i32) -> usize {
        (y - self.origin.1) as usize * self.w + (x - self.origin.0) as usize
    }

    pub fn bounds(&self) -> Rect {
        let (ox, oy) = self.origin;
        Rect::new(ox, oy, ox + self.w as i32, oy + self.h as i32)
    }

    #[inline]
    pub fn clip(&self) -> Rect {
        self.clip
    }

    /// Restrict all further drawing to `r` (intersected with the buffer).
    pub fn set_clip(&mut self, r: Rect) {
        self.clip = r.intersect(&self.bounds());
    }

    pub fn reset_clip(&mut self) {
        self.clip = self.bounds();
    }

    pub fn clear(&mut self, c: Color) {
        let clip = self.clip;
        self.clear_rect(clip, c);
    }

    pub fn clear_rect(&mut self, r: Rect, c: Color) {
        let r = r.intersect(&self.clip);
        if r.is_empty() {
            return;
        }
        for y in r.y0..r.y1 {
            let i = self.index(r.x0, y);
            self.px[i..i + r.width() as usize].fill(c);
        }
        // Cleared pixels hold no depth: painter keys are rewritten by whatever
        // is drawn into the rectangle afterwards.
        self.clear_depth_rect(r, 0);
    }

    /// Overwrite the whole buffer (dimensions must match).
    pub fn copy_from(&mut self, src: &Framebuffer) {
        debug_assert_eq!((self.w, self.h), (src.w, src.h));
        self.px.copy_from_slice(&src.px);
    }

    /// Source-over blit of `src`, skipping pixels that are exactly transparent.
    /// Copy only the clip rectangle from `src` (the partial-present path, B30).
    pub fn copy_from_clip(&mut self, src: &Framebuffer) {
        debug_assert_eq!((self.w, self.h), (src.w, src.h));
        let c = self.clip.intersect(&Rect::from_size(src.w, src.h));
        if c.is_empty() {
            return;
        }
        for y in c.y0.max(0)..c.y1.min(self.h as i32) {
            let a = y as usize * src.w + c.x0.max(0) as usize;
            let b = y as usize * src.w + c.x1.min(src.w as i32) as usize;
            self.px[a..b].copy_from_slice(&src.px[a..b]);
        }
    }

    /// Copy the sky band from a sky-only buffer (B34: the sky is stored only as
    /// tall as the band, because the static layer is opaque below it).
    /// B87: copy the pixels of `r` (clamped to both buffers) from a same-size
    /// buffer, ignoring the clip. The cached base plane goes back this way.
    pub fn copy_rect_from(&mut self, src: &Framebuffer, r: Rect) {
        debug_assert_eq!((src.w, src.h), (self.w, self.h));
        let r = r.intersect(&self.bounds()).intersect(&src.bounds());
        if r.is_empty() {
            return;
        }
        let (x0, x1) = (r.x0 as usize, r.x1 as usize);
        for y in r.y0 as usize..r.y1 as usize {
            let row = y * self.w;
            self.px[row + x0..row + x1].copy_from_slice(&src.px[row + x0..row + x1]);
        }
    }

    pub fn copy_sky(&mut self, sky: &Framebuffer) {
        let clip = self.clip;
        if clip.is_empty() || sky.h == 0 {
            return;
        }
        debug_assert_eq!(sky.w, self.w);
        let y1 = (clip.y1 as usize).min(sky.h).min(self.h);
        for y in (clip.y0.max(0) as usize)..y1 {
            let row = y * self.w;
            self.px[row + clip.x0.max(0) as usize..row + clip.x1.min(self.w as i32) as usize]
                .copy_from_slice(
                    &sky.px[row + clip.x0.max(0) as usize..row + clip.x1.min(self.w as i32) as usize],
                );
        }
    }

    /// Source-over blit of `src` with the ambient tint applied on the way (B34:
    /// the tinted copy of the static layer is gone). Lit windows are emissive
    /// and keep their colour at night (D25).
    pub fn blit_fb_tinted(&mut self, src: &Framebuffer, lut: &crate::light::Lut) {
        debug_assert_eq!((self.w, self.h), (src.w, src.h));
        let clip = self.clip;
        if clip.is_empty() {
            return;
        }
        for y in clip.y0..clip.y1 {
            let row = y as usize * self.w;
            for x in clip.x0..clip.x1 {
                let s = src.px[row + x as usize];
                if s == TRANSPARENT {
                    continue;
                }
                self.px[row + x as usize] = if s == crate::palette::pal::WINDOW_LIT {
                    s
                } else {
                    crate::light::apply(s, lut)
                };
            }
        }
    }

    /// Copy the static layer through the clip: transparent source pixels are
    /// left alone.
    ///
    /// D71: a row with no transparent pixel (the ordinary case — the static
    /// layer is opaque over the land) is one `copy_from_slice`, not a
    /// per-pixel test. At 7040x1440 a damage-limited frame copies ~350k px a
    /// frame, and this was the single biggest per-pixel loop in the composite.
    pub fn blit_fb(&mut self, src: &Framebuffer) {
        debug_assert_eq!((self.w, self.h), (src.w, src.h));
        let clip = self.clip.intersect(&Rect::from_size(self.w, self.h));
        if clip.is_empty() {
            return;
        }
        let (x0, x1) = (clip.x0.max(0) as usize, clip.x1.min(self.w as i32) as usize);
        if x1 <= x0 {
            return;
        }
        for y in clip.y0.max(0)..clip.y1.min(self.h as i32) {
            let row = y as usize * self.w;
            let a = row + x0;
            let b = row + x1;
            let s = &src.px[a..b];
            if s.iter().all(|&p| p != TRANSPARENT) {
                self.px[a..b].copy_from_slice(s);
                continue;
            }
            for x in x0..x1 {
                let s = src.px[row + x];
                if s != TRANSPARENT {
                    self.px[row + x] = s;
                }
            }
        }
    }

    /// `blit_fb`, but a transparent source pixel below `sky_h` takes the haze
    /// colour instead of leaving the old frame content (D71). This used to be
    /// two full passes over the damage (`fill_far_floor`, then the blit); they
    /// are the same pass now, and the haze only exists where the world does
    /// not reach — which is what it was for.
    /// B84: `from` is the first row of `src` that can hold anything (the
    /// static layer's land line). Rows above it are sky, which `copy_sky` has
    /// already laid down, so they are skipped instead of read — that is what
    /// keeps the unused top of the layer out of the process's resident set.
    pub fn blit_fb_hazed(
        &mut self,
        src: &Framebuffer,
        haze: &[Color],
        sky_h: i32,
        opaque_rows: &[bool],
        from: i32,
    ) {
        debug_assert_eq!((self.w, self.h), (src.w, src.h));
        let clip = self.clip.intersect(&Rect::from_size(self.w, self.h));
        if clip.is_empty() {
            return;
        }
        let (x0, x1) = (clip.x0.max(0) as usize, clip.x1.min(self.w as i32) as usize);
        if x1 <= x0 {
            return;
        }
        let row_wide = haze.len() == self.w;
        let flat = haze.first().copied().unwrap_or(TRANSPARENT);
        for y in clip.y0.max(from).max(0)..clip.y1.min(self.h as i32) {
            let row = y as usize * self.w;
            let a = row + x0;
            let b = row + x1;
            let s = &src.px[a..b];
            if opaque_rows.get(y as usize).copied().unwrap_or(false)
                || s.iter().all(|&p| p != TRANSPARENT)
            {
                self.px[a..b].copy_from_slice(s);
                continue;
            }
            let below = y >= sky_h;
            // B90: the layer reaches far up into the sky band now (a tower may
            // rise past the horizon), and above the land line most rows are
            // empty: one vectorised scan instead of a per-pixel loop.
            if !below && s.iter().all(|&p| p == TRANSPARENT) {
                continue;
            }
            for x in x0..x1 {
                let p = src.px[row + x];
                if p != TRANSPARENT {
                    self.px[row + x] = p;
                } else if below {
                    self.px[row + x] = if row_wide { haze[x] } else { flat };
                }
            }
        }
    }

    /// Apply the cast-shadow plane (B60) to what is already in the frame, inside
    /// the clip. A row whose shadow bytes are all zero (most of them) costs one
    /// vectorised scan; only shadowed pixels pay the table lookups.
    pub fn shade_rows(&mut self, shadow: &[u8], lut: &crate::light::ShadowLut) {
        if shadow.len() != self.w * self.h {
            return;
        }
        let clip = self.clip.intersect(&Rect::from_size(self.w, self.h));
        if clip.is_empty() {
            return;
        }
        let (x0, x1) = (clip.x0.max(0) as usize, clip.x1.min(self.w as i32) as usize);
        for y in clip.y0.max(0)..clip.y1.min(self.h as i32) {
            let row = y as usize * self.w;
            let m = &shadow[row + x0..row + x1];
            if m.iter().all(|&v| v == 0) {
                continue;
            }
            for (k, &v) in m.iter().enumerate() {
                if v != 0 {
                    let i = row + x0 + k;
                    self.px[i] = lut.apply(self.px[i], v);
                }
            }
        }
    }

    /// Blit `src` with the frame's cast-shadow plane applied on the way (B60).
    /// `shadow` holds one byte per pixel (0 = full sun, 255 = full shadow) and
    /// `lut` the per-channel multiply for every strength, so a shadowed pixel
    /// costs one byte load and three table reads.
    ///
    /// B57's cloud shadows are a *second* multiply of the same kind, applied
    /// after the layer is down (see `Scene::composite_rect`), because they move
    /// every frame while the cast plane only re-bakes every minute.
    pub fn blit_fb_shadow(
        &mut self,
        src: &Framebuffer,
        shadow: &[u8],
        lut: &crate::light::ShadowLut,
    ) {
        debug_assert_eq!((self.w, self.h), (src.w, src.h));
        let clip = self.clip;
        if clip.is_empty() {
            return;
        }
        let shadowed = shadow.len() == self.w * self.h;
        for y in clip.y0..clip.y1 {
            let row = y as usize * self.w;
            for x in clip.x0..clip.x1 {
                let i = row + x as usize;
                let s = src.px[i];
                if s == TRANSPARENT {
                    continue;
                }
                self.px[i] = if shadowed {
                    lut.apply(s, shadow[i])
                } else {
                    s
                };
            }
        }
    }

    #[inline]
    /// `PIX_WATCH=x,y`: report every write to that screen pixel, with the call
    /// stack, so a stray mark can be traced to the code that drew it (B76 r3
    /// found the autumn leaf columns this way). Silent unless the variable is
    /// set; one cached lookup per write.
    #[inline]
    fn pix_watch(x: i32, y: i32, c: Color, what: &str) {
        static WATCH: std::sync::OnceLock<Option<(i32, i32)>> = std::sync::OnceLock::new();
        let w = WATCH.get_or_init(|| {
            std::env::var("PIX_WATCH").ok().and_then(|v| {
                let p: Vec<i32> = v.split(',').filter_map(|s| s.parse().ok()).collect();
                (p.len() == 2).then(|| (p[0], p[1]))
            })
        });
        if *w == Some((x, y)) {
            eprintln!(
                "PIX_WATCH {what} {:06x} from:\n{}",
                c & 0xFFFFFF,
                std::backtrace::Backtrace::force_capture()
            );
        }
    }

    pub fn put(&mut self, x: i32, y: i32, c: Color) {
        Self::pix_watch(x, y, c, "put");
        if x >= self.clip.x0 && x < self.clip.x1 && y >= self.clip.y0 && y < self.clip.y1 {
            let i = self.index(x, y);
            self.px[i] = self.tag(c);
            if let Some(d) = &mut self.depth {
                d[i] = self.depth_key;
            }
        }
    }

    /// One-pixel Bresenham line (used for water/bank edges and outlines).
    pub fn line(&mut self, a: (f32, f32), b: (f32, f32), c: Color) {
        static W: std::sync::OnceLock<Option<(i32, i32)>> = std::sync::OnceLock::new();
        if let Some(p) = *W.get_or_init(|| {
            std::env::var("PIX_WATCH").ok().and_then(|v| {
                let q: Vec<i32> = v.split(',').filter_map(|s| s.parse().ok()).collect();
                (q.len() == 2).then(|| (q[0], q[1]))
            })
        }) {
            let (x0, y0) = (a.0.min(b.0) as i32, a.1.min(b.1) as i32);
            let (x1, y1) = (a.0.max(b.0) as i32, a.1.max(b.1) as i32);
            if p.0 >= x0 - 1 && p.0 <= x1 + 1 && p.1 >= y0 - 1 && p.1 <= y1 + 1 {
                Self::pix_watch(p.0, p.1, c, "line");
            }
        }
        let (x0, y0) = (a.0.round() as i32, a.1.round() as i32);
        let (x1, y1) = (b.0.round() as i32, b.1.round() as i32);
        let dx = (x1 - x0).abs();
        let dy = -(y1 - y0).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;
        let (mut x, mut y) = (x0, y0);
        loop {
            self.put(x, y, c);
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

    /// A Bresenham line of alpha-blended pixels (coverage `cov`, 0..=1): the
    /// water's glints, which fade in and out over whatever the sea is.
    pub fn line_blended(&mut self, a: (f32, f32), b: (f32, f32), c: Color, cov: f32) {
        let (x0, y0) = (a.0.round() as i32, a.1.round() as i32);
        let (x1, y1) = (b.0.round() as i32, b.1.round() as i32);
        let dx = (x1 - x0).abs();
        let dy = -(y1 - y0).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;
        let (mut x, mut y) = (x0, y0);
        loop {
            self.put_blended(x, y, c, cov);
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

    /// Alpha-blend one pixel with coverage `cov` (0..=1), no allocation.
    #[inline]
    pub fn put_blended(&mut self, x: i32, y: i32, c: Color, cov: f32) {
        if x >= self.clip.x0 && x < self.clip.x1 && y >= self.clip.y0 && y < self.clip.y1 {
            let i = self.index(x, y);
            self.px[i] = palette::blend(self.px[i], c, cov);
        }
    }

    /// Additive blend one pixel: `dst + src * cov`, clipped, no depth, and the
    /// surface's own alpha kept. This is *light falling on a surface* (B73r2:
    /// a sign's coloured wash on the wall and the pavement), not paint over it:
    /// a pool on dark asphalt brightens without bleaching to the light's colour.
    #[inline]
    pub fn add_blended(&mut self, x: i32, y: i32, c: Color, cov: f32) {
        if cov <= 0.0 {
            return;
        }
        if x < self.clip.x0 || x >= self.clip.x1 || y < self.clip.y0 || y >= self.clip.y1 {
            return;
        }
        let i = self.index(x, y);
        let d = self.px[i];
        let f = |a: u8, b: u8| -> u8 {
            let v = a as f32 + b as f32 * cov;
            if v >= 255.0 {
                255
            } else {
                v as u8
            }
        };
        self.px[i] = palette::with_alpha(
            palette::rgb(
                f(palette::r(d), palette::r(c)),
                f(palette::g(d), palette::g(c)),
                f(palette::b(d), palette::b(c)),
            ),
            palette::alpha(d),
        );
    }

    /// Alpha-blend a whole rectangle (coverage 0..=1, no allocation) — the
    /// night sky's dabs/smudges and the meteor streaks. Clipped, no depth.
    pub fn fill_blended(&mut self, x: i32, y: i32, w: i32, h: i32, c: Color, cov: f32) {
        if cov <= 0.0 {
            return;
        }
        let r = Rect::new(x, y, x + w, y + h).intersect(&self.clip);
        for yy in r.y0..r.y1 {
            for xx in r.x0..r.x1 {
                let i = self.index(xx, yy);
                self.px[i] = palette::blend(self.px[i], c, cov);
            }
        }
    }

    pub fn fill_rect(&mut self, x: i32, y: i32, w: i32, h: i32, c: Color) {
        static W: std::sync::OnceLock<Option<(i32, i32)>> = std::sync::OnceLock::new();
        if let Some(p) = *W.get_or_init(|| {
            std::env::var("PIX_WATCH").ok().and_then(|v| {
                let q: Vec<i32> = v.split(',').filter_map(|s| s.parse().ok()).collect();
                (q.len() == 2).then(|| (q[0], q[1]))
            })
        }) {
            if p.0 >= x && p.0 < x + w && p.1 >= y && p.1 < y + h {
                Self::pix_watch(p.0, p.1, c, "fill_rect");
            }
        }
        let r = Rect::new(x, y, x + w, y + h).intersect(&self.clip);
        if r.is_empty() {
            return;
        }
        for yy in r.y0..r.y1 {
            self.fill_span(yy, r.x0, r.x1, c);
        }
    }

    /// Apply the solid tag to a colour about to be written (B42b).
    #[inline]
    pub fn tag(&self, c: Color) -> Color {
        if self.solid != 0 && c != TRANSPARENT && (c >> 24) == 0xFF {
            (c & 0x00FF_FFFF) | ((self.solid as Color) << 24)
        } else {
            c
        }
    }

    #[inline]
    fn fill_span(&mut self, y: i32, x0: i32, x1: i32, c: Color) {
        let a = x0.max(self.clip.x0);
        let b = x1.min(self.clip.x1);
        if b <= a {
            return;
        }
        let i = self.index(a, y);
        let n = (b - a) as usize;
        let c = self.tag(c);
        self.px[i..i + n].fill(c);
        if let Some(d) = &mut self.depth {
            d[i..i + n].fill(self.depth_key);
        }
    }

    /// Even-odd scanline fill of a convex/simple polygon. No antialiasing:
    /// this is pixel art.
    pub fn fill_poly(&mut self, pts: &[(f32, f32)], c: Color) {
        if pts.len() < 3 {
            return;
        }
        let bbox = Rect::from_points(pts);
        let y0 = bbox.y0.max(self.clip.y0);
        let y1 = bbox.y1.min(self.clip.y1);
        if y1 <= y0 {
            return;
        }

        let mut xs = [0f32; 16];
        for y in y0..y1 {
            let yc = y as f32 + 0.5;
            let mut n = 0usize;
            for i in 0..pts.len() {
                let a = pts[i];
                let b = pts[(i + 1) % pts.len()];
                let crosses = (a.1 <= yc && b.1 > yc) || (b.1 <= yc && a.1 > yc);
                if crosses && n < xs.len() {
                    let t = (yc - a.1) / (b.1 - a.1);
                    xs[n] = a.0 + t * (b.0 - a.0);
                    n += 1;
                }
            }
            if n < 2 {
                continue;
            }
            xs[..n].sort_by(|p, q| p.partial_cmp(q).unwrap_or(std::cmp::Ordering::Equal));
            let mut i = 0;
            while i + 1 < n {
                // Pixel centre (x + 0.5) inside [xa, xb).
                let xa = (xs[i] - 0.5).ceil() as i32;
                let xb = (xs[i + 1] - 0.5).ceil() as i32;
                if xb > xa {
                    self.fill_span(y, xa, xb, c);
                }
                i += 2;
            }
        }
    }

    /// As [`Self::fill_poly`], but blended with coverage `cov` over what is
    /// already there — how a moving thing's shadow falls across the frame (B60).
    pub fn fill_poly_blended(&mut self, pts: &[(f32, f32)], c: Color, cov: f32) {
        if pts.len() < 3 || cov <= 0.0 {
            return;
        }
        let bbox = Rect::from_points(pts);
        let y0 = bbox.y0.max(self.clip.y0);
        let y1 = bbox.y1.min(self.clip.y1);
        if y1 <= y0 {
            return;
        }
        let mut xs = [0f32; 16];
        for y in y0..y1 {
            let yc = y as f32 + 0.5;
            let mut n = 0usize;
            for i in 0..pts.len() {
                let a = pts[i];
                let b = pts[(i + 1) % pts.len()];
                let crosses = (a.1 <= yc && b.1 > yc) || (b.1 <= yc && a.1 > yc);
                if crosses && n < xs.len() {
                    let t = (yc - a.1) / (b.1 - a.1);
                    xs[n] = a.0 + t * (b.0 - a.0);
                    n += 1;
                }
            }
            if n < 2 {
                continue;
            }
            xs[..n].sort_by(|p, q| p.partial_cmp(q).unwrap_or(std::cmp::Ordering::Equal));
            let mut i = 0;
            while i + 1 < n {
                let xa = (xs[i] - 0.5).ceil() as i32;
                let xb = (xs[i + 1] - 0.5).ceil() as i32;
                for x in xa.max(self.clip.x0)..xb.min(self.clip.x1) {
                    let k = self.index(x, y);
                    self.px[k] = palette::blend(self.px[k], c, cov);
                }
                i += 2;
            }
        }
    }

    /// Blit a 1-bit-alpha sprite with its top-left at `(x, y)`.
    pub fn blit_sprite(&mut self, s: &Sprite, x: i32, y: i32) {
        let r = Rect::new(x, y, x + s.w as i32, y + s.h as i32).intersect(&self.clip);
        if r.is_empty() {
            return;
        }
        for yy in r.y0..r.y1 {
            let srow = (yy - y) as usize * s.w;
            let drow = yy as usize * self.w;
            for xx in r.x0..r.x1 {
                let sc = s.px[srow + (xx - x) as usize];
                if sc != TRANSPARENT {
                    let i = drow + xx as usize;
                    self.px[i] = self.tag(sc);
                    if let Some(d) = &mut self.depth {
                        d[i] = self.depth_key;
                    }
                }
            }
        }
    }

    /// B76 round 3: the same blit, with snow lying on the surfaces the builder
    /// drew.
    ///
    /// Round 2 capped the top rows of the sprite, which on a tower is its crown:
    /// the crown's vertical faces went white and read as a pale pyramid floating
    /// over a bare grey roof. Now every pixel carries the surface it belongs to
    /// (`crate::snow`), so the load lies on the **roof plane** — evenly on a flat
    /// deck, thinner the steeper a slope is, with the ridge, the sills and the
    /// copings drawn as their own thin lines, and a chimney or an AC box poking
    /// through dark with snow on its top. Two extra passes deepen the load where
    /// it drifts against a parapet or a box, and let a pixel of snow hang past
    /// the roof's own edge so the eave has a lip.
    ///
    /// `heat` (0..1) is a roof that warms from inside: it thaws first, so the
    /// deck keeps less and shows wet patches. Runs only while the static layer
    /// is re-rasterised, never per frame.
    pub fn blit_sprite_flat_snow(
        &mut self,
        k: &crate::scene::KitSprite,
        x: i32,
        y: i32,
        snow: f32,
        heat: f32,
    ) {
        static TRACE0: std::sync::OnceLock<Option<(i32, i32)>> = std::sync::OnceLock::new();
        let trace0 = *TRACE0.get_or_init(|| {
            std::env::var("SNOW_TRACE").ok().and_then(|v| {
                let p: Vec<i32> = v.split(',').filter_map(|s| s.parse().ok()).collect();
                (p.len() == 2).then(|| (p[0], p[1]))
            })
        });
        if snow <= 0.04 || k.surf.is_empty() {
            if let Some(t) = trace0 {
                let (sx, sy) = (t.0 - x, t.1 - y);
                if sx >= 0
                    && sy >= 0
                    && (sx as usize) < k.w
                    && (sy as usize) < k.h
                    && k.px[sy as usize * k.w + sx as usize] != TRANSPARENT
                {
                    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
                    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    eprintln!(
                        "#{n} snow trace: sprite covers {t:?} but snow {snow:.2}, surf {} B for {} px",
                        k.surf.len(),
                        k.w * k.h
                    );
                }
            }
            return self.blit_sprite_flat(k, x, y);
        }
        use crate::snow;
        let r = Rect::new(x, y, x + k.w as i32, y + k.h as i32).intersect(&self.clip);
        if r.is_empty() {
            return;
        }
        // A debug switch: paint what the load touched in magenta, so a roof that
        // stays bare can be told from one that was simply never tagged.
        static TINT: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        let tint = *TINT.get_or_init(|| std::env::var("SNOW_TINT").is_ok());
        static TRACE: std::sync::OnceLock<Option<(i32, i32)>> = std::sync::OnceLock::new();
        let trace = *TRACE.get_or_init(|| {
            std::env::var("SNOW_TRACE").ok().and_then(|v| {
                let p: Vec<i32> = v.split(',').filter_map(|s| s.parse().ok()).collect();
                (p.len() == 2).then(|| (p[0], p[1]))
            })
        });
        let sy0 = (r.y0 - y).max(0) as usize;
        let sy1 = ((r.y1 - y) as usize).min(k.h);
        let lip = crate::palette::rgb(0xe8, 0xf1, 0xfa);
        let lip_on = snow > 0.3;
        // One pass: the load by surface, the drift against anything standing on
        // the deck, and the eave lip past the roof's own edge. A second pass over
        // the sprite would cost the sliced rebuild a millisecond of its budget.
        for sy in sy0..sy1 {
            let srow = sy * k.w;
            let drow = (y + sy as i32) as usize * self.w;
            for xx in r.x0..r.x1 {
                let sx = (xx - x) as usize;
                let c = k.px[srow + sx];
                if c == TRANSPARENT {
                    continue;
                }
                let tag = snow::get(&k.surf, srow + sx);
                if trace == Some((xx, y + sy as i32)) {
                    static N2: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
                    let n = N2.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    eprintln!(
                        "#{n} snow trace at {xx},{}: tag {tag} base {:06x} snow {snow:.2} heat {heat:.2}",
                        y + sy as i32,
                        c & 0xFFFFFF
                    );
                }
                let out = match snow::on_tag(tag, c, snow, heat, xx, y + sy as i32) {
                    Some(mut w) => {
                        if tint {
                            w = crate::palette::rgb(0xff, 0x00, 0xff);
                        }
                        w
                    }
                    None => c,
                };
                let i = drow + xx as usize;
                let mut painted = out;
                if tag == snow::FLAT {
                    // A drift: the load banks up where the deck meets a parapet,
                    // a stair head or a tank, and at the deck's own far rim.
                    let mut against = false;
                    for (dx, dy) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
                        let (nx, ny) = (sx as i32 + dx, sy as i32 + dy);
                        if nx < 0 || ny < 0 || nx as usize >= k.w || ny as usize >= k.h {
                            against = true;
                            break;
                        }
                        if snow::get(&k.surf, ny as usize * k.w + nx as usize) == snow::EQUIP_SIDE {
                            against = true;
                            break;
                        }
                    }
                    if against {
                        painted = snow::drift(out, snow, heat);
                    }
                } else if lip_on
                    && matches!(
                        tag,
                        snow::SLOPE_GENTLE
                            | snow::SLOPE_MEDIUM
                            | snow::SLOPE_STEEP
                            | snow::EQUIP_TOP
                            | snow::LEDGE
                    )
                {
                    // The eave lip: one pixel hangs past the roof's own edge,
                    // never two, and only where nothing else is drawn.
                    for (dx, dy) in [(-1i32, 0i32), (1, 0), (0, -1)] {
                        let (nsx, nsy) = (sx as i32 + dx, sy as i32 + dy);
                        let inside = nsx >= 0
                            && nsy >= 0
                            && (nsx as usize) < k.w
                            && (nsy as usize) < k.h
                            && k.px[nsy as usize * k.w + nsx as usize] != TRANSPARENT;
                        if inside {
                            continue;
                        }
                        let (nx, ny) = (xx + dx, y + sy as i32 + dy);
                        if nx < self.clip.x0
                            || nx >= self.clip.x1
                            || ny < self.clip.y0
                            || ny >= self.clip.y1
                        {
                            continue;
                        }
                        let j = ny as usize * self.w + nx as usize;
                        if j < self.px.len() && self.px[j] == TRANSPARENT {
                            self.px[j] = self.tag(lip);
                        }
                    }
                }
                self.px[i] = self.tag(painted);
                if let Some(dd) = &mut self.depth {
                    dd[i] = self.depth_key;
                }
            }
        }
    }

    /// Blit a flat pixel block (a pre-rendered building) with its top-left at
    /// `(x, y)`: transparent pixels are skipped, everything else takes the
    /// current depth key.
    pub fn blit_sprite_flat(&mut self, k: &crate::scene::KitSprite, x: i32, y: i32) {
        let r = Rect::new(x, y, x + k.w as i32, y + k.h as i32).intersect(&self.clip);
        if r.is_empty() {
            return;
        }
        for yy in r.y0..r.y1 {
            let srow = (yy - y) as usize * k.w;
            let drow = yy as usize * self.w;
            for xx in r.x0..r.x1 {
                let c = k.px[srow + (xx - x) as usize];
                if c != TRANSPARENT {
                    let i = drow + xx as usize;
                    // A kit pixel may carry the emissive flag (0xFE); `tag`
                    // leaves anything that is not plain opaque alone.
                    self.px[i] = self.tag(c);
                    if let Some(d) = &mut self.depth {
                        d[i] = self.depth_key;
                    }
                }
            }
        }
    }

    /// Blit a sprite, hiding each pixel that something nearer already wrote.
    /// `depth` is the static layer's plane; `key` is the actor's painter key.
    pub fn blit_sprite_depth(&mut self, s: &Sprite, x: i32, y: i32, key: u16, depth: &[u16]) {
        let r = Rect::new(x, y, x + s.w as i32, y + s.h as i32).intersect(&self.clip);
        if r.is_empty() {
            return;
        }
        for yy in r.y0..r.y1 {
            let srow = (yy - y) as usize * s.w;
            let drow = yy as usize * self.w;
            for xx in r.x0..r.x1 {
                let sc = s.px[srow + (xx - x) as usize];
                if sc != TRANSPARENT && depth[drow + xx as usize] <= key {
                    self.px[drow + xx as usize] = sc;
                }
            }
        }
    }

    /// Coarser variant: test one pixel (the sprite's foot) and draw or skip the
    /// whole sprite. Returns whether it was drawn.
    pub fn blit_sprite_anchor(
        &mut self,
        s: &Sprite,
        x: i32,
        y: i32,
        key: u16,
        depth: &[u16],
    ) -> bool {
        let ax = x + s.w as i32 / 2;
        let ay = y + s.h as i32 - 1;
        if ax >= 0
            && ay >= 0
            && (ax as usize) < self.w
            && (ay as usize) < self.h
            && depth[ay as usize * self.w + ax as usize] > key
        {
            return false;
        }
        self.blit_sprite(s, x, y);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::palette::rgb;

    #[test]
    fn poly_fills_expected_pixels() {
        let mut fb = Framebuffer::new(32, 16);
        fb.fill_poly(
            &[(0.0, 0.0), (16.0, 0.0), (16.0, 8.0), (0.0, 8.0)],
            rgb(1, 2, 3),
        );
        assert_eq!(fb.px()[0], rgb(1, 2, 3));
        assert_eq!(fb.px()[7 * 32 + 15], rgb(1, 2, 3));
        assert_eq!(fb.px()[16], TRANSPARENT);
        assert_eq!(fb.px()[8 * 32 + 15], TRANSPARENT);
    }

    #[test]
    fn clip_limits_drawing() {
        let mut fb = Framebuffer::new(16, 16);
        fb.set_clip(Rect::new(4, 4, 8, 8));
        fb.fill_rect(0, 0, 16, 16, rgb(9, 9, 9));
        assert_eq!(fb.px()[0], TRANSPARENT);
        assert_eq!(fb.px()[4 * 16 + 4], rgb(9, 9, 9));
        assert_eq!(fb.px()[8 * 16 + 8], TRANSPARENT);
    }

    #[test]
    fn sprite_blit_skips_transparent() {
        let mut fb = Framebuffer::new(4, 4);
        fb.clear(rgb(0, 0, 0));
        let mut s = Sprite::new(2, 2);
        s.set(1, 1, rgb(255, 0, 0));
        fb.blit_sprite(&s, 0, 0);
        assert_eq!(fb.px()[0], rgb(0, 0, 0));
        assert_eq!(fb.px()[5], rgb(255, 0, 0));
    }
}

#[cfg(test)]
mod b76_snow_tests {
    use super::*;
    use crate::snow;

    /// A sprite with a wall band and a flat deck on top, tagged the way the
    /// builder tags them (deck FLAT, walls NONE).
    fn flat_roof_block() -> crate::scene::KitSprite {
        let (w, h) = (40usize, 40usize);
        let wall = 0xFF7A5A44u32;
        let deck = 0xFF6A6E74u32;
        let mut px = vec![0u32; w * h];
        let mut tags = vec![snow::NONE; w * h];
        for y in 0..h {
            for x in 0..w {
                if y < 18 {
                    // The roof: a deck, with its far edge as a slope so the eave
                    // lip has something to hang from.
                    px[y * w + x] = deck;
                    tags[y * w + x] = if y == 0 { snow::SLOPE_GENTLE } else { snow::FLAT };
                } else {
                    px[y * w + x] = wall;
                }
            }
        }
        // A chimney stack standing in the deck: sides vertical, a cap on top.
        for y in 6..14 {
            for x in 18..22 {
                px[y * w + x] = 0xFF2a2a2e;
                tags[y * w + x] = if y < 8 { snow::EQUIP_TOP } else { snow::EQUIP_SIDE };
            }
        }
        crate::scene::KitSprite {
            w,
            h,
            ox: 20,
            oy: 40,
            px: std::sync::Arc::new(px),
            top: 20,
            foot: [0.0; 4],
            roof: None,
            panes: Vec::new(),
            surf: snow::pack(&tags),
        }
    }

    #[test]
    fn the_load_lies_on_the_deck_and_the_walls_keep_their_colour() {
        let k = flat_roof_block();
        let mut fb = Framebuffer::new(64, 64);
        fb.blit_sprite_flat_snow(&k, 4, 4, 1.0, 0.0);
        // A deck pixel well inside the roof (not next to the chimney).
        let deck = fb.px()[(4 + 14) * 64 + 4 + 8];
        assert_ne!(deck, 0xFF6A6E74, "the deck took no snow");
        let (r, g, b) = (palette::r(deck), palette::g(deck), palette::b(deck));
        assert!(r > 200 && g > 200 && b > 200, "the deck is not snowy: {r},{g},{b}");
        // A wall pixel keeps its colour.
        assert_eq!(fb.px()[(4 + 30) * 64 + 4 + 20], 0xFF7A5A44, "a wall was snowed");
    }

    #[test]
    fn a_chimney_pokes_out_dark_with_snow_on_its_cap() {
        let k = flat_roof_block();
        let mut fb = Framebuffer::new(64, 64);
        fb.blit_sprite_flat_snow(&k, 4, 4, 1.0, 0.0);
        // The stack's side (rows 9+) is dark; its cap (rows 6-7) is white.
        assert_eq!(fb.px()[(4 + 12) * 64 + 4 + 20], 0xFF2a2a2e, "the stack wore a hat");
        let cap = fb.px()[(4 + 7) * 64 + 4 + 20];
        assert_ne!(cap, 0xFF2a2a2e, "the cap took no snow");
    }

    #[test]
    fn no_snow_lands_outside_the_building() {
        // The load may hang one pixel past the roof's own edge (the eave lip),
        // never more, and never anywhere away from the sprite.
        let k = flat_roof_block();
        let mut fb = Framebuffer::new(64, 64);
        fb.blit_sprite_flat_snow(&k, 4, 4, 1.0, 0.0);
        let mut outside = 0;
        for y in 0..64i32 {
            for x in 0..64i32 {
                if fb.px()[(y * 64 + x) as usize] == TRANSPARENT {
                    continue;
                }
                let (sx, sy) = (x - 4, y - 4);
                if sx < 0 || sy < 0 || sx as usize >= k.w || sy as usize >= k.h {
                    outside += 1;
                    continue;
                }
                if k.px[sy as usize * k.w + sx as usize] == TRANSPARENT {
                    outside += 1;
                }
            }
        }
        assert!(outside > 0, "the eave should have a lip");
        assert!(outside <= 2 * 40, "snow spilled past the silhouette: {outside} px");
    }

    #[test]
    fn a_vertical_face_never_takes_snow() {
        let k = flat_roof_block();
        let mut fb = Framebuffer::new(64, 64);
        fb.blit_sprite_flat_snow(&k, 4, 4, 1.0, 0.0);
        // Every wall pixel of row 20..40 is unchanged.
        for y in 22..40 {
            assert_eq!(fb.px()[(4 + y) * 64 + 4 + 5], 0xFF7A5A44, "row {y} wall snowed");
        }
    }

    #[test]
    fn a_sprite_without_tags_takes_no_snow() {
        let mut k = flat_roof_block();
        k.surf.clear();
        let mut fb = Framebuffer::new(64, 64);
        fb.blit_sprite_flat_snow(&k, 4, 4, 1.0, 0.0);
        assert_eq!(fb.px()[(4 + 14) * 64 + 4 + 8], 0xFF6A6E74, "an untagged sprite was snowed");
    }
}
