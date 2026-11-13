//! Street furniture art (B41/D63): painted lines, kerb-side edge lines, street
//! light poles, street-name blades and stop signs, traffic lights, bus stops,
//! and the crews that install them. 64-px art, drawn on the static layer once
//! installed; only the signal lamps and the crews animate.

use crate::framebuffer::Framebuffer;
use crate::iso::Camera;
use crate::palette::{self, rgb};
use crate::tex;
use map::TileMap;

/// What the town has installed, in the form the renderer wants.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StreetLook {
    /// (x, y, 1 = dashes, 2 = double line)
    pub paint: Vec<(i32, i32, u8)>,
    /// (x, y) of lamp posts.
    pub lamps: Vec<(i32, i32)>,
    /// (x, y, 0 = name blade, 1 = stop sign)
    pub signs: Vec<(i32, i32, u8)>,
    /// (x, y) of traffic lights.
    pub lights: Vec<(i32, i32)>,
}

impl StreetLook {
    pub fn is_empty(&self) -> bool {
        self.paint.is_empty()
            && self.lamps.is_empty()
            && self.signs.is_empty()
            && self.lights.is_empty()
    }
}

const PAINT_WHITE: palette::Color = rgb(0xe8, 0xe4, 0xd4);
const POLE: palette::Color = rgb(0x3e, 0x42, 0x48);
const POLE_LIT: palette::Color = rgb(0x6a, 0x70, 0x78);
const SIGN_BLADE: palette::Color = rgb(0x3a, 0x6a, 0x48);
const STOP_RED: palette::Color = rgb(0xb8, 0x2a, 0x28);

/// The tile centre and the tile's two screen-space axes, for drawing along a
/// street (a tile is a diamond: the x axis runs down-right, the y down-left).
fn axes(cam: &Camera, map: &TileMap, x: i32, y: i32) -> ((f32, f32), (f32, f32), (f32, f32)) {
    let z = crate::iso::level_px(map.surface_level_at(x as f32 + 0.5, y as f32 + 0.5));
    let c = cam.project(x as f32 + 0.5, y as f32 + 0.5, z);
    let ax = cam.project(x as f32 + 1.5, y as f32 + 0.5, z);
    let ay = cam.project(x as f32 + 0.5, y as f32 + 1.5, z);
    let ux = (ax.0 - c.0, ax.1 - c.1);
    let uy = (ay.0 - c.0, ay.1 - c.1);
    (c, ux, uy)
}

/// Which way the street runs through this tile: true = along x.
fn runs_along_x(map: &TileMap, x: i32, y: i32) -> bool {
    map.at(x - 1, y).kind.is_lane() || map.at(x + 1, y).kind.is_lane()
}

/// Painted lines on one paved tile (B41 item 2, redrawn in B83).
///
/// * **Nothing at a junction.** A centre line stops at the mouth; the junction
///   itself is bare asphalt, which is what makes it read as one paved area
///   instead of two strips laid over each other.
/// * **Centre line by class**: a dashed line on a collector, a double solid
///   line on an avenue, nothing on a residential local street.
/// * **Edge lines only on an avenue**, and only where the kerb is.
/// * The dashes are phased off the tile's *absolute* position, so a run of
///   tiles gives one continuous dashed line rather than a dash per tile.
pub fn draw_paint(fb: &mut Framebuffer, cam: &Camera, map: &TileMap, x: i32, y: i32, style: u8) {
    if lane_degree(map, x, y) >= 3 {
        return;
    }
    let (c, ux, uy) = axes(cam, map, x, y);
    let along_x = runs_along_x(map, x, y);
    let (u, v) = if along_x { (ux, uy) } else { (uy, ux) };
    let at = |along: f32, across: f32| {
        (
            c.0 + u.0 * (along - 0.5) + v.0 * across,
            c.1 + u.1 * (along - 0.5) + v.1 * across,
        )
    };
    match style {
        2 => {
            // Avenue: a double solid line down the middle (the median), and an
            // edge line inside each kerb.
            for off in [-0.035f32, 0.035] {
                fb.line(at(0.0, off), at(1.0, off), PAINT_WHITE);
            }
            let (px, py) = if along_x { (0, 1) } else { (1, 0) };
            for (sign, on) in [
                (1.0f32, !map.at(x + px, y + py).kind.is_lane()),
                (-1.0, !map.at(x - px, y - py).kind.is_lane()),
            ] {
                if !on {
                    continue;
                }
                // A lane edge line, kept low-contrast: drawn bright it read as a
                // tram rail running down the street.
                let off = sign * 0.33;
                let ink = palette::mix(PAINT_WHITE, palette::rgb(0x6e, 0x6a, 0x60), 0.45);
                fb.line(at(0.18, off), at(0.82, off), ink);
            }
        }
        1 => {
            // Collector: one dashed line, phased on the tile's absolute index so
            // it runs on across tiles. 0.7 of a tile per period, 0.34 painted.
            let base = if along_x { x as f32 } else { y as f32 };
            let (period, dash) = (0.70f32, 0.34f32);
            let mut s = ((base / period).ceil()) * period;
            while s < base + 1.0 {
                let a = (s - base).max(0.0);
                let b = (s - base + dash).min(1.0);
                if b > a + 0.02 {
                    fb.line(at(a, 0.0), at(b, 0.0), PAINT_WHITE);
                }
                s += period;
            }
        }
        _ => {}
    }
}

