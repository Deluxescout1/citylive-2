//! B47 — the living sky: aurora, the ISS and live flights (D67, D71, D73, D74).
//!
//! Three things up there that the wallpaper did not draw before, all of them
//! driven by the data layer B44 put in `world` (Kp, the ISS's sub-satellite
//! point, ADS-B aircraft) and all of them *real-world logic* (D73):
//!
//! * **Aurora** — the planetary Kp index turned into the level visible at this
//!   latitude ([`aurora_strength`]), drawn as three curtains of light near the
//!   top of the band. The band faces south, so the northern sky is *behind* the
//!   camera: the glow is wide and low, its brightest hem about a third of the
//!   way down the frame, with rays shimmering downward at the frame rate (D71).
//! * **The ISS** — [`iss_alpha`] gates it on the two things that make it
//!   visible (the sun down at *you*, the sun up at the *station*), and the real
//!   az/alt maps onto the band through [`crate::draw::band_x`] /
//!   [`crate::draw::band_y`], the same mapping the stars and the Moon use.
//! * **Flights** — real aircraft from the ADS-B feed as small pixel planes at
//!   their true bearing and elevation, dead-reckoned between the feed's sparse
//!   fixes, with nav lights and a strobe at night and a contrail when the air up
//!   there is cold enough ([`contrail_forms`]).
//!
//! **Cost.** Everything here draws on the *composite* (the band is only
//! re-baked about once a minute, but these move every frame) with the meteor
//! layer's discipline: the pixels from the last pass are put back by the
//! caller from the scene, the layer reports the rectangles it swept, and
//! nothing is drawn at all when nothing is up. No allocation on the hot path.
//!
//! The brief asks that a strong aurora also tints water and snow faintly; that
//! would need a static-layer re-bake per frame of glow and is left out (see the
//! handback).

use render::framebuffer::Framebuffer;
use render::palette::{self, Color};
use render::Rect;

use crate::draw::{band_x, band_y};

/// The visible sky band, in framebuffer pixels: the frame's width, the height of
/// the sky layer the land sits under (`scene.sky_height()`) and the row where
/// the skyline is — the highest thing standing up into the band (a treeline, a
/// spire, a ridge), above which every column is true sky.
///
/// The floor matters: the band is only ~144 rows tall on the owner's union, and the
/// treeline takes the bottom third of it, so the sky a person actually sees is
/// the top two-thirds. An aircraft at 12° would land behind the trees if the
/// altitude were mapped across the whole band; mapping it across the *visible*
/// sky puts it just above the treeline, which is where the eye puts it too.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Band {
    pub w: usize,
    pub h: i32,
    /// The row of the highest thing in the band; altitudes map into the rows
    /// above it. Equals `h` when nothing stands into the band.
    pub floor: i32,
}

impl Band {
    pub fn new(w: usize, h: i32) -> Self {
        Self { w, h, floor: h }
    }

    /// With the skyline: `floor` is the row the visible sky starts at.
    pub fn with_floor(w: usize, h: i32, floor: i32) -> Self {
        let floor = floor.clamp((h as f32 * 0.08) as i32, h);
        Self { w, h, floor }
    }

    /// Screen x for a true azimuth (0 = north, clockwise; south at the centre,
    /// east to the left — the band's own convention, [`band_x`]).
    #[inline]
    pub fn x(&self, az: f64) -> f32 {
        band_x(az, self.w)
    }

    /// Screen y for an altitude in degrees above the horizon: 0 is the
    /// *skyline* (`floor`, or the band's bottom when nothing stands in it) and
    /// 90 is just under the top edge.
    #[inline]
    pub fn y(&self, alt: f64) -> f32 {
        band_y(alt, self.floor, self.h.max(0) as usize)
    }

    /// Is the band big enough to draw in at all?
    #[inline]
    pub fn usable(&self) -> bool {
        self.w > 8 && self.h > 8
    }
}

/// The band's skyline: the topmost row that holds something *standing up* into
/// the sky (a tree, a spire, a wall, a ridge — the static layer's prop and
/// built tags, exactly what `Scene::mask_above_land` keeps). Altitudes map into
/// the rows above it.
///
/// One pass over the band, called when the sky is rebuilt (about once a minute),
/// never per frame.
pub fn skyline(scene: &render::Scene, band_h: i32) -> i32 {
    if band_h <= 4 {
        return band_h.max(0);
    }
    // B69: swaying trees are not in the static layer; their treeline counts.
    let tree_row = scene.tree_top_row().map(|y| (y + 1).clamp(0, band_h));
    let sl = scene.static_layer();
    let w = sl.w;
    if w == 0 {
        return band_h;
    }
    let rows = (band_h as usize).min(sl.h);
    // B84: the static layer holds nothing above its land line (every static
    // pass is clipped to it), so the scan starts there. It used to start at row
    // 0 and touch the whole sky band of the layer — ten megabytes of empty
    // pages, once per sky rebuild.
    let first = scene.static_row0().min(rows);
    let px = sl.px();
    for y in first..rows {
        let row = &px[y * w..(y + 1) * w];
        if row.iter().any(|&p| {
            matches!(
                palette::alpha(p),
                render::framebuffer::TAG_PROP | render::framebuffer::TAG_BUILT
            )
        }) {
            // One row of margin: a bird does not perch on the treeline's pixel.
            let row = (y as i32 + 1).min(band_h);
            return tree_row.map_or(row, |t| t.min(row));
        }
    }
    tree_row.unwrap_or(band_h)
}

// ------------------------------------------------------------------ aurora

/// Everything the aurora's brightness depends on. A plain struct so a caller
/// (and a test) can say exactly what sky it means.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AuroraSky {
    /// Planetary Kp index, 0..9.
    pub kp: f32,
    /// The oval's level at this latitude, 0..1 (`Aurora::visible_at`).
    pub level: f32,
    /// The sun's altitude at the viewer, degrees.
    pub sun_alt: f64,
    /// Cloud cover, 0..1.
    pub cloud: f32,
    /// The Moon's altitude, degrees.
    pub moon_alt: f64,
    /// The Moon's illuminated fraction, 0..1.
    pub moon_illum: f64,
}

impl Default for AuroraSky {
    fn default() -> Self {
        Self {
            kp: 0.0,
            level: 0.0,
            sun_alt: -30.0,
            cloud: 0.0,
            moon_alt: -20.0,
            moon_illum: 0.0,
        }
    }
}

/// How strong the aurora is right now, 0 (nothing) .. 1 (a storm overhead).
///
/// * daylight kills it outright: above the horizon the glow is lost — the ramp
///   from −4° to −10° is the twilight edge;
/// * the level the oval reaches at this latitude sets the body of it;
/// * a deck of cloud dims it (it is high above the weather, but it is *seen
///   through* it), and a bright Moon washes the faintest parts out.
pub fn aurora_strength(s: &AuroraSky) -> f32 {
    if s.level <= 0.0 || s.kp <= 0.0 {
        return 0.0;
    }
    // Never in daylight (the brief's hard rule): fully gone above −4°.
    let dark = ((-4.0 - s.sun_alt) / 6.0).clamp(0.0, 1.0) as f32;
    if dark <= 0.0 {
        return 0.0;
    }
    let body = s.level.clamp(0.0, 1.0);
    let storm = (0.55 * body + 0.45 * ((s.kp - 4.0) / 5.0).clamp(0.0, 1.0)).clamp(0.0, 1.0);
    let cloud = s.cloud.clamp(0.0, 1.0);
    let clear = 1.0 - 0.85 * cloud.powf(1.3);
    // A bright Moon above the horizon is the other thing that hides it.
    let moon = if s.moon_alt > 0.0 {
        s.moon_illum.clamp(0.0, 1.0) as f32 * (s.moon_alt / 40.0).min(1.0) as f32
    } else {
        0.0
    };
    (storm * dark * clear * (1.0 - 0.45 * moon)).clamp(0.0, 1.0)
}

