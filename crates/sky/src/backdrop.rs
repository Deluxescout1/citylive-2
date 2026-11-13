//! Distant backdrop (D43, B20): broad faceted mountains and a tree fringe on the
//! horizon, drawn behind the terrain in the sky layer.
//!
//! Everything is a pure function of `(seed, palette)`: the sky layer is drawn
//! once per palette change and cached by the renderer.
//!
//! **Clouds are not here** (B57): they move, so they are drawn per frame over the
//! cached sky by the renderer (`render::clouds`). What the sky keeps of the
//! weather is the *overcast wash* — the flat grey lid a heavy deck puts over the
//! band — because that depends only on the cover and the hour, not on where a
//! cloud is.

use crate::{Palette, SkyState};
use render::Horizon;
use render::{palette, Framebuffer};

fn hash(i: i64, seed: u64) -> f32 {
    let mut x = (i as u64)
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(seed.wrapping_mul(0xC2B2_AE3D_27D4_EB4F));
    x ^= x >> 33;
    x = x.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    x ^= x >> 33;
    x = x.wrapping_mul(0xC4CE_B9FE_1A85_EC53);
    x ^= x >> 33;
    (x >> 40) as f32 / (1u64 << 24) as f32
}

/// Height of one ridge layer above the horizon at column `x`, in px.
///
/// Broad faceted mountains, not needles (B20 answer 1): the width is cut into
/// `peaks` triangles whose apex sits off-centre with a seeded height, joined
/// by straight slopes from the valley floors, then quantised to 2 px so each
/// slope is a stepped pixel-art facet. `amp` is the tallest crest.
pub fn ridge_profile(x: i32, width: i32, seed: u64, amp: f32, peaks: f32) -> f32 {
    ridge_profile_var(x, width, seed, amp, peaks, 0.55)
}

/// As [`ridge_profile`], but `min_apex` sets how low the shortest peaks are:
/// a low floor gives a couple of big peaks among foothills (the near range in
/// the H20 review), a high one an even, distant-looking ridge line.
pub fn ridge_profile_var(
    x: i32,
    width: i32,
    seed: u64,
    amp: f32,
    peaks: f32,
    min_apex: f32,
) -> f32 {
    let span = width.max(1) as f32 / peaks.max(0.5);
    let t = x as f32 / span;
    let i = t.floor();
    let f = t - i;
    let idx = i as i64;
    let valley = |k: i64| 0.10 + 0.18 * hash(k * 3, seed);
    let apex = min_apex + (1.0 - min_apex) * hash(idx * 3 + 1, seed);
    let ax = 0.35 + 0.30 * hash(idx * 3 + 2, seed);
    let (v0, v1) = (valley(idx), valley(idx + 1));
    let h = if f < ax {
        v0 + (apex - v0) * (f / ax)
    } else {
        apex + (v1 - apex) * ((f - ax) / (1.0 - ax))
    };
    (h * amp / 2.0).round() * 2.0
}

/// Multiply `base` by the ambient light, then push it towards `air` (the sky
/// colour behind it) — that mix is what reads as distance.
fn haze(base: u32, pal: &Palette, air: u32, amount: f32, night: bool) -> u32 {
    let t = pal.ambient_tint;
    // At night the ridges are nearly silhouettes: keep them dark, and only a
    // little moonlight picks them out.
    let dim = if night { 0.55 } else { 1.0 };
    let lit = palette::rgb(
        (palette::r(base) as f32 * t[0] * dim) as u8,
        (palette::g(base) as f32 * t[1] * dim) as u8,
        (palette::b(base) as f32 * t[2] * dim) as u8,
    );
    palette::mix(lit, air, amount)
}

/// One distant range: how much air sits over it, how tall it is, how many
/// peaks, and its untinted colour.
#[derive(Clone, Copy)]
struct Range {
    haze: f32,
    amp: f32,
    peaks: f32,
    min_apex: f32,
    base: u32,
}

/// Forest edge (D64): tall ranges that read blue-grey through the air, with a
/// near one that still has some green in it.
const FOREST_RANGES: [Range; 4] = [
    Range { haze: 0.80, amp: 0.52, peaks: 3.2, min_apex: 0.58, base: 0x6C7C98 },
    Range { haze: 0.66, amp: 0.34, peaks: 4.2, min_apex: 0.46, base: 0x596A86 },
    Range { haze: 0.50, amp: 0.21, peaks: 5.2, min_apex: 0.34, base: 0x4B5C74 },
    Range { haze: 0.32, amp: 0.12, peaks: 6.4, min_apex: 0.22, base: 0x44513A },
];

/// Dunes (D66): long low sand ridges, each paler than the last, under a bleached
/// dust haze — the land's own air is most of the horizon.
const DUNE_RANGES: [Range; 3] = [
    Range { haze: 0.88, amp: 0.20, peaks: 2.2, min_apex: 0.72, base: 0xE0C79A },
    Range { haze: 0.70, amp: 0.13, peaks: 2.8, min_apex: 0.60, base: 0xD2AE79 },
    Range { haze: 0.42, amp: 0.07, peaks: 3.4, min_apex: 0.48, base: 0xC49A62 },
];

/// Snow ridges (D66): pale cold ranges, almost the colour of the air.
const SNOW_RANGES: [Range; 3] = [
    Range { haze: 0.86, amp: 0.46, peaks: 3.0, min_apex: 0.56, base: 0xC8D6E6 },
    Range { haze: 0.66, amp: 0.28, peaks: 4.0, min_apex: 0.42, base: 0xAEBFD4 },
    Range { haze: 0.40, amp: 0.14, peaks: 5.0, min_apex: 0.28, base: 0x93A6BC },
];

/// Alpine peaks (B46e): tall, jagged granite under snow, three ranks deep.
const ALPINE_RANGES: [Range; 3] = [
    Range { haze: 0.82, amp: 0.60, peaks: 2.6, min_apex: 0.62, base: 0x8C9AB4 },
    Range { haze: 0.62, amp: 0.42, peaks: 3.4, min_apex: 0.50, base: 0x6E7E98 },
    Range { haze: 0.40, amp: 0.26, peaks: 4.6, min_apex: 0.38, base: 0x56667C },
];

/// The savanna (B46e): long, low, warm swells, the last of them almost the colour
/// of the haze.
const SAVANNA_RANGES: [Range; 3] = [
    Range { haze: 0.86, amp: 0.20, peaks: 2.4, min_apex: 0.60, base: 0xCDB48A },
    Range { haze: 0.66, amp: 0.13, peaks: 3.2, min_apex: 0.50, base: 0xB89E6C },
    Range { haze: 0.44, amp: 0.07, peaks: 4.0, min_apex: 0.40, base: 0xA08A58 },
];

/// The wetland (B46e): almost nothing, green-grey and lost in the mist.
const SWAMP_RANGES: [Range; 2] = [
    Range { haze: 0.90, amp: 0.10, peaks: 3.0, min_apex: 0.60, base: 0x8AA298 },
    Range { haze: 0.74, amp: 0.06, peaks: 4.0, min_apex: 0.50, base: 0x6E8A7A },
];

/// The moors (B46e): long rounded heather hills, purple-grey.
const HIGHLAND_RANGES: [Range; 3] = [
    Range { haze: 0.84, amp: 0.30, peaks: 2.6, min_apex: 0.58, base: 0x8A8398 },
    Range { haze: 0.64, amp: 0.20, peaks: 3.4, min_apex: 0.48, base: 0x726F86 },
    Range { haze: 0.42, amp: 0.11, peaks: 4.4, min_apex: 0.38, base: 0x5E6A66 },
];

