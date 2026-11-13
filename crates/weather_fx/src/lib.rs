//! CityLive Iso weather overlay (M6, brief B14).
//!
//! Cheap effects, drawn *on top of a finished frame* and driven
//! entirely by a [`world::Weather`] snapshot (which the `world` crate refreshes
//! on its own cadence; P6 keeps accumulation coarse, so the effects here are
//! pure per-frame cosmetics):
//!
//! * **rain** — thin slanted 1-px streaks, one per pool slot, slant set by the
//!   wind (speed + direction), density from precipitation, short-lived splash
//!   dots at the bottom in heavy rain;
//! * **snow** — slow 1-2 px flakes with a per-flake lateral sway, density from
//!   precipitation;
//! * **fog / mist** — a translucent light wash whose strength grows towards the
//!   bottom of the frame (mist hugs the ground), driven by visibility and the
//!   fog flag;
//! * **cloud shadow** — *not here any more* (B61c/D74): the clouds' shadows are
//!   B57's, cast through the B60 light model from the cloud field the same
//!   weather drives (`render::clouds`, `Scene::cloud_tick`). This crate used to
//!   drift a few large ellipses of its own, which meant two shadow systems on
//!   one sky; the overcast dimming wash stayed, in the ambient;
//! * **lightning** — during thunder (WMO ≥ 95) an occasional brief
//!   whole-frame white flash with a sharp attack and an exponential decay.
//!
//! [`Fx`] owns all particles in fixed pools created once in [`Fx::new`]
//! (nothing allocates in `update`/`draw`), everything is deterministic in
//! `(seed, dt-sequence, weather-sequence)`, and [`Fx::is_active`] is `false`
//! for clear weather so the wallpaper can skip the pass entirely.
//!
//! This crate does **not** accumulate anything (snow depth, wet ground — that
//! is the sim's business, M6 proper); it only paints what a snapshot says is
//! happening right now.
//!
//! Overlay colours are defined here rather than in `render::palette` because
//! this brief is add-only (`crates/weather_fx` and `tools/weather-strip`);
//! when the effects are wired into the renderer the constants can join the
//! palette tables for season/night swaps (planner's call).

pub mod ease;
pub mod intensities;
pub mod look;

pub use intensities::Intensity;

use map::rng::Rng;
use render::framebuffer::Framebuffer;
use render::palette::{self, Color, rgb};

/// Maximum streaks in the rain pool (pool is fixed; intensity selects a
/// prefix of it). B85 round 2: raised so a downpour is a curtain — several
/// hundred streaks per screen-height band across the 7040-wide union — and
/// clearly visible at phone size once downsampled, still inside the 2 ms budget.
pub const RAIN_POOL: usize = 5000;

/// Maximum flakes in the snow pool.
pub const SNOW_POOL: usize = 600;

/// Fixed ring of splash slots (heavy rain only). B85 round 2: a downpour lands
/// thousands of drops a second, so the ring is large enough that the ground
/// "boils" with crowns instead of overwriting them before they fade.
const SPLASH_SLOTS: usize = 512;

/// Fixed ring of water-ripple slots (heavy rain on water).
const RIPPLE_SLOTS: usize = 192;

const TAU: f32 = std::f32::consts::TAU;

// ---- overlay colours ------------------------------------------------------
//
// B61b: the washes and the cloud shadows moved into `render::weather` (the
// scene paints them, so a steady sky costs nothing per frame). Their colours
// and the fixed-point blend maths went with them, and are re-exported here so
// the strip tools and the tests keep using one source of truth.
pub use render::weather::{DIM, FLASH, FOG, MIST, SNOW_HAZE};

/// Thin raindrop streak (B61b: the particles keep their own colours here).
const RAIN: Color = rgb(0xd4, 0xe5, 0xf5);
/// Splash dot (a bright water pop).
const SPLASH: Color = rgb(0xe8, 0xf7, 0xff);
/// Snow flake (bright, near-white, faintly blue).
const SNOW: Color = rgb(0xf2, 0xf7, 0xff);
/// Sandstorm streaks and the dust veil (D66): bleached ochre, the colour the
/// old CityLive's dune sea gave its air.
const DUST: Color = rgb(0xd8, 0xb4, 0x80);

/// One pooled rain streak (B85: the pool also carries the sandstorm's grit).
///
/// `depth` is 0 (far, fine and faint) .. 1 (near, long and bright) and picks
/// the layer the drop falls in; `bright` is its own brightness 0..1. `len` is
/// the base particle length the grit uses too, and the rain's visible streak is
/// `len` stretched by `depth`.
#[derive(Clone, Copy, Debug)]
struct Drop {
    x: f32,
    y: f32,
    len: u8,
    speed: f32,
    depth: f32,
    bright: f32,
}

/// One pooled snow flake.
#[derive(Clone, Copy, Debug)]
struct Flake {
    x: f32,
    y: f32,
    /// 1 or 2 px.
    size: u8,
    /// Fall speed, px/s.
    speed: f32,
    /// Sway amplitude, px.
    sway_a: f32,
    /// Sway angular speed, rad/s.
    sway_w: f32,
    phase: f32,
    /// Constant personal drift, px/s (gust variation).
    drift: f32,
}

/// One short-lived splash dot: `t` is the remaining life, s. `y` is where it
/// lands (B85: in the scene's depth, not the screen bottom).
#[derive(Clone, Copy, Debug)]
struct Splash {
    x: f32,
    y: f32,
    t: f32,
}

/// One expanding ring ripple on water (B85 r2): `t` is the remaining life, s;
/// the ring grows from nothing to its full radius as `t` runs out.
#[derive(Clone, Copy, Debug)]
struct Ripple {
    x: f32,
    y: f32,
    t: f32,
}

/// Lightning flash scheduling + envelope.
#[derive(Clone, Copy, Debug)]
struct Flash {
    /// Elapsed time since the flash started, s (only while `live`).
    t: f32,
    /// Peak white alpha.
    peak: f32,
    /// Total lifetime, s.
    dur: f32,
    /// Time until the next flash, s (only while idle).
    next_in: f32,
    live: bool,
}

impl Flash {
    fn scheduled(&mut self, next_in: f32) {
        self.t = 0.0;
        self.peak = 0.0;
        self.dur = 0.0;
        self.next_in = next_in;
        self.live = false;
    }

    fn begin(&mut self, peak: f32, dur: f32) {
        self.t = 0.0;
        self.peak = peak;
        self.dur = dur;
        self.live = true;
        self.next_in = 0.0;
    }

    /// Current white alpha (0 .. peak).  Sharp ~35 ms attack, then an
    /// exponential decay — reads as a lightning strike, not a fade.
    fn alpha(&self) -> f32 {
        if !self.live {
            return 0.0;
        }
        if self.t < 0.035 {
            self.peak * (self.t / 0.035)
        } else {
            let x = -((self.t - 0.035) / 0.075);
            self.peak * x.exp()
        }
    }
}