/// How much red/purple the tops carry: nothing below a moderate storm, plenty
/// in a Kp 8–9 one.
#[inline]
pub fn aurora_red(kp: f32, strength: f32) -> f32 {
    ((kp - 5.5) / 3.5).clamp(0.0, 1.0) * (strength * 1.6).clamp(0.0, 1.0)
}

/// The ink the aurora is drawn with: how strong, how red.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AuroraInk {
    pub strength: f32,
    pub red: f32,
}

impl AuroraInk {
    /// Build the ink from a sky, or `None` when there is nothing to draw —
    /// which is what keeps a quiet night exactly as free as it was.
    pub fn of(s: &AuroraSky) -> Option<Self> {
        let strength = aurora_strength(s);
        if strength < 0.02 {
            return None;
        }
        Some(Self {
            strength,
            red: aurora_red(s.kp, strength),
        })
    }
}

// The curtain's colours: an aurora is emitted light, not reflected sky, so
// these do not come from the palette. A green body with a white-green rim and a
// cyan tint above it (the old CityLive's three curtains were green, cyan and
// violet), red-violet tops only in a real storm.
const AURORA_GREEN: Color = 0x3C_E0_82;
/// The hem's colour: the curtain's own green, nudged toward its cyan. B99c round
/// 2 — the old pale rim (0xA6F4C6) read as snow on a ridge at 1x; an aurora's
/// lowest light is still its own colour, just brighter.
const AURORA_HEM: Color = 0x86_F2_C2;
const AURORA_CYAN: Color = 0x74_D8_E6;
const AURORA_VIOLET: Color = 0xA8_74_EE;
const AURORA_RED: Color = 0xFF_58_6A;

/// Splitmix64's finaliser, for a stable 0..1 per column.
#[inline]
fn hash01(x: u64) -> f32 {
    let mut z = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z >> 11) as f32 / (1u64 << 53) as f32
}

/// Add `c` to a pixel, `cov` of the way — the old CityLive's `lighter` blend,
/// which is what an emissive curtain in front of a night sky does.
#[inline]
fn add_px(fb: &mut Framebuffer, x: i32, y: i32, c: Color, cov: f32) {
    if cov <= 0.0 {
        return;
    }
    let clip = fb.clip();
    if x < clip.x0 || x >= clip.x1 || y < clip.y0 || y >= clip.y1 {
        return;
    }
    let i = fb.index(x, y);
    let px = fb.px_mut();
    let old = px[i];
    let r = (((old >> 16) & 0xFF) as f32 + palette::r(c) as f32 * cov).min(255.0) as u32;
    let g = (((old >> 8) & 0xFF) as f32 + palette::g(c) as f32 * cov).min(255.0) as u32;
    let b = ((old & 0xFF) as f32 + palette::b(c) as f32 * cov).min(255.0) as u32;
    px[i] = (old & 0xFF00_0000) | (r << 16) | (g << 8) | b;
}

/// Blend `c` over a pixel, `cov` of the way — for the things that are *opaque*
/// and merely lit (an airframe silhouette at dusk), which an additive pass
/// would turn into a white streak.
#[inline]
fn blend_px(fb: &mut Framebuffer, x: i32, y: i32, c: Color, cov: f32) {
    if cov <= 0.0 {
        return;
    }
    let clip = fb.clip();
    if x < clip.x0 || x >= clip.x1 || y < clip.y0 || y >= clip.y1 {
        return;
    }
    let i = fb.index(x, y);
    let px = fb.px_mut();
    px[i] = palette::blend(px[i], c, cov);
}

/// Is the composite pixel at `(x, y)` still bare sky? (The meteor layer's test:
/// the sky framebuffer only holds the band, so land, treeline, buildings and
/// clouds all differ from it and are left alone.)
#[inline]
fn bare_sky(fb: &Framebuffer, sky: &Framebuffer, x: i32, y: i32) -> bool {
    if sky.w != fb.w || x < 0 || y < 0 || x >= fb.w as i32 || y >= sky.h as i32 {
        return false;
    }
    let i = fb.index(x, y);
    (fb.px()[i] ^ sky.px()[i]) & 0x00FF_FFFF == 0
}

/// Paint one vertical run of `n` px wide and `rows` tall, adding `c` at `a`
/// over whatever is bare sky. The clip and the two framebuffers are resolved
/// once, so a pixel costs a compare and three adds.
#[inline]
fn add_run(
    fb: &mut Framebuffer,
    sky: &Framebuffer,
    x: i32,
    n: i32,
    y0: i32,
    y1: i32,
    c: Color,
    a: f32,
) {
    if a <= 0.002 || n <= 0 {
        return;
    }
    let clip = fb.clip();
    let x0 = x.max(clip.x0);
    let x1 = (x + n).min(clip.x1);
    if x0 >= x1 {
        return;
    }
    let y0 = y0.max(clip.y0);
    let y1 = y1.min(clip.y1).min(sky.h as i32);
    if y0 >= y1 {
        return;
    }
    let w = fb.w;
    let same = sky.w == w;
    let (dr, dg, db) = (
        palette::r(c) as f32 * a,
        palette::g(c) as f32 * a,
        palette::b(c) as f32 * a,
    );
    let sk = if same { sky.px() } else { &[] };
    let px = fb.px_mut();
    for y in y0..y1 {
        let base = y as usize * w;
        for i in base + x0 as usize..base + x1 as usize {
            if same && (px[i] ^ sk[i]) & 0x00FF_FFFF != 0 {
                continue;
            }
            let old = px[i];
            let r = (((old >> 16) & 0xFF) as f32 + dr).min(255.0) as u32;
            let g = (((old >> 8) & 0xFF) as f32 + dg).min(255.0) as u32;
            let b = ((old & 0xFF) as f32 + db).min(255.0) as u32;
            px[i] = (old & 0xFF00_0000) | (r << 16) | (g << 8) | b;
        }
    }
}

/// Add `c` along one row over bare sky, thinning out as `a` falls: a pixel
/// survives with probability `a` at a strength of `a`, so a fade becomes a dither
/// that disappears into the sky instead of a bar of colour with a hard bottom
/// edge. This is the curtain's hem (B99c round 2).
#[inline]
#[allow(clippy::too_many_arguments)]
fn add_row_fade(
    fb: &mut Framebuffer,
    sky: &Framebuffer,
    x: i32,
    n: i32,
    y: i32,
    c: Color,
    a: f32,
    salt: u64,
) {
    if a <= 0.004 || n <= 0 {
        return;
    }
    let clip = fb.clip();
    let x0 = x.max(clip.x0);
    let x1 = (x + n).min(clip.x1);
    if x0 >= x1 || y < clip.y0 || y >= clip.y1 || y >= sky.h as i32 {
        return;
    }
    let w = fb.w;
    let same = sky.w == w;
    let (dr, dg, db) = (
        palette::r(c) as f32 * a,
        palette::g(c) as f32 * a,
        palette::b(c) as f32 * a,
    );
    let sk = if same { sky.px() } else { &[] };
    let px = fb.px_mut();
    let base = y as usize * w;
    for i in base + x0 as usize..base + x1 as usize {
        let s = (i as u64).wrapping_mul(0x9E37_79B1) ^ salt;
        if hash01(s) > a.min(1.0) {
            continue;
        }
        if same && (px[i] ^ sk[i]) & 0x00FF_FFFF != 0 {
            continue;
        }
        let old = px[i];
        let r = (((old >> 16) & 0xFF) as f32 + dr).min(255.0) as u32;
        let g = (((old >> 8) & 0xFF) as f32 + dg).min(255.0) as u32;
        let b = ((old & 0xFF) as f32 + db).min(255.0) as u32;
        px[i] = (old & 0xFF00_0000) | (r << 16) | (g << 8) | b;
    }
}

