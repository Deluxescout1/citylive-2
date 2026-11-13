//! Sky drawing (B12): vertical gradient + sun/moon discs (B45: the real
//! night sky — catalogue stars, Milky Way, planets, comet, eclipse shading)
//! into a [`render::Framebuffer`].
//!
//! Cheap on purpose: the gradient is one fill per row (no per-pixel mixing),
//! and the night field is a [`NightSky`] cache — the projections (a few
//! thousand alt/az conversions) are recomputed **at most once a minute** (B45
//! cost rule); every sky redraw after that only replays cached blits.

use crate::{Palette, SkyState};
use render::Horizon;
use render::{Framebuffer, palette};
use world::astro;
use world::moon;

/// The city's own glow (B45): how far the sky's faint end is drowned, from
/// the population the sim reports and the hour (a city dims its lights in
/// the small hours — the old `lpK = cityG*(0.30+0.70*lpLit)`).
pub fn light_pollution_for(s: &SkyState, population: Option<u32>) -> f32 {
    let Some(pop) = population else {
        return 0.08; // no city: a dark site, the full Milky Way.
    };
    // Growth anchors on the sim's era thresholds (settlement → metropolis).
    let growth = match pop {
        0..=10 => 0.06,
        _ => {
            const ANCH: [(f64, f32); 5] = [
                (40.0, 0.12),
                (140.0, 0.20),
                (1_200.0, 0.40),
                (4_000.0, 0.60),
                (20_000.0, 0.80),
            ];
            let p = pop as f64;
            if p >= ANCH[4].0 {
                0.85
            } else {
                let mut g = ANCH[0].1;
                for w in ANCH.windows(2) {
                    if p >= w[0].0 && p < w[1].0 {
                        let t = ((p.log10() - w[0].0.log10()) / (w[1].0.log10() - w[0].0.log10()))
                            as f32;
                        g = w[0].1 + (w[1].1 - w[0].1) * t;
                        break;
                    }
                }
                g
            }
        }
    };
    let hour = ((s.ms as f64 / 3_600_000.0 + s.lon / 15.0).rem_euclid(24.0)) as f32;
    let lit = if (1.0..5.0).contains(&hour) {
        0.55 // the deep-night lull
    } else {
        1.0
    };
    (growth * (0.30 + 0.70 * lit)).min(1.0)
}

// ---------------------------------------------------------------- NightSky cache

/// One cached star blit (screen space, post-projection).
struct StarBlit {
    x: i16,
    y: i16,
    c: u32,
    a: f32,
    /// 0 = 1-px dot, 1 = 3-px diffraction cross, 2 = 5-px bright spikes.
    kind: u8,
}

/// One Milky Way dab.
struct DabBlit {
    x: i16,
    y: i16,
    c: u32,
    a: f32,
    sz: u8,
}

/// A planet: a steady coloured dot (no twinkle — that is how you tell).
struct PlanetBlit {
    x: i16,
    y: i16,
    c: u32,
    a: f32,
    glow: bool,
}

/// The comet: head position + the anti-sunward tail unit vector.
struct CometBlit {
    x: f32,
    y: f32,
    tx: f32,
    ty: f32,
}

/// The real night sky for one geometry + instant: everything projected to
/// screen space once per minute (or per geometry change) and replayed as
/// cached blits. Owned by the caller (`present` keeps one in its state; the
/// tools build one per render).
#[derive(Default)]
pub struct NightSky {
    key: (i64, u64, u64, u32, u32, i32, u16),
    stars: Vec<StarBlit>,
    dabs: Vec<DabBlit>,
    planets: Vec<PlanetBlit>,
    comet: Option<CometBlit>,
    /// Andromeda (M31): the farthest thing a naked eye can reach — an
    /// elongated smudge at its real position.
    andromeda: Option<(i16, i16, f32)>,
    /// A lunar eclipse in progress: the umbral bite (0..1) and whether the
    /// disc has gone blood-red (near totality). The Moon's position itself is
    /// two trig ops on the caller's `SkyState` — computed per redraw.
    moon: Option<(f32, bool)>, // shadow bite 0..1, blood
    /// A solar eclipse in progress over this observer: the dark disc's
    /// offset in sun radii, its size ratio, and the obscuration.
    sun_bite: Option<(f32, f32, f32, f32)>,
    /// The active shower's radiant in band units (drives meteor directions).
    radiant: Option<(f32, f32, f32)>, // x01, y01, strength
    /// The sky clock the cache was built for (meteor slot windows).
    ms_sky: i64,
    /// Whether the last rebuild projected the night field (vs eclipse-only).
    built_night: bool,
}

/// The band mapping (B45 item 5): the visible sky band covers the
/// south-facing half — azimuth wraps the full width with SOUTH at the centre
/// (east left, west right — the same orientation as the sun's
/// [`screen_pos_at`]), and the north behind the viewer folding to the two
/// edges.
#[inline]
pub fn band_x(az_deg: f64, w: usize) -> f32 {
    let az = az_deg.rem_euclid(360.0);
    (az / 360.0) as f32 * w as f32
}

#[inline]
pub fn band_y(alt_deg: f64, horizon_y: i32, band_h: usize) -> f32 {
    let u = ((alt_deg / 90.0).clamp(0.0, 1.0)) as f32;
    horizon_y as f32 - u * (horizon_y as f32 - 0.04 * band_h as f32)
}

/// How much of the band row `y` is true sky when the compositor is done with
/// it (B45b item 2): the land is painted `HORIZON_FADE` rows into the band and
/// then dissolved back towards the exact sky pixels (
/// `Scene::fade_horizon`), least at the land line. Multiplying the night
/// field by that same curve keeps stars, dabs and planets off the treeline —
/// they blend out behind the horizon layer instead of shining through it.
#[inline]
fn sky_weight(y: f32, horizon_y: i32) -> f32 {
    let fade = render::scene::HORIZON_FADE as f32;
    let u = ((horizon_y as f32 - y) / fade).clamp(0.0, 1.0);
    u * u * (3.0 - 2.0 * u)
}

/// A deterministic per-segment jitter 0..1 for the Milky Way bed (B45b
/// item 1): build-time only, so the replay stays a plain blit. Splitmix64's
/// avalanche steps.
#[inline]
fn jitter01(x: u64) -> f32 {
    let mut z = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z >> 11) as f32 / (1u64 << 53) as f32
}

