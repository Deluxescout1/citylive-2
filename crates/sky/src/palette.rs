//! Sky + scene-light palettes (B12), blended smoothly across twilight.
//!
//! Every field is a smooth function of the sun's altitude and velocity (and
//! the moon's altitude for moonlight), so consecutive minutes move smoothly.
//! The palette targets (D4): *bright clear day*, *warm sunset* (golden
//! horizon, pinkish zenith), *moody blue night* with stars and moonlight.
//! Colours are `0xAARRGGBB` via `render::palette::rgb`.

use crate::{SkyState, Color};
use render::palette;

#[inline]
const fn clamp01(v: f32) -> f32 {
    v.max(0.0).min(1.0)
}

/// Smoothstep-based ramp: 0 for `x <= lo`, 1 for `x >= hi`, smooth between.
#[inline]
pub fn ramp(x: f64, lo: f64, hi: f64) -> f64 {
    if hi <= lo {
        return if x >= hi { 1.0 } else { 0.0 };
    }
    let t = ((x - lo) / (hi - lo)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// One anchor of the sky look: zenith/horizon colours, an ambient tint to
/// multiply the scene by, a directional-light strength, a window-glow level
/// (D25) and a star alpha.
#[derive(Debug, Clone, Copy)]
struct Anchor {
    zenith: (u8, u8, u8),
    horizon: (u8, u8, u8),
    /// Multiply colour for the scene, per channel (0..=1).
    ambient_tint: (f32, f32, f32),
    light_dir_strength: f32,
    window_glow: f32,
    star_alpha: f32,
}

// --- Anchor look targets (D4) ---------------------------------------------

const DAY: Anchor = Anchor {
    zenith: (56, 132, 214),   // bright clear blue zenith
    horizon: (169, 214, 240), // pale warm-white haze at the horizon
    ambient_tint: (1.00, 1.00, 0.99),
    light_dir_strength: 1.00,
    window_glow: 0.03,
    star_alpha: 0.0,
};

/// Warm sunset: golden/orange horizon, pinkish zenith.
///
/// B99d: the *sky* is where the sunset's colour belongs. The scene's ambient was
/// (1.00, 0.74, 0.52) — a heavy orange multiply over every pixel, which turned
/// snow peach and every wall the same sepia. The land now gets a light, slightly
/// warm dusk tone, and the gold arrives on the surfaces that face up into the low
/// sun (the building kit's `surf`) rather than over the whole frame.
const DUSK: Anchor = Anchor {
    zenith: (196, 106, 134),  // pinkish-mauve zenith
    horizon: (255, 158, 72),  // golden orange horizon
    ambient_tint: (1.00, 0.78, 0.74),
    light_dir_strength: 0.55,
    window_glow: 0.90,        // homes come alive in the evening (D25)
    star_alpha: 0.0,
};

/// Pre-sunrise: a touch cooler and pinker than dusk so dawn reads distinctly.
const DAWN: Anchor = Anchor {
    zenith: (132, 98, 168),   // violet-pink zenith
    horizon: (240, 152, 128), // soft salmon horizon
    ambient_tint: (0.92, 0.76, 0.86),
    light_dir_strength: 0.45,
    window_glow: 0.30,        // the night has faded (D25: out by mid-morning)
    star_alpha: 0.0,
};

/// Moody blue night (D4): deep blue zenith, cooler horizon, moonlight.
const NIGHT: Anchor = Anchor {
    zenith: (12, 18, 42),
    horizon: (39, 52, 90),
    ambient_tint: (0.18, 0.24, 0.62),
    light_dir_strength: 0.00, // + moonlight added separately
    window_glow: 1.00,        // full night lighting (D25)
    star_alpha: 1.0,
};

/// The 4-state look the renderer should blend towards.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    /// Sky zenith colour (top of the frame).
    pub zenith: Color,
    /// Sky horizon colour (meets the city line).
    pub horizon: Color,
    /// Per-channel multiply to apply over the scene colours.
    pub ambient_tint: [f32; 3],
    /// 0..=1: strength of the main (sun or moon) directional light.
    pub light_dir_strength: f32,
    /// 0..=1: how much the night window lighting is on (D25).
    pub window_glow: f32,
    /// 0..=1: star field visibility.
    pub star_alpha: f32,
}

/// The dawn/dusk blend: 0 = fully dusk (warm orange), 1 = fully dawn (cool
/// pink). Driven by the sun's *vertical velocity* (climbing → dawn) so it is
/// continuous in time — it never flips at the horizon crossing.
fn twilight_mix(s: &SkyState) -> f32 {
    // At most ~0.35 deg/min; scale to ±1 at ~0.12 deg/min.
    let k = (s.sun_rate / 0.12).clamp(-1.0, 1.0);
    0.5 + 0.5 * k as f32
}

/// Blend two anchors (0 = `a`, 1 = `b`).
fn mix_anchor(a: &Anchor, b: &Anchor, t: f64) -> Anchor {
    fn m(x: u8, y: u8, t: f64) -> u8 {
        (x as f64 * (1.0 - t) + y as f64 * t).round().clamp(0.0, 255.0) as u8
    }
    fn f3(x: f32, y: f32, t: f64) -> f32 {
        x + (y - x) * t as f32
    }
    Anchor {
        zenith: (
            m(a.zenith.0, b.zenith.0, t),
            m(a.zenith.1, b.zenith.1, t),
            m(a.zenith.2, b.zenith.2, t),
        ),
        horizon: (
            m(a.horizon.0, b.horizon.0, t),
            m(a.horizon.1, b.horizon.1, t),
            m(a.horizon.2, b.horizon.2, t),
        ),
        ambient_tint: (
            f3(a.ambient_tint.0, b.ambient_tint.0, t),
            f3(a.ambient_tint.1, b.ambient_tint.1, t),
            f3(a.ambient_tint.2, b.ambient_tint.2, t),
        ),
        light_dir_strength: f3(a.light_dir_strength, b.light_dir_strength, t),
        window_glow: f3(a.window_glow, b.window_glow, t),
        star_alpha: f3(a.star_alpha, b.star_alpha, t),
    }
}

/// Compute the palette for a sky state. Cost: a few multiplies only on top of
/// the already-known altitudes (measured in H12).
pub fn palette(s: &SkyState) -> Palette {
    let alt = s.sun_alt;

    // Smooth weights for the three look regions from the sun altitude. The
    // day/night ramps sit outside the twilight band (±16°) so that at the
    // horizon the whole weight is the (dawn/dusk) anchor and it is pure day
    // or night well away from it. Weights are renormalised below, so the
    // mild overlap in the mid-band is harmless.
    let w_day = ramp(alt, 0.0, 16.0);
    let w_night = 1.0 - ramp(alt, -16.0, 0.0);
    let w_twilight = 1.0 - (alt.abs() / 16.0).min(1.0);

    // Blend the two twilight anchors smoothly from velocity (no flip when
    // the sun's rate crosses zero at the exact top/bottom of the day).
    let tmix = twilight_mix(s);
    let tw_anchor = mix_anchor(&DUSK, &DAWN, tmix as f64);

    let mut p = blend3(&DAY, &tw_anchor, &NIGHT, w_day, w_twilight, w_night);

    // Moonlight: add a small directional strength when the moon is up, so a
    // full-moon night still has a light direction (it does not change the sky
    // colour, only the light the scene sees).
    let moon_up = ramp(s.moon_alt, -2.0, 20.0);
    let moon_light = 0.30 * moon_up as f32 * s.moon_illum as f32;
    p.light_dir_strength = (p.light_dir_strength + moon_light).min(1.0);

    // B27: cloud greys the sky. An overcast day is a flat grey lid, not a blue
    // sky with puffs in it, and the light loses its blue cast too.
    let cover = s.cloud.clamp(0.0, 1.0) as f64;
    if cover > 0.02 {
        // A thin sky keeps its colour: the grey lid only starts once the cover
        // is real (a 30 % cover is still a blue sky with puffs in it).
        let k = 0.72 * ((cover - 0.15) / 0.85).clamp(0.0, 1.0);
        let grey_z = palette::rgb(0x8e, 0x95, 0x9e);
        let grey_h = palette::rgb(0xc2, 0xc6, 0xca);
        p.zenith = palette::mix(p.zenith, grey_z, k as f32);
        p.horizon = palette::mix(p.horizon, grey_h, k as f32);
        let dim = 1.0 - 0.22 * ((cover - 0.35) / 0.65).clamp(0.0, 1.0);
        for c in p.ambient_tint.iter_mut() {
            *c = (*c as f64 * dim) as f32;
        }
        // Stars only go when the sky is actually covered.
        let hide = 1.0 - 0.92 * ((cover - 0.35) / 0.65).clamp(0.0, 1.0);
        p.star_alpha = (p.star_alpha as f64 * hide) as f32;
    }

    p
}

/// 3-way weighted mix of three anchors with weights summing to 1.
fn blend3(a: &Anchor, b: &Anchor, c: &Anchor, wa: f64, wb: f64, wc: f64) -> Palette {
    let sum = wa + wb + wc;
    let wa = wa / sum;
    let wb = wb / sum;
    let wc = wc / sum;

    // Combine colours/tints as weighted sums.
    let z = weighted_zenith(a, b, c, wa, wb, wc);
    let hz = weighted_horizon(a, b, c, wa, wb, wc);
    let tint = [
        a.ambient_tint.0 * wa as f32 + b.ambient_tint.0 * wb as f32 + c.ambient_tint.0 * wc as f32,
        a.ambient_tint.1 * wa as f32 + b.ambient_tint.1 * wb as f32 + c.ambient_tint.1 * wc as f32,
        a.ambient_tint.2 * wa as f32 + b.ambient_tint.2 * wb as f32 + c.ambient_tint.2 * wc as f32,
    ];
    let light = a.light_dir_strength * wa as f32 + b.light_dir_strength * wb as f32 + c.light_dir_strength * wc as f32;
    let glow = a.window_glow * wa as f32 + b.window_glow * wb as f32 + c.window_glow * wc as f32;
    let star = a.star_alpha * wa as f32 + b.star_alpha * wb as f32 + c.star_alpha * wc as f32;

    Palette {
        zenith: z,
        horizon: hz,
        ambient_tint: [tint[0].clamp(0.0, 1.0), tint[1].clamp(0.0, 1.0), tint[2].clamp(0.0, 1.0)],
        light_dir_strength: clamp01(light),
        window_glow: clamp01(glow),
        star_alpha: clamp01(star),
    }
}

fn weighted_zenith(a: &Anchor, b: &Anchor, c: &Anchor, wa: f64, wb: f64, wc: f64) -> Color {
    let r = a.zenith.0 as f64 * wa + b.zenith.0 as f64 * wb + c.zenith.0 as f64 * wc;
    let g = a.zenith.1 as f64 * wa + b.zenith.1 as f64 * wb + c.zenith.1 as f64 * wc;
    let bl = a.zenith.2 as f64 * wa + b.zenith.2 as f64 * wb + c.zenith.2 as f64 * wc;
    palette::rgb(r.round() as u8, g.round() as u8, bl.round() as u8)
}

fn weighted_horizon(a: &Anchor, b: &Anchor, c: &Anchor, wa: f64, wb: f64, wc: f64) -> Color {
    let r = a.horizon.0 as f64 * wa + b.horizon.0 as f64 * wb + c.horizon.0 as f64 * wc;
    let g = a.horizon.1 as f64 * wa + b.horizon.1 as f64 * wb + c.horizon.1 as f64 * wc;
    let bl = a.horizon.2 as f64 * wa + b.horizon.2 as f64 * wb + c.horizon.2 as f64 * wc;
    palette::rgb(r.round() as u8, g.round() as u8, bl.round() as u8)
}

// --- Tests -----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state;

    const SITE_LAT: f64 = 41.0;
    const SITE_LON: f64 = -75.0;
    // 21 June 2026 00:00 local (EDT) / 21 December 2026 00:00 local (EST).
    const JUN21: i64 = 1_782_014_400_000;
    const DEC21: i64 = 1_797_829_200_000;

    fn rgb(c: Color) -> (u8, u8, u8) {
        (palette::r(c), palette::g(c), palette::b(c))
    }

    #[test]
    fn palette_values_in_range() {
        // Sample a full day in June and a full day in December at 10-min
        // steps; every scalar field must stay in its documented range.
        for (base, day) in [(JUN21, JUN21), (DEC21, DEC21)] {
            let _ = day;
            for minute in (0..(24 * 60)).step_by(10) {
                let p = palette(&state(base + minute * 60_000, SITE_LAT, SITE_LON));
                assert!(p.ambient_tint.iter().all(|&v| (0.0..=1.0).contains(&v)));
                assert!((0.0..=1.0).contains(&p.light_dir_strength));
                assert!((0.0..=1.0).contains(&p.window_glow));
                assert!((0.0..=1.0).contains(&p.star_alpha));
                // Colours are opaque and have u8 channels by construction.
                assert_eq!(palette::alpha(p.zenith), 255);
                assert_eq!(palette::alpha(p.horizon), 255);
            }
        }
    }

    /// B99d: the twilight's *scene* light is a light dusk tone, not a sepia
    /// multiply over everything (snow turned peach). The gold lives in the sky.
    #[test]
    fn dusk_light_does_not_sepia_the_scene() {
        // The sample nearest the dusk anchor (the sky is at its glow) on
        // 21 December at the test site.
        let mut best: Option<(f32, Palette)> = None;
        for minute in (15 * 60..18 * 60).step_by(2) {
            let st = state(DEC21 + minute * 60_000, SITE_LAT, SITE_LON);
            let p = palette(&st);
            let d = (p.window_glow - 0.90).abs();
            if best.map(|(b, _)| d < b).unwrap_or(true) {
                best = Some((d, p));
            }
        }
        let p = best.expect("no dusk sample").1;
        let [r, g, b] = p.ambient_tint;
        assert!(r > b, "the twilight should still be warm ({r}, {g}, {b})");
        // The old tint was (1.00, 0.74, 0.52): a strong orange that made snow
        // peach. A light, pinkish warm keeps the snow/hwall hue near neutral while
        // staying warm enough for the golden hour (see the D4 test).
        assert!(b / r > 0.70, "the twilight tint is a sepia ({r}, {g}, {b})");
        assert!(g / r > 0.74, "the twilight tint is too orange ({r}, {g}, {b})");
    }

    #[test]
    fn noon_is_bright_clear_day() {
        let p = palette(&state(JUN21 + 12 * 3_600_000, SITE_LAT, SITE_LON));
        // Day zenith is clearly blue (b > r) and horizon is a pale blue-white.
        let (zr, zg, zb) = rgb(p.zenith);
        assert!(zb > zr, "zenith not blue-ish: {p:?}");
        assert!(zg > zr, "zenith not bright: {p:?}");
        let (hr, hg, hb) = rgb(p.horizon);
        assert!(hb >= hr && hb >= hg, "horizon not pale-blue: {p:?}");
        // Day is strongly lit, no stars, no windows.
        assert!(p.light_dir_strength > 0.9);
        assert!(p.star_alpha < 0.05);
        assert!(p.window_glow < 0.15);
    }

    #[test]
    fn midnight_is_moody_blue_night() {
        let p = palette(&state(JUN21 + 1 * 3_600_000, SITE_LAT, SITE_LON));
        let (zr, zg, zb) = rgb(p.zenith);
        assert!(zb > zr && zb > zg, "night zenith should be moody blue: {p:?}");
        assert!(zb < 120, "night zenith too bright: {p:?}");
        // Stars out, windows on, low light.
        assert!(p.star_alpha > 0.9);
        assert!(p.window_glow > 0.8);
        assert!(p.light_dir_strength < 0.4);
    }

    #[test]
    fn sunset_is_warm_with_golden_horizon() {
        // Find the Dusk minute closest to sunset and check the horizon reads
        // warm (r > b) with a pinkish zenith (r > b).
        let mut best: Option<Palette> = None;
        for minute in (0..(24 * 60)).step_by(5) {
            let s = state(JUN21 + minute * 60_000, SITE_LAT, SITE_LON);
            if s.phase == crate::Phase::Dusk && s.sun_alt < 2.0 && s.sun_alt > -3.0 {
                best = Some(palette(&s));
                break;
            }
        }
        let p = best.expect("no warm dusk minute found");
        let (hr, _hg, hb) = rgb(p.horizon);
        assert!(hr > hb, "sunset horizon not golden/orange: {p:?}");
        let (zr, _zg, zb) = rgb(p.zenith);
        assert!(zr > zb, "sunset zenith not pinkish: {p:?}");
    }

    #[test]
    fn no_hard_jumps_between_consecutive_minutes() {
        // The brief's required smoothness check: the max per-channel colour
        // delta between palettes one minute apart stays below a small bound.
        // We sweep a June day (fast sunrise) and a December day (slow) at
        // 1-minute steps, 24 h each.
        const MAX_CHANNEL_DELTA: i32 = 12; // per-channel 0..255 per minute
        for base in [JUN21, DEC21] {
            let mut prev = palette(&state(base, SITE_LAT, SITE_LON));
            let mut worst = 0i32;
            for minute in 1..=(24 * 60) {
                let cur = palette(&state(base + minute as i64 * 60_000, SITE_LAT, SITE_LON));
                let (pa0, pa1, pa2) = (
                    palette::r(prev.zenith) as i32,
                    palette::g(prev.zenith) as i32,
                    palette::b(prev.zenith) as i32,
                );
                let (pb0, pb1, pb2) = (
                    palette::r(cur.zenith) as i32,
                    palette::g(cur.zenith) as i32,
                    palette::b(cur.zenith) as i32,
                );
                worst = worst.max((pa0 - pb0).abs());
                worst = worst.max((pa1 - pb1).abs());
                worst = worst.max((pa2 - pb2).abs());
                let (ha0, ha1, ha2) = (
                    palette::r(prev.horizon) as i32,
                    palette::g(prev.horizon) as i32,
                    palette::b(prev.horizon) as i32,
                );
                let (hb0, hb1, hb2) = (
                    palette::r(cur.horizon) as i32,
                    palette::g(cur.horizon) as i32,
                    palette::b(cur.horizon) as i32,
                );
                worst = worst.max((ha0 - hb0).abs());
                worst = worst.max((ha1 - hb1).abs());
                worst = worst.max((ha2 - hb2).abs());
                // Also bound the scalar fields' per-minute rate.
                let dl = (cur.light_dir_strength - prev.light_dir_strength).abs();
                let dg = (cur.window_glow - prev.window_glow).abs();
                let ds = (cur.star_alpha - prev.star_alpha).abs();
                assert!(dl < 0.02, "light_dir jumped {dl} at minute {minute}");
                assert!(dg < 0.02, "window_glow jumped {dg} at minute {minute}");
                assert!(ds < 0.02, "star_alpha jumped {ds} at minute {minute}");
                prev = cur;
            }
            assert!(
                worst <= MAX_CHANNEL_DELTA,
                "max colour delta {worst} per minute exceeds {MAX_CHANNEL_DELTA}"
            );
        }
    }
}