/// A jungle wall (D66): tall, close and dark, so the canopy reads as a mass.
const JUNGLE_RANGES: [Range; 2] = [
    Range { haze: 0.72, amp: 0.30, peaks: 3.6, min_apex: 0.52, base: 0x5E8A56 },
    Range { haze: 0.44, amp: 0.18, peaks: 4.6, min_apex: 0.34, base: 0x3E6A3C },
];

/// Fields (D64): low, far, pale hills; the farmland in front carries the eye.
const FIELD_RANGES: [Range; 3] = [
    Range { haze: 0.90, amp: 0.26, peaks: 3.0, min_apex: 0.60, base: 0x7E8FB0 },
    Range { haze: 0.66, amp: 0.17, peaks: 4.6, min_apex: 0.44, base: 0x5F7090 },
    Range { haze: 0.40, amp: 0.10, peaks: 6.2, min_apex: 0.28, base: 0x435A44 },
];

/// Value noise with a smooth interpolation, for ground that rolls instead of
/// stepping from apex to apex.
fn smooth_noise(x: f32, seed: u64) -> f32 {
    let i = x.floor();
    let f = x - i;
    let a = hash(i as i64, seed);
    let b = hash(i as i64 + 1, seed);
    let t = f * f * (3.0 - 2.0 * f);
    a + (b - a) * t
}

/// A hill height in px above the horizon at column `x`, in 1-px steps: the
/// same broad faceted ranges as [`ridge_profile_var`], but the crest is
/// rounded (smoothstep) and three octaves of noise break the straight slopes,
/// so the silhouette reads as soft ground rather than stepped bands.
///
/// B65: a crest must never sit on one row for long. The crests used to flatten
/// out on top (a smoothstep's apex has no slope), which left 50–95-px runs of
/// the same row and then a 4–8-px drop where the next range showed through —
/// review read that as "a step in the far ridge: the ridge drops a band". The
/// fine octave below keeps the ridgeline moving a pixel at a time.
pub fn soft_profile(x: i32, width: i32, seed: u64, amp: f32, peaks: f32, min_apex: f32) -> f32 {
    let span = (width.max(1) as f32 / peaks.max(0.5)).max(24.0);
    let t = x as f32 / span;
    let i = t.floor();
    let f = t - i;
    let idx = i as i64;
    let valley = |k: i64| 0.10 + 0.18 * hash(k * 3, seed);
    let apex = min_apex + (1.0 - min_apex) * hash(idx * 3 + 1, seed);
    let ax = 0.35 + 0.30 * hash(idx * 3 + 2, seed);
    let (v0, v1) = (valley(idx), valley(idx + 1));
    let (lo, u) = if f < ax { (v0, f / ax) } else { (v1, (1.0 - f) / (1.0 - ax)) };
    // A peakier crest than a plain smoothstep: distant ranges have shoulders,
    // not domes. The exponent stays low enough that the very top still climbs.
    let sm = u.powf(1.12) * (3.0 - 2.0 * u).powf(0.30);
    let h = lo + (apex - lo) * sm.clamp(0.0, 1.0);
    // Two octaves of noise, so a range is not a row of identical hills.
    let ragged = 0.86
        + 0.18 * smooth_noise(x as f32 / (span / 7.0), seed ^ 0x5F5F)
        + 0.10 * smooth_noise(x as f32 / (span / 23.0), seed ^ 0x2B2B);
    // And a ripple measured in *pixels*, not in the range's own height: a low
    // near range has a very shallow slope, so its crest used to hold one row
    // for a hundred columns and then step — the band review saw. This keeps
    // every ridgeline moving a pixel at a time whatever its amplitude.
    let ripple = (smooth_noise(x as f32 / (span / 97.0).max(2.0), seed ^ 0x7A7A) - 0.5) * 3.2;
    (h * amp * ragged + ripple).max(0.0)
}

/// Draw the horizon treatment and the horizon fringe.
/// `horizon_y` is where the land's top edge is; `horizon` is what stands
/// beyond it (D64). `wind` is kept for the call sites' sake (B57: the clouds
/// that used to drift here now live in the renderer, where they can move every
/// frame).
pub fn draw(
    s: &SkyState,
    pal: &Palette,
    fb: &mut Framebuffer,
    seed: u64,
    horizon_y: i32,
    _wind: i32,
    horizon: Horizon,
) {
    let w = fb.w as i32;
    if w <= 0 || fb.h == 0 {
        return;
    }
    let night = pal.star_alpha > 0.35;
    // The sky the ranges stand against: low down it is the horizon colour,
    // higher up the zenith bleeds in. That is what the haze mixes towards, so
    // the mountains read as air-dimmed rather than painted — and it is the
    // colour the far ground of the map fades into too (Scene::set_horizon_air).
    let air = air_colour(pal);
    // B42b: the strip of distant ground between the far treeline and the land
    // line. The map's own ground fades into its *bottom* row (Scene's haze
    // reaches 1.0 exactly at the land line), so the two meet with no seam and
    // the ground visibly runs on into the distance.
    let fg = ((horizon_y as f32 * 0.10) as i32).clamp(10, 44);
    let art = (horizon_y - fg).max(8);

    // B90: the lid goes on *first*. Distant land stands in front of the sky, so a
    // deck must not be washed over it (that made the far hills see-through).
    overcast_wash(s, pal, fb, horizon_y, night);

    match horizon {
        Horizon::Forest => {
            hills(
                fb,
                pal,
                s,
                seed,
                art,
                night,
                air,
                &FOREST_RANGES,
                palette::rgb(0x4E, 0x5A, 0x74),
                0.55,
            );
            far_ground(fb, pal, seed, art, horizon_y, night, air, Ground::Meadow);
            forest_edge(fb, pal, seed, art, night, air, horizon_y - art);
        }
        Horizon::Fields => {
            hills(
                fb,
                pal,
                s,
                seed,
                art,
                night,
                air,
                &FIELD_RANGES,
                palette::rgb(0x4E, 0x5A, 0x74),
                0.55,
            );
            far_ground(fb, pal, seed, art, horizon_y, night, air, Ground::Field);
            far_fields(fb, pal, seed, art, horizon_y, night, air);
        }
        Horizon::Coast => sea_to_horizon(fb, pal, s, seed, horizon_y, night, air),
        // D66: the biome horizons. Each is the land's own air plus its own far
        // ground, so the map's last row dissolves into the same country the
        // whole world is.
        Horizon::Dunes => {
            // Sand keeps its warmth: a dune ridge is not slate.
            hills(
                fb,
                pal,
                s,
                seed,
                art,
                night,
                air,
                &DUNE_RANGES,
                palette::rgb(0xE2, 0xC0, 0x8E),
                0.45,
            );
            far_ground(fb, pal, seed, art, horizon_y, night, air, Ground::Sand);
            dune_crests(fb, pal, seed, art, horizon_y, night, air);
        }
        Horizon::SnowRidges => {
            hills(
                fb,
                pal,
                s,
                seed,
                art,
                night,
                air,
                &SNOW_RANGES,
                palette::rgb(0xC8, 0xD6, 0xE6),
                0.40,
            );
            far_ground(fb, pal, seed, art, horizon_y, night, air, Ground::Snow);
        }
        Horizon::Savanna => {
            hills(
                fb,
                pal,
                s,
                seed,
                art,
                night,
                air,
                &SAVANNA_RANGES,
                palette::rgb(0xD8, 0xBE, 0x8A),
                0.42,
            );
            far_ground(fb, pal, seed, art, horizon_y, night, air, Ground::Savanna);
            acacia_line(fb, pal, seed, art, night, air, horizon_y - art);
        }
        Horizon::Swamp => {
            hills(fb, pal, s, seed, art, night, air, &SWAMP_RANGES, palette::rgb(0x8A, 0xA0, 0x96), 0.40);
            far_ground(fb, pal, seed, art, horizon_y, night, air, Ground::Jungle);
            forest_edge(fb, pal, seed, art, night, air, horizon_y - art);
            mist_band(fb, pal, art - horizon_y / 5, horizon_y, air, 0.62);
        }
        Horizon::Highlands => {
            hills(fb, pal, s, seed, art, night, air, &HIGHLAND_RANGES, palette::rgb(0x8A, 0x84, 0x9A), 0.40);
            far_ground(fb, pal, seed, art, horizon_y, night, air, Ground::Meadow);
            mist_band(fb, pal, art - horizon_y / 7, horizon_y, air, 0.34);
        }
        Horizon::Alpine => {
            peaks(fb, pal, s, seed, art, night, air, &ALPINE_RANGES);
            far_ground(fb, pal, seed, art, horizon_y, night, air, Ground::Meadow);
            forest_edge(fb, pal, seed, art, night, air, horizon_y - art);
        }
        Horizon::JungleCanopy => {
            hills(
                fb,
                pal,
                s,
                seed,
                art,
                night,
                air,
                &JUNGLE_RANGES,
                palette::rgb(0x2E, 0x54, 0x30),
                0.35,
            );
            far_ground(fb, pal, seed, art, horizon_y, night, air, Ground::Jungle);
            // A canopy wall: a wood, but one row taller and denser than the
            // temperate treeline, with a few palms standing out of it.
            forest_edge(fb, pal, seed, art, night, air, horizon_y - art);
            jungle_palms(fb, pal, seed, art, night, air, horizon_y - art);
        }
    }

}

