//! B78 (D84): the years on a building, painted into the static layer.
//!
//! `sim::ageing` decides what a lot wears (grime, moss, spray tags, a mural,
//! boards over the windows, shutters, scaffolding); this module puts it on the
//! pixels. Everything here runs inside the static rebuild, once per change, so
//! a weathered city costs exactly what a crisp one costs **per frame** — nothing.
//!
//! It works on the kit sprite in *sprite space*: the sprite is already on the
//! frame (blitted a moment ago), and its own pixels say where the walls, the
//! roof band and the window panes are. Painting by screen coordinate instead
//! would need the camera's maths a second time and would drift from the art.

use crate::city::CityObj;
use crate::framebuffer::Framebuffer;
use crate::palette::{self, Color};
use crate::scene::KitSprite;

// Local colour names, so the file reads like the thing it draws.
use palette::rgb;

/// Grime: soot-grey, and the green of damp stone. Mixed, never multiplied, so a
/// red brick wall greys rather than turns black.
const SOOT: Color = rgb(0x3c, 0x39, 0x33);
const MOSS: Color = rgb(0x4a, 0x6b, 0x38);
/// Dirt: the warm grey a wall turns when nobody has washed it in years.
const DIRT: Color = rgb(0x6a, 0x62, 0x54);
const LICHEN: Color = rgb(0x7d, 0x8a, 0x52);

/// Spray-can colours for a tag: nobody paints a wall in beige.
const SPRAY: [Color; 5] = [
    rgb(0xd2, 0x3c, 0xe0),
    rgb(0xf2, 0xf2, 0xf2),
    rgb(0x2e, 0xd6, 0xc8),
    rgb(0xf2, 0xc3, 0x3c),
    rgb(0xe0, 0x46, 0x3c),
];

/// Mural grounds: an invented piece, never a real artwork (D75).
const MURAL_BG: [Color; 5] = [
    rgb(0x2c, 0x5a, 0x8e),
    rgb(0xc8, 0x50, 0x3a),
    rgb(0x2e, 0x86, 0x74),
    rgb(0xd8, 0xa8, 0x3c),
    rgb(0x5a, 0x38, 0x7a),
];

/// Plywood, and its grain.
const PLY: Color = rgb(0x9a, 0x74, 0x4c);
const PLY_GRAIN: Color = rgb(0x7d, 0x5c, 0x3a);
/// A roller shutter, rib by rib.
const SHUTTER_A: Color = rgb(0xb2, 0xb2, 0xb8);
const SHUTTER_B: Color = rgb(0x92, 0x92, 0x9a);
/// Scaffolding: steel tube and scaffold board.
const TUBE: Color = rgb(0x84, 0x8a, 0x94);
const BOARD: Color = rgb(0xc0, 0x9c, 0x6c);
/// A worker's tabard.
const HIVIS: Color = rgb(0xee, 0x7a, 0x1e);
const SKIN: Color = rgb(0xd8, 0xa4, 0x7c);
const BOOT: Color = rgb(0x32, 0x2e, 0x2a);

#[inline]
fn hash(x: i32, y: i32) -> u32 {
    let mut h = (x as u32).wrapping_mul(0x8da6_bb43) ^ (y as u32).wrapping_mul(0xd816_3841);
    h ^= h >> 13;
    h = h.wrapping_mul(0xca5a_8265);
    h ^ (h >> 16)
}

#[inline]
fn opaque(c: Color) -> bool {
    c != palette::TRANSPARENT
}

#[inline]
fn emissive(c: Color) -> bool {
    palette::alpha(c) == 0xFE
}

/// `CITYLIVE_NO_WEAR=1`: draw the city as it was before B78, so a report can
/// measure the same frame with and without the years on it.
fn off() -> bool {
    use std::sync::OnceLock;
    static OFF: OnceLock<bool> = OnceLock::new();
    *OFF.get_or_init(|| std::env::var("CITYLIVE_NO_WEAR").map(|v| v == "1").unwrap_or(false))
}

