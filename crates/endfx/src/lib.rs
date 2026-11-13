//! The glue between the sim's world ending and the renderer (B50).
//!
//! The sim says what is happening ([`sim::ending::View`]); the renderer draws
//! it ([`render::ending::Fx`]). Both the wallpaper and the spike need the same
//! translation, the same weather overrides and the same "what does the scene
//! need to hear this frame", so it lives here once.

pub mod civic;
pub mod kinds;

use render::ending::Fx;
use render::terrain::WeatherLook;
use sim::ending::{Ending, Phase};

/// The renderer's picture of the ending, or `None` while the world lives.
pub fn fx(s: &sim::Sim, sun_px: Option<(f32, f32)>) -> Option<Fx> {
    let v = s.ending_view()?;
    if v.phase == Phase::Done {
        return None;
    }
    Some(Fx {
        kind: v.kind.id(),
        phase: v.phase.id(),
        p: v.p,
        u: v.u,
        secs: v.secs,
        dur: v.dur,
        site: v.site,
        aux: v.aux,
        seed: v.seed,
        frac: v.frac,
        sun_px,
        fires: false,
    })
}

/// The ruin code of every lot, in lot order.
pub fn ruins(s: &sim::Sim) -> Vec<u8> {
    // B52: the ending's own codes win; otherwise a lot wears its civic state
    // (vacant, scorched, lights out, acid-stained).
    (0..s.lots.len())
        .map(|i| match s.ruin_of(i) {
            0 => s.civic_ruin_of(i),
            c => c,
        })
        .collect()
}

/// What the scene has been told, so it is only told again when it changes.
#[derive(Default)]
pub struct Tracker {
    pub ruin_epoch: u64,
    pub overlay_epoch: u64,
    pub overlay_kind: u8,
    pub lots: usize,
}

/// Burning and smouldering lots, for the smoke and the glow.
pub fn marks(s: &sim::Sim, objs: &[render::CityObj]) -> Vec<(f32, f32, u8, f32)> {
    let mut out = Vec::new();
    for (i, l) in s.lots.iter().enumerate() {
        let mut code = s.ruin_of(i);
        // The smog's smokestacks: every works chimney smokes.
        if code != sim::ending::R_FIRE
            && code != sim::ending::R_RUBBLE
            && l.kind == sim::LotKind::Industry
            && s.ending.kind == sim::ending::Ending::Smog
            && s.ending_phase().0 >= sim::ending::Phase::Warning
        {
            code = 20;
        }
        if code != sim::ending::R_FIRE && code != sim::ending::R_RUBBLE && code != sim::ending::R_FROST && code != 20 {
            continue;
        }
        // Frost drifts on a share of the lots only (they are many).
        if code == sim::ending::R_FROST && (l.x * 7 + l.y * 13) % 3 != 0 {
            continue;
        }
        let z = match (code, objs.get(i)) {
            (20, Some(o)) => o.level as f32 * render::iso::HEIGHT_UNIT as f32 + o.wall_height() as f32 + 14.0,
            (sim::ending::R_FROST, _) => 0.0,
            (sim::ending::R_FIRE, Some(o)) => o.level as f32 * render::iso::HEIGHT_UNIT as f32 + o.wall_height() as f32 * 0.62,
            (_, Some(o)) => o.level as f32 * render::iso::HEIGHT_UNIT as f32 + 8.0,
            _ => 10.0,
        };
        out.push((l.x as f32 + l.w as f32 * 0.5, l.y as f32 + l.h as f32 * 0.5, code, z));
    }
    out
}

/// Tell the scene what the ending looks like right now. Returns
/// `(redraw, rebuild_city)`: the frame needs compositing, and the lots' ruin
/// codes changed so the city's sprites must be attached again.
pub fn sync(
    scene: &mut render::Scene,
    s: &mut sim::Sim,
    tr: &mut Tracker,
    objs: &[render::CityObj],
    sun_px: Option<(f32, f32)>,
) -> (bool, bool) {
    let mut redraw = false;
    // The viewer's copy of the map learns about the morph and the new water.
    if let Some(sy) = s.take_ending_sync() {
        scene.sync_morph(sy.neighbour, &sy.flipped, &sy.classes);
        for &(x, y) in &sy.flipped {
            scene.dirty_tile(x, y);
        }
        for &(x, y, _) in &sy.classes {
            scene.dirty_tile(x, y);
        }
        redraw = true;
    }
    let f = fx(s, sun_px);
    let on = f.is_some();
    if on {
        // The overlay (lava, ice, traces, foam) when the list changes.
        let kind = f.as_ref().map(|f| f.kind).unwrap_or(0);
        let last = if kind == tr.overlay_kind { tr.overlay_epoch } else { 0 };
        if let Some((epoch, tiles)) = s.ending_overlay_if_changed(last) {
            tr.overlay_epoch = epoch;
            tr.overlay_kind = kind;
            scene.set_ending_tiles(tiles);
        }
        tr.overlay_kind = kind;
        redraw |= scene.set_ending_marks(marks(s, objs));
    } else if scene.ending_on() {
        scene.set_ending_tiles(Vec::new());
        scene.set_ending_marks(Vec::new());
        tr.overlay_epoch = 0;
    }
    redraw |= scene.set_ending(f);
    let rebuild = s.ruin_epoch() != tr.ruin_epoch || s.lots.len() != tr.lots;
    (redraw, rebuild)
}