/// A fixed-pool, allocation-free weather overlay; see the crate docs.
pub struct Fx {
    w: usize,
    h: usize,
    rng: Rng,
    inten: Intensity,
    /// B67: the intensities follow the forecast through a spring, so a front
    /// builds and clears over minutes instead of switching on a frame.
    ease: ease::IntensityEase,
    drops: Vec<Drop>,
    flakes: Vec<Flake>,
    splashes: [Splash; SPLASH_SLOTS],
    splash_i: usize,
    ripples: [Ripple; RIPPLE_SLOTS],
    ripple_i: usize,
    flash: Flash,
    /// Active prefix length of each pool (density from intensity).
    drops_n: usize,
    flakes_n: usize,
    /// Active dust streaks (the same pool as the rain, moved sideways).
    dust_n: usize,
    /// Latched in `update`; what `is_active`/`draw` report.
    active: bool,
    /// B85: where the ground (or a roof/tree) sits per column, in frame y — the
    /// drops splash here instead of at the screen bottom. `None` before the
    /// scene hands one over (the strip tools), in which case the frame bottom
    /// is the fallback.
    ground: Option<Vec<i32>>,
    /// B85 r2: the water surface per column (frame y, or -1 = no water) — a drop
    /// that lands on it makes a ring ripple instead of a ground crown.
    water: Option<Vec<i32>>,
    /// B61b: the static contribution (washes + cloud shadows), kept up to date by
    /// `update` so reading it allocates nothing and the scene can paint it.
    ambient: render::Ambient,
}

/// What the particles can paint on (B61b): the whole-frame `Framebuffer` of a
/// layer-shell frame, or one head's slice of the shared buffer.
pub trait Surface {
    fn put(&mut self, x: i32, y: i32, c: Color);
    /// The part of the frame this surface can paint, in frame coordinates. The
    /// wallpaper paints one head at a time, so a drop that is on another head
    /// must be skipped rather than walked over and clipped pixel by pixel.
    fn world_bounds(&self) -> render::Rect;
    fn put_blended(&mut self, x: i32, y: i32, c: Color, cov: f32);
}

impl Surface for Framebuffer {
    #[inline]
    fn world_bounds(&self) -> render::Rect {
        render::Rect::new(0, 0, self.w as i32, self.h as i32)
    }
    #[inline]
    fn put(&mut self, x: i32, y: i32, c: Color) {
        Framebuffer::put(self, x, y, c);
    }
    #[inline]
    fn put_blended(&mut self, x: i32, y: i32, c: Color, cov: f32) {
        Framebuffer::put_blended(self, x, y, c, cov);
    }
}

/// A surface viewed through an offset: pixel `(0,0)` here is `(ox, oy)` there.
struct Shifted<'a, S: Surface> {
    s: &'a mut S,
    ox: i32,
    oy: i32,
}

impl<S: Surface> Surface for Shifted<'_, S> {
    #[inline]
    fn world_bounds(&self) -> render::Rect {
        let r = self.s.world_bounds();
        render::Rect::new(r.x0 + self.ox, r.y0 + self.oy, r.x1 + self.ox, r.y1 + self.oy)
    }
    #[inline]
    fn put(&mut self, x: i32, y: i32, c: Color) {
        self.s.put(x - self.ox, y - self.oy, c);
    }
    #[inline]
    fn put_blended(&mut self, x: i32, y: i32, c: Color, cov: f32) {
        self.s.put_blended(x - self.ox, y - self.oy, c, cov);
    }
}

impl Fx {
    /// Create the pools for a `w×h` frame.  Deterministic in `seed`; all
    /// randomness afterwards comes from the same stream, so identical
    /// `(seed, dt, weather)` sequences produce identical frames.
    pub fn new(seed: u64, w: usize, h: usize) -> Self {
        let mut rng = Rng::new(seed ^ 0x7E47_391B);
        let wf = w as f32;
        let hf = h as f32;

        // Drops are seeded a margin outside the frame so the screen fills
        // gradually instead of starting fully rained-out. B85 round 2: three
        // depth layers — far (fine, faint, slow), mid, near (long, bright, fast,
        // 2 px wide) — weighted towards the layers you actually see, so a rain
        // or downpour reads as rain at phone size instead of a faint sprinkle.
        let mut drops = Vec::with_capacity(RAIN_POOL);
        for _ in 0..RAIN_POOL {
            let r = rng.range_f32(0.0, 1.0);
            let (depth, speed, len, bright) = if r < 0.40 {
                (0.15 + rng.range_f32(0.0, 1.0) * 0.25, 320.0 + rng.range_f32(0.0, 1.0) * 200.0, 5 + rng.range(0, 4) as u8, 0.45 + rng.range_f32(0.0, 1.0) * 0.20)
            } else if r < 0.80 {
                (0.50 + rng.range_f32(0.0, 1.0) * 0.25, 520.0 + rng.range_f32(0.0, 1.0) * 200.0, 6 + rng.range(0, 4) as u8, 0.65 + rng.range_f32(0.0, 1.0) * 0.20)
            } else {
                (0.80 + rng.range_f32(0.0, 1.0) * 0.20, 720.0 + rng.range_f32(0.0, 1.0) * 230.0, 7 + rng.range(0, 5) as u8, 0.82 + rng.range_f32(0.0, 1.0) * 0.18)
            };
            drops.push(Drop {
                x: rng.range_f32(-60.0, wf + 60.0),
                y: rng.range_f32(-hf, hf),
                len,
                speed,
                depth,
                bright,
            });
        }
        let mut flakes = Vec::with_capacity(SNOW_POOL);
        for _ in 0..SNOW_POOL {
            flakes.push(Flake {
                x: rng.range_f32(-20.0, wf + 20.0),
                y: rng.range_f32(-hf, hf),
                size: if rng.chance(0.25) { 2 } else { 1 },
                speed: rng.range_f32(16.0, 44.0),
                sway_a: rng.range_f32(6.0, 18.0),
                sway_w: rng.range_f32(0.4, 1.2),
                phase: rng.range_f32(0.0, TAU),
                drift: rng.range_f32(-8.0, 8.0),
            });
        }
        let mut fx = Self {
            w,
            h,
            rng,
            inten: Intensity::default(),
            ease: ease::IntensityEase::default(),
            drops,
            flakes,
            splashes: [Splash { x: 0.0, y: 0.0, t: 0.0 }; SPLASH_SLOTS],
            splash_i: 0,
            ripples: [Ripple { x: 0.0, y: 0.0, t: 0.0 }; RIPPLE_SLOTS],
            ripple_i: 0,
            flash: Flash {
                t: 0.0,
                peak: 0.0,
                dur: 0.0,
                next_in: 10.0,
                live: false,
            },
            drops_n: 0,
            flakes_n: 0,
            dust_n: 0,
            active: false,
            ambient: render::Ambient::default(),
            ground: None,
            water: None,
        };
        // Start idle, first strike 6-20 s in (the stream keeps determinism).
        fx.flash.scheduled(fx.rng.range_f32(6.0, 20.0));
        fx
    }

