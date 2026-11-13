//! Procedural architectural parts, textures, windows, roofs, and details.

use render::palette::{Color, rgb};
use crate::color::{self, pal, FACE_L, FACE_R, FACE_TOP};
use crate::geom::{self, Quad, project, box_faces, box_top, fill_polygon, fill_polygon_blended};
use crate::types::{Light, Sprite};

// -----------------------------------------------------------------------------
// Procedural Wall Textures
// -----------------------------------------------------------------------------

/// Paint clapboard siding with horizontal lap shadows and subtle vertical butt joints.
pub fn paint_clapboard(
    q: &Quad,
    sprite: &mut Sprite,
    light: &Light,
    base_color: Color,
    side_shading: f32,
    age: f32,
) {
    let row_pitch = 4; // 4px per lap course at 64px scale
    let rows = q.rows().max(1);
    q.paint(sprite, &mut |u, v, x, y| {
        let lap = (v % row_pitch) == row_pitch - 1; // bottom edge shadow
        let mut factor = if lap { 0.85 } else { 1.0 };
        if (v % row_pitch) == 0 {
            factor = 1.06; // sun highlight on top edge
        }
        // Faint staggered vertical butt joints
        if u % 14 == 0 && (v % (row_pitch * 2)) < row_pitch {
            factor *= 0.92;
        }
        // Vertical light gradient (stronger contrast between top skylight and base)
        let v_rel = v as f32 / rows as f32;
        let vert_grad = 1.10 - 0.22 * v_rel;
        // Deep under-eaves AO and ground contact AO
        let top_ao = if v == 0 { 0.70 } else if v == 1 { 0.82 } else if v == 2 { 0.92 } else { 1.0 };
        let bot_ao = if v >= rows - 1 { 0.72 } else if v >= rows - 2 { 0.82 } else if v >= rows - 4 { 0.92 } else { 1.0 };

        let jit = color::jcell(x, y, 4, 0.03);
        let c = color::shade(base_color, factor * jit * vert_grad * top_ao * bot_ao);
        let weathered = color::weather(c, x, y, age);
        let out_c = color::surf(weathered, light, side_shading);
        let depth = (q.z_top as f32 + (q.z_base as f32 - q.z_top as f32) * (v as f32 / rows as f32)) as u16;
        Some((out_c, depth))
    });
}

/// Paint brick wall with mortar courses and staggered vertical joints.
pub fn paint_brick(
    q: &Quad,
    sprite: &mut Sprite,
    light: &Light,
    brick_color: Color,
    mortar_color: Color,
    side_shading: f32,
    age: f32,
) {
    let course_h = 4; // 4px brick course
    let brick_w = 7; // 7px brick length
    let rows = q.rows().max(1);
    q.paint(sprite, &mut |u, v, x, y| {
        let is_horiz_mortar = (v % course_h) == course_h - 1;
        let stagger = if (v / course_h) % 2 == 0 { 0 } else { brick_w / 2 };
        let is_vert_mortar = ((u + stagger) % brick_w) == brick_w - 1;

        // Vertical light gradient & AO (stronger depth cues)
        let v_rel = v as f32 / rows as f32;
        let vert_grad = 1.10 - 0.22 * v_rel;
        let top_ao = if v == 0 { 0.70 } else if v == 1 { 0.82 } else if v == 2 { 0.92 } else { 1.0 };
        let bot_ao = if v >= rows - 1 { 0.72 } else if v >= rows - 2 { 0.82 } else if v >= rows - 4 { 0.92 } else { 1.0 };
        let ao = vert_grad * top_ao * bot_ao;

        let raw_c = if is_horiz_mortar || is_vert_mortar {
            color::shade(mortar_color, ao)
        } else {
            let brick_id = ((u + stagger) / brick_w) ^ ((v / course_h) * 57);
            let tint = 0.93 + 0.14 * color::hash01(brick_id, 3);
            let grain = color::jcell(x, y, 3, 0.02);
            color::shade(brick_color, tint * grain * ao)
        };
        let weathered = color::weather(raw_c, x, y, age);
        let out_c = color::surf(weathered, light, side_shading);
        let depth = (q.z_top as f32 + (q.z_base as f32 - q.z_top as f32) * (v as f32 / rows as f32)) as u16;
        Some((out_c, depth))
    });
}

/// Paint smooth stucco with subtle surface variations and moisture blotches.
pub fn paint_stucco(
    q: &Quad,
    sprite: &mut Sprite,
    light: &Light,
    base_color: Color,
    side_shading: f32,
    age: f32,
) {
    let rows = q.rows().max(1);
    q.paint(sprite, &mut |_u, v, x, y| {
        let v_rel = v as f32 / rows as f32;
        let vert_grad = 1.10 - 0.22 * v_rel;
        let top_ao = if v == 0 { 0.70 } else if v == 1 { 0.82 } else if v == 2 { 0.92 } else { 1.0 };
        let bot_ao = if v >= rows - 1 { 0.72 } else if v >= rows - 2 { 0.82 } else if v >= rows - 4 { 0.92 } else { 1.0 };

        let mut factor = color::jcell(x, y, 4, 0.035) * vert_grad * top_ao * bot_ao;
        if v == 0 {
            factor *= 1.05;
        }
        let c = color::shade(base_color, factor);
        let weathered = color::weather(c, x, y, age);
        let out_c = color::surf(weathered, light, side_shading);
        let depth = (q.z_top as f32 + (q.z_base as f32 - q.z_top as f32) * (v as f32 / rows as f32)) as u16;
        Some((out_c, depth))
    });
}

/// Paint rusticated stone ashlar blocks with chamfered joints.
pub fn paint_stone_ashlar(
    q: &Quad,
    sprite: &mut Sprite,
    light: &Light,
    stone_color: Color,
    side_shading: f32,
    age: f32,
) {
    let block_h = 8;
    let block_w = 16;
    let rows = q.rows().max(1);
    q.paint(sprite, &mut |u, v, x, y| {
        let is_horiz_joint = (v % block_h) == 0;
        let stagger = if (v / block_h) % 2 == 0 { 0 } else { block_w / 2 };
        let is_vert_joint = ((u + stagger) % block_w) == 0;

        let v_rel = v as f32 / rows as f32;
        let vert_grad = 1.10 - 0.22 * v_rel;
        let top_ao = if v == 0 { 0.70 } else if v == 1 { 0.82 } else if v == 2 { 0.92 } else { 1.0 };
        let bot_ao = if v >= rows - 1 { 0.72 } else if v >= rows - 2 { 0.82 } else if v >= rows - 4 { 0.92 } else { 1.0 };

        let mut factor = vert_grad * top_ao * bot_ao;
        if is_horiz_joint || is_vert_joint {
            factor *= 0.72; // joint groove
        } else if (v % block_h) == 1 || ((u + stagger) % block_w) == 1 {
            factor *= 1.08; // stone bevel highlight
        } else {
            let block_id = ((u + stagger) / block_w) ^ ((v / block_h) * 101);
            factor *= 0.94 + 0.12 * color::hash01(block_id, 7);
            factor *= color::jcell(x, y, 4, 0.025);
        }
        let c = color::shade(stone_color, factor);
        let weathered = color::weather(c, x, y, age);
        let out_c = color::surf(weathered, light, side_shading);
        let depth = (q.z_top as f32 + (q.z_base as f32 - q.z_top as f32) * (v as f32 / rows as f32)) as u16;
        Some((out_c, depth))
    });
}

/// Paint modern glass curtain wall with aluminum mullions, floor spandrels, and diagonal reflections.
///
/// B99d: the evening is a fade. The glass keeps reflecting the sky and just
/// darkens towards the night pane (see [`Light::night`]); the lit panes grow in
/// number from none to the night share, chosen by the *same* hash so the dusk
/// set is a subset of the night set — the lights come on, they do not all change
/// over at once. Lit panes carry the emissive tag (`0xFE`), so the frame's light
/// pass dims them smoothly instead of tinting them like a wall.
pub fn paint_glass_curtain(
    q: &Quad,
    sprite: &mut Sprite,
    light: &Light,
    floor_h: i32,
    bay_w: i32,
    side_shading: f32,
    seed: u64,
) {
    paint_glass_curtain_tinted(
        q,
        sprite,
        light,
        floor_h,
        bay_w,
        side_shading,
        seed,
        pal::GLASS_BLUE_SKY,
        pal::GLASS_REFLECTIVE,
    );
}