#[cfg(test)]
mod lighting_tests {
    use super::*;
    use crate::{Phase, state};

    /// Mean RGB of the untinted village art (measured on H11's day-8 frame).
    const ART: [f64; 3] = [104.0, 147.0, 74.0];

    fn lum(c: [f64; 3]) -> f64 {
        0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
    }

    fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
        let y = if m <= 2 { y - 1 } else { y };
        let era = if y >= 0 { y } else { y - 399 } / 400;
        let yoe = y - era * 400;
        let mp = ((m + 9) % 12) as i64;
        let doy = (153 * mp + 2) / 5 + d as i64 - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146097 + doe - 719468
    }

    /// D4/B16: the light must read as a bright day, a warm golden hour and a
    /// moody blue night, sampled every 10 minutes across a whole day.
    #[test]
    fn lighting_meets_d4_over_a_whole_day() {
        let (lat, lon) = (41.0, -74.0);
        let base = days_from_civil(2026, 11, 9) * 86_400_000;
        let mut samples: Vec<(f64, [f64; 3], f64, Phase)> = Vec::new();
        for i in 0..144 {
            let ms = base + (i as i64 * 10) * 60_000;
            let st = state(ms, lat, lon);
            let pal = palette(&st);
            let t = pal.ambient_tint;
            let mean = [
                ART[0] * t[0] as f64,
                ART[1] * t[1] as f64,
                ART[2] * t[2] as f64,
            ];
            let ratio = mean[0] / mean[1].max(1.0);
            samples.push((lum(mean), mean, ratio, st.phase));
        }
        let noon = samples.iter().map(|s| s.0).fold(0.0, f64::max);
        let noon_ratio = samples
            .iter()
            .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap())
            .unwrap()
            .2;

        // Bright day: the untinted art, ±5 %.
        assert!((noon / lum(ART) - 1.0).abs() < 0.05, "noon luminance {noon:.0} vs art {:.0}", lum(ART));

        // Golden hour: a warm shift with 75-90 % of noon light.
        let golden = samples
            .iter()
            .filter(|s| s.0 >= 0.72 * noon && s.0 <= 0.92 * noon)
            .max_by(|a, b| a.2.partial_cmp(&b.2).unwrap());
        let g = golden.expect("no golden-hour sample");
        assert!(
            g.2 >= 1.25 * noon_ratio,
            "golden R/G {:.2} vs noon {:.2}",
            g.2,
            noon_ratio
        );
        assert_eq!(g.3, Phase::Dusk, "golden sample should be dusk");

        // Moody blue night: 20-35 % of noon, blue dominant.
        for s in samples.iter().filter(|s| s.3 == Phase::Night) {
            let frac = s.0 / noon;
            assert!(frac >= 0.20 && frac <= 0.35, "night luminance {:.0}% of noon", frac * 100.0);
            assert!(s.1[2] >= s.1[0] && s.1[2] >= s.1[1], "night is not blue-dominant: {:?}", s.1);
        }

        // Smooth: no sample jumps more than 10 % of noon from its neighbour.
        for w in samples.windows(2) {
            let d = (w[1].0 - w[0].0).abs() / noon;
            assert!(d < 0.10, "luminance jumps {:.0}% in 10 minutes", d * 100.0);
        }
    }
}