impl NightSky {
    /// The sky clock (ms) this cache was built for — the meteor slot machine
    /// reads it to know which shower's window the night is in.
    pub fn ms_sky(&self) -> i64 {
        self.ms_sky
    }

    /// Rebuild the cache if the minute, the location, the geometry or the
    /// light pollution moved; otherwise this is a no-op. `night_visible`
    /// (the star fade) skips the catalogue/Milky-Way/planet projection when
    /// the sky is bright — a day redraw only needs the eclipse state.
    fn ensure(
        &mut self,
        s: &SkyState,
        fb_w: usize,
        fb_h: usize,
        horizon_y: i32,
        light_pollution: f32,
        night_visible: bool,
    ) {
        if fb_w == 0 || fb_h == 0 {
            return;
        }
        let key = (
            s.ms.div_euclid(60_000),
            s.lat.to_bits(),
            s.lon.to_bits(),
            fb_w as u32,
            fb_h as u32,
            horizon_y,
            (light_pollution * 64.0) as u16,
        );
        if key == self.key && self.built_night == night_visible {
            return;
        }
        self.key = key;
        self.ms_sky = s.ms;
        self.built_night = night_visible;
        self.rebuild(s, fb_w, fb_h, horizon_y, light_pollution, night_visible);
    }

    fn rebuild(&mut self, s: &SkyState, w: usize, h: usize, horizon_y: i32, lp: f32, night_visible: bool) {
        self.stars.clear();
        self.dabs.clear();
        self.planets.clear();
        self.comet = None;
        self.andromeda = None;
        self.moon = None;
        self.sun_bite = None;
        self.radiant = None;

        let lst = moon::lst_hours(s.ms, s.lon);
        let lat = s.lat;
        let lp = lp.clamp(0.0, 1.0);

        // Eclipses (the expensive Moon series — this is the once-a-minute
        // path, never per frame).
        let ecl = astro::Eclipses::at(s.ms, lat, s.lon);
        if ecl.lunar.visible && ecl.lunar.umb_mag > 0.0 {
            let bite = ecl.lunar.umb_mag.clamp(0.0, 1.0) as f32;
            let blood = ecl.lunar.umb_mag >= 0.9;
            self.moon = Some((bite, blood));
        }
        if ecl.solar.visible && ecl.solar.obsc > 0.003 {
            self.sun_bite = Some((ecl.solar.dx as f32, ecl.solar.dy as f32, ecl.solar.moon_ratio as f32, ecl.solar.obsc as f32));
        }

        // ---- the catalogue (and everything else that needs the band) ------
        // B45b: during the day the night field is not drawn at all (the
        // replay is gated on the star fade), so skip the whole projection and
        // keep only the eclipse state above.
        if !night_visible {
            return;
        }

        // B45b item 3: the asterism *lines* (the old dotted hints between
        // named stars) are gone — recognisable constellations come from the
        // real star positions alone. The named stars themselves carry the
        // shapes: every named star gets at least the bright 4-point cross so
        // the constellation's outline reads at a glance.

        for (si, st) in astro::CATALOG.iter().enumerate() {
            let (alt, az) = moon::alt_az(st.ra_hours(), st.dec_deg(), lst, lat);
            if alt < 2.0 {
                continue;
            }
            let mag = st.mag();
            // The city glow drowns the faint end of the catalogue first; the
            // low sky washes out before the zenith.
            let pollution = 1.0 - lp as f64 * (((mag - 0.5) / 4.0).clamp(0.0, 1.0) * 0.95);
            let alt_vis = ((alt - 2.0) / 22.0).clamp(0.10, 1.0);
            let mut a = (((5.8 - mag) / 4.6).clamp(0.0, 1.0) * alt_vis * pollution) as f32;
            let y = band_y(alt, horizon_y, h);
            // B45b item 2: behind the land's dissolve the sky is not on the
            // screen any more — the blit fades out rather than showing over
            // the treeline.
            a *= sky_weight(y, horizon_y);
            if a < 0.04 {
                continue;
            }
            let (cr, cg, cb) = astro::star_colour(st.bv());
            // Hot white-blue core for the bright ones (the old code's
            // diffraction-cross look); named stars at least a 4-point cross
            // (B45b item 3) so the constellation outline reads.
            let named = astro::NAMED
                .iter()
                .any(|n| n.idx as usize == si);
            let (kind, c) = if mag < 0.8 {
                (2, palette::rgb(242, 246, 255))
            } else if mag < 1.6 || named {
                (1, palette::mix(palette::rgb(cr, cg, cb), 0xF2F6FF, 0.55))
            } else {
                (0, palette::rgb(cr, cg, cb))
            };
            self.stars.push(StarBlit {
                x: band_x(az, w) as i16,
                y: y as i16,
                c,
                a,
                kind,
            });
        }

        // ---- the Milky Way --------------------------------------------------
        // B45b item 1: **faint, grainy, irregular — never a solid shape.**
        // The previous draw built the bed as 7 overlapping spine strokes;
        // on a real 144-px band (`scale` clamps to 0.36) the offsets
        // collapse on top of each other, the strokes stack to alpha ~1 (the
        // reviewed pale slab), and steep spine climbs stair-step into solid
        // vertical bars (the reviewed end drips). So the bed is the dab
        // field alone — one blit per star-cloud grain, alpha jittered per
        // dab and hard-capped, so counting overlaps still reads as a
        // mottled river. The spine is projected only to decide which dabs
        // carry the course (the per-dab weight below).
        let (spine, dabs) = astro::milky_way();
        let _ = spine;
        let wash = 1.0 - lp; // the city glow washes the band out
        if wash > 0.04 {
            // Pass: the mottled dab field (star clouds warm-gold near the
            // core, blue-white elsewhere, the Great Rift carved at build).
            for (di, d) in dabs.iter().enumerate() {
                let (alt, az) = moon::alt_az(
                    d.ra_ms_h as f64 / 1000.0,
                    d.dec_cd as f64 / 100.0,
                    lst,
                    lat,
                );
                if alt < 2.0 {
                    continue;
                }
                let boost = if alt > 50.0 { 1.4 } else { 1.0 };
                let j = 0.60 + 0.40 * jitter01(0xD0B1_0000 + di as u64 + (d.sz as u64) * 1_000_003);
                let mut a = ((d.w_th as f32 / 1000.0) * wash * boost).min(1.0) * 1.35 * j;
                if a < 0.03 {
                    continue;
                }
                let c = match d.class {
                    2 => palette::rgb(255, 164, 186), // rose H-alpha nebulae
                    1 => palette::rgb(255, 226, 172), // warm-gold core clouds
                    _ => palette::rgb(206, 220, 255), // blue-white starlight
                };
                let y = band_y(alt, horizon_y, h);
                a *= sky_weight(y, horizon_y);
                if a < 0.03 {
                    continue;
                }
                self.dabs.push(DabBlit {
                    x: band_x(az, w) as i16,
                    y: y as i16,
                    c,
                    a: a.min(0.30),
                    sz: d.sz,
                });
            }
            // Andromeda (M31) at RA 0.712h, Dec +41.27 — a faint smudge.
            let (alt, az) = moon::alt_az(0.712, 41.27, lst, lat);
            if alt > 3.0 {
                let boost = if alt > 50.0 { 1.5 } else { 1.0 };
                let mut a = (0.46 * wash * boost) as f32;
                if a > 0.02 {
                    let y = band_y(alt, horizon_y, h);
                    a *= sky_weight(y, horizon_y);
                    if a > 0.02 {
                        self.andromeda =
                            Some((band_x(az, w) as i16, y as i16, a));
                    }
                }
            }
        }

        // ---- planets (steady points in their real places) -------------------
        for p in astro::PLANETS {
            let rd = astro::planet_ra_dec(p, s.ms);
            let (alt, az) = moon::alt_az(rd.ra, rd.dec, lst, lat);
            if alt < 2.0 {
                continue;
            }
            let alt_vis = ((alt - 2.0) / 22.0).clamp(0.25, 1.0);
            // Planets resist the city glow more than the faint stars.
            let mut a = (alt_vis * (1.0 - 0.55 * lp as f64)) as f32;
            let (pr, pg, pb) = p.colour();
            let bright = matches!(p, astro::Planet::Venus | astro::Planet::Jupiter);
            let y = band_y(alt, horizon_y, h);
            a *= sky_weight(y, horizon_y);
            if a < 0.04 {
                continue;
            }
            self.planets.push(PlanetBlit {
                x: band_x(az, w) as i16,
                y: y as i16,
                c: palette::rgb(pr, pg, pb),
                a: a * if bright { 1.0 } else { 0.85 },
                glow: bright,
            });
        }

        // ---- the great comet (its seasons only) -----------------------------
        if let Some(c) = astro::comet_now(s.ms) {
            let (alt, az) = moon::alt_az(c.ra, c.dec, lst, lat);
            if alt > 2.0 {
                let y = band_y(alt, horizon_y, h);
                // B45b item 2: a comet low behind the land is not on screen —
                // the head weight gates the whole object.
                if sky_weight(y, horizon_y) > 0.1 {
                    let hx = band_x(az, w);
                    // The tail points away from the Sun.
                    let sun = astro::sun_ra_dec(s.ms);
                    let (salt, saz) = moon::alt_az(sun.ra, sun.dec, lst, lat);
                    let (sx, sy) = (band_x(saz, w), band_y(salt, horizon_y, h));
                    let (mut tx, mut ty) = (hx - sx, y - sy);
                    let n = (tx * tx + ty * ty).sqrt().max(1.0);
                    tx /= n;
                    ty /= n;
                    self.comet = Some(CometBlit { x: hx, y, tx, ty });
                }
            }
        }

        // ---- the shower radiant (meteor directions) --------------------------
        if let Some(sh) = astro::current_shower(s.ms) {
            let (alt, az) = moon::alt_az(sh.ra, sh.dec, lst, lat);
            self.radiant = Some((band_x(az, w) / w as f32, band_y(alt, horizon_y, h) / h as f32, sh.s as f32));
        }
    }
}