/// The sprite rows and columns the ground floor occupies: the band a tag, a
/// shutter or a board can live in without landing on the roof.
struct Band {
    y0: i32,
    y1: i32,
    x0: i32,
    x1: i32,
    /// The first drawn row of the sprite.
    top: i32,
}

fn band_of(k: &KitSprite) -> Option<Band> {
    let mut top = -1i32;
    let mut bot = -1i32;
    for sy in 0..k.h {
        if (0..k.w).any(|sx| opaque(k.px[sy * k.w + sx])) {
            if top < 0 {
                top = sy as i32;
            }
            bot = sy as i32;
        }
    }
    if top < 0 || bot < top {
        return None;
    }
    // The ground floor: the bottom third of the drawn height, at least two rows
    // (so even a shed has a wall to paint on).
    let wall = ((bot - top + 1) as f32 * 0.34).max(2.0);
    let y0 = bot - wall as i32 + 1;
    let mut x0 = i32::MAX;
    let mut x1 = i32::MIN;
    for sy in y0..=bot {
        for sx in 0..k.w {
            if opaque(k.px[sy as usize * k.w + sx]) {
                x0 = x0.min(sx as i32);
                x1 = x1.max(sx as i32);
            }
        }
    }
    if x0 > x1 {
        return None;
    }
    Some(Band { y0, y1: bot, x0, x1, top })
}

/// Overlay everything this lot wears onto the frame, in sprite space, with the
/// sprite's top-left at `(x, y)`. Called right after the sprite's own blit.
pub fn over(fb: &mut Framebuffer, o: &CityObj, k: &KitSprite, x: i32, y: i32) {
    let w = o.wear;
    if w.is_plain() || off() {
        return;
    }
    // Everything below needs to know where the walls are: one scan, shared.
    let Some(b) = band_of(k) else { return };
    // 1. The weather on every wall and roof.
    grime(fb, o, k, x, y, &b);
    // 2. Everything the street did.
    if w.boarded {
        boards(fb, k, x, y);
    }
    if w.shut {
        shutters(fb, k, x, y, &b, w.art_seed);
    }
    if w.art == 1 {
        tags(fb, k, x, y, &b, w.art_seed);
    } else if w.art >= 2 {
        mural(fb, k, x, y, &b, w.art_seed);
    }
    if w.scaffold {
        scaffold(fb, k, x, y, &b, w.art_seed);
    }
}