/// As [`paint_glass_curtain`], in a design's own glass: `glass` is the pane's
/// day colour and `glass_hi` the reflective band that catches the sky (B73b's
/// generator tints the curtain wall per material: blue, green, bronze, grey).
///
/// Only the two base colours change. The floors, mullions, the lit share and
/// the hash that picks which panes are lit are the same as the untinted wall,
/// so the window-light model (B72/D78) owns every tower alike.
#[allow(clippy::too_many_arguments)]
pub fn paint_glass_curtain_tinted(
    q: &Quad,
    sprite: &mut Sprite,
    light: &Light,
    floor_h: i32,
    bay_w: i32,
    side_shading: f32,
    seed: u64,
    glass: Color,
    glass_hi: Color,
) {
    let night = light.night.clamp(0.0, 1.0);
    // B72 (D78): every pane is baked dark and exported; the renderer lights the
    // ones whose floor has people on it, so a tower's office floors come on and
    // go out one by one instead of all at dusk.
    let mut panes: Vec<(i32, i32, Vec<(i16, i16)>)> = Vec::new();
    q.paint(sprite, &mut |u, v, x, y| {
        let is_floor_spandrel = (v % floor_h) < 2; // horizontal floor divider
        let is_mullion = (u % bay_w) == 0; // vertical mullion
        let rows = q.rows().max(1);
        let depth = (q.z_top as f32 + (q.z_base as f32 - q.z_top as f32) * (v as f32 / rows as f32)) as u16;

        if is_floor_spandrel || is_mullion {
            // The frame is metal all evening: it never becomes a lit window.
            let dim = 1.0 - 0.35 * night;
            let mullion_c = if is_floor_spandrel {
                pal::METAL_STEEL_DARK
            } else {
                color::shade(pal::METAL_STEEL_DARK, 1.15)
            };
            return Some((color::surf(color::shade(mullion_c, dim), light, side_shading), depth));
        }

        // Inside a window pane
        let v_rel = v as f32 / rows as f32;
        let vert_grad = 1.14 - 0.26 * v_rel;
        let bot_ao = if v >= rows - 1 { 0.72 } else if v >= rows - 2 { 0.82 } else if v >= rows - 5 { 0.92 } else { 1.0 };
        let is_pane_reveal = (u % bay_w == 1) || (v % floor_h == 2);
        let _ = is_pane_reveal;

        let pane_x = u / bay_w;
        let pane_y = v / floor_h;
        // Remember the pane's glass pixels (the light is switched on later).
        match panes.iter_mut().find(|p| p.0 == pane_x && p.1 == pane_y) {
            Some(p) => p.2.push((x as i16, y as i16)),
            None => panes.push((pane_x, pane_y, vec![(x as i16, y as i16)])),
        }

        // Glass: the day's reflection of the sky, darkening and cooling towards
        // the night pane as the evening goes on. The diagonal "cloud bands" stay
        // visible in both, so a tower is always a sheet of glass with something
        // in it, all day and all evening.
        let diag = (x * 2 - y + (seed as i32 % 37)) / 8;
        let cloud_band = (diag % 6) == 0 || (diag % 6) == 1;
        let mut glass_c = glass;
        if cloud_band {
            glass_c = glass_hi;
        }
        // A dusk sky reflects warm and dark rather than blue and bright.
        let dusk_c = color::mix(glass_c, rgb(0xc8, 0x92, 0x76), 0.45);
        let glass_c = color::mix(glass_c, dusk_c, (night * 1.6).clamp(0.0, 1.0));
        let glass_c = color::mix(glass_c, color::WIN_OFF_NIGHT, (night * night * 0.55).clamp(0.0, 1.0));
        let reveal_factor = if is_pane_reveal { 0.78 } else { 1.0 };
        let grad = vert_grad * bot_ao * reveal_factor;
        Some((color::surf(color::shade(glass_c, grad), light, side_shading), depth))
    });
    // One switchable pane per glass cell of the curtain wall.
    for (px, py, px_list) in panes {
        let hash = color::hash_seed(seed, (px * 79 + py * 113) as u32);
        sprite.win.push(crate::types::WinPane {
            px: px_list,
            warm: (hash % 3) != 0,
        });
    }
}

/// Paint corrugated vertical metal siding for industrial warehouses.
pub fn paint_corrugated(
    q: &Quad,
    sprite: &mut Sprite,
    light: &Light,
    base_color: Color,
    side_shading: f32,
    age: f32,
) {
    let rib_w = 3;
    let rows = q.rows().max(1);
    q.paint(sprite, &mut |u, v, x, y| {
        let rib = u % rib_w;
        let v_rel = v as f32 / rows as f32;
        let vert_grad = 1.04 - 0.12 * v_rel;
        let top_ao = if v == 0 { 0.80 } else if v == 1 { 0.90 } else { 1.0 };
        let bot_ao = if v >= rows - 1 { 0.82 } else if v >= rows - 2 { 0.91 } else { 1.0 };

        let factor = match rib {
            0 => 1.12, // sun catching rib peak
            1 => 0.88, // rib valley shadow
            _ => 1.00,
        } * vert_grad * top_ao * bot_ao;
        let c = color::shade(base_color, factor);
        let weathered = color::weather(c, x, y, age);
        let out_c = color::surf(weathered, light, side_shading);
        let depth = (q.z_top as f32 + (q.z_base as f32 - q.z_top as f32) * (v as f32 / rows as f32)) as u16;
        Some((out_c, depth))
    });
}

// -----------------------------------------------------------------------------
// Windows & Doors
// -----------------------------------------------------------------------------

/// Tag a painted pixel as emitted light: the frame's light pass tints a wall but
/// never a lamp, and it fades a lamp in and out with the evening (B99d). The
/// alpha is the renderer's flag, not opacity.
#[inline]
pub fn emissive(c: Color) -> Color {
    render::palette::with_alpha(c, 0xFE)
}

#[derive(Clone, Copy, Debug)]
pub struct WinOpts {    pub w: i32,
    pub h: i32,
    pub frame: Color,
    pub sill: bool,
    pub lintel: bool,
    pub shutters: bool,
    pub shutter_c: Color,
    pub seed: u64,
    pub lit_p: f32,
}

impl Default for WinOpts {
    fn default() -> Self {
        Self {
            w: 8,
            h: 12,
            frame: pal::TRIM_WHITE,
            sill: true,
            lintel: true,
            shutters: false,
            shutter_c: pal::SHUTTER_GREEN,
            seed: 0,
            lit_p: 0.65,
        }
    }
}