/// Add colour to a 2×`n` block, `a` of the way, over bare sky only. The body of/// the curtain is drawn in these blocks: two pixels tall and `COL_W` wide, which
/// is the same granularity as the sprite art around it, so the soft gradient
/// steps in *pixels* rather than in bands.
#[inline]
fn add_block(
    fb: &mut Framebuffer,
    sky: &Framebuffer,
    x: i32,
    n: i32,
    y: i32,
    c: Color,
    a: f32,
) {
    if a <= 0.004 {
        return;
    }
    let clip = fb.clip();
    let x0 = x.max(clip.x0);
    let x1 = (x + n).min(clip.x1);
    if x0 >= x1 {
        return;
    }
    let w = fb.w;
    let same = sky.w == w;
    let (dr, dg, db) = (
        palette::r(c) as f32 * a,
        palette::g(c) as f32 * a,
        palette::b(c) as f32 * a,
    );
    let sk = if same { sky.px() } else { &[] };
    let px = fb.px_mut();
    for yy in [y, y + 1] {
        if yy < clip.y0 || yy >= clip.y1 || yy >= sky.h as i32 || yy < 0 {
            continue;
        }
        let base = yy as usize * w;
        for i in base + x0 as usize..base + x1 as usize {
            if same && (px[i] ^ sk[i]) & 0x00FF_FFFF != 0 {
                continue;
            }
            let old = px[i];
            let r = (((old >> 16) & 0xFF) as f32 + dr).min(255.0) as u32;
            let g = (((old >> 8) & 0xFF) as f32 + dg).min(255.0) as u32;
            let b = ((old & 0xFF) as f32 + db).min(255.0) as u32;
            px[i] = (old & 0xFF00_0000) | (r << 16) | (g << 8) | b;
        }
    }
}

/// The width of one column of the curtain, in framebuffer pixels.
const COL_W: i32 = 3;
/// How far apart the curtain's folds are, in framebuffer pixels (a fold is
/// ~40–70 px wide here, which is what reads as a ray at 3x).
const FOLD_PX: f32 = 64.0;

/// One column of the curtain: from this column's own tip down to the bright hem,
/// in 2-px blocks, plus its tail hanging below the hem.
///
/// The tip height is the ray's own (a few columns share one), so the top of the
/// curtain is a ragged edge of ray tips rather than a straight cut, and the
/// body's brightness rises towards the hem.
#[allow(clippy::too_many_arguments)]
fn aurora_column(
    fb: &mut Framebuffer,
    sky: &Framebuffer,
    x: i32,
    top: f32,
    hem: f32,
    red: f32,
    a_peak: f32,
    patch: f32,
) {
    let y_hem = hem.round() as i32;
    let y_top = top.round() as i32;
    if y_hem <= y_top {
        return;
    }
    let span = (y_hem - y_top) as f32;
    // The seam of a storm's red-violet: the tops of the rays shade out of green
    // into it — a gradient, never a band across the sky (a hard line up there
    // reads as a mountain ridge, not as an aurora).
    let cap = if red > 0.08 {
        Some(if red > 0.5 {
            palette::mix(AURORA_VIOLET, AURORA_RED, (red - 0.5) * 2.0)
        } else {
            AURORA_VIOLET
        })
    } else {
        None
    };
    // The body: brightest at the hem, decaying upward; high up it goes cyan and,
    // in a storm, red-violet.
    let mut y = y_top;
    while y < y_hem {
        let u = ((y - y_top) as f32 / span).clamp(0.0, 1.0);
        let prof = u.powf(2.1);
        // In a storm the top of the curtain shades out of green into
        // violet-red — mixed into the body colour, so the tint follows the
        // light instead of drawing an outline along the silhouette.
        let body = match cap {
            Some(c) if u < 0.55 => palette::mix(AURORA_GREEN, c, red * 0.85 * (1.0 - u / 0.55)),
            _ => AURORA_GREEN,
        };
        add_block(fb, sky, x, COL_W, y, body, a_peak * prof);
        if u < 0.45 {
            // The blue-green of a real curtain's upper half — faded by the same
            // profile, or it draws a fringe along the silhouette.
            add_block(
                fb,
                sky,
                x,
                COL_W,
                y,
                AURORA_CYAN,
                a_peak * 0.30 * prof * (1.0 - u / 0.45),
            );
        }
        y += 2;
    }
    // The hem: the curtain's last light, thinning out into the sky (B99c round
    // 2). The body above is untouched — the rays and their ragged tips were the
    // part that read right. What changes is the bottom: no bar, no slab under it,
    // just a 6–12 row fade in the curtain's own green, dithered and a different
    // depth per column, so the lower edge varies instead of ruling a line.
    let salt = (x as u64).wrapping_mul(0x9E37_79B1) ^ 0x51ED_2C7F;
    let depth = 6 + (hash01(salt) * 7.0) as i32; // 6..=12 rows
    // Dimmer than the body's hem row and falling quadratically, or the fade is
    // itself a band: the curtain's last light is a glow, not a stripe.
    let rim = a_peak * (0.34 + 0.20 * patch);
    for k in 0..depth {
        let f = 1.0 - (k as f32 + 0.5) / depth as f32;
        let a = rim * f * f;
        if a > 0.10 {
            // The body's own grain: a solid row, dimming as it goes down.
            add_run(fb, sky, x, COL_W, y_hem + k, y_hem + k + 1, AURORA_HEM, a);
        } else {
            // The last of the light breaks up instead of stopping on a line.
            add_row_fade(fb, sky, x, COL_W, y_hem + k, AURORA_HEM, a, salt ^ (0x40 + k as u64));
        }
    }
}