    /// Advance by `dt` seconds under `weather`: re-derive the intensities,
    /// move the active particle prefixes, and tick the lightning.  No
    /// allocation.
    pub fn update(&mut self, dt: f32, weather: &world::Weather) {
        let dt_real = dt.clamp(0.0, 5.0);
        let dt = dt.clamp(0.0, 0.1); // a hiccup must not teleport particles
        // B67: what the forecast says is where the weather is *heading*.
        let inten = self.ease.step(&Intensity::from_weather(weather), dt_real);
        self.inten = inten;

        // ---- rain: grow/shrink the active prefix, step every active drop ---
        let n = (inten.rain * RAIN_POOL as f32) as usize;
        if n > self.drops_n {
            for i in self.drops_n..n {
                self.respawn_drop(i);
            }
        }
        self.drops_n = n;
        let wf = self.w as f32;
        let hf = self.h as f32;
        for i in 0..n {
            let d = &mut self.drops[i];
            d.y += d.speed * dt;
            d.x += inten.wind_x * dt;
            // B85: the drop lands at the surface of its own depth layer. The
            // ground profile is the *far* surface (the topmost opaque pixel per
            // column); a far drop splashes there, a near drop splashes near the
            // bottom, so the rain hits roofs and hills at every depth instead of
            // one line at the frame bottom.
            let ground = self
                .ground
                .as_ref()
                .and_then(|g| {
                    let xi = d.x as i32;
                    (xi >= 0 && (xi as usize) < g.len()).then(|| g[xi as usize] as f32)
                })
                .unwrap_or(hf + 10.0);
            let landing = ground + d.depth * (hf - ground);
            if d.y > landing {
                let land_x = d.x;
                let land_y = landing.clamp(0.0, hf - 1.0);
                d.y -= hf + 20.0;
                d.x = self.rng.range_f32(-60.0, wf + 60.0);
                if inten.rain >= 0.45 {
                    // On water the drop makes a ring ripple, on ground a crown.
                    let xi = land_x as i32;
                    let on_water = self.water.as_ref().is_some_and(|w| {
                        xi >= 0 && (xi as usize) < w.len() && w[xi as usize] >= 0
                    });
                    if on_water {
                        self.ripples[self.ripple_i] = Ripple {
                            x: land_x,
                            y: land_y,
                            t: 0.5,
                        };
                        self.ripple_i = (self.ripple_i + 1) % RIPPLE_SLOTS;
                    } else {
                        self.splashes[self.splash_i] = Splash {
                            x: land_x,
                            y: land_y,
                            t: 0.12,
                        };
                        self.splash_i = (self.splash_i + 1) % SPLASH_SLOTS;
                    }
                }
            } else if d.x < -70.0 {
                d.x += wf + 140.0;
            } else if d.x > wf + 70.0 {
                d.x -= wf + 140.0;
            }
        }
        for s in &mut self.splashes {
            if s.t > 0.0 {
                s.t -= dt;
            }
        }
        for r in &mut self.ripples {
            if r.t > 0.0 {
                r.t -= dt;
            }
        }

        // ---- dust: the desert's downpour (D66) -----------------------------
        // Same pool as the rain (a desert never rains), but blown sideways:
        // a sandstorm is a horizontal wall of grit, not a vertical shower.
        let dust_n = if self.inten.dust > 0.01 {
            (self.inten.dust * RAIN_POOL as f32) as usize
        } else {
            0
        };
        if dust_n > self.dust_n {
            for i in self.dust_n..dust_n {
                self.respawn_drop(i);
            }
        }
        self.dust_n = dust_n;
        for i in 0..dust_n {
            let d = &mut self.drops[i];
            let sideways = if inten.wind_x.abs() > 1.0 {
                inten.wind_x.signum()
            } else {
                1.0
            };
            d.x += (d.speed * 0.55 + inten.wind_x.abs() * 0.5) * sideways * dt;
            // A slow sink, so the wall of grit fills the frame.
            d.y += (14.0 + d.speed * 0.04) * dt;
            if d.x < -80.0 {
                d.x += wf + 160.0;
            } else if d.x > wf + 80.0 {
                d.x -= wf + 160.0;
            }
            if d.y > hf + 4.0 {
                d.y -= hf + 8.0;
            }
        }

        // ---- snow ----------------------------------------------------------
        let m = (inten.snow * SNOW_POOL as f32) as usize;
        if m > self.flakes_n {
            for i in self.flakes_n..m {
                self.respawn_flake(i);
            }
        }
        self.flakes_n = m;
        for i in 0..m {
            let f = &mut self.flakes[i];
            f.y += f.speed * dt;
            f.phase = (f.phase + f.sway_w * TAU * dt) % TAU;
            f.x += (inten.wind_x * 0.5 + f.drift) * dt;
            if f.y > hf + 4.0 {
                f.y -= hf + 8.0;
                f.x = self.rng.range_f32(-20.0, wf + 20.0);
            } else if f.x < -30.0 {
                f.x += wf + 60.0;
            } else if f.x > wf + 30.0 {
                f.x -= wf + 60.0;
            }
        }

        // ---- lightning ------------------------------------------------------
        // A live flash always runs to completion (a storm ending mid-bolt
        // shouldn't clip it); only the *scheduling* is weather-driven.
        if self.flash.live {
            self.flash.t += dt;
            if self.flash.t > self.flash.dur || self.flash.alpha() < 0.004 {
                self.flash
                    .scheduled(if inten.thunder {
                        self.rng.range_f32(6.0, 20.0)
                    } else {
                        10.0
                    });
            }
        } else {
            self.flash.next_in -= dt;
            if inten.thunder && self.flash.next_in <= 0.0 {
                self.flash
                    .begin(self.rng.range_f32(0.35, 0.65), self.rng.range_f32(0.3, 0.6));
            } else if !inten.thunder {
                // Not storming: hold the timer at a fixed re-arm delay
                // (state-only, no RNG — determinism preserved).
                self.flash.scheduled(10.0);
            }
        }

        self.active = self.inten.any() || self.flash.alpha() > 0.004;
        // B61b: the ambient the caller paints, refreshed in place (no
        // allocation after the first frame, so `update` stays alloc-free).
        let active = self.active;
        let a = &mut self.ambient;
        a.dim = if active { self.inten.dim } else { 0.0 };
        a.fog = if active { self.inten.fog } else { 0.0 };
        a.mist = if active { self.inten.rain } else { 0.0 };
        a.haze = if active { self.inten.snow } else { 0.0 };
        a.flash = self.flash.alpha();
    }