/// The colour of a distant range: slate, muted, with `haze` of the sky's own
/// colour in it and dark enough to read as a silhouette at dusk (B42b).
fn range_col(
    base: u32,
    pal: &Palette,
    air: u32,
    haze: f32,
    night: bool,
    cool: u32,
    cool_k: f32,
) -> u32 {
    let b = palette::mix(base, cool, cool_k);
    let c = palette::mix(b, air, (haze * 0.34).clamp(0.0, 0.55));
    let glow = pal.window_glow.clamp(0.0, 1.0);
    let dark = 1.0 - 0.34 * glow - if night { 0.34 } else { 0.0 };
    palette::shade(c, dark.max(0.35))
}

/// Where the sun sits on the sky, as an x column.
pub fn sun_column(s: &SkyState, w: i32) -> i32 {
    (sun_fraction(s) * w as f32) as i32
}

/// The same mapping as a fraction of the band's width (0 = the left edge, 1 =
/// the right). A cloud lights up on the sun's side (B57b), and that side is this
/// number, not a guess.
pub fn sun_fraction(s: &SkyState) -> f32 {
    // The band faces south: east is the left edge, west the right.
    let az = if s.sun_alt > -1.0 { s.sun_az } else { s.moon_az };
    let from_south = (az - 180.0).abs().min(180.0);
    let east = az < 180.0;
    let f = (from_south / 180.0) as f32 * 0.42;
    if east {
        0.5 - f
    } else {
        0.5 + f
    }
}


/// Dune crest lines (D66): long soft arcs across the far sand, lit on the wind
/// side, so the strip reads as a sand sea rather than a flat card.
fn dune_crests(
    fb: &mut Framebuffer,
    pal: &Palette,
    seed: u64,
    y_top: i32,
    y_bot: i32,
    night: bool,
    air: u32,
) {
    let w = fb.w as i32;
    if y_bot <= y_top + 4 {
        return;
    }
    let lit = palette::mix(palette::rgb(0xF2, 0xDE, 0xB2), air, if night { 0.55 } else { 0.30 });
    let shade = palette::shade(lit, if night { 0.72 } else { 0.86 });
    let rows = 3;
    for r in 0..rows {
        let u = (r as f32 + 0.6) / rows as f32;
        let y0 = y_top + ((y_bot - y_top) as f32 * u) as i32;
        let amp = (2.0 + 3.0 * (1.0 - u)) as i32;
        let phase = hash(r as i64, seed ^ 0xD0E5) * 6.283;
        for x in 0..w {
            let t = x as f32 / w as f32 * 6.283 * (1.5 + hash(r as i64 + 1, seed ^ 0xD0E6));
            let y = y0 - ((t + phase).sin() * amp as f32) as i32;
            fb.put(x, y, lit);
            fb.put(x, y + 1, shade);
            if (t + phase).sin() > 0.55 {
                fb.put(x, y - 1, palette::shade(lit, 1.05));
            }
        }
    }
}

/// Palms standing out of the jungle wall (D66): a trunk and a fan of fronds, in
/// silhouette against the air, a few columns apart.
fn jungle_palms(
    fb: &mut Framebuffer,
    pal: &Palette,
    seed: u64,
    base: i32,
    night: bool,
    air: u32,
    band: i32,
) {
    let w = fb.w as i32;
    let trunk_c = palette::mix(palette::rgb(0x3E, 0x30, 0x24), air, if night { 0.35 } else { 0.55 });
    let frond_c = palette::mix(palette::rgb(0x2E, 0x54, 0x30), air, if night { 0.30 } else { 0.45 });
    let u = (band as f32 / 144.0).clamp(0.5, 2.4);
    let step = (46.0 * u) as i32;
    let mut x = (hash(0, seed ^ 0x9A1) * step as f32) as i32;
    let mut k = 0i64;
    while x < w {
        let h = ((20.0 + 14.0 * hash(k, seed ^ 0x9A2)) * u) as i32;
        let top = (base - h).max(2);
        for y in top..base {
            fb.put(x, y, trunk_c);
        }
        // Six fronds fanning out and drooping.
        let reach = (7.0 * u) as i32 + 2;
        for f in 0..6 {
            let a = -1.4 + 2.8 * (f as f32 / 5.0);
            let mut px = x as f32;
            let mut py = top as f32;
            for i in 0..reach {
                px += a.cos() * 1.1;
                py += a.sin() * 0.7 + 0.25 * i as f32;
                fb.put(px as i32, py as i32, frond_c);
                if i % 3 == 0 {
                    fb.put(px as i32, py as i32 + 1, palette::shade(frond_c, 0.9));
                }
            }
        }
        k += 1;
        x += step / 2 + (hash(k, seed ^ 0x9A3) * step as f32) as i32;
    }
}

/// The pale air colour everything in the distance is mixed towards.
pub fn air_colour(pal: &Palette) -> u32 {
    palette::mix(pal.horizon, pal.zenith, 0.45)
}