/// How many lane neighbours a tile has (the renderer's copy of the sim's rule).
pub fn lane_degree(map: &TileMap, x: i32, y: i32) -> u8 {
    [(1i32, 0i32), (-1, 0), (0, 1), (0, -1)]
        .iter()
        .filter(|(dx, dy)| map.at(x + dx, y + dy).kind.is_lane())
        .count() as u8
}

/// B83: the markings at a junction mouth — a stop (give-way) bar and a zebra,
/// on the approach tile that touches the junction, never inside the junction
/// and never in the middle of a segment. A path that crosses a street mid-block
/// gets its own zebra (and only that).
pub fn draw_mouth(fb: &mut Framebuffer, cam: &Camera, map: &TileMap, x: i32, y: i32) {
    if lane_degree(map, x, y) != 2 {
        return;
    }
    let (c, ux, uy) = axes(cam, map, x, y);
    let along_x = runs_along_x(map, x, y);
    let (u, v) = if along_x { (ux, uy) } else { (uy, ux) };
    let at = |along: f32, across: f32| {
        (
            c.0 + u.0 * (along - 0.5) + v.0 * across,
            c.1 + u.1 * (along - 0.5) + v.1 * across,
        )
    };
    let (dx, dy) = if along_x { (1, 0) } else { (0, 1) };
    // A zebra is a set of short bars across the carriageway.
    let zebra = |fb: &mut Framebuffer, from: f32, towards_junction: f32| {
        for i in 0..3 {
            let f = from + i as f32 * 0.05 * towards_junction;
            fb.line(at(f, -0.22), at(f, 0.22), palette::mix(PAINT_WHITE, palette::rgb(0xb0, 0xac, 0x9c), 0.15));
        }
    };
    for sign in [1.0f32, -1.0] {
        let (nx, ny) = (x + dx * sign as i32, y + dy * sign as i32);
        if lane_degree(map, nx, ny) >= 3 {
            // The mouth: zebra just off the junction edge, the bar behind it.
            let edge = if sign > 0.0 { 0.94 } else { 0.06 };
            let dir = -sign;
            zebra(fb, edge, dir);
            let bar = if sign > 0.0 { 0.58 } else { 0.42 };
            fb.line(at(bar, -0.20), at(bar, 0.20), PAINT_WHITE);
            continue;
        }
        // A path crossing the street here (not near a junction) gets a zebra.
        let (px, py) = if along_x { (0, 1) } else { (1, 0) };
        for side in [1i32, -1] {
            let (sx, sy) = (x + px * side, y + py * side);
            if !map.in_bounds(sx, sy) || map.at(sx, sy).kind.is_lane() {
                continue;
            }
            if map::WearStage::of(map.wear(sx, sy)) >= map::WearStage::Footpath {
                if sign > 0.0 {
                    zebra(fb, 0.50, 1.0);
                    zebra(fb, 0.50, -1.0);
                }
                break;
            }
        }
    }
}