    /// Force a lightning strike to start now (tests / the strip tool).
    pub fn trigger_lightning(&mut self) {
        self.flash
            .begin(self.rng.range_f32(0.35, 0.65), self.rng.range_f32(0.3, 0.6));
    }

    /// Intensity state from the last `update`.
    pub fn intensities(&self) -> &Intensity {
        &self.inten
    }

    /// Alpha of the current flash (0 when none — public for tests/benches).
    pub fn flash_alpha(&self) -> f32 {
        self.flash.alpha()
    }

    /// True once at least one effect has something to draw.  The wallpaper
    /// uses it to skip the pass on clear frames (cheap sleep, D15).
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// B85: hand over the per-column landing y (in frame coordinates) the drops
    /// splash against. `None` clears it (the frame bottom becomes the fallback).
    /// The scene builds this from its depth plane whenever its static layer
    /// changes; it is read, not copied, so it must stay valid while set.
    pub fn set_ground(&mut self, ground: Option<Vec<i32>>) {
        self.ground = ground;
    }

    /// B85 r2: hand over the per-column water surface y (-1 = no water). A drop
    /// that lands on it makes a ring ripple instead of a ground crown.
    pub fn set_water(&mut self, water: Option<Vec<i32>>) {
        self.water = water;
    }

    fn respawn_drop(&mut self, i: usize) {
        self.drops[i].x = self.rng.range_f32(-60.0, self.w as f32 + 60.0);
        self.drops[i].y = self
            .rng
            .range_f32(-(self.h as f32) * 0.5, self.h as f32);
    }

    fn respawn_flake(&mut self, i: usize) {
        self.flakes[i].x = self.rng.range_f32(-20.0, self.w as f32 + 20.0);
        self.flakes[i].y = self
            .rng
            .range_f32(-(self.h as f32) * 0.5, self.h as f32);
    }

    // ---- drawing ------------------------------------------------------------

    /// The weather's *static* contribution for this frame: the washes, as plain
    /// data the scene paints inside its own composite (B61b). Empty when nothing
    /// is active.
    pub fn ambient(&self) -> &render::Ambient {
        &self.ambient
    }

    /// Draw everything: the ambient, then the particles (the strip tool and the
    /// tests use this; the wallpaper paints the ambient in its scene and the
    /// particles separately, see `draw_particles`).
    pub fn draw(&self, fb: &mut Framebuffer) {
        if !self.active {
            return;
        }
        self.ambient.apply(fb, render::Rect::from_size(fb.w, fb.h));
        self.draw_overlay(fb, 0, 0);
    }

    /// Draw the moving particles only (rain, splashes, snow, dust grit) onto
    /// `s`, whose pixel (0,0) is `(ox, oy)` of the scene frame (B61b: the
    /// wallpaper paints one head's slice of the overlay straight into that
    /// head's shared buffer).
    pub fn draw_particles<S: Surface>(&self, s: &mut S, ox: i32, oy: i32) {
        if !self.active {
            return;
        }
        let mut shifted = Shifted { s, ox, oy };
        self.draw_rain(&mut shifted);
        self.draw_snow(&mut shifted);
        self.draw_dust(&mut shifted);
    }

    /// Draw the moving half of the weather: the drifting cloud shadows, then the
    /// particles (B61b). This is what the wallpaper paints per frame — into the
    /// layer-shell frame, or into each head's shared buffer — while the washes
    /// are the scene's. Order matches the old whole-frame pass exactly.
    pub fn draw_overlay<S: Surface>(&self, s: &mut S, ox: i32, oy: i32) {
        if !self.active {
            return;
        }
        let mut shifted = Shifted { s, ox, oy };
        self.draw_rain(&mut shifted);
        self.draw_snow(&mut shifted);
        self.draw_dust(&mut shifted);
    }

    /// The screen rectangles the overlay covers this frame, for the scene's
    /// damage list (B61b, layer-shell path): one rect per particle. The scene merges them onto its 64x32 damage grid and falls
    /// back to a full frame when they cover too much of it, exactly as before.
    pub fn overlay_rects(&self, out: &mut Vec<render::Rect>) {
        self.push_overlay_rects(out, None);
    }

    /// B86: the same coverage as [`overlay_rects`](Self::overlay_rects), but a
    /// slanted rain streak is cut into slices of `slice_rows` rows, each boxed
    /// tightly around its own pixels. A streak is one pixel wide, so its
    /// bounding box is mostly empty (a leaning 40-row streak boxes ~800 pixels
    /// for ~40 drawn); the shared-memory export copies and uploads rectangles,
    /// so the tight slices are what keeps a downpour cheap. The reader's table
    /// merges slices back together where a join is cheaper than two boxes.
    pub fn overlay_rects_fine(&self, out: &mut Vec<render::Rect>, slice_rows: i32) {
        self.push_overlay_rects(out, Some(slice_rows.max(1)));
    }

    fn push_overlay_rects(&self, out: &mut Vec<render::Rect>, slice: Option<i32>) {
        out.clear();

        for i in 0..self.drops_n {
            let d = &self.drops[i];
            // The same bounding box `draw_rain` covers: the streak leans with
            // the wind and its length grows with depth. A drop is restored by
            // this exact rectangle next frame, so a moving streak never trails.
            let slant = self.inten.wind_x / d.speed;
            let vis_len = (d.len as f32 * (0.7 + d.depth * 2.6)).max(3.0);
            let top_x = d.x - slant * vis_len;
            let top_y = d.y - vis_len;
            let x0 = top_x.min(d.x).floor() as i32;
            let x1 = top_x.max(d.x).ceil() as i32 + 1;
            let y0 = top_y as i32;
            let y1 = d.y as i32 + 1;
            match slice {
                Some(rows) => {
                    // `draw_rain` puts pixel k of the streak at row `y0 + k`,
                    // x = top_x + k * slant (truncated; the accumulation there
                    // adds a rounding error far below the 1 px margin kept here).
                    let n = vis_len as i32;
                    let mut k0 = 0;
                    while k0 < n {
                        let k1 = (k0 + rows).min(n);
                        let xa = top_x + slant * k0 as f32;
                        let xb = top_x + slant * (k1 - 1) as f32;
                        out.push(render::Rect::new(
                            (xa.min(xb) - 0.01).floor() as i32,
                            y0 + k0,
                            // `as i32` truncates toward zero, so left of the
                            // frame the drawn pixel can be one right of the floor.
                            ((xa.max(xb) + 0.01) as i32).max((xa.max(xb) + 0.01).floor() as i32) + 1,
                            y0 + k1,
                        ));
                        k0 = k1;
                    }
                }
                None => out.push(render::Rect::new(x0, y0, x1, y1)),
            }
        }
        if self.inten.rain >= 0.45 {
            for s in &self.splashes {
                if s.t <= 0.0 {
                    continue;
                }
                let x = s.x as i32;
                let y = s.y as i32;
                out.push(render::Rect::new(x - 1, y - 2, x + 2, y + 1));
            }
            // A ripple's ring can reach ~7 px, so name its whole bounding box.
            for r in &self.ripples {
                if r.t <= 0.0 {
                    continue;
                }
                let x = r.x as i32;
                let y = r.y as i32;
                out.push(render::Rect::new(x - 7, y - 7, x + 8, y + 8));
            }
        }
        for i in 0..self.flakes_n {
            let f = &self.flakes[i];
            let x = (f.x + f.sway_a * f.phase.sin()) as i32;
            let y = f.y as i32;
            let n = if f.size == 2 { 2 } else { 1 };
            out.push(render::Rect::new(x, y, x + n, y + n));
        }
        let d = self.inten.dust;
        if d > 0.01 {
            let left = self.inten.wind_x < 0.0;
            let cov_len = |dr: &Drop| (4 + dr.len as i32) * if d > 0.6 { 2 } else { 1 };
            for i in 0..self.dust_n {
                let dr = &self.drops[i];
                let x = dr.x as i32;
                let y = dr.y as i32;
                let len = cov_len(dr);
                let (x0, x1) = if left { (x - len, x + 1) } else { (x, x + len + 1) };
                out.push(render::Rect::new(x0, y, x1, y + len / 6 + 2));
            }
        }
        // The scene clips to the frame; a particle outside it costs a merge
        // step and nothing else.
    }