// ---------------------------------------------------------------- the drawing

/// Draw the full sky (gradient + real night field + sun + moon) into `fb`,
/// over all of it (respecting the current clip). Deterministic in `seed`.
pub fn draw(s: &SkyState, pal: &Palette, fb: &mut Framebuffer, seed: u64) {
    draw_with_horizon(s, pal, fb, seed, (fb.h as f32 * 0.72) as i32, 0);
}

/// Draw into `fb` with the sky occupying everything above `horizon_y` (D43):
/// the gradient ends there, the sun and moon sink to it, and the backdrop
/// ridges stand on it so the sun slides behind them.
pub fn draw_with_horizon(
    s: &SkyState,
    pal: &Palette,
    fb: &mut Framebuffer,
    seed: u64,
    horizon_y: i32,
    wind: i32,
) {
    // The compat path (tools/one-shot renders): a throwaway cache, a modest
    // town's glow. `present` owns a persistent one.
    let mut night = NightSky::default();
    draw_with_treatment(s, pal, fb, seed, horizon_y, wind, Horizon::default(), &mut night, 0.25);
}

/// As [`draw_with_horizon`], with the world's horizon treatment (D64) and the
/// real night sky: `night` caches the star/planet/Milky-Way projections
/// (rebuilt at most once a minute), `light_pollution` 0..1 washes the band
/// and the faint stars out as the city grows.
pub fn draw_with_treatment(
    s: &SkyState,
    pal: &Palette,
    fb: &mut Framebuffer,
    seed: u64,
    horizon_y: i32,
    wind: i32,
    horizon: Horizon,
    night: &mut NightSky,
    light_pollution: f32,
) {
    gradient_to(pal, fb, horizon_y);
    // B28's rule for the discs applies to the night field too: a solid deck
    // hides the stars, a thick overcast thins them.
    let cover = s.cloud.clamp(0.0, 1.0);
    let fade = pal.star_alpha * (1.0 - cover);
    // The night field is only projected when the sky is faint enough to
    // show it (B45b): a day rebuild keeps the eclipse state and skips the
    // few thousand alt/az conversions outright.
    night.ensure(s, fb.w, fb.h, horizon_y, light_pollution, fade > 0.02);
    // The last mutable use of `night` was `ensure`; from here it is read-only.
    let night = &*night;
    if fade > 0.02 {
        replay_night(night, fade, fb);
    }
    // B28 item 1: a solid deck hides the disc; a thick overcast leaves only a
    // diffuse brightening where the sun would be.
    let sun_xy = screen_pos_at(s.sun_alt, s.sun_az, fb.w, fb.h, horizon_y);
    if s.sun_alt > -0.833 {
        if cover < 0.92 {
            glow(sun_xy, pal, fb);
        }
        if cover < 0.72 {
            sun_disc_at(sun_xy, s, fb);
            // A solar eclipse in progress: the Moon's dark disc slides across
            // the Sun (and the glow above was dimmed by the obscuration).
            if let Some((dx, dy, ratio, _obsc)) = night.sun_bite {
                let (cx, cy) = sun_xy;
                let r = 11.0;
                let mx = cx + dx * r;
                let my = cy + dy * r;
                let mr = (r * ratio).round() as i32;
                for ddy in -mr..=mr {
                    for ddx in -mr..=mr {
                        if ddx * ddx + ddy * ddy <= mr * mr {
                            fb.put(mx as i32 + ddx, my as i32 + ddy, 0x14161E);
                        }
                    }
                }
            }
        }
    }
    if s.moon_alt > -0.833 && cover < 0.72 {
        // The Moon at its real alt/az on the band mapping (B45) — two trig
        // ops on the caller's state, always fresh — with the cached eclipse
        // state (a partial takes a dark bite; near totality the disc goes
        // blood red). B45b: like the stars, the disc is clipped to true sky —
        // the land dissolve eats it as the Moon sinks (weight → 0 at the
        // horizon row).
        let (x, y) = (
            band_x(s.moon_az, fb.w),
            band_y(s.moon_alt, horizon_y, fb.h),
        );
        let (bite, blood) = night.moon.unwrap_or((0.0, false));
        let wgt = sky_weight(y, horizon_y);
        if wgt > 0.02 {
            moon_disc_xy((x, y), s, fb, bite, blood, wgt);
        }
    }
    // Ridges last: the sun and moon slide behind them (D43 item 4).
    crate::backdrop::draw(s, pal, fb, seed, horizon_y, wind, horizon);
}