/// Draw the aurora into the composite. `t_ms` is the animation clock (full frame
/// rate, D71); `ink` comes from [`AuroraInk::of`]. Returns the rectangle swept
/// (the whole curtain, whether or not every pixel of it was sky), or `None`
/// when nothing was painted.
///
/// The curtain faces north, and the band's north edge is *behind* the camera, so
/// there is no azimuth to place it at: it is a wide sheet of rays across the
/// upper band, dense and bright along a hem past half way down, ragged at the
/// top where the ray tips end.
pub fn draw_aurora(
    fb: &mut Framebuffer,
    sky: &Framebuffer,
    band: Band,
    t_ms: i64,
    ink: &AuroraInk,
) -> Option<Rect> {
    if !band.usable() || ink.strength < 0.02 {
        return None;
    }
    let t = t_ms as f32 * 0.001;
    let h = band.h as f32;
    let w = fb.w.min(band.w);
    if w == 0 {
        return None;
    }
    let k = ink.strength;
    let hem_base = h * 0.60;
    // The curtain's extent, from geometry: the top of the frame down to the
    // lowest row a tail can reach. That is the damage, whatever happened to be
    // sky.
    let bottom = (hem_base + h * 0.12).min(band.h as f32) as i32;
    // The hem is a slow wave: an eastward drift plus three periods, evaluated
    // fresh every frame so it never steps (D71).
    let hem_at = |x: f32| {
        let drift = t * 0.9;
        hem_base
            + (((x + drift) * 0.0031 + t * 0.10).sin() * 0.045
                + ((x + drift * 0.6) * 0.0107 - t * 0.17).sin() * 0.016
                + ((x + drift * 1.4) * 0.0341 + t * 0.31).sin() * 0.005)
                * h
    };
    // Each ray spans a few columns, so its tip is coherent instead of a comb:
    // one height per group, with a little raggedness on top.
    let mut x = 0i32;
    while x < w as i32 {
        let fx = x as f32;
        // The top of the curtain is a continuous, ragged line: a low wave
        // everywhere, and folds where the sheet turns — smooth triangular peaks
        // a few tens of pixels wide, never a rectangle (a fold is a ridge of
        // stacked light, and a ridge has sloped sides).
        let wave = 0.15
            + 0.055 * (fx * 0.0121 + t * 0.09).sin()
            + 0.032 * (fx * 0.0302 - t * 0.14).sin()
            + 0.018 * (fx * 0.071 + t * 0.23).sin();
        let mut fold = 0.0f32;
        let gi = (fx / FOLD_PX) as i64;
        for g in (gi - 1)..=(gi + 1) {
            let centre = g as f32 * FOLD_PX + FOLD_PX * 0.5;
            let hgt = hash01((g as u64).wrapping_mul(0x9E37_79B1) ^ 0x51ED_2C7F);
            // A little less than half the columns carry a fold.
            let hgt = ((hgt - 0.42) / 0.58).max(0.0);
            let dx = ((fx - centre) / (FOLD_PX * 0.62)).abs();
            let fall = (1.0 - dx).max(0.0);
            fold = fold.max(hgt * fall * fall.sqrt());
        }
        let up = h * (wave + 0.40 * fold * (0.45 + 0.55 * k));
        let hem = hem_at(fx);
        let top = (hem - up).max(h * 0.02);
        // The shimmer: this column brightens and dims with its own slow phase.
        let shim = 0.5 + 0.5 * (t * 0.62 + hash01(x as u64 ^ 0x9E37) * 6.283_185).sin();
        // And the curtain is brighter in folds: two periods across the width,
        // the slow one making a wide bright patch, the fast one a band.
        let patch = ((fx * 0.0016 + t * 0.05).sin() * (fx * 0.0007 - t * 0.03).cos()).abs();
        let vary = hash01((gi as u64).wrapping_mul(0x27D4_EB2F) ^ 0x1656_67B1);
        // A fold stacks light, so it reads brighter as well as taller.
        let a_peak = 0.98
            * k
            * (0.62 + 0.38 * shim)
            * (0.72 + 0.28 * patch)
            * (0.85 + 0.15 * vary)
            * (0.85 + 0.30 * fold);
        aurora_column(fb, sky, x, top, hem, ink.red, a_peak, patch);
        // B99c round 2: no tails. The blocks hanging under every other column
        // were the flat slab below the cut; the hem's own fade is the bottom now.
        x += COL_W;
    }
    Some(Rect::new(0, 0, w as i32, bottom))
}

// --------------------------------------------------------------------- ISS

/// The ISS is visible when **you** are in the dark and the **station** is in
/// sunlight: `look_alt` is its altitude above your horizon (the brief's > 10°),
/// `sun_viewer` the sun's altitude where you are, `sun_subpoint` the sun's
/// altitude at the point under the station. At 420 km the Earth's limb shadows
/// the station once the sun at its sub-point is below about −20°.
///
/// Returns the ink (0..1), with soft edges at all three limits so nothing pops.
pub fn iss_alpha(look_alt: f64, sun_viewer: f64, sun_subpoint: f64) -> f32 {
    // Daylight at the viewer: nothing to see.
    if sun_viewer > -2.0 {
        return 0.0;
    }
    // Low above the horizon: fade in over the first few degrees of the pass.
    let alt = ((look_alt - 6.0) / 6.0).clamp(0.0, 1.0) as f32;
    // The eclipse edge: the station is in the Earth's shadow when the sun at
    // its sub-point drops below the limb angle.
    const LIMB: f64 = -20.3;
    let lit = ((sun_subpoint - (LIMB - 4.0)) / 5.0).clamp(0.0, 1.0) as f32;
    (alt * lit).clamp(0.0, 1.0)
}

/// Draw the station as what it is: a bright, steady point (no blink, no trail)
/// with the faintest halo. Returns the swept rect.
pub fn draw_iss(
    fb: &mut Framebuffer,
    sky: &Framebuffer,
    band: Band,
    x: f32,
    y: f32,
    a: f32,
) -> Option<Rect> {
    if !band.usable() || a <= 0.02 {
        return None;
    }
    let (xi, yi) = (x.round() as i32, y.round() as i32);
    if xi < 1 || yi < 1 || xi + 2 >= fb.w as i32 || yi + 2 >= band.h {
        return None;
    }
    if !bare_sky(fb, sky, xi, yi) {
        return None;
    }
    // A 2×2 body reads as a point of light at 3×; a one-pixel halo keeps it from
    // being a hard square. Steady: the alpha only follows the pass.
    add_px(fb, xi, yi, 0xE8_F2_FF, 0.95 * a);
    add_px(fb, xi + 1, yi, 0xE8_F2_FF, 0.75 * a);
    add_px(fb, xi, yi + 1, 0xD8_E4_F8, 0.70 * a);
    add_px(fb, xi + 1, yi + 1, 0xD8_E4_F8, 0.50 * a);
    for (dx, dy, w) in [(-1, 0, 0.16), (2, 0, 0.16), (0, -1, 0.16), (0, 2, 0.12), (-1, 1, 0.10), (2, 2, 0.10)]
    {
        if bare_sky(fb, sky, xi + dx, yi + dy) {
            add_px(fb, xi + dx, yi + dy, 0xBF_D8_FF, w * a);
        }
    }
    Some(Rect::new(xi - 1, yi - 1, xi + 3, yi + 3))
}

// ----------------------------------------------------------------- flights

/// A flight as the band shows it: where, which way the nose points on screen,
/// and whether a contrail is behind it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlanePose {
    /// Band position, pixels.
    pub x: f32,
    pub y: f32,
    /// The aircraft's *screen* direction of travel, unit vector (nose side).
    pub dx: f32,
    pub dy: f32,
    /// Nose left or right — which wingtip carries the red light.
    pub dir: i8,
    /// Low, and close: the airfield's own traffic.
    pub low: bool,
    /// A contrail forms at this height and temperature.
    pub contrail: bool,
    /// Fade, 0..1 (entering or leaving the band, or a fix gone stale).
    pub alpha: f32,
}

/// Does a contrail form behind an aircraft at `alt_m`, with a ground
/// temperature of `ground_c`? A standard lapse (−6.5 °C/km) and the classic
/// rule of thumb: the exhaust freezes below about −40 °C, and it has to be high
/// enough for the trail to read as one.
pub fn contrail_forms(alt_m: f32, ground_c: f32) -> bool {
    if alt_m < 7000.0 {
        return false;
    }
    ground_c - 6.5 * (alt_m / 1000.0) <= -40.0
}