/// B59: a highway lane. Two lanes share one dashed centre line, and the outer
/// side of the carriageway carries an edge line and a guardrail, so a highway
/// reads as a road that passes the city rather than one more street.
pub fn draw_highway(fb: &mut Framebuffer, cam: &Camera, map: &TileMap, x: i32, y: i32) {
    let (c, ux, uy) = axes(cam, map, x, y);
    let along_x = runs_along_x(map, x, y);
    let (u, v) = if along_x { (ux, uy) } else { (uy, ux) };
    // Dashed centre line: half the tiles paint a dash, so the line reads as
    // dashes at this scale without a per-pixel stipple.
    if (x + y) % 2 == 0 {
        let a = (c.0 + u.0 * -0.5, c.1 + u.1 * -0.5);
        let b = (c.0 + u.0 * 0.5, c.1 + u.1 * 0.5);
        let a2 = (a.0 + (b.0 - a.0) * 0.25, a.1 + (b.1 - a.1) * 0.25);
        let b2 = (a.0 + (b.0 - a.0) * 0.75, a.1 + (b.1 - a.1) * 0.75);
        fb.line(a2, b2, PAINT_WHITE);
    }
    // Edge lines and the guardrail, on the side that is not another lane.
    let mut rail = false;
    for sign in [1.0f32, -1.0] {
        // `v` is a whole tile wide: the edge line stands 0.44 of a tile off the
        // centre line, just inside the kerb (B59b: it was ten tiles off).
        let off = sign * 0.44;
        let edge = (c.0 + v.0 * off, c.1 + v.1 * off);
        let a = (edge.0 + u.0 * -0.5, edge.1 + u.1 * -0.5);
        let b = (edge.0 + u.0 * 0.5, edge.1 + u.1 * 0.5);
        // Which neighbour is across the carriageway this way?
        let (px, py) = if along_x { (0, sign as i32) } else { (sign as i32, 0) };
        if !map.at(x + px, y + py).kind.is_lane() {
            fb.line(a, b, palette::mix(PAINT_WHITE, palette::rgb(0x9a, 0x96, 0x88), 0.30));
            // Guardrail: posts every half tile, with the rail between them.
            let rail_c = palette::rgb(0xb0, 0xb4, 0xb8);
            for t in [0.05f32, 0.55] {
                let p0 = (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
                fb.line((p0.0, p0.1 - 4.0), p0, rail_c);
            }
            rail = true;
        }
    }
    let _ = rail;
}

/// B59b: a ramp tile of an interchange — a leg from the highway to the street.
/// Solid white edge lines and a guardrail on both sides, a chevron showing the
/// way on, and at the mouth (the tile that touches the highway) hatching on the
/// gore and a green sign on a post.
pub fn draw_ramp(fb: &mut Framebuffer, cam: &Camera, map: &TileMap, x: i32, y: i32) {
    let (c, ux, uy) = axes(cam, map, x, y);
    let along_x = runs_along_x(map, x, y);
    let (u, v) = if along_x { (ux, uy) } else { (uy, ux) };
    let edge = palette::rgb(0xe6, 0xe2, 0xd4);
    let rail_c = palette::rgb(0xb0, 0xb4, 0xb8);
    // The highway tile this one joins, if any: the mouth.
    let mouth = [(1i32, 0i32), (-1, 0), (0, 1), (0, -1)]
        .into_iter()
        .find(|&(dx, dy)| map.at(x + dx, y + dy).aux & map::AUX_HIGHWAY != 0);
    for sign in [1.0f32, -1.0] {
        let off = sign * 0.44;
        let ec = (c.0 + v.0 * off, c.1 + v.1 * off);
        let a = (ec.0 + u.0 * -0.5, ec.1 + u.1 * -0.5);
        let b = (ec.0 + u.0 * 0.5, ec.1 + u.1 * 0.5);
        let (px, py) = if along_x { (0, sign as i32) } else { (sign as i32, 0) };
        // No line or rail where another lane (the highway or a street) opens.
        if map.at(x + px, y + py).kind.is_lane() {
            continue;
        }
        fb.line(a, b, edge);
        for t in [0.1f32, 0.5, 0.9] {
            let q = (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
            fb.line((q.0, q.1 - 3.0), q, rail_c);
        }
        fb.line((a.0, a.1 - 3.0), (b.0, b.1 - 3.0), palette::shade(rail_c, 0.85));
    }
    // A chevron on the lane, pointing along the ramp (away from the mouth).
    let dir = match mouth {
        Some((dx, dy)) => {
            // Along the ramp, away from the highway.
            let s = if along_x { -(dx as f32) } else { -(dy as f32) };
            s.signum()
        }
        None => 1.0,
    };
    let arrow = palette::rgb(0xd8, 0xd8, 0xcc);
    let tip = (c.0 + u.0 * 0.22 * dir, c.1 + u.1 * 0.22 * dir);
    let tail = (c.0 - u.0 * 0.20 * dir, c.1 - u.1 * 0.20 * dir);
    fb.line(tail, tip, arrow);
    for side in [-1.0f32, 1.0] {
        let wing = (tip.0 - u.0 * 0.13 * dir + v.0 * 0.10 * side, tip.1 - u.1 * 0.13 * dir + v.1 * 0.10 * side);
        fb.line(tip, wing, arrow);
    }
    if let Some((dx, dy)) = mouth {
        // The gore: short diagonal hatching beside the lane where it meets the
        // carriageway.
        let toward = (c.0 + (if dx != 0 { ux.0 * dx as f32 } else { uy.0 * dy as f32 }) * 0.4, c.1 + (if dx != 0 { ux.1 * dx as f32 } else { uy.1 * dy as f32 }) * 0.4);
        for k in 0..4 {
            let f = k as f32 / 3.0 - 0.5;
            let p0 = (toward.0 + v.0 * 0.32 * f - u.0 * 0.0, toward.1 + v.1 * 0.32 * f);
            fb.line(p0, (p0.0 + 3.0, p0.1 - 2.0), palette::rgb(0xe6, 0xe2, 0xd4));
        }
        // The green sign on a post at the roadside.
        let side = if map.at(x + (if along_x { 0 } else { 1 }), y + (if along_x { 1 } else { 0 })).kind.is_lane() { -1.0 } else { 1.0 };
        let foot = (c.0 + v.0 * 0.58 * side, c.1 + v.1 * 0.58 * side);
        let top = (foot.0, foot.1 - 15.0);
        fb.line(foot, top, rail_c);
        let (bx, by) = (top.0 as i32 - 6, top.1 as i32 - 7);
        fb.fill_rect(bx - 1, by - 1, 14, 9, palette::rgb(0xe8, 0xe8, 0xe0));
        fb.fill_rect(bx, by, 12, 7, palette::rgb(0x1e, 0x6a, 0x42));
        // An arrow pointing up the ramp: a shaft and a head.
        fb.fill_rect(bx + 3, by + 3, 6, 1, palette::rgb(0xf4, 0xf4, 0xee));
        fb.put(bx + 8, by + 2, palette::rgb(0xf4, 0xf4, 0xee));
        fb.put(bx + 8, by + 4, palette::rgb(0xf4, 0xf4, 0xee));
    }
}

/// Where a lamp stands and where its head reaches.
///
/// B42g: the tile axes from [`axes`] are a whole tile wide (32×16 px), not unit
/// vectors, so the 9-px offsets they were multiplied by threw the pole and the
/// head most of a tile away from the kerb — which is why the pool sat diagonally
/// off its head. Everything here is in pixels now.
fn lamp_geom(
    cam: &Camera,
    map: &TileMap,
    x: i32,
    y: i32,
) -> ((f32, f32), (f32, f32), (f32, f32), (f32, f32)) {
    let (c, ux, uy) = axes(cam, map, x, y);
    let along_x = runs_along_x(map, x, y);
    let (px, py) = if along_x { (0, 1) } else { (1, 0) };
    let side = if !map.at(x + px, y + py).kind.is_lane() { 1.0 } else { -1.0 };
    let v = if along_x { uy } else { ux };
    let v = unit(v);
    let u = if along_x { unit(ux) } else { unit(uy) };
    // The pole stands at the kerb, one stride off the tile's centre.
    let foot = (c.0 + v.0 * 11.0 * side, c.1 + v.1 * 11.0 * side);
    // ...and the head reaches back over the street, 21 px up.
    let head = (foot.0 - v.0 * 13.0 * side, foot.1 - v.1 * 13.0 * side - 21.0);
    (foot, head, u, v)
}

fn unit(v: (f32, f32)) -> (f32, f32) {
    let l = (v.0 * v.0 + v.1 * v.1).sqrt();
    if l < 0.001 {
        (0.0, 0.0)
    } else {
        (v.0 / l, v.1 / l)
    }
}

/// Where a street pole stands on screen and how tall it is, in pixels, for the
/// shadow pass (B68): `kind` 0 = lamp, 1 = sign, 2 = traffic light.
pub fn pole_foot(cam: &Camera, map: &TileMap, kind: u8, x: i32, y: i32) -> ((f32, f32), f32) {
    match kind {
        0 => (lamp_geom(cam, map, x, y).0, 24.0),
        1 => {
            let (c, _, _) = axes(cam, map, x, y);
            ((c.0, c.1 - 2.0), 15.0)
        }
        _ => {
            let (c, _, _) = axes(cam, map, x, y);
            ((c.0 + 6.0, c.1 - 2.0), 19.0)
        }
    }
}

/// A street light: a pole at the kerb, a head over the road (B41 item 2). The
/// pool on the ground is a second pass (B42f: [`draw_lamp_pool`]).
///
/// B49/D72: the head is whatever the world fits now — a **gas lantern** (a
/// cast-iron post with a glass box), a **sodium** lamp (an orange cobra head),
/// or an **LED** (a slim white head). The night colour follows the head.
pub fn draw_lamp(
    fb: &mut Framebuffer,
    cam: &Camera,
    map: &TileMap,
    x: i32,
    y: i32,
    glow: f32,
    level: u8,
) {
    let (foot, head, _u, _v) = lamp_geom(cam, map, x, y);
    let (bx, by) = foot;
    // Pole: two pixels wide with a lit edge, and a base plate. A gas lamp's post
    // is slimmer and darker (cast iron); the electric ones are steel.
    let pole = match level {
        0 => palette::rgb(0x2e, 0x30, 0x34),
        _ => POLE,
    };
    fb.fill_rect(bx as i32, by as i32 - 21, 2, 21, pole);
    fb.fill_rect(bx as i32 + 1, by as i32 - 21, 1, 21, POLE_LIT);
    fb.fill_rect(bx as i32 - 1, by as i32 - 1, 4, 2, palette::rgb(0x55, 0x58, 0x5c));
    match level {
        0 => {
            // Gas: the head sits straight on top of the post, a glass box in a
            // little iron cage.
            let lx = (bx as i32) - 1;
            let ly = by as i32 - 26;
            if glow > 0.35 {
                fb.fill_rect(lx, ly, 4, 5, palette::rgb(0xff, 0xc0, 0x6a));
                fb.fill_rect(lx + 1, ly + 1, 2, 3, palette::rgb(0xff, 0xf0, 0xc0));
            } else {
                fb.fill_rect(lx, ly, 4, 5, palette::rgb(0xd8, 0xd2, 0xb8));
                fb.fill_rect(lx + 1, ly + 1, 2, 3, palette::rgb(0xc4, 0xbe, 0xa4));
            }
            fb.fill_rect(lx - 1, ly - 1, 6, 1, palette::rgb(0x2a, 0x2c, 0x30));
            fb.fill_rect(lx - 1, ly + 5, 6, 1, palette::rgb(0x2a, 0x2c, 0x30));
            fb.put(lx - 1, ly + 2, palette::rgb(0x2a, 0x2c, 0x30));
            fb.put(lx + 4, ly + 2, palette::rgb(0x2a, 0x2c, 0x30));
        }
        1 => {
            // Sodium: the familiar cobra head, a shallow dish of orange light.
            fb.line((bx, by - 21.0), head, POLE);
            fb.fill_rect(head.0 as i32 - 3, head.1 as i32 - 1, 7, 2, palette::rgb(0x4a, 0x4e, 0x54));
            if glow > 0.35 {
                fb.fill_rect(head.0 as i32 - 2, head.1 as i32 + 1, 5, 2, palette::rgb(0xff, 0x9c, 0x38));
            } else {
                fb.fill_rect(head.0 as i32 - 2, head.1 as i32 + 1, 5, 2, palette::rgb(0xc8, 0xa8, 0x84));
            }
        }
        _ => {
            // LED: a slim white head on the same arm.
            fb.line((bx, by - 21.0), head, POLE);
            fb.fill_rect(head.0 as i32 - 3, head.1 as i32 - 1, 6, 2, palette::rgb(0x5a, 0x60, 0x68));
            if glow > 0.35 {
                fb.fill_rect(head.0 as i32 - 2, head.1 as i32 + 1, 5, 2, palette::rgb(0xe8, 0xf2, 0xff));
            } else {
                fb.fill_rect(head.0 as i32 - 2, head.1 as i32 + 1, 5, 2, palette::rgb(0xd8, 0xdc, 0xe2));
            }
        }
    }
}

/// B42f/B42g: the warm pool a lit street light throws on the road and pavement.
///
/// * **Under the head.** The pool is centred on the ground point directly below
///   the lamp head, not on the head itself (B42g: it sat diagonally off).
/// * **Ground only.** It brightens ground, road and pavement pixels and nothing
///   else: a building, a roof or a tree crown keeps its colour, so a house in
///   front hides the pool the way depth order says it should.
/// * **Along the street.** The falloff is an ellipse in *tile* space (long along
///   the street, narrow across it), so it follows the iso diamond instead of
///   being a screen-aligned blob.
/// * Pool pixels are written with the emissive tag (`0xFE`), so the night tint
///   keeps them bright and a re-bake never dims or stacks them.
///
/// A junction (two or more lamps within two tiles) gets a wider, brighter pool.
pub fn draw_lamp_pool(
    fb: &mut Framebuffer,
    cam: &Camera,
    map: &TileMap,
    x: i32,
    y: i32,
    glow: f32,
    neighbours: u8,
    level: u8,
) {
    if glow <= 0.35 {
        return;
    }
    let (_c, ux, uy) = axes(cam, map, x, y);
    let (_foot, head, _u, _v) = lamp_geom(cam, map, x, y);
    // The ground directly below the head: the head hangs 21 px above the pole.
    let cx = head.0;
    let cy = head.1 + 21.0;
    let along_x = runs_along_x(map, x, y);
    let busy = neighbours >= 2;
    // Radii in **tiles**: long along the street, narrow across it.
    // One pool per lamp, not a continuous ribbon: the reach stays under the
    // spacing the crews use (a lamp every two or three tiles).
    let (r_along, r_across) = if busy { (0.74f32, 0.40f32) } else { (0.60f32, 0.32f32) };
    // B49/D72: the colour of the light is the technology's. Gas is a dim amber
    // patch, sodium throws a wide orange one, LED a tighter, brighter white.
    let (light, gain_k) = match level {
        0 => (palette::rgb(0xff, 0xc0, 0x74), 0.24),
        1 => (palette::rgb(0xff, 0xa8, 0x50), 0.32),
        _ => (palette::rgb(0xdc, 0xe8, 0xff), 0.34),
    };
    let gain = if busy { gain_k + 0.06 } else { gain_k } * glow;
    // Screen delta -> tile delta (the inverse of the iso projection).
    let det = ux.0 * uy.1 - ux.1 * uy.0;
    if det.abs() < 0.001 {
        return;
    }
    let to_tile = |dx: f32, dy: f32| {
        (
            (dx * uy.1 - dy * uy.0) / det,
            (ux.0 * dy - ux.1 * dx) / det,
        )
    };
    let reach = 52.0;
    let w = fb.w as i32;
    let clipped = fb.clip();
    let (x0, x1) = ((cx - reach) as i32, (cx + reach) as i32);
    let (y0, y1) = ((cy - reach * 0.8) as i32, (cy + reach * 0.8) as i32);
    for py in y0..=y1 {
        for px in x0..=x1 {
            if px < 0 || py < 0 || px >= w || py >= fb.h as i32 {
                continue;
            }
            if px < clipped.x0 || px >= clipped.x1 || py < clipped.y0 || py >= clipped.y1 {
                continue;
            }
            let (a_t, b_t) = to_tile(px as f32 - cx, py as f32 - cy);
            let (along, across) = if along_x { (a_t, b_t) } else { (b_t, a_t) };
            let e = ((along / r_along).powi(2) + (across / r_across).powi(2)).sqrt();
            if e >= 1.0 {
                continue;
            }
            let i = py as usize * w as usize + px as usize;
            let cur = fb.px()[i];
            let a_cur = palette::alpha(cur);
            // Ground only: a prop's crown or a building's wall keeps its
            // colour. Roads and pavements carry the "built" alpha tag (they are
            // man-made and must not take the haze), so buildings are told apart
            // by the painter key: tiles are odd, buildings even (`static_key`).
            // A building in front of the road therefore hides the pool.
            if cur == palette::TRANSPARENT || a_cur == 0xFE || a_cur == crate::framebuffer::TAG_PROP {
                continue;
            }
            if let Some(depth) = fb.depth() {
                let key = depth[i];
                if key != 0 && key & 1 == 0 {
                    continue;
                }
            }
            // Smooth falloff, then an ordered dither at a fine quantum: the
            // gradient has no visible ring, and the dither is subtle because
            // the step is 3 % alpha (B42g: a coarse rim dither read as
            // salt-and-pepper over the whole street).
            let a = gain * (1.0 - e).powf(2.0);
            const STEP: f32 = 0.03;
            let q = a / STEP;
            let base = q.floor();
            let mut a = base * STEP;
            if q - base > hash01(px, py) {
                a += STEP;
            }
            if a <= 0.0 {
                continue;
            }
            let blended = palette::mix(cur, light, a.clamp(0.0, 0.9));
            fb.px_mut()[i] = palette::with_alpha(blended, 0xFE);
        }
    }
}

/// A deterministic 0..1 from a pixel position (the pool's rim dither).
fn hash01(x: i32, y: i32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x9E37_79B1) ^ (y as u32).wrapping_mul(0x85EB_CA6B);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2545_F491);
    h ^= h >> 13;
    (h & 0xFFFF) as f32 / 65535.0
}