/// Darken and streak the walls, and grow moss on a wet land's roof.
fn grime(fb: &mut Framebuffer, o: &CityObj, k: &KitSprite, x: i32, y: i32, band: &Band) {
    let w = o.wear;
    if w.grime == 0 && !w.wet {
        return;
    }
    // The steps are deliberately uneven: 1 is a few years of sun, 3 is a wall the
    // town stopped painting.
    let g = [0.0f32, 0.30, 0.65, 1.0][(w.grime as usize).min(3)];
    // The roof band: the top of the silhouette, where moss and lichen grow —
    // never more than a roof's depth, so a tower's shaft stays a shaft.
    let (r0, r1) = (band.top, band.y0);
    let band_lo = r1;
    let moss_band = ((r1 - r0) as f32 * 0.45).clamp(2.0, 14.0);
    // Coherent patches, never per-pixel noise: at 1x a wall is 20-40 px tall, so
    // grime has to arrive in blocks and columns a person can read as dirt.
    const CHALK: Color = rgb(0xc4, 0xc0, 0xb4);
    for sy in 0..k.h {
        let fy = y + sy as i32;
        let srow = sy * k.w;
        for sx in 0..k.w {
            let c = k.px[srow + sx];
            if !opaque(c) || emissive(c) {
                continue;
            }
            let (sxi, syi) = (sx as i32, sy as i32);
            // A rain-wash column: two pixels wide, running from the eave down.
            let streak = hash(sxi / 2 + o.x * 7, 0) % 11 == 0 && syi > band.top + 2;
            // Damp patches: 6x5 blocks, the size a stain actually is.
            let patch = hash(sxi / 6 + o.x * 3, syi / 5 + o.y * 5) % 6 == 0;
            // Soot gathers at the base of the wall, where the street throws it.
            let near_base = syi >= band_lo - 3;
            // Sun-faded paint: a chalky band high on the wall, above the dirt.
            let wall_mid = band.top + (band_lo - band.top) / 2;
            let faded = syi > wall_mid
                && syi < band_lo - 2
                && hash(sxi / 6 + 5, syi / 7 + o.y) % 4 == 0
                && !streak;
            // Age dulls a wall before it dirties it: the paint loses its
            // saturation and a little of its brightness, keeping its own hue (a
            // cream wall goes tired, not grey). Dirt goes on top of that.
            // Weathering on a grey concrete tower: desaturation alone does
            // nothing to a neutral wall, so the dulling is a mix towards a warm
            // dirt grey plus a shade — a cream wall goes tired, a white one goes
            // dingy, red brick goes brown.
            let base = palette::shade(
                palette::mix(c, DIRT, (0.24 * g).min(0.30)),
                1.0 - 0.12 * g,
            );
            let mut out = base;
            if patch {
                out = palette::mix(out, SOOT, 0.22 * g);
            }
            if streak {
                out = palette::mix(out, SOOT, 0.36 * g);
            }
            if near_base {
                out = palette::mix(out, SOOT, 0.22 * g);
            }
            if faded {
                out = palette::mix(out, CHALK, 0.16 * g);
            }
            // Moss and lichen: patches on the roof band of a wet land, thicker
            // the older the roof.
            if w.wet && (r0..(r0 + moss_band as i32)).contains(&syi) {
                let patches = hash(sxi / 3 + o.x * 11, syi / 3 + o.y * 5);
                let p = if o.wear.grime >= 2 { 3 } else { 5 };
                if patches % p == 0 {
                    let target = if patches % 2 == 0 { MOSS } else { LICHEN };
                    out = palette::mix(out, target, (0.34 + 0.14 * g).min(0.55));
                }
            }
            let fx = x + sxi;
            if fx < 0 || fy < 0 {
                continue;
            }
            fb.put(fx, fy, out);
        }
    }
}

/// Plywood over every pane: a building waiting for its rebuild is shut tight,
/// and an unlit one (`paint_kit_pane` will not repaint a pixel that is no longer
/// its own dark glass) keeps it dark at night for free.
fn boards(fb: &mut Framebuffer, k: &KitSprite, x: i32, y: i32) {
    // The ground floor and the first floor: what a boarded-up building actually
    // has boarded. The upper windows stay glass (and dark — nobody is home).
    let Some(b) = band_of(k) else { return };
    let top = (b.y0 - (b.y1 - b.y0) / 2).max(b.top);
    for p in &k.panes {
        for &(px, py) in &p.px {
            if (py as i32) < top {
                continue;
            }
            let seam = py as i32 % 4 == 0;
            let c = if seam { PLY_GRAIN } else { PLY };
            fb.put(x + px as i32, y + py as i32, c);
        }
    }
    // A batten down the middle of the shopfront, so the boarding reads as
    // boards nailed on rather than a brown panel.
    let mx = (b.x0 + b.x1) / 2;
    for yy in top..=b.y1 {
        if k.px[(yy as usize) * k.w + mx as usize] != palette::TRANSPARENT {
            fb.put(x + mx, y + yy, PLY_GRAIN);
        }
    }
    // And one across, at the top of the boarding.
    for xx in b.x0..=b.x1 {
        if k.px[(top as usize) * k.w + xx as usize] != palette::TRANSPARENT {
            fb.put(x + xx, y + top, PLY_GRAIN);
        }
    }
}