/// The elevation of something `alt_m` high and `dist` metres away: the angle a
/// person actually sees. An airliner 10 km up and 30 km out sits at 18° — near
/// the horizon, which is where aircraft are.
pub fn elevation_deg(east_m: f64, north_m: f64, alt_m: f32) -> f64 {
    let ground = (east_m * east_m + north_m * north_m).sqrt().max(1.0);
    (alt_m as f64 / ground).atan().to_degrees()
}

/// The bearing of an aircraft from the viewer, degrees from north.
pub fn bearing_deg(east_m: f64, north_m: f64) -> f64 {
    east_m.atan2(north_m).to_degrees().rem_euclid(360.0)
}

/// Draw one aircraft: a small side-on plane, nav lights and a strobe at night,
/// a grey silhouette by day, and a contrail when the air is cold. Returns the
/// swept rectangle.
pub fn draw_plane(
    fb: &mut Framebuffer,
    sky: &Framebuffer,
    band: Band,
    p: &PlanePose,
    t_ms: i64,
    night: bool,
) -> Option<Rect> {
    if !band.usable() || p.alpha <= 0.02 {
        return None;
    }
    let xi = p.x.round() as i32;
    let yi = p.y.round() as i32;
    if yi < 3 || yi + 3 >= band.h || xi < -8 || xi + 8 >= fb.w as i32 {
        return None;
    }
    let a = p.alpha;
    let mut rect: Option<Rect> = None;
    // The airframe and its trail are lit surfaces: blended. The lights are
    // emitters: added.
    let put = |fb: &mut Framebuffer, x: i32, y: i32, c: Color, cov: f32| {
        if bare_sky(fb, sky, x, y) {
            blend_px(fb, x, y, c, cov);
        }
    };
    let light = |fb: &mut Framebuffer, x: i32, y: i32, c: Color, cov: f32| {
        if bare_sky(fb, sky, x, y) {
            add_px(fb, x, y, c, cov);
        }
    };
    // The contrail first, so the aircraft sits at its head: a long thin streak
    // behind it, fading with distance (a real one is kilometres long).
    if p.contrail {
        let len = if p.low { 30.0 } else { 84.0 };
        let (bx, by) = (-p.dx, -p.dy);
        let mut d = 4.0f32;
        while d < len {
            let tx = p.x + bx * d;
            let ty = p.y + by * d;
            let fade = (1.0 - d / len).powf(1.4) * 0.26 * a;
            if fade > 0.02 && ty >= 0.0 && ty < band.h as f32 {
                let (tx_i, ty_i) = (tx.round() as i32, ty.round() as i32);
                put(fb, tx_i, ty_i, 0xE8_EE_F4, fade);
                // Two rows wide near the aircraft, one further back.
                if d < len * 0.45 {
                    put(fb, tx_i, ty_i + 1, 0xDA_E2_EC, fade * 0.65);
                }
                rect = Some(match rect {
                    None => Rect::new(tx_i, ty_i, tx_i + 1, ty_i + 2),
                    Some(r) => r.union(&Rect::new(tx_i, ty_i, tx_i + 1, ty_i + 2)),
                });
            }
            d += 1.5;
        }
    }
    // The airframe: 9 px of fuselage, a fin, a wing. Side-on, because that is
    // what is readable at this size (D4 pixel style).
    let body = if night { 0x6C_76_8A } else { 0xA8_B2_C2 };
    let wing = if night { 0x54_5C_6E } else { 0x8C_96_A6 };
    let d = p.dir as i32;
    for i in -4..=4 {
        put(fb, xi + i, yi, body, a);
        put(fb, xi + i, yi + 1, body, a * 0.8);
    }
    // Fin at the tail, on the side opposite the nose.
    put(fb, xi - d * 3, yi - 1, body, a);
    put(fb, xi - d * 4, yi - 2, body, a * 0.85);
    // Wing, the darker underside of it.
    for i in -2..=2 {
        put(fb, xi + i, yi + 2, wing, a * 0.75);
    }
    // Nav lights: red to port, green to starboard, and a white strobe.
    if night {
        let (port, starboard) = if d >= 0 { (-4, 4) } else { (4, -4) };
        light(fb, xi + port, yi + 2, 0xFF_33_44, a);
        light(fb, xi + starboard, yi + 2, 0x33_FF_66, a);
        // An anti-collision strobe: two quick flashes a second.
        let ph = (t_ms.rem_euclid(1200)) as i32;
        if ph < 90 || (240..330).contains(&ph) {
            light(fb, xi, yi - 1, 0xFF_FF_FF, a);
            light(fb, xi - d, yi - 1, 0xE8_F0_FF, a * 0.4);
        }
        if p.low {
            // Landing lights, pointing the way it is going.
            light(fb, xi + d * 5, yi + 1, 0xFF_F4_BE, a);
            light(fb, xi + d * 6, yi + 1, 0xFF_F4_BE, a * 0.55);
        }
    }
    rect = Some(match rect {
        None => Rect::new(xi - 6, yi - 3, xi + 7, yi + 4),
        Some(r) => r.union(&Rect::new(xi - 6, yi - 3, xi + 7, yi + 4)),
    });
    rect
}

/// A flight from the feed, dead-reckoned forward from its last fix.
#[derive(Clone, Debug)]
pub struct Track {
    /// Its ICAO hex (or the callsign when the feed omitted the hex): the key
    /// that keeps one aircraft one track between fixes.
    key: String,
    east_m: f64,
    north_m: f64,
    alt_m: f32,
    speed_kt: f32,
    heading: f32,
    baro_rate: f32,
    at_ms: i64,
    until_ms: i64,
}

impl Track {
    /// Start a track from one feed record.
    pub fn from_flight(key: &str, f: &world::flights::Flight, at_ms: i64) -> Self {
        Self {
            key: key.to_string(),
            east_m: f.east_m,
            north_m: f.north_m,
            alt_m: f.altitude_m,
            speed_kt: f.speed_kt,
            heading: f.heading,
            baro_rate: f.baro_rate,
            at_ms,
            // A fix is good for a couple of minutes: after that the aircraft is
            // somewhere we do not know and it is dropped (no fake ghosts).
            until_ms: at_ms + 150_000,
        }
    }

    /// Which aircraft this is.
    pub fn key(&self) -> &str {
        &self.key
    }

    /// Has its fix run out (the feed stopped mentioning it, so we no longer
    /// know where it is)?
    pub fn is_stale(&self, now_ms: i64) -> bool {
        now_ms > self.until_ms + 60_000
    }

    /// A new fix for the same aircraft.
    pub fn advance(&mut self, f: &world::flights::Flight, at_ms: i64) {
        self.east_m = f.east_m;
        self.north_m = f.north_m;
        self.alt_m = f.altitude_m;
        self.speed_kt = f.speed_kt;
        self.heading = f.heading;
        self.baro_rate = f.baro_rate;
        self.at_ms = at_ms;
        self.until_ms = at_ms + 150_000;
    }

    pub fn alt_m(&self) -> f32 {
        self.alt_m
    }

    /// Where it is at `now_ms`: metres east/north and metres up.
    pub fn at(&self, now_ms: i64) -> (f64, f64, f32) {
        let dt = ((now_ms - self.at_ms) as f64 / 1000.0).clamp(-5.0, 150.0);
        let mps = self.speed_kt as f64 * 0.514_444;
        let hd = (self.heading as f64).to_radians();
        let east = self.east_m + hd.sin() * mps * dt;
        let north = self.north_m + hd.cos() * mps * dt;
        // Vertical: feet per minute to metres, clamped the way a real climb or
        // descent is (no aircraft holds 4000 fpm for two minutes).
        let vs = (self.baro_rate as f64 * 0.00508).clamp(-7.0, 9.0);
        let alt = (self.alt_m as f64 + vs * dt).max(0.0) as f32;
        (east, north, alt)
    }