    /// A sandstorm (D66): long horizontal streaks of grit. Only runs while the
    /// storm does, so a clear desert frame pays nothing.
    fn draw_dust<S: Surface>(&self, fb: &mut S) {
        let d = self.inten.dust;
        if d <= 0.01 {
            return;
        }
        // The cloud of dust itself is *not* a wash here: a full-frame blend
        // measured 23 ms at 7040x1440, so the veil lives in the scene's ambient
        // light (`WeatherLook::dust`, applied once per light change) and this
        // pass only draws the grit.
        // The grit: horizontal streaks, longer and looser as the wind rises.
        let left = self.inten.wind_x < 0.0;
        let cov = (0.30 + 0.45 * d).min(0.8);
        let world = fb.world_bounds();
        for i in 0..self.dust_n {
            let dr = &self.drops[i];
            let x = dr.x as i32;
            let y = dr.y as i32;
            if y < world.y0 - 2 || y > world.y1 + 2 {
                continue;
            }
            let len = (4 + dr.len as i32) * if d > 0.6 { 2 } else { 1 };
            for k in 0..len {
                let px = if left { x - k } else { x + k };
                fb.put_blended(px, y + k / 6, DUST, cov);
                if k % 3 == 0 {
                    fb.put_blended(px, y + 1 + k / 6, palette::shade(DUST, 0.86), cov * 0.7);
                }
            }
        }
    }

    fn draw_rain<S: Surface>(&self, fb: &mut S) {
        if self.drops_n == 0 {
            return;
        }
        let world = fb.world_bounds();
        let rain = self.inten.rain;
        for i in 0..self.drops_n {
            let d = &self.drops[i];
            // The streak leans with the wind: the horizontal offset per pixel of
            // fall is the wind's px/s over the drop's own fall speed, so a light
            // breeze is near-vertical and a gale is a real slant (B85 — the old
            // fixed 45° slant read as a wall of dashes).
            let slant = self.inten.wind_x / d.speed;
            // The visible streak is the drop's trail: longer and brighter near,
            // fine and faint far (depth layers).
            let vis_len = (d.len as f32 * (0.7 + d.depth * 2.6)).max(3.0);
            let top_x = d.x - slant * vis_len;
            let top_y = d.y - vis_len;
            let x0 = top_x.min(d.x).floor() as i32;
            let x1 = top_x.max(d.x).ceil() as i32 + 1;
            let y0 = top_y as i32;
            let y1 = d.y as i32 + 1;
            if x1 < world.x0 || x0 > world.x1 || y1 < world.y0 || y0 > world.y1 {
                continue;
            }
            // Translucent and scene-lit: the blend picks up whatever is behind
            // the drop, so rain darkens at night and catches a bright window.
            // Round 2: brighter and denser, so a rain reads everywhere and a
            // downpour is a curtain.
            let cov = ((0.32 + 0.36 * d.depth) * (0.7 + 0.6 * rain) * d.bright)
                .clamp(0.12, 0.85);
            let n = vis_len as i32;
            let mut fx = top_x;
            for k in 0..n {
                fb.put_blended(fx as i32, y0 + k, RAIN, cov);
                fx += slant;
            }
        }
        if rain >= 0.45 {
            for s in &self.splashes {
                if s.t <= 0.0 {
                    continue;
                }
                let x = s.x as i32;
                let y = s.y as i32;
                // A crown splash (B85 round 2): a small bright V that reads as a
                // drop bursting where it hit, fading as its life runs out.
                let life = (s.t / 0.12).clamp(0.0, 1.0);
                fb.put_blended(x, y, SPLASH, 0.70 * life);
                fb.put_blended(x - 1, y - 1, SPLASH, 0.50 * life);
                fb.put_blended(x + 1, y - 1, SPLASH, 0.50 * life);
                fb.put_blended(x, y - 2, SPLASH, 0.36 * life);
            }
            // Ring ripples on water (B85 r2): an octagon that grows from nothing
            // to a few pixels across as its life runs out, then fades.
            for r in &self.ripples {
                if r.t <= 0.0 {
                    continue;
                }
                let x = r.x as i32;
                let y = r.y as i32;
                let life = (r.t / 0.5).clamp(0.0, 1.0);
                let rad = ((1.0 - life) * 7.0).max(0.0);
                let ri = rad as i32;
                let cov = 0.55 * life;
                if ri <= 0 {
                    fb.put_blended(x, y, SPLASH, cov);
                    continue;
                }
                let di = (ri as f32 * 0.7) as i32;
                for &(dx, dy) in &[
                    (ri, 0),
                    (-ri, 0),
                    (0, ri),
                    (0, -ri),
                    (di, di),
                    (-di, di),
                    (di, -di),
                    (-di, -di),
                ] {
                    fb.put_blended(x + dx, y + dy, SPLASH, cov);
                }
            }
        }
    }