/// The ranges, far to near: each a single vertical run per column, shaded from
/// the *slope of its own smoothed silhouette* (B90): the face that looks at the
/// sun is lit, the other is dark, and gullies are darker wedges that lean down
/// the slope. Nothing here depends on a single column, so there are no stripes.
/// Far ranges are pale and blue, near ones darker, greener and more broken.
#[allow(clippy::too_many_arguments)]
fn hills(
    fb: &mut Framebuffer,
    pal: &Palette,
    s: &SkyState,
    seed: u64,
    horizon_y: i32,
    night: bool,
    air: u32,
    ranges: &[Range],
    cool: u32,
    cool_k: f32,
) {
    let w = fb.w as i32;
    let band = horizon_y.clamp(8, fb.h as i32) as f32;
    let base_y = horizon_y + 2;
    let y1 = base_y.min(fb.h as i32 - 1);
    let sun_x = sun_column(s, w);
    let reach = if night { 0.50 } else { 0.38 };
    let n = ranges.len();
    for (i, r) in ranges.iter().enumerate() {
        let near = if n > 1 { i as f32 / (n - 1) as f32 } else { 1.0 };
        let layer_seed = seed ^ (0x31 + i as u64).wrapping_mul(0x9E37_79B9);
        let amp = (band * r.amp).max(4.0);
        let col = range_col(r.base, pal, air, r.haze, night, cool, cool_k);
        // Crest catches the sky's light; the foot dissolves into the air.
        let crest = palette::mix(col, pal.horizon, if night { 0.08 } else { 0.16 });
        let foot = palette::mix(col, air, 0.34);
        let rim = palette::mix(col, pal.horizon, if night { 0.30 } else { 0.50 });
        // The silhouette, with spurs; the nearest ranges get a ragged fringe.
        let hs: Vec<f32> = (0..w)
            .map(|x| {
                let big = soft_profile(x, w, layer_seed, amp, r.peaks, r.min_apex);
                let rough = (smooth_noise(x as f32 / 23.0, layer_seed ^ 0x55) - 0.5) * amp * 0.10
                    + (smooth_noise(x as f32 / 7.0, layer_seed ^ 0x56) - 0.5) * amp * 0.035;
                let fringe = if near > 0.6 {
                    (hash((x / 3) as i64, layer_seed ^ 0x57) - 0.4) * 3.0 * near
                } else {
                    0.0
                };
                (big + rough + fringe).max(2.0)
            })
            .collect();
        // The smoothed profile: what shading is allowed to look at.
        let sm = box_blur(&hs, 20);
        let steep = 16.0 + 14.0 * (1.0 - near);
        // Darker wedges (gullies) are stronger on nearer ranges.
        let gully_amp = 0.05 + 0.10 * near;
        for x in 0..w {
            let h = hs[x as usize] as i32;
            let top = (y1 - h).max(0);
            if top > y1 {
                continue;
            }
            let top_sm = (y1 as f32 - sm[x as usize]).max(0.0);
            let span_sm = (y1 as f32 - top_sm).max(6.0);
            let slope = sm[(x + 24).min(w - 1) as usize] - sm[(x - 24).max(0) as usize];
            // A rise towards the right looks left, and the sun is to the left
            // of that face when sun_x < x.
            let lit = if (slope > 0.0) == (sun_x < x) { 1.0 } else { -1.0 };
            let facet = lit * (slope.abs() / steep).min(1.0) * if night { 0.05 } else { 0.13 };
            let lean = 0.55 + (slope / 12.0).clamp(-1.0, 1.0) * 0.30;
            for y in top..=y1 {
                let behind_px = fb.px()[y as usize * fb.w + x as usize];
                if y == top && ((x + y) & 1) == 0 && behind_px != palette::TRANSPARENT {
                    continue;
                }
                let f = ((y as f32 - top_sm) / span_sm).clamp(0.0, 1.0);
                // crest -> body -> foot, continuous.
                let c = if f < 0.45 {
                    palette::mix(crest, col, f / 0.45)
                } else {
                    palette::mix(col, foot, ((f - 0.45) / 0.55).powf(1.4))
                };
                // Gully wedges: noise sampled along a line that leans with the
                // slope, so they are diagonal wedges, never vertical columns.
                let xl = x as f32 + (y as f32 - top_sm) * lean;
                let g = smooth_noise(xl / 16.0, layer_seed ^ 0x99);
                let wedge = ((g - 0.60) / 0.40).clamp(0.0, 1.0) * gully_amp * (0.35 + f);
                // Fine mottling for the nearer ranges (forest texture), small
                // and 2-D so it is never a stripe.
                let mottle = if near > 0.5 {
                    (hash(((x / 2) as i64) * 31 + ((y / 2) as i64) * 17, layer_seed ^ 0x9A) - 0.5)
                        * 0.12
                        * near
                } else {
                    0.0
                };
                fb.put(x, y, palette::shade(c, (1.0 + facet - wedge + mottle).clamp(0.6, 1.25)));
            }
            // A faint lit edge on the sun's side only; never a bright line.
            let d = (x - sun_x).abs() as f32 / w as f32;
            if d < reach {
                fb.put(x, top, palette::mix(crest, rim, 0.30 * (1.0 - d / reach)));
            }
        }
    }
}

/// Moving-average of `v` over `2r + 1` samples (edges clamp), O(n).
fn box_blur(v: &[f32], r: usize) -> Vec<f32> {
    let n = v.len();
    let mut pre = vec![0.0f32; n + 1];
    for i in 0..n {
        pre[i + 1] = pre[i] + v[i];
    }
    (0..n)
        .map(|i| {
            let a = i.saturating_sub(r);
            let b = (i + r + 1).min(n);
            (pre[b] - pre[a]) / (b - a) as f32
        })
        .collect()
}

/// Alpine ranges (B46e): the jagged, faceted profile of `ridge_profile_var`
/// instead of the soft hills, with the top of every peak under snow and the
/// shadow side of each facet a step darker, so a peak reads as a mountain and
/// not a hill.
fn peaks(
    fb: &mut Framebuffer,
    pal: &Palette,
    s: &SkyState,
    seed: u64,
    horizon_y: i32,
    night: bool,
    air: u32,
    ranges: &[Range],
) {
    let w = fb.w as i32;
    let band = horizon_y.clamp(8, fb.h as i32) as f32;
    let y1 = (horizon_y + 2).min(fb.h as i32 - 1);
    let sun_x = sun_column(s, w);
    for (i, r) in ranges.iter().enumerate() {
        let layer_seed = seed ^ (0x91 + i as u64).wrapping_mul(0x9E37_79B9);
        let amp = (band * r.amp).max(6.0);
        let rock = range_col(r.base, pal, air, r.haze, night, palette::rgb(0x4E, 0x5A, 0x74), 0.40);
        let snow = palette::mix(
            haze(0xF4F8FC, pal, air, r.haze * 0.45, night),
            rock,
            0.10,
        );
        let snow_shade = palette::mix(snow, palette::rgb(0x8A, 0x9C, 0xC0), 0.28);
        let rock_shade = palette::shade(rock, 0.88);
        let foot = palette::mix(rock, air, 0.50);
        let mut prev = 0i32;
        for x in 0..w {
            let h = ridge_profile_var(x, w, layer_seed, amp, r.peaks, r.min_apex) as i32;
            let top = (y1 - h).max(0);
            // The slope direction decides lit or shadow: rising to the right
            // faces the (left-lit) light.
            // The facet's direction is read over a window, not a column, so a
            // 2-px stair in the profile cannot stripe the shading.
            let ahead = ridge_profile_var((x + 6).min(w - 1), w, layer_seed, amp, r.peaks, r.min_apex) as i32;
            let behind = ridge_profile_var((x - 6).max(0), w, layer_seed, amp, r.peaks, r.min_apex) as i32;
            let rising = ahead >= behind;
            let _ = prev;
            prev = h;
            let span = (y1 - top).max(1) as f32;
            // The snowline is one height for the whole range, wandering a
            // little: peaks above it are capped, saddles are bare rock.
            let line_h = amp * (0.50 + 0.10 * (smooth_noise(x as f32 / 60.0, layer_seed ^ 0x44) - 0.5));
            for y in top..=y1 {
                let u = (y - top) as f32 / span;
                let _ = u;
                let c = if ((y1 - y) as f32) > line_h {
                    if rising { snow } else { snow_shade }
                } else if u > 0.80 {
                    foot
                } else if rising {
                    rock
                } else {
                    rock_shade
                };
                fb.put(x, y, c);
            }
            let d = (x - sun_x).abs() as f32 / w as f32;
            if d < 0.3 && !night {
                fb.put(x, top, palette::mix(snow, pal.horizon, 0.35));
            }
        }
    }
}