/// A shop with its shutters down and a TO LET board over them.
fn shutters(fb: &mut Framebuffer, k: &KitSprite, x: i32, y: i32, b: &Band, seed: u32) {
    let h = b.y1 - b.y0 + 1;
    let rib = 2i32;
    for yy in b.y0..=b.y1 {
        let c = if ((yy - b.y0) / rib) % 2 == 0 { SHUTTER_A } else { SHUTTER_B };
        for xx in b.x0..=b.x1 {
            if k.px[(yy as usize) * k.w + xx as usize] == palette::TRANSPARENT {
                continue;
            }
            fb.put(x + xx, y + yy, c);
        }
    }
    // A shut box on one side: the roller housing.
    for xx in b.x0..=(b.x0 + 3).min(b.x1) {
        if k.px[(b.y0 as usize) * k.w + xx as usize] != palette::TRANSPARENT {
            fb.put(x + xx, y + b.y0, palette::shade(SHUTTER_B, 0.7));
        }
    }
    // TO LET: a small white board with a red border and a short red line of
    // invented lettering (D75: no real words), hung above the shutter.
    let bw = ((b.x1 - b.x0) / 2).clamp(5, 9);
    let bx = b.x0 + ((b.x1 - b.x0 - bw) / 2).max(0) + ((seed >> 3) as i32 % 2);
    let by = (b.y0 - 4).max(b.top);
    if h >= 3 {
        for yy in 0..4 {
            for xx in 0..bw {
                let edge = yy == 0 || yy == 3 || xx == 0 || xx == bw - 1;
                let c = if edge { rgb(0xc0, 0x38, 0x30) } else { rgb(0xf4, 0xf2, 0xea) };
                let (fx, fy) = (bx + xx, by + yy);
                if fx < 0 || fy < 0 {
                    continue;
                }
                // Only over the wall: a board does not hang in mid-air.
                let (sxx, syy) = (fx - x, fy - y);
                if sxx >= 0
                    && (sxx as usize) < k.w
                    && syy >= 0
                    && (syy as usize) < k.h
                    && k.px[(syy as usize) * k.w + sxx as usize] != palette::TRANSPARENT
                {
                    fb.put(fx, fy, c);
                }
            }
        }
        // Two dashes of red "lettering" on the white board.
        for xx in 2..bw - 2 {
            if xx % 2 == 0 {
                fb.put(x + bx + xx, y + by + 1, rgb(0xc0, 0x38, 0x30));
            }
        }
    }
}

/// Spray tags: a few short strokes along the base of the wall.
fn tags(fb: &mut Framebuffer, k: &KitSprite, x: i32, y: i32, b: &Band, seed: u32) {
    for i in 0..3i32 {
        let h = hash(seed as i32 + i * 37, b.y1 + i);
        // The stroke lives on a wall pixel: find the opaque run of the row it is
        // drawn on, so a tag never starts in the transparent margin.
        // A few rows up the wall, where the face is wide enough to spray on.
        let row = b.y1 - 4 - (h as i32 >> 9).rem_euclid(8);
        if row < b.top || row > b.y1 {
            continue;
        }
        let (mut lo, mut hi) = (i32::MAX, i32::MIN);
        for sx in b.x0..=b.x1 {
            if k.px[(row as usize) * k.w + sx as usize] != palette::TRANSPARENT {
                lo = lo.min(sx);
                hi = hi.max(sx);
            }
        }
        if lo > hi {
            continue;
        }
        let span = hi - lo + 1;
        let len = (5 + (h % 4) as i32).min(span);
        let start = lo + (h as i32 >> 5).rem_euclid((span - len).max(1));
        let col = SPRAY[(h as usize >> 3) % SPRAY.len()];
        for j in 0..len {
            let xx = start + j;
            if xx > hi {
                break;
            }
            fb.put(x + xx, y + row, col);
            // Two rows tall on every other pixel: a tag has to be a mark at 1x,
            // not a hair.
            if j % 2 == 0 && row + 1 <= b.y1
                && k.px[((row + 1) as usize) * k.w + xx as usize] != palette::TRANSPARENT
            {
                fb.put(x + xx, y + row + 1, col);
            }
        }
    }
}