    fn draw_snow<S: Surface>(&self, fb: &mut S) {
        if self.flakes_n == 0 {
            return;
        }
        let world = fb.world_bounds();
        for i in 0..self.flakes_n {
            let f = &self.flakes[i];
            let x = (f.x + f.sway_a * f.phase.sin()) as i32;
            let y = f.y as i32;
            if x < world.x0 - 2 || x > world.x1 + 2 || y < world.y0 - 2 || y > world.y1 + 2 {
                continue;
            }
            fb.put(x, y, SNOW);
            if f.size == 2 {
                // 2×2 flake, slightly dimmer so it reads as further away.
                fb.put_blended(x + 1, y, SNOW, 0.8);
                fb.put_blended(x, y + 1, SNOW, 0.8);
                fb.put_blended(x + 1, y + 1, SNOW, 0.6);
            }
        }
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    fn weather(mut f: impl FnMut(&mut world::Weather)) -> world::Weather {
        let mut w = world::Weather::default();
        f(&mut w);
        w
    }

    /// A flake's sway phase is an angle: it must not grow for ever (an f32 that
    /// only grows stops resolving a frame step and the flake stops swaying).
    #[test]
    fn a_flakes_sway_phase_stays_an_angle() {
        let snow = crate::look::preset("snow").unwrap();
        let mut fx = Fx::new(5, 640, 360);
        for _ in 0..30 {
            fx.update(0.033, &snow);
        }
        assert!(fx.flakes_n > 0);
        fx.flakes[0].phase = 1_000_000.0;
        fx.update(0.033, &snow);
        assert!(fx.flakes[0].phase < TAU, "phase {}", fx.flakes[0].phase);
    }

    /// B61b: the frame the wallpaper builds must be the frame the old
    /// whole-frame draw produced. `Fx::draw` (the strip tool's path) is the
    /// reference: ambient + particles. The wallpaper applies the same ambient
    /// inside its scene composite and then the same particles, so comparing the
    /// two on one base frame is the look test.
    #[test]
    fn the_scene_path_and_the_whole_frame_draw_agree() {
        let w = weather(|w| {
            w.code = 63; // rain
            w.precip = 2.0;
            w.cloud = 100.0;
            w.wind = 20.0;
            w.wind_dir = Some(270.0);
            w.visibility = Some(4000.0);
        });
        let mut fx = Fx::new(11, 200, 120);
        for _ in 0..40 {
            fx.update(1.0 / 30.0, &w);
        }
        fx.trigger_lightning();
        for _ in 0..2 {
            fx.update(1.0 / 30.0, &w);
        }
        assert!(fx.is_active());

        let mut base = Framebuffer::new(200, 120);
        for (i, p) in base.px_mut().iter_mut().enumerate() {
            *p = 0xFF00_0000 | ((i * 3) as u32 & 0xFF) << 16 | ((i * 11) as u32 & 0xFF);
        }
        let mut old = base.px().to_vec();
        let mut old_fb = Framebuffer::new(200, 120);
        old_fb.px_mut().copy_from_slice(&old);
        fx.draw(&mut old_fb);

        // The wallpaper's order: the scene paints the ambient inside its
        // composite (clipped to the damaged rects), then the particles.
        let mut new_fb = Framebuffer::new(200, 120);
        new_fb.px_mut().copy_from_slice(&old);
        fx.ambient().apply(&mut new_fb, render::Rect::from_size(200, 120));
        fx.draw_overlay(&mut new_fb, 0, 0);

        assert_eq!(old_fb.px(), new_fb.px(), "the split must not change the look");
        let _ = &mut old;
    }

    /// The ambient only changes when the weather does: a steady sky must not ask
    /// the scene for a new composite every frame (B61b's whole point).
    #[test]
    fn a_steady_sky_keeps_the_same_ambient_key() {
        let w = weather(|w| {
            w.code = 61;
            w.precip = 0.4;
            w.cloud = 90.0;
            w.wind = 12.0;
            w.wind_dir = Some(180.0);
        });
        let mut fx = Fx::new(5, 320, 200);
        for _ in 0..30 {
            fx.update(1.0 / 30.0, &w);
        }
        let k0 = fx.ambient().key();
        for _ in 0..5 {
            fx.update(1.0 / 30.0, &w);
        }
        let k1 = fx.ambient().key();
        assert_eq!(k0, k1, "the washes must not move while the weather holds");
    }

    #[test]
    fn clear_weather_is_inactive_and_draws_nothing() {
        let w = world::Weather::default(); // cloud 30, no precip, no visibility
        let mut fx = Fx::new(7, 960, 540);
        for _ in 0..10 {
            fx.update(1.0 / 30.0, &w);
        }
        assert!(!fx.is_active());
        let mut base = Framebuffer::new(960, 540);
        base.fill_rect(0, 0, 30, 27, rgb(9, 120, 20)); // mixed contents, incl. transparent
        let before: Vec<Color> = base.px().to_vec();
        fx.draw(&mut base);
        assert_eq!(before, base.px().to_vec(), "clear weather must not paint");
    }

    #[test]
    fn intensities_map_weather_sensibly() {
        let w = weather(|w| {
            w.code = 63;
            w.precip = 2.4;
            w.cloud = 100.0;
            w.wind = 30.0;
            w.wind_dir = Some(270.0);
        });
        let i = Intensity::from_weather(&w);
        assert!(i.rain >= 0.9 && i.rain <= 1.0, "rain: {i:?}");
        assert_eq!(i.snow, 0.0);
        assert!(i.dim > 0.99, "dim: {i:?}");
        assert!(i.shadow > 0.99, "shadow: {i:?}");
        assert!(!i.thunder);
        // Wind from the west (270°) → blows east → screen-left (H12 mapping).
        assert!((i.wind_x - (-120.0)).abs() < 1.0, "wind_x: {i:?}");

        let w = weather(|w| {
            w.code = 75;
            w.precip = 1.0;
            w.cloud = 80.0;
            w.visibility = Some(1500.0);
        });
        let i = Intensity::from_weather(&w);
        assert!((i.snow - 1.0).abs() < 0.001, "snow: {i:?}");
        // B76: a snowfall gets a quarter of the visibility mist — its own flakes
        // and its lifted light are the snow's veil, and a full fog wash hid it.
        assert!((i.fog - 0.25).abs() < 0.01, "fog (low visibility in snow): {i:?}");
        assert!(i.rain <= 0.01);

        let w = weather(|w| {
            w.code = 45;
            w.visibility = Some(15_000.0); // fog flag must win anyway
            w.cloud = 60.0;
        });
        let i = Intensity::from_weather(&w);
        assert!((i.fog - 1.0).abs() < 0.001, "fog flag wins: {i:?}");

        let w = weather(|w| {
            w.code = 96;
            w.precip = 0.4;
            w.cloud = 100.0;
        });
        let i = Intensity::from_weather(&w);
        assert!(i.thunder, "thunder: {i:?}");
        assert!((i.rain - 0.6).abs() < 0.001, "storm rain floor: {i:?}");

        // No direction → legacy west wind → leftward slant.
        let w = weather(|w| {
            w.code = 61;
            w.precip = 0.5;
            w.wind = 10.0;
        });
        assert!(Intensity::from_weather(&w).wind_x < 0.0);
    }

    fn run_frames(seed: u64, w: &world::Weather) -> Vec<Color> {
        let mut fx = Fx::new(seed, 960, 540);
        let mut f = Framebuffer::new(960, 540);
        f.fill_rect(0, 0, 960, 540, rgb(30, 70, 40));
        for _ in 0..90 {
            fx.update(1.0 / 30.0, w);
            fx.draw(&mut f);
        }
        f.px().to_vec()
    }

    #[test]
    fn same_seed_same_sequence_same_frames() {
        let w = weather(|w| {
            w.code = 65;
            w.precip = 2.0;
            w.cloud = 100.0;
            w.wind = 25.0;
            w.wind_dir = Some(245.0);
        });
        let a = run_frames(99, &w);
        let b = run_frames(99, &w);
        assert_eq!(a, b, "same seed + same dt/weather → same image");
        let c = run_frames(100, &w);
        assert!(a != c, "different seed → different particle layout");
    }

    #[test]
    fn lightning_schedules_strikes_and_decays_deterministically() {
        let w = weather(|w| {
            w.code = 95;
            w.precip = 1.0;
            w.cloud = 100.0;
        });
        // 60 s in 1/30 s steps: strikes every 6-20 s → several of them.
        fn strikes(seed: u64, w: &world::Weather) -> Vec<f32> {
            let mut fx = Fx::new(seed, 960, 540);
            let mut out = Vec::new();
            for _ in 0..1800 {
                fx.update(1.0 / 30.0, w);
                out.push(fx.flash_alpha());
            }
            out
        }
        let a = strikes(42, &w);
        let b = strikes(42, &w);
        assert_eq!(a, b, "identical strike timing for an identical run");
        let strikes = a
            .windows(2)
            .filter(|w| w[0] == 0.0 && w[1] > 0.0)
            .count();
        assert!(strikes >= 2, "60 s of storm should give ≥ 2 strikes, got {strikes}");
        let peak = a.iter().cloned().fold(0.0f32, f32::max);
        assert!(peak > 0.3 && peak <= 0.65, "peak alpha out of range: {peak}");
        assert_eq!(*a.last().unwrap(), 0.0, "last sample must be settled (flash done decaying)");
    }

    #[test]
    fn trigger_lightning_fires_immediately() {
        let mut fx = Fx::new(1, 960, 540);
        let w = world::Weather::default();
        fx.update(1.0 / 30.0, &w);
        assert!(!fx.is_active());
        fx.trigger_lightning();
        fx.update(1.0 / 30.0, &w);
        assert!(fx.flash_alpha() > 0.05, "flash should be rising on the first frame");
        for _ in 0..120 {
            fx.update(1.0 / 30.0, &w);
        }
        assert_eq!(fx.flash_alpha(), 0.0, "flash must have decayed");
    }

    #[test]
    fn particles_stay_bounded_after_many_steps() {
        let w = weather(|w| {
            w.code = 85;
            w.precip = 1.0;
            w.cloud = 95.0;
            w.wind = 30.0;
            w.wind_dir = Some(90.0); // strong rightward push
        });
        let mut fx = Fx::new(5, 960, 540);
        for _ in 0..1800 {
            fx.update(1.0 / 30.0, &w);
        }
        let n = fx.drops_n;
        assert!(
            fx.drops[..n]
                .iter()
                .all(|d| d.y < 580.0 && d.x > -80.0 && d.x < 1040.0),
            "rain escapes its margins"
        );
        let m = fx.flakes_n;
        assert!(
            fx.flakes[..m]
                .iter()
                .all(|f| f.y < 550.0 && f.x > -40.0 && f.x < 1000.0),
            "snow escapes its margins"
        );
    }

    #[test]
    fn fast_blend_matches_f32_blend_closely() {
        let (km, add) = render::weather::fast_blend_table(rgb(200, 180, 90), 0.3);
        let dst = rgb(34, 120, 210);
        let got = render::weather::mix_pixel(dst, km, add);
        let want = palette::blend(dst, rgb(200, 180, 90), 0.3);
        for (a, b) in [
            (palette::r(got) as i32, palette::r(want) as i32),
            (palette::g(got) as i32, palette::g(want) as i32),
            (palette::b(got) as i32, palette::b(want) as i32),
        ] {
            assert!(a.abs_diff(b) <= 2, "channel drift {a} vs {b}");
        }
    }

    /// Cost measurement (planner relaxed the original < 1 ms bound on
    /// 2026-11-09; the merged wash stays the cost centre).  Only meaningful
    /// in release; run with:
    /// `cargo test -p weather_fx --release -- --ignored --exact cost_bounds_960x540`
    #[ignore]
    #[test]
    fn cost_bounds_960x540() {
        let mut fx = Fx::new(0xC17E_15E7, 960, 540);
        let heavy = weather(|w| {
            w.code = 65;
            w.precip = 3.0;
            w.cloud = 100.0;
            w.wind = 30.0;
            w.wind_dir = Some(250.0);
        });
        let foggy = weather(|w| {
            w.code = 45;
            w.visibility = Some(1500.0);
            w.cloud = 55.0;
        });
        let mut fb = Framebuffer::new(960, 540);
        fb.fill_rect(0, 0, 960, 540, rgb(80, 120, 70));

        // Warm up.
        for _ in 0..30 {
            fx.update(1.0 / 30.0, &heavy);
            fx.draw(&mut fb);
        }
        let t = std::time::Instant::now();
        for _ in 0..300 {
            fx.update(1.0 / 30.0, &heavy);
            fx.draw(&mut fb);
        }
        let rain_ns = t.elapsed().as_nanos() as f64 / 300.0;
        assert!(rain_ns < 3_000_000.0, "heavy rain {rain_ns:.0} ns over relaxed budget (3 ms)");

        let mut fx2 = Fx::new(0xC17E_15E7, 960, 540);
        for _ in 0..30 {
            fx2.update(1.0 / 30.0, &foggy);
            fx2.draw(&mut fb);
        }
        let t = std::time::Instant::now();
        for _ in 0..300 {
            fx2.update(1.0 / 30.0, &foggy);
            fx2.draw(&mut fb);
        }
        let fog_ns = t.elapsed().as_nanos() as f64 / 300.0;
        assert!(fog_ns < 3_000_000.0, "fog {fog_ns:.0} ns over relaxed budget (3 ms)");

        println!("heavy rain update+draw: {rain_ns:.0} ns (relaxed budget < 3 ms)");
        println!("fog update+draw:       {fog_ns:.0} ns (relaxed budget < 3 ms)");
    }
}

#[cfg(test)]
mod flash_envelope {
    use super::*;