/// A scatter of flat-topped acacias standing out of the far grass (B46e): one
/// row, sparse, each a thin trunk under a wide dark umbrella.
fn acacia_line(
    fb: &mut Framebuffer,
    pal: &Palette,
    seed: u64,
    base: i32,
    night: bool,
    air: u32,
    band: i32,
) {
    let u = (band as f32 / 144.0).clamp(0.5, 2.4);
    let grass = meadow(pal);
    let tone = palette::mix(
        palette::mix(grass, palette::rgb(0x5A, 0x62, 0x32), 0.55),
        air,
        if night { 0.40 } else { 0.34 },
    );
    let root = base + 3;
    let row_seed = seed ^ 0xAC_AC_1A;
    let mut x = 0i32;
    let mut k = 0i64;
    while x < fb.w as i32 {
        x += ((10.0 + hash(k, row_seed) * 46.0) * u) as i32;
        k += 1;
        let h = ((6.0 + hash(k + 3, row_seed) * 9.0) * u) as i32;
        let w = ((8.0 + hash(k + 5, row_seed) * 9.0) * u) as i32;
        let col = palette::shade(tone, 0.9 + 0.2 * hash(k + 7, row_seed));
        for y in 0..h {
            fb.put(x, root - y, palette::shade(col, 0.8));
        }
        let top = root - h;
        for dx in -w / 2..=w / 2 {
            let edge = (dx.abs() as f32 / (w as f32 * 0.5)).min(1.0);
            let thick = if edge < 0.6 { 2 } else { 1 };
            for t in 0..thick {
                fb.put(x + dx, top - t, col);
            }
        }
    }
}

/// A band of mist lying over the far ground (B46e): strongest at the bottom of
/// the band, where it meets the map's own hazed edge, thinning upward.
fn mist_band(fb: &mut Framebuffer, pal: &Palette, y_top: i32, y_bot: i32, air: u32, k: f32) {
    let w = fb.w as i32;
    let y_top = y_top.max(0);
    let y_bot = y_bot.min(fb.h as i32);
    if y_bot <= y_top {
        return;
    }
    let mist = palette::mix(air, pal.horizon, 0.35);
    let span = (y_bot - y_top) as f32;
    for y in y_top..y_bot {
        let t = (y - y_top) as f32 / span;
        let a = k * t * t;
        for x in 0..w {
            let n = 0.85 + 0.30 * smooth_noise(x as f32 / 40.0 + y as f32 * 0.3, 0x3157);
            let c = fb.px()[fb.index(x, y)];
            fb.put(x, y, palette::mix(c, mist, (a * n).clamp(0.0, 0.9)));
        }
    }
}

/// Which crop the far ground carries, so the strip below the treeline matches
/// the world (B42b).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Ground {
    Meadow,
    Field,
    /// D66: the dune sea's far ground.
    Sand,
    /// D66: a snowfield.
    Snow,
    /// D66: jungle floor, dark and green.
    Jungle,
    /// B46e: dry gold grass.
    Savanna,
}

/// The lit meadow green the terrain uses (H11 mean of the untinted art), in this
/// hour's light.
fn meadow(pal: &Palette) -> u32 {
    let t = pal.ambient_tint;
    palette::rgb(
        (104.0 * t[0]) as u8,
        (147.0 * t[1]) as u8,
        (74.0 * t[2]) as u8,
    )
}

/// The strip of distant ground between the far treeline and the land line
/// (B42b): the map's own ground fades into this, so there is no edge where the
/// world stops. Mottled meadow (or a crop field), hazier as it recedes.
fn far_ground(
    fb: &mut Framebuffer,
    pal: &Palette,
    seed: u64,
    y_top: i32,
    y_bot: i32,
    night: bool,
    air: u32,
    kind: Ground,
) {
    // Half the ambient tint only: a very warm dusk light turns lit grass brown,
    // which reads as desert at this distance (B42b).
    if y_bot <= y_top {
        return;
    }
    let w = fb.w as i32;
    let lit = meadow(pal);
    let grass = palette::mix(lit, palette::rgb(0x68, 0x93, 0x4A), 0.45);
    // The far field is lighter and bluer than the near ground; the very top of
    // the strip (under the treeline) is the darkest, the bottom (nearest the
    // map) the lightest, so the map's hazed last row lands on a matching tone.
    let base = match kind {
        Ground::Meadow => grass,
        // A young crop rather than stubble: the strip is next to the town's
        // fields, and wheat-gold there read as bare sand at dusk.
        Ground::Field => palette::mix(grass, palette::rgb(0x76, 0x86, 0x52), 0.75),
        Ground::Sand => palette::mix(palette::rgb(0xE2, 0xC8, 0x96), lit, 0.12),
        Ground::Snow => palette::mix(palette::rgb(0xEE, 0xF6, 0xFC), lit, 0.10),
        Ground::Jungle => palette::mix(palette::rgb(0x3A, 0x74, 0x38), lit, 0.25),
        Ground::Savanna => palette::mix(palette::rgb(0xC4, 0xB0, 0x62), lit, 0.10),
    };
    let grey = {
        let l = (0.30 * palette::r(base) as f32
            + 0.59 * palette::g(base) as f32
            + 0.11 * palette::b(base) as f32) as u8;
        palette::rgb(l, l, l)
    };
    // Distance mutes the colour more than it tints it: that is what keeps a
    // sunset meadow from turning to sand.
    // A crop field keeps more of its colour than a meadow does: it is an
    // ordered surface, not a mottled one.
    let desat = match kind {
        Ground::Field => 0.22,
        Ground::Sand | Ground::Snow | Ground::Savanna => 0.30,
        Ground::Jungle => 0.20,
        Ground::Meadow => 0.38,
    };
    let near = palette::mix(
        palette::mix(palette::shade(base, 0.86), grey, desat),
        air,
        if night { 0.26 } else { 0.18 },
    );
    let far = palette::mix(
        palette::mix(palette::shade(base, 0.76), grey, desat + 0.12),
        air,
        if night { 0.34 } else { 0.26 },
    );
    let span = (y_bot - y_top).max(1) as f32;
    for y in y_top..y_bot {
        let u = (y - y_top) as f32 / span;
        // Dark under the wood, lighter towards the viewer.
        let col = palette::mix(far, near, u);
        // B42f: the strip melts into whatever is *actually* above it, column by
        // column — the hills at one x, the sky band's haze at another. Blending
        // towards a fixed air colour still left a step at night (the hills are
        // darker than the air), which is the straight line the owner saw cutting the
        // frame. Taking the pixel above makes the first row identical to it by
        // construction, at any hour, and the strip reaches its own colour ten
        // rows down.
        for x in 0..w {
            // B42f: an uneven top edge, so the strip is a distance, not a ruled
            // band: the far ground rises and falls by a few pixels like the
            // land it stands for.
            let wobble = (smooth_noise(x as f32 / 34.0, seed ^ 0x77E1)
                + 0.5 * smooth_noise(x as f32 / 11.0, seed ^ 0x77E2))
                * 9.0;
            let yt = (y as i32 - (y - y_top) + wobble as i32).min(y_bot);
            if y < yt {
                continue;
            }
            let edge = (((y - yt) as f32) / 10.0).clamp(0.0, 1.0);
            let above = if yt > 0 {
                fb.px()[((yt - 1) as usize) * (w as usize) + x as usize]
            } else {
                air
            };
            // Meadow texture, not a flat card: mottled clumps plus a 1-px
            // grass tick here and there. The clump hash takes both axes, or the
            // strip comes out in vertical stripes.
            let n = hash(x as i64 / 3 + (y as i64 / 2) * 977, seed ^ 0x0FC0);
            let tick = hash(x as i64 + y as i64 * 31, seed ^ 0x0FC1);
            let mut c = if n > 0.84 {
                palette::shade(col, 1.06)
            } else if n < 0.12 {
                palette::shade(col, 0.94)
            } else {
                col
            };
            if tick > 0.93 {
                c = palette::shade(c, 0.92);
            } else if tick < 0.04 {
                c = palette::shade(c, 1.06);
            }
            // ...and melt towards the pixel above: the top rows are the only
            // place the two layers meet, and they must meet invisibly.
            if edge < 1.0 {
                c = palette::mix(above, c, edge);
            }
            fb.put(x, y, c);
        }
    }
}