    /// Its pose in the band at `now_ms`, with the screen direction of travel
    /// taken from one second earlier (so the nose and the contrail agree with
    /// the motion, whatever the mapping does at that point of the sky).
    pub fn pose(
        &self,
        band: Band,
        now_ms: i64,
        ground_c: f32,
        has_airfield: bool,
    ) -> Option<PlanePose> {
        if now_ms > self.until_ms {
            return None;
        }
        let (e, n, alt) = self.at(now_ms);
        if alt < 2800.0 && !has_airfield {
            // Without an airfield in the city, low aircraft are not in our sky:
            // they are over somebody else's rooftops.
            return None;
        }
        if alt < 60.0 {
            return None;
        }
        let x = band.x(bearing_deg(e, n));
        let y = band.y(elevation_deg(e, n, alt));
        if y < 3.0 || y >= band.h as f32 - 3.0 {
            return None;
        }
        // The screen direction: where it was a second ago.
        let (e0, n0, a0) = self.at(now_ms - 1000);
        let x0 = band.x(bearing_deg(e0, n0));
        let y0 = band.y(elevation_deg(e0, n0, a0));
        let (mut dx, mut dy) = (x - x0, y - y0);
        // A step across the band's wrap is not a real direction here.
        if dx.abs() > band.w as f32 * 0.5 {
            dx = 0.0;
        }
        let m = (dx * dx + dy * dy).sqrt();
        if m < 0.001 {
            dx = 1.0;
            dy = 0.0;
        } else {
            dx /= m;
            dy /= m;
        }
        // Fade where the band's bottom edge is the horizon — an aircraft does
        // not pop into being mid-sky — and as its fix goes stale.
        let fade_bottom = ((band.h as f32 - y) / 6.0).clamp(0.0, 1.0);
        let fade_top = (y / 4.0).clamp(0.0, 1.0);
        let life = ((self.until_ms - now_ms) as f32 / 20_000.0).clamp(0.0, 1.0);
        let alpha = (fade_bottom * fade_top * life).clamp(0.0, 1.0);
        if alpha <= 0.02 {
            return None;
        }
        Some(PlanePose {
            x,
            y,
            dx,
            dy,
            dir: if dx >= 0.0 { 1 } else { -1 },
            low: alt < 2500.0 && (e * e + n * n).sqrt() < 15_000.0,
            contrail: contrail_forms(alt, ground_c),
            alpha,
        })
    }
}

// ------------------------------------------------------ the offline schedule

/// One fake flight: the schedule's stand-in when the feed has nothing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FakeFlight {
    /// Direction of flight, degrees from north.
    pub heading: f32,
    /// Height above the ground, metres.
    pub alt_m: f32,
    /// How far through its crossing it is, 0..1.
    pub progress: f32,
    /// Speed, knots.
    pub speed_kt: f32,
    /// A stable id, so one crossing is one aircraft frame to frame.
    pub id: u64,
}

/// The populated hours: the morning and evening peaks, and a thin midday.
fn busy_band(hour: f32) -> f32 {
    if (7.0..10.0).contains(&hour) || (16.0..20.0).contains(&hour) {
        0.62 // a third of the slots carry an aircraft
    } else if (10.0..16.0).contains(&hour) || (20.0..23.0).contains(&hour) {
        0.45
    } else {
        0.25
    }
}

/// The fake flights up at a local `hour` (0..24): none between 01:00 and 05:00,
/// never more than three, and often none at all — a believable sky is mostly
/// empty, which is what makes a plane worth looking at.
pub fn fake_flights(hour: f32, out: &mut Vec<FakeFlight>) {
    out.clear();
    let hour = hour.rem_euclid(24.0);
    // 01:00 to 05:00: the sky is empty (the brief's one hard rule).
    if (1.0..5.0).contains(&hour) {
        return;
    }
    let busy = busy_band(hour);
    // A slot every 20 minutes; a crossing lasts about that long, so the same
    // slot index means the same aircraft frame to frame.
    let slot = (hour * 3.0).floor() as u64;
    let phase = (hour * 60.0).fract(); // 0..1 through the slot
    let (wanted, keep) = if busy > 0.5 { (3, busy) } else { (2, busy) };
    for i in 0..wanted as u64 {
        let seed = slot.wrapping_mul(2_654_435_761).wrapping_add(i * 97);
        let h = hash01(seed);
        if h > keep {
            continue;
        }
        let idx = slot.wrapping_mul(3).wrapping_add(i);
        let h2 = hash01(idx.wrapping_mul(0x9E37_79B1) ^ 0x5DEE_CE66);
        let h3 = hash01(idx.wrapping_mul(0xC2B2_AE35) ^ 0x27D4_EB2F);
        let h4 = hash01(idx.wrapping_mul(0x85EB_CA6B) ^ 0x1656_67B1);
        // A crossing takes two slots (~40 min); each aircraft enters at its own
        // point in that window so they do not appear in a row.
        let progress = (phase * 0.5 + h4 * 0.5).fract();
        // The airways over New England run mostly east–west; some cross.
        let heading = if h2 < 0.65 {
            if h3 < 0.5 { 84.0 } else { 262.0 }
        } else {
            20.0 + h3 * 320.0
        };
        out.push(FakeFlight {
            heading,
            alt_m: 8500.0 + h2 * 4200.0,
            progress,
            speed_kt: 380.0 + h3 * 120.0,
            id: idx,
        });
        if out.len() >= 3 {
            break;
        }
    }
}

/// The pose of a fake flight in the band: a straight crossing at a plausible
/// height, so it is stateless and identical on every monitor.
pub fn fake_pose(f: &FakeFlight, band: Band, ground_c: f32) -> Option<PlanePose> {
    let alt_m = f.alt_m;
    // A crossing about 60 km out, so the elevation is a real 8–14°.
    let p = f.progress;
    let along = (p * 2.0 - 1.0) as f64 * 34_000.0;
    let across = 20_000.0 + (1.0 - (p - 0.5).abs() * 2.0) as f64 * 22_000.0;
    let hd = (f.heading as f64).to_radians();
    // East/north of the path's centre, then the path's own bearing.
    let east = along * hd.sin() + across * hd.cos() * 0.35;
    let north = along * hd.cos() - across * hd.sin() * 0.35 + 14_000.0;
    let el = elevation_deg(east, north, alt_m);
    let x = band.x(bearing_deg(east, north));
    let y = band.y(el);
    if y < 3.0 || y >= band.h as f32 - 3.0 {
        return None;
    }
    let (e0, n0) = (east - 130.0 * hd.sin(), north - 130.0 * hd.cos());
    let x0 = band.x(bearing_deg(e0, n0));
    let y0 = band.y(elevation_deg(e0, n0, alt_m));
    let (mut dx, mut dy) = (x - x0, y - y0);
    if dx.abs() > band.w as f32 * 0.5 {
        dx = 0.0;
    }
    let m = (dx * dx + dy * dy).sqrt();
    if m < 0.001 {
        dx = 1.0;
        dy = 0.0;
    } else {
        dx /= m;
        dy /= m;
    }
    // Fade in and out at the ends of the crossing.
    let edge = (p * 10.0).min((1.0 - p) * 10.0).clamp(0.0, 1.0);
    Some(PlanePose {
        x,
        y,
        dx,
        dy,
        dir: if dx >= 0.0 { 1 } else { -1 },
        low: false,
        contrail: contrail_forms(alt_m, ground_c),
        alpha: edge,
    })
}