    /// Triggered strikes must actually render: after the attack (45 ms in)
    /// the whole frame is washed and the alpha is well into the peak range.
    #[test]
    fn triggered_flash_reaches_peak_and_washes_the_frame() {
        let mut w = world::Weather::default();
        w.code = 95;
        w.precip = 2.5;
        w.cloud = 100.0;
        w.wind = 25.0;
        w.wind_dir = Some(230.0);
        let mut fx = Fx::new(0xC17E_15E7, 960, 540);
        for _ in 0..60 {
            fx.update(1.0 / 30.0, &w);
        }
        fx.trigger_lightning();
        fx.update(0.045, &w);
        let a = fx.flash_alpha();
        assert!(a > 0.2, "flash should be near peak 45 ms after trigger: {a}");

        let mut fb = Framebuffer::new(960, 540);
        fb.fill_rect(0, 0, 960, 540, rgb(74, 146, 104));
        fx.draw(&mut fb);
        let c = fb.px()[270 * 960 + 480];
        let (r, g, b) = (render::palette::r(c), render::palette::g(c), render::palette::b(c));
        assert!(
            r > 110 && b > 150,
            "flash wash expected over the green fill, got ({r}, {g}, {b})"
        );
    }

    /// The envelope itself: zero at t=0, rising through the ~35 ms attack,
    /// monotonically decaying afterwards, always in [0, peak].
    #[test]
    fn envelope_shape() {
        let mut f = Flash {
            t: 0.0,
            peak: 0.5,
            dur: 0.6,
            next_in: 0.0,
            live: false,
        };
        f.begin(0.5, 0.6);
        assert_eq!(f.alpha(), 0.0, "alpha at t=0");
        let mut prev: f32 = 0.0;
        let mut peaked: f32 = 0.0;
        let mut decaying = true;
        for i in 1..=40 {
            f.t = i as f32 * 0.01; // 10 ms steps, 400 ms
            let a = f.alpha();
            assert!(a >= 0.0, "alpha negative at t={}: {a}", f.t);
            assert!(a <= 0.5);
            if f.t > 0.04 {
                decaying &= a <= prev + 1e-6;
            }
            peaked = peaked.max(a);
            prev = a;
        }
        assert!(peaked > 0.45, "attack should reach ~full peak: {peaked}");
        assert!(decaying, "decay must be monotonic after the attack");
        assert!(f.t > 0.04 && f.alpha() < 0.2, "should be decaying by 400 ms");
    }
}

#[cfg(test)]
mod cost {
    use super::*;
    #[ignore]
    #[test]
    fn component_costs() {
        let mut fb = Framebuffer::new(960, 540);
        for row in 0..27 {
            fb.fill_rect(0, row as i32 * 20, 960, 20, rgb(60 + row as u8, 100, 70));
        }
        // rain only: heavy precip, low cloud (no dim/shadow), no visibility
        let rain_only = { let mut w = world::Weather::default(); w.code = 63; w.precip = 3.0; w.cloud = 30.0; w.wind = 30.0; w.wind_dir = Some(250.0); w };
        let mut fx = Fx::new(1, 960, 540);
        for _ in 0..30 { fx.update(1.0/30.0, &rain_only); fx.draw(&mut fb); }
        let t = std::time::Instant::now();
        for _ in 0..300 { fx.update(1.0/30.0, &rain_only); fx.draw(&mut fb); }
        let rain_ns = t.elapsed().as_nanos() as f64 / 300.0;

        // wash only: overcast dim (cloud 100) no precip, plus fog band
        let wash = { let mut w = world::Weather::default(); w.code = 0; w.cloud = 100.0; w.visibility = Some(1500.0); w };
        let mut fx2 = Fx::new(1, 960, 540);
        for _ in 0..30 { fx2.update(1.0/30.0, &wash); fx2.draw(&mut fb); }
        let t = std::time::Instant::now();
        for _ in 0..300 { fx2.update(1.0/30.0, &wash); fx2.draw(&mut fb); }
        let wash_ns = t.elapsed().as_nanos() as f64 / 300.0;

        // dim only
        let dim_only = { let mut w = world::Weather::default(); w.code = 0; w.cloud = 100.0; w };
        let mut fx3 = Fx::new(1, 960, 540);
        for _ in 0..30 { fx3.update(1.0/30.0, &dim_only); fx3.draw(&mut fb); }
        let t = std::time::Instant::now();
        for _ in 0..300 { fx3.update(1.0/30.0, &dim_only); fx3.draw(&mut fb); }
        let dim_ns = t.elapsed().as_nanos() as f64 / 300.0;

        println!("rain-only (drops+splash): {rain_ns:.0} ns");
        println!("wash  (dim full + fog band): {wash_ns:.0} ns");
        println!("dim only (full frame): {dim_ns:.0} ns");
    }