/// A street-name blade (at a junction) or a stop sign (on a minor approach).
pub fn draw_sign(fb: &mut Framebuffer, cam: &Camera, map: &TileMap, x: i32, y: i32, kind: u8) {
    let (c, _ux, _uy) = axes(cam, map, x, y);
    let bx = c.0;
    let by = c.1 - 2.0;
    fb.line((bx, by), (bx, by - 12.0), POLE);
    if kind == 1 {
        // A stop sign: a red octagon on the pole.
        let (cx, cy) = (bx as i32, by as i32 - 14);
        fb.fill_rect(cx - 3, cy - 2, 7, 5, STOP_RED);
        fb.fill_rect(cx - 2, cy - 3, 5, 7, STOP_RED);
        fb.fill_rect(cx - 1, cy - 1, 3, 3, rgb(0xe8, 0xe4, 0xd8));
        fb.put(cx, cy - 1, STOP_RED);
    } else {
        // A street-name blade: a green plate with a pale border, set crosswise.
        let (cx, cy) = (bx as i32, by as i32 - 15);
        fb.fill_rect(cx - 8, cy - 2, 17, 5, rgb(0x24, 0x40, 0x2c));
        fb.fill_rect(cx - 7, cy - 1, 15, 3, SIGN_BLADE);
        fb.fill_rect(cx - 6, cy, 13, 1, rgb(0xd8, 0xd4, 0xc0));
    }
}