// ---------------------------------------------------------------- restore

/// Put the pixels a previous pass painted back, from the scene, and hand the
/// rectangles on as this frame's damage.
///
/// The band is only re-baked about once a minute, so these layers live on the
/// composite; like the meteors and the wildlife, they must erase themselves
/// from the scene before the next pass draws again. Costs only the rectangles
/// that were swept (measured: 0.3 ms for the whole band at 2347×144).
pub fn restore(
    scene: &render::Scene,
    fb: &mut Framebuffer,
    painted: &mut Vec<Rect>,
    damage: &mut Vec<Rect>,
) {
    let bounds = Rect::from_size(fb.w, fb.h);
    for r in painted.drain(..) {
        let r = r.intersect(&bounds);
        if r.is_empty() {
            continue;
        }
        scene.composite_rect(fb, r);
        damage.push(r);
    }
}

/// The sun's altitude at a point on the globe: the ISS's eclipse test needs it
/// at the station's sub-satellite point.
pub fn sun_alt_at(ms: i64, lat: f64, lon: f64) -> f64 {
    world::sun::position(ms, lat, lon).0
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: usize = 2347;
    const H: i32 = 144;

    fn band() -> Band {
        Band::new(W, H)
    }

    #[test]
    fn band_mapping_cardinal_points() {
        let b = band();
        // Azimuth wraps the width: north at the edges, south at the centre.
        assert!((b.x(0.0) - 0.0).abs() < 0.5);
        assert!((b.x(180.0) - W as f32 / 2.0).abs() < 0.5);
        assert!((b.x(90.0) - W as f32 / 4.0).abs() < 0.5);
        assert!((b.x(270.0) - 3.0 * W as f32 / 4.0).abs() < 0.5);
        // And 360 is 0 again (no seam in the arithmetic).
        assert!((b.x(360.0) - b.x(0.0)).abs() < 0.5);
        // Altitude: the horizon at the band's bottom, the zenith just under
        // the top edge.
        assert!((b.y(0.0) - H as f32).abs() < 0.5);
        assert!((b.y(90.0) - 0.04 * H as f32).abs() < 1.5);
        // Monotonic: the higher up something is, the less far down it sits.
        let mut last = b.y(0.0);
        for d in 1..=90 {
            let y = b.y(d as f64);
            assert!(y <= last + 0.01, "alt {d} went down");
            last = y;
        }
    }

    #[test]
    fn the_skyline_lifts_the_horizon() {
        // With a treeline at 100 of 144 rows, an aircraft at 12° sits just
        // above it instead of behind it.
        let b = Band::with_floor(W, H, 100);
        assert!((b.y(0.0) - 100.0).abs() < 0.5);
        let y12 = b.y(12.0);
        assert!(y12 < 100.0 && y12 > 80.0, "12° at y {y12}");
        // And still monotonic up to the zenith.
        assert!(b.y(60.0) < b.y(12.0));
        // A floor that would eat the sky is clamped.
        let silly = Band::with_floor(W, H, H);
        assert!(silly.floor <= H);
    }

    #[test]
    fn nothing_in_daylight() {
        // The brief's hard rule, for both of the night layers.
        for sun in [0.0, 5.0, 20.0, 45.0, -1.0, -3.0] {
            let s = AuroraSky {
                kp: 9.0,
                level: 1.0,
                sun_alt: sun,
                cloud: 0.0,
                moon_alt: -30.0,
                moon_illum: 0.0,
            };
            assert_eq!(aurora_strength(&s), 0.0, "aurora in daylight at {sun}°");
            assert!(AuroraInk::of(&s).is_none());
        }
        // And the ISS: the sun must be down at the viewer.
        for sun in [0.0, 2.0, -1.0] {
            assert_eq!(iss_alpha(60.0, sun, 30.0), 0.0, "ISS at sun {sun}°");
        }
    }

    #[test]
    fn aurora_needs_a_storm_at_this_latitude() {
        // the test site sits at ~50.8° geomagnetic: the oval's edge is 9° north of
        // that at Kp 4, so the curtain is a faint northern glow at best and a
        // storm is what puts it overhead.
        let quiet = AuroraSky {
            kp: 4.0,
            level: world::aurora::Aurora::level_for(4.0, 41.0, -75.0),
            sun_alt: -30.0,
            cloud: 0.1,
            ..Default::default()
        };
        let storm = AuroraSky {
            kp: 9.0,
            level: world::aurora::Aurora::level_for(9.0, 41.0, -75.0),
            ..Default::default()
        };
        let q = aurora_strength(&quiet);
        let s = aurora_strength(&storm);
        assert!(q > 0.0, "the level is not zero at Kp 4");
        assert!(q < 0.15, "but it is barely there: {q}");
        assert!(s > 0.5, "Kp 9 is a storm here: {s}");
        // Well south of the oval, even a storm does not reach.
        let miami = AuroraSky {
            kp: 9.0,
            level: world::aurora::Aurora::level_for(9.0, 25.76, -80.19),
            ..Default::default()
        };
        assert_eq!(aurora_strength(&miami), 0.0, "nothing over Miami in a storm");
    }

    #[test]
    fn cloud_and_moon_dim_the_aurora() {
        let base = AuroraSky {
            kp: 9.0,
            level: 1.0,
            sun_alt: -30.0,
            cloud: 0.0,
            moon_alt: -30.0,
            moon_illum: 0.0,
        };
        let clear = aurora_strength(&base);
        let overcast = aurora_strength(&AuroraSky { cloud: 0.95, ..base });
        let moonlit = aurora_strength(&AuroraSky {
            moon_alt: 60.0,
            moon_illum: 1.0,
            ..base
        });
        assert!(
            overcast < clear * 0.4,
            "a deck hides it: {overcast} vs {clear}"
        );
        assert!(moonlit < clear * 0.75, "a full Moon washes it out");
        assert!(moonlit > 0.0, "but it does not switch it off");
        // Moonlight only counts when the Moon is up.
        let moon_down = aurora_strength(&AuroraSky {
            moon_alt: -10.0,
            moon_illum: 1.0,
            ..base
        });
        assert!((moon_down - clear).abs() < 1e-6);
    }

    #[test]
    fn red_tops_only_in_a_real_storm() {
        assert_eq!(aurora_red(4.0, 0.6), 0.0);
        assert_eq!(aurora_red(5.5, 0.6), 0.0);
        assert!(aurora_red(9.0, 1.0) > 0.8);
        let mid = aurora_red(7.0, 0.8);
        assert!(mid > 0.1 && mid < 0.8, "{mid}");
    }

    #[test]
    fn iss_needs_ten_degrees_and_a_lit_station() {
        // Too low: nothing (the fade runs from 6° to 12°).
        assert_eq!(iss_alpha(5.0, -30.0, 30.0), 0.0);
        // A good pass, sun down here, sun up there: bright.
        let full = iss_alpha(45.0, -30.0, 30.0);
        assert!(full > 0.95, "{full}");
        // It fades as it drops back to the horizon.
        let low = iss_alpha(8.0, -30.0, 30.0);
        assert!(low > 0.0 && low < 0.5, "{low}");
        // In eclipse: the sun below the station's horizon.
        assert_eq!(iss_alpha(45.0, -30.0, -35.0), 0.0);
        let edge = iss_alpha(45.0, -30.0, -22.0);
        assert!(edge > 0.0 && edge < 0.7, "fades into the shadow: {edge}");
    }

    #[test]
    fn iss_alpha_is_smooth_across_the_eclipse_edge() {
        let mut last = iss_alpha(45.0, -30.0, -10.0);
        for i in 0..40 {
            let sun = -10.0 - i as f64 * 0.5;
            let a = iss_alpha(45.0, -30.0, sun);
            assert!(a <= last + 1e-6, "went up in shadow at {sun}");
            assert!((a - last).abs() < 0.3, "a step at {sun}: {last} -> {a}");
            last = a;
        }
    }

    #[test]
    fn elevation_and_bearing_are_the_real_angles() {
        // 10 km up, 10 km out: 45°.
        let e = elevation_deg(0.0, 10_000.0, 10_000.0);
        assert!((e - 45.0).abs() < 0.5, "{e}");
        // Nothing blows up straight overhead.
        assert!(elevation_deg(0.0, 10.0, 30_000.0) > 89.0);
        // An airliner 10 km up and 60 km out is about 9.5°, near the horizon.
        let far = elevation_deg(0.0, 60_000.0, 10_000.0);
        assert!((far - 9.46).abs() < 0.2, "{far}");
        // Bearing: north 0, east 90, south 180, west 270.
        assert!((bearing_deg(0.0, 10.0) - 0.0).abs() < 0.01);
        assert!((bearing_deg(10.0, 0.0) - 90.0).abs() < 0.01);
        assert!((bearing_deg(0.0, -10.0) - 180.0).abs() < 0.01);
        assert!((bearing_deg(-10.0, 0.0) - 270.0).abs() < 0.01);
    }

    #[test]
    fn contrails_need_cold_air_up_high() {
        // 11 km on a cold day: the exhaust freezes.
        assert!(contrail_forms(11_000.0, 5.0));
        // Lower, on a hot day, the air is still too warm for one.
        assert!(!contrail_forms(9_000.0, 30.0));
        assert!(!contrail_forms(11_000.0, 45.0));
        // Below 7 km: no contrail whatever the temperature.
        assert!(!contrail_forms(6_000.0, -30.0));
    }

    #[test]
    fn the_fake_schedule_is_empty_overnight() {
        let mut v = Vec::new();
        for m in 0..(5 * 60) {
            let hour = m as f32 / 60.0;
            fake_flights(hour, &mut v);
            if (1.0..5.0).contains(&hour) {
                assert!(v.is_empty(), "a flight at {hour} local: {v:?}");
            }
        }
        // Through the day it is sparse but present.
        let mut busy_minutes = 0;
        for m in 5 * 60..24 * 60 {
            fake_flights(m as f32 / 60.0, &mut v);
            assert!(v.len() <= 3, "a swarm at minute {m}: {}", v.len());
            if !v.is_empty() {
                busy_minutes += 1;
            }
        }
        assert!(busy_minutes > 60, "the day has flights: {busy_minutes}");
        assert!(
            busy_minutes < 19 * 60,
            "but not always: {busy_minutes} of 1140 minutes"
        );
    }

    #[test]
    fn the_fake_schedule_is_stable() {
        let (mut a, mut b) = (Vec::new(), Vec::new());
        fake_flights(8.25, &mut a);
        fake_flights(8.25, &mut b);
        assert_eq!(a, b);
        // A fake flight's pose is a pure function of the flight and the band.
        if let Some(f) = a.first() {
            assert_eq!(fake_pose(f, band(), 5.0), fake_pose(f, band(), 5.0));
        }
    }

    #[test]
    fn a_track_dead_reckons_along_its_track() {
        // An aircraft due east of us, heading north at 400 kt: a minute later
        // it is about 12 km north of where it was.
        let t = Track {
            key: "abc123".into(),
            east_m: 20_000.0,
            north_m: 0.0,
            alt_m: 9_000.0,
            speed_kt: 400.0,
            heading: 0.0,
            baro_rate: 0.0,
            at_ms: 1_000_000,
            until_ms: 1_150_000,
        };
        let (e, n, alt) = t.at(1_060_000);
        assert!((e - 20_000.0).abs() < 1.0);
        assert!((n - 12_347.0).abs() < 200.0, "north {n}");
        assert!((alt - 9_000.0).abs() < 1.0);
        // Past its life it is not drawn at all.
        assert!(t.pose(band(), 1_400_000, 10.0, false).is_none());
        // Inside its life it is.
        assert!(t.pose(band(), 1_060_000, 10.0, false).is_some());
    }

    #[test]
    fn planes_without_an_airfield_are_not_in_the_sky_when_low() {
        let low = Track {
            key: "def456".into(),
            east_m: 5_000.0,
            north_m: 5_000.0,
            alt_m: 900.0,
            speed_kt: 140.0,
            heading: 90.0,
            baro_rate: -600.0,
            at_ms: 1_000_000,
            until_ms: 1_150_000,
        };
        assert!(low.pose(band(), 1_010_000, 10.0, false).is_none());
        assert!(low.pose(band(), 1_010_000, 10.0, true).is_some());
    }

    #[test]
    fn the_aurora_sweeps_only_the_top_of_the_band() {
        let mut fb = Framebuffer::new(W, H as usize);
        let sky = Framebuffer::new(W, H as usize);
        let ink = AuroraInk {
            strength: 1.0,
            red: 0.8,
        };
        let r = draw_aurora(&mut fb, &sky, band(), 1_000, &ink).expect("drawn");
        assert_eq!(r.x0, 0);
        assert_eq!(r.x1, W as i32);
        // The upper band only: the land line (the band's bottom rows) is never
        // touched.
        assert!(r.y1 < (H as f32 * 0.80) as i32, "y1 {}", r.y1);
        // Nothing in daylight: no ink, no draw.
        assert!(AuroraInk::of(&AuroraSky { kp: 9.0, level: 1.0, sun_alt: 5.0, ..Default::default() }).is_none());
    }

    #[test]
    fn the_aurora_keeps_off_the_land() {
        // A night sky above row 30; below it, land that the composite carries
        // and the sky framebuffer does not (which is exactly the test the
        // meteor layer uses).
        // The land starts three-quarters down the band, where the back-row
        // treeline stands.
        const GROUND: usize = 118;
        let land = 0xFF_55_66_44;
        let mut fb = Framebuffer::new(W, H as usize);
        let mut sky = Framebuffer::new(W, H as usize);
        for y in 0..H as usize {
            for x in 0..W {
                let c = if y < GROUND { 0xFF_00_00_00 } else { land };
                fb.put(x as i32, y as i32, c);
                sky.put(x as i32, y as i32, if y < GROUND { c } else { 0xFF_00_00_00 });
            }
        }
        let ink = AuroraInk {
            strength: 1.0,
            red: 0.0,
        };
        draw_aurora(&mut fb, &sky, band(), 1_000, &ink);
        // Every pixel of the "land" is exactly as it was.
        for y in GROUND..H as usize {
            for x in 0..W {
                assert_eq!(
                    fb.px()[y * W + x] & 0x00FF_FFFF,
                    land & 0x00FF_FFFF,
                    "the aurora painted over the land at ({x},{y})"
                );
            }
        }
        // And it did paint something up there.
        let lit = (0..GROUND)
            .flat_map(|y| (0..W).map(move |x| (x, y)))
            .filter(|&(x, y)| fb.px()[y * W + x] & 0x00FF_FFFF != 0)
            .count();
        assert!(lit > 1000, "nothing was drawn: {lit} pixels");
    }
}