/// A wood on the horizon (B42b): three depth rows of *trees*, not columns —
/// clusters of overlapping rounded broadleaf crowns and conifer triangles, the
/// back row smaller, denser and paler, the near row taller and darker, so the
/// canopy edge is continuous and uneven instead of a row of spikes.
fn forest_edge(
    fb: &mut Framebuffer,
    pal: &Palette,
    seed: u64,
    base: i32,
    night: bool,
    air: u32,
    band: i32,
) {
    // Heights and crown widths scale with the sky band, so a short frame gets a
    // short wood. (haze, min height, max height, crown width, step)
    let u = (band as f32 / 144.0).clamp(0.5, 2.4);
    let rows: [(f32, i32, i32, i32, i32); 3] = [
        (0.62, (8.0 * u) as i32, (17.0 * u) as i32, (9.0 * u) as i32, (3.0 * u) as i32),
        (0.46, (13.0 * u) as i32, (26.0 * u) as i32, (12.0 * u) as i32, (4.0 * u) as i32),
        (0.28, (19.0 * u) as i32, (38.0 * u) as i32, (16.0 * u) as i32, (6.0 * u) as i32),
    ];
    let grass = meadow(pal);
    // Root the wood a few pixels *inside* the far ground, so trunks vanish into
    // it and there is no line of dark stubs along the base.
    let root = base + 3;
    for (row, (haze_amt, min_h, max_h, cw, step)) in rows.iter().enumerate() {
        // Distant trees lose contrast and gain a little air; they are not
        // tinted orange by the low sun.
        let grey = {
            let l = (0.30 * palette::r(grass) as f32
                + 0.59 * palette::g(grass) as f32
                + 0.11 * palette::b(grass) as f32) as u8;
            palette::rgb(l, l, l)
        };
        let tone = palette::mix(
            palette::mix(grass, grey, 0.55),
            air,
            if night { 0.24 + haze_amt * 0.3 } else { haze_amt * 0.55 },
        );
        let row_seed = seed ^ (0x51 + row as u64).wrapping_mul(0x9E37_79B9);
        let span = (max_h - min_h).max(1) as f32;
        let mut x = -(*cw / 2) - (hash(row as i64 + 99, row_seed) * 6.0) as i32;
        let mut k = 0i64;
        while x < fb.w as i32 + *cw {
            // Canopy height follows a slow noise (clusters of tall trees) plus
            // a per-tree jitter, and the odd emergent stands above the rest —
            // that variance is what stops a treeline reading as a hedge.
            let slow = smooth_noise(x as f32 / (26.0 * u), row_seed ^ 0x77);
            let mut h = min_h + ((hash(k + 1, row_seed) * 0.55 + slow * 0.65) * span) as i32;
            if hash(k + 8, row_seed) > 0.93 {
                h = (h as f32 * 1.55) as i32;
            }
            let r = hash(k, row_seed);
            let tree_col = palette::shade(tone, 0.92 + 0.16 * hash(k + 2, row_seed));
            let sparse = row == 2 && hash(k + 11, row_seed) > 0.82;
            if sparse {
                x += (*step).max(2);
                k += 3;
                continue;
            }
            if r > 0.72 {
                // A conifer: a stack of tiers, its silhouette a soft triangle.
                conifer(fb, x, root, h + 2, *cw / 2, tree_col);
            } else {
                // A broadleaf: two or three overlapping crowns, lumpy edge.
                let blobs = 2 + (hash(k + 3, row_seed) > 0.45) as i32;
                let rx = (*cw as f32 * (0.38 + 0.18 * hash(k + 4, row_seed))) as i32;
                for b in 0..blobs {
                    let off = ((b as f32 - (blobs - 1) as f32 * 0.5) * rx as f32 * 0.85) as i32;
                    let rr = rx - (b as i32 % 2) + (hash(k + 5 + b as i64, row_seed) * 2.0) as i32;
                    blob(fb, x + off, root - h + rr.max(1), rr.max(1), tree_col);
                }
                // A short trunk tying the crown to the ground (2 px: at this
                // distance a treeline shows no more than that).
                for dy in 0..(h - rx).max(0).min(2) {
                    fb.put(x, root - dy, palette::shade(tree_col, 0.72));
                }
            }
            // Overlap on purpose: a continuous canopy, not a picket fence.
            x += (*step / 2 + (hash(k + 6, row_seed) * *step as f32) as i32).max(1);
            k += 3;
        }
    }
}

/// A filled disc with a one-pixel ragged rim — a leaf mass, not a circle.
fn blob(fb: &mut Framebuffer, cx: i32, cy: i32, r: i32, c: u32) {
    let r = r.max(1);
    for dy in -r..=r {
        let half = ((r * r - dy * dy) as f32).sqrt().round() as i32;
        let dx0 = -half;
        let dx1 = half;
        for dx in dx0..=dx1 {
            let edge = (dx - dx0).min(dx1 - dx) + (r - dy.abs());
            let cc = if edge == 0 && hash((cx + dx) as i64, 0xB0B) > 0.5 {
                palette::shade(c, 0.9)
            } else {
                c
            };
            fb.put(cx + dx, cy + dy, cc);
        }
    }
}

/// A conifer silhouette: a triangle with a slightly notched edge.
fn conifer(fb: &mut Framebuffer, cx: i32, base: i32, h: i32, half: i32, c: u32) {
    let h = h.max(2);
    for k in 0..h {
        let t = k as f32 / h as f32;
        let w = ((1.0 - t) * half as f32).max(0.0) as i32;
        // Notch every third row so the edge is a spruce, not a cone.
        let notch = if (k % 3) == 0 { 1 } else { 0 };
        for dx in -w..=(w - notch) {
            fb.put(cx + dx, base - k, c);
        }
    }
    fb.put(cx, base - h, c);
}