pub fn draw_window(
    q: &Quad,
    sprite: &mut Sprite,
    light: &Light,
    upx: i32,
    vpx: i32,
    o: WinOpts,
    z_height: i32,
) {
    // B99d: a window that does not fit its wall is not drawn. Sub-quads
    // extrapolate outside their wall when the caller's offset runs past the end
    // (the last row of a building's windows, a sill below the plinth), which used
    // to paint white trim and glass across the roof and the ground beyond it.
    if upx < 1 || vpx < 2 || upx + o.w > q.top_len() || vpx + o.h + 2 > q.rows() {
        return;
    }
    let rows = q.rows() as f32;
    let wq = q.sub_px(upx, vpx, upx + o.w, vpx + o.h, rows);
    let frame_c = color::surf(o.frame, light, 1.0);
    let shutter_c = color::surf(o.shutter_c, light, 0.90);

    // B72 (D78): the window is baked **dark**. Whether it is lit is nobody's
    // business at sprite time any more: it follows the people living in the
    // building, so the renderer paints it (and switches it) from the sim's own
    // lights. The pane's rect is exported with the sprite for that.
    let night = light.night.clamp(0.0, 1.0);
    let is_lit = false;
    let warm = (color::hash_seed(o.seed, upx as u32) % 3) != 0;

    let glass_lit_c = color::mix(color::WIN_OFF_DAY, color::WIN_OFF_NIGHT, night);
    let mut pane_px: Vec<(i16, i16)> = Vec::new();

    wq.paint(sprite, &mut |u, v, x, y| {
        // Shutters on left/right edges
        if o.shutters && (u == 0 || u == 1 || u == o.w - 1 || u == o.w - 2) {
            let louver = (v % 2) == 0;
            let c = if louver { shutter_c } else { color::shade(shutter_c, 0.85) };
            return Some((c, z_height as u16));
        }

        let inner_u = if o.shutters { u - 2 } else { u };
        let inner_w = if o.shutters { o.w - 4 } else { o.w };

        // Outer frame
        let is_frame = inner_u <= 0 || inner_u >= inner_w - 1 || v <= 0 || v >= o.h - 1;
        // Muntins (sash bars): 1px vertical bar at center, 1px horizontal bar at center
        let is_muntin = inner_u == inner_w / 2 || v == o.h / 2;
        // 1-px reveal drop shadow along top and left inside frame (recessed window opening depth cue)
        let is_reveal_shadow = (inner_u == 1 || v == 1) && !is_lit;

        if is_frame || is_muntin {
            Some((frame_c, z_height as u16))
        } else if is_reveal_shadow {
            let shadow_c = color::surf(rgb(0x18, 0x20, 0x2c), light, 1.0);
            Some((shadow_c, z_height as u16))
        } else {
            // B72: this pixel is the window's glass — remember it, so the
            // light can be switched on later.
            pane_px.push((x as i16, y as i16));
            if night < 1.0 {
                // Daytime glint
                let is_glint = (inner_u + v) == 5 || (inner_u + v) == 6;
                let c = if is_glint && night < 0.5 { pal::GLASS_REFLECTIVE } else { glass_lit_c };
                return Some((color::surf(c, light, 1.0), z_height as u16));
            }
            Some((glass_lit_c, z_height as u16))
        }
    });
    // B72 round 2: the glass itself. The per-pixel pass above leaves only a
    // sliver of glass (its `u` runs at two steps a pixel, so most of a window
    // reads as frame), and a *shuttered* window had none at all. The body is
    // drawn here in pixel space instead: inside the frame, between the shutters
    // when there are shutters, with a 1-px muntin across the middle. It is dark
    // by day and at night, and its pixels are the switchable pane.
    // Two pixels of shutter on each side leave a strip of glass between them —
    // the catalogue's house windows are six pixels wide, so a deeper inset left
    // no glass at all (which is why half the town could never light).
    let inset = if o.shutters { 2 } else { 1 };
    let gx0 = upx + inset;
    let gx1 = upx + o.w - inset;
    let gy0 = vpx + 2;
    let gy1 = vpx + o.h - 1;
    if gx1 - gx0 >= 1 && gy1 - gy0 >= 2 {
        let gq = q.sub_px(gx0, gy0, gx1, gy1, rows);
        let gh = (gy1 - gy0) as i32;
        let gw = (gx1 - gx0) as i32;
        gq.paint(sprite, &mut |_u, v, x, y| {
            // The sash bar across the middle keeps it reading as a window at 1x.
            if v == gh / 2 {
                return Some((frame_c, z_height as u16));
            }
            // A dark pane, deepening through the evening (glass reflects the sky
            // by day and cools into the night pane after dusk).
            let t = v as f32 / gh as f32;
            let day = if (v % 3) == 1 { pal::GLASS_REFLECTIVE } else { color::WIN_OFF_DAY };
            let c = color::mix(day, color::WIN_OFF_NIGHT, night * night);
            // Recess shadow at the top of the pane.
            let c = if v == 0 { color::shade(c, 0.78) } else { c };
            let _ = (gw, t);
            Some((color::surf(c, light, 1.0), z_height as u16))
        });
        // Every pixel of the body (not the sash bar) is the pane.
        gq.paint(sprite, &mut |_u, v, x, y| {
            if v != gh / 2 {
                pane_px.push((x as i16, y as i16));
            }
            None
        });
    }
    if !pane_px.is_empty() {
        sprite.win.push(crate::types::WinPane { px: pane_px, warm });
    }

    // Projecting stone sill with highlight and drop shadow
    if o.sill && vpx + o.h < q.rows() {
        let sill_q = q.sub_px(upx - 1, vpx + o.h, upx + o.w + 1, vpx + o.h + 2, rows);
        // B76 r3: a sill's top edge holds a 1-px line of snow.
        sprite.on_surface(render::snow::LEDGE);
        sill_q.paint(sprite, &mut |_u, v, _x, _y| {
            (v == 0).then_some((color::surf(color::shade(pal::TRIM_WHITE, 1.15), light, FACE_TOP), z_height as u16))
        });
        sprite.on_surface(render::snow::NONE);
        sill_q.paint(sprite, &mut |_u, v, _x, _y| {
            (v != 0).then_some((color::surf(color::shade(pal::TRIM_WHITE, 0.76), light, FACE_TOP), z_height as u16))
        });
    }
    // Decorative lintel above window
    if o.lintel && vpx >= 2 {
        let lintel_q = q.sub_px(upx - 1, vpx - 2, upx + o.w + 1, vpx, rows);
        lintel_q.fill(sprite, color::surf(pal::TRIM_WHITE, light, 1.0));
    }
}

/// Storefront display window on ground floor.
pub fn draw_storefront(
    q: &Quad,
    sprite: &mut Sprite,
    light: &Light,
    upx: i32,
    vpx: i32,
    w: i32,
    h: i32,
    frame_c: Color,
    base_z: i32,
    seed: u64,
) {
    let rows = q.rows() as f32;
    let sq = q.sub_px(upx, vpx, upx + w, vpx + h, rows);
    let surf_frame = color::surf(frame_c, light, 1.0);

    let night = light.night.clamp(0.0, 1.0);
    let lit = if night > 0.0 {
        ((color::hash_seed(seed, 101) % 100) as f32) < 80.0 * night // shops come on in the evening
    } else {
        false
    };

    sq.paint(sprite, &mut |u, v, _x, _y| {
        // Kickplate at bottom (3px)
        if v >= h - 3 {
            return Some((color::surf(frame_c, light, 0.8), base_z as u16));
        }
        // Outer frame (1px)
        if u == 0 || u == w - 1 || v == 0 {
            return Some((surf_frame, (base_z + h - v) as u16));
        }
        // Central brass mullion
        if u == w / 2 {
            return Some((color::surf(pal::METAL_CHAMPAGNE, light, 1.0), (base_z + h - v) as u16));
        }
        if lit {
            // Warm illuminated shop interior
            let warm = color::WIN_WARM;
            Some((emissive(color::shade(warm, 0.60 + 0.35 * night)), (base_z + h - v) as u16))
        } else {
            // Day glass, darkening into the night pane across the evening.
            let glint = (u + v) % 8 == 2;
            let day_c = if glint { pal::GLASS_REFLECTIVE } else { pal::GLASS_DARK };
            let c = color::mix(day_c, color::WIN_OFF_NIGHT, night * night);
            Some((color::surf(c, light, 1.0), (base_z + h - v) as u16))
        }
    });
}

/// Entrance door (wood paneled or commercial glass).
pub fn draw_door(
    q: &Quad,
    sprite: &mut Sprite,
    light: &Light,
    upx: i32,
    vpx: i32,
    w: i32,
    h: i32,
    door_c: Color,
    frame_c: Color,
    base_z: i32,
) {
    let rows = q.rows() as f32;
    let dq = q.sub_px(upx, vpx, upx + w, vpx + h, rows);
    let surf_frame = color::surf(frame_c, light, 1.0);
    let surf_door = color::surf(door_c, light, 0.95);

    dq.paint(sprite, &mut |u, v, _x, _y| {
        // Frame
        if u == 0 || u == w - 1 || v == 0 {
            return Some((surf_frame, (base_z + h - v) as u16));
        }
        // Brass knob
        if u == w - 3 && v == h / 2 {
            return Some((pal::METAL_CHAMPAGNE, (base_z + h - v) as u16));
        }
        // Door panels
        let is_panel_border = (u == 2 || u == w - 3) && (v > 2 && v < h - 2);
        if is_panel_border {
            Some((color::shade(surf_door, 0.8), (base_z + h - v) as u16))
        } else {
            Some((surf_door, (base_z + h - v) as u16))
        }
    });
}

// -----------------------------------------------------------------------------
// Roofs
// -----------------------------------------------------------------------------

/// Pitched gable roof (oriented along tile X axis or Y axis).
pub fn roof_gable(
    sprite: &mut Sprite,
    light: &Light,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    z_base: i32,
    rise: i32,
    roof_color: Color,
    ox: f32,
    oy: f32,
    age: f32,
) {
    let my = (y0 + y1) / 2.0;
    sprite.roof = Some(render::scene::RoofInfo { x0, y0, x1, y1, eave: z_base as f32, ridge: (z_base + rise) as f32, form: 1 });
    // B76 r3: the slopes hold snow by their own pitch, the ridge keeps its line.
    let st = render::snow::slope_tag(rise as f32, 0.5 * (x1 - x0).min(y1 - y0).max(0.2) * 32.0);
    sprite.on_surface(st);
    // Left roof slope (faces front-left, catches sun)
    let slope_l = [
        project(x0, y0, z_base as f32, ox, oy),
        project(x1, y0, z_base as f32, ox, oy),
        project(x1, my, (z_base + rise) as f32, ox, oy),
        project(x0, my, (z_base + rise) as f32, ox, oy),
    ];
    let col_l = color::surf(color::weather(roof_color, ox as i32, oy as i32, age), light, FACE_TOP);
    fill_polygon(sprite, &slope_l, col_l, (z_base + rise) as u16);

    // Right roof slope (faces front-right, shaded)
    let slope_r = [
        project(x0, my, (z_base + rise) as f32, ox, oy),
        project(x1, my, (z_base + rise) as f32, ox, oy),
        project(x1, y1, z_base as f32, ox, oy),
        project(x0, y1, z_base as f32, ox, oy),
    ];
    let col_r = color::surf(color::weather(color::shade(roof_color, 0.78), ox as i32, oy as i32, age), light, FACE_R);
    fill_polygon(sprite, &slope_r, col_r, (z_base + rise) as u16);

    // Ridge cap line
    let (ax, ay) = project(x0, my, (z_base + rise) as f32, ox, oy);
    let (bx, by) = project(x1, my, (z_base + rise) as f32, ox, oy);
    let ridge_c = color::surf(color::shade(roof_color, 1.15), light, FACE_TOP);
    sprite.on_surface(render::snow::RIDGE);
    geom::draw_line(sprite, ax.round() as i32, ay.round() as i32, bx.round() as i32, by.round() as i32, ridge_c, (z_base + rise) as u16);
    sprite.on_surface(render::snow::NONE);
}

