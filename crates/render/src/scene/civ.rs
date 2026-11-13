//! B52: the scene's civic layer. The glue hands over one [`CivicLook`] whenever
//! the sim's civic state changes (a new election phase, a fire's stage, the
//! hour's street life); between those the layer only repaints what animates.

use super::*;
use crate::actors::{actor_key, static_key};
use crate::civic::{self, CivicLook, CivicProp, PropKind};

impl Scene {
    /// Replace the civic look. Returns true when the static layer was re-baked
    /// (the rally dress or a blackout's edge changed).
    pub fn set_civic(&mut self, look: CivicLook) -> bool {
        let stat_changed = self.civic.stat != look.stat;
        let same = self.civic.dynamic == look.dynamic;
        // The figures travel with the people (see `set_people`); keep the old
        // list until the next people pass replaces it.
        self.civic = look;
        if stat_changed {
            self.rebuild_static();
            self.retint_static();
            self.mark_full();
        }
        if !same {
            let mut now = self.civic_rects();
            // B66: the smog's veil covers the whole land band, so it is marked
            // when its thickness changes — not on every street prop that moves.
            let level = self.haze_level();
            if level != self.civic_haze {
                self.civic_haze = level;
                now.push(self.haze_rect());
            }
            let old = std::mem::take(&mut self.civic_drawn);
            for r in old.iter().chain(now.iter()) {
                self.mark_rect(*r);
            }
            self.civic_drawn = now;
        }
        stat_changed
    }

    /// Mark the boxes of everything that moves (called once per frame while
    /// [`CivicDyn::animated`]); a still civic layer costs one comparison.
    pub fn civic_tick(&mut self) {
        if !self.civic.dynamic.animated() {
            return;
        }
        // The haze is still air: only its changes (set_civic) damage it. The
        // chimney plumes drift a pixel in several eighths of a second, so they
        // are repainted eight times a second (their motion is smooth at that
        // rate); fires, engines and the like repaint every frame.
        let q = (self.civic_t * 8.0).floor() as i64;
        let stacks_due = q != self.civic_stack_q;
        self.civic_stack_q = q;
        let d = &self.civic.dynamic;
        let mut v: Vec<Rect> = Vec::new();
        for p in d.props.iter().filter(|p| {
            matches!(p.kind, PropKind::Sax | PropKind::Notes | PropKind::Engine | PropKind::Truck | PropKind::Cat | PropKind::Rod)
        }) {
            v.push(civic::prop_bounds(&self.cam, &self.map, p));
        }
        for f in &d.fires {
            v.push(civic::fire_bounds(&self.cam, f));
        }
        if stacks_due {
            for s in &d.stacks {
                v.push(civic::stack_bounds(&self.cam, s));
            }
        }
        for r in v {
            if !r.is_empty() {
                self.mark_rect(r);
            }
        }
    }

    /// Set the civic layer's animation clock (seconds since the app started).
    pub fn set_civic_clock(&mut self, t: f32) {
        self.civic_t = t;
    }

    /// Is the civic layer animating (so the wallpaper keeps drawing frames)?
    pub fn civic_live(&self) -> bool {
        self.civic.dynamic.animated()
    }

    /// Screen boxes of every dynamic civic item. The smog veil is not one of
    /// them (B66): it is even air over the whole land, marked by `set_civic`
    /// only when its thickness changes.
    fn civic_rects(&self) -> Vec<Rect> {
        let d = &self.civic.dynamic;
        let mut v: Vec<Rect> = Vec::with_capacity(d.props.len() + d.fires.len() + d.stacks.len() + 1);
        for p in &d.props {
            v.push(civic::prop_bounds(&self.cam, &self.map, p));
        }
        for f in &d.fires {
            v.push(civic::fire_bounds(&self.cam, f));
        }
        for s in &d.stacks {
            v.push(civic::stack_bounds(&self.cam, s));
        }
        v.retain(|r| !r.is_empty());
        v
    }

    /// The land band the smog's veil covers: everything below the land line,
    /// across the whole frame (B66: once a disc centred on the works, which read
    /// as a brown blob in whatever corner the industry happened to be).
    fn haze_rect(&self) -> Rect {
        Rect::new(0, self.sky_h as i32, self.width() as i32, self.height() as i32)
    }

    /// How thick the veil is, as a bucket: 0 when there is no smog. The veil is
    /// air, so it only repaints when this bucket changes — the rest of the time
    /// it is already in the framebuffer and only the boxes that move repaint.
    fn haze_level(&self) -> i32 {
        let d = &self.civic.dynamic;
        if d.smog < 0.05 {
            return 0;
        }
        let calm = 1.0 - (self.spec.wind_kmh / 22.0).clamp(0.0, 1.0);
        ((d.smog * (0.55 + 0.45 * calm)) * 8.0).round() as i32
    }