/// A painted mural: an invented abstract piece, a landscape or a face, drawn on
/// the wall above the pavement and then left to weather.
fn mural(fb: &mut Framebuffer, k: &KitSprite, x: i32, y: i32, b: &Band, seed: u32) {
    let span = b.x1 - b.x0 + 1;
    // The panel takes the ground floor's front face: big enough to read at 1x,
    // which is the whole point of a mural.
    let pw = ((span * 3) / 5).clamp(8, 30);
    let ph = (b.y1 - b.y0 + 1).max(3);
    if span < 8 || ph < 3 {
        // Too small a wall for a painting: a tag is what it gets.
        return tags(fb, k, x, y, b, seed);
    }
    let px0 = b.x0 + 1 + (seed as i32 >> 4).rem_euclid((span - pw - 2).max(1));
    let py0 = b.y0;
    let py1 = b.y1 - 1;
    let bg = MURAL_BG[(seed as usize >> 2) % MURAL_BG.len()];
    let kind = seed % 3;
    let ink = |sx: i32, sy: i32| -> Color {
        let (u, v) = (sx - px0, sy - py0);
        let frame = u == 0 || u == pw - 1 || v == 0 || v == ph - 1;
        if frame {
            return rgb(0xf0, 0xe8, 0xd0);
        }
        match kind {
            // A landscape: sky, a sun, two hills.
            0 => {
                let hill = ph * 2 / 3;
                if v >= hill {
                    palette::shade(rgb(0x3e, 0x6a, 0x3a), 0.9 + 0.1 * ((u as f32 / pw as f32)))
                } else if (u - pw / 2).abs() <= 1 && (v - 2).abs() <= 1 {
                    rgb(0xf2, 0xd0, 0x5a)
                } else {
                    bg
                }
            }
            // Abstract: diagonal bands and a disc.
            1 => {
                let d = u + v * 2;
                let (cx, cy) = (pw / 2, ph / 2);
                if (u - cx).abs() + (v - cy).abs() <= ph / 4 {
                    rgb(0xf0, 0x6a, 0x3c)
                } else if d % 6 < 2 {
                    palette::shade(bg, 1.35)
                } else {
                    bg
                }
            }
            // A face: two eyes, a nose and a mouth, three pixels each.
            _ => {
                if v == 1 && (u == 2 || u == pw - 3) {
                    rgb(0x1c, 0x1c, 0x20)
                } else if v == 2 && (u == pw / 2 || u == pw / 2 - 1) {
                    rgb(0x1c, 0x1c, 0x20)
                } else if v == ph - 2 && u > 1 && u < pw - 2 && u % 2 == 0 {
                    rgb(0x1c, 0x1c, 0x20)
                } else {
                    bg
                }
            }
        }
    };
    for sy in py0..=py1 {
        for sx in px0..(px0 + pw) {
            if sy < 0 || sx < 0 {
                continue;
            }
            if sx as usize >= k.w || sy as usize >= k.h {
                continue;
            }
            if k.px[(sy as usize) * k.w + sx as usize] == palette::TRANSPARENT {
                continue;
            }
            let mut c = ink(sx, sy);
            // The painting weathers with the wall it is on.
            if b.y0 > 0 {
                let g = (1.0 - (sx - px0) as f32 / pw as f32).clamp(0.0, 1.0);
                let _ = g;
            }
            if seed % 5 == 0 && hash(sx, sy) % 7 == 0 {
                c = palette::mix(c, SOOT, 0.16);
            }
            fb.put(x + sx, y + sy, c);
        }
    }
}