/// 4-sided hip roof with terracotta barrel tiles.
pub fn roof_hip_tile(
    sprite: &mut Sprite,
    light: &Light,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    z_base: i32,
    rise: i32,
    tile_color: Color,
    ox: f32,
    oy: f32,
    age: f32,
) {
    let mx = (x0 + x1) / 2.0;
    let my = (y0 + y1) / 2.0;
    sprite.roof = Some(render::scene::RoofInfo { x0, y0, x1, y1, eave: z_base as f32, ridge: (z_base + rise) as f32, form: 2 });
    let dx = (x1 - x0) * 0.25;
    let _dy = (y1 - y0) * 0.25;
    let st = render::snow::slope_tag(rise as f32, 0.5 * (x1 - x0).min(y1 - y0).max(0.2) * 32.0);
    sprite.on_surface(st);

    // Ridge endpoints
    let r0 = project(mx - dx, my, (z_base + rise) as f32, ox, oy);
    let r1 = project(mx + dx, my, (z_base + rise) as f32, ox, oy);

    let c_top_left = project(x0, y0, z_base as f32, ox, oy);
    let c_top_right = project(x1, y0, z_base as f32, ox, oy);
    let c_bot_right = project(x1, y1, z_base as f32, ox, oy);
    let c_bot_left = project(x0, y1, z_base as f32, ox, oy);

    // Left slope
    let poly_l = [c_top_left, c_top_right, r1, r0];
    let col_l = color::surf(color::weather(tile_color, ox as i32, oy as i32, age), light, FACE_TOP);
    fill_polygon(sprite, &poly_l, col_l, (z_base + rise) as u16);

    // Right slope
    let poly_r = [c_top_right, c_bot_right, r1];
    let col_r = color::surf(color::weather(color::shade(tile_color, 0.76), ox as i32, oy as i32, age), light, FACE_R);
    fill_polygon(sprite, &poly_r, col_r, (z_base + rise) as u16);

    // Front slope
    let poly_f = [r0, r1, c_bot_right, c_bot_left];
    let col_f = color::surf(color::weather(color::shade(tile_color, 0.88), ox as i32, oy as i32, age), light, 0.90);
    fill_polygon(sprite, &poly_f, col_f, (z_base + rise) as u16);

    // Ridge highlights
    let ridge_c = color::surf(color::shade(tile_color, 1.20), light, FACE_TOP);
    sprite.on_surface(render::snow::RIDGE);
    geom::draw_line(sprite, r0.0.round() as i32, r0.1.round() as i32, r1.0.round() as i32, r1.1.round() as i32, ridge_c, (z_base + rise) as u16);
    sprite.on_surface(render::snow::NONE);
}

/// French mansard roof with steep lower slope, slate courses, and dormer windows.
pub fn roof_mansard(
    sprite: &mut Sprite,
    light: &Light,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    z_base: i32,
    mansard_h: i32,
    slate_color: Color,
    ox: f32,
    oy: f32,
    age: f32,
) {
    let inset = 0.22;
    let ix0 = x0 + inset;
    let iy0 = y0 + inset;
    let ix1 = x1 - inset;
    let iy1 = y1 - inset;
    let z_deck = z_base + mansard_h;
    sprite.roof = Some(render::scene::RoofInfo { x0, y0, x1, y1, eave: z_base as f32, ridge: z_deck as f32, form: 3 });
    // A mansard's lower faces are steep: they keep only a thin, broken load.
    sprite.on_surface(if mansard_h * 2 > (x1 - x0).max(y1 - y0) as i32 * 32 / 2 {
        render::snow::SLOPE_STEEP
    } else {
        render::snow::SLOPE_MEDIUM
    });

    // Steep lower faces (properly aligned along y=y1 front-left and x=x1 front-right)
    let lf = Quad::new(
        project(ix0, iy1, z_deck as f32, ox, oy),
        project(ix1, iy1, z_deck as f32, ox, oy),
        project(x1, y1, z_base as f32, ox, oy),
        project(x0, y1, z_base as f32, ox, oy),
        z_base,
        z_deck,
    );
    let rf = Quad::new(
        project(ix1, iy0, z_deck as f32, ox, oy),
        project(ix1, iy1, z_deck as f32, ox, oy),
        project(x1, y1, z_base as f32, ox, oy),
        project(x1, y0, z_base as f32, ox, oy),
        z_base,
        z_deck,
    );

    let slate_course_h = 3;
    let col_l = color::surf(color::weather(slate_color, ox as i32, oy as i32, age), light, FACE_TOP);
    let col_r = color::surf(color::weather(color::shade(slate_color, 0.82), ox as i32, oy as i32, age), light, FACE_R);

    lf.paint(sprite, &mut |_u, v, _x, _y| {
        let is_lap = (v % slate_course_h) == 0;
        let c = if is_lap { color::shade(col_l, 0.85) } else { col_l };
        Some((c, (z_deck - v) as u16))
    });
    rf.paint(sprite, &mut |_u, v, _x, _y| {
        let is_lap = (v % slate_course_h) == 0;
        let c = if is_lap { color::shade(col_r, 0.85) } else { col_r };
        Some((c, (z_deck - v) as u16))
    });

    // Flat roof deck on top
    let deck = box_top(ix0, iy0, ix1, iy1, z_deck, ox, oy);
    sprite.on_surface(render::snow::FLAT);
    deck.fill(sprite, color::surf(pal::ROOF_GRAVEL, light, FACE_TOP));

    // Decorative iron cresting along roof deck edge
    let (d0, d1) = (project(ix0, iy1, z_deck as f32, ox, oy), project(ix1, iy1, z_deck as f32, ox, oy));
    let crest_c = color::surf(pal::WROUGHT_IRON, light, 1.0);
    sprite.on_surface(render::snow::LEDGE);
    geom::draw_line(sprite, d0.0.round() as i32, d0.1.round() as i32 - 1, d1.0.round() as i32, d1.1.round() as i32 - 1, crest_c, z_deck as u16);
    sprite.on_surface(render::snow::NONE);

    // Stone dormer window with pediment set into the right mansard slope
    let dormer_y = (y0 + y1) / 2.0;
    sprite.on_surface(render::snow::NONE);
    let (dw_lf, dw_rf) = box_faces(x1 - 0.20, dormer_y - 0.15, x1 + 0.05, dormer_y + 0.15, z_base + 3, z_base + mansard_h - 2, ox, oy);
    dw_lf.fill(sprite, color::surf(pal::STONE_LIMESTONE, light, FACE_L));
    dw_rf.fill(sprite, color::surf(pal::STONE_LIMESTONE, light, FACE_R));
    let wo = WinOpts {
        w: 5,
        h: 7,
        frame: pal::TRIM_WHITE,
        sill: true,
        lintel: true,
        shutters: false,
        shutter_c: pal::SHUTTER_BLACK,
        seed: 42,
        lit_p: 0.70,
    };
    draw_window(&dw_rf, sprite, light, 2, 2, wo, z_base + mansard_h - 2);
}

/// Draw a triangular gable end wall with sealed seam to avoid rasterizer subpixel gaps.
pub fn draw_gable_end(
    sprite: &mut Sprite,
    light: &Light,
    p0: (f32, f32),
    p1: (f32, f32),
    apex: (f32, f32),
    color: Color,
    side_shading: f32,
    z_base: i32,
    z_apex: i32,
) {
    let pts = [p0, p1, apex];
    let c = color::surf(color, light, side_shading);
    geom::fill_polygon(sprite, &pts, c, z_apex as u16);
    geom::draw_line(sprite, p0.0.round() as i32, p0.1.round() as i32, p1.0.round() as i32, p1.1.round() as i32, c, z_base as u16);
}

/// Draw a gabled roof dormer window.
pub fn draw_dormer(
    sprite: &mut Sprite,
    light: &Light,
    x: f32,
    y: f32,
    z: i32,
    h: i32,
    ox: f32,
    oy: f32,
) {
    let (dw, dd) = (0.30, 0.28);
    let (lf, rf) = box_faces(x, y, x + dw, y + dd, z, z + h, ox, oy);
    lf.fill(sprite, color::surf(pal::TRIM_WHITE, light, FACE_L));
    rf.fill(sprite, color::surf(pal::TRIM_WHITE, light, FACE_R));
    let wo = WinOpts {
        w: 5,
        h: 6,
        frame: pal::TRIM_WHITE,
        sill: true,
        lintel: true,
        shutters: false,
        shutter_c: pal::SHUTTER_BLACK,
        seed: 42,
        lit_p: 0.70,
    };
    draw_window(&rf, sprite, light, 2, 2, wo, z + h);
    let mx = x + dw / 2.0;
    let ped = [
        project(x, y + dd, (z + h) as f32, ox, oy),
        project(x + dw, y + dd, (z + h) as f32, ox, oy),
        project(mx, y + dd, (z + h + 5) as f32, ox, oy),
    ];
    geom::fill_polygon(sprite, &ped, color::surf(pal::TRIM_WHITE, light, FACE_R), (z + h + 5) as u16);
}