/// A traffic light: a pole with a three-lamp head. `phase` is 0..1 through the
/// cycle; the lit lamp glows at night (B41 item 2).
pub fn draw_traffic_light(
    fb: &mut Framebuffer,
    cam: &Camera,
    map: &TileMap,
    x: i32,
    y: i32,
    phase: f32,
    glow: f32,
) {
    let (c, _ux, _uy) = axes(cam, map, x, y);
    let bx = c.0 + 6.0;
    let by = c.1 - 2.0;
    fb.line((bx, by), (bx, by - 15.0), POLE);
    let (hx, hy) = (bx as i32, by as i32 - 19);
    fb.fill_rect(hx - 2, hy - 3, 5, 10, palette::rgb(0x26, 0x2a, 0x2e));
    fb.fill_rect(hx - 1, hy - 2, 3, 8, palette::rgb(0x3a, 0x3e, 0x44));
    // Red for the first half of the cycle, green for the second (an amber flash
    // either side of the change).
    let red = phase < 0.45;
    let amber = (0.45..0.52).contains(&phase) || (0.95..1.0).contains(&phase);
    let green = phase >= 0.52 && phase < 0.95;
    let dim = |c: palette::Color| palette::mix(c, palette::rgb(0x20, 0x20, 0x22), 0.55);
    let red_c = if red || amber {
        rgb(0xff, 0x4a, 0x38)
    } else {
        dim(rgb(0x8a, 0x24, 0x20))
    };
    let amber_c = if amber {
        rgb(0xff, 0xc0, 0x40)
    } else {
        dim(rgb(0x8a, 0x6a, 0x20))
    };
    let green_c = if green {
        rgb(0x48, 0xf0, 0x78)
    } else {
        dim(rgb(0x1e, 0x6a, 0x34))
    };
    fb.fill_rect(hx - 1, hy - 2, 3, 2, red_c);
    fb.fill_rect(hx - 1, hy + 1, 3, 2, amber_c);
    fb.fill_rect(hx - 1, hy + 4, 3, 2, green_c);
    if glow > 0.35 {
        let lit = if red {
            red_c
        } else if green {
            green_c
        } else {
            amber_c
        };
        let ly = if red {
            hy - 1
        } else if green {
            hy + 5
        } else {
            hy + 2
        };
        for dy in -3i32..=3 {
            for dx in -4i32..=4 {
                let d = ((dx * dx) as f32 / 16.0 + (dy * dy) as f32 / 9.0).sqrt();
                let a = (1.0 - d).clamp(0.0, 1.0) * 0.5 * glow;
                if a > 0.03 {
                    fb.put_blended(hx + dx, ly + dy, lit, a);
                }
            }
        }
    }
}