    /// D66: what a sandstorm costs at the owner's desktop union (7040x1440), the same
    /// frame the wallpaper composites. Run with
    /// `cargo test -p weather_fx --release sandstorm_cost -- --ignored --nocapture`.
    #[test]
    #[ignore = "timing measurement, run with --ignored"]
    fn sandstorm_cost() {
        let (w, h) = (7040usize, 1440usize);
        let mut fb = Framebuffer::new(w, h);
        let wx = crate::look::preset("sandstorm").unwrap();
        let mut fx = Fx::new(5, w, h);
        for _ in 0..120 {
            fx.update(1.0 / 30.0, &wx);
            fx.draw(&mut fb);
        }
        assert!(fx.is_active());
        let t = std::time::Instant::now();
        for _ in 0..120 {
            fx.update(1.0 / 30.0, &wx);
            fx.draw(&mut fb);
        }
        let ms = t.elapsed().as_secs_f64() * 1000.0 / 120.0;
        println!("sandstorm overlay at {w}x{h}: {ms:.2} ms/frame");

        // Where the cost goes: the same frame with the same cloud and fog but no
        // dust at all. If the two are the same, the dust pass is free and the
        // bill is the engine's existing full-frame fog wash.
        let mut fog = wx;
        fog.code = 45;
        fog.precip = 0.0;
        let mut fx2 = Fx::new(5, w, h);
        for _ in 0..120 {
            fx2.update(1.0 / 30.0, &fog);
            fx2.draw(&mut fb);
        }
        let t = std::time::Instant::now();
        for _ in 0..120 {
            fx2.update(1.0 / 30.0, &fog);
            fx2.draw(&mut fb);
        }
        let fog_ms = t.elapsed().as_secs_f64() * 1000.0 / 120.0;
        println!("the same fog/cloud without dust: {fog_ms:.2} ms/frame");
        // The dust pass must not be what costs: it is ~20 k streak pixels.
        assert!(ms - fog_ms < 2.0, "the dust pass costs {:.2} ms", ms - fog_ms);
        assert!(ms < 16.0, "a sandstorm frame costs {ms:.2} ms");
    }
}