/// Draw a solid corner seam line between lf and rf at (x1, y1) from z0 to z1.
pub fn corner_seam(sprite: &mut Sprite, light: &Light, x1: f32, y1: f32, z0: i32, z1: i32, c: Color, ox: f32, oy: f32) {
    let (p0_x, p0_y) = project(x1, y1, z0 as f32, ox, oy);
    let (p1_x, p1_y) = project(x1, y1, z1 as f32, ox, oy);
    let seam_c = color::surf(c, light, 0.92);
    geom::draw_line(sprite, p0_x.round() as i32, p0_y.round() as i32, p1_x.round() as i32, p1_y.round() as i32, seam_c, z1 as u16);
}

/// Flat roof with raised parapet walls and gravel/tar texture.
pub fn roof_flat(
    sprite: &mut Sprite,
    light: &Light,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    z: i32,
    parapet_h: i32,
    coping_c: Color,
    ox: f32,
    oy: f32,
    age: f32,
) {
    // Flat gravel roof floor
    let top = box_top(x0, y0, x1, y1, z, ox, oy);
    // B76 r3: the deck is a horizontal surface, so snow lies on it evenly; the
    // parapet is a vertical one, so the load drifts against it instead.
    sprite.on_surface(render::snow::FLAT);
    top.paint(sprite, &mut |_u, _v, x, y| {
        let grain = color::jcell(x, y, 2, 0.04);
        let weathered = color::weather(color::shade(pal::ROOF_GRAVEL, grain), x, y, age);
        Some((color::surf(weathered, light, FACE_TOP), z as u16))
    });

    // Parapet front walls
    sprite.on_surface(render::snow::EQUIP_SIDE);
    let (lf, rf) = box_faces(x0, y0, x1, y1, z, z + parapet_h, ox, oy);
    lf.fill(sprite, color::surf(coping_c, light, FACE_L));
    rf.fill(sprite, color::surf(color::shade(coping_c, 0.85), light, FACE_R));
    corner_seam(sprite, light, x1, y1, z, z + parapet_h, coping_c, ox, oy);

    // Stone coping on top of front parapet walls
    let (p_lf_start, p_corner, p_rf_start) = (
        project(x0, y1, (z + parapet_h) as f32, ox, oy),
        project(x1, y1, (z + parapet_h) as f32, ox, oy),
        project(x1, y0, (z + parapet_h) as f32, ox, oy),
    );
    let coping_top = color::surf(color::shade(coping_c, 1.10), light, FACE_TOP);
    // The coping is the lip the snow hangs over.
    sprite.on_surface(render::snow::LEDGE);
    geom::draw_line(sprite, p_lf_start.0.round() as i32, p_lf_start.1.round() as i32, p_corner.0.round() as i32, p_corner.1.round() as i32, coping_top, (z + parapet_h) as u16);
    geom::draw_line(sprite, p_corner.0.round() as i32, p_corner.1.round() as i32, p_rf_start.0.round() as i32, p_rf_start.1.round() as i32, coping_top, (z + parapet_h) as u16);
    sprite.on_surface(render::snow::NONE);
}

// -----------------------------------------------------------------------------
// Details & Props
// -----------------------------------------------------------------------------

/// Wrought-iron balcony with decorative balusters and red blooming geraniums.
pub fn draw_balcony(
    q: &Quad,
    sprite: &mut Sprite,
    light: &Light,
    upx: i32,
    vpx: i32,
    w: i32,
    h: i32,
    z_height: i32,
) {
    let rows = q.rows() as f32;
    let bq = q.sub_px(upx, vpx, upx + w, vpx + h, rows);
    let iron_c = color::surf(pal::WROUGHT_IRON, light, 1.0);
    let flower_c = color::surf(pal::FLOWER_RED, light, 1.0);
    let leaf_c = color::surf(pal::FOLIAGE_GREEN, light, 1.0);

    // B76 r3: the handrail is a ledge, the balusters and the box are not.
    sprite.on_surface(render::snow::LEDGE);
    bq.paint(sprite, &mut |_u, v, _x, _y| {
        (v == 0).then_some((iron_c, (z_height + h) as u16))
    });
    sprite.on_surface(render::snow::NONE);
    bq.paint(sprite, &mut |u, v, _x, _y| {
        // Balcony bottom floor slab
        if v == h - 1 {
            return Some((iron_c, z_height as u16));
        }
        // Top handrail (already drawn as the ledge above)
        if v == 0 {
            return None;
        }
        // Vertical balusters every 2px
        if u % 2 == 0 {
            return Some((iron_c, (z_height + h - v) as u16));
        }
        // Blooming flower box in middle row
        if v == h - 2 || v == h - 3 {
            let has_flower = (u % 3) == 1;
            let c = if has_flower { flower_c } else { leaf_c };
            return Some((c, (z_height + h - v) as u16));
        }
        None
    });
}

/// Striped canvas awning with scalloped hem.
pub fn draw_awning(
    q: &Quad,
    sprite: &mut Sprite,
    light: &Light,
    upx: i32,
    vpx: i32,
    w: i32,
    h: i32,
    stripe_a: Color,
    stripe_b: Color,
    z_height: i32,
) {
    let rows = q.rows() as f32;
    let aq = q.sub_px(upx, vpx, upx + w, vpx + h, rows);

    // B76 r3: an awning's top edge catches the snow first, like a sill.
    sprite.on_surface(render::snow::LEDGE);
    aq.paint(sprite, &mut |u, v, _x, _y| {
        if v != 0 {
            return None;
        }
        let is_stripe_a = (u / 3) % 2 == 0;
        let c = if is_stripe_a { stripe_a } else { stripe_b };
        Some((color::surf(c, light, 1.0), z_height as u16))
    });
    sprite.on_surface(render::snow::NONE);
    aq.paint(sprite, &mut |u, v, _x, _y| {
        if v == 0 {
            return None;
        }
        // Scalloped bottom hem
        if v == h - 1 && (u % 3 == 0) {
            return None; // scallop cutout
        }
        // Striped pattern every 3px
        let is_stripe_a = (u / 3) % 2 == 0;
        let c = if is_stripe_a { stripe_a } else { stripe_b };
        // Shading on lower valance
        let factor = if v >= h - 2 { 0.85 } else { 1.0 };
        Some((color::surf(color::shade(c, factor), light, 1.0), z_height as u16))
    });
}

/// Rooftop HVAC chiller unit with compressor fans and ducting.
pub fn rooftop_hvac(
    sprite: &mut Sprite,
    light: &Light,
    x: f32,
    y: f32,
    z: i32,
    ox: f32,
    oy: f32,
) {
    let (w, d, h) = (0.45, 0.40, 8);
    let (lf, rf) = box_faces(x, y, x + w, y + d, z, z + h, ox, oy);
    let metal_c = color::surf(pal::CONCRETE_GREY, light, FACE_L);
    // B76 r3: a chiller's casing is vertical (it pokes through the load); only
    // its top face holds snow.
    sprite.on_surface(render::snow::EQUIP_SIDE);
    lf.fill(sprite, metal_c);
    rf.fill(sprite, color::shade(metal_c, 0.8));

    let top = box_top(x, y, x + w, y + d, z + h, ox, oy);
    sprite.on_surface(render::snow::EQUIP_TOP);
    top.paint(sprite, &mut |u, _v, _x, _y| {
        // Circular fan grille in center
        let is_fan = (u == 2 || u == 3) && true;
        let c = if is_fan { pal::METAL_STEEL_DARK } else { pal::CONCRETE_LIGHT };
        Some((color::surf(c, light, FACE_TOP), (z + h) as u16))
    });
}

/// Rooftop elevator / stair penthouse.
pub fn elevator_penthouse(
    sprite: &mut Sprite,
    light: &Light,
    x: f32,
    y: f32,
    w: f32,
    d: f32,
    z: i32,
    h: i32,
    ox: f32,
    oy: f32,
) {
    let (lf, rf) = box_faces(x, y, x + w, y + d, z, z + h, ox, oy);
    let wall_c = color::surf(pal::CONCRETE_GREY, light, FACE_L);
    sprite.on_surface(render::snow::EQUIP_SIDE);
    lf.fill(sprite, wall_c);
    rf.fill(sprite, color::shade(wall_c, 0.82));

    let top = box_top(x, y, x + w, y + d, z + h, ox, oy);
    sprite.on_surface(render::snow::EQUIP_TOP);
    top.fill(sprite, color::surf(pal::ROOF_GRAVEL, light, FACE_TOP));
    sprite.on_surface(render::snow::NONE);
}