/// Mark that the city was rebuilt for the current ruin codes.
pub fn rebuilt(tr: &mut Tracker, s: &sim::Sim) {
    tr.ruin_epoch = s.ruin_epoch();
    tr.lots = s.lots.len();
}

/// Take out the converted residents (the AI ending's `lift == 255` marker) and
/// return where they are, to be drawn as drones.
pub fn split_drones(figs: &mut Vec<sim::people::Figure>) -> Vec<(f32, f32)> {
    let mut d = Vec::new();
    figs.retain(|f| {
        if f.lift == 255 {
            d.push((f.x, f.y));
            false
        } else {
            true
        }
    });
    d
}

/// The ending's say in the weather: the reddened sun's heat, smog, the ice
/// age's snow, ash, dust. Applied to the look the scene is about to get.
pub fn weather(look: &mut WeatherLook, s: &sim::Sim) {
    let Some(v) = s.ending_view() else { return };
    if v.phase == Phase::Done {
        return;
    }
    let u = v.u;
    let r = |x: f32, a: f32, b: f32| ((x - a) / (b - a)).clamp(0.0, 1.0);
    let mut dust = 0.0f32;
    let mut fog = 0.0f32;
    let mut cloud = 0.0f32;
    let mut snow = 0.0f32;
    let mut cover = 0.0f32;
    let mut storm = 0.0f32;
    match v.kind {
        Ending::RedGiant => {
            dust = 0.62 * r(u, 0.2, 1.4) * (1.0 - 0.5 * r(u, 2.4, 3.6));
            cloud = -1.0; // the clouds burn off
        }
        Ending::Smog => {
            dust = 0.78 * r(u, 0.1, 1.2) * (1.0 - 0.6 * r(u, 2.8, 4.0));
            fog = 0.35 * r(u, 0.3, 1.2);
        }
        Ending::IceAge => {
            cover = r(u, 0.15, 1.2) * (1.0 - r(u, 3.0, 3.9));
            snow = r(u, 0.85, 1.0) * (1.0 - r(u, 2.0, 2.3));
            cloud = 0.8 * r(u, 0.8, 1.0) * (1.0 - r(u, 2.0, 2.6));
            fog = 0.4 * snow;
            storm = 0.0;
        }
        Ending::Volcano => {
            dust = 0.6 * r(u, 1.0, 1.3) * (1.0 - 0.7 * r(u, 2.0, 3.2));
            cloud = 0.7 * r(u, 1.0, 1.3) * (1.0 - r(u, 2.0, 3.2));
        }
        Ending::Meteor => {
            let hit = r(u, 1.36, 1.5);
            dust = 0.45 * hit * (1.0 - r(u, 2.0, 3.4));
            cloud = 0.6 * hit * (1.0 - r(u, 2.0, 3.0));
        }
        Ending::FallingMoon => {
            let hit = r(u, 1.7, 1.8);
            dust = 0.55 * hit * (1.0 - r(u, 2.2, 3.6));
            cloud = 0.8 * hit * (1.0 - r(u, 2.2, 3.4));
        }
        Ending::Kaiju => {
            dust = 0.28 * r(u, 0.95, 1.4) * (1.0 - r(u, 2.0, 3.2));
        }
        Ending::Flood => {
            cloud = 0.85 * r(u, 0.5, 0.9) * (1.0 - r(u, 2.0, 3.0));
            storm = 0.3 * r(u, 0.9, 1.2) * (1.0 - r(u, 1.7, 2.2));
            fog = 0.2 * r(u, 0.9, 1.4) * (1.0 - r(u, 2.0, 3.0));
        }
        Ending::Sunset => {
            let gold = r(u, 0.1, 0.9) * (1.0 - r(u, 1.2, 1.6));
            let dusk = r(u, 1.2, 1.8) * (1.0 - r(u, 2.1, 2.5));
            dust = 0.34 * gold + 0.5 * dusk;
        }
        Ending::Aliens => {
            dust = 0.2 * r(u, 0.95, 1.3) * (1.0 - r(u, 2.0, 2.6));
        }
        Ending::AiTakeover => {
            fog = 0.1 * r(u, 1.0, 1.6) * (1.0 - r(u, 2.4, 3.0));
        }
        Ending::Decline => {}
    }
    look.dust = look.dust.max(dust);
    look.fog = look.fog.max(fog);
    if cloud < 0.0 {
        look.cloud = 0.0;
    } else {
        look.cloud = look.cloud.max(cloud);
    }
    look.snow = look.snow.max(snow);
    look.snow_cover = look.snow_cover.max(cover);
    look.storm = look.storm.max(storm);
}

/// A bucketed fingerprint of the ending's effect on the weather, so the
/// caller knows when to run [`weather`] again without doing it every frame.
pub fn weather_key(s: &sim::Sim) -> u32 {
    match s.ending_view() {
        Some(v) => (v.kind.id() as u32) << 16 ^ ((v.u * 100.0) as u32),
        None => 0,
    }
}