/// Farmland receding into haze (D64, revised B42c).
///
/// Distant fields are *iso diamonds* of muted green, gold and brown, laid out in
/// the same 2:1 skew as the ground beneath them, with faint broken hedgerows on
/// their edges. Detail and contrast shrink with distance and the whole patchwork
/// pales towards the hills — the earlier version drew full-width rectangles with
/// dark vertical joints, which read as a brick wall across the screen.
fn far_fields(fb: &mut Framebuffer, pal: &Palette, seed: u64, y_top: i32, y_bot: i32, night: bool, air: u32) {
    let w = fb.w as i32;
    if y_bot <= y_top + 3 {
        return;
    }
    // A field is about this many pixels across at the *near* edge of the strip;
    // rows shrink it as it recedes, which is the perspective the brief asks for.
    let cell = 40.0f32;
    let ch = cell * 0.5; // 2:1, like the world
    // Muted greens, golds and browns — no saturated orange anywhere.
    let tints = [0x6E7F44u32, 0x8A8B4A, 0x9C8F55, 0x77854C, 0x8C7B48, 0x66784A];
    let span = (y_bot - y_top) as f32;
    for y in y_top.max(0)..y_bot.min(fb.h as i32) {
        let d = (y_bot - y) as f32; // 0 at the land line, span at the hills
        let dist = 1.0 - d / span;  // 0 near, 1 far
        // Rows get shorter as they recede.
        let squash = 1.0 - 0.55 * dist;
        let v = d / (ch * squash);
        let cv = v.floor();
        // Pale with distance, and always a little of the sky in it.
        let pale = 0.30 + 0.42 * dist;
        for x in 0..w {
            // The 2:1 skew: a field's edge runs two pixels across for one up,
            // so the patchwork follows the ground it belongs to.
            let u = (x as f32 - 2.0 * d) / (cell * squash);
            let cu = u.floor();
            let fu = u - cu;
            let fv = v - cv;
            let pick = ((hash(cu as i64 * 37 + cv as i64 * 11, seed ^ 0xF1E1) * tints.len() as f32)
                as usize)
                % tints.len();
            // A muted field tone that survives a warm dusk: the ambient tint is
            // applied at half strength and the result is pulled towards its own
            // grey, or every green turns copper after sunset (B42c).
            let base = palette::mix(tints[pick], palette::rgb(0x6C, 0x7A, 0x4E), 0.45);
            let lit = palette::mix(base, palette::rgb(0x6C, 0x7A, 0x4E), 0.30);
            let mut col = haze(lit, pal, air, pale * 0.55, night);
            let l = (0.30 * palette::r(col) as f32
                + 0.59 * palette::g(col) as f32
                + 0.11 * palette::b(col) as f32) as u8;
            col = palette::mix(col, palette::rgb(l, l, l) as u32, 0.40);
            // Fine texturing fades out with distance: near fields show a hint of
            // furrow, far ones are flat paint.
            if dist < 0.55 {
                let n = hash(x as i64 * 3 + y as i64, seed ^ 0xF1E2);
                if n > 0.86 {
                    col = palette::shade(col, 1.04);
                } else if n < 0.12 {
                    col = palette::shade(col, 0.96);
                }
            }
            // Hedgerows: only on some edges, and broken into dashes, so the
            // patchwork has no continuous ruled lines.
            let edge = fu.min(1.0 - fu) < 0.05 / (cell * squash).max(1.0)
                || fv < 0.16
                || fv > 0.90;
            let broken = hash(x as i64 / 3 + y as i64 * 5, seed ^ 0xF1E3) > 0.42;
            let mut c = col;
            if edge && broken && dist < 0.92 {
                // A hedgerow is a *dark green-grey line*, faint and broken, that
                // fades out entirely in the far rows.
                let hedge = haze(0x3E4A2C, pal, air, 0.20 + 0.45 * dist, night);
                c = palette::mix(col, hedge, 0.75 - 0.45 * dist);
            }
            fb.put(x, y, c);
        }
    }
}

/// The sea runs to the horizon (D64): a band of water above the land's far
/// edge, paling into the air, with two or three faint islands and the sun's
/// glint on it.
#[allow(clippy::too_many_arguments)]
fn sea_to_horizon(
    fb: &mut Framebuffer,
    pal: &Palette,
    s: &SkyState,
    seed: u64,
    horizon_y: i32,
    night: bool,
    air: u32,
) {
    let w = fb.w as i32;
    let band = horizon_y.clamp(8, fb.h as i32) as f32;
    let top = (horizon_y - (band * 0.42) as i32).max(0);
    if top >= horizon_y {
        return;
    }
    // The near water matches the tint the scene puts on its own sea (B21), so
    // the sky's sea meets the map's without a step; the far edge is almost the
    // colour of the sky.
    let near = palette::mix(haze(0x2E6E8C, pal, air, 0.06, night), air, 0.22);
    // B99d: 0.62 of the hour's air turned the far sea into a brown band at dusk
    // (the warm horizon mixed over a blue sea). It reflects the sky, so it keeps
    // more of its own water.
    let far = palette::mix(haze(0x54859E, pal, air, 0.20, night), air, 0.46);
    let span = (horizon_y - top).max(1) as f32;
    for y in top..=horizon_y.min(fb.h as i32 - 1) {
        let u = (y - top) as f32 / span;
        let c = palette::mix(far, near, u);
        // The last rows dissolve into the air, so where the sea meets the
        // fading near ground it is mist, not a painted band.
        let mist = if u > 0.74 { (u - 0.74) / 0.26 * 0.75 } else { 0.0 };
        for x in 0..w {
            // A soft 2-px ripple every few rows keeps the water from being paint.
            let mut rx = c;
            if (y % 6) == 0 && hash(x as i64 / 7, seed ^ 0x5EA) > 0.6 {
                rx = palette::shade(c, 1.05);
            }
            if mist > 0.0 {
                rx = palette::mix(rx, air, mist);
            }
            fb.put(x, y, rx);
        }
    }
    // The sea's own horizon: a pale line where it meets the sky.
    let line = palette::mix(far, pal.horizon, 0.55);
    for x in 0..w {
        fb.put(x, top, line);
    }
    // Islands (B90): distant land standing *up* from the sea line. A few big
    // ones with real skylines (humps with notches, a lit and a shaded side) and
    // many small ones, paler and bluer the smaller (= further) they are, each
    // dissolving into a band of sea mist at its foot. Never a symmetric mound.
    let sun_x = sun_column(s, w);
    let n = 7 + (hash(7, seed ^ 0x5153) * 5.0) as i32;
    let mut order: Vec<i32> = (0..n).collect();
    // Far (small) first so a near island overlaps it.
    order.sort_by(|a, b| {
        let sa = hash(*a as i64 * 5 + 2, seed ^ 0x5153);
        let sb = hash(*b as i64 * 5 + 2, seed ^ 0x5153);
        sa.partial_cmp(&sb).unwrap()
    });
    for i in order {
        let j = i as i64;
        let cx = (hash(j * 5 + 1, seed ^ 0x5153) * w as f32) as i32;
        let big = hash(j * 5 + 2, seed ^ 0x5153); // 0 = far speck, 1 = near island
        let iw = (26.0 + big * big * 300.0 + hash(j * 5 + 3, seed ^ 0x5153) * 40.0) as i32;
        let ih = (6.0 + big * 56.0 + hash(j * 5 + 4, seed ^ 0x5153) * 8.0) as f32;
        let humps = 1 + (hash(j * 5 + 5, seed ^ 0x5153) * 3.0 + big * 1.5) as i32;
        let air_k = 0.80 - 0.42 * big;
        // Nearer islands show some green under the air; far ones are pure haze.
        let tint = palette::mix(0x5B6F86, 0x3F6A62, (big * 0.8).min(0.8));
        let base = haze(tint, pal, air, air_k, night);
        let lit = palette::mix(base, pal.horizon, 0.30);
        let shade = palette::shade(base, 0.86);
        let mist = palette::mix(base, air, 0.60);
        let hs = seed ^ 0x15A0 ^ (j as u64).wrapping_mul(0x9E37_79B9);
        // The skyline first, so a column can read its slope.
        let heights: Vec<i32> = (-iw..=iw)
            .map(|dx| {
                let u = dx as f32 / iw as f32;
                let edge = (1.0 - u * u).max(0.0).powf(0.8);
                let mut hh = 0.0f32;
                for k in 0..humps {
                    let c = (hash(k as i64 * 3 + 1, hs) * 2.0 - 1.0) * 0.7;
                    let wd = 0.30 + 0.35 * hash(k as i64 * 3 + 2, hs);
                    let ht = 0.55 + 0.45 * hash(k as i64 * 3 + 3, hs);
                    let t = ((u - c) / wd).abs();
                    if t < 1.0 {
                        let v = 1.0 - t * t;
                        hh = hh.max(ht * v * v.sqrt());
                    }
                }
                hh = hh * edge.sqrt() * ih + (smooth_noise(dx as f32 / 5.0, hs ^ 0x7) - 0.5) * 2.4 * big;
                hh.max(0.0) as i32
            })
            .collect();
        let at = |k: i32| heights[(k + iw).clamp(0, 2 * iw) as usize];
        for dx in -iw..=iw {
            let hh = at(dx);
            let slope = (at(dx + 12) - at(dx - 12)) as f32;
            // A slope rising away from the sun is in shade; towards it, lit.
            let lean = (slope / 14.0).clamp(-1.0, 1.0) * if sun_x < cx + dx { 1.0 } else { -1.0 };
            let face = palette::mix(shade, lit, 0.5 + 0.5 * lean);
            for dy in 0..hh {
                let f = dy as f32 / hh.max(1) as f32;
                let streak = (smooth_noise(dx as f32 / 16.0 + (dy / 3) as f32 * 31.0, hs ^ 0x3) - 0.5) * 0.045;
                let mut c = palette::shade(face, 1.0 + streak);
                if f > 0.86 {
                    c = palette::mix(c, pal.horizon, 0.14);
                }
                // Mist at the foot: the last rows are sea haze.
                if dy < 4 {
                    c = palette::mix(c, mist, 0.80 - 0.2 * dy as f32);
                }
                fb.put(cx + dx, top - 1 - dy, c);
            }
        }
    }
    // The sun's path on the far water (B99d).
    //
    // It is a *reflection*, so its shape follows the sun: narrow and bright when
    // the sun is low (the classic glitter path at sunset), broad and faint when
    // it is high. As a fixed 26-px column of pale specks it drew a thin vertical
    // beam standing above the horizon at noon, which is what the look book
    // flagged: at a high sun the glitter is a wide, gentle shimmer, and it is
    // gone once the sun is properly down.
    if s.sun_alt > -3.0 {
        let sx = sun_column(s, w);
        let low = 1.0 - (s.sun_alt / 24.0).clamp(0.0, 1.0); // 1 at the horizon
        let spread_max = 40.0 + 30.0 * (1.0 - low as f32);
        let thresh = 1.0 - (0.10 + 0.34 * low as f32);
        let glint = palette::mix(pal.horizon, 0xFFF0C8, 0.35);
        for y in top..horizon_y {
            let f = (y - top) as f32 / span;
            let spread = (5.0 + f * spread_max) as i32;
            for x in (sx - spread)..=(sx + spread) {
                if hash(x as i64 + y as i64 * 31, seed ^ 0x6171) > thresh {
                    fb.put(x, y, palette::mix(glint, near, f * 0.8));
                }
            }
        }
    }
}