/// Replay the cached night field. Only blits — no trig, no allocation.
fn replay_night(night: &NightSky, fade: f32, fb: &mut Framebuffer) {
    // The Milky Way: the dab field (grainy, irregular, capped faint).
    for d in night.dabs.iter() {
        let a = d.a * fade;
        if a < 0.03 {
            continue;
        }
        let sz = d.sz as i32;
        fb.fill_blended(d.x as i32, d.y as i32, sz, sz, d.c, a);
    }
    // Andromeda: an elongated smudge.
    if let Some((x, y, a)) = night.andromeda {
        let a = a * fade;
        if a > 0.02 {
            let inner = palette::with_alpha(palette::rgb(236, 240, 255), 255);
            let (x, y) = (x as i32, y as i32);
            fb.fill_blended(x - 2, y, 5, 2, palette::rgb(208, 212, 238), a * 0.55);
            fb.put_blended(x, y, inner, a);
        }
    }
    // Catalogue stars.
    for st in night.stars.iter() {
        let a = st.a * fade;
        if a < 0.04 {
            continue;
        }
        match st.kind {
            2 => {
                // The very brightest: longer spikes + hot core.
                let (x, y) = (st.x as i32, st.y as i32);
                fb.put_blended(x - 2, y, 0xBED6FF, 0.28 * a);
                fb.put_blended(x + 2, y, 0xBED6FF, 0.28 * a);
                fb.put_blended(x, y - 2, 0xBED6FF, 0.28 * a);
                fb.put_blended(x, y + 2, 0xBED6FF, 0.28 * a);
                fb.put_blended(x, y, st.c, 0.95 * a);
            }
            1 => {
                // Bright: a soft 4-point cross, not a square box.
                let (x, y) = (st.x as i32, st.y as i32);
                fb.put_blended(x - 1, y, 0xBED6FF, 0.22 * a);
                fb.put_blended(x + 1, y, 0xBED6FF, 0.22 * a);
                fb.put_blended(x, y - 1, 0xBED6FF, 0.22 * a);
                fb.put_blended(x, y + 1, 0xBED6FF, 0.22 * a);
                fb.put_blended(x, y, st.c, 0.9 * a);
            }
            _ => fb.put_blended(st.x as i32, st.y as i32, st.c, a),
        }
    }
    // Planets: steady coloured points, the bright pair with a soft cross.
    for p in night.planets.iter() {
        let a = p.a * fade;
        if a < 0.05 {
            continue;
        }
        let (x, y) = (p.x as i32, p.y as i32);
        if p.glow {
            fb.put_blended(x - 1, y, 0xBED6FF, 0.25 * a);
            fb.put_blended(x + 1, y, 0xBED6FF, 0.25 * a);
            fb.put_blended(x, y - 1, 0xBED6FF, 0.25 * a);
            fb.put_blended(x, y + 1, 0xBED6FF, 0.25 * a);
        }
        fb.put_blended(x, y, p.c, 0.95 * a);
    }
    // The great comet: a bright head and a long anti-sunward tail.
    if let Some(c) = &night.comet {
        for t in 0..28i32 {
            let a = 0.5 * (1.0 - t as f32 / 28.0) * fade;
            fb.put_blended(
                (c.x + c.tx * (t as f32 * 1.5)) as i32,
                (c.y + c.ty * (t as f32 * 1.5)) as i32,
                0xB4DCFF,
                a,
            );
        }
        fb.fill_blended(c.x as i32, c.y as i32, 2, 2, 0xEBF5FF, 0.95 * fade);
    }
}

// ---------------------------------------------------------------- meteors