/// A crew at work on the street: a paint truck (kind 0) or a worker with a
/// sign/light (1-3). Drawn over the road, so it is visible moving along it.
pub fn draw_crew(fb: &mut Framebuffer, cam: &Camera, map: &TileMap, x: i32, y: i32, kind: u8) {
    let (c, ux, uy) = axes(cam, map, x, y);
    let along_x = runs_along_x(map, x, y);
    let (u, v) = if along_x { (ux, uy) } else { (uy, ux) };
    let bx = c.0 + u.0 * 0.15;
    let by = c.1 + u.1 * 0.15;
    if kind >= 4 {
        // B49: a service van and its worker — a fitting crew (lamps, small
        // cells, charger bays) parked at the kerb with the beacon on.
        let (vx, vy) = (bx as i32 - 8, by as i32 - 9);
        fb.fill_rect(vx, vy, 15, 7, rgb(0xf2, 0xf0, 0xe8));
        fb.fill_rect(vx, vy, 15, 1, rgb(0xd8, 0xd4, 0xca));
        fb.fill_rect(vx + 1, vy + 2, 4, 3, palette::rgb(0x8a, 0x96, 0xa4));
        fb.fill_rect(vx + 6, vy + 2, 5, 3, palette::rgb(0x8a, 0x96, 0xa4));
        fb.fill_rect(vx + 12, vy + 2, 2, 3, palette::rgb(0x9a, 0xa6, 0xb4));
        fb.fill_rect(vx + 2, vy - 1, 4, 2, rgb(0xe0, 0x7a, 0x28));
        fb.fill_rect(vx + 3, vy - 2, 2, 1, rgb(0xf4, 0xd0, 0x40));
        fb.fill_rect(vx + 3, vy + 7, 2, 1, palette::rgb(0x28, 0x28, 0x2e));
        fb.fill_rect(vx + 11, vy + 7, 2, 1, palette::rgb(0x28, 0x28, 0x2e));
        // The worker beside it, with the tool.
        let (px, py) = (vx + 17, by as i32 - 4);
        fb.fill_rect(px, py - 4, 3, 5, rgb(0xf0, 0x8a, 0x2a));
        fb.fill_rect(px, py - 7, 3, 3, palette::rgb(0xe8, 0xc4, 0xa0));
        fb.fill_rect(px, py - 7, 3, 1, rgb(0xf4, 0xd0, 0x40));
        fb.fill_rect(px, py, 1, 2, palette::rgb(0x3a, 0x3a, 0x44));
        fb.fill_rect(px + 2, py, 1, 2, palette::rgb(0x3a, 0x3a, 0x44));
        fb.line((px as f32 + 4.0, py as f32 - 4.0), (px as f32 + 4.0, py as f32 - 16.0), rgb(0x6a, 0x6e, 0x76));
    } else if kind == 0 {
        // The line-painting truck: a white cab, an orange tank, a spray bar.
        fb.fill_rect(bx as i32 - 7, by as i32 - 7, 15, 5, rgb(0xf0, 0xee, 0xe6));
        fb.fill_rect(bx as i32 - 7, by as i32 - 7, 15, 1, rgb(0xc8, 0xc4, 0xb8));
        fb.fill_rect(bx as i32 - 3, by as i32 - 10, 7, 4, rgb(0xe0, 0x7a, 0x28));
        fb.fill_rect(bx as i32 - 3, by as i32 - 10, 7, 1, rgb(0xf4, 0xa0, 0x50));
        fb.fill_rect(bx as i32 - 7, by as i32 - 3, 2, 2, rgb(0x2c, 0x2c, 0x30));
        fb.fill_rect(bx as i32 + 5, by as i32 - 3, 2, 2, rgb(0x2c, 0x2c, 0x30));
        // The spray bar behind, and a fresh white stripe under it.
        fb.line(
            (bx - u.0 * 9.0 - v.0 * 6.0, by - u.1 * 9.0 - v.1 * 6.0),
            (bx - u.0 * 9.0 + v.0 * 6.0, by - u.1 * 9.0 + v.1 * 6.0),
            rgb(0x9a, 0x9a, 0x9a),
        );
        fb.fill_rect(bx as i32 - 4, by as i32 - 2, 9, 1, PAINT_WHITE);
        // A worker walking behind the truck.
        fb.fill_rect(bx as i32 - 11, by as i32 - 8, 3, 5, rgb(0xf0, 0x8a, 0x2a));
        fb.fill_rect(bx as i32 - 11, by as i32 - 11, 3, 3, palette::rgb(0xe8, 0xc4, 0xa0));
        fb.fill_rect(bx as i32 - 11, by as i32 - 11, 3, 1, rgb(0xf4, 0xd0, 0x40));
    } else {
        // A worker with a pole: an orange vest and a tool over the shoulder.
        let (px, py) = (bx as i32 - 4, by as i32 - 4);
        fb.fill_rect(px, py - 4, 3, 5, rgb(0xf0, 0x8a, 0x2a));
        fb.fill_rect(px, py - 7, 3, 3, palette::rgb(0xe8, 0xc4, 0xa0));
        fb.fill_rect(px, py - 7, 3, 1, rgb(0xf4, 0xd0, 0x40));
        fb.fill_rect(px, py, 1, 2, palette::rgb(0x3a, 0x3a, 0x44));
        fb.line(
            (px as f32 + 4.0, py as f32 - 3.0),
            (px as f32 + 4.0, py as f32 - 16.0),
            rgb(0x6a, 0x6e, 0x76),
        );
    }
    let _ = tex::hash2;
}