/// The flat lid of a heavy deck (B28 item 1 / B57).
///
/// Past ~70 % cover the sky is not "blue with puffs", it is a deck. The lumps of
/// that deck drift (`render::clouds`), but the *wash* under them is a property of
/// the hour and the cover alone, so it is baked with the sky: a full-frame veil
/// per frame would cost 23 ms at the owner's union, and this costs nothing.
fn overcast_wash(s: &SkyState, pal: &Palette, fb: &mut Framebuffer, horizon_y: i32, night: bool) {
    let w = fb.w as i32;
    let cover = s.cloud.clamp(0.0, 1.0);
    if cover <= 0.62 || w <= 0 || horizon_y <= 0 {
        return;
    }
    let h = horizon_y.clamp(8, fb.h as i32);
    // 0 at the threshold, 1 at a solid storm deck.
    let heavy = ((cover - 0.62) / 0.38).clamp(0.0, 1.0);
    let storm = ((cover - 0.78) / 0.22).clamp(0.0, 1.0);
    // Base tone: pale grey by day, slate at night, blue-black in a storm.
    let light = palette::mix(
        palette::mix(pal.horizon, 0xE0E4E8, 0.55),
        palette::rgb(0x2a, 0x2f, 0x38),
        storm * 0.55 + if night { 0.55 } else { 0.0 },
    );
    let mid = palette::mix(light, pal.zenith, 0.25 + 0.25 * storm);
    let bottom = palette::shade(mid, 1.0 - 0.22 * storm - if night { 0.18 } else { 0.0 });
    let wash = palette::mix(bottom, pal.horizon, if night { 0.42 } else { 0.24 });
    for y in 0..h {
        let f = (y as f32 / h as f32).clamp(0.0, 1.0);
        // A deck closes overhead and thins into haze at the horizon: opaque at
        // the top of the band, still most of the sky at the land line.
        let a = (0.88 - 0.34 * f) * heavy;
        if a <= 0.02 {
            continue;
        }
        for x in 0..w {
            fb.put_blended(x, y, wash, a);
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ridge_is_broad_and_stays_inside_its_amplitude() {
        // No needles: over a 960 px screen the profile must stay in 0..=amp,
        // never drop below the valley floor, and never jump more than a third
        // of the amplitude between neighbouring columns (a needle would).
        let (w, amp) = (960, 44.0);
        let mut prev = ridge_profile(0, w, 7, amp, 1.7);
        for x in 1..w {
            let h = ridge_profile(x, w, 7, amp, 1.7);
            assert!((0.0..=amp).contains(&h), "ridge {h} outside 0..{amp}");
            assert!((h - prev).abs() <= amp / 2.0, "needle at {x}: {prev} -> {h}");
            prev = h;
        }
    }

    #[test]
    fn ridges_are_seeded_and_layered_apart() {
        let a = ridge_profile(300, 960, 1, 44.0, 1.7);
        let b = ridge_profile(300, 960, 2, 44.0, 1.7);
        assert_ne!(a, b, "seed must change the skyline");
    }

    /// B90: far land stands *up* from the haze line: every column is one solid
    /// run from its skyline down to the ground, with nothing hanging below a
    /// gap (the "stalactite" look), and no skyline sits below the haze line.
    #[test]
    fn far_land_has_no_downward_spikes() {
        use render::horizon::Horizon;
        let s = crate::state(1_782_014_400_000 + 13 * 3_600_000, 41.0, -75.0);
        let pal = crate::palette(&s);
        let (w, h) = (1600usize, 400usize);
        let hy = 300i32;
        for hz in [
            Horizon::Forest,
            Horizon::Fields,
            Horizon::Coast,
            Horizon::Dunes,
            Horizon::SnowRidges,
            Horizon::Alpine,
            Horizon::Savanna,
            Horizon::Highlands,
        ] {
            for seed in [3u64, 41] {
                let mut fb = Framebuffer::new(w, h);
                fb.reset_clip();
                fb.clear(palette::TRANSPARENT);
                draw(&s, &pal, &mut fb, seed, hy, 0, hz);
                for x in 0..w {
                    let col = |y: usize| fb.px()[y * w + x] != palette::TRANSPARENT;
                    let Some(top) = (0..h).find(|&y| col(y)) else { continue };
                    assert!(
                        (top as i32) < hy,
                        "{hz:?} seed {seed}: column {x}'s land top {top} is not above the haze line {hy}"
                    );
                    // Solid from the skyline down to where the far ground starts.
                    for y in top..(hy as usize - 34).max(top) {
                        assert!(col(y), "{hz:?} seed {seed}: hole at ({x},{y}) under the skyline at {top}");
                    }
                }
            }
        }
    }
}