/// Seed-driven varied rooftop elements: HVAC chillers, wooden water tank, rooftop garden, solar panels, or elevator penthouse.
pub fn draw_seed_rooftop_elements(
    sprite: &mut Sprite,
    light: &Light,
    w: f32,
    d: f32,
    z_top: i32,
    seed: u64,
    ox: f32,
    oy: f32,
) {
    let pick = (seed % 5) as u32;
    match pick {
        0 => {
            // Rooftop HVAC units
            rooftop_hvac(sprite, light, w * 0.25, d * 0.30, z_top, ox, oy);
            rooftop_hvac(sprite, light, w * 0.65, d * 0.30, z_top, ox, oy);
        }
        1 => {
            // Wooden water tank on timber/steel trestle
            let (cx, cy) = (w * 0.45, d * 0.35);
            sprite.on_surface(render::snow::EQUIP_SIDE);
            let (w_lf, w_rf) = box_faces(cx, cy, cx + 0.38, cy + 0.38, z_top + 4, z_top + 16, ox, oy);
            w_lf.fill(sprite, color::surf(pal::ROOF_CEDAR_SHAKE, light, FACE_L));
            w_rf.fill(sprite, color::surf(pal::ROOF_CEDAR_SHAKE, light, FACE_R));
            let w_top = box_top(cx, cy, cx + 0.38, cy + 0.38, z_top + 16, ox, oy);
            sprite.on_surface(render::snow::EQUIP_TOP);
            w_top.fill(sprite, color::surf(pal::ROOF_CEDAR_SHAKE, light, FACE_TOP));
        }
        2 => {
            // Landscaped rooftop garden with planter
            sprite.on_surface(render::snow::EQUIP_SIDE);
            let (p_lf, p_rf) = box_faces(w * 0.2, d * 0.2, w * 0.8, d * 0.8, z_top, z_top + 3, ox, oy);
            p_lf.fill(sprite, color::surf(pal::CONCRETE_GREY, light, FACE_L));
            p_rf.fill(sprite, color::surf(pal::CONCRETE_GREY, light, FACE_R));
            let p_top = box_top(w * 0.2, d * 0.2, w * 0.8, d * 0.8, z_top + 3, ox, oy);
            sprite.on_surface(render::snow::FOLIAGE);
            p_top.fill(sprite, color::surf(pal::FOLIAGE_GREEN, light, FACE_TOP));
        }
        3 => {
            // Solar photovoltaic panel array
            sprite.on_surface(render::snow::EQUIP_SIDE);
            let (s_lf, s_rf) = box_faces(w * 0.2, d * 0.2, w * 0.8, d * 0.7, z_top + 1, z_top + 5, ox, oy);
            s_lf.fill(sprite, color::surf(rgb(0x18, 0x2e, 0x58), light, FACE_L));
            s_rf.fill(sprite, color::surf(rgb(0x18, 0x2e, 0x58), light, FACE_R));
            let s_top = box_top(w * 0.2, d * 0.2, w * 0.8, d * 0.7, z_top + 5, ox, oy);
            // B76 r3: solar panels are dark and steep — they clear themselves, so
            // they keep a dusting rather than a slab.
            sprite.on_surface(render::snow::SLOPE_STEEP);
            s_top.fill(sprite, color::surf(rgb(0x28, 0x48, 0x88), light, FACE_TOP));
        }
        _ => {
            // Elevator penthouse + satellite antenna
            elevator_penthouse(sprite, light, w * 0.25, d * 0.35, 0.45, 0.40, z_top, 8, ox, oy);
            sprite.on_surface(render::snow::NONE);
            spire_needle(sprite, light, w * 0.75, d * 0.45, z_top, 12, ox, oy);
        }
    }
    sprite.on_surface(render::snow::NONE);
}

/// Spire needle mast with aircraft warning beacon.
pub fn spire_needle(
    sprite: &mut Sprite,
    light: &Light,
    x: f32,
    y: f32,
    z_base: i32,
    height: i32,
    ox: f32,
    oy: f32,
) {
    let (p0_x, p0_y) = project(x, y, z_base as f32, ox, oy);
    let (p1_x, p1_y) = project(x, y, (z_base + height) as f32, ox, oy);
    let mast_c = color::surf(pal::METAL_STEEL_DARK, light, 1.0);
    geom::draw_line(
        sprite,
        p0_x.round() as i32,
        p0_y.round() as i32,
        p1_x.round() as i32,
        p1_y.round() as i32,
        mast_c,
        (z_base + height) as u16,
    );

    // Flashing red warning beacon at pinnacle
    let beacon_c = color::BEACON_RED;
    let bx = p1_x.round() as i32;
    let by = p1_y.round() as i32;
    sprite.set_pixel(bx, by, beacon_c, (z_base + height) as u16);
    if light.is_night {
        // Glowing halo at night
        sprite.set_pixel(bx - 1, by, color::shade(beacon_c, 0.6), (z_base + height) as u16);
        sprite.set_pixel(bx + 1, by, color::shade(beacon_c, 0.6), (z_base + height) as u16);
        sprite.set_pixel(bx, by - 1, color::shade(beacon_c, 0.6), (z_base + height) as u16);
    }
}

/// Directional isometric cast shadow thrown on the ground.
pub fn building_shadow(
    sprite: &mut Sprite,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    z_height: i32,
    ox: f32,
    oy: f32,
) {
    let shadow_len = z_height as f32 * 0.70;
    // Ground footprint corners
    let c_top_left = project(x0, y0, 0.0, ox, oy);
    let c_top_right = project(x1, y0, 0.0, ox, oy);
    let _c_bot_right = project(x1, y1, 0.0, ox, oy);
    let c_bot_left = project(x0, y1, 0.0, ox, oy);

    // Projected shadow tips (offset towards -x screen direction)
    let s_top_right = (c_top_right.0 - shadow_len, c_top_right.1 + shadow_len * 0.25);
    let s_top_left = (c_top_left.0 - shadow_len, c_top_left.1 + shadow_len * 0.25);

    let shadow_poly = [c_top_left, c_top_right, s_top_right, s_top_left, c_bot_left];
    fill_polygon_blended(sprite, &shadow_poly, rgb(0x10, 0x14, 0x1c), 0.38);
}

// -----------------------------------------------------------------------------
// Vernacular parts (B46b, D66 item 4)
// -----------------------------------------------------------------------------

/// A flat roof with a parapet ring — the adobe / mud-brick answer to a gable.
/// The top is a few pixels below the parapet, so the roof reads as a walled
/// terrace you could stand on, which is what these buildings have.
#[allow(clippy::too_many_arguments)]
pub fn roof_flat_parapet(
    sprite: &mut Sprite,
    light: &Light,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    z: i32,
    parapet: i32,
    roof_c: Color,
    wall_c: Color,
    ox: f32,
    oy: f32,
    age: f32,
) {
    // The roof deck, slightly darker than the walls (it is in its own shade).
    let deck = box_top(x0, y0, x1, y1, z, ox, oy);
    sprite.on_surface(render::snow::FLAT);
    deck.paint(sprite, &mut |u, v, x, y| {
        let joint = (u % 9 == 0) || (v % 9 == 0);
        let c = if joint {
            color::shade(roof_c, 0.90)
        } else {
            color::shade(color::weather(roof_c, x, y, age), color::jcell(x, y, 4, 0.04))
        };
        Some((color::surf(c, light, FACE_TOP), z as u16))
    });
    // The parapet: four thin walls standing on the roof edge. Only the two that
    // face the camera are visible, plus their inner faces.
    let (pf, pr) = box_faces(
        x0 - 0.04,
        y0 - 0.04,
        x1 + 0.04,
        y1 + 0.04,
        z,
        z + parapet,
        ox,
        oy,
    );
    sprite.on_surface(render::snow::EQUIP_SIDE);
    pf.fill(sprite, color::surf(wall_c, light, FACE_L));
    pr.fill(sprite, color::surf(color::shade(wall_c, 0.88), light, FACE_R));
    // The lit top of the parapet, a 1-px coping.
    let (cf, cr) = box_faces(
        x0 - 0.04,
        y0 - 0.04,
        x1 + 0.04,
        y1 + 0.04,
        z + parapet,
        z + parapet + 1,
        ox,
        oy,
    );
    sprite.on_surface(render::snow::LEDGE);
    cf.fill(sprite, color::surf(color::shade(wall_c, 1.10), light, FACE_TOP));
    cr.fill(sprite, color::surf(wall_c, light, FACE_TOP));
    sprite.on_surface(render::snow::NONE);
}

/// Protruding round beam ends (vigas) along a wall's top, the signature of
/// adobe building: the roof poles stick out through the wall.
#[allow(clippy::too_many_arguments)]
pub fn vigas(
    sprite: &mut Sprite,
    light: &Light,
    q: &Quad,
    z: i32,
    count: i32,
    beam_c: Color,
    out_px: i32,
) {
    let rows = q.rows() as f32;
    let len = q.top_len() as f32;
    let beam = color::surf(beam_c, light, 0.95);
    let cap = color::surf(color::shade(beam_c, 0.72), light, 0.85);
    for i in 0..count {
        let u = ((i as f32 + 0.5) / count as f32 * len) as i32;
        let q1 = q.sub_px(u, 1, u + 2, 2, rows);
        // The stub: two pixels across and a few out of the wall.
        for k in 0..out_px {
            let (x, y) = (q1.tl.0 + k as f32, q1.tl.1 + k as f32 * 0.5);
            sprite.set_pixel(x.round() as i32, y.round() as i32, beam, z as u16);
        }
        sprite.set_pixel(
            (q1.tl.0 + out_px as f32).round() as i32,
            (q1.tl.1 + out_px as f32 * 0.5).round() as i32,
            cap,
            z as u16,
        );
    }
}