#[cfg(test)]
mod road_tests {
    use super::*;
    use crate::framebuffer::Framebuffer;

    /// A straight street (3..=18 at y=10) with a T at (10,10) going north.
    fn street_map() -> TileMap {
        let mut m = map::generate_with(3, 21, 21, Some(map::Biome::Temperate)).expect("map");
        for y in 0..m.h {
            for x in 0..m.w {
                m.with_tile_mut(x, y, |t| {
                    t.kind = map::Kind::Grass;
                    t.corner = [4; 4];
                    t.prop = map::Prop::None;
                });
            }
        }
        for x in 3..=18 {
            m.with_tile_mut(x, 10, |t| t.kind = map::Kind::Paved);
        }
        for y in 6..=10 {
            m.with_tile_mut(10, y, |t| t.kind = map::Kind::Paved);
        }
        m.wear.iter_mut().for_each(|w| *w = 0);
        m
    }

    fn painted(fb: &Framebuffer) -> usize {
        fb.px().iter().filter(|&&p| p == PAINT_WHITE).count()
    }

    /// B83: a junction is bare asphalt — a centre line through it is what made
    /// the crossings look like two strips laid over each other.
    #[test]
    fn no_marking_is_painted_inside_a_junction() {
        let m = street_map();
        assert_eq!(lane_degree(&m, 10, 10), 3, "the T is not a T");
        let cam = Camera::new(320, -180);
        let mut fb = Framebuffer::new(640, 280);
        for style in [1u8, 2] {
            draw_paint(&mut fb, &cam, &m, 10, 10, style);
            assert_eq!(painted(&fb), 0, "style {style} painted the junction tile");
        }
    }