    /// Paint the civic layer over the world, inside `r`.
    pub(crate) fn draw_civic_over(&self, dst: &mut Framebuffer, r: Rect) {
        let d = &self.civic.dynamic;
        if d.props.is_empty() && d.fires.is_empty() && d.stacks.is_empty() && d.smog < 0.05 {
            return;
        }
        let t = self.civic_t;
        let wind = self.spec.wind;
        // Still air thickens the smog; a breeze clears it.
        let calm = 1.0 - (self.spec.wind_kmh / 22.0).clamp(0.0, 1.0);
        if d.smog >= 0.05 {
            let hr = self.haze_rect();
            if hr.intersects(&r) {
                // Cloud cover and night dim the wash with the world's tint.
                let tint = self.tint;
                let clip = dst.clip();
                dst.set_clip(r.intersect(&hr));
                civic::haze(dst, r, (self.sky_h as i32, self.height() as i32), d.smog * (0.55 + 0.45 * calm), tint);
                dst.set_clip(clip);
            }
        }
        let depth = self.depth();
        let tint = self.tint;
        let glow = self.glow;
        let clip = dst.clip();
        dst.set_clip(r.intersect(&clip));
        for s in &d.stacks {
            if !civic::stack_bounds(&self.cam, s).intersects(&r) {
                continue;
            }
            let key = actor_key(s.x as i32, s.y as i32).saturating_add(8);
            let mut pen = civic::Pen { fb: dst, depth, key };
            civic::stack(&mut pen, &self.cam, s, (t * 8.0).floor() / 8.0, wind);
        }
        for f in &d.fires {
            if !civic::fire_bounds(&self.cam, f).intersects(&r) {
                continue;
            }
            // Over its own walls, under anything in front of it.
            let key = static_key(f.x + f.w - 1 + f.y + f.h - 1, true).saturating_add(1);
            let mut pen = civic::Pen { fb: dst, depth, key };
            civic::fire(&mut pen, &self.cam, f, t, wind, glow);
        }
        for p in &d.props {
            if !civic::prop_bounds(&self.cam, &self.map, p).intersects(&r) {
                continue;
            }
            self.draw_civic_prop(dst, depth, p, t, tint, glow);
        }
        dst.set_clip(clip);
        // The fire's glow on the walls and the street after dark.
        if glow > 0.3 && !d.fires.is_empty() {
            let mut lights: Vec<crate::spectacle::Light> = Vec::new();
            for f in d.fires.iter().filter(|f| f.stage == 1) {
                let base = f.level as i32 * crate::iso::HEIGHT_UNIT;
                let c = self.cam.project(f.x as f32 + f.w as f32 * 0.5, (f.y + f.h) as f32, base + 10);
                let k = 0.55 + 0.25 * ((t * 9.0).sin() * 0.5 + 0.5);
                lights.push(crate::spectacle::Light { x: c.0, y: c.1, r: 110.0, rgb: [150.0, 70.0, 20.0], k });
            }
            if !lights.is_empty() {
                let clip = dst.clip();
                dst.set_clip(r.intersect(&clip));
                crate::spectacle::apply_lights(dst, &lights, ((glow - 0.3) / 0.5).clamp(0.0, 1.0));
                dst.set_clip(clip);
            }
        }
    }

    fn draw_civic_prop(&self, dst: &mut Framebuffer, depth: &[u16], p: &CivicProp, t: f32, tint: [f32; 3], glow: f32) {
        let (sx, sy) = civic::ground(&self.cam, &self.map, p.x, p.y);
        let sy = sy - p.lift;
        let (tx, ty) = (p.x.floor() as i32, p.y.floor() as i32);
        let slates = self.civic.stat.slates;
        let mut key = actor_key(tx, ty);
        // A cat on a roof belongs to its building's painter key.
        if p.kind == PropKind::Cat {
            key = actor_key(tx + 1, ty + 1).saturating_add(4);
        }
        if matches!(p.kind, PropKind::Engine | PropKind::Truck) {
            key = key.saturating_add(12);
        }
        let mut pen = civic::Pen { fb: dst, depth, key };
        let on_x = matches!(p.dir % 4, 0 | 2);
        match p.kind {
            PropKind::Poster => civic::poster(&mut pen, sx, sy, slates[(p.v & 1) as usize]),
            PropKind::Cat => {
                let blink = (t * 0.7 + p.x).fract() > 0.93;
                civic::cat(&mut pen, sx, sy, p.v, p.dir % 2 == 0, glow, blink);
            }
            PropKind::Lemonade => civic::lemonade(&mut pen, sx, sy),
            PropKind::Rod => civic::rod(&mut pen, sx, sy, p.dir, t),
            PropKind::Sax => civic::sax(&mut pen, sx, sy, t, p.dir % 2 == 0),
            PropKind::Notes => civic::notes(&mut pen, sx, sy, t),
            PropKind::Engine => {
                // The hose runs to the burning building: the glue puts the
                // building's door in `v`/`lift`-free tile space via `dir`.
                let target = (sx + if on_x { 26 } else { -26 }, sy - 18);
                civic::engine(&mut pen, sx, sy, t, target);
            }
            PropKind::Truck => civic::vehicle(&mut pen, sx, sy, 0, on_x, t, glow),
            PropKind::PostVan => civic::vehicle(&mut pen, sx, sy, 1, on_x, t, glow),
            PropKind::PoliceCar => civic::vehicle(&mut pen, sx, sy, 2, on_x, t, glow),
            PropKind::Barrier => civic::barrier(&mut pen, sx, sy),
            PropKind::Board => civic::board(&mut pen, sx, sy, slates, p.v as f32 / 255.0),
        }
        let _ = tint;
    }
}