/// Scaffolding over the front faces, with two of the crew at the base.
fn scaffold(fb: &mut Framebuffer, k: &KitSprite, x: i32, y: i32, b: &Band, seed: u32) {
    let top = b.y0 - ((b.y0 - b.top) / 2).max(2);
    let (x0, x1) = (b.x0, b.x1);
    let paint = |fb: &mut Framebuffer, sx: i32, sy: i32, c: Color| {
        if sx < 0 || sy < 0 || sx as usize >= k.w || sy as usize >= k.h {
            return;
        }
        if k.px[(sy as usize) * k.w + sx as usize] == palette::TRANSPARENT {
            return;
        }
        fb.put(x + sx, y + sy, c);
    };
    // Standards: a tube every fourth column, floor to eave.
    let mut sx = x0 + 1;
    while sx <= x1 {
        for sy in top..=b.y1 {
            paint(fb, sx, sy, TUBE);
        }
        sx += 4;
    }
    // Ledgers and boards: a lift every four rows, with a scaffold board on top.
    let mut sy = top + 2;
    while sy <= b.y1 - 1 {
        for xx in x0..=x1 {
            paint(fb, xx, sy, BOARD);
            paint(fb, xx, sy + 1, palette::shade(TUBE, 0.82));
        }
        sy += 5;
    }
    // One diagonal brace, the way a scaffold is actually stiffened.
    let mut d = 0;
    let mut bx = x0 + 1;
    let mut by = top + 2;
    while bx <= x1 && by <= b.y1 - 1 {
        if d % 3 == 0 {
            paint(fb, bx, by, palette::shade(TUBE, 1.12));
        }
        bx += 1;
        by += 1;
        d += 1;
    }
    // The crew: two figures in tabards under the lift, one with a brush.
    let spots = [x0 + 2 + (seed as i32 % 2), x1 - 3 - (seed as i32 % 2)];
    for (i, fx) in spots.iter().enumerate() {
        let fy = b.y1;
        let body = if i == 0 { HIVIS } else { rgb(0xe8, 0xc8, 0x3c) };
        paint(fb, *fx, fy - 3, SKIN);
        paint(fb, *fx, fy - 2, body);
        paint(fb, *fx + 1, fy - 2, body);
        paint(fb, *fx, fy - 1, BOOT);
        paint(fb, *fx + 1, fy - 1, BOOT);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::city::Wear;

    fn sprite(w: usize, h: usize) -> KitSprite {
        let mut px = vec![palette::TRANSPARENT; w * h];
        // A box: a wall from row 4 down, one span wide.
        for sy in 4..h {
            for sx in 2..w - 2 {
                px[sy * w + sx] = rgb(0x98, 0x4e, 0x3c);
            }
        }
        KitSprite {
            w,
            h,
            ox: w as i32 / 2,
            oy: h as i32,
            px: std::sync::Arc::new(px),
            ..Default::default()
        }
    }

    fn lot(kind: crate::city::ObjKind) -> CityObj {
        CityObj {
            x: 4,
            y: 4,
            w: 2,
            h: 2,
            level: 2,
            kind,
            storeys: 2,
            style: 0,
            stage: crate::city::Stage::Finished,
            seed: 9,
            front: 1,
            t: 0.0,
            kit: 1,
            wear: Wear::default(),
        }
    }

    #[test]
    fn plain_wear_draws_nothing() {
        let mut fb = Framebuffer::new(40, 40);
        let k = sprite(12, 12);
        over(&mut fb, &lot(crate::city::ObjKind::House), &k, 5, 5);
        assert!(fb.px().iter().all(|&c| c == palette::TRANSPARENT), "a new building is untouched");
    }

    #[test]
    fn grime_darkens_and_a_board_covers_the_panes() {
        let mut fb = Framebuffer::new(40, 40);
        let mut k = sprite(12, 12);
        k.panes = vec![crate::scene::KitSpritePane {
            px: vec![(3, 9), (4, 9), (3, 10), (4, 10)],
            warm: true,
        }];
        let mut o = lot(crate::city::ObjKind::Shop);
        o.wear = Wear { grime: 3, boarded: true, ..Default::default() };
        over(&mut fb, &o, &k, 5, 5);
        // The board covers the pane with timber, and the wall beside it is
        // darker than the bare brick it was.
        let pane = fb.px()[(5 + 9) * 40 + 5 + 3];
        assert!(
            palette::r(pane) > palette::g(pane) && palette::g(pane) > palette::b(pane),
            "a boarded window is timber, not glass: {pane:08x}"
        );
        assert_ne!(pane & 0x00FF_FFFF, 0x984e3c, "the pane is covered");
        let wall = fb.px()[(5 + 9) * 40 + 5 + 8];
        assert!(palette::luma(wall) < palette::luma(rgb(0x98, 0x4e, 0x3c)));
    }
}