/// The meteor layer: streaks alive at `wall_ms` (real wall clock, so they
/// animate even with a `--time` override), drawn into the *composite* frame
/// (rows 0..`band_h` — present's current_fb, not the sky fb, because the sky
/// band is only recomposited once a minute). Returns the swept rect.
pub fn meteor_layer(
    fb: &mut Framebuffer,
    sky: &Framebuffer,
    night: &NightSky,
    wall_ms: i64,
    band_h: i32,
    fade: f32,
) -> Option<render::Rect> {
    if fade < 0.3 || fb.w == 0 || band_h <= 0 {
        return None;
    }
    let streaks = astro::meteors_now(wall_ms, night.ms_sky);
    if streaks.is_empty() {
        return None;
    }
    let w = fb.w as f32;
    let h = band_h as f32;
    let mut bounds: Option<render::Rect> = None;
    for m in streaks {
        let (dx, dy) = if m.shower {
            // Away from the radiant, in band space.
            let (rx, ry) = match night.radiant {
                Some((rx, ry, _)) => (rx, ry),
                None => (0.5, 0.3),
            };
            let (vx, vy) = (m.x01 - rx, m.y01 - ry);
            let n = (vx * vx + vy * vy).sqrt().max(0.001);
            (vx / n, vy / n)
        } else {
            // A sporadic: flatter than the shower streaks.
            (m.angle.cos(), m.angle.sin() * 0.55)
        };
        // 11 points along the path, fading behind the head (the old 850 ms
        // streak, dot-for-dot).
        for tl in 0..11i32 {
            let tp = m.prog - tl as f32 * 0.028;
            if tp < 0.0 {
                break;
            }
            let x = (m.x01 + tp * m.travel * dx) * w;
            let y = (m.y01 + tp * m.travel * dy) * h;
            let a = 0.9 * (1.0 - tl as f32 / 11.0) * (1.0 - (m.prog - 0.5).abs() * 0.6)
                * m.bright
                * fade;
            if a < 0.02 {
                continue;
            }
            let sz = if tl < 2 { 2 } else { 1 };
            // B45c: only pixels that are still bare sky. Land, treeline,
            // props and buildings differ from the sky layer and are skipped.
            for xx in x as i32..x as i32 + sz {
                let yy = y as i32;
                if xx < 0 || yy < 0 || xx >= fb.w as i32 || yy >= band_h || yy >= sky.h as i32 {
                    continue;
                }
                if sky.w != fb.w {
                    continue;
                }
                let i = yy as usize * fb.w + xx as usize;
                if (fb.px()[i] ^ sky.px()[i]) & 0x00FF_FFFF != 0 {
                    continue;
                }
                fb.put_blended(xx, yy, 0xFFFCE1, a);
            }
            bounds = expand(bounds, x as i32, y as i32, sz);
        }
    }
    bounds
}

#[inline]
fn expand(r: Option<render::Rect>, x: i32, y: i32, pad: i32) -> Option<render::Rect> {
    let p = pad + 1;
    match r {
        None => Some(render::Rect::new(x - p, y - p, x + p, y + p)),
        Some(r) => Some(render::Rect::new(
            r.x0.min(x - p),
            r.y0.min(y - p),
            r.x1.max(x + p),
            r.y1.max(y + p),
        )),
    }
}

// ---------------------------------------------------------------- gradient

fn gradient_to(pal: &Palette, fb: &mut Framebuffer, horizon_y: i32) {
    if fb.h == 0 {
        // B35: a scene with no sky band hands the sky a zero-height buffer.
        return;
    }
    // The gradient has to run over the *sky band* (D43/B20), not the whole
    // frame: with a 24 % band the old full-height ramp put nothing but the
    // zenith colour in it, which is why the band came out flat.
    let hor = horizon_y.clamp(1, fb.h as i32);
    // A warm horizon deepens the top of the sky towards blue/violet (B20
    // answer 4: deep blue/violet → orange/gold), the way a real sunset does;
    // a neutral or night sky is left alone.
    let warm = ((palette::r(pal.horizon) as i32 - palette::b(pal.horizon) as i32) as f32 / 120.0)
        .clamp(0.0, 1.0);
    let top = palette::mix(pal.zenith, 0x3D3A78, 0.55 * warm);
    // The middle of a sunset sky is dustier than either anchor, so pull it
    // towards its own grey (the mock-ups' mauve, not hot pink).
    let mid0 = palette::mix(pal.zenith, pal.horizon, 0.50);
    let grey = {
        let l = (0.30 * palette::r(mid0) as f32
            + 0.59 * palette::g(mid0) as f32
            + 0.11 * palette::b(mid0) as f32) as u8;
        palette::rgb(l, l, l)
    };
    let mid = palette::mix(mid0, grey, 0.30 * warm);
    for y in 0..fb.h as i32 {
        let u = (y as f32 / hor as f32).clamp(0.0, 1.0);
        let c = if u < 0.6 {
            let t = u / 0.6;
            palette::mix(top, mid, t * t * (3.0 - 2.0 * t))
        } else {
            let t = (u - 0.6) / 0.4;
            palette::mix(mid, pal.horizon, t * t * (3.0 - 2.0 * t))
        };
        fb.fill_rect(0, y, fb.w as i32, 1, c);
    }
}

/// Screen position from alt/az (deg), for a fixed south-facing camera at
/// mid-northern latitudes (the view the city uses, P3/D7): south = centre of
/// the horizon, east = left, west = right; the horizon sits at 72% of the
/// frame height and the zenith at 5%.
pub fn screen_pos_at(alt: f64, az: f64, w: usize, h: usize, horizon_y: i32) -> (f32, f32) {
    let hor_y = horizon_y as f32;
    let top_y = 0.04 * h as f32;
    let from_south = (az - 180.0).abs().min(180.0);
    let east = az < 180.0;
    let f = (from_south / 180.0) as f32 * 0.42;
    let x = if east { 0.5 - f } else { 0.5 + f };
    // Altitude → height in the band, generous on purpose (B20 answer 2): the
    // band is only ~24 % of the frame, so a sun 2-3° up must already clear
    // the ridge tops (the far crest is ~a third of the band tall) and only
    // the last degree goes behind them.
    let a = (alt.max(0.0) / 14.0).clamp(0.0, 1.0).sqrt() as f32;
    let y = hor_y - a * (hor_y - top_y);
    (x * w as f32, y)
}