/// An arched door: a rectangle with a semicircular head, under a stone lintel.
#[allow(clippy::too_many_arguments)]
pub fn draw_arch_door(
    sprite: &mut Sprite,
    light: &Light,
    q: &Quad,
    upx: i32,
    vpx: i32,
    w: i32,
    h: i32,
    door_c: Color,
    frame_c: Color,
    base_z: i32,
) {
    let rows = q.rows() as f32;
    let dq = q.sub_px(upx, vpx, upx + w, vpx + h, rows);
    let frame = color::surf(frame_c, light, 1.0);
    let door = color::surf(door_c, light, 0.95);
    let r = (w as f32 / 2.0).max(1.0);
    dq.paint(sprite, &mut |u, v, _x, _y| {
        // v = 0 at the top row. The head is the top `r` rows: a half circle of
        // radius r centred on v = r, so a pixel above that is inside only if it
        // is within the circle.
        let dx = (u as f32 - w as f32 / 2.0 + 0.5) / r;
        let inside = if v as f32 >= r {
            true
        } else {
            let dy = (r - v as f32) / r;
            dx * dx + dy * dy <= 1.0
        };
        if !inside {
            return None;
        }
        if u <= 0 || u >= w - 1 || v <= 1 {
            Some((frame, (base_z + h - v) as u16))
        } else {
            Some((door, (base_z + h - v) as u16))
        }
    });
}

/// Posts under a raised floor (a stilt house over water or a wet shore).
#[allow(clippy::too_many_arguments)]
pub fn stilts(
    sprite: &mut Sprite,
    light: &Light,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    z_top: i32,
    post_c: Color,
    ox: f32,
    oy: f32,
) {
    let lit = color::surf(post_c, light, FACE_L);
    let dark = color::surf(color::shade(post_c, 0.72), light, FACE_R);
    for (fx, fy) in [
        (x0 + 0.10, y1 - 0.10),
        (x1 - 0.10, y1 - 0.10),
        (x0 + 0.10, y0 + 0.35),
        (x1 - 0.10, y0 + 0.35),
        ((x0 + x1) * 0.5, y1 - 0.10),
    ] {
        let (px, py) = project(fx, fy, z_top as f32, ox, oy);
        let bottom = project(fx, fy, 0.0, ox, oy).1;
        let h = (bottom - py).max(1.0);
        for k in 0..2 {
            geom::draw_line(
                sprite,
                px.round() as i32 + k,
                py.round() as i32,
                px.round() as i32 + k,
                bottom.round() as i32,
                if k == 0 { lit } else { dark },
                z_top as u16,
            );
        }
        // A brace between the front posts.
        if (fx - x0).abs() < 0.2 {
            let _ = h;
        }
    }
    // The floor slab's edge, so the house sits on something.
    let (fl, fr) = box_faces(x0, y0, x1, y1, z_top - 1, z_top, ox, oy);
    fl.fill(sprite, color::surf(color::shade(post_c, 0.90), light, FACE_L));
    fr.fill(sprite, color::surf(color::shade(post_c, 0.78), light, FACE_R));
}

/// A thick, ragged thatch roof: a hipped cone with a straw texture and a
/// drooping eave, palm-thatch for the early tropic village.
#[allow(clippy::too_many_arguments)]
pub fn roof_thatch(
    sprite: &mut Sprite,
    light: &Light,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    z: i32,
    rise: i32,
    thatch_c: Color,
    ox: f32,
    oy: f32,
    age: f32,
) {
    let mx = (x0 + x1) * 0.5;
    let my = (y0 + y1) * 0.5;
    sprite.roof = Some(render::scene::RoofInfo { x0, y0, x1, y1, eave: z as f32, ridge: (z + rise) as f32, form: 2 });
    // Four slopes, eave extended 0.12 tiles past the wall.
    let e = 0.12;
    let (a, b, c, d) = (x0 - e, y0 - e, x1 + e, y1 + e);
    let apex = project(mx, my, (z + rise) as f32, ox, oy);
    let p = |x: f32, y: f32| project(x, y, z as f32, ox, oy);
    sprite.on_surface(render::snow::slope_tag(rise as f32, 0.5 * (x1 - x0).min(y1 - y0).max(0.2) * 32.0));
    let slopes = [
        ([p(a, b), p(c, b), apex], 1.06),
        ([p(c, b), p(c, d), apex], 0.82),
        ([p(c, d), p(a, d), apex], 0.94),
        ([p(a, d), p(a, b), apex], 0.88),
    ];
    for (pts, shade) in slopes {
        let base = color::weather(color::shade(thatch_c, shade), ox as i32, oy as i32, age);
        let col = color::surf(base, light, 1.0);
        fill_polygon(sprite, &pts, col, (z + rise) as u16);
        // Straw lines running down the fall of the roof.
        let (x0s, y0s) = (pts[0].0.min(pts[1].0), pts[0].1.max(pts[1].1));
        let (x1s, y1s) = (pts[0].0.max(pts[1].0), pts[0].1.max(pts[1].1));
        let steps = ((x1s - x0s).abs() as i32 / 3).max(2);
        for k in 0..steps {
            let t = k as f32 / steps as f32;
            let sx = x0s + (x1s - x0s) * t;
            let sy = y0s + (y1s - y0s) * t;
            geom::draw_line(
                sprite,
                sx.round() as i32,
                sy.round() as i32,
                apex.0.round() as i32,
                apex.1.round() as i32,
                color::shade(col, if k % 2 == 0 { 0.90 } else { 1.06 }),
                (z + rise) as u16,
            );
        }
        // The ragged eave: a dark line with a few straw fringes.
        geom::draw_line(
            sprite,
            pts[0].0.round() as i32,
            pts[0].1.round() as i32,
            pts[1].0.round() as i32,
            pts[1].1.round() as i32,
            color::shade(col, 0.72),
            z as u16,
        );
    }
    // The ridge tuft.
    sprite.on_surface(render::snow::RIDGE);
    sprite.set_pixel(
        apex.0.round() as i32,
        apex.1.round() as i32,
        color::shade(color::surf(thatch_c, light, 1.0), 1.14),
        (z + rise + 1) as u16,
    );
    sprite.on_surface(render::snow::NONE);
}

/// A steep snow-laden gable: the pitched roof, then a thick white load with a
/// rounded drift on the sunlit slope and icicles under the eave.
#[allow(clippy::too_many_arguments)]
pub fn roof_snow_steep(
    sprite: &mut Sprite,
    light: &Light,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    z: i32,
    rise: i32,
    roof_c: Color,
    snow_c: Color,
    ox: f32,
    oy: f32,
    age: f32,
) {
    roof_gable(sprite, light, x0, y0, x1, y1, z, rise, roof_c, ox, oy, age);
    let my = (y0 + y1) / 2.0;
    // The load is a slab on each slope, inset a little from the eave so a dark
    // line of bare roof shows at the melt edge, and thinning towards the ridge.
    let e = 0.10;
    let p = |x: f32, y: f32, zz: i32| project(x, y, zz as f32, ox, oy);
    let zr = z + rise;
    let snow_lit = color::surf(color::shade(snow_c, 0.97), light, FACE_TOP);
    let snow_lit2 = color::surf(color::shade(snow_c, 0.90), light, FACE_TOP);
    let snow_shade = color::surf(color::shade(snow_c, 0.70), light, FACE_R);
    let sunlit = [
        p(x0 + e, y0 + e, z),
        p(x1 - e, y0 + e, z),
        p(x1 - e, my, zr),
        p(x0 + e, my, zr),
    ];
    fill_polygon(sprite, &sunlit, snow_lit, (zr + 1) as u16);
    // A soft drift line across the sunlit slope.
    let mid = project(x0 + e, y0 + e + (my - y0 - e) * 0.55, (z + (rise as f32 * 0.45) as i32) as f32, ox, oy);
    let mid2 = project(x1 - e, y0 + e + (my - y0 - e) * 0.55, (z + (rise as f32 * 0.45) as i32) as f32, ox, oy);
    geom::draw_line(
        sprite,
        mid.0.round() as i32,
        mid.1.round() as i32,
        mid2.0.round() as i32,
        mid2.1.round() as i32,
        snow_lit2,
        (zr + 1) as u16,
    );
    let shaded = [
        p(x0 + e, my, zr),
        p(x1 - e, my, zr),
        p(x1 - e, y1 - e, z),
        p(x0 + e, y1 - e, z),
    ];
    fill_polygon(sprite, &shaded, snow_shade, (zr + 1) as u16);
    // The ridge cap: a rounded white line, the place the load is deepest.
    let r0 = p(x0 + e, my, zr);
    let r1 = p(x1 - e, my, zr);
    geom::draw_line(
        sprite,
        r0.0.round() as i32,
        r0.1.round() as i32,
        r1.0.round() as i32,
        r1.1.round() as i32,
        color::shade(snow_lit, 1.06),
        (zr + 2) as u16,
    );
    // Icicles along the near eave.
    for k in 0..5 {
        let f = (k as f32 + 0.5) / 5.0;
        let q = project(x0 + (x1 - x0) * f, y1, z as f32, ox, oy);
        let len = 2 + (k % 3);
        geom::draw_line(
            sprite,
            q.0.round() as i32,
            q.1.round() as i32,
            q.0.round() as i32,
            (q.1.round() as i32) + len,
            color::surf(rgb(0xd8, 0xe8, 0xf4), light, 1.0),
            z as u16,
        );
    }
}