    /// B83: the stop bar and the zebra are on the approach, at the mouth — not in
    /// the middle of the segment, and never on a tile that touches nothing.
    #[test]
    fn mouth_markings_are_at_the_mouth_only() {
        let m = street_map();
        let cam = Camera::new(320, -180);
        // The approach west of the T, whose +x side is the junction.
        let mut fb = Framebuffer::new(640, 280);
        draw_mouth(&mut fb, &cam, &m, 9, 10);
        let n = painted(&fb);
        assert!(n > 0, "the approach tile has no mouth markings");
        let (cx, _) = cam.project(9.5, 10.5, 0); // tile centre
        // Every bar sits on the junction half of the tile.
        let mut min_x = f32::MAX;
        for (i, &p) in fb.px().iter().enumerate() {
            if p == PAINT_WHITE {
                min_x = min_x.min((i % fb.w) as f32);
            }
        }
        assert!(min_x > cx - 6.0, "markings run back into the segment (x {min_x}, centre {cx})");

        // A tile two streets away from any junction gets nothing at all.
        let mut fb2 = Framebuffer::new(640, 280);
        draw_mouth(&mut fb2, &cam, &m, 16, 10);
        assert_eq!(painted(&fb2), 0, "a plain segment got a zebra");
    }

    /// B83: the renderer's lane degree agrees with the map it is looking at.
    #[test]
    fn lane_degree_counts_the_arms() {
        let m = street_map();
        assert_eq!(lane_degree(&m, 16, 10), 2);
        assert_eq!(lane_degree(&m, 10, 10), 3);
        assert_eq!(lane_degree(&m, 10, 6), 1);
        assert_eq!(lane_degree(&m, 1, 1), 0);
    }
}