/// Soft glow around the sun plus a band of horizon light at its azimuth.
fn glow(pos: (f32, f32), pal: &Palette, fb: &mut Framebuffer) {
    let (cx, cy) = (pos.0 as i32, pos.1 as i32);
    let warm = palette::mix(pal.horizon, 0xFFD9A0, 0.75);
    // A wide halo with a smooth quadratic falloff: an annulus per ring left
    // visible concentric circles (B20 review).
    const R: i32 = 72;
    for dy in -R..=R {
        for dx in -R..=R {
            let d = ((dx * dx + dy * dy) as f32).sqrt();
            if d > R as f32 {
                continue;
            }
            let f = 1.0 - d / R as f32;
            let a = 0.34 * f * f;
            if a > 0.004 {
                fb.put_blended(cx + dx, cy + dy, warm, a);
            }
        }
    }
    // Brightest strip of horizon at the sun's column.
    let w = fb.w as i32;
    for y in (cy + 8)..(cy + 40) {
        let f = 1.0 - ((y - cy - 8) as f32 / 32.0).clamp(0.0, 1.0);
        for x in (cx - w / 4)..(cx + w / 4) {
            let d = (x - cx).abs() as f32 / (w as f32 / 4.0);
            let a = f * (1.0 - d).clamp(0.0, 1.0) * 0.12;
            if a > 0.01 {
                fb.put_blended(x, y, warm, a);
            }
        }
    }
}

fn sun_disc_at(pos: (f32, f32), s: &SkyState, fb: &mut Framebuffer) {
    sun_disc_xy(pos, s, fb);
}

fn sun_disc_xy(pos: (f32, f32), s: &SkyState, fb: &mut Framebuffer) {
    let (cx, cy) = pos;
    let cx = cx.round() as i32;
    let cy = cy.round() as i32;
    // B20: larger and flattened near the horizon (a setting sun looks oval).
    let r = 11;
    let squash = if s.sun_alt < 12.0 { 0.60 } else { 0.85 };
    // Warmth: white-yellow high up, deep orange when the sun is low.
    let w = ((s.sun_alt / 50.0).clamp(0.0, 1.0)) as f32;
    let col = palette::rgb(
        (255.0) as u8,
        (216.0 + 39.0 * w) as u8,
        (140.0 + 115.0 * w) as u8,
    );
    // Soft glow (a 2-px annulus at low alpha), then the disc. Both are small
    // circle tests, so one 21×21 pass each — cheap.
    let grow: i32 = r + 2;
    for dy in -grow..=grow {
        for dx in -grow..=grow {
            let d = ((dx as f64) * (dx as f64) + (dy as f64) * (dy as f64)).sqrt();
            if d > r as f64 && d <= grow as f64 {
                fb.put_blended(cx + dx, cy + dy, col, 0.18);
            }
        }
    }
    for dy in -r..=r {
        for dx in -r..=r {
            let nx = dx as f32 / r as f32;
            let ny = dy as f32 / (r as f32 * squash);
            if nx * nx + ny * ny <= 1.0 {
                fb.put(cx + dx, cy + dy, col);
            }
        }
    }
}

/// Moon: a disc drawn with its phase. `k = cos(2πp)`; the terminator is
/// the ellipse `dx = k·√(1-dy²)`, mirrored for waxing (lit right) vs waning
/// (lit left). New moon (p≈0) lights nothing, full (p≈0.5) the whole disc.
/// B45: an eclipse takes a dark bite from the lit limb (`bite` = umbral
/// magnitude), and near totality (`blood`) the whole disc goes copper-red —
/// a partial is a bite out of a still-white Moon, which is what a partial
/// actually looks like. `wgt` (B45b item 2) is the compositors' sky weight:
/// the disc fades out behind the land dissolve instead of shining over it.
fn moon_disc_xy(pos: (f32, f32), s: &SkyState, fb: &mut Framebuffer, bite: f32, blood: bool, wgt: f32) {
    let (cx, cy) = pos;
    let cx = cx.round() as i32;
    let cy = cy.round() as i32;
    // Fully on screen → solid stamps, as reviewed; fading → blended.
    let solid = wgt >= 0.999;
    // B62: a blood moon is the spectacle, not a detail: a wider copper disc
    // (17 px across) in a glow, so it reads at 1x on a 1440p monitor.
    let r = if blood { 8 } else { 5 };
    let k = (2.0 * std::f64::consts::PI * s.moon_phase).cos();
    let waxing = s.moon_phase <= 0.5;
    let bright: f32 = 0.55 + 0.45 * s.moon_illum as f32;
    let col = if blood {
        palette::rgb(184, 60, 40)
    } else {
        moon_col(bright)
    };
    let dark_col = if blood {
        palette::rgb(122, 40, 24)
    } else {
        palette::rgb(150, 158, 178)
    };
    let r2 = (r as f32) * (r as f32);
    // A soft halo at night (the old code's lighter-mode rim).
    let halo_col = if blood { 0xC83C28 } else { 0xDCE2F0 };
    let halo_a = (0.05 + 0.14 * s.moon_illum as f32) * bright;
    if blood {
        let reach = 26i32;
        for dy in -reach..=reach {
            for dx in -reach..=reach {
                let d = ((dx * dx + dy * dy) as f32).sqrt();
                if d > r as f32 - 0.5 && d < reach as f32 {
                    let f = 1.0 - (d - r as f32) / (reach as f32 - r as f32);
                    fb.put_blended(cx + dx, cy + dy, 0xC83C28, 0.62 * f.powf(1.6) * wgt);
                }
            }
        }
    }
    for dy in -(r + 2)..=(r + 2) {
        for dx in -(r + 2)..=(r + 2) {
            let d2 = (dx * dx + dy * dy) as f32;
            if d2 > r2 + 2.0 && d2 < (r as f32 + 2.5).powi(2) {
                fb.put_blended(cx + dx, cy + dy, halo_col, halo_a * wgt);
            }
        }
    }
    for dy in -r..=r {
        for dx in -r..=r {
            let d2 = (dx as f32) * (dx as f32) + (dy as f32) * (dy as f32);
            if d2 > r2 + if blood { 2.0 } else { 0.0 } {
                continue;
            }
            let (nx, ny) = (dx as f64 / r as f64, dy as f64 / r as f64);
            let term = k * (1.0 - ny * ny).sqrt();
            let lit = if waxing { nx > term } else { nx < -term };
            let mut c = if lit { col } else { dark_col };
            if blood {
                // Lit from one edge by the sunrise and sunset ringing the Earth
                // (the copper is brightest on the limb nearest the umbra's rim).
                let g = (0.5 - 0.35 * (nx + ny)).clamp(0.0, 1.0) as f32;
                c = palette::mix(palette::rgb(110, 34, 22), palette::rgb(206, 84, 54), g);
            }
            // The umbra eats the disc from one limb (screen-left of the
            // anti-solar edge, like the old draw).
            if !blood && bite > 0.0 {
                let sh_edge = -(r as f32) + 2.0 * r as f32 * bite;
                if (dx as f32) <= sh_edge {
                    c = palette::rgb(96, 44, 36);
                }
            }
            if lit || !blood {
                if solid {
                    fb.put(cx + dx, cy + dy, c);
                } else {
                    fb.put_blended(cx + dx, cy + dy, c, wgt);
                }
            }
        }
    }
}