/// A hearth plume from a chimney top: three soft pixels that lean with the wind.
pub fn smoke_plume(sprite: &mut Sprite, light: &Light, x: i32, y: i32, z: u16) {
    let c = color::surf(rgb(0xd8, 0xd4, 0xd0), light, 1.0);
    for k in 0..3 {
        let dx = k as i32;
        let a = if k == 0 { 0.9 } else { 0.6 - k as f32 * 0.12 };
        sprite.blend_pixel(x + dx, y - k, c, z, a);
    }
}

/// Paint horizontal log courses: each log is a rounded band with a dark seam,
/// a highlight along its top and a shadow at its foot. The walls of a timber
/// cabin in a boreal village (B46b).
pub fn paint_log(
    q: &Quad,
    sprite: &mut Sprite,
    light: &Light,
    log_color: Color,
    side_shading: f32,
    age: f32,
) {
    let pitch = 4;
    let rows = q.rows().max(1);
    q.paint(sprite, &mut |u, v, x, y| {
        let row = v / pitch;
        let in_row = v % pitch;
        let seam = in_row == pitch - 1;
        let top_ao = if v == 0 { 0.74 } else if v < 2 { 0.86 } else { 1.0 };
        let bot_ao = if v >= rows - 1 { 0.74 } else if v >= rows - 3 { 0.86 } else { 1.0 };
        let this_log = color::shade(log_color, 0.94 + 0.12 * color::hash01(row, 41));
        let c = if seam {
            color::shade(this_log, 0.62)
        } else if in_row == 0 {
            color::shade(this_log, 1.10)
        } else if in_row == pitch - 2 {
            color::shade(this_log, 0.86)
        } else {
            this_log
        };
        // The log's end: a pale cut ring where the wall corner is.
        // `u` counts half-pixel steps along the face (B99): the cut ring is the last
        // 1-2 px of a log wall, not its whole second half.
        let end = u >= q.top_len() * 2 - 3;
        let c = if end {
            color::shade(rgb(0xc8, 0xa8, 0x7c), 0.9 + 0.1 * color::hash01(row + 7, 3))
        } else {
            c
        };
        let jit = color::jcell(x, y, 3, 0.03);
        let weathered = color::weather(c, x, y, age);
        Some((
            color::surf(color::shade(weathered, jit * top_ao * bot_ao), light, side_shading),
            (q.z_top as f32 + (q.z_base as f32 - q.z_top as f32) * (v as f32 / rows as f32)) as u16,
        ))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{BuildingSpec, BuildingStyle, BuildingType, BuildingFunction, ConstructionStage, Era};
    use crate::style::BiomeStyle;

    fn tower() -> BuildingSpec {
        BuildingSpec {
            building_type: Some(BuildingType::TowerGlassCurtainWall),
            function: BuildingFunction::Tower,
            landmark_kind: None,
            style: BuildingStyle::ModernGlass,
            footprint: (1, 1),
            storeys: 38,
            era: Era::City,
            stage: ConstructionStage::Finished,
            age: 0.0,
            seed: 0x51ee_7,
            variant: 0,
            biome: BiomeStyle::Temperate,
        }
    }

    fn draw_at(light: &Light) -> crate::Sprite {
        let mut s = crate::Sprite::new(1, 1);
        crate::draw(&tower(), light, &mut s);
        s
    }

    fn stats(s: &crate::Sprite) -> (usize, usize, f32) {
        // (emissive pixels, opaque pixels, mean blue-minus-red of the walls)
        let mut lit = 0;
        let mut n = 0;
        let mut db = 0.0;
        for &c in s.px.iter() {
            if c == 0 {
                continue;
            }
            let a = (c >> 24) & 0xFF;
            let (r, g, b) = ((c >> 16) & 0xFF, (c >> 8) & 0xFF, c & 0xFF);
            n += 1;
            if a == 0xFE {
                lit += 1;
            } else {
                db += b as f32 - r as f32;
            }
        }
        (lit, n, db / (n.max(1) - lit).max(1) as f32)
    }

    /// B99d, revised for B72 (D78): the evening is a fade, but *which* panes are
    /// lit is the people's business now, so the sprite bakes every pane dark and
    /// exports them for the renderer. What is still the sprite's to get right:
    /// no glow of its own at any hour, glass that goes on reflecting (cool)
    /// from noon to midnight, and the panes handed over.
    #[test]
    fn glass_stays_glass_and_the_panes_are_exported() {
        let day = draw_at(&Light::day());
        let dusk = draw_at(&crate::light_for([1.0, 1.0, 1.0], 0.60, 16.0));
        let night = draw_at(&Light::night());

        let (lit_d, _, cool_d) = stats(&day);
        let (lit_k, _, cool_k) = stats(&dusk);
        let (lit_n, _, cool_n) = stats(&night);

        assert_eq!(lit_d, 0, "no office lights on in daylight");
        // B72: the panes themselves are baked dark — the light is painted later,
        // so a pane pixel must not already carry the emissive flag. (The crown,
        // the lobby and the beacons are other lights and stay as they are.)
        for pane in &night.win {
            for &(x, y) in &pane.px {
                let c = night.px[y as usize * night.w + x as usize];
                assert_ne!((c >> 24) & 0xFF, 0xFE, "a pane pixel is already lit at bake");
            }
        }
        let _ = (lit_n, lit_k);
        assert!(cool_d > 0.0 && cool_k > 0.0, "glass reflects cool by day {cool_d} and at dusk {cool_k}");
        assert!(cool_n > 0.0, "night glass is still cool ({cool_n}), not a brown box");
        // Every pane the sim can switch is handed over, with its pixels.
        assert!(night.win.len() > 40, "a 38-storey tower exports its panes: {}", night.win.len());
        assert!(
            night.win.iter().all(|p| !p.px.is_empty()),
            "no pane without pixels"
        );
        assert!(
            night.win.iter().any(|p| p.warm),
            "some panes burn warm"
        );
    }

    /// B99d/B73: saturated cyan is a *designed* light, not a material. By day and
    /// at dusk a tower may not carry it at all; at night it may only be a crown
    /// band — the top three rows of the sprite (D80's per-tower crown colours).
    /// This is the rule the tower art and the nightlife's crown lights share.
    fn cyan_ok(c: u32, row_from_top: i32, night: bool) -> bool {
        let (r, g, b) = ((c >> 16) & 0xFF, (c >> 8) & 0xFF, c & 0xFF);
        let saturated_cyan = b > 0xD0 && g > 0xC0 && r < 0x60;
        !saturated_cyan || (night && row_from_top <= 2)
    }

    #[test]
    fn no_flat_cyan_parapet_on_a_tower() {
        for light in [Light::day(), crate::light_for([1.0, 1.0, 1.0], 0.60, 16.0), Light::night()] {
            let s = draw_at(&light);
            let night = light.bucket == Light::NIGHT_BUCKET;
            for j in 0..s.h {
                for i in 0..s.w {
                    let c = s.px[j * s.w + i];
                    if c == 0 {
                        continue;
                    }
                    assert!(
                        cyan_ok(c, j as i32, night),
                        "cyan pixel #{:06x} at row {j} of {}, bucket {}",
                        c & 0xFFFFFF,
                        s.h,
                        light.bucket
                    );
                }
            }
        }
    }

    /// The relaxed rule must still catch what B99d removed: a flat cyan band
    /// across the parapet, more than a designed crown's three rows.
    #[test]
    fn a_flat_cyan_band_is_still_caught() {
        let cyan = render::palette::rgb(0x26, 0xe0, 0xff);
        // A designed crown band, three rows at the top: allowed at night.
        for row in 0..3 {
            assert!(cyan_ok(cyan, row, true), "a crown band at row {row} is a design");
        }
        // A band four rows deep, or any cyan by day: not.
        assert!(!cyan_ok(cyan, 3, true), "a 4-row cyan slab is a bug, not a crown");
        assert!(!cyan_ok(cyan, 0, false), "no cyan by day");
    }
}