#[inline]
fn moon_col(bright: f32) -> u32 {
    #[allow(non_upper_case_globals)]
    const MOON_BASE: u32 = 0xA0AEC9;
    let (r, g, b) = (
        palette::r(MOON_BASE) as f32,
        palette::g(MOON_BASE) as f32,
        palette::b(MOON_BASE) as f32,
    );
    palette::rgb(
        (r * bright).round() as u8,
        (g * bright).round() as u8,
        (b * bright).round() as u8,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state;

    const SITE_LAT: f64 = 41.0;
    const SITE_LON: f64 = -75.0;
    // 21 June 2026 00:00 local (EDT).
    const JUN21: i64 = 1_782_014_400_000;

    fn night_state(ms: i64) -> (crate::SkyState, crate::Palette) {
        let s = state(ms, SITE_LAT, SITE_LON);
        let pal = crate::palette(&s);
        (s, pal)
    }

    #[test]
    fn draw_is_deterministic() {
        // Night sky, so the star field (the seed-dependent part) is present.
        let render = |seed: u64| {
            let (s, pal) = night_state(JUN21 + 1 * 3_600_000);
            let mut fb = Framebuffer::new(960, 540);
            draw(&s, &pal, &mut fb, seed);
            fb.px().to_vec()
        };
        assert_eq!(render(42), render(42), "same seed → same image");
        assert_ne!(render(42), render(43), "different seed → different clouds");
    }

    #[test]
    fn night_field_reuses_its_cache() {
        // `ms_sky` is stamped only on rebuild, so it proves whether a draw
        // replayed the cache or rebuilt (the B45 cost rule: at most once a
        // minute).
        let (s, pal) = night_state(JUN21 + 1 * 3_600_000);
        let mut fb = Framebuffer::new(960, 540);
        let hy = fb.h as i32 * 72 / 100;
        let mut night = NightSky::default();
        draw_with_treatment(&s, &pal, &mut fb, 42, hy, 0, Horizon::default(), &mut night, 0.3);
        let n0 = night.stars.len();
        assert!(n0 > 300, "a dark site should show hundreds of stars, got {n0}");
        let mut s2 = s;
        s2.ms += 1_000; // one second later: same minute bucket → no rebuild
        draw_with_treatment(&s2, &pal, &mut fb, 42, hy, 0, Horizon::default(), &mut night, 0.3);
        assert_eq!(night.ms_sky, s.ms, "same minute → cached replay, no rebuild");
        let mut s3 = s;
        s3.ms += 61_000; // a minute later: rebuild
        draw_with_treatment(&s3, &pal, &mut fb, 42, hy, 0, Horizon::default(), &mut night, 0.3);
        assert_eq!(night.ms_sky, s3.ms, "new minute → rebuild");
        let drift = (night.stars.len() as i32 - n0 as i32).abs();
        assert!(drift <= 12, "a minute of rotation moved {drift} stars across the cutoffs");
    }

    #[test]
    fn light_pollution_grows_with_the_city_and_the_night() {
        let (s, _) = night_state(JUN21 + 23 * 3_600_000);
        let empty = light_pollution_for(&s, None);
        let village = light_pollution_for(&s, Some(80));
        let city = light_pollution_for(&s, Some(3_000));
        let metro = light_pollution_for(&s, Some(30_000));
        assert!(empty < village && village < city && city < metro, "{empty} {village} {city} {metro}");
        assert!(metro <= 0.9);
        // The deep-night lull dims the glow.
        let (s3am, _) = night_state(JUN21 + 3 * 3_600_000);
        assert!(light_pollution_for(&s3am, Some(3_000)) < city);
    }

    /// The Perseids frame's instant (2026-08-12 23:00 EDT at the fallback
    /// lon) — the Milky Way crosses high in the band then, so the bed
    /// invariants below have something to bite on.
    fn perseids_ms() -> i64 {
        crate::clock_ms(Some("23:00"), Some("2026-08-12"), -75.0)
    }

    #[test]
    fn orion_is_up_for_the_geminiids_crop() {
        // The 4x Orion deliverable is cut from the Geminids frame; if Orion
        // ever sets before the pinned 23:00 hour, `h45-shots orion_4x` would
        // crop an empty patch (H45 review item). Pin that it cannot.
        use world::moon;
        let ms = crate::clock_ms(Some("23:00"), Some("2026-12-13"), -75.0);
        let lst = moon::lst_hours(ms, -75.0);
        for name in ["Betelgeuse", "Rigel", "Bellatrix", "Saiph", "Alnitak", "Alnilam", "Mintaka"] {
            let Some(n) = astro::NAMED.iter().find(|n| n.name == name) else {
                continue;
            };
            let st = astro::CATALOG[n.idx as usize];
            let (alt, az) = moon::alt_az(st.ra_hours(), st.dec_deg(), lst, 41.0);
            assert!(alt > 2.0, "{name} below the horizon at the Geminids instant ({alt:.0} deg)");
            let x = (az.rem_euclid(360.0) / 360.0) * 2347.0;
            let u = ((alt / 90.0).clamp(0.0, 1.0)) as f32;
            let y = 144.0 - u * (144.0 - 0.04 * 144.0);
            println!("{name}: alt {alt:.1} az {az:.1} -> fb x {x:.0} y {y:.0}");
        }
    }

    #[test]
    fn night_field_blits_never_outshine_the_visible_sky() {
        // B45b item 2: every cached night blit is dimmer than (or equal to)
        // the sky-weight at its own row — nothing renders brighter than the
        // true-sky share of the row, i.e. the field is clipped to sky pixels
        // and fades out behind the land dissolve.
        let (s, pal) = night_state(perseids_ms());
        let mut fb = Framebuffer::new(960, 540);
        let hy = fb.h as i32 * 72 / 100;
        let mut night = NightSky::default();
        draw_with_treatment(&s, &pal, &mut fb, 42, hy, 0, Horizon::default(), &mut night, 0.12);
        assert!(night.stars.len() > 100, "a dark Perseids night needs stars");
        assert!(!night.dabs.is_empty(), "the Milky Way grain should be up");
        for st in night.stars.iter() {
            assert!(st.y < hy as i16 - 2, "a star on the land line row: y={}", st.y);
            assert!(
                st.a <= sky_weight(st.y as f32, hy as i32) + 1e-3,
                "star alpha {} exceeds the sky weight at row {}",
                st.a, st.y
            );
        }
        for d in night.dabs.iter() {
            assert!(d.a <= sky_weight(d.y as f32, hy as i32) + 1e-3);
            assert!(d.a <= 0.30 + 1e-4, "dab alpha above the faint cap: {}", d.a);
        }
        for p in night.planets.iter() {
            assert!(p.a <= sky_weight(p.y as f32, hy as i32) + 1e-3);
        }
    }

    #[test]
    fn a_bright_day_rebuild_skips_the_night_field() {
        // B45b: when the sky is bright the replay is gated on the star fade,
        // so the projection is skipped outright — the cache keeps only the
        // eclipse state. The next night rebuild fills the field again.
        let (s_night, _pal) = night_state(perseids_ms());
        let s = state(crate::clock_ms(Some("12:00"), Some("2026-08-12"), -75.0),
                      SITE_LAT, SITE_LON);
        let mut fb = Framebuffer::new(960, 540);
        let hy = fb.h as i32 * 72 / 100;
        let mut night = NightSky::default();
        night.ensure(&s, fb.w, fb.h, hy, 0.2, false);
        assert!(
            night.stars.is_empty() && night.dabs.is_empty(),
            "a bright-sky rebuild must not project the night field"
        );
        night.ensure(&s_night, fb.w, fb.h, hy, 0.2, true);
        assert!(night.stars.len() > 100, "the night field comes back");
    }

    #[test]
    fn band_mapping_is_south_centred_and_linear() {
        // South at the centre, east left, west right, north at the edges.
        assert!((band_x(180.0, 1000) - 500.0).abs() < 0.01);
        assert!((band_x(90.0, 1000) - 250.0).abs() < 0.01);
        assert!((band_x(270.0, 1000) - 750.0).abs() < 0.01);
        assert!(band_x(0.0, 1000) < 1.0);
        assert!(band_x(359.9, 1000) > 999.0);
        // The horizon row is alt 0, the top of the band alt 90.
        let y0 = band_y(0.0, 144, 144);
        let y90 = band_y(90.0, 144, 144);
        assert!((y0 - 144.0).abs() < 0.01);
        assert!(y90 < 144.0 * 0.06);
        assert!(band_y(45.0, 144, 144) < 144.0 * 0.55);
    }

    #[test]
    fn meteor_layer_returns_a_bounded_rect() {
        let (s, pal) = night_state(JUN21 + 1 * 3_600_000);
        let mut fb = Framebuffer::new(960, 540);
        let mut night = NightSky::default();
        draw_with_treatment(&s, &pal, &mut fb, 42, 400, 0, Horizon::default(), &mut night, 0.3);
        let fade = 1.0;
        // Scan a minute of wall clock for a live streak.
        let base = 1_782_018_000_000i64;
        let mut found = false;
        for i in 0..1200 {
            if let Some(r) = meteor_layer(&mut fb.clone(), &fb, &night, base + i * 50, 400, fade) {
                found = true;
                assert!(r.width() <= 120 && r.height() <= 60, "meteor rect {r:?}");
                break;
            }
        }
        assert!(found, "no meteor found in a minute of sporadic slots");
    }

    #[test]
    fn meteors_never_touch_land_or_props() {
        // B45c: a night frame whose lower rows (and a column of "props") are
        // land: no meteor pixel may change them, only sky above the line.
        let (s, pal) = night_state(JUN21 + 1 * 3_600_000);
        let mut sky_fb = Framebuffer::new(960, 540);
        let mut night = NightSky::default();
        draw_with_treatment(&s, &pal, &mut sky_fb, 42, 400, 0, Horizon::default(), &mut night, 0.3);
        let land_y = 300usize;
        let mut frame = sky_fb.clone();
        for y in land_y..540 {
            for x in 0..960 {
                frame.px_mut()[y * 960 + x] = 0x203018;
            }
        }
        for y in 0..land_y {
            for x in 400..460 {
                frame.px_mut()[y * 960 + x] = 0x602010; // a tower standing in the sky
            }
        }
        let before = frame.px().to_vec();
        let base = 1_782_018_000_000i64;
        let mut drawn = 0;
        for i in 0..2400 {
            let mut f = frame.clone();
            if meteor_layer(&mut f, &sky_fb, &night, base + i * 50, 400, 1.0).is_some() {
                for (idx, (a, b)) in f.px().iter().zip(before.iter()).enumerate() {
                    if a != b {
                        drawn += 1;
                        let (x, y) = (idx % 960, idx / 960);
                        assert!(y < land_y, "meteor pixel on land at ({x},{y})");
                        assert!(!(400..460).contains(&x), "meteor pixel over a prop at ({x},{y})");
                    }
                }
            }
        }
        assert!(drawn > 0, "no meteor pixel drawn in the scan");
    }

    #[test]
    fn draw_is_fast_at_internal_scale() {
        // The per-redraw cost (replay only — the minute rebuild is measured in
        // the sky-strip bench): must stay well under the debug-build bound.
        let (s, pal) = night_state(JUN21 + 1 * 3_600_000);
        let mut fb = Framebuffer::new(960, 540);
        fb.reset_clip();
        let hy = fb.h as i32 * 72 / 100;
        let mut night = NightSky::default();
        draw_with_treatment(&s, &pal, &mut fb, 42, hy, 0, Horizon::default(), &mut night, 0.3);
        let start = std::time::Instant::now();
        for _ in 0..30 {
            draw_with_treatment(&s, &pal, &mut fb, 42, hy, 0, Horizon::default(), &mut night, 0.3);
        }
        let per = start.elapsed().as_nanos() as f64 / 30.0;
        println!("draw 960×540 night replay (debug build): {per:.0} ns");
        assert!(per < 8e6, "draw took {per:.0} ns");
    }
}
