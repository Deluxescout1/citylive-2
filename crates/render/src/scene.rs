//! The scene: terrain and buildings in a cached static layer with a per-pixel
//! depth plane, life and weather composited on top.
//!
//! Layer model:
//! * `sky`     - gradient, built once (becomes time-of-day driven in M5)
//! * `static`  - terrain + buildings, 1-bit alpha over the sky, plus a `u16`
//!   painter key per pixel (P4). Rebuilt only for dirty rectangles.
//! * per frame - copy sky, blit static, draw actors (depth-tested), draw weather.
//!
//! Static items are painted in diagonal order (`x + y`). The same order is kept
//! in per-diagonal buckets, so a dirty flush only visits the diagonals that can
//! overlap the damaged rectangle and still paints back-to-front.

use std::time::Instant;

mod spec;
mod civ;
mod probe;
pub use probe::Probe;
pub use spec::{BalloonNow, SideKind, SideNow};

use map::{Tile, TileMap, Viewport, generate, world_for_viewport};

use crate::actors::{DepthMode, Life, key_diagonal, static_key};
use crate::building::Building;
use crate::city::{CityObj, ObjKind, Stage};
use crate::framebuffer::{Framebuffer, Rect, TAG_BUILT, TAG_PROP};
use crate::horizon::Horizon;
use map::iso_consts::HALF_H;

use crate::iso::{Camera, level_px, tile_bounds};
use crate::light::{LightState, ShadowLut};
use crate::palette::{self, Color, TRANSPARENT, pal};
use crate::rain::Rain;
use crate::shadow::HeightField;
use crate::terrain;
use crate::testscene;

/// Empty land (D20) or land plus the test city.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SceneMode {
    /// The growth sim owns the map: terrain plus whatever it has built.
    Sim,
    /// Terrain only: what the real world starts as.
    Empty,
    /// Terrain with roads, buildings and actors, for looking at and measuring.
    #[default]
    Test,
}

#[derive(Clone, Copy, Debug)]
pub struct SceneConfig {
    pub width: usize,
    pub height: usize,
    pub seed: u64,
    pub mode: SceneMode,
    pub cars: usize,
    pub peds: usize,
    pub buildings: usize,
    pub rain: bool,
}

impl Default for SceneConfig {
    fn default() -> Self {
        Self {
            width: 960,
            height: 540,
            seed: 7,
            mode: SceneMode::Test,
            cars: crate::actors::DEFAULT_CARS,
            peds: crate::actors::DEFAULT_PEDS,
            buildings: 150,
            rain: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SceneStats {
    /// Cost of the last full static-layer rebuild.
    pub full_rebuild_ms: f64,
    /// B76 r2: the dearest slice of the last sliced rebuild, and how many
    /// frames it took (the wallpaper's no-hitch check).
    pub rebuild_worst_ms: f64,
    pub rebuild_slices: u32,
    /// Cost of the last dirty-rect flush.
    pub last_flush_ms: f64,
    pub last_flush_rects: usize,
    pub last_flush_area: i64,
    /// Items actually rasterised during the last flush (after culling).
    pub last_flush_draws: usize,
    /// Items the flush had to consider at all (candidate diagonals).
    pub last_flush_visited: usize,
    /// Rectangles before merging (the flush merges overlapping ones).
    pub last_flush_rects_raw: usize,
    /// City objects drawn from a pre-rendered kit sprite in the last rebuild.
    pub kit_draws: usize,
    /// City objects drawn with the engine's own art in the last rebuild
    /// (construction sites, parks — and, if this is ever non-zero for a
    /// finished building, a mix of the two art systems, which is a bug).
    pub engine_draws: usize,
}

#[derive(Clone, Copy, Debug)]
enum Item {
    Tile(i32, i32),
    Building(usize),
    /// B46d: a landform sprite (index into `Scene::landforms`).
    Landform(usize, i32),
}

/// Bytes the scene's own layers need at a given framebuffer, and where they go.
/// B34: the budget is checked by a test and reported by the bench, so a new
/// full-frame buffer cannot creep in unnoticed.
#[derive(Clone, Copy, Debug, Default)]
pub struct LayerBytes {
    /// The static layer: colour (4 B) + the per-pixel painter key (2 B).
    pub static_b: usize,
    /// The sky, only as tall as the band (B34).
    pub sky_b: usize,
    /// The frame the caller composites into (kept by the presenter).
    pub frame_b: usize,
    /// B60: the cast-shadow plane and the one a bake is crossfading towards
    /// (one byte per pixel each). The height field is tiny: a tile is 8×8
    /// samples of two `f32`.
    pub shadow_b: usize,
}

impl LayerBytes {
    pub fn total(&self) -> usize {
        self.static_b + self.sky_b + self.frame_b + self.shadow_b
    }
}

/// The layer budget for a framebuffer (D61: ≤ 200 MB at the owner's union, B34:
/// ≤ 170 MB). `sky_frac` is the sky band (D43).
pub fn layer_bytes(w: usize, h: usize, sky_frac: f32) -> LayerBytes {
    let px = w * h;
    let band = sky_pixels(h, sky_frac);
    LayerBytes {
        static_b: px * 6,
        sky_b: w * band * 4,
        frame_b: px * 4,
        shadow_b: px * 2,
    }
}

/// Pixels of sky at the top of the frame (D43).
pub fn sky_pixels(height: usize, frac: f32) -> usize {
    ((height as f32 * frac.clamp(0.0, 0.4)).round() as usize).min(height.saturating_sub(64))
}

fn build_sky(w: usize, h: usize) -> Framebuffer {
    let mut fb = Framebuffer::new(w, h);
    let denom = (h.max(2) - 1) as f32;
    for y in 0..h {
        let t = (y as f32 / denom).powf(0.85);
        let c = palette::mix(pal::SKY_TOP, pal::SKY_HORIZON, t);
        fb.fill_rect(0, y as i32, w as i32, 1, c);
    }
    fb
}

/// Fill any pixel the terrain left transparent with the colour of the last
/// opaque pixel to its left (or the one above it at the start of a row).
///
/// A tile's top face and the riser beside it meet at exact pixel centres, and on
/// a slope the two can leave a 1-px diagonal line of sky through a hillside.
/// Rather than chase every case in the rasteriser, the layer is sealed here: a
/// pixel copies a neighbouring terrain colour, so the seam is invisible and the
/// colour is always one that belongs to the ground beside it.
fn seal_seams(fb: &mut Framebuffer, r: Rect, key: u16) {
    let r = r.intersect(&fb.bounds());
    if r.is_empty() {
        return;
    }
    let w = fb.w;
    for y in r.y0..r.y1 {
        let mut last: Option<Color> = None;
        // Seed from the pixel above the row, if it is ground.
        if y > 0 {
            let above = fb.px()[(y - 1) as usize * w + r.x0 as usize];
            if above != TRANSPARENT {
                last = Some(above);
            }
        }
        for x in r.x0..r.x1 {
            let i = y as usize * w + x as usize;
            let c = fb.px()[i];
            if c != TRANSPARENT {
                last = Some(c);
                continue;
            }
            if let Some(lc) = last {
                fb.px_mut()[i] = lc;
                if let Some(d) = fb.depth_mut() {
                    d[i] = key;
                }
            }
        }
    }
}

/// B46d: the landforms' bodies throw the shadows (the real massif is smoothed
/// away on the drawing map, the sprite is what the eye sees).
fn stamp_landforms(f: &mut HeightField, lfs: &[crate::landform::Landform]) {
    for lf in lfs {
        let r = lf.site.radius + 1.0;
        let (cx, cy) = (lf.site.cx, lf.site.cy);
        f.stamp_fn(cx - r, cy - r, cx + r, cy + r, &|x, y| lf.top_at(x, y));
    }
}

/// Does this plot draw an open surface a figure may stand on, rather than a
/// solid building? (B65: a park, a churchyard, a civic square.)
fn open_ground(kind: ObjKind) -> bool {
    matches!(
        kind,
        ObjKind::Park
            | ObjKind::Cemetery
            | ObjKind::Landmark(crate::city::LandmarkKind::Plaza)
    )
}

/// B100: one painted piece of a city object. A lot is painted as one piece
/// when a single painter diagonal orders it correctly, and as several vertical
/// slices when it does not: a long or non-square lot is nearer the viewer at
/// one end than at the other, so each screen column of it gets the diagonal of
/// the nearest ground it stands on (`city_slices`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CitySlice {
    /// Index into the scene's city objects.
    pub idx: u32,
    /// The screen columns this piece paints, `x0..x1` (open ends are
    /// `i32::MIN` / `i32::MAX`, so a sprite that overhangs its lot is whole).
    pub x0: i32,
    pub x1: i32,
    /// The painter diagonal of this piece (also its depth key for actors).
    pub d: i32,
    /// The first piece of its object: per-object work (pane bookkeeping)
    /// happens once, here.
    pub first: bool,
    /// An overhang piece: columns past the lot's own ground, where the sprite
    /// leans out over a neighbour. Painted before every real piece of the
    /// same diagonal, so it never covers a neighbour's own wall.
    pub ext: bool,
}

/// B100: the painter pieces of a lot, left to right on screen.
///
/// Screen column band `k` (`ox + 32k .. ox + 32k + 32`) lies over two ground
/// diagonals of cells, `x - y = k` and `x - y = k + 1`. The nearest ground of
/// the lot in that band is the cell with the largest `x + y` on either, and
/// that is the diagonal the band is painted at. Two lots with disjoint
/// footprints are then ordered correctly in **every** band they share, which
/// one key per lot (the far corner's) cannot do for lots of different shapes.
/// Neighbouring bands with the same diagonal merge, so a square lot is
/// typically one piece.
pub fn city_slices(cam: &Camera, o: &CityObj, idx: u32) -> Vec<CitySlice> {
    let whole = |d: i32| {
        vec![CitySlice { idx, x0: i32::MIN, x1: i32::MAX, d, first: true, ext: false }]
    };
    // A/B switch for the before/after numbers and crops (B100).
    static OFF: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let off = *OFF.get_or_init(|| std::env::var("CITYLIVE_NO_SLICES").is_ok());
    if off || o.w <= 0 || o.h <= 0 {
        return whole(o.diagonal());
    }
    let (x0, y0, x1, y1) = (o.x, o.y, o.x + o.w, o.y + o.h);
    // Nearest cell diagonal on each cell column u = x - y.
    let (umin, umax) = (x0 - (y1 - 1), (x1 - 1) - y0);
    let dmax = |u: i32| -> Option<i32> {
        let xm = (x1 - 1).min(y1 - 1 + u);
        (xm >= x0.max(y0 + u)).then(|| 2 * xm - u)
    };
    let mut out: Vec<CitySlice> = Vec::new();
    for k in (umin - 1)..=umax {
        let d = match (dmax(k), dmax(k + 1)) {
            (Some(a), Some(b)) => a.max(b),
            (Some(a), None) | (None, Some(a)) => a,
            (None, None) => continue,
        };
        let left = cam.ox + 32 * k;
        match out.last_mut() {
            Some(last) if last.d == d => last.x1 = left + 32,
            _ => out.push(CitySlice { idx, x0: left, x1: left + 32, d, first: false, ext: false }),
        }
    }
    if out.is_empty() {
        return whole(o.diagonal());
    }
    if out.len() == 1 {
        return whole(out[0].d);
    }
    // The sprite may lean past the lot's diamond: those columns are painted
    // as separate overhang pieces (see `CitySlice::ext`).
    let (l, r) = (out[0].x0, out[out.len() - 1].x1);
    let (ld, rd) = (out[0].d, out[out.len() - 1].d);
    out[0].first = true;
    out.insert(0, CitySlice { idx, x0: i32::MIN, x1: l, d: ld, first: false, ext: true });
    out.push(CitySlice { idx, x0: r, x1: i32::MAX, d: rd, first: false, ext: true });
    out
}

/// Width of one screen-x band of the static index, in pixels.
const BAND: i32 = 64;
/// B76 r2: how many x bands of one painter diagonal a rebuild slice draws. A
/// whole diagonal of a dense downtown is 7 ms on its own; 24 bands (1 536 px)
/// keeps a slice inside the frame budget while drawing enough to amortise the
/// per-call setup.
const SLICE_BANDS: i32 = 12;

/// How far the world is painted into the sky band before it dissolves into it
/// (B42). Tall enough for the tallest tree on the back row, so nothing is
/// visibly cut; the dissolve itself is `Scene::fade_horizon`.
pub const HORIZON_FADE: i32 = 72;

/// B90: how far above the land line the world is kept at all. A tower on the
/// back row, a crane or a landmark rises well past the horizon and must keep
/// its whole roof: it is drawn over the sky and the far land, never cut by the
/// land line. The tallest thing the city builds is a ~40-storey tower (400 px)
/// plus its roof furniture; bare ground up there is still masked away
/// ([`Scene::mask_above_land`]), only things that stand up are kept.
pub const HORIZON_ROOM: i32 = 460;
/// The cloud height the "has the sun moved its shadows?" probe uses, in px
/// (B57). Any cloud's offset scales with its own `lift`; one representative
/// height answers "did every shadow shift?".
const CLOUD_OFFSET_PROBE: f32 = 150.0;

/// B61f: how far a fresh shadow bake may differ from the plane on screen and
/// still be taken without a repaint, in levels (255 = full shadow). The mean is
/// over every fourth pixel of the frame, so it counts the whole picture, not
/// just the shadowed ground: at 0.6 a bake moves the average pixel by 0.24 % of
/// the darkening, which cannot be seen. `SNAP_DRIFT` caps what a long run of
/// such bakes may accumulate before one of them crossfades (and repaints) as it
/// always did.
const SNAP_MEAN: f32 = 0.6;
const SNAP_DRIFT: f32 = 2.0;

/// Draw every static item whose bounds touch `clip`, restricted to diagonals
/// `d0..=d1` and the x bands that `bx0..=bx1` covers.
///
/// Candidates come from the index out of order (one item can sit in several x
/// bands), so they are sorted by item index first: item order is the painter
/// order, and repainting out of order would change the result.
#[allow(clippy::too_many_arguments)]
fn draw_items(
    map: &TileMap,
    landforms: &[crate::landform::Landform],
    buildings: &[Building],
    cam: &Camera,
    items: &[Item],
    keys: &[u16],
    bounds: &[Rect],
    index: &[Vec<Vec<u32>>],
    city: &[CityObj],
    city_diag: &[Vec<CitySlice>],
    kit: &[KitSprite],
    holiday: &crate::holiday::HolidayLook,
    market: bool,
    dress: crate::spectacle::Dress,
    fest_host: Option<usize>,
    kit_draws: &mut usize,
    engine_draws: &mut usize,
    glow: f32,
    hour: f32,
    water: &terrain::WaterLook,
    weather: &terrain::WeatherLook,
    season: &terrain::SeasonLook,
    cleared: &terrain::Cleared,
    biome: &crate::biome::Pair,
    plots: &[u16],
    claims: &[u8],
    derived: &[Option<map::Prop>],
    fixtures: &crate::rail::Fixtures,
    street: &StreetKey,
    tech: &crate::tech::TechKey,
    civic: &crate::civic::CivicStatic,
    fb: &mut Framebuffer,
    d0: i32,
    d1: i32,
    bx0: i32,
    bx1: i32,
    land_y: i32,
) -> (usize, usize) {
    // B53: the gameday host (stadium, else the biggest park), by index into `city`.
    let host = gameday_host(city);
    let clip = fb.clip();
    let mut drawn = 0usize;
    let mut visited = 0usize;
    let mut cand: Vec<u32> = Vec::new();
    // Diagonal by diagonal: the ground of a diagonal, then the buildings that
    // stand on it. That keeps hillside plots behind the hill in front of them.
    for d in d0.max(0)..=d1 {
        cand.clear();
        if let Some(row) = index.get(d as usize) {
            for bx in bx0.max(0)..=bx1 {
                if let Some(cell) = row.get(bx as usize) {
                    cand.extend_from_slice(cell);
                }
            }
        }
        visited += cand.len();
        cand.sort_unstable();
        cand.dedup();
        for &i in &cand {
            let i = i as usize;
            if !bounds[i].intersects(&clip) {
                continue;
            }
            fb.set_depth_key(keys[i]);
            // Ground takes the distance haze; anything standing on it does not
            // (B42b). `draw_tile` re-flags roads and props itself.
            fb.solid = 0;
            match items[i] {
                Item::Tile(x, y) => {
                    // B90: the layer reaches far above the land line so a tower
                    // keeps its roof, but bare ground, lanes and furniture up
                    // there are behind the horizon: only the old band of them
                    // is painted (it is masked and dissolved in the composite).
                    if bounds[i].y1 <= land_y - HORIZON_FADE {
                        continue;
                    }
                    terrain::draw_tile(
                        fb, cam, map, x, y, land_y, glow, water, weather, season, holiday, cleared,
                        biome, plots, claims, derived,
                    );
                    if !fixtures.is_empty() {
                        crate::rail::draw_fixture_tile(fb, cam, map, x, y, fixtures, glow);
                    }
                    // B41: the street furniture installed on this tile.
                    if let Some(&style) = street.paint.get(&(x, y)) {
                        crate::street::draw_paint(fb, cam, map, x, y, style);
                        // B83: the mouth of a junction is where a stop bar and a
                        // zebra belong (a path crossing gets its own zebra).
                        crate::street::draw_mouth(fb, cam, map, x, y);
                    }
                    // B59: a highway lane is painted as one.
                    if map.at(x, y).aux & map::AUX_HIGHWAY != 0 {
                        crate::street::draw_highway(fb, cam, map, x, y);
                    } else if map.at(x, y).aux & map::AUX_RAMP != 0 {
                        crate::street::draw_ramp(fb, cam, map, x, y);
                    }
                    if street.lamps.contains(&(x, y)) {
                        // B49/D72: the head the technology crews fitted here.
                        // B52: a blackout puts the district's lamps out.
                        let lamp_glow = if civic.is_dark(x, y) { 0.0 } else { glow };
                        crate::street::draw_lamp(fb, cam, map, x, y, lamp_glow, tech.lamp_level(x, y));
                    }
                    if let Some(&kind) = street.signs.get(&(x, y)) {
                        crate::street::draw_sign(fb, cam, map, x, y, kind);
                    }
                    // B49: the technology installed on this tile. The wires run
                    // to a neighbour that is already down, so a span is drawn
                    // once (the +x or +y pole only).
                    if tech.poles.contains(&(x, y)) {
                        crate::tech::draw_pole(fb, cam, map, x, y);
                        // B49b: the wires run to the next pole down the street
                        // (the crews set one every fourth tile), in +x and +y
                        // only, so each span is drawn once.
                        for (dx, dy) in [(1i32, 0i32), (0, 1)] {
                            let mut k = 1;
                            while k <= 6 {
                                let (nx, ny) = (x + dx * k, y + dy * k);
                                if !map.at(nx, ny).kind.is_lane() {
                                    break;
                                }
                                if tech.poles.contains(&(nx, ny)) {
                                    crate::tech::draw_wire(fb, cam, map, x, y, nx, ny, k);
                                    break;
                                }
                                k += 1;
                            }
                        }
                    }
                    if tech.buried.contains(&(x, y)) {
                        crate::tech::draw_cabinet(fb, cam, map, x, y);
                    }
                    if tech.towers.contains(&(x, y)) {
                        crate::tech::draw_tower(fb, cam, map, x, y);
                    }
                    if tech.cells.contains(&(x, y)) {
                        crate::tech::draw_small_cell(fb, cam, map, x, y);
                    }
                    if tech.farm.contains(&(x, y)) {
                        crate::tech::draw_farm(fb, cam, map, x, y);
                    }
                    if tech.turbines.contains(&(x, y)) {
                        // B49b: the mast and nacelle are static; the rotor turns
                        // every frame (see `draw_turbine_rotor`).
                        crate::tech::draw_turbine_mast(fb, cam, map, x, y);
                    }
                    if tech.chargers.contains(&(x, y)) {
                        crate::tech::draw_charger(fb, cam, map, x, y);
                    }
                }
                Item::Building(b) => {
                    fb.solid = TAG_BUILT;
                    buildings[b].draw_with_snow(fb, cam, weather.snow_cover);
                }
                Item::Landform(l, d) => {
                    // B46d: rock is ground: it takes the distance haze. One
                    // slice per painter diagonal, so a house in front of a flank
                    // still stands in front of it.
                    landforms[l].blit_slice(fb, cam, d);
                }
            }
            fb.solid = 0;
            drawn += 1;
        }
        if let Some(list) = city_diag.get(d as usize) {
            for &sl in list {
                let c = sl.idx as usize;
                let o = &city[c];
                let ob = obj_bounds(cam, o);
                let piece = Rect::new(ob.x0.max(sl.x0), ob.y0, ob.x1.min(sl.x1), ob.y1);
                if piece.is_empty() || !clip.intersects(&piece) {
                    continue;
                }

                // Something whose ground line is behind the horizon is not in
                // the world any more: skip it whole rather than cut it (B42b).
                let (_, foot) = cam.project(
                    o.x as f32,
                    (o.y + o.h) as f32,
                    o.level as i32 * crate::iso::HEIGHT_UNIT,
                );
                if (foot as i32) < land_y - 4 {
                    continue;
                }
                // B100: a sliced lot paints only its own screen columns here.
                fb.set_clip(Rect::new(clip.x0.max(sl.x0), clip.y0, clip.x1.min(sl.x1), clip.y1));
                // Same painter key as the ground: the depth mask hides actors
                // behind the house, and the house behind a hill.
                fb.set_depth_key(static_key(d, true));
                // Engine art keys itself: tell it this piece's diagonal, and
                // let only the first piece record windows (B100).
                fb.set_slice_diag(Some(d));
                let rec = fb.record_windows;
                fb.record_windows = rec && sl.first;
                // A building is solid: the horizon haze leaves it alone, and it
                // is hidden once it is past the horizon (B42b).
                fb.solid = TAG_BUILT;
                {
                    // `SNOW_LOT=x,y`: is this lot drawn by the pass, and with what
                    // snow? (B76 r3, a debug lever for a roof that stays bare.)
                    static WATCH: std::sync::OnceLock<Option<(i32, i32)>> = std::sync::OnceLock::new();
                    let watch = *WATCH.get_or_init(|| {
                        std::env::var("SNOW_LOT").ok().and_then(|v| {
                            let p: Vec<i32> = v.split(',').filter_map(|s| s.parse().ok()).collect();
                            (p.len() == 2).then(|| (p[0], p[1]))
                        })
                    });
                    if watch == Some((o.x, o.y)) {
                        eprintln!(
                            "lot drawn at {},{}+{}x{} kit {} snow {:.2} d{}",
                            o.x, o.y, o.w, o.h, o.kit, weather.snow_cover, d
                        );
                    }
                }
                let blitted = if o.kit != 0 {
                    if let Some(k) = kit.get(o.kit as usize - 1) {
                        let base = o.level as i32 * crate::iso::HEIGHT_UNIT;
                        // B60: no painted ground shadow here — the light casts
                        // the real one from the height field on every bake.
                        let (sx, sy) = cam.project(o.x as f32, o.y as f32, base);
                        // B76: snow lies on the surfaces the builder tagged —
                        // a heated roof (a factory, a block with a plant room)
                        // thaws first (r3).
                        fb.blit_sprite_flat_snow(
                            k,
                            sx as i32 - k.ox,
                            sy as i32 - k.oy,
                            weather.snow_cover,
                            crate::snow::heat_of(o),
                        );
                        // B78 (D84): the years on this building go on top of it:
                        // grime, moss, spray tags and murals, boards, shutters,
                        // scaffolding. Static art, so nothing per frame.
                        crate::wear::over(fb, o, k, sx as i32 - k.ox, sy as i32 - k.oy);
                        // B72: the sprite's window panes are switchable lights.
                        if sl.first && !k.panes.is_empty() {
                            fb.push_kit_panes(
                                crate::windows::obj_key(o.x, o.y),
                                &k.panes,
                                sx as i32 - k.ox,
                                sy as i32 - k.oy,
                                o.kit as u32 - 1,
                            );
                        }
                        true
                    } else {
                        false
                    }
                } else {
                    false
                };
                if !blitted {
                    // D66: the plot's own ground (park lawn, pond, yard) wears
                    // the world's land.
                    o.draw_in_land(fb, cam, glow, hour, weather.snow_cover, season, biome.at(map, o.x, o.y));
                    if sl.first {
                        *engine_draws += 1;
                    }
                } else if sl.first {
                    *kit_draws += 1;
                }
                // B76: the season on the built city — snow on a kit roof,
                // flowers on a spring windowsill, a summer café on the pavement.
                crate::seasons::building(fb, cam, o, season, hour, holiday);
                // B49: rooftop solar, fitted by a crew roof by roof. Drawn on
                // the roof of the lot whose anchor tile the sim marked, so it
                // appears the moment the crew reaches it and never floats off a
                // building that has moved.
                if let Some(&style) = tech.solar.get(&(o.x, o.y)) {
                    // A kit building knows its own roof (the array is fitted to
                    // the sprite); the engine's art reports its wall height.
                    match kit.get(o.kit as usize - 1) {
                        Some(k2) if blitted => {
                            let base = o.level as i32 * crate::iso::HEIGHT_UNIT;
                            let (sxp, syp) = cam.project(o.x as f32, o.y as f32, base);
                            crate::tech::draw_solar_on_sprite(fb, k2, sxp as i32, syp as i32, style);
                        }
                        _ => {
                            let top_px =
                                o.wall_height() + if o.kind.is_massing() { 3 } else { 7 };
                            crate::tech::draw_solar_roof(fb, cam, o, style, top_px);
                        }
                    }
                }
                // B52: the government district's flags, lamps and pillar box.
                crate::civic::civic_dressing(fb, cam, o, glow);
                // B36: the holiday dressing goes on top of either art system.
                if holiday.is_decorated() {
                    use crate::city::{LandmarkKind, ObjKind};
                    match o.kind {
                        ObjKind::Landmark(LandmarkKind::Plaza) => {
                            // The city's tree, in the middle of the square.
                            let base = o.level as i32 * crate::iso::HEIGHT_UNIT;
                            let (cx, cy) = (
                                o.x as f32 + o.w as f32 * 0.5,
                                o.y as f32 + o.h as f32 * 0.5,
                            );
                            crate::holiday::plaza_tree(fb, cam, cx, cy, o.level, holiday);
                            // Weekend market stalls round the fountain (B36).
                            if market {
                                for i in 0..6 {
                                    let a = i as f32 / 6.0 * std::f32::consts::TAU + 0.4;
                                    crate::holiday::market_stall(
                                        fb,
                                        cam,
                                        cx + a.cos() * 1.6,
                                        cy + a.sin() * 1.6,
                                        o.level,
                                        i as u8,
                                    );
                                }
                            }
                            let _ = base;
                        }
                        ObjKind::Landmark(LandmarkKind::Stadium) => {
                            crate::holiday::stadium_lights(
                                fb,
                                cam,
                                o.x,
                                o.y,
                                o.w,
                                o.h,
                                o.level,
                                glow,
                            );
                            // B53: the crowd at the stadium lot, and the banner.
                            tailgate_row(fb, cam, o, holiday);
                            crate::holiday::banner_sign(
                                fb,
                                cam,
                                o.x as f32 + o.w as f32 * 0.5,
                                o.y as f32 + o.h as f32 * 0.5,
                                o.level,
                                holiday,
                            );
                        }
                        // B53: no stadium in the city: the biggest park is the
                        // gameday host (tailgaters and the banner).
                        ObjKind::Park if host == Some(c as usize) => {
                            tailgate_row(fb, cam, o, holiday);
                            crate::holiday::banner_sign(
                                fb,
                                cam,
                                o.x as f32 + o.w as f32 * 0.5,
                                o.y as f32 + o.h as f32 * 0.5,
                                o.level,
                                holiday,
                            );
                        }
                        ObjKind::House
                        | ObjKind::Store
                        | ObjKind::Shop
                        | ObjKind::Townhouse
                        | ObjKind::Church
                        | ObjKind::Hall => {
                            crate::holiday::house_prop(
                                fb, cam, o.x, o.y, o.w, o.h, o.level, o.front, holiday, o.wall_height(),
                            );
                        }
                        _ => {}
                    }
                }
                // B62: the square dressed for a festival or the night market, and
                // the gameday bunting across it.
                if fest_host == Some(c as usize) {
                    let sq = crate::spectacle::Square {
                        x: o.x as f32 + 0.13,
                        y: o.y as f32 + 0.13,
                        w: o.w as f32 - 0.26,
                        h: o.h as f32 - 0.26,
                        level: o.level,
                    };
                    if dress.any() {
                        crate::spectacle::festival_static(fb, cam, &sq, dress);
                    } else if civic.rally {
                        // B52: the rally and election night.
                        crate::civic::rally_static(fb, cam, &sq, civic.slates);
                    } else if holiday.holiday.is_bills() {
                        crate::spectacle::gameday_static(fb, cam, &sq);
                    } else if holiday.holiday.has_banner() {
                        // A personal date: bunting, a banner and balloons in its colours.
                        crate::spectacle::banner_dress(fb, cam, &sq, [holiday.bulb(0), holiday.bulb(1)], true);
                    }
                }
                fb.set_clip(clip);
                fb.set_slice_diag(None);
                fb.record_windows = rec;
                drawn += 1;
            }
        }
    }
    (drawn, visited)
}

/// B71: a pitched roof as the builder drew it (exact, in tiles from the plot's
/// origin and pixels above the ground), so the shadow field casts from the real
/// eave and ridge instead of a guess. `form`: 1 gable (ridge along x), 2 hip,
/// 3 mansard (hip that reaches its ridge 0.22 tiles in from the wall).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RoofInfo {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    pub eave: f32,
    pub ridge: f32,
    pub form: u8,
}

/// B72: a window pane on a kit sprite: the glass pixels it owns, in the
/// sprite's own coordinates, and whether it burns warm or cool.
#[derive(Clone, Debug, Default)]
pub struct KitSpritePane {
    pub px: Vec<(i16, i16)>,
    /// Warm tungsten or cool LED.
    pub warm: bool,
}

/// A pre-rendered building from the buildkit catalogue (B33). The renderer never
/// depends on `buildkit`; the caller hands the pixels over in this shape.
#[derive(Clone, Debug, Default)]
pub struct KitSprite {
    pub w: usize,
    pub h: usize,
    /// Anchor: the sprite pixel that sits on the footprint's tile origin.
    pub ox: i32,
    pub oy: i32,
    /// B84: the pixels are **shared** with the builder's cache, not copied into
    /// the scene (`sprites_from` used to clone every one). One copy of the
    /// city's art in RAM instead of two: 83.6 + 83.4 MB on the day-60 bench
    /// world, 1.4 + 1.4 MB on the desktop-like fixture.
    pub px: std::sync::Arc<Vec<Color>>,
    /// B68: the tallest drawn pixel's true height above the ground (the
    /// builder's own depth plane), which the shadow field casts from. The
    /// anchor offset `oy` is not a height: it also holds the footprint's
    /// screen depth.
    pub top: u16,
    /// B68: where the walls stand, in tiles from the plot's origin corner:
    /// `[x0, y0, x1, y1]`. A kit building does not fill its plot (a 2x2 lot
    /// holds a ~1.15 x 1.05 house in its top corner), and the shadow has to
    /// start at the wall, not at the middle of the lot.
    pub foot: [f32; 4],
    /// B71: the builder's own roof (None for a flat roof or an unknown form).
    pub roof: Option<RoofInfo>,
    /// B72: the window panes whose light the sim switches. The sprite bakes
    /// them dark; the scene paints the lit ones.
    pub panes: Vec<KitSpritePane>,
    /// B76 r3: what surface each pixel is (`crate::snow`'s tags), two pixels
    /// to a byte. The builder stamps it while it draws, so snow lands on the
    /// roof plane it was drawn on rather than on the top rows of the sprite.
    /// Empty only for a sprite built outside the kit.
    pub surf: Vec<u8>,
}

impl KitSprite {
    /// B76 r3: the surface tag of a pixel in the sprite's own coordinates.
    #[inline]
    pub fn surf_at(&self, sx: usize, sy: usize) -> u8 {
        if sx >= self.w || sy >= self.h {
            return crate::snow::NONE;
        }
        crate::snow::get(&self.surf, sy * self.w + sx)
    }

    /// Read the wall footprint off the sprite's silhouette: the lowest pixel
    /// is the near corner `(x1, y1)`, and the left/right extremes are the
    /// west `(x0, y1)` and east `(x1, y0)` corners (screen u = (x-y)*32,
    /// v = (x+y)*16 from the anchor).
    pub fn footprint_of(px: &[Color], w: usize, h: usize, ox: i32, oy: i32) -> [f32; 4] {
        let (mut umin, mut umax) = (i32::MAX, i32::MIN);
        let (mut bot, mut sum, mut n) = (-1i32, 0i64, 0i64);
        for j in 0..h {
            for i in 0..w {
                if px[j * w + i] == TRANSPARENT {
                    continue;
                }
                let u = i as i32 - ox;
                umin = umin.min(u);
                umax = umax.max(u);
                if j as i32 > bot {
                    bot = j as i32;
                    sum = 0;
                    n = 0;
                }
                if j as i32 == bot {
                    sum += u as i64;
                    n += 1;
                }
            }
        }
        if n == 0 {
            return [0.0; 4];
        }
        let v = (bot - oy) as f32 / 16.0;
        let u = (sum as f32 / n as f32) / 32.0;
        let (x1, y1) = ((v + u) * 0.5, (v - u) * 0.5);
        let x0 = umin as f32 / 32.0 + y1;
        let y0 = x1 - umax as f32 / 32.0;
        [x0.max(0.0), y0.max(0.0), x1.max(x0 + 0.3), y1.max(y0 + 0.3)]
    }
}

/// B100: how far a sprite reaches past the ground of its own `lw` x `lh` lot,
/// in pixels: the worst of the left and right overshoot of the lot diamond's
/// width and how far below its two front edges any pixel is drawn. `0` means
/// the sprite stands inside its lot; the anchor `(ox, oy)` is the lot's
/// origin corner. `opaque(i)` says whether sprite pixel `i` (row-major) is
/// drawn.
pub fn lot_overhang(
    w: usize,
    h: usize,
    ox: i32,
    oy: i32,
    lw: i32,
    lh: i32,
    opaque: impl Fn(usize) -> bool,
) -> i32 {
    let (ul, ub, ur) = (-lh * 32, (lw - lh) * 32, lw * 32);
    let (vl, vb) = (lh * 16, (lw + lh) * 16);
    let mut worst = 0;
    for j in 0..h {
        let v = j as i32 - oy;
        for i in 0..w {
            if !opaque(j * w + i) {
                continue;
            }
            let u = i as i32 - ox;
            let mut over = (ul - u).max(u - ur + 1);
            let edge = if u <= ub { vl + (u - ul) / 2 } else { vb - (u - ub) / 2 };
            over = over.max(v - edge);
            worst = worst.max(over);
        }
    }
    worst.max(0)
}

/// Screen bounds of a city object (its plot plus the height of the building).
fn obj_bounds(cam: &Camera, o: &CityObj) -> Rect {
    let base = o.level as i32 * crate::iso::HEIGHT_UNIT;
    let mut r = Rect::new(i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for (x, y) in [
        (o.x as f32, o.y as f32),
        ((o.x + o.w) as f32, o.y as f32),
        ((o.x + o.w) as f32, (o.y + o.h) as f32),
        (o.x as f32, (o.y + o.h) as f32),
    ] {
        for z in [base, base + 48.max(o.wall_height() + 34)] {
            let p = cam.project(x, y, z);
            r.x0 = r.x0.min(p.0.floor() as i32);
            r.y0 = r.y0.min(p.1.floor() as i32);
            r.x1 = r.x1.max(p.0.ceil() as i32);
            r.y1 = r.y1.max(p.1.ceil() as i32);
        }
    }
    r.expanded(2)
}

/// Merge rectangles that overlap (or come within `slack` px of each other) so a
/// burst of dirty tiles costs one repaint per cluster instead of one each.
pub fn merge_rects(mut rects: Vec<Rect>, slack: i32) -> Vec<Rect> {
    loop {
        let mut merged = false;
        let mut out: Vec<Rect> = Vec::with_capacity(rects.len());
        'outer: for r in rects.drain(..) {
            for o in out.iter_mut() {
                if r.expanded(slack).intersects(o) {
                    *o = o.union(&r);
                    merged = true;
                    continue 'outer;
                }
            }
            out.push(r);
        }
        rects = out;
        if !merged {
            return rects;
        }
    }
}

#[inline]
fn band_of(x: i32) -> i32 {
    x.div_euclid(BAND)
}

#[inline]
fn cfg_bands(width: usize) -> usize {
    width / BAND as usize + 2
}

/// The installed street furniture, keyed by tile so the static pass is O(1)
/// per tile (B41).
#[derive(Clone, Debug, Default)]
pub struct StreetKey {
    pub paint: std::collections::BTreeMap<(i32, i32), u8>,
    pub lamps: std::collections::BTreeSet<(i32, i32)>,
    pub signs: std::collections::BTreeMap<(i32, i32), u8>,
    pub lights: std::collections::BTreeSet<(i32, i32)>,
}

pub struct Scene {
    pub cam: Camera,
    /// B46d: the map the static layer draws: the real map with each landform's
    /// footprint flattened (the sim and the shadows keep `map`).
    draw_map: TileMap,
    /// B46d: the composite landform sprites, baked for the light in force.
    landforms: Vec<crate::landform::Landform>,
    landform_ms: f64,
    pub viewport: Viewport,
    pub map: TileMap,
    pub buildings: Vec<Building>,
    pub life: Life,
    pub rain: Option<Rain>,
    pub stats: SceneStats,
    /// How actors are tested against the depth plane.
    pub depth_mode: DepthMode,
    seed: u64,
    /// The sky, only as tall as the sky band (B34): below it the static layer is
    /// opaque, so the rest of the frame never needed a sky.
    sky: Framebuffer,
    /// Sky band height in pixels (D43).
    sky_h: usize,
    /// B90: the highest row above the old painted band that anything standing
    /// (a tall building) can occupy; rows above it are known empty, so the
    /// composite does not scan them.
    above_top: i32,
    /// B42: what stands beyond the map's far edge (D64), chosen per seed.
    horizon: Horizon,
    /// The air colour the far ground fades into (the backdrop's own haze
    /// colour): what makes distance read at any hour.
    haze_air: Color,
    /// Per column, the colour the far ground fades into: the sky band's own
    /// bottom pixel, pulled half-way to `haze_air`. Sampled whenever the sky is
    /// re-baked, so the fade matches a treeline, farmland or a sea horizon, and
    /// goes dark at dusk instead of glowing pale.
    haze_row: Vec<Color>,
    /// The static layer with the ambient tint baked in, refreshed only for the
    /// rectangles a flush repainted (B13). B34 tried tinting during the
    /// composite instead — 40 MB smaller, but every frame's damage paid the
    /// per-pixel multiply, which cost more CPU than the memory was worth
    /// (measured: composite 1.0 -> 3.0 ms at 5 fps).
    tinted_static: Framebuffer,
    /// B50: the world's ending, if it is on, and what it draws with.
    ending: Option<crate::ending::Fx>,
    ending_key: u64,
    sky_clean: Option<Framebuffer>,
    ending_rects: Vec<Rect>,
    ending_gbb: Rect,
    ending_wbb: Rect,
    ending_slices: Vec<Rect>,
    ending_slow: u64,
    ending_clock: Option<f32>,
    ending_tiles: Vec<(i32, i32, f32)>,
    ending_marks: Vec<(f32, f32, u8, f32)>,
    ending_drones: Vec<(f32, f32)>,
    /// The ambient multiply baked into `tinted_static`.
    tint: [f32; 3],
    /// Per row of `tinted_static`: is the row fully opaque? (D71 — the
    /// composite copies those rows with one memcpy instead of scanning them
    /// for transparent pixels first, which was the biggest single cost of a
    /// full-frame composite: a rainy frame at 7040x1440.)
    static_opaque: Vec<bool>,
    /// The whole light model for this instant (D74): sun, moon, sky ambient,
    /// window glow, cloud. Everything that is lit reads it.
    light: LightState,
    /// Terrain + casters in tile space (D74): geometry only, so it survives
    /// every light change and is rebuilt only when the city moves.
    field: HeightField,
    /// Does the field no longer match the geometry?
    field_dirty: bool,
    /// Per-sample shadow coverage (0 = sun, 255 = shadow) and the frame-sized
    /// plane it is projected onto.
    shadow_mask: Vec<u8>,
    shadow: Vec<u8>,
    /// B57: the cloud field over the world and in the sky band. It owns its own
    /// sprites, so the cast geometry can stay bakes-and-crossfades while the
    /// weather moves every frame.
    clouds: crate::clouds::Clouds,
    /// The hour's cloud colour (B57), the multiply its shadows take, and its
    /// strength relative to the cast shadow's (0 = no cloud shadows at all).
    cloud_ink: crate::clouds::Ink,
    cloud_lut: Box<ShadowLut>,
    cloud_k: f32,
    /// The shadow offset last seen, so a sun that has moved is noticed (the
    /// shadows are drawn per frame, so nothing else has to be re-stamped).
    cloud_off: Option<(i32, i32)>,
    /// Scratch for the cloud damage rectangles (no allocation per frame).
    cloud_damage: Vec<Rect>,
    /// B57: do the clouds paint their shadows? (Off only for the before/after
    /// renders and the tests that need a cloudless ground.)
    cloud_shadows: bool,
    /// The plane the crossfade is heading for (B60: shadows never jump).
    /// B61f: only held while a fade is actually running — a bake whose picture
    /// has not visibly moved is adopted at once (see [`Self::adopt_shadow`]),
    /// which keeps the 9.7 MB second plane out of the resting footprint.
    shadow_to: Option<Vec<u8>>,
    /// 0..=1 crossfade progress; 1 means settled.
    shadow_fade: f32,
    /// B61f: how far (in levels, summed) the plane on screen has drifted from
    /// the bakes adopted without a repaint, so an unbroken run of invisible
    /// bakes cannot creep away from the picture.
    shadow_drift: f32,
    /// The shadow multiply for its current strength.
    shadow_lut: Box<ShadowLut>,
    /// The strength the baked shadows were made for (so a dimmer sun re-bakes).
    shadow_strength: f32,
    /// Cost of the last shadow bake, ms (reported by the bench).
    shadow_ms: f64,
    /// B67: a shadow bake in progress, a few rows a frame (the live wallpaper's
    /// way: the whole bake is 270 ms at 7040x1440, which froze every mover for
    /// eight frames once a minute).
    shadow_job: Option<ShadowJob>,
    /// B67: a light change arrived while a bake was running; bake again after.
    shadow_again: bool,
    /// B67: bake in slices (the wallpaper) instead of at once (tools, tests).
    async_shadow: bool,
    /// B67: a re-tint of the static layer in progress, a band of rows a frame.
    retint_job: Option<RetintJob>,
    /// B76 r2: a re-rasterise of the static layer in progress, a few diagonals
    /// a frame (the wallpaper's way: a whole pass is ~450 ms at 7040x1440,
    /// which was one visible stall on every snow step and every December night).
    rebuild_job: Option<RebuildJob>,
    /// B76 r2: street tiles the winter service has cleared, and how far.
    cleared: terrain::Cleared,
    /// 0..=1 night lighting for the windows (D25).
    glow: f32,
    /// How water is lit and animated (B21).
    water: terrain::WaterLook,
    /// Rain, snow, fog and wind (B27). `weather` alone is cheap: it is only read
    /// when a tile or the static layer is drawn.
    weather: terrain::WeatherLook,
    /// The season (B28): canopy colour, bareness, grass and field stage.
    season: terrain::SeasonLook,
    /// Which land the world is (D66): the ground, water and vegetation tables,
    /// plus the neighbour blending in at one edge.
    biome: crate::biome::Pair,
    /// B41: the street furniture installed so far — paint, lamps, signs and
    /// traffic lights, keyed by tile for the static pass.
    street: StreetKey,
    /// B49: what the technology crews have installed, keyed by tile.
    tech: crate::tech::TechKey,
    /// The crews at work, drawn per frame over the road.
    street_crews: Vec<(i32, i32, u8)>,
    /// B49: delivery drones, in tile space, drawn per frame (near-future days).
    drones: Vec<(f32, f32)>,
    /// B49b: the wind farm — the masts are static, these rotors turn per frame.
    turbines: Vec<(i32, i32)>,
    /// B49b: the technology crews, drawn per frame with the depth plane.
    tech_crews: Vec<(i32, i32, u8)>,
    /// B52: the civic picture (static part baked, dynamic part per composite).
    civic: crate::civic::CivicLook,
    /// The boxes the civic overlay painted last frame (for damage).
    civic_drawn: Vec<Rect>,
    /// The civic layer's own animation clock, in seconds.
    civic_t: f32,
    /// Last eighth-second the chimney plumes were damaged for.
    civic_stack_q: i64,
    /// B66: the smog veil's thickness bucket (0 = no smog), so the land band is
    /// only repainted when the veil changes.
    civic_haze: i32,
    /// B39: the settler party, the camp and the felled logs, drawn per frame
    /// (they move) on top of the static layer.
    settlers: Vec<(f32, f32, u8, f32)>,
    /// B40: trees on their way down — (x, y, fraction, dir, size, variant).
    falling: Vec<(i32, i32, f32, i8, u8, u8)>,
    camp: Option<(i32, i32, Vec<(i32, i32)>, Option<(i32, i32)>)>,
    logs: Vec<(i32, i32, u8)>,
    /// The holiday the city is dressed for (B36), and the fireworks clock.
    holiday: crate::holiday::HolidayLook,
    holiday_t: f32,
    /// Is today a market day (a weekend, by day)? Set by the caller, which owns
    /// the calendar (B36).
    market: bool,
    /// Is a parade running (a holiday, late morning), and where along the
    /// avenue it has got to (0..1 over its length).
    parade: Option<f32>,
    /// B62: festivals, balloons, rainbow, bolts, side-monitor moments.
    spec: spec::SpecState,
    /// The bucket tuple last baked into the static layer, so a weather change
    /// re-rasterises the ground once per visible step, not per frame.
    weather_baked: (i32, i32, i32, i32),
    /// The same for the season (B28).
    season_baked: (i32, i32, i32, i32, i32),
    /// Villagers to draw this frame (D45): tile position + walk frame.
    walkers: Vec<(f32, f32, u8)>,
    /// The residents' layer (B43/D65): everyone doing something.
    people: Vec<crate::people::Figure>,
    /// Scratch for `set_people` (D71): the previous frame's list, recycled so a
    /// per-frame update allocates nothing.
    people_buf: Vec<crate::people::Figure>,
    /// Each resident's screen position, computed once per frame (B61c): a
    /// damage-limited composite draws the people once per damaged rectangle and
    /// re-projecting every figure for every rectangle was most of a rush-hour
    /// frame.
    people_px: Vec<(i32, i32)>,
    /// Scratch for the same.
    people_px_buf: Vec<(i32, i32)>,
    /// B75: the frame's figures in painter order (by screen y), so the band
    /// index and a straight list walk draw them in the same order.
    people_sorted: Vec<(i32, i32, crate::people::Figure)>,
    /// Vehicles' and movers' screen positions, cached beside their list (B61d):
    /// `draw_moving_shadows` runs once per damaged rectangle, and projecting
    /// every vehicle for every rectangle was 3 ms of a rush-hour frame.
    traffic_px: Vec<(i32, i32)>,
    movers_px: Vec<(i32, i32)>,
    /// Scratch for the "did it move" comparison, so it allocates nothing.
    traffic_px_buf: Vec<(i32, i32)>,
    movers_px_buf: Vec<(i32, i32)>,
    /// B61e: the screen-row index of each per-frame list, so a damaged
    /// rectangle visits only its own rows instead of the whole list.
    people_bands: crate::bands::Bands,
    traffic_bands: crate::bands::Bands,
    parked_px: Vec<(i32, i32)>,
    parked_bands: crate::bands::Bands,
    /// B101: the same cars bucketed by their ground (z = 0) screen row, which
    /// is what the shadow pass culls on.
    parked_shadow_bands: crate::bands::Bands,
    mover_bands: crate::bands::Bands,
    /// The animated water: tile, screen position, and its row index (static, so
    /// it is rebuilt with the static layer rather than per frame).
    water_px: Vec<(i32, i32, i32, i32)>,
    water_bands: crate::bands::Bands,
    /// Street lamps and their screen rows (static, like the water).
    lamp_px: Vec<(i32, i32, i32, i32)>,
    lamp_bands: crate::bands::Bands,
    /// B61e: did a moving layer (residents, traffic, movers) change this frame?
    /// The presenter can then skip the pixel hash: the scene already knows the
    /// rectangle changed, which is what the hash was there to find out.
    moving_changed: bool,
    /// Materials at construction sites (B43): x, y, kind.
    sites: Vec<(f32, f32, u8)>,
    /// B43c: cranes, diggers, trucks and site facilities, and the work clock
    /// that drives them (it stops when the crews go home).
    rigs: Vec<crate::rigs::Rig>,
    rig_clock: f64,
    rig_last_t: f32,
    rig_work: bool,
    /// Per tile: the painter key of the city object standing on it (B43). A
    /// figure on a park or a construction pad must beat the plot's own key.
    city_key: Vec<u16>,
    /// B74: per-tile claim bits for draw-time cleaning and planting.
    claims: Vec<u8>,
    /// B74: the derived garden/street trees, per tile (drawn over the terrain).
    derived: Vec<Option<map::Prop>>,
    /// Vehicles on the streets this frame (B29).
    traffic: Vec<crate::traffic::Body>,
    /// B59: cars standing in their bays (a car park, a forecourt, a driveway).
    parked: Vec<crate::traffic::Body>,
    /// Rail/station/port fixtures and the movers (B31).
    fixtures: crate::rail::Fixtures,
    movers: Vec<crate::rail::Body>,
    /// B101: the boats' distance-haze rows (empty when no boat is on screen),
    /// built when the movers are set instead of once per damaged rectangle.
    mover_haze: Vec<f32>,
    mover_sprites: crate::rail::MoverRenderer,
    /// What changed since the last composite, in framebuffer space (B30). `None`
    /// (via `full_pending`) means "redraw everything".
    pending: Vec<Rect>,
    /// B87: what the water overlay painted at the last tick, per entry of
    /// `water_px`. The next tick paints nothing new and damages only the
    /// boxes of strokes that are not the same as before. Empty = unknown (the
    /// next tick damages the whole visible water, as before B87).
    water_prims: Vec<terrain::TilePrims>,
    /// The boxes of the strokes that moved, merged onto a grid at `take_pending`.
    water_pending: Vec<Rect>,
    /// The haze rows (`strong`) the strokes were taken with.
    water_haze: i32,
    /// B69b: the sway layer's damage. Many small boxes (a crown's moved rows),
    /// coalesced on a fine grid in `take_pending`, so a breeze costs the pixels
    /// that moved and not a 64x60 cell per tree.
    tree_pending: Vec<Rect>,
    full_pending: bool,
    traffic_sprites: crate::traffic::TrafficRenderer,
    /// Residents at true scale (B43): pre-rasterised poses, retinted on a
    /// palette change like the peds.
    people_sprites: crate::people::PeopleRenderer,
    /// B61b: the weather's static contribution — whole-frame washes and the
    /// drifting cloud-shadow blobs. Applied inside `composite_rect`, so it is
    /// re-done only where the scene repaints (and once when the weather
    /// changes), instead of over the whole frame every frame.
    ambient: crate::weather::Ambient,

    /// Water tiles currently in the item list (kept live for `water_live`).
    water_count: usize,
    /// The same tiles as a plain list (D71): the animated composite walks this
    /// instead of every item in the viewport, which was the hot path (perf:
    /// `map.at` + `Item` matching, 13.7 % of a busy frame).
    water_tiles: Vec<(i32, i32)>,
    /// B92: open sea tiles per 64-px screen column, so the sun's glitter path
    /// can slide to the sea when its own column is under land.
    sea_cols: Vec<u16>,
    /// B92: the smooth coast of each water tile in `water_px` order.
    water_coast: Vec<Option<terrain::Coast>>,
    /// B101: the land look that owns each water tile (same order).
    water_ground: Vec<&'static crate::biome::Ground>,
    /// Scratch for `animate_water` (B61): the visible water tiles are collected
    /// here and reused, so a per-frame water tick allocates nothing.
    water_scratch: Vec<(i32, i32)>,
    /// B64: the water night step the static layer was last rasterised with.
    water_baked_night: u8,
    /// B69: the trees that sway with the wind (drawn over the static layer).
    trees: crate::sway::TreeLayer,
    /// B69: are trees captured for sway (true) or painted into the static
    /// layer as before (false: only for A/B comparisons and tests)?
    sway_on: bool,
    static_layer: Framebuffer,
    /// B72 (D78): every building's windows and which of them are lit.
    windows: crate::windows::Field,
    /// B72: the latest lights plan from the sim, keyed by lot anchor. Applied to
    /// the field when the city changes too, so a plan that arrives before the
    /// buildings do is not lost.
    window_plan: Vec<(i32, i32, u64)>,
    items: Vec<Item>,
    item_keys: Vec<u16>,
    item_bounds: Vec<Rect>,
    /// Static index: `index[d][band]` = item indices on painter diagonal `d`
    /// whose bounds touch that 64 px screen-x band.
    index: Vec<Vec<Vec<u32>>>,
    /// How far above its base line the tallest item can reach, in pixels (the
    /// flush needs it to know which diagonals can touch a rectangle).
    reach_px: i32,
    /// Per tile: does it touch the framebuffer at all? (B3/Q2 — later the sim
    /// skips invisible tiles.)
    visible: Vec<bool>,
    /// City objects (plots, civic buildings), indexed by painter diagonal.
    city: Vec<CityObj>,
    /// B73/D80: the nightlife plan (signs, crowns, beacons) for this city, with
    /// its screen positions. Rebuilt only when the city changes.
    night: crate::nightlife::Plan,
    /// B73r2: the sim's own town facts (D80 gating by population and era).
    town_stats: crate::nightlife::TownStats,
    /// The nightlife's animation clock and the 1/ANIM_HZ slot it was damaged for.
    sign_clock: f64,
    sign_slot: i64,
    /// Scratch for the animated panels' damage (no allocation per tick).
    sign_rects: Vec<Rect>,
    city_diag: Vec<Vec<CitySlice>>,
    /// Pre-rendered building sprites (B33 buildkit), indexed by `CityObj::kit - 1`.
    kit: Vec<KitSprite>,
    dirty: Vec<Rect>,
}

impl Scene {
    /// No sky band: the world fills the frame (the historical behaviour).
    pub fn new(cfg: SceneConfig) -> Self {
        Self::new_with_map(cfg, None, 0.0)
    }

    /// With a sky band at the top (D43).
    pub fn new_with_sky(cfg: SceneConfig, sky_frac: f32) -> Self {
        Self::new_with_map(cfg, None, sky_frac)
    }

    fn new_with_map(cfg: SceneConfig, supplied: Option<TileMap>, sky_frac: f32) -> Self {
        let scene = Self::new_with_map_deferred(cfg, supplied, sky_frac, (0, 0));
        let mut scene = scene;
        scene.rebuild_static();
        scene.retint_static();
        scene
    }

    fn new_with_map_deferred(
        cfg: SceneConfig,
        supplied: Option<TileMap>,
        sky_frac: f32,
        shift: (i32, i32),
    ) -> Self {
        let sky_h = sky_pixels(cfg.height, sky_frac);
        let land_h = cfg.height.saturating_sub(sky_h).max(64);
        // The world is built for the *land* band only (P3: nothing simulated that
        // cannot be seen); the camera is then pushed down by the sky band.
        let vp = world_for_viewport(cfg.width as i32, land_h as i32);
        let mut cam = Camera::for_viewport(&vp);
        cam.oy += sky_h as i32;
        // A still of a place away from the wallpaper's strip (tools): the
        // camera is moved *before* anything is baked, so nothing is stale.
        cam.ox += shift.0;
        cam.oy += shift.1;
        let mut map = supplied.unwrap_or_else(|| generate(cfg.seed, vp.w, vp.h));
        // D66: what stands beyond the map's far edge is the land's own horizon,
        // and the ground/vegetation tables come from the same place.
        let horizon = Horizon::for_biome(map.biome, cfg.seed, map.ocean.is_some());
        let biome = crate::biome::Pair::of(&map);

        let (buildings, life) = match cfg.mode {
            SceneMode::Empty | SceneMode::Sim => (Vec::new(), Life::empty(cfg.seed)),
            SceneMode::Test => {
                let ts =
                    testscene::build(&mut map, &cam, cfg.seed, cfg.buildings, cfg.cars, cfg.peds);
                (ts.buildings, ts.life)
            }
        };

        // B46d: the big landforms are composite sprites on a flattened footprint.
        let (sites, hills) = crate::landform::place(&map);
        let mut draw_map = map.clone();
        crate::landform::flatten(&mut draw_map, &sites, &hills);
        let sun0 = crate::landform::Sun::default_day();
        let landforms: Vec<crate::landform::Landform> = sites
            .iter()
            .map(|&site| crate::landform::Landform::bake(site, &sun0))
            .collect();

        let mut items: Vec<(u16, Item)> =
            Vec::with_capacity((map.w * map.h) as usize + buildings.len());
        for y in 0..map.h {
            for x in 0..map.w {
                items.push((static_key(x + y, false), Item::Tile(x, y)));
            }
        }
        for (i, b) in buildings.iter().enumerate() {
            let d = b.tx + b.fw - 1 + b.ty + b.fh - 1;
            items.push((static_key(d, true), Item::Building(i)));
        }
        for (i, lf) in landforms.iter().enumerate() {
            for d in lf.d_min..=lf.d_max {
                items.push((static_key(d, true), Item::Landform(i, d)));
            }
        }
        items.sort_by_key(|(k, _)| *k);

        let keys: Vec<u16> = items.iter().map(|(k, _)| *k).collect();
        let items_only: Vec<Item> = items.into_iter().map(|(_, it)| it).collect();

        let mut item_bounds = Vec::with_capacity(items_only.len());
        let mut reach_px = HALF_H;
        for (i, item) in items_only.iter().enumerate() {
            let b = match *item {
                Item::Tile(x, y) => {
                    let t = map.at(x, y);
                    let mut r = tile_bounds(&cam, x, y, t.corner.map(|c| c as f32));
                    let rise = terrain::prop_rise(&t);
                    if rise > 0 {
                        r.y0 -= rise;
                    }
                    // 1 px of slack: water bank lines and shade edges can land
                    // just outside the diamond; 6 more above for B70's smoothed
                    // corners (at most half a level up).
                    r.y0 -= 6;
                    r.expanded(1)
                }
                Item::Building(bi) => buildings[bi].bounds,
                Item::Landform(li, _) => landforms[li].bounds(&cam),
            };
            // Base line of a diagonal d is `oy + d * HALF_H`.
            let base_y = cam.oy + key_diagonal(keys[i]) * HALF_H;
            reach_px = reach_px.max(base_y - b.y0 + HALF_H);
            item_bounds.push(b);
        }

        // Which tiles touch the framebuffer at all (B3/Q2). The list is
        // row-major (`y * w + x`) because that is how `is_visible` indexes it —
        // the item list itself is in painter (diagonal) order, so it cannot be
        // reused for this.
        let frame_bounds = Rect::from_size(cfg.width, cfg.height);
        let mut visible = vec![false; (map.w * map.h) as usize];
        for (i, item) in items_only.iter().enumerate() {
            if let Item::Tile(x, y) = *item {
                visible[(y * map.w + x) as usize] = item_bounds[i].intersects(&frame_bounds);
            }
        }

        let diagonals = (vp.w + vp.h + 2) as usize;
        let bands = (cfg.width / BAND as usize) + 2;
        let mut index: Vec<Vec<Vec<u32>>> = vec![vec![Vec::new(); bands]; diagonals];
        for (i, k) in keys.iter().enumerate() {
            let d = key_diagonal(*k) as usize;
            if d >= diagonals {
                continue;
            }
            let b = &item_bounds[i];
            let lo = band_of(b.x0).clamp(0, bands as i32 - 1);
            let hi = band_of(b.x1).clamp(0, bands as i32 - 1);
            for band in lo..=hi {
                index[d][band as usize].push(i as u32);
            }
        }

        let rain = cfg
            .rain
            .then(|| Rain::new(cfg.width, cfg.height, cfg.seed, crate::rain::DEFAULT_DROPS));

        let field = {
            let mut f = HeightField::new(&draw_map);
            stamp_landforms(&mut f, &landforms);
            f
        };
        // B57: the cloud field knows the frame and the world it hangs over.
        let mut clouds = crate::clouds::Clouds::new(cfg.seed);
        clouds.set_bounds(cfg.width, cfg.height, sky_h, &map);

        // B61f: the city key is per *tile* (B43). Allocating the frame's
        // 10.1 Mpx here only to replace it with 26 KB at the first
        // `install_city` put a 20 MB spike in the startup peak.
        let tiles = (map.w * map.h).max(0) as usize;
        let mut scene = Self {
            cam,
            draw_map,
            landforms,
            landform_ms: 0.0,
            viewport: vp,
            map,
            buildings,
            life,
            rain,
            stats: SceneStats::default(),
            depth_mode: DepthMode::PerPixel,
            seed: cfg.seed,
            sky: build_sky(cfg.width, sky_h),
            tinted_static: Framebuffer::new(cfg.width, cfg.height),
            ending: None,
            ending_key: 0,
            sky_clean: None,
            ending_rects: Vec::new(),
            ending_gbb: Rect::EMPTY,
            ending_wbb: Rect::EMPTY,
            ending_slices: Vec::new(),
            ending_slow: 0,
            ending_clock: None,
            ending_tiles: Vec::new(),
            ending_marks: Vec::new(),
            ending_drones: Vec::new(),
            sky_h,
            above_top: (sky_h as i32 - HORIZON_FADE - 160).max(0),
            horizon,
            haze_air: 0xFFFF_FFFF,
            haze_row: Vec::new(),
            tint: [1.0, 1.0, 1.0],
            static_opaque: Vec::new(),
            light: LightState::default(),
            field,
            field_dirty: true,
            shadow_mask: Vec::new(),
            shadow: vec![0; cfg.width * cfg.height],
            shadow_to: None,
            shadow_drift: 0.0,
            shadow_fade: 1.0,
            shadow_lut: ShadowLut::none(),
            shadow_strength: 0.0,
            shadow_ms: 0.0,
            shadow_job: None,
            shadow_again: false,
            async_shadow: false,
            retint_job: None,
            rebuild_job: None,
            cleared: terrain::Cleared::default(),
            clouds,
            cloud_ink: crate::clouds::Ink::default(),
            cloud_lut: ShadowLut::none(),
            cloud_k: 0.0,
            cloud_off: None,
            cloud_damage: Vec::new(),
            cloud_shadows: true,
            glow: 0.0,
            water: terrain::WaterLook::default(),
            weather: terrain::WeatherLook::default(),
            season: terrain::SeasonLook::default(),
            biome,
            street: StreetKey::default(),
            tech: crate::tech::TechKey::default(),
            street_crews: Vec::new(),
            drones: Vec::new(),
            turbines: Vec::new(),
            tech_crews: Vec::new(),
            civic: crate::civic::CivicLook::default(),
            civic_drawn: Vec::new(),
            civic_t: 0.0,
            civic_stack_q: i64::MIN,
            civic_haze: 0,
            settlers: Vec::new(),
            falling: Vec::new(),
            camp: None,
            logs: Vec::new(),
            holiday: crate::holiday::HolidayLook::default(),
            holiday_t: 0.0,
            market: false,
            parade: None,
            spec: spec::SpecState::default(),
            weather_baked: (-1, -1, -1, -1),
            season_baked: (-1, -1, -1, -1, -1),
            walkers: Vec::new(),
            people: Vec::new(),
            people_buf: Vec::new(),
            people_px: Vec::new(),
            people_px_buf: Vec::new(),
            people_sorted: Vec::new(),
            traffic_px: Vec::new(),
            movers_px: Vec::new(),
            traffic_px_buf: Vec::new(),
            movers_px_buf: Vec::new(),
            people_bands: crate::bands::Bands::default(),
            traffic_bands: crate::bands::Bands::default(),
            parked_px: Vec::new(),
            parked_bands: crate::bands::Bands::default(),
            parked_shadow_bands: crate::bands::Bands::default(),
            mover_bands: crate::bands::Bands::default(),
            water_px: Vec::new(),
            water_bands: crate::bands::Bands::default(),
            lamp_px: Vec::new(),
            lamp_bands: crate::bands::Bands::default(),
            moving_changed: false,
            sites: Vec::new(),
            rigs: Vec::new(),
            rig_clock: 0.0,
            rig_last_t: f32::NAN,
            rig_work: false,
            city_key: vec![0; tiles],
            claims: Vec::new(),
            derived: Vec::new(),
            traffic: Vec::new(),
            parked: Vec::new(),
            fixtures: crate::rail::Fixtures::default(),
            movers: Vec::new(),
            mover_haze: Vec::new(),
            mover_sprites: crate::rail::MoverRenderer::new(),
            people_sprites: crate::people::PeopleRenderer::new(),
            pending: Vec::new(),
            water_prims: Vec::new(),
            water_pending: Vec::new(),
            water_haze: 0,
            tree_pending: Vec::new(),
            full_pending: true,
            traffic_sprites: crate::traffic::TrafficRenderer::new(),
            ambient: crate::weather::Ambient::default(),

            water_count: 0,
            water_tiles: Vec::new(),
            sea_cols: Vec::new(),
            water_coast: Vec::new(),
            water_ground: Vec::new(),
            water_scratch: Vec::new(),
            water_baked_night: 0,
            trees: crate::sway::TreeLayer::default(),
            sway_on: true,
            static_layer: Framebuffer::with_depth(cfg.width, cfg.height),
            windows: Default::default(),
            window_plan: Vec::new(),
            items: items_only,
            item_keys: keys,
            item_bounds,
            index,
            reach_px,
            visible,
            city: Vec::new(),
            city_diag: vec![Vec::new(); (vp.w + vp.h + 2) as usize],
            night: crate::nightlife::Plan::default(),
            town_stats: crate::nightlife::TownStats::default(),
            sign_clock: 0.0,
            sign_slot: -1,
            sign_rects: Vec::new(),
            kit: Vec::new(),
            dirty: Vec::new(),
        };
        // B42d: the per-column horizon colour exists from the first frame (the
        // far-floor fill and the haze both read it), not only after a sky bake.
        scene.resample_haze();
        scene.refresh_soft();
        scene
    }

    /// B70: recompute the smooth-ground layer of the drawing map from the tiles
    /// and the lots standing on them. Returns the tiles whose look changed.
    fn refresh_soft(&mut self) -> Vec<(i32, i32)> {
        let (w, h) = (self.draw_map.w, self.draw_map.h);
        let mut occ = vec![false; (w * h) as usize];
        let mut mark = |x0: i32, y0: i32, fw: i32, fh: i32| {
            // One tile of margin: a wall, a yard and a kerb want flat ground.
            for y in (y0 - 1).max(0)..(y0 + fh + 1).min(h) {
                for x in (x0 - 1).max(0)..(x0 + fw + 1).min(w) {
                    occ[(y * w + x) as usize] = true;
                }
            }
        };
        for b in &self.buildings {
            mark(b.tx, b.ty, b.fw, b.fh);
        }
        for o in &self.city {
            mark(o.x, o.y, o.w, o.h);
        }
        crate::softground::compute(&mut self.draw_map, &|x, y| occ[(y * w + x) as usize])
    }

    /// Which land this world is (D66).
    pub fn biome(&self) -> map::Biome {
        self.map.biome
    }

    /// The land's look tables (D66): ground, water, vegetation, horizon.
    pub fn ground(&self) -> &'static crate::biome::Ground {
        self.biome.host
    }

    /// The projection in use: a tool that has to crop around a *place* in the
    /// world (a lot, a shore, a landmark) needs the same camera the frame was
    /// drawn with.
    pub fn camera(&self) -> &crate::iso::Camera {
        &self.cam
    }

    pub fn seed(&self) -> u64 {
        self.seed
    }

    pub fn width(&self) -> usize {
        self.static_layer.w
    }

    /// B61f: what this scene holds, biggest first, so a memory reading can name
    /// the buffers instead of guessing (`CITYLIVE_MEM=1` prints it; see
    /// `WallpaperState::print_mem_report`). Byte counts, not capacities of the
    /// nested index structs.
    pub fn mem_report(&self) -> Vec<(&'static str, usize)> {
        fn v<T>(x: &Vec<T>) -> usize {
            x.capacity() * std::mem::size_of::<T>()
        }
        let mut out: Vec<(&'static str, usize)> = vec![
            (
                "static layer (colour)",
                self.static_layer.colour_bytes(),
            ),
            ("static layer (depth)", self.static_layer.depth_bytes()),
            ("tinted static (copy)", self.tinted_static.bytes()),
            (
                "draw map (tiles)",
                self.draw_map.tiles.capacity() * std::mem::size_of::<map::Tile>(),
            ),
            ("window field (faces + panes)", self.windows.bytes()),
            ("sky band", self.sky.bytes()),
            ("cast-shadow plane", v(&self.shadow)),
            (
                "cast-shadow crossfade target",
                self.shadow_to.as_ref().map_or(0, |p| p.capacity()),
            ),
            ("cast-shadow mask", v(&self.shadow_mask)),
            (
                "height field",
                (self.field.ground.capacity()
                    + self.field.occl.capacity()
                    + self.field.lo.capacity()) * 2
                    + self.field.half.capacity(),
            ),
            ("city key (per tile)", v(&self.city_key)),
            ("item list + keys + bounds", v(&self.items) + v(&self.item_keys) + v(&self.item_bounds)),
            (
                "item band index",
                self.index.capacity() * std::mem::size_of::<Vec<Vec<u32>>>()
                    + self
                        .index
                        .iter()
                        .flat_map(|b| b.iter())
                        .map(|x| x.capacity() * 4)
                        .sum::<usize>(),
            ),
            ("tree sway sprites", self.trees.bytes()),
            (
                "nightlife",
                self.night.spot_count() * std::mem::size_of::<crate::nightlife::Spot>()
                    + self.night.beacons.len() * std::mem::size_of::<crate::nightlife::Beacon>()
                    + self.night.vents.len() * std::mem::size_of::<crate::nightlife::Vent>()
                    + self.night.spot_count()
                        * (std::mem::size_of::<(i32, i32)>() + std::mem::size_of::<Rect>()),
            ),
            (
                "kit sprites (pixels shared with the cache)",
                self.kit.capacity() * std::mem::size_of::<KitSprite>()
                    + self.kit.iter().map(|k| k.px.capacity() * 4).sum::<usize>(),
            ),
            (
                "landform sprites",
                self.landforms
                    .iter()
                    .map(|l| l.bytes())
                    .sum::<usize>(),
            ),
            ("people sprites", self.people_sprites.bytes()),
            ("traffic sprites", self.traffic_sprites.bytes()),
            ("mover sprites", self.mover_sprites.bytes()),
            (
                "actor lists (people/traffic/movers)",
                v(&self.people)
                    + v(&self.people_buf)
                    + v(&self.people_px)
                    + v(&self.people_px_buf)
                    + v(&self.traffic)
                    + v(&self.parked)
                    + v(&self.movers)
                    + v(&self.traffic_px)
                    + v(&self.movers_px)
                    + v(&self.traffic_px_buf)
                    + v(&self.movers_px_buf)
                    + v(&self.parked_px),
            ),
            (
                "city objects",
                v(&self.city) + v(&self.city_diag) + self.city_diag.iter().map(|d| d.capacity() * std::mem::size_of::<CitySlice>()).sum::<usize>(),
            ),
            (
                "sorted lists + band index (people/traffic/movers/water/lamps)",
                v(&self.people_sorted)
                    + v(&self.water_px)
                    + v(&self.lamp_px)
                    + self.people_bands.bytes()
                    + self.traffic_bands.bytes()
                    + self.parked_bands.bytes()
                    + self.mover_bands.bytes()
                    + self.water_bands.bytes()
                    + self.lamp_bands.bytes(),
            ),
            (
                "street + tech keys",
                self.street.paint.len() * 12
                    + self.street.lamps.len() * 8
                    + self.street.signs.len() * 12
                    + self.street.lights.len() * 8
                    + self.tech.lamps.len() * 12
                    + self.tech.poles.len() * 8
                    + self.tech.buried.len() * 8,
            ),
            (
                "map + per-tile planes",
                self.map.tiles.capacity() * std::mem::size_of::<map::Tile>()
                    + v(&self.map.water_class)
                    + v(&self.map.wear)
                    + v(&self.visible),
            ),
            ("grow-only scratches", v(&self.pending) + v(&self.tree_pending) + v(&self.water_scratch)),
        ];
        if let Some(s) = &self.sky_clean {
            out.push(("sky (clean copy)", s.bytes()));
        }
        out.sort_by(|a, b| b.1.cmp(&a.1));
        out
    }

    pub fn height(&self) -> usize {
        self.static_layer.h
    }

    /// B84: the static layer's colour plane and its depth plane, with their
    /// sizes, so the memory report can say how much of each is resident.
    pub fn static_px(&self) -> &[crate::palette::Color] {
        self.static_layer.px()
    }

    pub fn static_depth(&self) -> &[u16] {
        self.static_layer.depth().unwrap_or(&[])
    }

    pub fn static_bytes_colour(&self) -> usize {
        self.static_layer.colour_bytes()
    }

    pub fn static_bytes_depth(&self) -> usize {
        self.static_layer.depth_bytes()
    }

    /// The tinted copy's plane (the composite's source) and its size.
    pub fn tinted_px(&self) -> &[crate::palette::Color] {
        self.tinted_static.px()
    }

    pub fn tinted_bytes(&self) -> usize {
        self.tinted_static.colour_bytes()
    }

    /// B84: the first row of the frame the static layer can hold anything on —
    /// the top of `land_clip`. Everything above it is sky band: the layer, its
    /// tinted copy and its depth plane are never written there (every static
    /// pass is clipped to `land_clip`), so those pages are left untouched and
    /// the three planes cost a quarter less resident memory than their size.
    #[inline]
    pub fn static_row0(&self) -> usize {
        // B84: `CITYLIVE_BAND_SKIP=0` puts the old behaviour back (every row of
        // the layer written and read), for a byte-for-byte A/B on one build.
        static OFF: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        if *OFF.get_or_init(|| std::env::var("CITYLIVE_BAND_SKIP").map_or(false, |v| v == "0")) {
            return 0;
        }
        self.land_top() as usize
    }

    /// The first row the composite has to look at: everything above it is
    /// empty in the static layer (see `above_top`).
    #[inline]
    fn scan_top(&self) -> i32 {
        self.above_top.max(self.land_top())
    }

    /// The first frame row the world may paint on (see [`HORIZON_ROOM`]).
    #[inline]
    fn land_top(&self) -> i32 {
        (self.sky_h as i32 - HORIZON_ROOM).max(0)
    }

    /// Per tile: does it touch the framebuffer? Off-screen tiles exist because
    /// the world is the rectangle around the frame's diamond.
    #[inline]
    pub fn is_visible(&self, x: i32, y: i32) -> bool {
        if !self.map.in_bounds(x, y) {
            return false;
        }
        self.visible[(y * self.map.w + x) as usize]
    }

    pub fn visible_tiles(&self) -> usize {
        self.visible.iter().filter(|v| **v).count()
    }

    /// Replace the city objects (plots at their construction stage). The tiles
    /// of every object that changed are dirtied, so nothing else is redrawn.
    /// Replace the sky layer (the `sky` crate draws it; the renderer only
    /// stores it, so the two crates stay acyclic).
    /// The land area: everything below the sky band (D43). The static layer is
    /// clipped to it, so terrain never pokes into the sky.
    fn land_clip(&self) -> Rect {
        // B42: the world is painted a little way into the sky band and then
        // dissolved into it (see `composite_rect`), so a tree or a spire on the
        // back row is never cut off by the horizon line.
        let top = self.land_top();
        Rect::new(0, top, self.width() as i32, self.height() as i32)
    }

    /// The land line and below: where `seal_seams` may work (B65).
    ///
    /// The sealer copies the last opaque pixel of a row into a transparent
    /// gap, which is right for a one-pixel seam between two tile tops and
    /// wrong above the land line, where the gap *is* the sky: a back-row tree
    /// had its rows dragged sideways across the band as flat green wedges.
    /// Ground painted into the band is masked away in the composite anyway, so
    /// nothing is lost by sealing only below the line.
    fn seal_clip(&self) -> Rect {
        Rect::new(
            0,
            self.sky_h as i32,
            self.width() as i32,
            self.height() as i32,
        )
    }

    /// The painter key a figure standing on `(x, y)` may reach: the plot's
    /// ground key, 0 where no lot stands (B43/B65).
    pub fn city_key_at(&self, x: i32, y: i32) -> u16 {
        let i = (y * self.map.w + x).max(0) as usize;
        self.city_key.get(i).copied().unwrap_or(0)
    }

    /// Height of the sky band in pixels (D43).
    pub fn sky_height(&self) -> usize {
        self.sky_h
    }

    pub fn set_sky(&mut self, sky: Framebuffer) {
        if std::env::var("PP_DEBUG").is_ok() {
            eprintln!("mark_full: set_sky");
        }
        // A new sky (clouds drift every 30 s, the light re-bakes) is a
        // whole-frame change (B30). B67: in the wallpaper the haze it feeds is
        // re-tinted a band at a time (each band damages itself), so the sky's own
        // change is the band alone.
        if self.async_shadow && self.sky.w == sky.w && self.sky.h == sky.h && sky.h > 0 {
            self.mark_rect(Rect::new(0, 0, sky.w as i32, sky.h as i32));
        } else {
            self.mark_full();
        }
        // B34: the sky is only as tall as the band — below it the static layer
        // is opaque (seal_seams fills every hole in the land clip).
        debug_assert_eq!((sky.w, sky.h), (self.width(), self.sky_h));
        self.sky = sky;
        // B50: a new clean sky: the ending's sky objects go on top again.
        self.sky_clean = None;
        if self.ending.is_some() {
            self.bake_ending_sky();
        }
        self.resample_haze();
        self.retint_soft();
    }

    /// B50: the ending's sky (the swollen sun, the falling moon, the comet, the
    /// washes) is painted into the sky band itself, so it costs nothing per
    /// frame and changes only when the ending's slow clock ticks.
    fn bake_ending_sky(&mut self) {
        if self.sky.h == 0 {
            return;
        }
        let base = match self.sky_clean.take() {
            Some(b) => b,
            None => self.sky.clone(),
        };
        let mut fb = base.clone();
        if self.ending.is_some() {
            if let Some(ctx) = self.ending_ctx() {
                crate::ending::draw_sky(&mut fb, &ctx);
            }
            self.sky_clean = Some(base);
        }
        self.sky = fb;
    }

    /// Re-derive the per-column fade colour from the sky's bottom row.
    ///
    /// The target is that pixel *exactly* (B42b): the backdrop already paints a
    /// distant meadow/forest floor at the bottom of the band, so the ground that
    /// fades into it lands on a colour the world is really made of. Pulling it
    /// half-way to the raw air colour is what turned the far map orange at dusk.
    fn resample_haze(&mut self) {
        self.haze_row.clear();
        if self.sky_h == 0 || self.sky.h == 0 {
            return;
        }
        let w = self.width();
        let row = (self.sky.h - 1) * w;
        let src = &self.sky.px()[row..row + w];
        // Box-blur the sampled row over 96 px (B42c). Taken per column it made
        // the haze follow every 1-px mottle and 2-px trunk in the band, which
        // streaked the far ground with vertical smears.
        const BLUR: usize = 96;
        let mut out = Vec::with_capacity(w);
        let mut acc: u32 = 0;
        let (mut r, mut g, mut b) = (0u32, 0u32, 0u32);
        for i in 0..BLUR {
            let c = src[i.min(w - 1)];
            r += palette::r(c) as u32;
            g += palette::g(c) as u32;
            b += palette::b(c) as u32;
        }
        for x in 0..w {
            let c = src[(x + BLUR / 2).min(w - 1)];
            r += palette::r(c) as u32;
            g += palette::g(c) as u32;
            b += palette::b(c) as u32;
            let o = src[(x + w - BLUR / 2) % w];
            r -= palette::r(o) as u32;
            g -= palette::g(o) as u32;
            b -= palette::b(o) as u32;
            acc = BLUR as u32;
            // Desaturate a little: the band's bottom row is a *ground* colour,
            // and following it exactly dragged the far map towards whatever
            // saturated hue the light had (B42c).
            let c = palette::rgb((r / acc) as u8, (g / acc) as u8, (b / acc) as u8);
            let l = (0.30 * palette::r(c) as f32
                + 0.59 * palette::g(c) as f32
                + 0.11 * palette::b(c) as f32) as u8;
            out.push(palette::mix(c, palette::rgb(l, l, l), 0.35));
        }
        self.haze_row = out;
    }

    /// Which horizon treatment this world got (D64).
    pub fn horizon(&self) -> Horizon {
        self.horizon
    }

    /// The air colour beyond the horizon (B42). The sky is the authority: the
    /// caller hands over the same colour the backdrop hazes its hills towards,
    /// so the far ground dissolves into them exactly.
    pub fn set_horizon_air(&mut self, air: Color) {
        if air != self.haze_air {
            self.haze_air = air;
            self.resample_haze();
            self.retint_soft();
        }
    }

    /// The air colour behind this world's horizon.
    pub fn horizon_air(&self) -> Color {
        self.haze_air
    }

    /// Store the light without rebuilding anything (B34: at startup the first
    /// `set_light` used to re-rasterise the whole layer just to change the
    /// window glow, and then the city arrived and did it again).
    pub fn set_light_state_quiet(&mut self, ls: LightState) {
        self.light = ls;
        self.sync_water_glint();
        self.tint = ls.ambient;
        self.glow = ls.glow.clamp(0.0, 1.0);
    }

    pub fn set_light_quiet(&mut self, tint: [f32; 3], glow: f32) {
        self.tint = tint;
        self.light.ambient = tint;
        self.glow = glow.clamp(0.0, 1.0);
        self.light.glow = self.glow;
        self.people_sprites.retint(tint);
    }

    /// Store the static fixtures without rebuilding (the next
    /// `set_city_with_sprites` rebuild paints them).
    pub fn set_fixtures_quiet(&mut self, f: crate::rail::Fixtures) {
        self.fixtures = f;
        self.rebuild_planting();
    }

    /// Ambient tint and window glow: everything except the direction of the
    /// light. Kept for the tools and the tests that only care about the hour's
    /// colour; [`Self::set_light_state`] is the full model.
    pub fn set_light(&mut self, tint: [f32; 3], glow: f32) {
        let ls = LightState {
            ambient: tint,
            glow,
            ..self.light
        };
        self.set_light_state(ls);
    }

    /// The whole light model (D74): sun and moon positions, sky ambient, window
    /// glow and cloud cover, in one call. Cheap when nothing moved — the tinted
    /// layer, the window glow and the cast shadows each have their own gate.
    pub fn set_light_state(&mut self, ls: LightState) {
        let tint_changed = (0..3).any(|i| (ls.ambient[i] - self.tint[i]).abs() > 0.004);
        let glow_changed = (ls.glow - self.glow).abs() > 0.04;
        // B58b item 7: the windows follow the clock, so a new half hour at night
        // is a new lit pattern (the re-bake is a rare, ~20 ms event).
        let hour_changed = ls.glow > 0.12
            && self.glow > 0.12
            && (ls.hour - self.light.hour).abs() > 0.5;
        let was = caster_key(&self.light);
        let now = caster_key(&ls);
        let caster_changed = was.is_none() != now.is_none()
            || was
                .zip(now)
                .map(|(a, b)| {
                    // ~6° of azimuth, or a degree of altitude: below that the
                    // shadows move by a pixel or two and the bake is wasted.
                    let (dx, dy) = (a.0 - b.0, a.1 - b.1);
                    dx * dx + dy * dy > 0.01 || (a.2 - b.2).abs() > 1.0
                })
                .unwrap_or(false);
        let stale = self.field_dirty;
        let landforms_moved = caster_changed && !self.landforms.is_empty();
        self.light = ls;
        self.tint = ls.ambient;
        self.glow = ls.glow.clamp(0.0, 1.0);
        self.light.glow = self.glow;
        // B57: the cloud shadows are the same multiply as the cast ones, so all
        // they need is the relative strength; a deck (or no caster) turns them
        // off entirely.
        let lit = ls.caster().map(|c| c.strength).unwrap_or(0.0);
        let cloud_lit = ls.cloud_caster().map(|c| c.strength).unwrap_or(0.0);
        // A cloud's shadow blocks the *whole* direct beam, but `caster().strength`
        // has already been faded by the average cover (that is what makes a grey
        // day's building shadows soft). Compensate, so standing under a cumulus
        // reads as shade on a sunny day rather than as a slightly duller sunny
        // day: the ratio is the cover's own attenuation, inverted.
        let attenuation = (1.0 - 0.72 * ls.cloud.clamp(0.0, 1.0)).max(0.15);
        self.cloud_k = if self.cloud_shadows && self.clouds.cast && lit > 0.02 && cloud_lit > 0.02 {
            (0.95 * cloud_lit / lit / attenuation).clamp(0.0, 2.6)
        } else {
            0.0
        };
        // One multiply table for the weather, at the strength a cloud's own
        // shadow needs (see the note above).
        self.cloud_lut = if self.cloud_k > 0.002 {
            // The *absolute* strength is the occasional shadow's: 0.9 of a clear
            // noon. What `cloud_k` compensates for never enters the table.
            ShadowLut::new(ls.shadow_mult(), (lit * self.cloud_k).clamp(0.0, 1.0))
        } else {
            ShadowLut::none()
        };
        if landforms_moved {
            self.rebake_landforms();
        }
        if caster_changed || stale || self.shadow_lut_stale(&ls) {
            if self.async_shadow {
                self.request_shadow_bake();
            } else {
                self.rebake_shadow();
            }
        }
        // The shadows are drawn per frame from the light, so a moving sun needs
        // nothing re-baked — only a frame, since the shadows have moved on it.
        if !self.clouds.is_empty() {
            let off = crate::clouds::Clouds::shift(&ls, CLOUD_OFFSET_PROBE);
            if off != self.cloud_off {
                self.cloud_off = off;
                self.mark_full();
            }
        }
        if !tint_changed && !glow_changed && !hour_changed {
            return;
        }
        // B99: the fog stands in the same light as the city (dark at night).
        if self.ambient.set_air(self.sky_h, self.tint) {
            let h = self.ambient.rows_height();
            if h > 0 {
                self.ambient.rebuild_rows(h);
            }
        }
        if std::env::var("PP_DEBUG").is_ok() {
            eprintln!("mark_full: set_light");
        }
        if !self.async_shadow {
            self.mark_full();
        }
        // B72 (D78): a window is no longer part of the static layer's build. A
        // change in the hour or in the sky's darkness switches the panes that
        // cross their own threshold, a few dozen pixels each — never the whole
        // facade, and never a re-rasterisation of the city.
        if glow_changed || hour_changed {
            self.refresh_windows(None);
        }
        if glow_changed && !hour_changed {
            self.mark_full();
        }
        // B67: a tint step alone is a re-tint a band at a time.
        self.retint_soft();
        self.life.retint(self.tint);
        self.people_sprites.retint(self.tint);
    }

    /// B46d: re-bake the landform sprites for the caster in force and dirty
    /// their rectangles. The facets are lit by the same caster the cast shadows
    /// use (D74), so a sun that has moved a few degrees turns the lit and shade
    /// faces; below the caster's own change threshold nothing is redone.
    pub fn rebake_landforms(&mut self) {
        let t0 = Instant::now();
        let sun = crate::landform::Sun::from_caster(self.light.caster());
        let bounds = self.static_layer.bounds();
        for lf in self.landforms.iter_mut() {
            let old = lf.bounds(&self.cam);
            lf.relight(&sun);
            let r = old.union(&lf.bounds(&self.cam)).intersect(&bounds);
            if !r.is_empty() {
                self.dirty.push(r);
            }
        }
        self.landform_ms = t0.elapsed().as_secs_f64() * 1000.0;
    }

    /// Cost of the last landform re-bake, in ms.
    pub fn landform_ms(&self) -> f64 {
        self.landform_ms
    }

    /// The light model in force.
    pub fn light_state(&self) -> LightState {
        self.light
    }

    /// Cost of the last shadow bake, in ms.
    pub fn shadow_ms(&self) -> f64 {
        self.shadow_ms
    }

    /// Does the stored shadow multiply no longer match the light's strength?
    fn shadow_lut_stale(&self, ls: &LightState) -> bool {
        let want = ls.caster().map(|c| c.strength).unwrap_or(0.0);
        (want - self.shadow_strength).abs() > 0.02
    }

    /// B68: poles and signs are thinner than a field sample, so their shadows are
    /// lines laid straight onto the plane.
    fn stamp_street_posts(&self, plane: &mut Vec<u8>) {
        if let Some(c) = self.light.caster() {
            let mut posts = Vec::new();
            for &(x, y) in &self.street.lamps {
                let (f, h) = crate::street::pole_foot(&self.cam, &self.map, 0, x, y);
                posts.push((f.0, f.1, h));
            }
            for (&(x, y), _) in &self.street.signs {
                let (f, h) = crate::street::pole_foot(&self.cam, &self.map, 1, x, y);
                posts.push((f.0, f.1, h));
            }
            for &(x, y) in &self.street.lights {
                let (f, h) = crate::street::pole_foot(&self.cam, &self.map, 2, x, y);
                posts.push((f.0, f.1, h));
            }
            crate::shadow::stamp_posts(plane, &self.static_layer, &c, &posts);
        }
    }

    /// Rebuild the height field if the city moved, then re-march the shadows and
    /// start a crossfade to them (B60: the light moves continuously, so a bake
    /// every minute must not step).
    pub fn rebake_shadow(&mut self) {
        self.shadow_job = None;
        self.shadow_again = false;
        let t0 = Instant::now();
        if self.field_dirty {
            self.field
                .rebuild(&self.draw_map, &self.buildings, &self.city, &self.kit);
            stamp_landforms(&mut self.field, &self.landforms);
            self.field_dirty = false;
        }
        let t_field = t0.elapsed().as_secs_f64() * 1000.0;
        let strength = self.light.caster().map(|c| c.strength).unwrap_or(0.0);
        let mut mask = std::mem::take(&mut self.shadow_mask);
        // A caster too weak to read (a thin moon under cloud) costs nothing: the
        // frame goes back to full ambient rather than paying for a march — under
        // 10 % strength the darkening would be under 4 %, i.e. invisible.
        let (mut t_mask, mut t_screen) = (0.0, 0.0);
        let mut plane = self.spare_plane();
        if strength > 0.10 {
            let tm = Instant::now();
            let caster = crate::shadow::bake_mask(&self.field, &self.light, &mut mask).expect("strength > 0");
            let ceil = crate::shadow::bake_ceil(&self.field, &caster);
            t_mask = tm.elapsed().as_secs_f64() * 1000.0;
            let ts = Instant::now();
            crate::shadow::bake_screen(
                &self.static_layer,
                &self.cam,
                &self.field,
                &mask,
                &caster,
                &ceil,
                &mut plane,
                self.static_row0().min(self.static_layer.h),
            );
            t_screen = ts.elapsed().as_secs_f64() * 1000.0;
        } else {
            plane.fill(0);
        }
        self.stamp_street_posts(&mut plane);
        self.shadow_mask = mask;
        let ms = t0.elapsed().as_secs_f64() * 1000.0;
        if std::env::var("CITYLIVE_MOTION_TRACE").is_ok() {
            eprintln!(
                "BAKE field {t_field:.1} mask {t_mask:.1} screen {t_screen:.1} total {ms:.1} ms"
            );
        }
        self.adopt_shadow(plane, self.light.shadow_mult(), strength, ms);
    }

    /// A zeroed plane of the frame's size: the crossfade target's buffer if it
    /// is still around, otherwise a fresh one (B61f).
    fn spare_plane(&mut self) -> Vec<u8> {
        let n = self.width() * self.height();
        match self.shadow_to.take() {
            Some(mut p) if p.len() == n => {
                p.fill(0);
                p
            }
            _ => vec![0; n],
        }
    }

    /// Take a finished bake (B61f). A bake whose picture has not visibly moved
    /// is swapped in without a repaint at all: a crossfade of three whole-frame
    /// composites (63 ms each at 7040x1440, so ~2 frames dropped a time) is a
    /// high price for hiding a change nobody can see. The drift is summed so a
    /// run of invisible bakes cannot creep the plane away from the picture; past
    /// `SNAP_DRIFT` the next one crossfades as it always did.
    ///
    /// Threshold in levels of the shadow plane (255 = full shadow). One level is
    /// 0.4 % of the darkening, and the plane only covers the shadowed ground, so
    /// a mean of `SNAP_MEAN` over the whole frame is well under a JND.
    fn adopt_shadow(&mut self, plane: Vec<u8>, mult: [f32; 3], strength: f32, ms: f64) {
        self.shadow_lut = ShadowLut::new(mult, strength);
        self.shadow_strength = strength;
        self.shadow_ms = ms;
        if plane.len() != self.shadow.len() {
            self.shadow = plane;
            self.shadow_to = None;
            self.shadow_fade = 1.0;
            self.shadow_drift = 0.0;
            self.mark_full();
            return;
        }
        // Every fourth byte is plenty to tell a moved shadow from a still one,
        // and it keeps this scan to a few milliseconds.
        let mut sum = 0u64;
        let mut worst = 0u8;
        let mut n = 0u64;
        for i in (0..plane.len()).step_by(4) {
            let d = self.shadow[i].abs_diff(plane[i]);
            sum += d as u64;
            worst = worst.max(d);
            n += 1;
        }
        let mean = sum as f32 / n.max(1) as f32;
        let trace = std::env::var("CITYLIVE_MOTION_TRACE").is_ok();
        if mean <= SNAP_MEAN && self.shadow_drift + mean <= SNAP_DRIFT {
            self.shadow = plane;
            self.shadow_to = None;
            self.shadow_fade = 1.0;
            self.shadow_drift += mean;
            if trace {
                eprintln!(
                    "SHADOW snap mean {mean:.3} worst {worst} drift {:.2} (no repaint)",
                    self.shadow_drift
                );
            }
            return;
        }
        if trace {
            eprintln!("SHADOW fade mean {mean:.3} worst {worst}");
        }
        self.shadow_drift = 0.0;
        self.shadow_to = Some(plane);
        self.shadow_fade = 0.0;
        self.mark_full();
    }

    /// B67: bake the shadows a slice at a time. Call [`Self::set_async_shadow`]
    /// first; [`Self::shadow_tick`] then does a few milliseconds of the job each
    /// frame and the crossfade starts when it is done.
    pub fn set_async_shadow(&mut self, on: bool) {
        self.async_shadow = on;
    }

    /// Ask for a bake from the current light: starts one, or notes that another
    /// is wanted when the running one finishes (the running one is for a light a
    /// minute old at worst).
    fn request_shadow_bake(&mut self) {
        if self.shadow_job.is_some() {
            self.shadow_again = true;
            return;
        }
        self.begin_shadow_job();
    }

    fn begin_shadow_job(&mut self) {
        let t0 = Instant::now();
        if self.field_dirty {
            self.field
                .rebuild(&self.draw_map, &self.buildings, &self.city, &self.kit);
            stamp_landforms(&mut self.field, &self.landforms);
            self.field_dirty = false;
        }
        let strength = self.light.caster().map(|c| c.strength).unwrap_or(0.0);
        match self.light.caster() {
            Some(caster) if strength > 0.10 => {
                self.shadow_job = Some(ShadowJob {
                    caster,
                    mask: vec![0; self.field.w * self.field.h],
                    ceil: crate::shadow::Ceil::new(&self.field),
                    facet: 0,
                    plane: vec![0; self.static_layer.w * self.static_layer.h],
                    j: 0,
                    // B84: the rows above the land line have no receiver (the
                    // static layer is transparent there), so the projection
                    // starts at the land line and leaves those plane rows 0 —
                    // and the layer's own top rows unread.
                    y: self.static_row0().min(self.static_layer.h),
                    strength,
                    mult: self.light.shadow_mult(),
                    spent_ms: t0.elapsed().as_secs_f64() * 1000.0,
                    slices: 0,
                    worst_ms: 0.0,
                });
            }
            _ => {
                // Too weak to read: nothing to march, finish at once.
                let mut plane = self.spare_plane();
                plane.fill(0);
                let mult = self.light.shadow_mult();
                self.adopt_shadow(plane, mult, strength, t0.elapsed().as_secs_f64() * 1000.0);
            }
        }
    }

    /// Spend up to `budget_ms` on the running bake. Returns true when it
    /// finished (the crossfade has begun).
    fn shadow_work(&mut self, budget_ms: f64) -> bool {
        let Some(mut job) = self.shadow_job.take() else {
            return false;
        };
        // The layer changed size under us: start over.
        if job.plane.len() != self.static_layer.w * self.static_layer.h
            || job.mask.len() != self.field.w * self.field.h
        {
            self.begin_shadow_job();
            return false;
        }
        let t0 = Instant::now();
        let left = |t: &Instant| t.elapsed().as_secs_f64() * 1000.0 < budget_ms;
        const ROWS_MASK: usize = 4;
        const ROWS_SCREEN: usize = 8;
        while job.j < self.field.h && left(&t0) {
            let ts = Instant::now();
            let j1 = (job.j + ROWS_MASK).min(self.field.h);
            crate::shadow::bake_mask_rows(&self.field, &job.caster, &mut job.mask, job.j, j1);
            crate::shadow::bake_ceil_rows(&self.field, &job.caster, &mut job.ceil, job.j, j1);
            job.j = j1;
            job.slices += 1;
            job.worst_ms = job.worst_ms.max(ts.elapsed().as_secs_f64() * 1000.0);
        }
        while job.j >= self.field.h && job.facet < self.field.roofs.len() && left(&t0) {
            let r1 = (job.facet + 64).min(self.field.roofs.len());
            crate::shadow::bake_facets(&self.field, &job.caster, &mut job.ceil, job.facet, r1);
            job.facet = r1;
        }
        while job.j >= self.field.h && job.facet >= self.field.roofs.len() && job.y < self.static_layer.h && left(&t0) {
            let ts = Instant::now();
            let y1 = (job.y + ROWS_SCREEN).min(self.static_layer.h);
            crate::shadow::bake_screen_rows(
                &self.static_layer,
                &self.cam,
                &self.field,
                &job.mask,
                &job.caster,
                &job.ceil,
                &mut job.plane,
                job.y,
                y1,
            );
            job.y = y1;
            job.slices += 1;
            job.worst_ms = job.worst_ms.max(ts.elapsed().as_secs_f64() * 1000.0);
        }
        job.spent_ms += t0.elapsed().as_secs_f64() * 1000.0;
        if job.j >= self.field.h && job.y >= self.static_layer.h {
            if std::env::var("CITYLIVE_MOTION_TRACE").is_ok() {
                eprintln!("BAKE-SLICED slices {} worst slice {:.2} ms total {:.1} ms (field {}x{}, ceil {} KB, kit {} with roof {} field roofs {})", job.slices, job.worst_ms, job.spent_ms, self.field.w, self.field.h, (job.ceil.full.len() + job.ceil.none.len() + job.ceil.bmax.len()) * 2 / 1024, self.kit.len(), self.kit.iter().filter(|k| k.roof.is_some()).count(), self.field.roofs.len());
            }
            self.shadow_mask = job.mask;
            let mut plane = job.plane;
            self.stamp_street_posts(&mut plane);
            self.adopt_shadow(plane, job.mult, job.strength, job.spent_ms);
            if std::mem::take(&mut self.shadow_again) {
                self.begin_shadow_job();
            }
            true
        } else {
            self.shadow_job = Some(job);
            false
        }
    }


    /// Advance the shadow crossfade. Returns true while it still needs frames
    /// (the caller then has to keep compositing).
    pub fn shadow_tick(&mut self, dt: f32) -> bool {
        // B67: a few milliseconds of the running bake, every frame — but not
        // while a static pass is running: both are full-frame jobs, and the
        // shadow can wait the few seconds the pass takes (B76 r2).
        if self.shadow_job.is_some() && self.rebuild_job.is_none() {
            self.shadow_work(3.0);
        }
        // B76 r2: a few milliseconds of the running static rebuild, and only
        // then its re-tint (the rebuild publishes by starting it).
        let rebuilding = self.rebuild_job.is_some() && self.rebuild_work(3.0);
        let retinting = self.retint_job.is_some() && self.retint_work(1.2);
        if self.shadow_fade >= 1.0 {
            return self.shadow_job.is_some() || retinting || rebuilding;
        }
        // ~0.25 s at any frame rate: fast enough to be invisible, slow enough
        // that no frame can jump.
        let k = (dt / 0.08).clamp(0.0, 1.0);
        let Some(to) = self.shadow_to.take() else {
            // Nothing to fade to (a bake was adopted without one).
            self.shadow_fade = 1.0;
            return self.shadow_job.is_some() || retinting;
        };
        let mut to = to;
        for (a, b) in self.shadow.iter_mut().zip(to.iter()) {
            let d = *b as f32 - *a as f32;
            *a = (*a as f32 + d * k).clamp(0.0, 255.0) as u8;
        }
        self.shadow_fade = (self.shadow_fade + k).min(1.0);
        if self.shadow_fade >= 1.0 {
            // The fade has landed: the target *is* the plane now (B61f: moved,
            // not copied — the old buffer would only be freed a frame later).
            to.copy_from_slice(&self.shadow);
            self.shadow = to;
        } else {
            self.shadow_to = Some(to);
        }
        // Every shadowed pixel moved, so the frame did too: the presenter can
        // skip its pixel hash (B61f).
        self.moving_changed = true;
        self.mark_full();
        true
    }

    /// B61e: did a moving layer change this frame? Cleared by the presenter once
    /// it has used the answer.
    pub fn moving_changed(&self) -> bool {
        self.moving_changed
    }

    /// B61e: forget the "a moving layer changed" hint (the presenter calls this
    /// after it has presented the frame).
    pub fn clear_moving_changed(&mut self) -> bool {
        let was = self.moving_changed;
        self.moving_changed = false;
        was
    }

    /// The frame's shadow coverage plane (0 = full sun, 255 = full shadow).
    pub fn shadow_plane(&self) -> &[u8] {
        &self.shadow
    }

    // -----------------------------------------------------------------------
    // B57: clouds
    // -----------------------------------------------------------------------

    /// Replace the cloud field's cover and storm state (D72). Cheap and
    /// idempotent: the field only re-populates when the weather moves into a new
    /// band. Returns true when it did (the caller then wants a full frame).
    pub fn set_clouds(&mut self, cover: f32, storm: f32) -> bool {
        let biome = self.biome();
        let rebuilt = self.clouds.set(cover, storm, biome, &self.cam, &self.map);
        if rebuilt {
            self.cloud_off = crate::clouds::Clouds::shift(&self.light, CLOUD_OFFSET_PROBE);
            self.mark_full();
        }
        rebuilt
    }

    /// The wind the clouds drift with (from the weather: direction it blows
    /// *from*, and its speed).
    pub fn set_cloud_wind(&mut self, dir_deg: f32, kmh: f32) {
        self.clouds.set_wind(dir_deg, kmh);
        // B69: the trees lean into the same wind.
        self.trees.set_wind(dir_deg, kmh);
    }

    /// B69/D71: advance the trees' sway by `dt` seconds. Each tree whose
    /// picture changed damages its own small box; in calm air nothing is
    /// computed and nothing is damaged.
    pub fn tree_tick(&mut self, dt: f32) {
        let mut out = std::mem::take(&mut self.tree_pending);
        let frame = Rect::from_size(self.width(), self.height());
        let land = self.land_clip();
        let before = out.len();
        self.trees.tick(dt, &mut out);
        let mut i = before;
        while i < out.len() {
            let r = out[i].intersect(&frame).intersect(&land);
            if r.is_empty() {
                out.swap_remove(i);
            } else {
                out[i] = r;
                i += 1;
            }
        }
        self.tree_pending = out;
    }

    /// B69: turn tree sway off (trees are painted into the static layer, as
    /// before B69) or on. For A/B comparisons and tests; the wallpaper never
    /// turns it off (D71).
    pub fn set_tree_sway(&mut self, on: bool) {
        if on != self.sway_on {
            self.sway_on = on;
            self.rebuild_static();
            self.retint_static();
            // The cast-shadow plane reads the static layer's tags (a canopy is
            // not a receiver), so it follows.
            self.rebake_shadow();
            while self.shadow_tick(1.0) {}
            self.mark_full();
        }
    }

    /// Set tree sway before the first build (no rebuild). The wallpaper keeps
    /// sway off until it fits the budget at the real 7040x1440 (B69b).
    pub fn set_tree_sway_before_build(&mut self, on: bool) {
        self.sway_on = on;
    }

    /// B69, for tools and sheets: hold every tree at one lean (half pixels at
    /// the tip) until `None`.
    pub fn force_tree_lean(&mut self, q: Option<i16>) {
        self.trees.force_lean(q);
        self.tree_tick(0.0);
    }

    /// B69: what the sway layer is doing (trees held, moving, repainted).
    pub fn tree_stats(&self) -> crate::sway::SwayStats {
        self.trees.stats()
    }

    /// B69: how many trees something other than a tree can stand in front of.
    pub fn tree_occluded_count(&self) -> usize {
        self.trees.occluded_count()
    }

    /// B69: the lean (half pixels at the tip) of the tree on a tile.
    pub fn tree_lean(&self, x: i32, y: i32) -> Option<i16> {
        self.trees.lean_q((x, y))
    }

    /// B69: the screen box of the tree on a tile, at rest.
    pub fn tree_rect(&self, x: i32, y: i32) -> Option<Rect> {
        self.trees.rest_rect((x, y))
    }

    /// B69: the top-most screen row a swaying tree reaches (the treeline).
    pub fn tree_top_row(&self) -> Option<i32> {
        self.trees.top_row()
    }

    /// The hour's cloud colour (B57): built by the caller from the sky palette
    /// and the light, because `render` does not know the sky.
    pub fn set_cloud_ink(&mut self, ink: crate::clouds::Ink) -> bool {
        // B84 r3: a change nobody can see must not repaint the sky. In rain the
        // ink moves a level or two a second, and repainting the whole band for
        // it cost 21 ms a frame (the deck is drawn over 3.4 Mpx). Colours are
        // compared in 8/255 steps, opacity in 1/32.
        fn near(a: crate::palette::Color, b: crate::palette::Color) -> bool {
            let ch = |sh: u32| ((a >> sh) & 0xFF).abs_diff((b >> sh) & 0xFF) <= 8;
            ch(16) && ch(8) && ch(0) && (a >> 24) == (b >> 24)
        }
        let same = near(ink.body, self.cloud_ink.body)
            && near(ink.top, self.cloud_ink.top)
            && near(ink.base, self.cloud_ink.base)
            && near(ink.rim, self.cloud_ink.rim)
            && near(ink.rain, self.cloud_ink.rain)
            && near(ink.deck, self.cloud_ink.deck)
            && near(ink.deck_base, self.cloud_ink.deck_base)
            && (ink.alpha - self.cloud_ink.alpha).abs() < 0.03
            && (ink.lit - self.cloud_ink.lit).abs() < 0.03;
        if same {
            return false;
        }
        self.cloud_ink = ink;
        // Only the clouds' own pixels and their shadows change: damage them, not
        // the whole frame.
        for i in 0..self.clouds.lows.len() {
            let c = self.clouds.lows[i];
            let rects = self.clouds.rects_of(&c, &self.cam, &self.map, &self.light);
            for r in rects {
                self.mark_rect(r);
            }
        }
        for i in 0..self.clouds.band.len() {
            let c = self.clouds.band[i];
            for r in self.clouds.band_rects_of(&c) {
                self.mark_rect(r);
            }
        }
        // The deck spans the frame, so a colour change repaints the band.
        if self.clouds.deck > 0.02 {
            self.mark_rect(Rect::new(
                0,
                0,
                self.width() as i32,
                self.sky_height() as i32,
            ));
        }
        true
    }

    /// Do the clouds have anything to draw?
    pub fn clouds_live(&self) -> bool {
        !self.clouds.is_empty()
    }

    /// One line per cloud, for the tools (kind, sprite box, screen position).
    pub fn clouds_debug(&self) -> Vec<String> {
        let mut out = Vec::new();
        for c in self.clouds.lows.iter().filter(|c| c.life > 0.02) {
            let r = self.clouds.rects_of(c, &self.cam, &self.map, &self.light);
            out.push(format!(
                "shadow {:?} lift {:.0} rect {:?} / {:?}",
                c.kind, c.lift, r[0], r[1]
            ));
        }
        for c in self.clouds.band.iter().filter(|c| c.life > 0.02) {
            let r = self.clouds.band_rects_of(c);
            out.push(format!(
                "band {:?} life {:.2} base_y {:.0} shaft {:.0} rect {:?}",
                c.kind, c.life, c.cy, c.shaft, r[0]
            ));
        }
        out.push(format!(
            "deck {:.2} cover {:.2} storm {:.2} cast {} band_h {:.0}",
            self.clouds.deck, self.clouds.cover, self.clouds.storm, self.clouds.cast,
            self.clouds.band_h
        ));
        out
    }

    /// How many clouds are over the land / in the band (tools, handbacks).
    pub fn cloud_count(&self) -> (usize, usize) {
        // The clouds that are out (a roster member that is not out is not a cloud).
        self.clouds.visible()
    }

    /// The highest point any band cloud's sprite reaches (tests: no cloud may be
    /// cut by the top of the frame).
    pub fn band_cloud_top(&self) -> i32 {
        self.clouds
            .band
            .iter()
            .map(|c| self.clouds.band_tl_pub(c).1)
            .min()
            .unwrap_or(i32::MAX)
    }

    /// Turn the clouds' shadows on or off (B57's before/after renders). The
    /// field, its drift and the cloud sprites are untouched.
    pub fn set_cloud_shadows(&mut self, on: bool) {
        if on == self.cloud_shadows {
            return;
        }
        self.cloud_shadows = on;
        let ls = self.light;
        self.set_light_state(ls);
        self.mark_full();
    }

    /// Advance the weather by `dt`: every cloud drifts with the wind, and only
    /// the strips it swept are re-composited (D71: smooth, never a full frame
    /// for a cloud).
    pub fn cloud_tick(&mut self, dt: f32) -> bool {
        if self.clouds.is_empty() {
            return false;
        }
        let mut rects = std::mem::take(&mut self.cloud_damage);
        rects.clear();
        let moved = self.clouds.tick(dt, &self.cam, &self.map, &self.light, &mut rects);
        if moved {
            for r in &rects {
                self.mark_rect(*r);
            }
        }
        self.cloud_damage = rects;
        moved
    }

    /// The clouds' shadows cover this fraction of the frame (tools, handbacks).
    pub fn cloud_shadow_area(&self) -> f64 {
        self.clouds.shadow_area(self.width(), self.height())
    }

    /// Replace the city objects and repaint only the tiles that changed. This is
    /// the incremental path the sim uses every few seconds.
    /// Bake the ambient tint into a copy of the static layer (one pass over the
    /// frame, only when the light changes).
    ///
    /// B42: the same pass applies atmospheric perspective — the rows nearest the
    /// horizon are mixed towards the sky colour that sits directly above them,
    /// per column, so the far ground dissolves into whatever is beyond it
    /// (hills, farmland or sea) at any hour. Doing it here costs nothing per
    /// frame; doing it in the composite would tax every damage rect.
    fn retint_static(&mut self) {
        // B76 r2: never publish a layer that is still being drawn.
        if self.rebuild_job.is_some() {
            return;
        }
        self.retint_job = None;
        let lut = crate::light::lut(self.tint);
        let bulbs = &self.holiday.garland[..self.holiday.bulbs.max(1)];
        let w = self.width();
        let h = self.height();
        let haze = self.haze_profile();
        let air = std::mem::take(&mut self.haze_row);
        // B84: the rows above the land line hold nothing (every static pass is
        // clipped to `land_clip`) and are never read (the composite copies the
        // sky there). Skipping them keeps those pages untouched, so the two
        // full-frame planes cost a quarter less than their size.
        let row0 = self.static_row0();
        let src = self.static_layer.px();
        let dst = self.tinted_static.px_mut();
        let mut i = row0 * w;
        for y in row0..h {
            let amount = haze.get(y).copied().unwrap_or(0.0);
            for x in 0..w {
                let s = src[i];
                // B42b: a solid feature (a wall, a roof, a canopy, a lane) keeps
                // its colour all the way to the back of the map. Only the bare
                // ground takes the distance haze.
                let solid = matches!(
                    crate::palette::alpha(s),
                    crate::framebuffer::TAG_PROP | crate::framebuffer::TAG_BUILT
                );
                let mut c = night_pixel(s, &lut, bulbs);
                if amount > 0.0 && c != TRANSPARENT && !solid && air.len() == w {
                    c = palette::mix(c, air[x], amount);
                }
                dst[i] = c;
                i += 1;
            }
        }
        self.haze_row = air;
        self.refresh_opaque_rows();
    }

    /// B67: the same re-tint, a band of rows at a time. A weather or sky change
    /// moves the ambient tint by a level or two several times a minute; redoing
    /// ten million pixels at once froze the wallpaper for 80 ms each time. The job
    /// takes the light, the air and the haze as they are *now* and works through
    /// the layer from the top, a few milliseconds a frame; every band it finishes
    /// is damaged on its own, so nothing needs a full frame. (A static rebuild
    /// still calls [`Self::retint_static`], which does it all at once and cancels
    /// the job.)
    fn retint_soft(&mut self) {
        if !self.async_shadow {
            self.retint_static();
            return;
        }
        let bulbs = self.holiday.garland[..self.holiday.bulbs.max(1)].to_vec();
        // B84: start at the land line, like `retint_static` (the rows above it
        // are sky and must stay untouched).
        self.retint_job = Some(RetintJob {
            lut: crate::light::lut(self.tint),
            bulbs,
            haze: self.haze_profile(),
            air: self.haze_row.clone(),
            y: self.static_row0(),
        });
        // The opacity table is rebuilt row by row as the job goes.
        let rows = self.height();
        if self.static_opaque.len() != rows {
            self.static_opaque.resize(rows, false);
        }
    }

    /// Spend up to `budget_ms` on the running re-tint. True while it has more.
    fn retint_work(&mut self, budget_ms: f64) -> bool {
        let Some(mut job) = self.retint_job.take() else {
            return false;
        };
        let t0 = Instant::now();
        let (w, h) = (self.width(), self.height());
        if self.tinted_static.w != w || self.tinted_static.h != h || self.static_layer.w != w {
            // The layer changed shape under the job: do it the plain way.
            self.retint_static();
            return false;
        }
        let y_start = job.y;
        while job.y < h && t0.elapsed().as_secs_f64() * 1000.0 < budget_ms {
            let y1 = (job.y + 16).min(h);
            for y in job.y..y1 {
                let amount = job.haze.get(y).copied().unwrap_or(0.0);
                let row = y * w;
                let mut opaque = true;
                for x in 0..w {
                    let s = self.static_layer.px()[row + x];
                    let solid = matches!(
                        crate::palette::alpha(s),
                        crate::framebuffer::TAG_PROP | crate::framebuffer::TAG_BUILT
                    );
                    let mut c = night_pixel(s, &job.lut, &job.bulbs);
                    if amount > 0.0 && c != TRANSPARENT && !solid && job.air.len() == w {
                        c = palette::mix(c, job.air[x], amount);
                    }
                    opaque &= c != TRANSPARENT;
                    self.tinted_static.px_mut()[row + x] = c;
                }
                self.static_opaque[y] = opaque;
            }
            job.y = y1;
        }
        if job.y > y_start {
            // B84: the re-tint writes only rows at and below the land line, but
            // the frame is repainted from the top of the band it belongs to —
            // the first slice still damages row 0, exactly as before, so the
            // composite's own work (and every overlay over the sky band) is
            // unchanged.
            let from = if y_start <= self.static_row0() { 0 } else { y_start };
            self.mark_rect(Rect::new(0, from as i32, w as i32, job.y as i32));
        }
        if job.y < h {
            self.retint_job = Some(job);
            true
        } else {
            false
        }
    }

    // --- B72: the windows follow the people inside (D78) -------------------

    /// B72: the sim's lights plan — per lot anchor, the 64 window slots lit
    /// right now. Only the buildings whose lights actually changed are
    /// repainted, a pane at a time.
    pub fn set_window_plan(&mut self, plan: &[(i32, i32, u64)]) {
        let mut changed: Vec<u32> = Vec::new();
        for &(x, y, lights) in plan {
            let key = crate::windows::obj_key(x, y);
            if self.windows.set_lights(key, lights) {
                changed.push(key);
            }
        }
        self.window_plan.clear();
        self.window_plan.extend_from_slice(plan);
        if !changed.is_empty() {
            self.refresh_windows(Some(&changed));
        }
    }

    /// B72: re-apply the stored plan (after the city was rebuilt).
    fn apply_window_plan(&mut self) {
        for i in 0..self.window_plan.len() {
            let (x, y, lights) = self.window_plan[i];
            let key = crate::windows::obj_key(x, y);
            self.windows.set_lights(key, lights);
        }
    }

    /// B72: hand a kit pane to its building's record.
    fn windows_kit_add(&mut self, key: u32, p: crate::windows::KitPane) {
        self.windows.add_kit(key, p);
    }

    /// B72: how many switchable window panes the city has, and how many are lit
    /// (tools and handbacks).
    pub fn window_counts(&self) -> (usize, usize) {
        (self.windows.pane_count(), self.windows.lit_count())
    }

    /// B72: (anchor x, y, panes, lit panes) per building with windows (tools).
    pub fn window_objs(&self) -> Vec<((i32, i32), usize, usize)> {
        self.windows
            .objs
            .iter()
            .filter(|o| !o.panes.is_empty() || !o.kit.is_empty())
            .map(|o| {
                let lit = o.panes.iter().filter(|p| p.lit).count() + o.kit.iter().filter(|p| p.lit).count();
                (
                    ((o.key >> 16) as i32, (o.key & 0xFFFF) as i32),
                    o.panes.len() + o.kit.len(),
                    lit,
                )
            })
            .collect()
    }

    /// B72: where a lot's foot sits on screen (tools).
    pub fn lot_screen(&self, x: i32, y: i32, level: u8) -> (f32, f32) {
        self.cam
            .project(x as f32, y as f32, level as i32 * crate::iso::HEIGHT_UNIT)
    }

    /// B72: how many lit panes carry a spill, and how many spill pixels.
    pub fn spill_stats(&self) -> (usize, usize) {
        let mut panes = 0;
        let mut px = 0;
        for o in &self.windows.objs {
            for p in &o.panes {
                if !p.spill.is_empty() {
                    panes += 1;
                    px += p.spill.len();
                }
            }
            for p in &o.kit {
                if !p.spill.is_empty() {
                    panes += 1;
                    px += p.spill.len();
                }
            }
        }
        (panes, px)
    }

    /// B72: (unplanned objects with panes, their panes) — the horizon belt.
    pub fn window_unplanned(&self) -> (usize, usize) {
        self.windows
            .objs
            .iter()
            .filter(|o| !o.planned)
            .fold((0, 0), |(n, p), o| (n + 1, p + o.panes.len() + o.kit.len()))
    }

    /// B72: panes switched (a light on or off) since the scene was built.
    pub fn window_switches(&self) -> u64 {
        self.windows.switches
    }

    /// B72: bring every pane (or the ones listed) to the light its building's
    /// lights and the sky's darkness ask for. A pane that does not change costs
    /// a hash and a compare — nothing is repainted, nothing is damaged.
    fn refresh_windows(&mut self, only: Option<&[u32]>) {
        if self.windows.is_empty() {
            return;
        }
        let night = crate::windows::night_of(self.glow);
        let lut = crate::light::lut(self.tint);
        let bulbs: Vec<Color> = self.holiday.garland[..self.holiday.bulbs.max(1)].to_vec();
        let mut rects: Vec<Rect> = Vec::new();
        for oi in 0..self.windows.objs.len() {
            let key = self.windows.objs[oi].key;
            if let Some(only) = only {
                if !only.contains(&key) {
                    continue;
                }
            }
            let lights = if self.windows.objs[oi].planned {
                self.windows.objs[oi].lights
            } else {
                // The horizon belt: no lots, no residents — its own plan.
                crate::windows::belt_lights(key, self.light.hour)
            };
            let force = self.windows.objs[oi].force;
            let n = self.windows.objs[oi].panes.len();
            for pi in 0..n {
                let (face, mut pane) = {
                    let p = &self.windows.objs[oi].panes[pi];
                    (self.windows.objs[oi].faces[p.face as usize], p.clone())
                };
                let want = face.pane_lit(lights, night, pane.col as i32, pane.row as i32);
                if want == pane.lit && !force {
                    continue;
                }
                let mut spill = std::mem::take(&mut pane.spill);
                let painted = self.paint_engine_pane(&face, &pane, want, &lut, &bulbs, &mut spill);
                pane.spill = spill;
                if let Some(r) = painted {
                    rects.push(r);
                    self.windows.count_switch();
                }
                pane.lit = want;
                self.windows.objs[oi].panes[pi] = pane;
            }
            let nk = self.windows.objs[oi].kit.len();
            for ki in 0..nk {
                let want = {
                    let p = &self.windows.objs[oi].kit[ki];
                    p.want(lights, night)
                };
                if want == self.windows.objs[oi].kit[ki].lit && !force {
                    continue;
                }
                let mut pane = self.windows.objs[oi].kit[ki].clone();
                let mut spill = std::mem::take(&mut pane.spill);
                let painted = self.paint_kit_pane(&pane, want, night, &lut, &bulbs, &mut spill);
                pane.spill = spill;
                if let Some(r) = painted {
                    rects.push(r);
                    self.windows.count_switch();
                }
                pane.lit = want;
                self.windows.objs[oi].kit[ki] = pane;
            }
            if force {
                self.windows.objs[oi].force = false;
            }
        }
        if rects.len() > 400 {
            self.mark_full();
        } else {
            for r in rects {
                self.mark_rect(r);
            }
        }
    }

    /// B72: switch one engine-drawn pane. Every pixel is written only when the
    /// layer still holds this pane's own colour there (lit or dark), so a
    /// building drawn in front of it is never clobbered.
    fn paint_engine_pane(
        &mut self,
        face: &crate::windows::Face,
        pane: &crate::windows::Pane,
        want: bool,
        lut: &crate::light::Lut,
        bulbs: &[Color],
        spill: &mut Vec<(u32, Color)>,
    ) -> Option<Rect> {
        use crate::windows::Hit;
        let (col, row) = (pane.col as i32, pane.row as i32);
        let r = face
            .cell_rect(col, row)
            .intersect(&self.land_clip());
        if r.is_empty() {
            return None;
        }
        let w = self.width();
        let mut any = false;
        let mut done = Rect::new(r.x0, r.y0, r.x0, r.y0);
        for y in r.y0..r.y1 {
            for x in r.x0..r.x1 {
                let Some((up, vp)) = face.map.at(x, y) else {
                    continue;
                };
                if face.cell_of(up, vp) != (col, row) {
                    continue;
                }
                let (Hit::Ink(old), Hit::Ink(new)) =
                    (face.pane_px(up, vp, pane.lit), face.pane_px(up, vp, want))
                else {
                    continue;
                };
                if old == new {
                    continue;
                }
                let i = y as usize * w + x as usize;
                let cur = self.static_layer.px()[i] & 0x00FF_FFFF;
                // Only this pane's own pixels: dark or lit, nothing else.
                if cur != (old & 0x00FF_FFFF) && cur != (new & 0x00FF_FFFF) {
                    continue;
                }
                let tagged = palette::with_alpha(new, crate::framebuffer::TAG_BUILT);
                self.static_layer.px_mut()[i] = tagged;
                self.tinted_static.px_mut()[i] = night_pixel(tagged, lut, bulbs);
                if !any {
                    done = Rect::new(x, y, x + 1, y + 1);
                    any = true;
                } else {
                    done = done.union(&Rect::new(x, y, x + 1, y + 1));
                }
            }
        }
        // B72 round 2: the light on the wall/ground under a lit window. Cheap,
        // and it is what makes a house read as glowing at 1x rather than as a
        // bright rectangle stuck on a dark wall.
        // Below the window's own box, never on it: three rows of wall, sill or
        // pavement in front of it.
        let spill_rect = Rect::new(r.x0, r.y1 + 1, r.x1, r.y1 + 4);
        let spill = if want {
            self.spill_on(&spill_rect, spill, &lut, &bulbs)
        } else if !spill.is_empty() {
            self.spill_off(spill, &lut, &bulbs)
        } else {
            None
        };
        if let Some(sr) = spill {
            if !any {
                done = sr;
                any = true;
            } else {
                done = done.union(&sr);
            }
        }
        if any {
            Some(done)
        } else {
            None
        }
    }

    /// B72 round 2: light a window's spill — a soft warm wash on the wall or
    /// ground under it, three rows deep and two pixels wider than the pane. The
    /// pixels it covers are remembered, so switching the light off restores them
    /// exactly (and a nearer building's pixels are never touched: a pixel that
    /// is not "empty ground" is left alone).
    fn spill_on(
        &mut self,
        r: &Rect,
        store: &mut Vec<(u32, Color)>,
        lut: &crate::light::Lut,
        bulbs: &[Color],
    ) -> Option<Rect> {
        let r = r.intersect(&self.land_clip());
        if r.is_empty() {
            return None;
        }
        let w = self.width();
        let mut any = false;
        let mut done = Rect::new(r.x0, r.y0, r.x0, r.y0);
        let mid = (r.x0 + r.x1) as f32 * 0.5;
        let half = ((r.x1 - r.x0) as f32 * 0.5).max(1.0);
        for y in r.y0..r.y1 {
            // Dimmer with every row away from the window, and towards its sides,
            // so it reads as a wash of light rather than a stamped rectangle.
            let row = 1.0 - 0.30 * (y - r.y0) as f32 / (r.y1 - r.y0).max(1) as f32;
            for x in r.x0..r.x1 {
                let side = 1.0 - 0.55 * ((x as f32 + 0.5 - mid).abs() / half).clamp(0.0, 1.0);
                let fall = row * side;
                let i = y as usize * w + x as usize;
                let cur = self.static_layer.px()[i];
                let rgb = cur & 0x00FF_FFFF;
                let dark = (rgb >> 16) + (rgb >> 8 & 0xFF) + (rgb & 0xFF) < 300;
                // Only a dark surface takes the spill — never a roof, a tree, a
                // bright wall, or something drawn over the window.
                if !dark || fall < 0.25 {
                    continue;
                }
                let warm = crate::palette::mix(rgb | 0xFF00_0000, pal::WINDOW_LIT, 0.42 * fall);
                if store.len() < 96 {
                    store.push((i as u32, cur));
                }
                // Emissive (0xFE): the night's ambient multiply must not eat the
                // very light it is the shadow of. The spill is light, not paint.
                let tagged = crate::palette::with_alpha(warm, 0xFE);
                self.static_layer.px_mut()[i] = tagged;
                self.tinted_static.px_mut()[i] = night_pixel(tagged, lut, bulbs);
                if !any {
                    done = Rect::new(x, y, x + 1, y + 1);
                    any = true;
                } else {
                    done = done.union(&Rect::new(x, y, x + 1, y + 1));
                }
            }
        }
        if any {
            Some(done)
        } else {
            None
        }
    }

    /// B72 round 2: put a window's spill back exactly as it was.
    fn spill_off(
        &mut self,
        store: &mut Vec<(u32, Color)>,
        lut: &crate::light::Lut,
        bulbs: &[Color],
    ) -> Option<Rect> {
        if store.is_empty() {
            return None;
        }
        let mut any = false;
        let mut done = Rect::new(i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for &(i, c) in store.iter() {
            let (x, y) = ((i as usize % self.width()) as i32, (i as usize / self.width()) as i32);
            self.static_layer.px_mut()[i as usize] = c;
            self.tinted_static.px_mut()[i as usize] = night_pixel(c, lut, bulbs);
            if !any {
                done = Rect::new(x, y, x + 1, y + 1);
                any = true;
            } else {
                done = done.union(&Rect::new(x, y, x + 1, y + 1));
            }
        }
        store.clear();
        if any {
            Some(done)
        } else {
            None
        }
    }

    /// B72: switch one kit-sprite pane. Its dark pixels come from the sprite
    /// itself, so a window that goes out is exactly the art it was baked as.
    fn paint_kit_pane(
        &mut self,
        p: &crate::windows::KitPane,
        want: bool,
        night: f32,
        lut: &crate::light::Lut,
        bulbs: &[Color],
        spill: &mut Vec<(u32, Color)>,
    ) -> Option<Rect> {
        let r = p.rect().intersect(&self.land_clip());
        if r.is_empty() {
            return None;
        }
        let w = self.width();
        let mut any = false;
        let mut done = Rect::new(r.x0, r.y0, r.x0, r.y0);
        for &(px, py) in &p.px {
            let x = p.ox as i32 + px as i32;
            let y = p.oy as i32 + py as i32;
            if x < r.x0 || x >= r.x1 || y < r.y0 || y >= r.y1 {
                continue;
            }
            let sx = px as usize;
            let sy = py as usize;
            let dark = self
                .kit
                .get(p.spr as usize)
                .filter(|k| sx < k.w && sy < k.h)
                .map(|k| k.px[sy * k.w + sx]);
            let Some(dark) = dark else { continue };
            if dark == crate::palette::transparent() {
                continue;
            }
            let col = if want {
                crate::windows::kit_lit_colour(p.warm, night, 0.35)
            } else {
                dark
            };
            let i = y as usize * w + x as usize;
            let cur = self.static_layer.px()[i] & 0x00FF_FFFF;
            // Only this pane's own pixels: its dark glass or its light.
            let ink = crate::palette::with_alpha(col, 0xFF);
            if cur != (dark & 0x00FF_FFFF) && cur != (ink & 0x00FF_FFFF) {
                continue;
            }
            self.static_layer.px_mut()[i] =
                crate::palette::with_alpha(col, crate::framebuffer::TAG_BUILT);
            self.tinted_static.px_mut()[i] = night_pixel(col, lut, bulbs);
            if !any {
                done = Rect::new(x, y, x + 1, y + 1);
                any = true;
            } else {
                done = done.union(&Rect::new(x, y, x + 1, y + 1));
            }
        }
        // B72 round 2: the same warm wash under a kit window.
        let sr = Rect::new(r.x0, r.y1 + 1, r.x1, r.y1 + 4);
        let oily = if want {
            self.spill_on(&sr, spill, lut, bulbs)
        } else if !spill.is_empty() {
            self.spill_off(spill, lut, bulbs)
        } else {
            None
        };
        if let Some(sr) = oily {
            if !any {
                done = sr;
                any = true;
            } else {
                done = done.union(&sr);
            }
        }
        if any {
            Some(done)
        } else {
            None
        }
    }

    /// Recompute the per-row opacity of the tinted static layer (D71).
    fn refresh_opaque_rows(&mut self) {
        let w = self.width();
        let rows = self.height();
        // B84: rows above the land line are sky, not layer: false, and not read
        // (the composite never copies the static layer there).
        let row0 = self.static_row0().min(rows);
        self.static_opaque.clear();
        self.static_opaque.reserve(rows);
        self.static_opaque.resize(row0, false);
        let px = self.tinted_static.px();
        for y in row0..rows {
            let row = &px[y * w..(y + 1) * w];
            self.static_opaque.push(row.iter().all(|&p| p != TRANSPARENT));
        }
    }

    /// B42f/B42g: the warm pools the installed street lights throw on the road
    /// and pavement, brighter where several lamps stand close together.
    ///
    /// Painted into the **static** layer (where the ground/prop/building tags
    /// still exist, so a pool cannot light a roof or a tree crown — that is what
    /// went wrong in B42f), and each pool pixel is tagged emissive (`0xFE`) so
    /// the night tint keeps it bright without the pools stacking on every
    /// re-tint.
    fn draw_lamp_pools(&mut self, rect: Rect) {
        if self.glow <= 0.35 || self.street.lamps.is_empty() {
            return;
        }
        let rect = rect.intersect(&self.land_clip());
        if rect.is_empty() {
            return;
        }
        let glow = self.glow;
        let mut pools: Vec<(i32, i32, u8)> = Vec::new();
        for &(x, y) in self.street.lamps.iter() {
            // B52: a blackout's lamps throw no light.
            if self.civic.stat.is_dark(x, y) {
                continue;
            }
            let (sx, sy) = self.cam.project(x as f32 + 0.5, y as f32 + 0.5, 0);
            let b = Rect::new(sx as i32 - 34, sy as i32 - 34, sx as i32 + 34, sy as i32 + 24);
            if !b.intersects(&rect) {
                continue;
            }
            let mut near = 0u8;
            for dy in -2..=2 {
                for dx in -2..=2 {
                    if (dx != 0 || dy != 0) && self.street.lamps.contains(&(x + dx, y + dy)) {
                        near = near.saturating_add(1);
                    }
                }
            }
            pools.push((x, y, near));
        }
        if pools.is_empty() {
            return;
        }
        let cam = self.cam;
        let map = &self.map;
        let fb = &mut self.static_layer;
        fb.set_clip(rect);
        let levels = self.tech.lamps.clone();
        let base = self.tech.lamp_base;
        for (x, y, near) in pools {
            let level = levels.get(&(x, y)).copied().unwrap_or(base);
            crate::street::draw_lamp_pool(fb, &cam, map, x, y, glow, near, level);
        }
        fb.reset_clip();
    }

    /// Per-row haze amount, 0 far down the map and **1.0 only at the very back
    /// row** (B42b). Because it reaches 1.0 exactly where the map meets the sky
    /// band, the last row of ground *is* the colour painted behind it and there
    /// is no seam to see — the old model started at full strength, which read as
    /// a flat slab with a hard top edge.
    /// The ground's own haze, and past it the far water still greys and
    /// flattens towards the horizon: a boat there is a distant boat (B91).
    fn boat_haze_rows(&self) -> Vec<f32> {
        let mut rows = self.haze_profile();
        let reach = ((self.height() - self.sky_h) as f32 * 0.22).clamp(60.0, 220.0);
        for (y, slot) in rows.iter_mut().enumerate() {
            // Rows above the sky line (the belt's far water) are the furthest of all.
            let d = (y as f32 - self.sky_h as f32).max(0.0);
            let far = (1.0 - d / reach).max(0.0);
            let top = if y < self.sky_h { 0.95 } else { 0.85 };
            *slot = slot.max(far.powf(1.3) * top);
        }
        rows
    }

    fn haze_profile(&self) -> Vec<f32> {
        let h = self.height();
        let mut out = vec![0.0f32; h];
        if self.sky_h == 0 || h <= self.sky_h {
            return out;
        }
        let deep = self.haze_depth();
        for (k, slot) in out.iter_mut().enumerate().skip(self.sky_h) {
            let d = (k - self.sky_h) as f32;
            if d >= deep {
                break;
            }
            // A high power: nearly nothing for most of the band, then a firm
            // fade only in the last few rows, where the far wood is.
            let u = 1.0 - d / deep;
            let u2 = u * u;
            *slot = u2 * u2 * u2 * u2;
        }
        out
    }

    /// How deep the haze band runs, in rows below the horizon. Deep enough to
    /// read as distance rather than a band; a third of the land is plenty.
    fn haze_depth(&self) -> f32 {
        let land = (self.height() - self.sky_h) as f32;
        (land * 0.55).clamp(70.0, 320.0)
    }

    pub fn set_city(&mut self, objects: Vec<CityObj>) {
        // B61d: the wallpaper hands the city over on every frame while anything
        // is being built (a site's crews move on sim time), and this used to
        // dirty and re-rasterise the whole footprint of every lot each time —
        // 1385 lots at day 25, ~5 ms a frame of terrain and buildings repainted
        // for a city that had not changed. The object's `t` is the only field
        // that moves per frame and it does not reach the picture (`life()`, the
        // one reader, is not drawn), so an otherwise identical list is a no-op.
        if self.city.len() == objects.len()
            && self
                .city
                .iter()
                .zip(objects.iter())
                .all(|(a, b)| a.same_lot(b))
        {
            return;
        }
        // Dirty the tiles of both the old and the new set — but only the lots
        // that actually changed (B78: an ageing district changes a handful of
        // buildings an hour, and dirtying all 1 200 lots for that would repaint
        // the whole static layer).
        let mut dirty: Vec<(i32, i32)> = Vec::new();
        let n = self.city.len().max(objects.len());
        for i in 0..n {
            let (a, b) = (self.city.get(i), objects.get(i));
            if let (Some(a), Some(b)) = (a, b) {
                if a.same_lot(b) {
                    continue;
                }
            }
            for o in a.into_iter().chain(b.into_iter()) {
                for ty in o.y - 1..o.y + o.h + 1 {
                    for tx in o.x - 1..o.x + o.w + 1 {
                        dirty.push((tx, ty));
                    }
                }
            }
        }
        let soft = self.install_city(objects);
        for (x, y) in dirty.into_iter().chain(soft) {
            self.dirty_tile(x, y);
        }
        // Objects live in the static layer, so the repaint has to happen here:
        // forgetting this is what made H6 render roads with no buildings.
        self.flush_dirty();
    }

    /// The kit sprites the scene is drawing (B76 r3: a debug/report hook).
    pub fn kit_sprites(&self) -> &[KitSprite] {
        &self.kit
    }

    /// Hand the scene pre-rendered building sprites (B33). Call before
    /// `set_city`: the objects index into this list.
    pub fn set_kit_sprites(&mut self, kit: Vec<KitSprite>) {
        self.kit = kit;
        self.field_dirty = true;
        self.rebuild_static();
        self.retint_static();
        self.mark_full();
    }

    /// Append sprites built since the last call (B34). Indices already handed to
    /// objects stay valid, and nothing is redrawn until the caller's `set_city`
    /// repaints the tiles that use them.
    pub fn push_kit_sprites(&mut self, extra: Vec<KitSprite>) {
        self.kit.extend(extra);
        self.field_dirty = true;
    }

    /// How many sprites the scene holds (B34: the caller keeps its own cache in
    /// step with this).
    pub fn kit_sprite_count(&self) -> usize {
        self.kit.len()
    }

    /// Set the pre-rendered building sprites and the city objects together and
    /// rebuild the static layer **once** (B34: startup used to rebuild it three
    /// times — once for the terrain, once when the sprites arrived and once for
    /// the city, which is most of the startup CPU burst).
    pub fn set_city_with_sprites(&mut self, kit: Vec<KitSprite>, objects: Vec<CityObj>) {
        self.kit = kit;
        let _ = self.install_city(objects);
        self.rebuild_static();
        self.retint_static();
        self.mark_full();
    }

    /// Install the objects and their painter-diagonal index (no drawing).
    fn install_city(&mut self, objects: Vec<CityObj>) -> Vec<(i32, i32)> {
        // B90: how high the tallest building reaches (rows above stay unscanned).
        let base_top = (self.sky_h as i32 - HORIZON_FADE - 160).max(0);
        let mut top = base_top;
        for o in objects.iter() {
            top = top.min(obj_bounds(&self.cam, o).y0);
        }
        self.above_top = top.max(self.land_top());
        let diags = self.city_diag.len();
        self.city_diag = vec![Vec::new(); diags];
        for (i, o) in objects.iter().enumerate() {
            // Sprites of the kit are painted in column slices (B100); engine
            // art keeps its one diagonal (it stamps its own depth keys).
            for sl in city_slices(&self.cam, o, i as u32) {
                let d = sl.d.max(0) as usize;
                if d < diags {
                    self.city_diag[d].push(sl);
                }
            }
        }
        // Overhang pieces first (stable): see `CitySlice::ext`.
        for list in self.city_diag.iter_mut() {
            list.sort_by_key(|s| !s.ext);
        }
        // B43: the key each plot paints itself with, per tile, so a figure
        // standing on it can be drawn over the plot (a park has no walls).
        let (w, h) = (self.map.w, self.map.h);
        if self.city_key.len() != (w * h) as usize {
            self.city_key = vec![0; (w * h) as usize];
        }
        self.city_key.iter_mut().for_each(|k| *k = 0);
        for o in objects.iter() {
            // B65: what a figure standing on this plot is lifted to.
            //
            // A finished building's plot is lifted only to its *ground* key: the
            // figure may paint over the yard or the lawn, but the walls are a
            // solid a step above it, so a walker on the square under the clock
            // tower stays behind the shaft. Everything else — parks, plazas,
            // and every site under construction, which is open framework rather
            // than a wall — is lifted to the plot's building key, because there
            // the plot *is* the thing the figure has to stand in front of.
            // A car place's ground paints with a key under its first tile, so
            // the cars standing on it are drawn over the asphalt but behind the
            // canopy or the kiosk (those stamp their own keys, B59b).
            let solid = o.stage == Stage::Finished && !open_ground(o.kind) && !o.kind.is_car_place();
            // B100: a solid building's walls are painted in column slices, so a
            // figure on its plot is lifted to the ground key of the slice over
            // its own tile (for a square lot that is the far corner's, as
            // before; for a long lot it is the diagonal of the nearest ground
            // in that column).
            let slices = if solid { city_slices(&self.cam, o, 0) } else { Vec::new() };
            let d_at = |px: i32| slices.iter().find(|s| px >= s.x0 && px < s.x1).map(|s| s.d);
            let flat_key = if o.kind.is_car_place() {
                static_key(o.x + o.y - 1, true)
            } else {
                static_key(o.diagonal(), true)
            };
            let x0 = o.x.max(0);
            let y0 = o.y.max(0);
            let x1 = (o.x + o.w).min(w);
            let y1 = (o.y + o.h).min(h);
            for y in y0..y1 {
                for x in x0..x1 {
                    let i = (y * w + x) as usize;
                    let key = if solid {
                        let cx = self.cam.ox + (x - y) * 32;
                        let d = d_at(cx - 16).max(d_at(cx + 16)).unwrap_or(o.diagonal());
                        static_key(d, false)
                    } else {
                        flat_key
                    };
                    if key > self.city_key[i] {
                        self.city_key[i] = key;
                    }
                }
            }
        }
        self.city = objects;
        // B73/D80: the nightlife plan, from the district ids the sim already put
        // on the lots. It is a city-change cost, never a per-frame one.
        let land = crate::nightlife::Land::of(self.map.biome);
        // A/B switch for the handback's before/after numbers: with no plan the
        // per-frame cost is the one B99d measured.
        let build_t = std::time::Instant::now();
        self.night = if std::env::var("CITYLIVE_NO_SIGNS").is_ok() {
            crate::nightlife::Plan::default()
        } else {
            crate::nightlife::build_in(
                &self.city,
                &self.kit,
                &self.cam,
                land,
                self.width() as i32,
                self.height() as i32,
                self.town_stats,
                Some(&self.map),
            )
        };
        if std::env::var("CITYLIVE_SIGN_DEBUG").is_ok() {
            eprintln!(
                "nightlife: plan built in {:.2} ms for {} lots",
                build_t.elapsed().as_secs_f64() * 1000.0,
                self.city.len()
            );
        }
        self.sign_slot = -1;
        self.spec_invalidate();
        // B43c: the plant for every site, and the old plant's pixels dirty.
        let old: Vec<Rect> = self.rigs.iter().map(|r| r.bounds).collect();
        for r in old {
            self.mark_rect(r);
        }
        self.rigs = crate::rigs::rigs_for(&self.city, &self.cam, &self.map);
        let new: Vec<Rect> = self.rigs.iter().map(|r| r.bounds).collect();
        for r in new {
            self.mark_rect(r);
        }
        // B60: buildings are casters, so the height field is stale now.
        self.field_dirty = true;
        // B70: lots pin the ground under them.
        let soft = self.refresh_soft();
        // B74: the derived trees and the cleaning claims follow the lots.
        self.rebuild_planting();
        soft
    }

    /// B74: rebuild the draw-time claims bitmap and the derived garden/street
    /// trees from the current map, city and fixtures. Cheap (a few scans), and
    /// idempotent, so it runs whenever the city or the fixtures change.
    fn rebuild_planting(&mut self) {
        // B74 A/B: `CITYLIVE_NO_DRAWN_TREES` renders the frame as if the
        // draw-time cleaning and the derived trees were absent (the "before" of
        // a before/after crop).
        if std::env::var("CITYLIVE_NO_DRAWN_TREES").is_ok() {
            self.claims.clear();
            self.derived.clear();
            return;
        }
        self.claims = crate::planting::build_claims(&self.map, &self.city, &self.fixtures);
        self.derived = crate::planting::derive_trees(&self.map, &self.city, &self.claims);
    }

    /// B74: how many derived trees the frame is drawing, and how many *stored*
    /// terrain props draw-time cleaning skipped — the numbers behind the "trees
    /// back, nothing on the roads/rails/walls" claim.
    pub fn planting_stats(&self) -> (usize, usize) {
        let derived = self.derived.iter().filter(|p| p.is_some()).count();
        let mut skipped = 0usize;
        for y in 0..self.map.h {
            for x in 0..self.map.w {
                let prop = self.map.at(x, y).prop;
                if prop != map::Prop::None && crate::planting::skip_prop(&self.map, &self.claims, x, y, prop)
                {
                    skipped += 1;
                }
            }
        }
        (derived, skipped)
    }

    /// Build a scene on top of an existing map (the sim owns its terrain).
    pub fn from_map(map: TileMap, cfg: SceneConfig) -> Self {
        Self::from_map_with_sky(map, cfg, 0.0)
    }

    /// The sim's map, with a sky band at the top (D43).
    pub fn from_map_with_sky(map: TileMap, cfg: SceneConfig, sky_frac: f32) -> Self {
        let mut scene = Scene::new_with_map(cfg, Some(map), sky_frac);
        scene.city.clear();
        scene
    }

    /// As [`Self::from_map_with_sky`] but with **no** static rebuild: the caller
    /// sets the light, the fixtures and the city, then calls
    /// [`Self::set_city_with_sprites`] for one rebuild instead of four (B34
    /// startup burst).
    pub fn from_map_deferred(map: TileMap, cfg: SceneConfig, sky_frac: f32) -> Self {
        let mut scene = Scene::new_with_map_deferred(cfg, Some(map), sky_frac, (0, 0));
        scene.city.clear();
        scene
    }

    /// A scene whose camera is moved by `shift` pixels before the first bake
    /// (B59b: a tool that stills a place outside the wallpaper's strip; the old
    /// `set_camera` re-bake left stale bands). The sim's map is unchanged.
    pub fn from_map_shifted(
        map: TileMap,
        cfg: SceneConfig,
        sky_frac: f32,
        shift: (i32, i32),
    ) -> Self {
        let mut scene = Scene::new_with_map_deferred(cfg, Some(map), sky_frac, shift);
        scene.rebuild_static();
        scene.retint_static();
        scene.city.clear();
        scene
    }

    /// A checksum of the shadow height field. It is geometry only, so it must
    /// never change when just the light does (B60/D71).
    pub fn terrain_field_checksum(&self) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for v in self.field.ground.iter().chain(self.field.occl.iter()) {
            h ^= (*v as i32) as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01B3);
        }
        h
    }

    /// Height in pixels of the kit sprite a city object draws, if it has one
    /// (tools and tests; the shadow field's caster height).
    pub fn kit_height(&self, o: &CityObj) -> Option<f32> {
        if o.kit == 0 {
            return None;
        }
        self.kit.get(o.kit as usize - 1).map(|k| k.oy as f32)
    }

    /// The kit sprite a city object draws (tools and tests).
    pub fn kit_sprite(&self, o: &CityObj) -> Option<&KitSprite> {
        if o.kit == 0 {
            return None;
        }
        self.kit.get(o.kit as usize - 1)
    }

    /// The shadow height field (tools and tests).
    pub fn height_field(&self) -> &HeightField {
        &self.field
    }

    /// The city objects the scene is drawing (tools and tests).
    pub fn city_objects(&self) -> &[CityObj] {
        &self.city
    }

    /// B100: when the piece of object `i` that paints screen column `x` is
    /// painted, as a number that grows with paint order (tools and tests; the
    /// order audit compares two of them).
    pub fn paint_rank(&self, i: usize, x: i32) -> u64 {
        let o = &self.city[i];
        let sl = city_slices(&self.cam, o, i as u32);
        let s = sl.iter().find(|s| x >= s.x0 && x < s.x1).unwrap_or(&sl[0]);
        ((s.d.max(0) as u64) << 32) | ((!s.ext) as u64) << 31 | i as u64
    }

    pub fn static_layer(&self) -> &Framebuffer {
        &self.static_layer
    }

    /// The static layer with the ambient light and the distance haze baked in.
    pub fn tinted_layer(&self) -> &Framebuffer {
        &self.tinted_static
    }

    pub fn sky(&self) -> &Framebuffer {
        &self.sky
    }

    /// B61e: the positions of everything that never moves (the animated water,
    /// the street lamps), and its row index. These are screen positions, so they
    /// are built with the static layer and never per frame.
    fn rebuild_place_index(&mut self) {
        let h = self.height() as i32;
        self.water_px.clear();
        self.water_prims.clear();
        self.water_px.reserve(self.water_tiles.len());
        for &(x, y) in &self.water_tiles {
            let (sx, sy) = self.cam.project(x as f32, y as f32, 0);
            self.water_px.push((x, y, sx as i32, sy as i32));
        }
        self.water_ground.clear();
        {
            let pair = crate::biome::Pair::of(&self.map);
            self.water_ground
                .extend(self.water_tiles.iter().map(|&(x, y)| pair.at(&self.map, x, y)));
        }
        self.water_coast.clear();
        self.water_coast
            .extend(self.water_tiles.iter().map(|&(x, y)| terrain::coast_for(&self.map, x, y)));
        {
            let cols = self.width() / 64 + 2;
            self.sea_cols.clear();
            self.sea_cols.resize(cols, 0);
            for &(x, y, sx, _) in &self.water_px {
                if matches!(self.map.water_kind(x, y), map::WaterKind::Sea | map::WaterKind::Lagoon) {
                    let b = (sx.max(0) / 64) as usize;
                    if b < cols {
                        self.sea_cols[b] = self.sea_cols[b].saturating_add(1);
                    }
                }
            }
            self.sync_water_glint();
        }
        let px = std::mem::take(&mut self.water_px);
        // B87: in ascending screen x inside a band, so a damaged box finds
        // its few tiles by bisection (`Bands::for_each_x`).
        let mut order: Vec<(i32, i32, u32)> = px
            .iter()
            .enumerate()
            .map(|(i, p)| ((p.3.max(0) / crate::bands::BAND_H).min((h.max(1) - 1) / crate::bands::BAND_H), p.2, i as u32))
            .collect();
        order.sort_unstable();
        self.water_bands
            .rebuild(h, order.iter().map(|&(b, _, i)| (b * crate::bands::BAND_H, i)));
        self.water_px = px;

        self.lamp_px.clear();
        self.lamp_px.reserve(self.street.lamps.len());
        for &(x, y) in self.street.lamps.iter() {
            let (sx, sy) = self.cam.project(x as f32 + 0.5, y as f32 + 0.5, 0);
            self.lamp_px.push((x, y, sx as i32, sy as i32));
        }
        let px = std::mem::take(&mut self.lamp_px);
        self.lamp_bands
            .rebuild(h, px.iter().enumerate().map(|(i, p)| (p.3, i as u32)));
        self.lamp_px = px;
    }

    /// The per-pixel painter keys of the static layer.
    pub fn depth(&self) -> &[u16] {
        self.static_layer.depth().unwrap_or(&[])
    }

    /// B85 r2: the water tiles' screen positions `(tile x, tile y, screen x,
    /// screen y)` for the rain's ring ripples (a drop landing on water makes a
    /// ripple, not a ground splash). Built with the static layer.
    pub fn water_px(&self) -> &[(i32, i32, i32, i32)] {
        &self.water_px
    }

    /// Full re-rasterisation of terrain + buildings (static layer and depth).
    pub fn rebuild_static(&mut self) {
        // B76 r2: a sliced pass in flight is cancelled — this one draws the
        // whole layer and will publish it itself.
        if self.rebuild_job.take().is_some() && self.sway_on {
            let _ = crate::sway::uninstall();
        }
        let mut t_dbg = Instant::now();
        if std::env::var("SNOW_TRACE").is_ok() {
            eprintln!(
                "rebuild_static: snow {:.2} city {} items {}",
                self.weather.snow_cover,
                self.city.len(),
                self.items.len()
            );
        }
        let mut mark = |what: &str, last: &mut Instant| {
            if std::env::var("B76_PHASE").is_ok() {
                eprintln!("rebuild {what}: {:.1} ms", last.elapsed().as_secs_f64() * 1000.0);
                *last = Instant::now();
            }
        };
        self.water_tiles.clear();
        for it in &self.items {
            if let Item::Tile(x, y) = *it {
                if self.map.at(x, y).is_water() {
                    self.water_tiles.push((x, y));
                }
            }
        }
        self.water_count = self.water_tiles.len();
        self.rebuild_place_index();
        let t0 = Instant::now();
        let last = self.viewport.w + self.viewport.h;
        let band_max = (cfg_bands(self.width()) - 1) as i32;
        let self_glow = self.glow;
        let self_water = self.water;
        let self_weather = self.weather;
        let self_season = self.season;
        let self_cleared = self.cleared.clone();
        let self_biome = self.biome;
        let self_fixtures = self.fixtures.clone();
        let self_street = self.street.clone();
        let self_tech = self.tech.clone();
        // B53: the gameday host (stadium, else the biggest park), by index.
        let host = gameday_host(&self.city);
        let self_dress = self.spec_dress();
        let self_fest_host = festival_host(&self.city);
        let self_civic = self.civic.stat;
        let mut kit_draws = 0usize;
        let mut engine_draws = 0usize;
        let self_containers = self.fixtures.containers.clone();
        let self_port = self.fixtures.port;
        let land_clip = self.land_clip();
        let seal_clip = self.seal_clip();
        let Scene {
            map,
            draw_map,
            landforms,
            buildings,
            cam,
            items,
            item_keys,
            item_bounds,
            index,
            city,
            city_diag,
            kit,
            holiday,
            market,
            static_layer,
            trees,
            city_key: self_city_key,
            claims: self_claims,
            derived: self_derived,
            ..
        } = self;
        static_layer.reset_clip();
        static_layer.set_clip(land_clip);
        static_layer.clear(TRANSPARENT);
        // B69: the swaying trees are captured, not painted.
        trees.reset(static_layer.w, static_layer.h, self.sky_h as i32);
        if self.sway_on {
            crate::sway::install(std::mem::take(trees));
        }
        let (drawn, visited) = draw_items(
            draw_map,
            landforms,
            buildings,
            cam,
            items,
            item_keys,
            item_bounds,
            index,
            city,
            city_diag,
            kit,
            &*holiday,
            *market,
            self_dress,
            self_fest_host,
            &mut kit_draws,
            &mut engine_draws,
            self_glow,
            self.light.hour,
            &self_water,
            &self_weather,
            &self_season,
            &self_cleared,
            &self_biome,
            &self_city_key,
            &self_claims,
            &self_derived,
            &self_fixtures,
            &self_street,
            &self_tech,
            &self_civic,
            static_layer,
            0,
            last,
            0,
            band_max,
            self.sky_h as i32,
        );
        if self.sway_on {
            self.trees = crate::sway::uninstall();
        }
        mark("draw items", &mut t_dbg);
        self.trees.release_scratch();
        self.trees.reindex();
        self.trees.take_damage(&mut Vec::new());
        // B58c: the airfield and the harbour quay are drawn as whole surfaces
        // after every tile. Per-tile drawing left a seam of grass between each
        // pair of asphalt tiles and between the quay's stones.
        if let Some(ap) = &self.fixtures.transit.airport {
            crate::transit::draw_airport(static_layer, cam, map, ap, self_glow);
        }
        if let Some(h) = &self.fixtures.transit.harbour {
            crate::transit::draw_harbour(static_layer, cam, map, h, self_glow);
        }
        for j in &self.fixtures.transit.jetties {
            crate::jetty::draw_jetty(static_layer, cam, map, j, self_glow);
        }
        // Containers stand on the quay: static, drawn over the tiles.
        for &(x, y, pick) in &self_containers {
            crate::rail::draw_container(static_layer, cam, map, x, y, pick);
        }
        if let Some(p) = self_port {
            crate::rail::draw_cranes(static_layer, cam, map, p);
        }
        // Seal the 1-px seams a slope can leave between two tile tops — on the
        // land only: above the land line a gap is the sky, not a seam (B65).
        mark("fixtures", &mut t_dbg);
        seal_seams(static_layer, seal_clip, 1);
        mark("seams", &mut t_dbg);
        // B42f/B42g: the street lights' pools on the ground, after every tile
        // (a neighbour painted later used to cover them) and while the ground /
        // prop / building tags are still readable here.
        if self_glow > 0.35 && !self_street.lamps.is_empty() {
            static_layer.set_clip(land_clip);
            for &(lx, ly) in self_street.lamps.iter() {
                let mut near = 0u8;
                for dy in -2..=2 {
                    for dx in -2..=2 {
                        if (dx != 0 || dy != 0) && self_street.lamps.contains(&(lx + dx, ly + dy)) {
                            near = near.saturating_add(1);
                        }
                    }
                }
                let level = self_tech.lamp_level(lx, ly);
                crate::street::draw_lamp_pool(static_layer, cam, map, lx, ly, self_glow, near, level);
            }
            static_layer.reset_clip();
        }

        self.trees.settle(self.static_layer.depth().unwrap_or(&[]), self.static_layer.px());
        mark("lamps+trees.settle", &mut t_dbg);
        // B72: the whole layer was just drawn, so the window field is rebuilt
        // from what this pass recorded, and the lit panes are painted on top of
        // the dark ones the wall pass left.
        let faces = self.static_layer.take_faces();
        let kit = self.static_layer.take_kit_panes();
        self.windows.clear();
        self.windows.add_faces(faces);
        for (key, p) in kit {
            let i = self.windows.objs.len();
            let _ = i;
            self.windows_kit_add(key, p);
        }
        self.apply_window_plan();
        self.refresh_windows(None);
        mark("windows", &mut t_dbg);
        self.stats.full_rebuild_ms = t0.elapsed().as_secs_f64() * 1000.0;
        self.stats.last_flush_draws = drawn;
        self.stats.last_flush_visited = visited;
        self.stats.kit_draws = kit_draws;
        self.stats.engine_draws = engine_draws;
        self.dirty.clear();
    }

    /// B84 r2: how far the sliced re-rasterise has got, as (diagonal done,
    /// last diagonal), or `None` when no pass is running. A tool that wants a
    /// mid-pass picture steps to a *fixed* diagonal, so the frame it captures
    /// is the same picture whatever the machine's timing.
    pub fn rebuild_progress(&self) -> Option<(i32, i32)> {
        self.rebuild_job.as_ref().map(|j| (j.d, j.last))
    }

    /// B76 r2: ask for a re-rasterise of the static layer.
    ///
    /// A weather, season or holiday step changes *colour only* — no wall, roof
    /// or tree moves — so the pass can be spread over frames: draw a few
    /// diagonals at a time (into the same layer, over the old pixels, which
    /// keeps the depth plane and the tags coherent for everything reading them
    /// mid-pass), then publish with the B67 sliced re-tint. A tool or a test
    /// that does not opt in still gets the whole thing at once.
    fn request_static_rebuild(&mut self) {
        if !self.async_shadow {
            self.rebuild_static();
            self.retint_static();
            self.mark_full();
            return;
        }
        self.retint_job = None;
        let last = self.viewport.w + self.viewport.h;
        let band_max = (cfg_bands(self.width()) - 1) as i32;
        self.rebuild_job = Some(RebuildJob {
            d: 0,
            last,
            bx: 0,
            band_max,
            kit_draws: 0,
            engine_draws: 0,
            drawn: 0,
            visited: 0,
            slices: 0,
            worst_ms: 0.0,
            fixtures: self.fixtures.clone(),
            city_key: self.city_key.clone(),
            claims: self.claims.clone(),
            derived: self.derived.clone(),
            street: self.street.clone(),
            tech: self.tech.clone(),
            civic: self.civic.stat,
            dress: self.spec_dress(),
            fest_host: festival_host(&self.city),
            land_clip: self.land_clip(),
        });
        // The tree layer is captured fresh, so the old one keeps drawing the
        // trees until the pass is published.
        if self.sway_on {
            let mut fresh = crate::sway::TreeLayer::default();
            fresh.reset(self.width(), self.height(), self.sky_h as i32);
            crate::sway::install(fresh);
        }
        self.static_layer.reset_clip();
        self.static_layer.set_clip(self.land_clip());
        // The windows are unchanged by a colour-only pass: do not record them.
        self.static_layer.record_windows = false;
    }

    /// B76 r2: the winter service has just driven these tiles — clear them a
    /// level (two passes reach the asphalt), repaint them, and put the pushed
    /// snow along their kerb.
    pub fn plough_tiles(&mut self, tiles: &[(i32, i32)]) {
        let mut touched = false;
        for &(x, y) in tiles {
            if !self.map.at(x, y).kind.is_lane() {
                continue;
            }
            let lvl = self.cleared.0.get(&(x, y)).copied().unwrap_or(0);
            if lvl < 2 {
                self.cleared.0.insert((x, y), lvl + 1);
                touched = true;
            }
            // The kerb of a cleared lane keeps the heap of what was pushed
            // aside, so the edge is a drift and not a tile boundary.
            for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let (nx, ny) = (x + dx, y + dy);
                let n = self.map.at(nx, ny);
                if n.kind.is_lane() || n.is_water() {
                    continue;
                }
                let h = self.cleared.0.get(&(nx, ny)).copied().unwrap_or(0);
                if h < 2 {
                    self.cleared.0.insert((nx, ny), h.max(1).saturating_add(0));
                }
            }
        }
        if touched {
            // A plough is on the street: repaint the tiles it touched now (the
            // frame after, to keep the damage in one batch).
            for &(x, y) in tiles {
                for (dx, dy) in [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1)] {
                    self.dirty_tile(x + dx, y + dy);
                }
            }
        }
    }

    /// Drop the clearing (the snow has gone).
    pub fn clear_ploughed(&mut self) {
        if !self.cleared.is_empty() {
            self.cleared.clear();
            self.request_static_rebuild();
        }
    }

    /// Is a sliced static rebuild still running?
    pub fn static_rebuild_pending(&self) -> bool {
        self.rebuild_job.is_some()
    }

    /// Finish a sliced rebuild now (tools, tests and the spike's stills).
    pub fn flush_static(&mut self) {
        while self.static_rebuild_pending() {
            self.rebuild_work(1.0e9);
        }
        if self.retint_job.is_some() {
            while self.retint_work(1.0e9) {}
        }
    }

    /// Spend up to `budget_ms` on the running static rebuild. True while it has
    /// more to do.
    fn rebuild_work(&mut self, budget_ms: f64) -> bool {
        let Some(mut job) = self.rebuild_job.take() else {
            return false;
        };
        let t0 = Instant::now();
        let (w, h) = (self.width(), self.height());
        if self.static_layer.w != w || self.static_layer.h != h {
            // The layer changed shape under the job: do it the plain way.
            if self.sway_on {
                self.trees = crate::sway::uninstall();
            }
            self.rebuild_job = None;
            self.rebuild_static();
            self.retint_static();
            self.mark_full();
            return false;
        }
        let (glow, water, weather, season, biome) =
            (self.glow, self.water, self.weather, self.season, self.biome);
        let cleared = self.cleared.clone();
        let month_hour = self.light.hour;
        let sky_h = self.sky_h as i32;
        {
            let Scene {
                map,
                draw_map,
                landforms,
                buildings,
                cam,
                items,
                item_keys,
                item_bounds,
                index,
                city,
                city_diag,
                kit,
                holiday,
                market,
                static_layer,
                ..
            } = self;
            static_layer.reset_clip();
            static_layer.set_clip(job.land_clip);
            while job.d <= job.last {
                let (mut kd, mut ed) = (0usize, 0usize);
                let (drawn, visited) = draw_items(
                    draw_map,
                    landforms,
                    buildings,
                    cam,
                    items,
                    item_keys,
                    item_bounds,
                    index,
                    city,
                    city_diag,
                    kit,
                    &*holiday,
                    *market,
                    job.dress,
                    job.fest_host,
                    &mut kd,
                    &mut ed,
                    glow,
                    month_hour,
                    &water,
                    &weather,
                    &season,
                    &cleared,
                    &biome,
                    &job.city_key,
                    &job.claims,
                    &job.derived,
                    &job.fixtures,
                    &job.street,
                    &job.tech,
                    &job.civic,
                    static_layer,
                    job.d,
                    job.d,
                    job.bx,
                    (job.bx + SLICE_BANDS - 1).min(job.band_max),
                    sky_h,
                );
                job.kit_draws += kd;
                job.engine_draws += ed;
                job.drawn += drawn;
                job.visited += visited;
                job.bx += SLICE_BANDS;
                if job.bx > job.band_max {
                    job.bx = 0;
                    job.d += 1;
                }
                if t0.elapsed().as_secs_f64() * 1000.0 >= budget_ms {
                    break;
                }
            }
            static_layer.reset_clip();
        }
        job.slices += 1;
        if std::env::var("B76_REBUILD_DBG").is_ok() {
            eprintln!(
                "slice {} d {} -> {:.1} ms ({} kit, {} engine)",
                job.slices,
                job.d,
                t0.elapsed().as_secs_f64() * 1000.0,
                job.kit_draws,
                job.engine_draws
            );
        }
        job.worst_ms = job.worst_ms.max(t0.elapsed().as_secs_f64() * 1000.0);
        if job.d <= job.last {
            self.rebuild_job = Some(job);
            return true;
        }
        // --- done: keep the new tree layer, drain what a soft pass discards ---
        let t_tear = Instant::now();
        if self.sway_on {
            let mut fresh = crate::sway::uninstall();
            fresh.release_scratch();
            fresh.reindex();
            self.trees = fresh;
        }
        let t_trees = t_tear.elapsed().as_secs_f64() * 1000.0;
        // The geometry did not change: the live window field stays as it was.
        self.static_layer.record_windows = true;
        let t_lamps = Instant::now();
        // (No `seal_seams` here: a paint-over pass redraws the same tiles in
        // the same places, so the previous pass's seam fill is still under the
        // ones it rediscovered — sealing again cost 5 ms of the budget.)
        if self.glow > 0.35 && !self.street.lamps.is_empty() {
            self.static_layer.set_clip(self.land_clip());
            let lamps = self.street.lamps.clone();
            let tech = self.tech.clone();
            for &(lx, ly) in lamps.iter() {
                let mut near = 0u8;
                for dy in -2..=2 {
                    for dx in -2..=2 {
                        if (dx != 0 || dy != 0) && lamps.contains(&(lx + dx, ly + dy)) {
                            near = near.saturating_add(1);
                        }
                    }
                }
                let level = tech.lamp_level(lx, ly);
                crate::street::draw_lamp_pool(
                    &mut self.static_layer,
                    &self.cam,
                    &self.map,
                    lx,
                    ly,
                    self.glow,
                    near,
                    level,
                );
            }
            self.static_layer.reset_clip();
        }
        self.stats.last_flush_draws = job.drawn;
        self.stats.last_flush_visited = job.visited;
        self.stats.kit_draws = job.kit_draws;
        self.stats.engine_draws = job.engine_draws;
        if std::env::var("B76_REBUILD_DBG").is_ok() {
            eprintln!(
                "teardown: trees {:.1} ms, lamps {:.1} ms, total {:.1} ms",
                t_trees,
                t_lamps.elapsed().as_secs_f64() * 1000.0,
                t_tear.elapsed().as_secs_f64() * 1000.0
            );
        }
        self.stats.rebuild_worst_ms = self.stats.rebuild_worst_ms.max(job.worst_ms);
        self.stats.rebuild_slices = self.stats.rebuild_slices.saturating_add(job.slices);
        // Publish: the B67 sliced re-tint, which damages each band it finishes,
        // so neither the re-tint nor the composite of a whole frame lands on
        // one frame (`mark_full` here cost a 70 ms full composite).
        self.retint_soft();
        false
    }

    /// Conservative screen area a single tile can affect, including anything
    /// standing on it and the tallest hill behind it.
    fn tile_rect_at(&self, tx: i32, ty: i32, z: u8) -> Rect {
        let mut r = tile_bounds(&self.cam, tx, ty, [z as f32; 4]);
        r.y0 -= 24; // headroom for the tallest prop on the tile
        for b in &self.buildings {
            if tx >= b.tx && tx < b.tx + b.fw && ty >= b.ty && ty < b.ty + b.fh {
                r = r.union(&b.bounds);
            }
        }
        r.intersect(&self.static_layer.bounds())
    }

    /// B49: mark a whole lot's screen area (rooftop panels reach above the
    /// tile, so its anchor tile alone is not enough).
    pub fn dirty_lot(&mut self, x: i32, y: i32, w: i32, h: i32, level: u8) -> Rect {
        let base = level as i32 * crate::iso::HEIGHT_UNIT;
        let mut r = Rect::new(i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for (px, py) in [
            (x as f32, y as f32),
            ((x + w) as f32, y as f32),
            ((x + w) as f32, (y + h) as f32),
            (x as f32, (y + h) as f32),
        ] {
            for z in [base, base + 64] {
                let p = self.cam.project(px, py, z);
                r.x0 = r.x0.min(p.0.floor() as i32);
                r.y0 = r.y0.min(p.1.floor() as i32);
                r.x1 = r.x1.max(p.0.ceil() as i32);
                r.y1 = r.y1.max(p.1.ceil() as i32);
            }
        }
        let r = r.expanded(2).intersect(&self.static_layer.bounds());
        if !r.is_empty() {
            self.dirty.push(r);
        }
        r
    }

    /// Mark one tile's screen area for re-rasterisation.
    pub fn dirty_tile(&mut self, tx: i32, ty: i32) -> Rect {
        let z = self.map.at(tx, ty).max_level();
        let r = self.tile_rect_at(tx, ty, z);
        // A tile outside the framebuffer changes nothing: do not queue a
        // composite for it (the wallpaper's idle bug).
        if !r.is_empty() {
            self.dirty.push(r);
        }
        r
    }

    /// Replace one tile (surface/height/colour/prop) and mark its area dirty.
    /// Cached bounds are refreshed so height changes repaint correctly.
    pub fn set_tile(&mut self, tx: i32, ty: i32, tile: Tile) -> Rect {
        if !self.map.in_bounds(tx, ty) {
            return Rect::EMPTY;
        }
        let old = self.map.at(tx, ty);
        if old.is_water() != tile.is_water() {
            if tile.is_water() {
                self.water_count += 1;
                if !self.water_tiles.contains(&(tx, ty)) {
                    self.water_tiles.push((tx, ty));
                }
            } else {
                self.water_count = self.water_count.saturating_sub(1);
                self.water_tiles.retain(|&t| t != (tx, ty));
            }
            // B61e: the animated water's row index follows the tiles.
            self.rebuild_place_index();
        }
        // B87: a river's ripples read its neighbours' levels.
        self.water_prims.clear();
        self.map.set(tx, ty, tile);
        // B46d: the drawing map follows, but keeps its own (flattened) heights.
        let mut dtile = tile;
        let drawn = self.draw_map.at(tx, ty).corner;
        if drawn != old.corner {
            // This tile was reshaped by a landform's footprint: keep that shape.
            dtile.corner = drawn;
        }
        self.draw_map.set(tx, ty, dtile);
        let tile = dtile;
        // B70: the neighbours' smoothing follows what this tile became.
        let soft_changed = if old.corner != tile.corner
            || crate::softground::soft_kind(old.kind) != crate::softground::soft_kind(tile.kind)
            || old.is_water() != tile.is_water()
        {
            self.refresh_soft()
        } else {
            Vec::new()
        };
        self.field_dirty = true;
        if let Some(k) = self
            .items
            .iter()
            .position(|it| matches!(it, Item::Tile(x, y) if *x == tx && *y == ty))
        {
            let mut r = tile_bounds(&self.cam, tx, ty, tile.corner.map(|c| c as f32));
            let rise = if self.map.at(tx, ty).prop.is_tree() {
                26
            } else {
                0
            };
            r.y0 -= rise + 6;
            self.item_bounds[k] = r;
        }
        let r = self.tile_rect_at(tx, ty, old.max_level().max(tile.max_level()));
        if !r.is_empty() {
            self.dirty.push(r);
        }
        for (sx, sy) in soft_changed {
            if (sx, sy) != (tx, ty) {
                self.dirty_tile(sx, sy);
            }
        }
        r
    }

    /// Mark one building (and the ground under it) for re-rasterisation.
    pub fn dirty_building(&mut self, idx: usize) -> Rect {
        let (btx, bty, fw, fh, h) = {
            let b = &self.buildings[idx];
            (b.tx, b.ty, b.fw, b.fh, self.map.at(b.tx, b.ty).max_level())
        };
        let mut r = self.buildings[idx].bounds;
        for dy in 0..fh {
            for dx in 0..fw {
                r = r.union(&tile_bounds(&self.cam, btx + dx, bty + dy, [h as f32; 4]));
            }
        }
        let r = r.intersect(&self.land_clip());
        if !r.is_empty() {
            self.dirty.push(r);
        }
        r
    }

    pub fn has_dirty(&self) -> bool {
        self.dirty.iter().any(|r| !r.is_empty())
    }

    pub fn dirty_count(&self) -> usize {
        self.dirty.len()
    }

    /// Re-rasterise only the damaged rectangles (and their depth keys). All
    /// geometry is redrawn but clipped, so no ordering logic is special-cased.
    pub fn flush_dirty(&mut self) {
        // B76 r2: a sliced rebuild owns the layer (and the tree capture sink)
        // while it runs. Leave the dirty tiles queued; the pass covers most of
        // them, and whatever changed behind its cursor is flushed on the next
        // frame after it finishes.
        if self.rebuild_job.is_some() {
            return;
        }
        if self.dirty.is_empty() {
            self.stats.last_flush_ms = 0.0;
            self.stats.last_flush_rects = 0;
            self.stats.last_flush_area = 0;
            self.stats.last_flush_draws = 0;
            self.stats.last_flush_visited = 0;
            self.stats.last_flush_rects_raw = 0;
            return;
        }
        let t0 = Instant::now();
        let raw = std::mem::take(&mut self.dirty);
        let raw_count = raw.len();
        // 1 px of slack also merges rectangles that merely abut, which is
        // the common case for a burst of neighbouring tiles.
        let rects = merge_rects(raw, 1);
        let mut area = 0i64;
        let mut draws = 0usize;
        let mut visited = 0usize;
        let mut n = 0usize;
        let mut kit_total = 0usize;
        let mut engine_total = 0usize;
        // B61d: everything the tile painter needs that does not depend on the
        // rectangle is taken **once** for the whole flush. It used to be cloned
        // inside the loop — `street` twice, `fixtures`, the festival host and a
        // city scan — for every rectangle, which is what a four-rectangle flush
        // spent its 2 ms on.
        let flush_fixtures = self.fixtures.clone();
        let flush_street = self.street.clone();
        let flush_dress = self.spec_dress();
        let flush_fest_host = festival_host(&self.city);
        let flush_civic = self.civic.stat;
        let flush_land_clip = self.land_clip();
        let flush_seal_clip = self.seal_clip();
        // B69: trees in the repainted tiles are re-captured, not painted.
        if self.sway_on {
            crate::sway::install(std::mem::take(&mut self.trees));
        }
        for r in &rects {
            let r = r.intersect(&self.land_clip());
            if r.is_empty() {
                continue;
            }
            n += 1;
            area += r.area();
            // Diagonals whose item can overlap this rectangle. An item on
            // diagonal d has its base line at `oy + d * HALF_H`, reaches up to
            // `reach_px` above it and down to `HALF_H` below it:
            //   base - reach <= r.y1  and  base + HALF_H >= r.y0
            let d0 = ((r.y0 - self.cam.oy - 2 * HALF_H) / HALF_H) - 1;
            let d1 = ((r.y1 - self.cam.oy + self.reach_px) / HALF_H) + 1;

            let self_glow = self.glow;
        let self_hour = self.light.hour;
            let self_water = self.water;
            let self_weather = self.weather;
            let self_season = self.season;
            let self_cleared = self.cleared.clone();
            let self_biome = self.biome;
            let self_fixtures = &flush_fixtures;
            let self_street = &flush_street;
            let self_dress = flush_dress;
            let self_fest_host = flush_fest_host;
            let self_civic = flush_civic;
            let mut kit_draws = 0usize;
            let mut engine_draws = 0usize;
            let land_clip = flush_land_clip;
            let seal_clip = flush_seal_clip;
            let bx0 = band_of(r.x0);
            let bx1 = band_of(r.x1);
            let Scene {
                map,
                draw_map,
                landforms,
                buildings,
                cam,
                items,
                item_keys,
                item_bounds,
                index,
                city,
                city_diag,
                kit,
                holiday,
                market,
                static_layer,
                city_key: self_city_key,
                claims: self_claims,
                derived: self_derived,
                ..
            } = self;
            static_layer.set_clip(r.intersect(&land_clip));
            static_layer.clear_rect(r, TRANSPARENT);
            crate::sway::with_sink(|l| l.forget_parks_in(r));
            let (d, v) = draw_items(
                draw_map,
                landforms,
                buildings,
                cam,
                items,
                item_keys,
                item_bounds,
                index,
                city,
                city_diag,
                kit,
                &*holiday,
                *market,
                self_dress,
                self_fest_host,
                &mut kit_draws,
                &mut engine_draws,
                self_glow,
                self.light.hour,
                &self_water,
                &self_weather,
                &self_season,
                &self_cleared,
                &self_biome,
                &self_city_key,
                &self_claims,
                &self_derived,
                &self_fixtures,
                &self_street,
                &self.tech,
                &self_civic,
                static_layer,
                d0,
                d1,
                bx0,
                bx1,
                self.sky_h as i32,
            );
            draws += d;
            visited += v;
            kit_total += kit_draws;
            engine_total += engine_draws;
            let seal = seal_clip.intersect(&r);
            seal_seams(&mut self.static_layer, seal, 1);
        }
        if self.sway_on {
            self.trees = crate::sway::uninstall();
        }
        self.trees.release_scratch();
        self.trees.reindex();
        for r in &rects {
            self.trees.invalidate(r.expanded(2));
        }
        self.trees.settle(self.static_layer.depth().unwrap_or(&[]), self.static_layer.px());
        // The trees that came or went need their old and new boxes repainted.
        {
            let mut dmg = Vec::new();
            self.trees.take_damage(&mut dmg);
            for r in dmg {
                self.mark_rect(r);
            }
        }
        // B42g: the street lights' pools go on the ground itself, with the
        // ground's tags still readable (props and roofs must not take light),
        // and before the tint copy below so the light is not dimmed.
        self.draw_lamp_pools(Rect::from_size(self.width(), self.height()));
        self.static_layer.reset_clip();
        // Keep the tinted copy in step for the rectangles we just repainted
        // (B13/B34: tinting here, once per flush, is much cheaper than tinting
        // every composite's damage).
        let lut = crate::light::lut(self.tint);
        let bulbs: Vec<crate::palette::Color> =
            self.holiday.garland[..self.holiday.bulbs.max(1)].to_vec();
        let w = self.width();
        // B42: the same atmospheric haze `retint_static` bakes in, so a patched
        // rectangle keeps the horizon fade.
        let haze = self.haze_profile();
        let air = std::mem::take(&mut self.haze_row);
        for r in &rects {
            let r = r.intersect(&self.land_clip());
            if r.is_empty() {
                continue;
            }
            for y in r.y0..r.y1 {
                let amount = haze.get(y as usize).copied().unwrap_or(0.0);
                for x in r.x0..r.x1 {
                    let i = y as usize * w + x as usize;
                    let src = self.static_layer.px()[i];
                    let solid = matches!(
                        crate::palette::alpha(src),
                        crate::framebuffer::TAG_PROP | crate::framebuffer::TAG_BUILT
                    );
                    let mut c = night_pixel(src, &lut, &bulbs);
                    if amount > 0.0 && c != TRANSPARENT && !solid && air.len() == w {
                        c = palette::mix(c, air[x as usize], amount);
                    }
                    self.tinted_static.px_mut()[i] = c;
                }
            }
            self.draw_lamp_pools(r);
        }
        self.haze_row = air;
        // D71: the patched rows may have changed opacity.
        {
            let w = self.width();
            let mut done: Vec<i32> = Vec::new();
            for r in &rects {
                for y in r.y0.max(0)..r.y1.min(self.height() as i32) {
                    if done.contains(&y) {
                        continue;
                    }
                    done.push(y);
                    let row = &self.tinted_static.px()[y as usize * w..(y as usize + 1) * w];
                    let opaque = row.iter().all(|&p| p != TRANSPARENT);
                    if let Some(slot) = self.static_opaque.get_mut(y as usize) {
                        *slot = opaque;
                    } else if y >= 0 {
                        self.static_opaque.resize(self.height(), true);
                        self.static_opaque[y as usize] = opaque;
                    }
                }
            }
        }
        self.stats.last_flush_ms = t0.elapsed().as_secs_f64() * 1000.0;
        self.stats.last_flush_rects = n;
        self.stats.last_flush_rects_raw = raw_count;
        // The flush draws a subset: add what it drew to the last full rebuild's
        // counts, so the totals always describe the whole layer.
        self.stats.kit_draws += kit_total;
        self.stats.engine_draws += engine_total;
        self.stats.last_flush_area = area;
        self.stats.last_flush_draws = draws;
        self.stats.last_flush_visited = visited;
        // B72: the buildings this flush redrew have new faces recorded; their
        // old panes are dropped and their lights re-applied over the fresh dark
        // art.
        let faces = self.static_layer.take_faces();
        let kit = self.static_layer.take_kit_panes();
        if !faces.is_empty() || !kit.is_empty() {
            let mut keys: Vec<u32> = Vec::new();
            for (key, _) in &faces {
                keys.push(*key);
            }
            for (key, _) in &kit {
                keys.push(*key);
            }
            keys.sort_unstable();
            keys.dedup();
            for &key in &keys {
                self.windows.forget(key);
            }
            self.windows.add_faces(faces);
            for (key, p) in kit {
                self.windows.add_kit(key, p);
            }
            self.refresh_windows(Some(&keys));
        }
    }

    /// Advance life and weather by `dt` seconds.
    pub fn step(&mut self, dt: f32) {
        self.life.update(dt, &self.map);
        if let Some(r) = &mut self.rain {
            r.update(dt);
        }
    }

    pub fn rain_on(&self) -> bool {
        self.rain.is_some()
    }

    pub fn set_rain(&mut self, on: bool) {
        match (on, self.rain.is_some()) {
            (true, false) => {
                self.rain = Some(Rain::new(
                    self.width(),
                    self.height(),
                    self.seed,
                    crate::rain::DEFAULT_DROPS,
                ));
            }
            (false, true) => self.rain = None,
            _ => {}
        }
    }

    pub fn toggle_rain(&mut self) {
        self.set_rain(!self.rain_on());
    }

    /// Vehicles to draw this frame (B29). Off-screen ones are dropped, so a
    /// city whose traffic is outside the frame costs nothing.
    pub fn set_traffic(&mut self, bodies: Vec<crate::traffic::Body>) {
        // Filter on the *projected* position, not the tile flag: the map's
        // bounding box is larger than the frame's diamond, so a tile can be
        // "in the world" and still project off-screen (B29).
        let (w, h) = (self.width() as f32, self.height() as f32);
        let visible = |s: &Scene, b: &crate::traffic::Body| {
            let z = s.map.ground_px(b.x, b.y);
            let (sx, sy) = s.cam.project(b.x, b.y, z);
            sx > -16.0
                && sx < w + 16.0
                && sy > (s.sky_h as f32 - 2.0).max(-24.0)
                && sy < h + 16.0
        };
        // B30: the frame changed where a vehicle was and where it is now.
        let old: Vec<Rect> = self
            .traffic
            .iter()
            .filter(|b| visible(self, b))
            .map(|b| self.traffic_rect(b))
            .collect();
        for r in old {
            self.mark_rect(r);
        }
        self.traffic = bodies.into_iter().filter(|b| visible(self, b)).collect();
        // B61d: each vehicle's screen position, once, for the shadow pass (it
        // runs per damaged rectangle and used to re-project every one of them).
        let before_px = std::mem::take(&mut self.traffic_px);
        self.traffic_px.clear();
        self.traffic_px.reserve(self.traffic.len());
        for b in &self.traffic {
            let z = self.map.ground_px(b.x, b.y);
            let (sx, sy) = self.cam.project(b.x, b.y, z);
            self.traffic_px.push((sx as i32, sy as i32));
        }
        let h = self.height() as i32;
        self.traffic_bands
            .rebuild(h, self.traffic_px.iter().enumerate().map(|(i, p)| (p.1, i as u32)));
        // B61e: a vehicle that moved (or came or went) is a changed frame.
        self.moving_changed |= before_px.len() != self.traffic_px.len()
            || before_px.iter().zip(self.traffic_px.iter()).any(|(a, b)| a != b);
        self.traffic_px_buf = before_px;
        let now: Vec<Rect> = self.traffic.iter().map(|b| self.traffic_rect(b)).collect();
        for r in now {
            self.mark_rect(r);
        }
    }

    pub fn traffic_count(&self) -> usize {
        self.traffic.len()
    }

    /// B59: the cars standing in bays. They do not move, so their damage is
    /// marked only when the set changes — a full car park costs no more than an
    /// empty one.
    pub fn set_parked(&mut self, bodies: Vec<crate::traffic::Body>) {
        let (w, h) = (self.width() as f32, self.height() as f32);
        let keep: Vec<crate::traffic::Body> = bodies
            .into_iter()
            .filter(|b| {
                let (sx, sy) = self.cam.project(b.x, b.y, self.level_px(0) + b.lift as i32);
                sx > -16.0 && sx < w + 16.0 && sy > (self.sky_h as f32 - 2.0).max(-24.0) && sy < h + 16.0
            })
            .collect();
        let same = keep.len() == self.parked.len()
            && keep.iter().zip(self.parked.iter()).all(|(a, b)| {
                a.x == b.x && a.y == b.y && a.lift == b.lift && a.kind == b.kind && a.variant == b.variant
            });
        if same {
            return;
        }
        let old: Vec<Rect> = self.parked.iter().map(|b| self.screen_rect(b.x, b.y, 10)).collect();
        for r in old {
            self.mark_rect(r);
        }
        self.parked = keep;
        // B101: the bays never move, so their screen rows are indexed once, when
        // the set changes (they were scanned in full for every damaged rectangle).
        let h = self.height() as i32;
        // Keyed 32 px above the car's feet: the renderer's band search reaches 48 px
        // either way, and a tall vehicle (a van, a bus) reaches further up than that.
        let draw_rows: Vec<(i32, u32)> = self
            .parked
            .iter()
            .enumerate()
            .map(|(i, b)| (self.cam.project(b.x, b.y, self.map.ground_px(b.x, b.y) + b.lift as i32).1 as i32 - 32, i as u32))
            .collect();
        self.parked_bands.rebuild(h, draw_rows);
        let ground_rows: Vec<(i32, u32)> = self
            .parked
            .iter()
            .enumerate()
            .map(|(i, b)| (self.cam.project(b.x, b.y, 0).1 as i32, i as u32))
            .collect();
        self.parked_shadow_bands.rebuild(h, ground_rows);
        let now: Vec<Rect> = self.parked.iter().map(|b| self.screen_rect(b.x, b.y, 10)).collect();
        for r in now {
            self.mark_rect(r);
        }
    }

    pub fn parked_count(&self) -> usize {
        self.parked.len()
    }

    /// Movers on screen after culling (D71: the idle rule's half).
    pub fn movers_shown_count(&self) -> usize {
        self.movers.len()
    }

    /// How many rectangles are waiting for the next composite, and how many
    /// the last animation step added (D71: "did anything move" without a scan).
    pub fn pending_len(&self) -> usize {
        self.pending.len() + self.tree_pending.len() + self.water_pending.len()
    }

    /// B31: rail track, the station platform, the yard, the marina and the port.
    /// These never move, so they are baked into the static layer (a dirty-rect
    /// composite would otherwise paint over them).
    pub fn set_fixtures(&mut self, f: crate::rail::Fixtures) {
        let changed = f.track != self.fixtures.track
            || f.crossings != self.fixtures.crossings
            || f.station != self.fixtures.station
            || f.station_platform != self.fixtures.station_platform
            || f.tram_line.len() != self.fixtures.tram_line.len()
            || f.tram_stops.len() != self.fixtures.tram_stops.len()
            || f.yard != self.fixtures.yard
            || f.marina != self.fixtures.marina
            || f.port != self.fixtures.port
            || f.containers != self.fixtures.containers
            || f.transit != self.fixtures.transit;
        self.fixtures = f;
        self.rebuild_planting();
        if changed {
            self.rebuild_static();
            self.retint_static();
            self.mark_full();
        }
    }

    /// B31: trains, trams, boats and planes to draw this frame.
    pub fn set_movers(&mut self, movers: Vec<crate::rail::Body>) {
        let (w, h) = (self.width() as f32, self.height() as f32);
        let keep = |s: &Scene, b: &crate::rail::Body| {
            let z = if matches!(b.kind, crate::rail::Kind::Plane | crate::rail::Kind::Aircraft) {
                b.lift
            } else {
                s.map.ground_px(b.x, b.y)
            };
            let (sx, sy) = s.cam.project(b.x, b.y, z);
            sx > -40.0 && sx < w + 40.0 && sy > -60.0 && sy < h + 40.0
        };
        // B79: a towel, an umbrella or a sunbather that has not changed since the
        // last frame costs no repaint at all.
        let shore_key = |b: &crate::rail::Body| (b.x.to_bits(), b.y.to_bits(), b.set, b.lift, b.phase.to_bits());
        let same: std::collections::HashSet<(u32, u32, u8, i32, u32)> = {
            let new_keys: std::collections::HashSet<_> = movers
                .iter()
                .filter(|b| b.kind == crate::rail::Kind::Shore)
                .map(|b| shore_key(b))
                .collect();
            self.movers
                .iter()
                .filter(|b| b.kind == crate::rail::Kind::Shore)
                .map(|b| shore_key(b))
                .filter(|k| new_keys.contains(k))
                .collect()
        };
        let old: Vec<Rect> = self
            .movers
            .iter()
            .filter(|b| keep(self, b))
            .filter(|b| !(b.kind == crate::rail::Kind::Shore && same.contains(&shore_key(b))))
            .map(|b| self.mover_rect(b))
            .collect();
        for r in old {
            self.mark_rect(r);
        }
        self.movers = movers.into_iter().filter(|b| keep(self, b)).collect();
        let has_boats = self.movers.iter().any(|b| {
            matches!(b.kind, crate::rail::Kind::Boat | crate::rail::Kind::Ship | crate::rail::Kind::Ferry)
        });
        if !has_boats {
            self.mover_haze.clear();
        } else if self.mover_haze.is_empty() {
            self.mover_haze = self.boat_haze_rows();
        }
        // B61d: the shadow pass's screen positions, once per frame.
        let before_movers = std::mem::take(&mut self.movers_px);
        self.movers_px.clear();
        self.movers_px.reserve(self.movers.len());
        for b in &self.movers {
            let z = if matches!(b.kind, crate::rail::Kind::Plane | crate::rail::Kind::Aircraft) {
                b.lift
            } else {
                self.map.ground_px(b.x, b.y)
            };
            let (sx, sy) = self.cam.project(b.x, b.y, z);
            self.movers_px.push((sx as i32, sy as i32));
        }
        let h = self.height() as i32;
        self.mover_bands
            .rebuild(h, self.movers_px.iter().enumerate().map(|(i, p)| (p.1, i as u32)));
        self.moving_changed |= before_movers.len() != self.movers_px.len()
            || before_movers.iter().zip(self.movers_px.iter()).any(|(a, b)| a != b);
        self.movers_px_buf = before_movers;
        let now: Vec<Rect> = self
            .movers
            .iter()
            .filter(|b| !(b.kind == crate::rail::Kind::Shore && same.contains(&shore_key(b))))
            .map(|b| self.mover_rect(b))
            .collect();
        for r in now {
            self.mark_rect(r);
        }
    }

    /// B93: a moving thing's damage box grown by the shadow it throws.
    ///
    /// `draw_moving_shadows` lays a shadow of up to `SHADOW_REACH` px along the
    /// light, outside the sprite's own box; boxes without it left the shadow's
    /// tip cut off or stale when the thing moved on (worst at dawn and dusk).
    /// The reach is the same 56 px the shadow pass culls with.
    fn with_shadow(&self, r: Rect, x: f32, y: f32, height: f32) -> Rect {
        let Some(sh) = self.light.actor() else {
            return r;
        };
        let z = self.map.ground_px(x, y);
        let a = self.cam.project(x, y, z);
        let b = self.cam.project(x + sh.dx * height, y + sh.dy * height, z);
        let reach = SHADOW_REACH as f32;
        let ox = (b.0 - a.0).clamp(-reach, reach) as i32;
        let oy = (b.1 - a.1).clamp(-reach, reach) as i32;
        r.union(&Rect::new(r.x0 + ox, r.y0 + oy, r.x1 + ox, r.y1 + oy)).expanded(1)
    }

    /// The screen box one vehicle can touch: its sprite, its lamps and signals
    /// (B93: they reach 20 px behind and 8 px past the old 20 x 30 box, and an
    /// emergency flash 15 px above it) and its shadow.
    pub fn traffic_rect(&self, b: &crate::traffic::Body) -> Rect {
        let mut r = self.screen_rect(b.x, b.y, 10);
        r.x0 -= 21;
        r.x1 += 9;
        r.y1 += 6;
        if b.kind == crate::traffic::Kind::Emergency {
            r.y0 -= 16;
        }
        self.with_shadow(r, b.x, b.y, b.kind.height_px() as f32)
    }

    /// The screen rect one mover can touch: its head, every unit following it
    /// and the wake. A freight train is eight cars long, so this is not a fixed
    /// box any more (B58: the rects were sized for four).
    pub fn mover_rect(&self, b: &crate::rail::Body) -> Rect {
        let z = if matches!(b.kind, crate::rail::Kind::Plane | crate::rail::Kind::Aircraft) {
            b.lift
        } else {
            self.map.ground_px(b.x, b.y)
        };
        let (sx, sy) = self.cam.project(b.x, b.y, z);
        // B93: half-width, height above and depth below the anchor, measured
        // against what each kind paints (`tests/damage_equiv.rs`): a train, tram
        // or bus stands 30 px tall (the old 14 left its roof and wires behind), a
        // plane on the runway reaches 38 px below its anchor, a crossing's arms
        // 39 px sideways and 47 px down.
        let (px, up, down) = match b.kind {
            crate::rail::Kind::Plane => (46, 10, 10),
            crate::rail::Kind::Aircraft => (22, 14, 38),
            crate::rail::Kind::Crossing => (39, 30, 47),
            crate::rail::Kind::Crane => (24, 30, 33),
            crate::rail::Kind::Crew => (6, 8, 8),
            crate::rail::Kind::Shore => crate::shore::extent(b.set, b.lift),
            crate::rail::Kind::Train | crate::rail::Kind::Tram | crate::rail::Kind::Bus => (24, 30, 16),
            _ => (24, 14, 14),
        };
        let mut r = Rect::new(
            sx as i32 - px,
            sy as i32 - up,
            sx as i32 + px,
            sy as i32 + down,
        );
        // B91: a boat's own box, wake and mast included, plus a pixel to bob in.
        if matches!(b.kind, crate::rail::Kind::Boat | crate::rail::Kind::Ship | crate::rail::Kind::Ferry) {
            let class = match b.kind {
                crate::rail::Kind::Boat => crate::boats::Class::Boat,
                crate::rail::Kind::Ship => crate::boats::Class::Ship,
                _ => crate::boats::Class::Ferry,
            };
            let (bk, _) = crate::boats::BoatKind::decode(class, b.set);
            let (hw, up, down) = bk.extent();
            r = Rect::new(sx as i32 - hw, sy as i32 - up - 1, sx as i32 + hw, sy as i32 + down + 1);
        }
        // B93: the shadow it throws (not a boat's, not an airborne thing's, not
        // a barrier's: the shadow pass skips those).
        if b.lift <= 8
            && !matches!(
                b.kind,
                crate::rail::Kind::Boat
                    | crate::rail::Kind::Ship
                    | crate::rail::Kind::Ferry
                    | crate::rail::Kind::Crossing
                    | crate::rail::Kind::Crane
                    | crate::rail::Kind::Crew
            )
        {
            r = self.with_shadow(r, b.x, b.y, 16.0);
        }
        // The trailing units hang back along the route; a train's last car can
        // be 300 px away.
        if b.cars > 0 {
            for t in b.trail.iter().take(b.cars as usize) {
                let z2 = self.map.ground_px(t.0, t.1);
                let (cx, cy) = self.cam.project(t.0, t.1, z2);
                let (hw, up, down) = if matches!(b.kind, crate::rail::Kind::Boat | crate::rail::Kind::Ship | crate::rail::Kind::Ferry) {
                    crate::boats::BoatKind::Barge.extent()
                } else {
                    (24, 30, 16)
                };
                r = r.union(&Rect::new(
                    cx as i32 - hw,
                    cy as i32 - up - 1,
                    cx as i32 + hw,
                    cy as i32 + down + 1,
                ));
            }
        }
        r
    }

    pub fn mover_count(&self) -> usize {
        self.movers.len()
    }

    /// Actors only: the part of the frame the depth plane affects.
    pub fn draw_actors(&self, dst: &mut Framebuffer) {
        self.life
            .draw(dst, &self.cam, &self.map, self.depth(), self.depth_mode);
    }

    /// B60/D74 item 3: the small projected shadow every moving thing throws,
    /// from the same [`LightState`] as the static shadows. Drawn once per frame
    /// over the baked ground and *under* the thing itself, depth-tested against
    /// the static layer so a shadow behind a wall stays behind it.
    ///
    /// Extents are the actor's ground footprint in tiles and its height in
    /// pixels — the numbers the sprites are drawn from, so a bus throws a
    /// longer shadow than a person.
    pub fn draw_moving_shadows(&self, dst: &mut Framebuffer) {
        let Some(sh) = self.light.actor() else {
            return;
        };
        let depth = self.depth();
        let cam = &self.cam;
        let map = &self.map;
        // B61c: a damage-limited composite draws this once per *damaged
        // rectangle*, and a rush hour has nine hundred residents — drawing a
        // shadow for every one of them for every rectangle was 62 % of a
        // rush-hour frame (`perf`). The clip is what put_blended would enforce
        // anyway, so an actor whose shadow cannot land in it is skipped before
        // any projection.
        let clip = dst.clip();
        const REACH: i32 = SHADOW_REACH; // px: how far a figure's shadow can stretch
        let in_clip = |px: i32, py: i32| {
            px + REACH >= clip.x0
                && px - REACH < clip.x1
                && py + REACH >= clip.y0
                && py - REACH < clip.y1
        };
        // B101: the cheap exact cull. The swept footprint of a shadow lies inside
        // the box from the anchor, grown by the footprint and by the shadow's own
        // offset, so an actor whose box misses the clip never reaches the 8
        // projections `draw_actor_shadow_in` does to find that out. (The REACH
        // test above stays: it decides which shadows exist at all.)
        let reaches = |x: f32, y: f32, z: i32, hw: f32, hh: f32, height: f32| -> bool {
            let (sx, sy) = cam.project(x, y, z);
            let (sx, sy) = (sx as i32, sy as i32);
            let foot = hw.max(hh).max(0.01) * 2.0;
            let fx = (foot * map::iso_consts::HALF_W as f32).ceil() as i32 + 3;
            let fy = (foot * map::iso_consts::HALF_H as f32).ceil() as i32 + 3;
            let ox = ((sh.dx - sh.dy) * map::iso_consts::HALF_W as f32 * height) as i32;
            let oy = ((sh.dx + sh.dy) * map::iso_consts::HALF_H as f32 * height) as i32;
            sx + fx + ox.max(0) + 2 >= clip.x0
                && sx - fx + ox.min(0) - 2 < clip.x1
                && sy + fy + oy.max(0) + 2 >= clip.y0
                && sy - fy + oy.min(0) - 2 < clip.y1
        };
        // B59b: cars standing in bays cast the sun's shadow too (the deck's
        // roof cars stand at their deck's height).
        let mut parked_shadow = |i: usize| {
            let b = &self.parked[i];
            let (px, py) = self.cam.project(b.x, b.y, 0);
            if !in_clip(px as i32, py as i32) {
                return;
            }
            let len = b.kind.length_px() as f32 / map::iso_consts::TILE_W as f32;
            let wid = b.kind.width_px() as f32 / map::iso_consts::TILE_W as f32;
            let z = map.ground_px(b.x, b.y) + b.lift as i32;
            let (hw, hh) = if b.dir.0 != 0.0 { (len * 0.5, wid * 0.5) } else { (wid * 0.5, len * 0.5) };
            if !reaches(b.x, b.y, z, hw, hh, b.kind.height_px() as f32) {
                return;
            }
            let key = crate::actors::actor_key(b.x.floor() as i32, b.y.floor() as i32);
            let _ = crate::light::draw_actor_shadow_in(dst, cam, depth, &self.shadow, key, b.x, b.y, z, hw, hh, b.kind.height_px() as f32, &sh, true);
        };
        if self.parked_shadow_bands.is_empty() {
            for i in 0..self.parked.len() {
                parked_shadow(i);
            }
        } else {
            self.parked_shadow_bands.for_each(clip, REACH, |i| parked_shadow(i as usize));
        }
        let mut one = |x: f32, y: f32, hw: f32, hh: f32, height: f32, key: u16| {
            let z = map.ground_px(x, y);
            if !reaches(x, y, z, hw, hh, height) {
                return;
            }
            let _ = crate::light::draw_actor_shadow_in(dst, cam, depth, &self.shadow, key, x, y, z, hw, hh, height, &sh, true);
        };
        for c in &self.life.cars {
            let (x, y) = c.position();
            let (px, py) = self.screen_of(x, y);
            if !in_clip(px, py) {
                continue;
            }
            one(x, y, 0.15, 0.15, 12.0, crate::actors::actor_key(c.tx, c.ty));
        }
        for p in &self.life.peds {
            let (px, py) = self.screen_of(p.x, p.y);
            if !in_clip(px, py) {
                continue;
            }
            one(p.x, p.y, 0.04, 0.04, 8.0, crate::actors::actor_key(p.x.floor() as i32, p.y.floor() as i32));
        }
        // B61e: nine hundred residents, visited by band instead of by the
        // whole list once per damaged rectangle.
        let mut person = |i: usize| {
            let f = self.people[i];
            if let Some(&(px, py)) = self.people_px.get(i) {
                if !in_clip(px, py) {
                    return;
                }
            }
            let h = if f.child { 7.0 } else { 11.0 };
            one(f.x, f.y, 0.05, 0.05, h, crate::actors::actor_key(f.x.floor() as i32, f.y.floor() as i32));
        };
        if self.people_bands.is_empty() {
            for i in 0..self.people.len() {
                person(i);
            }
        } else {
            self.people_bands.for_each(clip, REACH, |i| person(i as usize));
        }
        // B61d: every kind of actor is culled against the clip from a screen
        // position computed once a frame (B61c did this for the residents only).
        // Without it each vehicle, tram and settler was projected *and* its
        // shadow diamonds walked for every damaged rectangle — 3 ms of a
        // rush-hour frame at 7040x1440.
        let mut vehicle = |i: usize| {
            let b = &self.traffic[i];
            if let Some(&(px, py)) = self.traffic_px.get(i) {
                if !in_clip(px, py) {
                    return;
                }
            }
            let len = b.kind.length_px() as f32 / map::iso_consts::TILE_W as f32;
            let wid = b.kind.width_px() as f32 / map::iso_consts::TILE_W as f32;
            let (hw, hh) = if b.dir.0 != 0.0 { (len * 0.5, wid * 0.5) } else { (wid * 0.5, len * 0.5) };
            one(
                b.x,
                b.y,
                hw,
                hh,
                b.kind.height_px() as f32,
                crate::actors::actor_key(b.x.floor() as i32, b.y.floor() as i32),
            );
        };
        if self.traffic_bands.is_empty() {
            for i in 0..self.traffic.len() {
                vehicle(i);
            }
        } else {
            self.traffic_bands.for_each(clip, REACH, |i| vehicle(i as usize));
        }
        let mut mover = |i: usize| {
            let m = &self.movers[i];
            if m.lift > 8 {
                return; // in the air: nothing to fall on
            }
            if matches!(
                m.kind,
                crate::rail::Kind::Crossing | crate::rail::Kind::Crane | crate::rail::Kind::Crew | crate::rail::Kind::Shore
            ) {
                return; // a barrier, a trolley and a worker's own shadow is not ours to cast
            }
            if matches!(
                m.kind,
                crate::rail::Kind::Boat | crate::rail::Kind::Ship | crate::rail::Kind::Ferry
            ) {
                return; // B91: a boat sits in the water with its own dark contact ring
            }
            if let Some(&(px, py)) = self.movers_px.get(i) {
                if !in_clip(px, py) {
                    return;
                }
            }
            one(m.x, m.y, 0.5, 0.16, 16.0, crate::actors::actor_key(m.x.floor() as i32, m.y.floor() as i32));
        };
        if self.mover_bands.is_empty() {
            for i in 0..self.movers.len() {
                mover(i);
            }
        } else {
            self.mover_bands.for_each(clip, REACH, |i| mover(i as usize));
        }
        for &(x, y, _f) in &self.walkers {
            let (px, py) = self.screen_of(x, y);
            if !in_clip(px, py) {
                continue;
            }
            one(x, y, 0.04, 0.04, 8.0, crate::actors::actor_key(x as i32, y as i32));
        }
        for &(x, y, _kind, _phase) in &self.settlers {
            let (px, py) = self.screen_of(x, y);
            if !in_clip(px, py) {
                continue;
            }
            one(x, y, 0.05, 0.05, 10.0, crate::actors::actor_key(x as i32, y as i32));
        }
        // B43c: cranes and site plant throw their own, from the same light.
        for rig in &self.rigs {
            if rig.bounds.expanded(120).intersects(&clip) {
                rig.shadow(dst, depth, cam, &sh, self.rig_t());
            }
        }
    }

    /// Compose one frame: sky, cached static layer, depth-tested actors, weather.
    pub fn composite(&self, dst: &mut Framebuffer) {
        self.composite_rect(dst, Rect::from_size(dst.w, dst.h));
    }

    /// Hide the map's bare ground above the land line, and nothing else
    /// (B42b).
    ///
    /// The world is painted a little way into the sky band so a tree or a spire
    /// on the back row is not cut off. Bare ground up there is replaced by the
    /// band's own pixel, so the ground simply ends where the painted distance
    /// begins — no melt, no slab, no seam. Anything standing up (a wall, a
    /// canopy, a lamp post) is kept whole, drawn over the band.
    fn mask_above_land(&self, dst: &mut Framebuffer, r: Rect) {
        if self.sky_h == 0 || self.sky.h == 0 {
            return;
        }
        let top = self.scan_top();
        let y0 = r.y0.max(top);
        let y1 = r.y1.min(self.sky_h as i32);
        if y1 <= y0 {
            return;
        }
        let r = r.intersect(&dst.bounds());
        let x0 = r.x0.max(0) as usize;
        let x1 = r.x1.max(0) as usize;
        if x1 <= x0 {
            return;
        }
        let w = self.width();
        for y in y0..y1 {
            let row = y as usize * w;
            if self.static_layer.px()[row + x0..row + x1].iter().all(|&p| p == TRANSPARENT) {
                continue;
            }
            for x in x0..x1 {
                let i = row + x;
                let src = self.static_layer.px()[i];
                if src == TRANSPARENT {
                    continue;
                }
                // B90: anything that stands up (a prop, a building) may rise
                // past the horizon and keeps every pixel; bare ground and lanes
                // belong to the world behind it and go.
                let a = palette::alpha(src);
                if a != TAG_PROP && a != TAG_BUILT {
                    dst.px_mut()[i] = self.sky.px()[i];
                }
            }
        }
    }

    /// Compose only `r` (B30): the same layers, clipped. The caller guarantees
    /// that nothing outside `r` changed since the last composite.
    pub fn composite_rect(&self, dst: &mut Framebuffer, r: Rect) {
        self.base_passes(dst, r);
        self.compose_over(dst, r);
    }

    /// The passes that lay down the world under the water's ripples: the sky,
    /// the static layer, the cast shadows, the horizon mask and the trees.
    fn base_passes(&self, dst: &mut Framebuffer, r: Rect) {
        let mut lap = pass_time::Lap::new();
        dst.set_clip(r);
        dst.copy_sky(&self.sky);
        lap.mark(0);
        // B42c/D71: the far-ground haze is laid down with the static layer in
        // one pass. Where the map's diamond has not reached (its back corners,
        // on some seeds) the zig-zag edge of the world would otherwise show
        // bare frame.
        dst.blit_fb_hazed(
            &self.tinted_static,
            &self.haze_row,
            self.sky_h as i32,
            &self.static_opaque,
            self.scan_top(),
        );
        lap.mark(1);
        // B60: the cast-shadow plane, applied over what the static layer laid down.
        dst.shade_rows(&self.shadow, &self.shadow_lut);
        lap.mark(2);
        self.mask_above_land(dst, r);
        lap.mark(3);
        // B69: the trees, swayed by the wind, over the ground and under
        // everything that stands in front of them.
        self.trees.draw(
            dst,
            r.intersect(&self.land_clip()),
            self.static_layer.depth().unwrap_or(&[]),
            self.static_layer.px(),
            self.tint,
            &self.holiday.garland[..self.holiday.bulbs.max(1)],
            night_pixel,
        );
        lap.mark(4);
        dst.set_clip(r);
        // B50: the ending's sky (the swollen sun, the falling moon, the comet)
        // and its ground (lava, ice, the machine traces, the flood's foam).
        if self.ending.is_some() {
            if let Some(ctx) = self.ending_ctx() {
                dst.set_clip(r);
                if r.intersects(&self.ending_gbb) {
                    crate::ending::draw_ground(dst, &ctx);
                }
            }
        }
        lap.mark(5);
        // B62: the rainbow hangs in the sky band, behind the clouds.
        if self.spec.rainbow_on() {
            dst.set_clip(r.intersect(&Rect::new(0, 0, self.width() as i32, self.sky_h as i32)));
            self.draw_rainbow(dst);
            dst.set_clip(r);
        }
    }

    /// B87: everything painted over the base plane: the water's ripples, the
    /// nightlife, the cloud shade, the traffic, the people and the weather.
    fn compose_over(&self, dst: &mut Framebuffer, r: Rect) {
        let mut lap = pass_time::Lap::new();
        dst.set_clip(r);
        if self.water.anim {
            dst.set_clip(r.intersect(&self.land_clip()));
            // B42f: the animated ripple highlights are *not* hazed by
            // `retint_static` (they are painted per frame, after it), so a lake
            // at the back of the map threw bright cyan streaks across the
            // horizon. In the rows where the haze is nearly opaque the water is
            // left to the hazed static layer alone: its highlights would be
            // invisible under the haze anyway.
            let strong = (self.haze_depth() * 0.142) as i32;
            let haze_bottom = self.sky_h as i32 + strong;
            // D71: only the tiles inside the damage rectangle can change a
            // pixel; a partial composite has no business walking the map.
            let clip = dst.clip();
            // B61e: the water's screen positions and row index are static (built
            // with the static layer), so a rectangle visits its own rows only.
            // B87: when the strokes of the last water tick are known, paint
            // those; they are what `water_overlay` yields, without redoing its
            // lookups for every damaged box.
            let cached = self.water_prims.len() == self.water_px.len() && self.water_haze == strong;
            let mut draw = |i: usize| {
                let (x, y, xs, ys) = self.water_px[i];
                if cached {
                    for st in self.water_prims[i].iter() {
                        if st.bounds().intersect(&clip).is_empty() {
                            continue;
                        }
                        st.paint(dst);
                    }
                    return;
                }
                if strong > 0 && ys < haze_bottom {
                    return;
                }
                if ys + 48 < clip.y0
                    || ys - 48 >= clip.y1
                    || xs + 48 < clip.x0
                    || xs - 48 >= clip.x1
                {
                    return;
                }
                terrain::water_overlay_c(dst, &self.cam, &self.map, &self.water, x, y, self.water_coast.get(i).copied(), self.water_ground.get(i).copied());
            };
            if self.water_bands.is_empty() {
                for i in 0..self.water_px.len() {
                    draw(i);
                }
            } else {
                self.water_bands
                    .for_each_x(clip, 48, 48, |i| self.water_px[i as usize].2, |i| draw(i as usize));
            }
            dst.set_clip(r);
        }
        lap.mark(6);
        // B73/D80: the nightlife — signs in their opening hours, crown lights,
        // aviation beacons. Drawn over the world and under the movers, from
        // descriptors (never a baked sprite per state).
        if !self.night.is_empty() {
            let look = crate::nightlife::Look {
                night: self.glow,
                hour: self.light.hour,
                t: self.sign_t(),
                wet: self.weather.wet,
                land: crate::nightlife::Land::of(self.map.biome),
            };
            crate::nightlife::draw(&self.night, dst, r, &look, self.static_layer.depth());
            dst.set_clip(r);
        }
        lap.mark(7);
        // B57/D72: the clouds' shadows. They move every frame, so they are a
        // multiply applied *after* the world is down — over ground, roads, roofs
        // and walls alike — rather than a second baked plane.
        // B84 r3: the sky takes no cloud shadow (the pass skips every pixel the
        // static layer has nothing on), so it is clipped to the land — over a
        // sky-band rectangle it was 20 ms of scanning 3 Mpx of sky, and a cloudy
        // frame names the whole band.
        if self.cloud_k > 0.002 && !self.clouds.is_empty() {
            self.clouds.draw_shadows(
                dst,
                r.intersect(&self.land_clip()),
                &self.cam,
                &self.map,
                &self.light,
                &self.cloud_lut,
                &self.static_layer,
            );
        }
        lap.mark(8);
        // B41: traffic lights cycle, and the crews move along their street.
        if !self.street.lights.is_empty() || !self.street_crews.is_empty() {
            let phase = (self.started_at_secs() * 0.02).fract();
            let horizon_y = self.sky_h as f32;
            let clip = dst.clip();
            for &(x, y) in self.street.lights.iter() {
                let (sx, sy) = self.cam.project(x as f32, y as f32, 0);
                if sy < horizon_y
                    || (sy as i32) + 40 < clip.y0
                    || (sy as i32) - 40 >= clip.y1
                    || (sx as i32) + 40 < clip.x0
                    || (sx as i32) - 40 >= clip.x1
                {
                    continue;
                }
                crate::street::draw_traffic_light(dst, &self.cam, &self.map, x, y, phase, self.glow);
            }
            for &(x, y, kind) in &self.street_crews {
                if self.cam.project(x as f32, y as f32, 0).1 < horizon_y {
                    continue;
                }
                crate::street::draw_crew(dst, &self.cam, &self.map, x, y, kind);
            }
        }
        // B49b: the wind farm's rotors, turning at the frame rate (the scene
        // damages only each rotor's own rectangle — `turbine_tick`).
        if !self.turbines.is_empty() {
            let t = self.started_at_secs();
            let depth = self.depth();
            let _ = depth;
            for &(x, y) in &self.turbines {
                if self.cam.project(x as f32, y as f32, 0).1 < self.sky_h as f32 {
                    continue;
                }
                crate::tech::draw_turbine_rotor(dst, &self.cam, &self.map, x, y, t);
            }
        }
        // B49b: the technology crews stand on the road and the pavement, and are
        // hidden by whatever is in front of them (the van never lands on a roof).
        if !self.tech_crews.is_empty() {
            let depth = self.depth();
            let horizon_y = self.sky_h as f32;
            for &(x, y, kind) in &self.tech_crews {
                if self.cam.project(x as f32, y as f32, 0).1 < horizon_y {
                    continue;
                }
                let key = crate::actors::actor_key(x, y);
                crate::tech::draw_crew(dst, &self.cam, &self.map, x, y, kind, depth, key);
            }
        }
        // B49: the delivery drones, per frame, with their shadows.
        if !self.drones.is_empty() {
            let t = self.started_at_secs();
            for &(dx, dy) in &self.drones {
                crate::tech::draw_drone(dst, &self.cam, &self.map, dx, dy, 0.0, t + dx);
            }
        }
        // B39: the settlement — logs on the ground, the camp, the party, and
        // the trees coming down (B40).
        for &(x, y, kind) in &self.logs {
            crate::holiday::log_pile(dst, &self.cam, &self.map, x, y, kind);
        }
        for &(x, y, frac, dir, size, variant) in &self.falling {
            crate::holiday::falling_tree(dst, &self.cam, &self.map, x, y, frac, dir, size, variant);
        }
        if let Some((fx, fy, tents, fire)) = &self.camp {
            let t = self.started_at_secs();
            // B41b: no fire once its ground belongs to the village.
            let fire = fire.unwrap_or((i32::MIN / 2, i32::MIN / 2));
            if !tents.is_empty() || fire.0 != i32::MIN / 2 {
                crate::holiday::camp(dst, &self.cam, &self.map, *fx, *fy, tents, fire, self.glow, t);
            }
        }
        for &(x, y, kind, phase) in &self.settlers {
            crate::holiday::settler(dst, &self.cam, &self.map, x, y, kind, phase);
        }
        // B62: parade floats and the gameday lot, under the people.
        lap.mark(9);
        self.draw_spec_under(dst, r);
        lap.mark(15);
        // B60: every mover's own shadow, before the movers are drawn on top.
        self.draw_moving_shadows(dst);
        lap.mark(16);
        self.draw_actors(dst);
        lap.mark(17);
        if !self.traffic.is_empty() {
            self.traffic_sprites.draw(
                dst,
                &self.cam,
                &self.map,
                &self.traffic,
                &self.traffic_bands,
                self.depth(),
                self.glow,
                self.depth_mode,
                &self.city_key,
                self.weather.snow_cover,
            );
        }
        lap.mark(18);
        if !self.parked.is_empty() {
            self.traffic_sprites.draw(
                dst,
                &self.cam,
                &self.map,
                &self.parked,
                &self.parked_bands,
                self.depth(),
                self.glow,
                self.depth_mode,
                &self.city_key,
                self.weather.snow_cover,
            );
        }
        lap.mark(19);
        for &(x, y, f) in &self.walkers {
            let level = self.map.surface_level_at(x, y);
            let (fx, fy) = self.cam.project(x, y, self.level_px(level as u8));
            let (sx, sy) = (fx as i32, fy as i32);
            let c = pal::WALKER;
            dst.put(sx, sy - 2, c);
            dst.put(sx, sy - 1, c);
            if f == 0 {
                dst.put(sx, sy, c);
            } else {
                dst.put(sx - 1, sy, c);
            }
        }
        // B43: people belong on the land. Clipped to the land band, so a figure
        // on the far edge can never stand in the sky (D64/B42).
        dst.set_clip(r.intersect(&self.land_clip()));
        // B43: the timber and bricks at every site, under the crews.
        if !self.sites.is_empty() {
            self.people_sprites.draw_sites(
                dst,
                &self.cam,
                &self.map,
                &self.sites,
                self.depth(),
                self.depth_mode,
            );
        }
        // B43c: cranes, the digger, trucks and the site cabin, depth-tested so
        // a building in front of a rig hides it.
        if !self.rigs.is_empty() {
            let clip = dst.clip();
            let (clock, work) = (self.rig_t(), self.rig_work);
            let depth = self.static_layer.depth().unwrap_or(&[]);
            for rig in &self.rigs {
                if !rig.bounds.intersects(&clip) {
                    continue;
                }
                let mut pen = crate::rigs::Pen {
                    fb: dst,
                    depth,
                    key: rig.key,
                    tint: self.tint,
                    glow: self.glow,
                };
                rig.draw(&mut pen, &self.cam, clock, work);
            }
        }
        lap.mark(20);
        // B43/D65: the residents — builders, farmers, commuters, the park.
        if !self.people.is_empty() {
            self.people_sprites.draw(
                dst,
                &self.cam,
                &self.map,
                &self.people,
                &self.people_px,
                &self.people_bands,
                self.depth(),
                self.depth_mode,
                self.holiday_t,
                &self.city_key,
                self.spec.extra_from,
            );
        }
        lap.mark(21);
        dst.set_clip(r);
        // B31: trains, trams, boats and planes, above the traffic. B58d: the
        // ground ones are clipped to the land band (the city clips everything it
        // draws there), so nothing rolls up over the sky; the sky overflight is
        // drawn with the whole frame, because the band is where it belongs.
        if !self.movers.is_empty() {
            let land = self.land_clip();
            dst.set_clip(r.intersect(&land));
            // B91: boats fade into the distance haze the way the water does.
            let haze = if self.mover_haze.is_empty() { None } else { Some(self.mover_haze.as_slice()) };
            lap.mark(22);
            self.mover_sprites.draw(
                dst,
                &self.cam,
                &self.map,
                &self.movers,
                &self.mover_bands,
                self.depth(),
                false,
                haze,
                self.tinted_static.px(),
            );
            dst.set_clip(r);
            self.mover_sprites.draw(
                dst,
                &self.cam,
                &self.map,
                &self.movers,
                &self.mover_bands,
                self.depth(),
                true,
                None,
                &[],
            );
        }
        lap.mark(23);
        // (B62: the parade is part of the spectacle layer now: floats under the
        // people, the band and the kerb crowd as people.)
        // B62: spectacle over the world (Bills win fireworks are in there).
        self.draw_spec_over(dst, r);
        // B52: fires, smoke, posters, cats and the rest of the civic layer.
        self.draw_civic_over(dst, r);
        if self.holiday.holiday == crate::holiday::Holiday::BillsWin {
            // drawn by draw_spec_over
        } else if self.holiday.holiday.is_night_fireworks() && self.glow > 0.45 {
            // B36: fireworks over the city on New Year's Eve and the 4th of July.
            crate::holiday::fireworks(
                dst,
                self.width(),
                self.height(),
                self.holiday_t,
                self.seed,
                &self.holiday,
            );
        }
        // B50: smoke, fire, creatures and ships over the world.
        if self.ending.is_some() {
            if let Some(ctx) = self.ending_ctx() {
                dst.set_clip(r);
                if r.intersects(&self.ending_wbb) {
                    crate::ending::draw_world(dst, &ctx);
                }
            }
        }
        lap.mark(11);
        // B57/D72: the weather. The band's deck first — only where the static
        // layer is empty, so a spire still stands in front of it — then the low
        // field drifting over the land and the buildings, semi-transparent.
        if !self.clouds.is_empty() {
            dst.set_clip(r);
            // B57b: the weather is *only* in the sky band — the camera is below
            // the clouds, so nothing is painted over the land. What the city
            // feels is the shade (`draw_cloud_shadows` above).
            self.clouds.draw_band_over(dst, &self.cloud_ink, Some(self.static_layer.px()));
        }
        lap.mark(12);
        dst.reset_clip();
        // B62: the lightning bolt, in front of the clouds.
        if self.spec.bolt_on() {
            dst.set_clip(r);
            self.draw_bolt(dst);
            dst.reset_clip();
        }
        // Rain covers the whole frame, so a rainy frame is always a full one.
        if let Some(rr) = &self.rain {
            rr.draw(dst);
        }
        lap.mark(13);
        // B61b: the weather's washes and cloud shadows belong to the scene, not
        // to a per-frame pass over the finished frame. `r` clips both, so a
        // damaged rectangle costs a damaged rectangle.
        if self.ambient.is_active() {
            self.ambient.apply(dst, r);
        }
        lap.mark(14);
    }

    fn ending_ctx(&self) -> Option<crate::ending::Ctx<'_>> {
        let fx = self.ending.as_ref()?;
        Some(crate::ending::Ctx {
            fx,
            cam: &self.cam,
            map: &self.map,
            tiles: &self.ending_tiles,
            marks: &self.ending_marks,
            drones: &self.ending_drones,
            depth: self.depth(),
            stat: self.static_layer.px(),
            tint: self.tint,
            glow: self.glow,
            t: self.ending_clock.unwrap_or_else(|| self.started_at_secs()),
            sky_h: self.sky_h as i32,
            w: self.width() as i32,
            h: self.height() as i32,
        })
    }

    /// B50: pin the ending's motion clock (a still, or a loop frame at a
    /// chosen time) instead of following the wall clock.
    pub fn set_ending_clock(&mut self, t: Option<f32>) {
        self.ending_clock = t;
    }

    /// B61b: replace the weather's static contribution (the washes). Returns
    /// true when the washes changed, which means every pixel moved: the caller
    /// composites once and then the sky costs nothing until the weather book
    /// ticks again (1 Hz at most).
    ///
    /// No allocation: the blob list and the per-row table are copied in place.
    pub fn set_ambient(&mut self, a: &crate::weather::Ambient, frame: Rect) -> bool {
        let changed = self.ambient.rows_height() != frame.y1.max(1) as usize;
        // B61d: the washes are compared at a 64th of full strength, not a
        // 512th. A live weather book wobbles by a few 512ths every frame while
        // rain is falling, and every wobble re-composited *the whole frame*
        // (30-45 ms at 7040x1440) — which is why a rainy rush hour was 60 % of a
        // core while the same city in clear weather was 25 %. A 64th is under
        // one percent on any pixel a wash tints, and the washes are a slow
        // gradient anyway (D71 forbids cutting animation, not quantising a
        // value nobody can see move).
        let q = |v: f32| (v.clamp(0.0, 1.0) * 64.0 + 0.5) as u32;
        let key_now = q(a.dim) | q(a.fog) << 8 | q(a.mist) << 16 | q(a.haze) << 24;
        let key_old = q(self.ambient.dim)
            | q(self.ambient.fog) << 8
            | q(self.ambient.mist) << 16
            | q(self.ambient.haze) << 24;
        let wash_changed = key_now != key_old
            || (a.flash - self.ambient.flash).abs() > 1.0 / 64.0
            || a.washes_active() != self.ambient.washes_active();
        self.ambient.copy_fields(a);
        let air_moved = self.ambient.set_air(self.sky_h, self.tint);
        self.spec_flash(a.flash);
        let wash_changed = wash_changed || (air_moved && a.washes_active());
        if wash_changed || changed {
            self.ambient.rebuild_rows(frame.y1.max(1) as usize);
            self.mark_full();
        }
        wash_changed || changed
    }

    /// The ambient the next composite will draw (for tests and the presenter).
    pub fn ambient(&self) -> &crate::weather::Ambient {
        &self.ambient
    }

    /// Seconds since the scene was made, for the smoke drift (B39).
    fn started_at_secs(&self) -> f32 {
        self.seed as f32 * 0.0 + (self.holiday_t.max(0.0))
    }

    /// Does the weather overlay have to be redrawn over the whole frame?
    pub fn rain_active(&self) -> bool {
        self.rain.is_some()
    }

    /// Level (0..=MAX_LEVEL) of the tallest terrain, for reporting.    /// Level (0..=MAX_LEVEL) of the tallest terrain, for reporting.
    pub fn max_level(&self) -> u8 {
        self.map
            .tiles
            .iter()
            .map(|t| t.max_level())
            .max()
            .unwrap_or(0)
    }

    /// Screen y of a diagonal's base line (used by tests and the handback).
    pub fn diagonal_y(&self, d: i32) -> i32 {
        self.cam.oy + d * 8
    }

    /// How water picks up the sky (B21 item 4). The colour is baked into the
    /// static layer, so a real change re-rasterises it — which happens on the
    /// palette cadence (about once a minute), not per frame.
    pub fn set_water_tint(&mut self, tint: crate::palette::Color) {
        let night_moved = self.water_baked_night != self.water.night_q;
        if self.water.tint != tint || night_moved {
            self.water_prims.clear();
            self.water.tint = tint;
            self.water_baked_night = self.water.night_q;
            self.rebuild_static();
            if night_moved {
                // B64: the baked tiles are the *source*; the tinted copy the
                // compositor reads must follow, or the water's new night
                // colour only shows at the next unrelated re-tint.
                self.retint_static();
                self.mark_full();
            }
        }
    }

    /// The villagers to draw on the next composite (D45).
    /// Replace the villagers. Walkers outside the framebuffer are dropped here
    /// (B26d): they cannot change a pixel, and keeping them made the wallpaper
    /// wake at 6 fps for figures nobody can see.
    pub fn set_walkers(&mut self, walkers: Vec<(f32, f32, u8)>) {
        let old: Vec<Rect> = self
            .walkers
            .iter()
            .map(|&(x, y, _)| self.screen_rect(x, y, 4))
            .collect();
        for r in old {
            self.mark_rect(r);
        }
        self.walkers = walkers
            .into_iter()
            .filter(|&(x, y, _)| self.is_visible(x.floor() as i32, y.floor() as i32))
            .collect();
        let old: Vec<Rect> = self
            .walkers
            .iter()
            .map(|&(x, y, _)| self.screen_rect(x, y, 4))
            .collect();
        for r in old {
            self.mark_rect(r);
        }
    }

    /// The villager list, for the caller to compare with the next one.
    pub fn walkers_snapshot(&self) -> Vec<(f32, f32, u8)> {
        self.walkers.clone()
    }

    /// Replace the residents' layer (B43/D65). Figures outside the frame are
    /// dropped — they cannot change a pixel — and both the old and the new
    /// positions are marked dirty, so a figure leaving a spot repaints it.
    ///
    /// D71: this runs once per display frame, so it allocates nothing, keeps
    /// the vector's capacity, and marks **only the figures that would change a
    /// pixel** — a standing crowd on a still frame costs the comparison and no
    /// composite work at all. Returns true when anything moved.
    pub fn set_people(&mut self, people: &[crate::people::Figure], t: f32) -> bool {
        self.advance_rigs(t);
        let mut new = std::mem::take(&mut self.people_buf);
        let mut new_px = std::mem::take(&mut self.people_px_buf);
        new.clear();
        new_px.clear();
        // B62: the festival crowd, the band and the street performers are
        // people like any other: same sprites, same depth, same damage rules.
        let extra = std::mem::take(&mut self.spec.extra);
        let civic_figs = std::mem::take(&mut self.civic.dynamic.figures);
        let mut regular = usize::MAX;
        for (fi, f) in people.iter().chain(extra.iter()).chain(civic_figs.iter()).enumerate() {
            if fi == people.len() {
                regular = new.len();
            }
            if !self.is_visible(f.x.floor() as i32, f.y.floor() as i32) {
                continue;
            }
            // One projection per figure per frame, kept for the composite.
            let z = self.map.ground_px(f.x, f.y);
            let (sx, sy) = self.cam.project(f.x, f.y, z + f.lift as i32 * 2);
            let frame = crate::people::walk_frame(f, t);
            new.push(crate::people::Figure { frame, ..*f });
            new_px.push((sx as i32, sy as i32));
        }
        self.spec.extra = extra;
        self.civic.dynamic.figures = civic_figs;
        self.spec.extra_from = regular;
        // B75: put the list in painter order (top of the screen first), so the
        // band index and a plain walk of the list visit the figures in the same
        // order and an overlapping crowd composites identically either way.
        // Without this a packed square flickered between two composites.
        let mut sorted = std::mem::take(&mut self.people_sorted);
        sorted.clear();
        sorted.extend(
            new.iter()
                .copied()
                .zip(new_px.iter().copied())
                .map(|(f, (px, py))| (py, px, f)),
        );
        sorted.sort_unstable_by_key(|(py, px, _)| (*py, *px));
        new.clear();
        new_px.clear();
        for (py, px, f) in sorted.iter() {
            new.push(*f);
            new_px.push((*px, *py));
        }
        // B84 r2: damage **after** the sort, so index `i` of this frame is the
        // same rank as index `i` of the last one. Comparing before the sort
        // paired the painter order of the previous frame with the sim's own
        // order of this one — i.e. every figure with a different person — and
        // every figure's box became the distance between two strangers: 10.4 M
        // pixels of damage for 150 figures, 29 % of the frame, every frame.
        let mut changed = false;
        for i in 0..new.len() {
            let f = new[i];
            let (px, py) = new_px[i];
            let old = self.people.get(i).copied();
            if matches!(old, Some(o) if o.same_pixel(&f, f.frame)) {
                continue;
            }
            changed = true;
            // One rectangle per figure: the old and the new position almost
            // always overlap, and one box is half the composite work of two.
            let r = Rect::new(px - 7, py - 14, px + 7, py + 7);
            let mut r = self.with_shadow(r, f.x, f.y, if f.child { 7.0 } else { 11.0 });
            if let Some(o) = old {
                let (ox, oy) = self.people_px.get(i).copied().unwrap_or((px, py));
                let ro = Rect::new(ox - 7, oy - 14, ox + 7, oy + 7);
                r = r.union(&self.with_shadow(ro, o.x, o.y, if o.child { 7.0 } else { 11.0 }));
            }
            self.mark_rect(r);
        }
        // Everything the old list had beyond the new one has left the screen.
        if self.people.len() > new.len() {
            for i in new.len()..self.people.len() {
                let o = self.people[i];
                let r = self.screen_rect(o.x, o.y, 7);
                let r = self.with_shadow(r, o.x, o.y, if o.child { 7.0 } else { 11.0 });
                self.mark_rect(r);
                changed = true;
            }
        }
        self.people_sorted = sorted;
        self.people_buf = std::mem::replace(&mut self.people, new);
        self.people_px_buf = std::mem::replace(&mut self.people_px, new_px);
        // B61e: one index per frame, from the positions just computed.
        let h = self.height() as i32;
        self.people_bands
            .rebuild(h, self.people_px.iter().enumerate().map(|(i, p)| (p.1, i as u32)));
        self.moving_changed |= changed;
        changed
    }


    /// B43c: advance the work clock and mark the rigs whose pixels changed.
    /// The clock only runs while the crews are in (the people's own working
    /// hours), so an idle site at night marks nothing at all.
    fn advance_rigs(&mut self, t: f32) {
        if self.rigs.is_empty() {
            self.rig_last_t = t;
            return;
        }
        let hour = (t / 3600.0).rem_euclid(24.0);
        let weekday = (((t as f64 / 86400.0) + 3.0).floor().rem_euclid(7.0)) as i32;
        let work = crate::rigs::site_hours(hour, weekday);
        let dt = if self.rig_last_t.is_nan() { 0.0 } else { (t - self.rig_last_t).clamp(0.0, 0.25) };
        self.rig_last_t = t;
        if work {
            self.rig_clock += dt as f64;
        }
        let work_changed = work != self.rig_work;
        self.rig_work = work;
        let clock = self.rig_t();
        let cam = self.cam;
        let shadow = self.light.actor();
        let mut marks: Vec<Rect> = Vec::new();
        for r in self.rigs.iter_mut() {
            let sig = r.signature(clock, work);
            let sig = if work_changed && matches!(r.kind, crate::rigs::Kind::Mixer | crate::rigs::Kind::Truck) {
                sig ^ 0x5555
            } else {
                sig
            };
            if sig != r.sig {
                let now = r.bbox(&cam, clock, shadow.as_ref());
                marks.push(now.union(&r.last));
                r.last = now;
                r.sig = sig;
            }
        }
        for m in marks {
            self.mark_rect(m);
        }
    }

    /// The rigs' work clock as they read it, wrapped like `sign_t` (B93: an f32
    /// that only grew stopped resolving a frame step after weeks of work hours).
    fn rig_t(&self) -> f32 {
        (self.rig_clock % 65536.0) as f32
    }

    /// Number of rigs on the map (for the reports).
    pub fn rig_count(&self) -> usize {
        self.rigs.len()
    }

    /// The materials standing at the construction sites (B43). Few and stable,
    /// so this marks only the squares that changed.
    pub fn set_sites(&mut self, sites: Vec<(f32, f32, u8)>) {
        if sites == self.sites {
            return;
        }
        let old: Vec<Rect> = self.sites.iter().map(|&(x, y, _)| self.screen_rect(x, y, 8)).collect();
        for r in old {
            self.mark_rect(r);
        }
        let new: Vec<Rect> = sites.iter().map(|&(x, y, _)| self.screen_rect(x, y, 8)).collect();
        for r in new {
            self.mark_rect(r);
        }
        self.sites = sites;
    }

    /// The residents' list, for the caller to compare with the next one.
    pub fn people_snapshot(&self) -> &[crate::people::Figure] {
        &self.people
    }

    /// On-screen figures (after culling), for the callers' idle checks.
    pub fn people_count(&self) -> usize {
        self.people.len()
    }

    /// The screen position of every drawn figure (B75: the frame's own report).
    pub fn people_px(&self) -> &[(i32, i32)] {
        &self.people_px
    }

    /// How many villagers would be drawn (0 means the tick can be skipped).
    pub fn walker_count(&self) -> usize {
        self.walkers.len()
    }

    /// Is there anything for the 6 fps water tick to change? (B26c: asking this
    /// every idle wake by scanning the item list cost ~0.9 % of a core on the owner's
    /// desktop, so the count is kept up to date instead.)
    pub fn water_live(&self) -> bool {
        self.water.anim && (self.water.brightness >= 0.08 || self.water.glint >= 0.05) && self.water_count > 0
    }

    /// Set the weather the ground and the overlays are drawn with. Returns true
    /// when it changed enough to re-rasterise the static layer.
    pub fn set_weather(&mut self, wx: terrain::WeatherLook) -> bool {
        // B89: what the weather does to the sea. The cloud cover is baked (its
        // bucket re-rasterises below); roughness and rain only move strokes,
        // which the water tick diffs.
        let q = |v: f32, n: f32| (v.clamp(0.0, 1.0) * n).round() / n;
        self.water.cloud = q(wx.cloud, 10.0);
        self.water.rain = q(wx.rain, 4.0);
        self.water.chop = q(0.22 + wx.wind_kmh / 70.0 + 0.45 * wx.storm.clamp(0.0, 1.0) + 0.10 * wx.rain.clamp(0.0, 1.0), 8.0);
        self.sync_water_glint();
        let b = wx.buckets();
        let changed = b != self.weather_baked;
        self.weather = wx;
        if changed {
            self.weather_baked = b;
            // B76: wet roads and lying snow are *baked* into the static ground
            // (`weatherise`), so a visible step has to re-rasterise it. Until
            // B76 this only marked a full composite, which is why snow cover
            // never reached the screen outside an ending.
            self.request_static_rebuild();
        }
        changed
    }

    pub fn weather(&self) -> terrain::WeatherLook {
        self.weather
    }

    /// B28: hand the scene the season. Returns true when it changed enough to
    /// re-rasterise the static layer (a few times a day at most).
    pub fn set_season(&mut self, season: terrain::SeasonLook) -> bool {
        let b = season.buckets();
        let changed = b != self.season_baked;
        self.season = season;
        // B69: what the gusts shake loose: turning leaves in autumn, petals in
        // spring.
        {
            let col = |c: [u8; 3]| palette::rgb(c[0], c[1], c[2]);
            let shed = if season.autumn > 0.25 { season.autumn * (1.0 - 0.7 * season.bareness) } else { 0.0 };
            let petals = if season.blossom > 0.2 { season.blossom * 0.8 } else { 0.0 };
            if petals > shed {
                self.trees.set_leaf_look(
                    [palette::rgb(0xf6, 0xd2, 0xe0), palette::rgb(0xf2, 0xb8, 0xcc), palette::rgb(0xfa, 0xe4, 0xec)],
                    petals,
                );
            } else {
                self.trees.set_leaf_look([col(season.turn[0]), col(season.turn[1]), col(season.turn[2])], shed);
            }
        }
        // B76: winter clothes. The people are sprites, not the static layer, so
        // this costs nothing to switch (no rebuild).
        let winter = season.bareness > 0.55 || season.grass_dry > 0.55;
        self.people_sprites.set_winter(winter);
        if changed {
            self.season_baked = b;
            // The trees, grass and fields are baked into the static layer, so a
            // season change re-rasterises it (B28: at most a few times a day).
            self.request_static_rebuild();
        }
        changed
    }

    pub fn season(&self) -> terrain::SeasonLook {
        self.season
    }

    /// Dress the city for a holiday (B36). A change re-rasterises the static
    /// layer once (a holiday lasts a day, so that is cheap).
    pub fn set_holiday(&mut self, look: crate::holiday::HolidayLook) -> bool {
        // B76: the lights go up house by house through December, so the share
        // is compared in tenths — ten rebuilds over the season, not one a day.
        let lights = |v: f32| (v * 10.0).round() as i32;
        let same = self.holiday.holiday == look.holiday
            && self.holiday.bulbs == look.bulbs
            && self.holiday.garland == look.garland
            && lights(self.holiday.lights) == lights(look.lights);
        if same {
            return false;
        }
        self.holiday = look;
        self.request_static_rebuild();
        true
    }

    /// The plaza gets its market stalls when the caller says it is a market day.
    pub fn set_market(&mut self, on: bool) -> bool {
        if self.market == on {
            return false;
        }
        self.market = on;
        self.request_static_rebuild();
        true
    }

    /// Run a parade along the main avenue (B36): `progress` 0..1 is how far the
    /// head of the procession has gone.
    pub fn set_parade(&mut self, progress: Option<f32>) -> bool {
        let changed = match (self.parade, progress) {
            (None, None) => false,
            (Some(a), Some(b)) => (a - b).abs() > 0.004,
            _ => true,
        };
        if !changed {
            return false;
        }
        // The floats move, so the band of the avenue they crossed is damaged.
        let old = self.parade;
        self.parade = progress;
        for p in [old, progress].into_iter().flatten() {
            for r in self.parade_rects(p) {
                self.mark_rect(r);
            }
        }
        true
    }

    /// The screen rectangles the procession covers at `progress`.
    fn parade_rects(&self, progress: f32) -> Vec<Rect> {
        let line = &self.fixtures.tram_line;
        if line.len() < 2 {
            return Vec::new();
        }
        let mut out = Vec::new();
        let floats = 6;
        for i in 0..floats {
            let t = (progress - i as f32 * 0.02).rem_euclid(1.0);
            let idx = ((t * line.len() as f32) as usize).min(line.len() - 1);
            let (x, y) = line[idx];
            let z = self.map.ground_px(x as f32, y as f32);
            let (sx, sy) = self.cam.project(x as f32, y as f32, z);
            out.push(Rect::new(sx as i32 - 22, sy as i32 - 24, sx as i32 + 22, sy as i32 + 10));
        }
        out
    }

    /// B50: the world's ending, as the sim describes it right now (`None`
    /// while the world simply lives). Returns true when the picture changed
    /// enough that the frame must be composited again.
    pub fn set_ending(&mut self, fx: Option<crate::ending::Fx>) -> bool {
        let key = fx.as_ref().map(|f| f.key()).unwrap_or(0);
        let was_none = self.ending.is_none();
        let now_none = fx.is_none();
        let slow = fx.as_ref().map(|f| (f.secs * 2.0) as u64 + 1).unwrap_or(0);
        let fires = !self.ending_marks.is_empty();
        self.ending = fx;
        if let Some(f) = self.ending.as_mut() {
            f.fires = fires;
        }
        // The slow things changed (or the ending began or ended): one full
        // composite, with the sky re-baked.
        if key != self.ending_key || was_none != now_none {
            self.ending_key = key;
            self.ending_slow = slow;
            if self.sky.h > 0 {
                self.bake_ending_sky();
                self.resample_haze();
            }
            self.ending_rects.clear();
            self.refresh_ending_bounds(true);
            // What changed is the sky band, the haze under it and the ground
            // overlays: recomposite those in slices over the next few frames
            // (a full-frame composite at 7040 wide is a visible hitch).
            self.ending_slices.clear();
            let w = self.width() as i32;
            let top = (self.sky_h as f32 + self.haze_depth()) as i32;
            let mut regions = vec![Rect::new(0, 0, w, top.min(self.height() as i32))];
            if !self.ending_gbb.is_empty() {
                regions.push(self.ending_gbb);
            }
            let full = Rect::from_size(self.width(), self.height());
            for r in regions {
                let r = r.intersect(&full);
                let mut y = r.y0;
                while y < r.y1 {
                    let y1 = (y + 300).min(r.y1);
                    self.ending_slices.push(Rect::new(r.x0, y, r.x1, y1));
                    y = y1;
                }
            }
            self.ending_slices.reverse();
            if was_none != now_none {
                self.ending_slices.clear();
                self.full_pending = true;
                self.pending.clear();
            } else if let Some(sl) = self.ending_slices.pop() {
                self.pending.push(sl);
            }
            return true;
        }
        // The next slice of a re-bake, if one is still being spread out.
        let mut popped = false;
        if let Some(sl) = self.ending_slices.pop() {
            self.pending.push(sl);
            popped = true;
        }
        // Everything else that moves is damage: where it was and where it is.
        let animated = self.ending.as_ref().map(|f| f.animated()).unwrap_or(false);
        if !animated {
            return popped;
        }
        let tick = slow != self.ending_slow;
        self.ending_slow = slow;
        let new = match self.ending_ctx() {
            Some(ctx) => crate::ending::damage_rects(&ctx, tick),
            None => Vec::new(),
        };
        let old = std::mem::take(&mut self.ending_rects);
        let full = Rect::from_size(self.width(), self.height());
        if !self.full_pending {
            for r in old.iter().chain(new.iter()) {
                if r.area() >= full.area() / 2 {
                    self.full_pending = true;
                    self.pending.clear();
                    break;
                }
            }
            if !self.full_pending {
                for r in old.iter().chain(new.iter()) {
                    self.pending.push(*r);
                }
            }
        }
        self.ending_rects = new;
        self.refresh_ending_bounds(false);
        true
    }

    /// B50: where the ending's layers can draw (see `ending::ground_bounds`).
    fn refresh_ending_bounds(&mut self, ground: bool) {
        let (g, w) = match self.ending_ctx() {
            Some(ctx) => (
                if ground { Some(crate::ending::ground_bounds(&ctx)) } else { None },
                crate::ending::world_bounds(&ctx, &self.ending_rects),
            ),
            None => (Some(Rect::EMPTY), Rect::EMPTY),
        };
        if let Some(g) = g {
            self.ending_gbb = g;
        }
        self.ending_wbb = w;
    }

    /// B50: is a re-bake still being spread over frames?
    pub fn ending_busy(&self) -> bool {
        !self.ending_slices.is_empty()
    }

    /// B50: is an ending on, and does it move every frame?
    pub fn ending_animated(&self) -> bool {
        self.ending.as_ref().map(|f| f.animated()).unwrap_or(false)
    }

    /// B50: is an ending on?
    pub fn ending_on(&self) -> bool {
        self.ending.is_some()
    }

    /// B50: the overlay tiles (lava, ice, traces, foam).
    pub fn set_ending_tiles(&mut self, tiles: Vec<(i32, i32, f32)>) {
        self.ending_tiles = tiles;
        self.full_pending = true;
        self.refresh_ending_bounds(true);
    }

    /// B50: lots that burn or smoulder, as `(x, y, code, z px)`.
    pub fn set_ending_marks(&mut self, marks: Vec<(f32, f32, u8, f32)>) -> bool {
        if self.ending_marks != marks {
            self.ending_marks = marks;
            self.refresh_ending_bounds(false);
            return true;
        }
        false
    }

    /// B50: converted residents, drawn as drones.
    pub fn set_ending_drones(&mut self, drones: Vec<(f32, f32)>) {
        self.ending_drones = drones;
    }

    /// B50: the morph. The viewer's copy of the map learns that the next land
    /// is coming in (`neighbour`), which tiles have turned, and their water.
    pub fn sync_morph(&mut self, neighbour: Option<map::Biome>, flipped: &[(i32, i32)], classes: &[(i32, i32, u8)]) {
        if let Some(n) = neighbour {
            self.map.begin_morph(n);
        }
        for &(x, y) in flipped {
            self.map.morph_tile(x, y);
        }
        let w = self.map.w;
        for &(x, y, c) in classes {
            if self.map.in_bounds(x, y) {
                self.map.water_class[(y * w + x) as usize] = c;
            }
        }
    }

    /// B39: the settlers walking in, the camp, and the logs they have cut.
    pub fn set_settlers(&mut self, party: Vec<(f32, f32, u8, f32)>) -> bool {
        let changed = party.len() != self.settlers.len()
            || party.iter().zip(self.settlers.iter()).any(|(a, b)| {
                (a.0 - b.0).abs() > 0.02 || (a.1 - b.1).abs() > 0.02 || a.2 != b.2
            });
        if !changed {
            return false;
        }
        self.settlers = party;
        true
    }

    /// B41: hand over the installed street furniture. Only the tiles that
    /// changed are redrawn (a crew installs one tile at a time).
    pub fn set_street(&mut self, look: &crate::street::StreetLook) {
        let mut paint = std::collections::BTreeMap::new();
        for &(x, y, s) in &look.paint {
            paint.insert((x, y), s);
        }
        let mut lamps = std::collections::BTreeSet::new();
        for &(x, y) in &look.lamps {
            lamps.insert((x, y));
        }
        let mut signs = std::collections::BTreeMap::new();
        for &(x, y, k) in &look.signs {
            signs.insert((x, y), k);
        }
        let mut lights = std::collections::BTreeSet::new();
        for &(x, y) in &look.lights {
            lights.insert((x, y));
        }
        let mut dirty: Vec<(i32, i32)> = Vec::new();
        for &(x, y) in paint.keys() {
            if self.street.paint.get(&(x, y)) != paint.get(&(x, y)) {
                dirty.push((x, y));
            }
        }
        for &(x, y) in lamps.iter() {
            if !self.street.lamps.contains(&(x, y)) {
                dirty.push((x, y));
            }
        }
        for &(x, y) in signs.keys() {
            if !self.street.signs.contains_key(&(x, y)) {
                dirty.push((x, y));
            }
        }
        for &(x, y) in lights.iter() {
            if !self.street.lights.contains(&(x, y)) {
                dirty.push((x, y));
            }
        }
        self.street.paint = paint;
        self.street.lamps = lamps;
        // B61e: the lamps' row index follows the furniture.
        self.rebuild_place_index();
        self.street.signs = signs;
        self.street.lights = lights;
        for (x, y) in dirty {
            self.dirty_tile(x, y);
        }
        self.flush_dirty();
    }

    /// B49: hand over what the technology crews have installed. Only the tiles
    /// that changed are redrawn (a crew installs one tile at a time), and a
    /// rooftop's panels dirty the whole lot, not just its anchor tile.
    pub fn set_tech(&mut self, look: &crate::tech::TechLook) {
        let next = crate::tech::TechKey::from_look(look);
        let bits = |k: &crate::tech::TechKey| {
            let mut m: std::collections::BTreeMap<(i32, i32), u32> =
                std::collections::BTreeMap::new();
            for (&(x, y), &l) in &k.lamps {
                *m.entry((x, y)).or_insert(0) |= (l as u32 + 1) & 0b11;
            }
            for &(x, y) in &k.poles {
                *m.entry((x, y)).or_insert(0) |= 1 << 2;
            }
            for &(x, y) in &k.buried {
                *m.entry((x, y)).or_insert(0) |= 1 << 3;
            }
            for &(x, y) in &k.towers {
                *m.entry((x, y)).or_insert(0) |= 1 << 4;
            }
            for &(x, y) in &k.cells {
                *m.entry((x, y)).or_insert(0) |= 1 << 5;
            }
            for &(x, y) in &k.farm {
                *m.entry((x, y)).or_insert(0) |= 1 << 6;
            }
            for &(x, y) in &k.turbines {
                *m.entry((x, y)).or_insert(0) |= 1 << 7;
            }
            for &(x, y) in &k.chargers {
                *m.entry((x, y)).or_insert(0) |= 1 << 8;
            }
            for (&(x, y), &s) in &k.solar {
                *m.entry((x, y)).or_insert(0) |= 1 << (9 + (s as u32 & 0b11));
            }
            m
        };
        let base_changed = next.lamp_base != self.tech.lamp_base;
        let old = bits(&self.tech);
        let new = bits(&next);
        let mut dirty: Vec<(i32, i32)> = Vec::new();
        for (k, v) in &new {
            if old.get(k) != Some(v) {
                dirty.push(*k);
            }
        }
        for k in old.keys() {
            if !new.contains_key(k) {
                dirty.push(*k);
            }
        }
        self.tech = next;
        // The world's lamp baseline moved (a new head the town fits now): every
        // lamp without an entry of its own has changed.
        if base_changed {
            dirty.extend(self.street.lamps.iter().copied());
        }
        if dirty.is_empty() {
            return;
        }
        for (x, y) in dirty {
            // A rooftop's panels reach above the tile: dirty the whole lot.
            let lot = self
                .city
                .iter()
                .find(|o| o.x == x && o.y == y)
                .map(|o| (o.x, o.y, o.w, o.h, o.level));
            match lot {
                Some((lx, ly, lw, lh, lv)) => {
                    self.dirty_lot(lx, ly, lw, lh, lv);
                }
                None => {
                    self.dirty_tile(x, y);
                }
            }
        }
        self.flush_dirty();
    }

    /// B49b: the wind farm's turbine tiles (the masts are in the static layer;
    /// the rotors are drawn per frame and damaged per frame).
    pub fn set_turbines(&mut self, tiles: Vec<(i32, i32)>) {
        if tiles == self.turbines {
            return;
        }
        // Dirty the swap both ways: a turbine that has just gone up, and one
        // that has gone.
        let marks: Vec<(i32, i32)> = self.turbines.iter().chain(tiles.iter()).copied().collect();
        for (x, y) in marks {
            let r = crate::tech::rotor_rect(&self.cam, &self.map, x, y)
                .union(&self.tile_rect_at(x, y, self.map.at(x, y).max_level()));
            if !r.is_empty() {
                self.dirty.push(r);
            }
            self.dirty_tile(x, y);
        }
        self.turbines = tiles;
    }

    /// B73r2: the sim's town facts (D80's gate: population and era, like the
    /// tower cap). Cheap — rebuilding the plan is ~0.5 ms and only happens when
    /// the numbers actually move.
    pub fn set_town_stats(&mut self, population: u32, era: u8) {
        let stats = crate::nightlife::TownStats { population, era };
        if stats == self.town_stats {
            return;
        }
        self.town_stats = stats;
        if self.city.is_empty() {
            return;
        }
        let land = crate::nightlife::Land::of(self.map.biome);
        self.night = crate::nightlife::build_in(
            &self.city,
            &self.kit,
            &self.cam,
            land,
            self.width() as i32,
            self.height() as i32,
            stats,
            Some(&self.map),
        );
        self.sign_slot = -1;
        self.mark_full();
    }

    pub fn town_stats(&self) -> crate::nightlife::TownStats {
        self.town_stats
    }

    /// B73/D80: advance the nightlife's own clock and damage the animated panels
    /// (chasers, screens, slow crown cycles, bad tubes) a few times a second,
    /// plus the neon quarter's steam, which moves every frame.
    ///
    /// Only the panels whose picture actually changed on this slot are marked, so
    /// a night city costs a handful of small boxes — never a full frame. By day
    /// (`glow` low) nothing animates and nothing is marked.
    pub fn sign_tick(&mut self, dt: f32) -> bool {
        // B93: the clock is kept in f64 and handed out wrapped (`sign_t`): as an
        // f32 that only ever grew, it stopped resolving a frame step after about
        // twelve days and every sign froze.
        self.sign_clock += dt.max(0.0) as f64;
        let slot = (self.sign_t() * crate::nightlife::ANIM_HZ) as i64;
        let new_slot = slot != self.sign_slot;
        self.sign_slot = slot;
        if self.night.is_empty() && !self.night.steam_live() {
            return false;
        }
        if self.glow < 0.18 {
            return false;
        }
        let frame = self.static_layer.bounds();
        let mut raw = std::mem::take(&mut self.sign_rects);
        raw.clear();
        if new_slot {
            // The panels that changed, and the steam's puffs: one damage pass,
            // a few times a second, so the puffs step with the signage's own
            // cadence rather than repainting the quarter every frame (D71's
            // budget is a few small boxes a frame, and a soft puff at 6 Hz reads
            // the same as the chaser bulbs do).
            self.night.animated_rects(frame, slot, &mut raw);
            if self.night.steam_live() && std::env::var("CITYLIVE_NO_STEAM").is_err() {
                self.night.steam_rects(frame, &mut raw);
            }
        }
        if raw.is_empty() {
            self.sign_rects = raw;
            return false;
        }
        // Merge only what touches: a marquee and the screen next to it are one
        // repaint, but two panels a street apart stay two boxes (the coalesce
        // pass in `take_pending` widens each to its 64x60 cell anyway, and a
        // chain of them would repaint half the block).
        let merged = merge_rects(raw, 6);
        for r in &merged {
            self.mark_rect(*r);
        }
        self.sign_rects = merged;
        true
    }

    /// Pin the nightlife clock (still renders: a frame strip over ~10 s is the
    /// same city six times with the chasers and screens at different moments).
    pub fn set_sign_clock(&mut self, t: f32) {
        self.sign_clock = t.max(0.0) as f64;
        self.sign_slot = -1;
    }

    /// The nightlife clock as the signs read it: seconds, wrapped every 65536 s
    /// (a whole number of 1/ANIM_HZ slots, and an even number, so the slot lanes
    /// agree across the wrap) to keep the f32 fine-grained.
    pub fn sign_t(&self) -> f32 {
        (self.sign_clock % 65536.0) as f32
    }

    pub fn sign_clock(&self) -> f32 {
        self.sign_t()
    }

    /// The nightlife plan (tests, handback counts).
    pub fn nightlife(&self) -> &crate::nightlife::Plan {
        &self.night
    }

    /// Time the nightlife layer alone over a whole frame, `reps` times. Returns
    /// (fastest, mean) in ms — the fastest is the honest number on a shared box.
    pub fn sign_layer_ms(&self, dst: &mut Framebuffer, reps: usize) -> (f64, f64) {
        if self.night.is_empty() {
            return (0.0, 0.0);
        }
        let look = crate::nightlife::Look {
            night: self.glow,
            hour: self.light.hour,
            t: self.sign_t(),
            wet: self.weather.wet,
            land: crate::nightlife::Land::of(self.map.biome),
        };
        let clip = Rect::from_size(dst.w, dst.h);
        let mut best = f64::MAX;
        let mut sum = 0.0;
        for _ in 0..reps.max(1) {
            let t = std::time::Instant::now();
            crate::nightlife::draw(&self.night, dst, clip, &look, self.static_layer.depth());
            let ms = t.elapsed().as_secs_f64() * 1000.0;
            best = best.min(ms);
            sum += ms;
        }
        (best, sum / reps.max(1) as f64)
    }

    /// B49b/D71: mark every visible rotor's own rectangle for repaint, so the
    /// blades turn smoothly without repainting the frame. Nothing is marked when
    /// there is no turbine in the picture, which is what keeps the idle cost at
    /// zero (the animation is never cut, only scoped — D71).
    pub fn turbine_tick(&mut self) {
        if self.turbines.is_empty() {
            return;
        }
        let frame = self.static_layer.bounds();
        let land = self.land_clip();
        for &(x, y) in &self.turbines {
            let r = crate::tech::rotor_rect(&self.cam, &self.map, x, y)
                .intersect(&frame)
                .intersect(&land);
            if !r.is_empty() && r.intersects(&land) {
                // B61d: the rotor is drawn into the *frame*, not the static
                // layer (which only holds the mast), so this is frame damage and
                // not a static-layer repaint. Queueing it as a dirty *tile* made
                // every flush re-rasterise the buildings around every rotor —
                // four rotors, 2 ms a frame, forever.
                self.pending.push(r);
            }
        }
    }

    /// B49b: the technology crews at work (van, ladder, worker), drawn per frame
    /// through the depth plane so a building in front hides them.
    pub fn set_tech_crews(&mut self, crews: Vec<(i32, i32, u8)>) -> bool {
        if crews == self.tech_crews {
            return false;
        }
        self.tech_crews = crews;
        true
    }

    /// B49: the delivery drones, in tile space (empty when there are none).
    pub fn set_drones(&mut self, drones: Vec<(f32, f32)>) -> bool {
        let changed = drones.len() != self.drones.len()
            || drones
                .iter()
                .zip(self.drones.iter())
                .any(|(a, b)| (a.0 - b.0).abs() > 0.01 || (a.1 - b.1).abs() > 0.01);
        if !changed {
            return false;
        }
        // Their old and new spots, and the shadow on the ground below them.
        for (x, y) in self.drones.iter().chain(drones.iter()) {
            let z = crate::iso::level_px(self.map.surface_level_at(*x, *y));
            let p = self.cam.project(*x, *y, z + 44);
            let r = Rect::new(p.0 as i32 - 6, p.1 as i32 - 6, p.0 as i32 + 6, p.1 as i32 + 10);
            let g = self.cam.project(*x, *y, z + 1);
            let sg = Rect::new(g.0 as i32 - 3, g.1 as i32 - 3, g.0 as i32 + 4, g.1 as i32 + 4);
            if !r.is_empty() {
                self.dirty.push(r);
            }
            if !sg.is_empty() {
                self.dirty.push(sg);
            }
        }
        self.drones = drones;
        true
    }

    /// B41: the crews at work, and the traffic lights, drawn per frame.
    pub fn set_street_crews(&mut self, crews: Vec<(i32, i32, u8)>) -> bool {
        if self.street_crews != crews {
            self.street_crews = crews;
            return true;
        }
        false
    }

    /// B40: the trees going over right now (they animate, so they dirty the
    /// frame every time their angle changes by a visible amount).
    pub fn set_falling(&mut self, trees: Vec<(i32, i32, f32, i8, u8, u8)>) -> bool {
        let changed = trees.len() != self.falling.len()
            || trees
                .iter()
                .zip(self.falling.iter())
                .any(|(a, b)| a.0 != b.0 || a.1 != b.1 || (a.2 - b.2).abs() > 0.01);
        if !changed {
            return false;
        }
        self.falling = trees;
        true
    }

    pub fn set_camp(&mut self, camp: Option<(i32, i32, Vec<(i32, i32)>, Option<(i32, i32)>)>) -> bool {
        if self.camp != camp {
            self.camp = camp;
            return true;
        }
        false
    }

    pub fn set_logs(&mut self, logs: Vec<(i32, i32, u8)>) -> bool {
        if self.logs != logs {
            self.logs = logs;
            return true;
        }
        false
    }

    pub fn holiday(&self) -> crate::holiday::HolidayLook {
        self.holiday
    }

    /// B53: the gameday host on screen, as `(cx, cy, span, area)`: its centre,
    /// its width, and the box the win fireworks may paint into.
    pub fn gameday_host_screen(&self) -> Option<(f32, f32, f32, Rect)> {
        let o = &self.city[gameday_host(&self.city)?];
        let z = o.level as i32 * crate::iso::HEIGHT_UNIT;
        let (cx, cy) = self
            .cam
            .project(o.x as f32 + o.w as f32 * 0.5, o.y as f32 + o.h as f32 * 0.5, z);
        let (x0, _) = self.cam.project(o.x as f32, o.y as f32, z);
        let (x1, _) = self.cam.project((o.x + o.w) as f32, o.y as f32, z);
        let span = (x1 - x0).abs().max(40.0);
        let area = Rect::new(
            (cx - 2.2 * span) as i32,
            (cy - 1.6 * span) as i32,
            (4.4 * span) as i32,
            (2.6 * span) as i32,
        );
        Some((cx, cy, span, area))
    }

    /// Advance the fireworks display (B36): at 8 fps, over the top third of the
    /// frame, so a holiday night costs a few small composites a second instead
    /// of a full frame at 30.
    pub fn animate_fireworks(&mut self, t: f32) -> bool {
        // B53: the win fireworks repaint only their own area over the host.
        if self.holiday.holiday == crate::holiday::Holiday::BillsWin {
            if self.glow < 0.45 || (t - self.holiday_t).abs() < 0.125 {
                return false;
            }
            self.holiday_t = t;
            return match self.gameday_show() {
                Some(show) => {
                    let cycle = t.rem_euclid(3600.0);
                    if cycle < 900.0 {
                        for r in crate::spectacle::show_rects(&show, cycle, 900.0, self.seed as u32 ^ 0xB11) {
                            self.mark_rect(r);
                        }
                    }
                    true
                }
                None => false,
            };
        }
        if !self.holiday.holiday.is_night_fireworks() || self.glow < 0.45 {
            return false;
        }
        if (t - self.holiday_t).abs() < 0.125 {
            return false;
        }
        self.holiday_t = t;
        let band = Rect::new(0, 0, self.width() as i32, (self.height() as i32 / 3).max(1));
        self.mark_rect(band);
        true
    }

    /// Mark the whole frame as changed: a light/season/sky update, a layout
    /// change, or the first frame (B30).
    #[track_caller]
    pub fn mark_full(&mut self) {
        // B67: with CITYLIVE_MOTION_TRACE set, say who asked (`FULL file:line`), so
        // a full-frame composite can be traced to its cause.
        static TRACE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        if *TRACE.get_or_init(|| std::env::var("CITYLIVE_MOTION_TRACE").is_ok()) {
            eprintln!("FULL {}", std::panic::Location::caller());
        }
        self.full_pending = true;
        self.pending.clear();
    }

    /// Add a framebuffer-space rectangle to the next composite (B30).
    pub fn mark_rect(&mut self, r: Rect) {
        if !r.is_empty() {
            self.pending.push(r);
        }
    }

    /// Take the rectangles the next composite has to redraw. `None` means the
    /// whole frame; `Some(vec![])` means nothing changed at all.
    pub fn take_pending(&mut self) -> Option<Vec<Rect>> {
        if self.full_pending {
            self.full_pending = false;
            self.pending.clear();
            self.tree_pending.clear();
            self.water_pending.clear();
            return None;
        }
        // B69b: the trees' small boxes go on a fine grid of their own.
        let tree_rects = std::mem::take(&mut self.tree_pending);
        // B87: the strokes that moved, on a grid sized so a box costs more than
        // the pixels it adds (a composite pays ~1.4 us per rectangle).
        let water_rects = std::mem::take(&mut self.water_pending);
        // B101: the water's moved strokes are tiny boxes (a few dozen pixels
        // each, hundreds of them). They used to be rounded out to a 64x32 grid,
        // which turned 30 kpx of change into 1.6 Mpx of composite; now they go
        // through the same cost-based merge as every other box (`merge_by_cost`).
        let water_spans = water_rects;
        let tree_spans = if tree_rects.is_empty() {
            Vec::new()
        } else {
            if tree_rects.len() <= 1500 {
                // A crown's moved rows are already a tight box: composite each
                // as it is (a fixed cost of well under a microsecond apiece).
                tree_rects
            } else {
                fine_spans(&tree_rects, Rect::from_size(self.width(), self.height()))
            }
        };
        let mut rects = std::mem::take(&mut self.pending);
        if rects.is_empty() {
            let mut v = tree_spans;
            v.extend(water_spans);
            return Some(merge_by_cost(v, self.rect_cost()));
        }
        // B31b: **no cascading merge**. `merge_rects` unions any two overlapping
        // boxes and iterates to a fixpoint, so a street of cars chains into one
        // long box, boxes at a junction chain again, and the damage ends up
        // covering the whole frame — which is why the full-frame fallback fired
        // every frame in H31. Instead: clip, drop duplicates, and merge only
        // *identical* boxes (a vehicle that did not move between two ticks).
        let frame = Rect::from_size(self.width(), self.height());
        let mut out: Vec<Rect> = Vec::with_capacity(rects.len());
        rects.sort_unstable_by_key(|r| (r.y0, r.x0, r.y1, r.x1));
        let mut last: Option<Rect> = None;
        for r in rects {
            let r = Rect::new(r.x0.max(0), r.y0.max(0), r.x1.min(frame.x1), r.y1.min(frame.y1));
            if r.is_empty() || Some(r) == last {
                continue;
            }
            last = Some(r);
            out.push(r);
        }
        if out.is_empty() {
            let mut v = tree_spans;
            v.extend(water_spans);
            return Some(merge_by_cost(v, self.rect_cost()));
        }
        // D71: hundreds of small boxes (a walking crowd, a street of cars) used
        // to blow the cap and send every frame down the full-frame path — which
        // is why smooth animation could not be afforded. Coalesce them onto a
        // coarse grid instead: each 64x32 cell that anything touched is
        // composited once, merged into row spans. Ten boxes' worth of extra
        // pixels buys one pass over the layers instead of four hundred.
        let (in_n, in_area) = (out.len(), out.iter().map(|r| r.area()).sum::<i64>());
        let (tree_n, tree_area) = (tree_spans.len(), tree_spans.iter().map(|r| r.area()).sum::<i64>());
        let (water_n, water_area) = (water_spans.len(), water_spans.iter().map(|r| r.area()).sum::<i64>());
        // The pending boxes (movers, lamps, signs, static repaints) go through the
        // grid, as they always have: its rounding is also the margin that the
        // producers' tight boxes rely on (a mover's lamps, a shadow's tip, a
        // sprite's overhang), and rectangles taken exactly leave a few thousand
        // stale pixels in a big city. The water's and the trees' boxes are exact
        // by construction (the strokes' own bounds) and numerous, so they are
        // merged by what a rectangle costs.
        let mut spans = coalesce(&out, frame);
        let mut fine = tree_spans;
        fine.extend(water_spans);
        spans.extend(merge_by_cost(fine, self.rect_cost()));
        let area: i64 = spans
            .iter()
            .map(|r| (r.x1 - r.x0).max(0) as i64 * (r.y1 - r.y0).max(0) as i64)
            .sum();
        if std::env::var("PP_DEBUG").is_ok() {
            eprintln!(
                "pending: {} rects ({} px) + trees {} ({} px) + water {} ({} px) -> {} boxes, {:.1} % area",
                in_n,
                in_area,
                tree_n,
                tree_area,
                water_n,
                water_area,
                spans.len(),
                area as f32 * 100.0 / (frame.x1 as f32 * frame.y1 as f32)
            );
        }
        if areas_too_big(&spans, frame, self.rect_cost()) {
            if std::env::var("PP_DEBUG").is_ok() {
                eprintln!("pp: FULL because {} spans / {:.1} % area", spans.len(), area as f32 * 100.0 / (frame.x1 as f32 * frame.y1 as f32));
            }
            return None;
        }
        Some(spans)
    }

    /// B101: what one more rectangle costs a composite, in pixels of area: the
    /// fixed work every `composite_rect` repeats (the per-layer culls, the band
    /// walks, the sprite lists) measured at about 2.6 kpx on the desktop save.
    fn rect_cost(&self) -> i64 {
        static C: std::sync::OnceLock<i64> = std::sync::OnceLock::new();
        *C.get_or_init(|| std::env::var("CITYLIVE_RECT_COST").ok().and_then(|v| v.parse().ok()).unwrap_or(RECT_COST_PX))
    }

    /// A point's screen position on the ground, for a cull test (B61d).
    fn screen_of(&self, x: f32, y: f32) -> (i32, i32) {
        let z = self.map.ground_px(x, y);
        let (sx, sy) = self.cam.project(x, y, z);
        (sx as i32, sy as i32)
    }

    /// The screen rectangle a tile-space point occupies, padded for a sprite of
    /// `pad` pixels above and below (B30).
    pub fn screen_rect(&self, x: f32, y: f32, pad: i32) -> Rect {
        let z = self.map.ground_px(x, y);
        let (sx, sy) = self.cam.project(x, y, z);
        Rect::new(
            sx as i32 - pad,
            sy as i32 - pad * 2,
            sx as i32 + pad,
            sy as i32 + pad,
        )
    }

    /// How lit the scene is, 0..1 (headlights come on below ~0.45).
    pub fn glow(&self) -> f32 {
        self.glow
    }

    /// B89: where the sun (or, at night, the moon) glitters on the sea and how
    /// strongly. The path lies under the body's screen column (the sky band's
    /// own mapping: azimuth over the full width), grows with its height, is
    /// warm and low at sunset, and is flattened by cloud.
    fn sync_water_glint(&mut self) {
        let w = self.width() as f32;
        let (sun, moon) = (self.light.sun, self.light.moon);
        let cloud = self.water.cloud;
        let (x, strength, col) = if sun.alt > -1.0 {
            // Orange near the horizon, near-white high up.
            let low = (1.0 - (sun.alt - 1.0) / 20.0).clamp(0.0, 1.0);
            let c = palette::mix(palette::rgb(0xff, 0xf8, 0xe4), palette::rgb(0xff, 0xa8, 0x50), low);
            let g = ((sun.alt + 1.0) / 3.0).clamp(0.0, 1.0) * (1.0 - cloud).powf(1.6);
            (sun.az.rem_euclid(360.0) / 360.0 * w, g, c)
        } else if moon.alt > 2.0 {
            let g = (1.0 * self.light.moon_illum.clamp(0.0, 1.0) * (1.0 - cloud).powf(1.6)).min(0.9);
            (moon.az.rem_euclid(360.0) / 360.0 * w, g, palette::rgb(0xd6, 0xe4, 0xff))
        } else {
            (0.0, 0.0, self.water.glint_c)
        };
        let alt = sun.alt;
        let dusk = if alt >= 0.0 { ((14.0 - alt) / 14.0).clamp(0.0, 1.0) } else { (1.0 + alt / 6.0).clamp(0.0, 1.0) };
        self.water.dusk = (dusk * 8.0).round() / 8.0;
        // B92: when the body's own column is under land (the island is the
        // middle of the frame), the path slides to the nearest column with
        // open sea in it, a little weaker, instead of vanishing.
        let (mut x, mut strength) = (x, strength);
        if strength > 0.0 && !self.sea_cols.is_empty() {
            let cols = self.sea_cols.len();
            let reach = |b: usize| -> u32 {
                (b.saturating_sub(3)..(b + 4).min(cols)).map(|i| self.sea_cols[i] as u32).sum()
            };
            let peak = (0..cols).map(reach).max().unwrap_or(0);
            let here = ((x / 64.0) as usize).min(cols - 1);
            if peak > 0 && reach(here) * 10 < peak * 4 {
                let mut best = here;
                for d in 1..cols {
                    let (lo, hi) = (here.saturating_sub(d), (here + d).min(cols - 1));
                    if reach(lo) * 10 >= peak * 6 {
                        best = lo;
                        break;
                    }
                    if reach(hi) * 10 >= peak * 6 {
                        best = hi;
                        break;
                    }
                }
                x = best as f32 * 64.0 + 32.0;
                strength *= 0.8;
            }
        }
        // Quantised: a slow sun must not change every stroke every tick.
        self.water.glint_x = (x / 32.0).round() * 32.0;
        self.water.glint = (strength * 6.0).round() / 6.0;
        self.water.glint_c = col;
    }

    /// How lit the scene is, for the water highlights (B21 item 4).
    pub fn set_water_brightness(&mut self, b: f32) {
        if self.water.brightness != b {
            self.water_prims.clear();
        }
        self.water.brightness = b;
        // B64: the night look of the water is baked into the tiles; call this
        // *before* `set_water_tint`, which re-rasterises when the step moved.
        self.water.night_q = terrain::WaterLook::night_steps(b);
    }

    /// Turn the water animation on or off (`--water-anim off`).
    pub fn set_water_anim(&mut self, on: bool) {
        self.water.anim = on;
        self.water_prims.clear();
    }

    /// Advance the water animation to `phase` (in old frames, 6 per second).
    /// Returns the number of on-screen water tiles that changed, so the caller
    /// only composites when there is something to show.
    pub fn animate_water(&mut self, phase: f32) -> usize {
        if !self.water.anim {
            return 0;
        }
        // Nothing to animate when the overlay would be invisible: deep night
        // (the highlights have faded to the water colour) or no water on screen
        // (H21 bug: the wallpaper was ticking for a frame that never changed).
        if (self.water.brightness < 0.08 && self.water.glint < 0.05) || self.visible_water() == 0 {
            return 0;
        }
        let phase = phase.rem_euclid(terrain::WATER_PHASE);
        // The highlight moves ~1 px per 0.1 of a phase unit; below that the
        // frame would be identical, so the dirty work is skipped.
        if (phase - self.water.phase).abs() < 0.05 {
            return 0;
        }
        self.water.phase = phase;
        let strong = (self.haze_depth() * 0.142) as i32;
        if self.water_prims.len() == self.water_px.len()
            && !self.water_px.is_empty()
            && self.water_haze == strong
        {
            return self.animate_water_strokes();
        }
        self.animate_water_all()
    }

    /// B87: the ripple strokes of tile `i` at the current look, or none where
    /// the compositor does not draw it (hazed rows, off the frame).
    fn water_strokes(&self, i: usize, haze_bottom: i32, strong: bool) -> terrain::TilePrims {
        let mut out = terrain::TilePrims::default();
        self.water_strokes_into(i, haze_bottom, strong, &mut out);
        out
    }

    /// [`Self::water_strokes`] into a buffer the caller reuses (B101: a tick
    /// asks for seventeen thousand tiles; a fresh 500-byte value each was a
    /// good part of its cost).
    fn water_strokes_into(&self, i: usize, haze_bottom: i32, strong: bool, out: &mut terrain::TilePrims) {
        out.clear();
        let (x, y, _xs, ys) = self.water_px[i];
        if strong && ys < haze_bottom {
            return;
        }
        terrain::water_overlay_c(out, &self.cam, &self.map, &self.water, x, y, self.water_coast.get(i).copied(), self.water_ground.get(i).copied());
    }

    /// B87: the water tick for a look the screen already shows: paint nothing,
    /// compare each tile's strokes with the ones last drawn, and damage the
    /// exact boxes of the strokes that moved, appeared or vanished. A lake's
    /// shimmer changes one frame in five, a sea's wave a few pixels at a time:
    /// the rest of the water is already right and is not recomposited.
    fn animate_water_strokes(&mut self) -> usize {
        let strong = (self.haze_depth() * 0.142) as i32;
        let haze_bottom = self.sky_h as i32 + strong;
        let mut boxes = std::mem::take(&mut self.water_pending);
        let mut prims = std::mem::take(&mut self.water_prims);
        let frame = Rect::from_size(self.width(), self.height());
        let land = self.land_clip();
        let mut changed = 0usize;
        let mut new = terrain::TilePrims::default();
        for i in 0..prims.len() {
            self.water_strokes_into(i, haze_bottom, strong > 0, &mut new);
            let old = &prims[i];
            if *old == new {
                continue;
            }
            changed += 1;
            // One box per tile: the old and new places of every stroke that
            // is not the same as before (they are a pixel or two apart).
            let mut u = Rect::new(i32::MAX, i32::MAX, i32::MIN, i32::MIN);
            for p in old.iter().filter(|p| !new.contains(p)).chain(new.iter().filter(|p| !old.contains(p))) {
                let b = p.bounds();
                u = Rect::new(u.x0.min(b.x0), u.y0.min(b.y0), u.x1.max(b.x1), u.y1.max(b.y1));
            }
            let r = u.intersect(&frame).intersect(&land);
            if !r.is_empty() {
                boxes.push(r);
            }
            prims[i] = new;
        }
        self.water_prims = prims;
        self.water_pending = boxes;
        changed
    }

    /// The water tick before the strokes are known (the first one, or after the
    /// look moved): damage every visible tile's run, and remember the strokes.
    fn animate_water_all(&mut self) -> usize {
        // B30/B31b: the water tiles are dirty this frame. B61: one rectangle
        // per **run of adjacent water tiles in a row**, not per tile and not per
        // 4x4 block. A block's bounding box is a 256x152 screen box for 16 tiles
        // that are really 16 diamonds, so a lake's damage was 4-5x its own
        // pixels; a row run is a tight band, and a lake still costs a handful of
        // rectangles rather than a few hundred.
        // D71: walk the water list, not every item; B61: the scratch list is
        // reused, so a per-frame water tick allocates nothing.
        let mut tiles = std::mem::take(&mut self.water_scratch);
        tiles.clear();
        for &(x, y) in self.water_tiles.iter() {
            if self.is_visible(x, y) {
                tiles.push((x, y));
            }
        }
        let n = tiles.len();
        tiles.sort_unstable();
        tiles.dedup();
        let mut i = 0usize;
        while i < tiles.len() {
            let (x0, y) = tiles[i];
            let mut x1 = x0;
            while i + 1 < tiles.len() && tiles[i + 1].1 == y && tiles[i + 1].0 == x1 + 1 {
                i += 1;
                x1 = tiles[i].0;
            }
            // The run's screen box, from the four corners of the tile span with
            // a tile of slack for the bank lines and the ripple overshoot.
            let (mut ax, mut ay) = (i32::MAX, i32::MAX);
            let (mut bx2, mut by2) = (i32::MIN, i32::MIN);
            for (cx, cy) in [
                (x0 - 1, y - 1),
                (x1 + 2, y - 1),
                (x0 - 1, y + 2),
                (x1 + 2, y + 2),
            ] {
                let p = self.cam.project(cx as f32, cy as f32, 0);
                ax = ax.min(p.0 as i32);
                ay = ay.min(p.1 as i32);
                bx2 = bx2.max(p.0 as i32);
                by2 = by2.max(p.1 as i32);
            }
            self.mark_rect(Rect::new(ax, ay - 12, bx2, by2 + 12));
            i += 1;
        }
        self.water_scratch = tiles;
        // Remember what the overlay now shows, tile by tile, for the next tick.
        let strong = (self.haze_depth() * 0.142) as i32;
        let haze_bottom = self.sky_h as i32 + strong;
        let mut prims = std::mem::take(&mut self.water_prims);
        prims.clear();
        for i in 0..self.water_px.len() {
            prims.push(self.water_strokes(i, haze_bottom, strong > 0));
        }
        self.water_prims = prims;
        self.water_haze = strong;
        n
    }

    /// How many on-screen tiles are water (for the animation cost report).
    /// B61: walks the water list (a few hundred tiles), not every item in the
    /// viewport with a `map.at` each (tens of thousands).
    pub fn visible_water(&self) -> usize {
        self.water_tiles
            .iter()
            .filter(|&&(x, y)| self.is_visible(x, y))
            .count()
    }

    /// Smallest level drawn at a pixel, for tests.
    pub fn level_px(&self, level: u8) -> i32 {
        level_px(level as f32)
    }
}

/// A re-rasterise of the static layer in progress (B76 r2): a diagonal (all x
/// bands) at a time, so no frame does more than a few milliseconds of it.
struct RebuildJob {
    /// Next painter diagonal, the last one in the frame, and the next x band
    /// inside it (a dense downtown diagonal is a few ms on its own, so the
    /// slice unit is one band of one diagonal).
    d: i32,
    last: i32,
    bx: i32,
    band_max: i32,
    kit_draws: usize,
    engine_draws: usize,
    drawn: usize,
    visited: usize,
    slices: u32,
    worst_ms: f64,
    /// B76 r2: the per-pass state, taken once when the pass starts. Cloning it
    /// every frame was a few milliseconds of the frame budget for nothing.
    fixtures: crate::rail::Fixtures,
    /// B83: the plot map (pavement vs verge), taken with the rest of the pass.
    city_key: Vec<u16>,
    /// B74: draw-time cleaning claims and derived trees, taken with the pass.
    claims: Vec<u8>,
    derived: Vec<Option<map::Prop>>,
    street: StreetKey,
    tech: crate::tech::TechKey,
    civic: crate::civic::CivicStatic,
    dress: crate::spectacle::Dress,
    fest_host: Option<usize>,
    land_clip: Rect,
}

/// A re-tint of the static layer in progress (B67).
struct RetintJob {
    lut: Box<crate::light::Lut>,
    bulbs: Vec<Color>,
    haze: Vec<f32>,
    air: Vec<Color>,
    /// Next row to do.
    y: usize,
}

/// A shadow bake in progress (B67).
struct ShadowJob {
    caster: crate::light::Caster,
    mask: Vec<u8>,
    /// B71: shadow ceilings for walls and roofs.
    ceil: crate::shadow::Ceil,
    /// Next pitched roof whose facets are still to be shaded.
    facet: usize,
    plane: Vec<u8>,
    /// Next mask row and next screen row to bake.
    j: usize,
    y: usize,
    strength: f32,
    mult: [f32; 3],
    spent_ms: f64,
    /// Slices run and the dearest one (CITYLIVE_MOTION_TRACE only).
    slices: u32,
    worst_ms: f64,
}

/// Damage cells (D71). Every rectangle that changed is rounded out to this
/// grid and the occupied cells are merged back into row spans, so a hundred
/// walking figures cost a handful of composites rather than a hundred — and
/// never the full frame.
/// The coalescer's cell, in pixels. B61d: the cell is scaled to the frame,
/// because the cost it trades is fixed-per-span (the per-layer culls) against
/// per-pixel: a tall band merges a crowd's or a downpour's damage vertically
/// into one rectangle, which is a big win at 7040x1440 and far too coarse on a
/// 520x320 test frame (where five bands cover the whole sky). 64x32 at the test
/// frame's size, 64x60 at the owner's union.
/// B69b: coalesce the sway layer's boxes onto a fine grid (16x8 px), merged
/// into row spans. Finer than `coalesce` because a crown's moved rows are a thin
/// band: a 64x60 cell per tree was ten times the pixels that changed.
/// B87: the grid the water's moved strokes are merged onto.
const WATER_CELL: (i32, i32) = (64, 32);

/// How far (px) a moving thing's shadow can reach from it (the shadow pass culls
/// with this, and the damage boxes grow by it).
const SHADOW_REACH: i32 = 56;

fn fine_spans(rects: &[Rect], frame: Rect) -> Vec<Rect> {
    grid_spans(rects, frame, 16, 6)
}

/// `fine_spans` on a grid of `CW` x `CH` cells.
fn grid_spans(rects: &[Rect], frame: Rect, cw: i32, ch: i32) -> Vec<Rect> {
    let cols = ((frame.x1 - frame.x0) + cw - 1) / cw;
    let rows = ((frame.y1 - frame.y0) + ch - 1) / ch;
    if cols <= 0 || rows <= 0 {
        return Vec::new();
    }
    let mut bits = vec![false; (cols * rows) as usize];
    for r in rects {
        let r = r.intersect(&frame);
        if r.is_empty() {
            continue;
        }
        let (c0, c1) = ((r.x0 - frame.x0) / cw, (r.x1 - 1 - frame.x0) / cw);
        let (r0, r1) = ((r.y0 - frame.y0) / ch, (r.y1 - 1 - frame.y0) / ch);
        for y in r0..=r1 {
            let row = (y * cols) as usize;
            bits[row + c0 as usize..=row + c1 as usize].fill(true);
        }
    }
    let mut out = Vec::new();
    for y in 0..rows {
        let row = (y * cols) as usize;
        let mut x = 0;
        while x < cols {
            if !bits[row + x as usize] {
                x += 1;
                continue;
            }
            let start = x;
            while x < cols && bits[row + x as usize] {
                x += 1;
            }
            out.push(Rect::new(
                frame.x0 + start * cw,
                frame.y0 + y * ch,
                (frame.x0 + x * cw).min(frame.x1),
                (frame.y0 + (y + 1) * ch).min(frame.y1),
            ));
        }
    }
    out
}

fn cell_w(frame: Rect) -> i32 {
    ((frame.x1 - frame.x0) / 110).clamp(48, 128)
}

fn cell_h(frame: Rect) -> i32 {
    ((frame.y1 - frame.y0) / 24).clamp(32, 96)
}

fn coalesce(rects: &[Rect], frame: Rect) -> Vec<Rect> {
    let (cw, ch) = (cell_w(frame), cell_h(frame));
    let cols = (((frame.x1 - frame.x0) + cw - 1) / cw).max(1) as usize;
    let rows = (((frame.y1 - frame.y0) + ch - 1) / ch).max(1) as usize;
    let mut bits = vec![false; cols * rows];
    for r in rects {
        if r.is_empty() {
            continue;
        }
        let c0 = ((r.x0 - frame.x0) / cw).clamp(0, cols as i32 - 1) as usize;
        let c1 = ((r.x1 - 1 - frame.x0) / cw).clamp(0, cols as i32 - 1) as usize;
        let r0 = ((r.y0 - frame.y0) / ch).clamp(0, rows as i32 - 1) as usize;
        let r1 = ((r.y1 - 1 - frame.y0) / ch).clamp(0, rows as i32 - 1) as usize;
        for y in r0..=r1 {
            let row = y * cols;
            for c in bits[row + c0..=row + c1].iter_mut() {
                *c = true;
            }
        }
    }
    // B84 r2: a run of touched cells used to be one rectangle per row band —
    // a *full-width* span, because the run was taken to the frame's edge. A
    // scattered rush hour (a hundred figures and cars over the whole city)
    // therefore damaged 29 % of the frame for 3 % of its pixels. Instead each
    // run is extended downwards for as long as the same cells stay touched, so
    // a figure's box is one small rectangle and a street of them is one per
    // row band, not one across the width.
    let mut out = Vec::with_capacity(96);
    for y in 0..rows {
        let mut x = 0;
        while x < cols {
            if !bits[y * cols + x] {
                x += 1;
                continue;
            }
            let start = x;
            while x < cols && bits[y * cols + x] {
                x += 1;
            }
            let (c0, c1) = (start, x);
            let mut y1 = y + 1;
            while y1 < rows {
                let row = y1 * cols;
                if (c0..c1).all(|c| bits[row + c]) {
                    y1 += 1;
                } else {
                    break;
                }
            }
            for yy in y..y1 {
                let row = yy * cols;
                for c in c0..c1 {
                    bits[row + c] = false;
                }
            }
            out.push(Rect::new(
                frame.x0 + c0 as i32 * cw,
                frame.y0 + y as i32 * ch,
                (frame.x0 + c1 as i32 * cw).min(frame.x1),
                (frame.y0 + y1 as i32 * ch).min(frame.y1),
            ));
        }
    }
    // Many scattered cells (a lake's animated tiles, a crowd) can give more
    // small boxes than the composite wants to pay the per-rectangle cost for.
    // Past the cap, take the row spans instead: fewer rectangles, more area,
    // and never the full-frame fallback.
    if out.len() > MAX_BOXES {
        return row_spans(&bits_reset(rects, frame, cols, rows, cw, ch), cols, rows, cw, ch, frame);
    }
    out
}

/// The box cap (B84 r2): beyond this the per-rectangle cost outweighs the area
/// the small boxes save.
const MAX_BOXES: usize = 192;

/// The same cell map the coalescer built, rebuilt from the rectangles (the
/// boxes pass consumes it).
fn bits_reset(rects: &[Rect], frame: Rect, cols: usize, rows: usize, cw: i32, ch: i32) -> Vec<bool> {
    let mut bits = vec![false; cols * rows];
    for r in rects {
        if r.is_empty() {
            continue;
        }
        let c0 = ((r.x0 - frame.x0) / cw).clamp(0, cols as i32 - 1) as usize;
        let c1 = ((r.x1 - 1 - frame.x0) / cw).clamp(0, cols as i32 - 1) as usize;
        let r0 = ((r.y0 - frame.y0) / ch).clamp(0, rows as i32 - 1) as usize;
        let r1 = ((r.y1 - 1 - frame.y0) / ch).clamp(0, rows as i32 - 1) as usize;
        for y in r0..=r1 {
            let row = y * cols;
            for c in bits[row + c0..=row + c1].iter_mut() {
                *c = true;
            }
        }
    }
    bits
}

/// One rectangle per touched run of cells, widened to the frame's edges — the
/// B61 behaviour, kept for the case where the boxes would be too many.
fn row_spans(bits: &[bool], cols: usize, rows: usize, cw: i32, ch: i32, frame: Rect) -> Vec<Rect> {
    let mut out = Vec::with_capacity(64);
    for y in 0..rows {
        let row = y * cols;
        let mut x = 0;
        while x < cols {
            if !bits[row + x] {
                x += 1;
                continue;
            }
            let start = x;
            while x < cols && bits[row + x] {
                x += 1;
            }
            out.push(Rect::new(
                frame.x0 + start as i32 * cw,
                frame.y0 + y as i32 * ch,
                (frame.x0 + x as i32 * cw).min(frame.x1),
                (frame.y0 + (y as i32 + 1) * ch).min(frame.y1),
            ));
        }
    }
    out
}

/// Is one full-frame composite cheaper than this list? (D71: the answer is
/// "no" for the ordinary animated frame, which is the whole point.)
fn areas_too_big(spans: &[Rect], frame: Rect, cost: i64) -> bool {
    // B101: judged by what the composite would pay (area plus a fixed cost per
    // rectangle), against what the whole frame costs: past 80 % of it a full
    // composite is the cheaper and simpler frame.
    let area: i64 = spans
        .iter()
        .map(|r| (r.x1 - r.x0).max(0) as i64 * (r.y1 - r.y0).max(0) as i64)
        .sum();
    let all = frame.x1 as i64 * frame.y1 as i64;
    (area + spans.len() as i64 * cost) * 10 > all * 8
}

/// B101: the fixed cost of one damage rectangle, in pixels (see `rect_cost`).
pub const RECT_COST_PX: i64 = 1000;

/// B101: merge damage boxes by what the composite will pay for them. A box
/// costs `cost + area` pixels; two boxes become their union when that is no
/// dearer than the pair. Sparse boxes stay apart (a union of two far ones is
/// all empty space, so nothing chains into a frame-wide box), and a dense
/// cluster (hundreds of stroke boxes in a lake, a street of cars) collapses to
/// a few rectangles instead of paying the fixed cost hundreds of times.
///
/// Boxes are binned into 256 px columns by their centre and swept top to
/// bottom against the next few in the column, twice (a merged box can reach a
/// neighbour it could not before).
pub fn merge_by_cost(mut rects: Vec<Rect>, cost: i64) -> Vec<Rect> {
    if rects.len() < 2 {
        return rects;
    }
    const COL: i32 = 256;
    const WINDOW: usize = 24;
    let cols = rects.iter().map(|r| ((r.x0 + r.x1) / 2).max(0) / COL).max().unwrap_or(0) as usize + 1;
    let mut bins: Vec<Vec<Rect>> = vec![Vec::new(); cols];
    for r in rects.drain(..) {
        bins[(((r.x0 + r.x1) / 2).max(0) / COL) as usize].push(r);
    }
    let area = |r: &Rect| (r.x1 - r.x0).max(0) as i64 * (r.y1 - r.y0).max(0) as i64;
    let mut out = Vec::new();
    for mut bin in bins {
        for _pass in 0..2 {
            if bin.len() < 2 {
                break;
            }
            bin.sort_unstable_by_key(|r| (r.y0, r.x0));
            let mut merged_any = false;
            let mut i = 0;
            while i < bin.len() {
                let mut j = i + 1;
                while j < bin.len() && j <= i + WINDOW {
                    let (a, b) = (bin[i], bin[j]);
                    let u = a.union(&b);
                    if area(&u) <= area(&a) + area(&b) + cost {
                        bin[i] = u;
                        bin.swap_remove(j);
                        merged_any = true;
                        // `swap_remove` moved the last box into `j`: look at it
                        // again, the window is still in order enough.
                        continue;
                    }
                    j += 1;
                }
                i += 1;
            }
            if !merged_any {
                break;
            }
        }
        out.extend(bin);
    }
    out
}

/// The active caster's ground direction, for the "has the light moved enough to
/// re-bake?" test. `None` when nothing is casting.
fn caster_key(ls: &LightState) -> Option<(f32, f32, f32)> {
    ls.caster().map(|c| (c.toward.0, c.toward.1, c.alt))
}

/// One static-layer pixel through the ambient light (B45c). Solid features
/// (park lots, plazas, walls, roofs) are tinted like the ground: B42b only
/// meant them to skip the distance haze, but the old branch also skipped the
/// light, so parks and plazas stayed noon-bright at night. Alpha-0xFE kit
/// lights and lit windows stay emissive; the solid tag is dropped.
///
/// B58b item 7: a pure multiply drove every material to the *same* blue at
/// night, so a brick terrace, a concrete tower and a wooden barn came out as one
/// navy mass. A fifth of the material's own colour is kept on top of the
/// moonlight, which is how a real night keeps its materials: the sky tints
/// everything, but brick still reads red-brown and slate still reads grey.
pub(crate) fn night_pixel(s: Color, lut: &crate::light::Lut, bulbs: &[Color]) -> Color {
    if s == TRANSPARENT {
        return TRANSPARENT;
    }
    let a = crate::palette::alpha(s);
    let opaque = crate::palette::with_alpha(s, 0xFF);
    if a == 0xFE || opaque == pal::WINDOW_LIT || opaque == pal::LAMP_GLOW || bulbs.contains(&opaque) {
        return opaque;
    }
    let lit = crate::light::apply(opaque, lut);
    // How much of the material survives the moonlight: a little, and less when
    // the night is very dark (so a moonless night still reads as night).
    let keep = 0.10 + 0.16 * (1.0 - crate::palette::r(lit) as f32 / 120.0).clamp(0.0, 1.0);
    let mut out = crate::palette::mix(lit, opaque, keep);
    // B99d: a warm hour must not repaint the snow. A bright, nearly neutral
    // surface (snow, ice, white paint, concrete) is lit by this hour's light —
    // it dims with it — but it does not take its colour *cast*. The flat
    // multiply turned a tundra's snow peach at 16:30 and every white wall cream.
    // Such a surface is pulled back towards a neutral version of itself (its own
    // colour at the hour's luminance), in proportion to how warm the hour is: no
    // change at noon, a continuous fade through dusk, and coloured walls, brick
    // and grass keep their cast.
    let warm = (lut[0][255] as f32 - lut[2][255] as f32) / 255.0;
    if warm > 0.04 {
        let (r, g, b) = (
            crate::palette::r(opaque) as f32,
            crate::palette::g(opaque) as f32,
            crate::palette::b(opaque) as f32,
        );
        let sat = (r.max(g).max(b) - r.min(g).min(b)) / 255.0;
        let lum0 = (0.30 * r + 0.59 * g + 0.11 * b) / 255.0;
        if sat < 0.20 && lum0 > 0.62 {
            let lum_t = (0.2126 * lut[0][255] as f32
                + 0.7152 * lut[1][255] as f32
                + 0.0722 * lut[2][255] as f32)
                / 255.0;
            let neut = crate::palette::rgb(
                (r * lum_t).min(255.0) as u8,
                (g * lum_t).min(255.0) as u8,
                (b * lum_t).min(255.0) as u8,
            );
            let f = ((warm - 0.04) / 0.26).clamp(0.0, 1.0);
            out = crate::palette::mix(out, neut, f * 0.85);
        }
    }
    // B64: a pale surface (concrete lot, plaza paving, light wall) kept its
    // albedo advantage under the moon and glowed lavender next to the dark
    // ground. At night, bright materials give back up to half their
    // brightness; by day (`dark` = 0) nothing changes, and the ramp follows
    // the ambient continuously so dusk and dawn cannot pop.
    let dark = ((0.6 - lut[1][255] as f32 / 255.0) / 0.4).clamp(0.0, 1.0);
    if dark > 0.0 {
        let lum = (0.30 * crate::palette::r(opaque) as f32
            + 0.59 * crate::palette::g(opaque) as f32
            + 0.11 * crate::palette::b(opaque) as f32)
            / 255.0;
        let pale = ((lum - 0.39) / 0.39).clamp(0.0, 1.0);
        let mut f = 1.0 - 0.5 * dark * pale;
        // A painted fountain or park pond (saturated cyan-blue, drawn into the
        // ground like a pavement) is still water: it goes dark like the lakes,
        // not a lit cyan disc on a dark lawn.
        let (pr, pg, pb) = (
            crate::palette::r(opaque) as i32,
            crate::palette::g(opaque) as i32,
            crate::palette::b(opaque) as i32,
        );
        let water_like = pb - pr > 60 && pg - pr > 25 && pb > 120;
        if water_like {
            f *= 1.0 - 0.45 * dark;
        }
        out = crate::palette::shade(out, f);
        if water_like && dark > 0.0 {
            // B99d: and it stays *water*. The flat darkening left a park pond or
            // a fountain basin a black hole in the lawn; the sea keeps its dark
            // blue at night (see `water_px`), so a painted basin does too.
            let night_water = crate::palette::rgb(0x1a, 0x28, 0x48);
            out = crate::palette::mix(out, night_water, 0.62 * dark);
        }
    }
    crate::palette::with_alpha(out, 0xFF)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// B42d: the far-floor fill must never come out as 1-px vertical stripes.
    ///
    /// the owner's 1080-tall output came up as "one flat colour per column" from the
    /// horizon down. The fill used to lay the sky band's *raw* bottom row under
    /// every land row, so any uncovered patch became columns of the `Fields`
    /// treatment (wheat/plough/pasture). It now uses the 96-px blurred horizon
    /// colour: a striped sky row must come out smooth.
    #[test]
    fn far_floor_fill_cannot_stripe() {
        let (w, h) = (640usize, 360usize);
        let mut scene = Scene::new_with_sky(
            SceneConfig {
                width: w,
                height: h,
                seed: 7,
                mode: SceneMode::Sim,
                cars: 0,
                peds: 0,
                buildings: 0,
                rain: false,
            },
            0.30,
        );
        // A sky whose bottom row alternates four saturated colours: the worst
        // case a `Fields` horizon can hand us.
        let stripe = [
            palette::rgb(214, 186, 96),
            palette::rgb(150, 108, 62),
            palette::rgb(96, 134, 74),
            palette::rgb(196, 122, 58),
        ];
        let sky_h = scene.sky_height();
        let mut sky = Framebuffer::new(w, sky_h);
        for y in 0..sky_h {
            for x in 0..w {
                sky.px_mut()[y * w + x] = stripe[x % 4];
            }
        }
        scene.set_sky(sky);
        // No ground at all: the far floor is what the whole land area shows
        // (B69: and no swaying trees over it either).
        scene.set_tree_sway(false);
        scene.static_layer.clear(TRANSPARENT);
        scene.tinted_static.clear(TRANSPARENT);
        let mut fb = Framebuffer::new(w, h);
        scene.composite(&mut fb);

        // The land rows are now the blurred horizon colour: neighbouring columns
        // differ by at most a few units, instead of jumping between the stripes.
        let mut worst = 0i32;
        for y in (sky_h + 8)..h {
            for x in 1..w {
                let a = fb.px()[y * w + x];
                let b = fb.px()[y * w + x - 1];
                for shift in [16, 8, 0] {
                    let d = ((a >> shift) & 0xff) as i32 - ((b >> shift) & 0xff) as i32;
                    worst = worst.max(d.abs());
                }
            }
        }
        if worst > 8 {
            for x in 0..12 {
                eprintln!(
                    "x {x}: haze {:06x} fb {:06x} sky {:06x}",
                    scene.haze_row[x],
                    fb.px()[(sky_h + 8) * w + x],
                    scene.sky.px()[(sky_h - 1) * w + x]
                );
            }
        }
        assert!(
            worst <= 8,
            "uncovered land stripes by {worst} per column step (the raw sky row is back)"
        );
        // And the raw row really would have failed this: it alternates by >100.
        let mut raw = 0i32;
        for x in 1..w {
            let a = stripe[x % 4];
            let b = stripe[(x - 1) % 4];
            for shift in [16, 8, 0] {
                raw = raw.max((((a >> shift) & 0xff) as i32 - ((b >> shift) & 0xff) as i32).abs());
            }
        }
        assert!(raw >= 90, "the test's own stripes must be harsh ({raw})");
    }

    /// B26d item 1: a villager outside the framebuffer cannot change a pixel, so
    /// it must not be counted as "walkers live" (that count decided the 6 fps
    /// wake, and waking for invisible figures is what cost the owner 3 % of a core).
    #[test]
    fn offscreen_walkers_are_dropped() {
        let mut s = Scene::new(SceneConfig {
            width: 480,
            height: 270,
            seed: 7,
            mode: SceneMode::Sim,
            cars: 0,
            peds: 0,
            buildings: 0,
            rain: false,
        });
        // A tile inside the frame and one far outside it.
        let (mut inside, mut outside) = ((0, 0), (-50, -50));
        for y in 0..s.map.h {
            for x in 0..s.map.w {
                if s.is_visible(x, y) {
                    inside = (x, y);
                } else {
                    outside = (x, y);
                }
            }
        }
        s.set_walkers(vec![
            (inside.0 as f32 + 0.5, inside.1 as f32 + 0.5, 0),
            (outside.0 as f32 + 0.5, outside.1 as f32 + 0.5, 1),
        ]);
        assert_eq!(s.walker_count(), 1, "only the visible villager counts");
        assert_eq!(s.walkers_snapshot().len(), 1);
    }

    /// B87: a daylight scene with as much water on screen as 40 seeds offer.
    fn water_heavy_scene() -> Scene {
        let mut best: Option<(usize, Scene)> = None;
        for seed in 1..=40u64 {
            let mut scene = Scene::new(SceneConfig {
                width: 960,
                height: 540,
                seed,
                mode: SceneMode::Sim,
                cars: 0,
                peds: 0,
                buildings: 0,
                rain: false,
            });
            scene.set_light([1.0, 0.99, 0.98], 0.0);
            scene.set_water_brightness(1.0);
            scene.set_water_anim(true);
            if scene.has_dirty() {
                scene.flush_dirty();
            }
            let n = scene.visible_water();
            if best.as_ref().map_or(true, |(m, _)| n > *m) {
                best = Some((n, scene));
            }
        }
        best.unwrap().1
    }

    /// B87: the water tick damages the strokes that moved, not the water. On
    /// the most watery of 40 seeds, a 30 fps tick (0.2 of a phase) must damage
    /// well under half the frame (40 % worst, 30 % mean at this test's small size, where the 64x32 cells are coarse) (before B87 it was every water tile's whole
    /// row run).
    #[test]
    fn a_water_tick_damage_stays_under_a_pixel_budget() {
        let mut scene = water_heavy_scene();
        assert!(scene.visible_water() >= 30, "no watery seed");
        let mut fb = Framebuffer::new(960, 540);
        scene.composite(&mut fb);
        let _ = scene.take_pending();
        // The first tick after a look change damages every water tile (it
        // learns the strokes); the budget is for the ticks that follow.
        scene.animate_water(0.2);
        for r in scene.take_pending().expect("first tick") {
            scene.composite_rect(&mut fb, r);
        }
        let frame = 960i64 * 540;
        let mut worst = 0i64;
        let mut total = 0i64;
        let ticks = 60;
        for i in 1..=ticks {
            scene.animate_water(0.2 + i as f32 * 0.2);
            let rects = scene.take_pending().expect("water damage, never the full-frame fallback");
            let area: i64 = rects.iter().map(|r| r.area()).sum();
            worst = worst.max(area);
            total += area;
            for r in &rects {
                scene.composite_rect(&mut fb, *r);
            }
        }
        let mean = total as f64 / ticks as f64;
        assert!(
            worst * 100 / frame <= 40,
            "a water tick damaged {} % of the frame at worst",
            worst * 100 / frame
        );
        assert!(mean * 100.0 / (frame as f64) < 30.0, "mean damage {:.1} %", mean * 100.0 / frame as f64);
    }

    /// B87: the animated frame is the full recomposite of the same phase, pixel
    /// for pixel: no stale ripple, no trail. Four seeds, three seconds of 30 fps
    /// ticks across the wrap of the phase, plus a dusk step that moves the
    /// ripples' colour.
    #[test]
    fn an_animated_water_frame_equals_a_full_recomposite() {
        for seed in [3u64, 7, 12, 21] {
            let mut scene = Scene::new(SceneConfig {
                width: 960,
                height: 540,
                seed,
                mode: SceneMode::Sim,
                cars: 0,
                peds: 0,
                buildings: 0,
                rain: false,
            });
            scene.set_light([1.0, 0.99, 0.98], 0.0);
            scene.set_water_brightness(1.0);
            scene.set_water_anim(true);
            if scene.has_dirty() {
                scene.flush_dirty();
            }
            let mut inc = Framebuffer::new(960, 540);
            let mut full = Framebuffer::new(960, 540);
            scene.composite(&mut inc);
            let _ = scene.take_pending();
            for i in 1..=90 {
                if i == 60 {
                    scene.set_water_brightness(0.7);
                }
                scene.animate_water(i as f32 * 0.2);
                match scene.take_pending() {
                    None => scene.composite(&mut inc),
                    Some(rects) => {
                        for r in rects {
                            scene.composite_rect(&mut inc, r);
                        }
                    }
                }
                scene.composite(&mut full);
                let bad = inc.px().iter().zip(full.px()).filter(|(a, b)| a != b).count();
                assert_eq!(bad, 0, "seed {seed}, tick {i}: {bad} pixels differ from a full recomposite");
            }
        }
    }

    /// The wallpaper's idle rule (H23/B26): at night, with no villagers and with
    /// the water highlight faded out, 60 s of ticks must queue **no** composite.
    #[test]
    fn idle_night_scene_never_composites() {
        let mut scene = Scene::new(SceneConfig {
            width: 480,
            height: 270,
            seed: 7,
            mode: SceneMode::Sim,
            cars: 0,
            peds: 0,
            buildings: 0,
            rain: false,
        });
        // Night light, and the water animation at its night brightness.
        scene.set_light([0.18, 0.24, 0.62], 1.0);
        scene.set_water_brightness(0.05);
        scene.set_water_anim(true);
        scene.set_walkers(Vec::new());
        if scene.has_dirty() {
            scene.flush_dirty();
        }
        let mut fb = Framebuffer::new(480, 270);
        scene.composite(&mut fb);
        let mut composites = 0;
        // 60 s at the 6 fps tick.
        for i in 0..360u32 {
            let mut tick = false;
            if scene.has_dirty() {
                scene.flush_dirty();
                tick = true;
            }
            let ws = Vec::new();
            if ws != scene.walkers_snapshot() {
                scene.set_walkers(ws);
                tick = true;
            }
            if scene.visible_water() > 0 && scene.animate_water(i as f32) > 0 {
                tick = true;
            }
            if tick {
                composites += 1;
                scene.composite(&mut fb);
            }
        }
        assert_eq!(composites, 0, "{composites} needless composites over 60 s");
    }

    /// B61: a daylight water tick must queue a composite of the water it just
    /// animated. The 4x4 damage blocks were built from the block's `(x0,y0)` and
    /// `(x0+4,y0+4)` corners, which land on the same screen x in this
    /// projection, so every rectangle was zero pixels wide, `mark_rect` dropped
    /// it, and the water overlay was never recomposited at all.
    #[test]
    fn a_daylight_water_tick_damages_the_water_it_animates() {
        let mut found = None;
        for seed in 1..=40u64 {
            let mut scene = Scene::new(SceneConfig {
                width: 960,
                height: 540,
                seed,
                mode: SceneMode::Sim,
                cars: 0,
                peds: 0,
                buildings: 0,
                rain: false,
            });
            scene.set_light([1.0, 0.99, 0.98], 0.0);
            scene.set_water_brightness(1.0);
            scene.set_water_anim(true);
            if scene.has_dirty() {
                scene.flush_dirty();
            }
            let mut fb = Framebuffer::new(960, 540);
            scene.composite(&mut fb);
            let _ = scene.take_pending();
            if scene.visible_water() >= 4 {
                found = Some(scene);
                break;
            }
        }
        let mut scene = found.expect("no seed with water on screen in 40 tries");
        let blocks = scene.animate_water(1.0);
        assert!(blocks > 0, "the tick reported no water blocks");
        let rects = scene
            .take_pending()
            .expect("water damage, not the full-frame fallback");
        assert!(!rects.is_empty(), "the water tick queued no composite at all");
        for r in &rects {
            assert!(
                (r.x1 - r.x0) > 0 && (r.y1 - r.y0) > 0,
                "a water damage rectangle has no area: {r:?}"
            );
        }
        let area: i64 = rects.iter().map(|r| r.area()).sum();
        assert!(area > 0);
    }

    fn small(mode: SceneMode) -> Scene {
        Scene::new(SceneConfig {
            width: 960,
            height: 540,
            seed: 3,
            mode,
            cars: 40,
            peds: 60,
            buildings: 40,
            rain: false,
        })
    }

    #[test]
    fn empty_scene_has_no_actors_or_buildings() {
        let s = small(SceneMode::Empty);
        assert!(s.buildings.is_empty());
        assert!(s.life.is_empty());
        assert!(s.map.count_trees() > 0);
    }

    #[test]
    fn flat_map_has_no_seams() {
        let vp = map::world_for_viewport(960, 540);
        let mut m = map::generate(7, vp.w, vp.h);
        for t in m.tiles.iter_mut() {
            t.corner = [5; 4];
            t.kind = map::Kind::Grass;
            t.prop = map::Prop::None;
        }
        let s = Scene::from_map(
            m,
            SceneConfig {
                width: 960,
                height: 540,
                seed: 7,
                mode: SceneMode::Sim,
                cars: 0,
                peds: 0,
                buildings: 0,
                rain: false,
            },
        );
        let gaps = s.depth().iter().filter(|k| **k == 0).count();
        assert_eq!(gaps, 0, "flat map: {gaps} uncovered pixels");
    }

    /// B34 item 2: the layers must fit the budget at the owner's union. D61 asked for
    /// ≤ 200 MB, the B34 review for ≤ 170 MB after the wallpaper measured 216 MB
    /// of RSS.
    #[test]
    fn layer_budget_at_the_triple_union() {
        let lb = layer_bytes(7040, 1440, 0.30);
        let mb = |b: usize| b as f64 / (1024.0 * 1024.0);
        // static (colour + depth) + the tinted copy + the band sky + the frame.
        let tinted = 7040 * 1440 * 4;
        let total = lb.total() + tinted;
        assert!(
            total <= 170 * 1024 * 1024,
            "layers at 7040x1440: {:.1} MB (static {:.1} + sky {:.1} + frame {:.1} + shadow {:.1})",
            mb(total),
            mb(lb.static_b),
            mb(lb.sky_b),
            mb(lb.frame_b),
            mb(lb.shadow_b)
        );
        // The sky is only the band, not the whole frame.
        assert!(lb.sky_b < lb.frame_b / 2, "sky layer is not band-sized");
    }

    #[test]
    fn terrain_covers_every_pixel() {
        for (w, h) in [(960, 540), (853, 480), (683, 384), (1720, 720)] {
            let s = Scene::new(SceneConfig {
                width: w,
                height: h,
                seed: 5,
                mode: SceneMode::Empty,
                cars: 0,
                peds: 0,
                buildings: 0,
                rain: false,
            });
            let sky = pal::SKY_TOP;
            let horizon = pal::SKY_HORIZON;
            let mut uncovered = 0;
            // Every pixel's painter key must have been written by something.
            for &k in s.depth() {
                if k == 0 {
                    uncovered += 1;
                }
            }
            assert_eq!(
                uncovered, 0,
                "{w}x{h}: {uncovered} pixels not covered by terrain (sky {sky:#x} {horizon:#x})"
            );
        }
    }

    /// A tile whose centre is comfortably on screen and which has no building
    /// standing on it.
    fn visible_free_tile(s: &Scene) -> (i32, i32) {
        for ty in 0..s.map.h {
            for tx in 0..s.map.w {
                let (sx, sy) = s.cam.project(tx as f32 + 0.5, ty as f32 + 0.5, 0);
                // 64-px tiles: keep a tile's worth of margin from the edge.
                if sx < 64.0
                    || sy < 64.0
                    || sx > s.width() as f32 - 64.0
                    || sy > s.height() as f32 - 64.0
                {
                    continue;
                }
                let covered = s
                    .buildings
                    .iter()
                    .any(|b| tx >= b.tx && tx < b.tx + b.fw && ty >= b.ty && ty < b.ty + b.fh);
                if !covered {
                    return (tx, ty);
                }
            }
        }
        panic!("no free visible tile");
    }

    fn mismatches(a: &[u32], b: &[u32]) -> usize {
        a.iter().zip(b.iter()).filter(|(x, y)| x != y).count()
    }

    fn depth_mismatches(a: &[u16], b: &[u16]) -> usize {
        a.iter().zip(b.iter()).filter(|(x, y)| x != y).count()
    }

    #[test]
    fn dirty_flush_matches_full_rebuild() {
        let mut s = small(SceneMode::Test);
        let before = s.static_layer().px().to_vec();
        let before_depth = s.depth().to_vec();
        let (tx, ty) = visible_free_tile(&s);
        let r1 = s.dirty_tile(tx, ty);
        let r2 = s.dirty_tile(tx + 2, ty - 1);
        assert!(
            !r1.is_empty() && !r2.is_empty(),
            "dirty rects must land on screen"
        );
        s.flush_dirty();
        assert_eq!(
            mismatches(&before, s.static_layer().px()),
            0,
            "colour differs after a clipped repaint"
        );
        assert_eq!(
            depth_mismatches(&before_depth, s.depth()),
            0,
            "depth differs after a clipped repaint"
        );
    }

    #[test]
    fn dirty_building_flush_matches_full_rebuild() {
        let mut s = small(SceneMode::Test);
        let before = s.static_layer().px().to_vec();
        let before_depth = s.depth().to_vec();
        s.dirty_building(0);
        s.dirty_building(s.buildings.len() - 1);
        s.flush_dirty();
        assert_eq!(mismatches(&before, s.static_layer().px()), 0);
        assert_eq!(depth_mismatches(&before_depth, s.depth()), 0);
    }

    #[test]
    fn set_tile_repaints_only_its_region() {
        let mut s = small(SceneMode::Test);
        let (tx, ty) = visible_free_tile(&s);
        let before = s.static_layer().px().to_vec();
        let mut t = s.map.at(tx, ty);
        t.corner = t.corner.map(|c| c.saturating_add(1).min(map::MAX_LEVEL));
        let r = s.set_tile(tx, ty, t);
        s.flush_dirty();
        let after = s.static_layer().px().to_vec();

        let (w, h) = (s.width(), s.height());
        let (mut inside, mut outside) = (0usize, 0usize);
        for y in 0..h {
            for x in 0..w {
                if before[y * w + x] != after[y * w + x] {
                    let in_rect = x as i32 >= r.x0
                        && (x as i32) < r.x1
                        && y as i32 >= r.y0
                        && (y as i32) < r.y1;
                    if in_rect {
                        inside += 1;
                    } else {
                        outside += 1;
                    }
                }
            }
        }
        assert!(inside > 0, "the raised tile did not repaint");
        assert_eq!(outside, 0, "nothing outside the dirty rect may change");
    }

    #[test]
    fn composite_is_deterministic_for_same_steps() {
        let mut a = small(SceneMode::Test);
        let mut b = small(SceneMode::Test);
        for _ in 0..10 {
            a.step(1.0 / 30.0);
            b.step(1.0 / 30.0);
        }
        let mut fa = Framebuffer::new(a.width(), a.height());
        let mut fb = Framebuffer::new(b.width(), b.height());
        a.composite(&mut fa);
        b.composite(&mut fb);
        assert_eq!(fa.px(), fb.px());
    }

    #[test]
    fn merge_collapses_overlapping_rects() {
        let a = Rect::new(0, 0, 10, 10);
        let b = Rect::new(5, 5, 20, 20);
        let c = Rect::new(100, 100, 110, 110);
        let merged = merge_rects(vec![a, b, c], 0);
        assert_eq!(merged.len(), 2);
        assert!(merged.contains(&Rect::new(0, 0, 20, 20)));

        // Touching rects merge with 1 px of slack; a gap needs more.
        let d = Rect::new(0, 0, 10, 10);
        let e = Rect::new(10, 0, 20, 10);
        assert_eq!(merge_rects(vec![d, e], 0).len(), 2);
        assert_eq!(merge_rects(vec![d, e], 1).len(), 1);
        let f = Rect::new(14, 0, 20, 10);
        assert_eq!(merge_rects(vec![d, f], 0).len(), 2);
        assert_eq!(merge_rects(vec![d, f], 5).len(), 1);
        assert!(merge_rects(vec![], 0).is_empty());
    }

    #[test]
    fn merged_flush_matches_full_rebuild() {
        let mut s = small(SceneMode::Test);
        let before = s.static_layer().px().to_vec();
        let before_depth = s.depth().to_vec();
        let (w, h) = (s.viewport.w, s.viewport.h);
        let mut rng = map::rng::Rng::new(9);
        for _ in 0..50 {
            s.dirty_tile(rng.range(2, w - 3), rng.range(2, h - 3));
        }
        let raw = s.dirty_count();
        s.flush_dirty();
        assert_eq!(mismatches(&before, s.static_layer().px()), 0);
        assert_eq!(depth_mismatches(&before_depth, s.depth()), 0);
        assert!(s.stats.last_flush_rects <= raw);
        assert!(s.stats.last_flush_rects_raw == raw);
    }

    #[test]
    fn visible_mask_matches_the_frame() {
        let s = small(SceneMode::Test);
        let visible = s.visible_tiles();
        assert!(visible > 0);
        assert!(
            visible < (s.map.w * s.map.h) as usize,
            "the world is a rectangle around a diamond"
        );
        // Tiles near the world's corners are off-screen.
        assert!(!s.is_visible(0, 0));
    }

    fn house(x: i32, y: i32, stage: crate::city::Stage) -> CityObj {
        CityObj {
            x,
            y,
            w: 1,
            h: 1,
            level: 3,
            kind: crate::city::ObjKind::House,
            storeys: 2,
            style: 0,
            stage,
            seed: 7,
            front: 0,
            t: 0.0,
            kit: 0, wear: Default::default(),
        }
    }

    fn city_scene() -> Scene {
        Scene::new(SceneConfig {
            width: 960,
            height: 540,
            seed: 3,
            mode: SceneMode::Sim,
            cars: 0,
            peds: 0,
            buildings: 0,
            rain: false,
        })
    }

    #[test]
    fn city_objects_reach_the_frame() {
        // B7 item 1: the H6 bug was that set_city dirtied tiles but never
        // repainted them, so no building ever reached a PNG.
        let mut s = city_scene();
        let mut without = Framebuffer::new(s.width(), s.height());
        s.composite(&mut without);

        // Spread out so the houses do not hide each other.
        // Sixteen cottages, spread out: the B7/B9 check is that the city layer
        // really reaches the framebuffer.
        // Around the middle of the frame, spread over ~9 tiles.
        let (cx, cy) = (s.viewport.w / 2, s.viewport.h / 2);
        let lots: Vec<CityObj> = (0..16)
            .map(|i| {
                let (x, y) = (cx - 6 + (i % 4) * 4, cy - 6 + (i / 4) * 4);
                let mut o = house(x, y, crate::city::Stage::Finished);
                o.level = s.map.at(x, y).max_level();
                o
            })
            .collect();
        s.set_city(lots.clone());
        let mut with = Framebuffer::new(s.width(), s.height());
        s.composite(&mut with);

        let diff: usize = (0..s.width() * s.height())
            .filter(|i| without.px()[*i] != with.px()[*i])
            .count();
        assert!(diff >= 2000, "only {diff} pixels changed when the city was added");

        // Every lot must be visible: its screen rect has to contain changes.
        for l in &lots {
            let mut r = Rect::new(i32::MAX, i32::MAX, i32::MIN, i32::MIN);
            for (x, y) in [
                (l.x as f32, l.y as f32),
                ((l.x + 1) as f32, (l.y + 1) as f32),
                ((l.x + 1) as f32, l.y as f32),
                (l.x as f32, (l.y + 1) as f32),
            ] {
                for z in [
                    l.level as i32 * crate::iso::HEIGHT_UNIT,
                    l.level as i32 * crate::iso::HEIGHT_UNIT + 48,
                ] {
                    let p = s.cam.project(x, y, z);
                    r.x0 = r.x0.min(p.0 as i32);
                    r.y0 = r.y0.min(p.1 as i32);
                    r.x1 = r.x1.max(p.0 as i32);
                    r.y1 = r.y1.max(p.1 as i32);
                }
            }
            let hit = (r.y0.max(0)..r.y1.min(s.height() as i32))
                .any(|y| {
                    (r.x0.max(0)..r.x1.min(s.width() as i32)).any(|x| {
                        let i = y as usize * s.width() + x as usize;
                        without.px()[i] != with.px()[i]
                    })
                });
            assert!(hit, "lot at {},{} is not drawn", l.x, l.y);
        }
    }

    #[test]
    fn every_stage_looks_different() {
        use crate::city::Stage;
        let mut s = city_scene();
        let (ccx, ccy) = (s.viewport.w / 2, s.viewport.h / 2);
        let mut frames: Vec<Vec<u32>> = Vec::new();
        for stage in Stage::ALL {
            let mut o = house(ccx, ccy, stage);
            o.level = s.map.at(ccx, ccy).max_level();
            s.set_city(vec![o]);
            let mut fb = Framebuffer::new(s.width(), s.height());
            s.composite(&mut fb);
            frames.push(fb.px().to_vec());
        }
        for i in 0..frames.len() {
            for j in i + 1..frames.len() {
                let diff = frames[i]
                    .iter()
                    .zip(frames[j].iter())
                    .filter(|(a, b)| a != b)
                    .count();
                assert!(diff > 20, "stages {i} and {j} render the same ({diff} px)");
            }
        }
    }

    #[test]
    fn actors_are_hidden_behind_buildings() {
        let mut s = small(SceneMode::Test);
        // B69: swaying trees are drawn over the static layer, so what an
        // occluded pixel shows is a tree there; this test is about buildings.
        s.set_tree_sway(false);
        // Let the traffic get among the buildings.
        for _ in 0..300 {
            s.step(1.0 / 30.0);
        }
        let (w, h) = (s.width(), s.height());
        let mut off = Framebuffer::new(w, h);
        let mut on = Framebuffer::new(w, h);
        s.depth_mode = DepthMode::Off;
        s.composite(&mut off);
        s.depth_mode = DepthMode::PerPixel;
        s.composite(&mut on);

        let static_px = s.static_layer().px();
        let sky = s.sky().px();
        // The static layer carries the "solid feature" tag in its alpha byte
        // (B42b); the composited frame does not.
        let plain = |c: palette::Color| palette::with_alpha(c, 0xFF);
        let mut hidden = 0;
        for i in 0..w * h {
            if off.px()[i] != on.px()[i] {
                hidden += 1;
                // Whatever the actor painted over must be what was underneath:
                // the static layer, or the sky it does not cover.
                let from_ground = plain(on.px()[i]) == plain(static_px[i]);
                let from_sky = i < sky.len() && on.px()[i] == sky[i];
                assert!(
                    from_ground || from_sky,
                    "occluded pixel {i} shows neither static layer nor sky"
                );
            }
        }
        assert!(
            hidden > 20,
            "expected actors to be hidden behind buildings, only {hidden} pixels differed"
        );
    }

    /// B61e: the row-band index must not change a single pixel. The same frame
    /// composed with the index and with it emptied (which falls back to the list
    /// order the index replaced) has to be identical — that is the proof the
    /// brief asks for, on a frame that has people, cars, walkers and water.
    #[test]
    fn the_band_index_draws_the_same_frame_as_the_list() {
        let mut s = small(SceneMode::Test);
        // A crowd and a queue of traffic spread over the whole frame, so the
        // bands are all in use and plenty of items share a band.
        let (w, h) = (s.width() as f32, s.height() as f32);
        let mut figures = Vec::new();
        for i in 0..240 {
            let x = s.map.w as f32 * ((i % 17) as f32 + 0.5) / 17.0;
            let y = s.map.h as f32 * ((i / 17) as f32 + 0.5) / 15.0;
            figures.push(crate::people::Figure {
                x,
                y,
                pose: crate::people::POSE_WALK,
                tone: (i % 6) as u8,
                child: i % 5 == 0,
                prop: crate::people::PROP_NONE,
                dir: (i % 4) as u8,
                open: false,
                lift: 0,
                frame: 0,
            });
        }
        s.set_people(&figures, 0.0);
        let mut bodies = Vec::new();
        for i in 0..120 {
            let x = s.map.w as f32 * ((i % 12) as f32 + 0.5) / 12.0;
            let y = s.map.h as f32 * ((i / 12) as f32 + 0.5) / 10.0;
            bodies.push(crate::traffic::Body {
                x,
                y,
                dir: if i % 2 == 0 { (1.0, 0.0) } else { (0.0, 1.0) },
                kind: crate::traffic::Kind::Car,
                style: crate::traffic::Style::Saloon,
                variant: (i % 4) as u8,
                lift: 0.0,
                lights: i % 3 == 0,
                brake: i % 7 == 0,
                blink: 0.0,
                signal: 0,
            });
        }
        s.set_traffic(bodies);
        s.set_water_anim(true);
        s.animate_water(0.37);
        let _ = (w, h);

        assert!(!s.people_bands.is_empty() && !s.traffic_bands.is_empty());
        let mut with_index = Framebuffer::new(s.width(), s.height());
        s.composite(&mut with_index);

        // Empty indices: every layer falls back to walking its list in full.
        s.people_bands.clear();
        s.traffic_bands.clear();
        s.parked_bands.clear();
        s.mover_bands.clear();
        s.water_bands.clear();
        s.lamp_bands.clear();
        let mut by_list = Framebuffer::new(s.width(), s.height());
        s.composite(&mut by_list);

        let a = with_index.px();
        let b = by_list.px();
        let mut diff = 0usize;
        let mut first = None;
        for i in 0..a.len() {
            if a[i] != b[i] {
                diff += 1;
                if first.is_none() {
                    first = Some((i % a.len() / s.width(), i % s.width()));
                }
            }
        }
        assert!(
            diff == 0,
            "the band index changed {diff} pixels (first at {:?})",
            first
        );
    }

    /// B61e: the band order must not be visible even when figures overlap *across*
    /// a band edge. A crowd packed into a small area (so sprites overlap, and
    /// with more than 1400 of them they are drawn as 2-px dots, which is a plain
    /// `put` and therefore order-sensitive) is the worst case.
    #[test]
    fn a_packed_crowd_is_band_order_independent() {
        let mut s = small(SceneMode::Test);
        let mut figures = Vec::new();
        for i in 0..1700 {
            // A tight block around the middle of the map: neighbouring figures
            // overlap on screen, and the block straddles many band edges.
            let x = s.map.w as f32 * 0.5 + ((i % 41) as f32 - 20.0) * 0.06;
            let y = s.map.h as f32 * 0.5 + ((i / 41) as f32 - 20.0) * 0.06;
            figures.push(crate::people::Figure {
                x,
                y,
                pose: crate::people::POSE_WALK,
                tone: (i % 6) as u8,
                child: false,
                prop: crate::people::PROP_NONE,
                dir: (i % 4) as u8,
                open: false,
                lift: 0,
                frame: 0,
            });
        }
        // The list order must not be the band order, or the test proves nothing:
        // a deterministic shuffle (an LCG walk) scrambles y against index.
        let n = figures.len();
        let mut seed = 12345u64;
        for i in (1..n).rev() {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let j = (seed >> 33) as usize % (i + 1);
            figures.swap(i, j);
        }
        s.set_people(&figures, 0.0);
        s.animate_water(0.2);
        let mut with_index = Framebuffer::new(s.width(), s.height());
        s.composite(&mut with_index);
        // Not a vacuous pass: the frame really has the crowd on it.
        let painted = with_index.px().iter().filter(|p| **p != 0).count();
        assert!(painted > 5_000, "the frame is nearly empty ({painted} px)");
        s.people_bands.clear();
        let mut by_list = Framebuffer::new(s.width(), s.height());
        s.composite(&mut by_list);
        assert!(
            with_index.px() == by_list.px(),
            "a packed crowd differs between band order and list order"
        );
    }

    #[test]
    fn rain_toggle_keeps_layers_consistent() {
        let mut s = small(SceneMode::Test);
        assert!(!s.rain_on());
        s.toggle_rain();
        assert!(s.rain_on());
        s.step(1.0 / 30.0);
        s.toggle_rain();
        assert!(!s.rain_on());
    }

    /// B43: an empty people layer must mark nothing. That is the whole 3 am
    /// contract — the wallpaper wakes, asks the sim for the street, gets nobody,
    /// and goes back to sleep instead of compositing the frame at 6 fps.
    #[test]
    fn an_empty_street_costs_no_composites() {
        let mut s = small(SceneMode::Sim);
        // The visible tile nearest the middle of the frame, so the figure is
        // comfortably inside it.
        let (mut tx, mut ty) = (s.map.w / 2, s.map.h / 2);
        let mut best = i32::MAX;
        for y in 0..s.map.h {
            for x in 0..s.map.w {
                if !s.is_visible(x, y) {
                    continue;
                }
                let d = (x - s.map.w / 2).abs() + (y - s.map.h / 2).abs();
                if d < best {
                    best = d;
                    tx = x;
                    ty = y;
                }
            }
        }
        let figure = crate::people::Figure {
            x: tx as f32 + 0.5,
            y: ty as f32 + 0.5,
            pose: crate::people::POSE_WALK,
            tone: 1,
            child: false,
            prop: crate::people::PROP_NONE,
            dir: 0,
            open: false,
            lift: 0,
            frame: 0,
        };
        // The scene starts with the whole frame pending; drain that first, so
        // the assertions below see only what the people layer marks.
        let _ = s.take_pending();
        s.set_people(&[figure], 0.0);
        assert_eq!(s.people_count(), 1, "the visible figure is kept");
        assert!(
            matches!(s.take_pending(), Some(v) if !v.is_empty()),
            "a figure entering the frame dirties its square"
        );
        // Same people again, same clock: nothing changed a pixel, so nothing
        // is repainted (D71: a standing crowd costs no composite work).
        s.set_people(&[figure], 0.0);
        assert!(
            matches!(s.take_pending(), Some(v) if v.is_empty()),
            "a figure that did not move must not repaint itself"
        );
        // A walker moved one pixel: that square is repainted.
        let mut moved = figure;
        moved.x += 1.0 / 64.0;
        s.set_people(&[moved], 0.0);
        assert!(
            matches!(s.take_pending(), Some(v) if !v.is_empty()),
            "a walking figure repaints its square"
        );
        // Night falls: the street empties. The spot is repainted once...
        s.set_people(&[], 0.0);
        assert_eq!(s.people_count(), 0);
        assert!(
            matches!(s.take_pending(), Some(v) if !v.is_empty()),
            "a leaver's square is repainted"
        );
        // ...and then there is nothing at all to do.
        s.set_people(&[], 0.0);
        assert!(
            matches!(s.take_pending(), Some(v) if v.is_empty()),
            "an empty street must not wake the compositor"
        );
    }

    /// A figure behind a building is hidden by it (per-pixel painter keys), and
    /// a figure in the open is drawn.
    #[test]
    fn people_are_occluded_by_buildings() {
        let mut s = small(SceneMode::Test);
        let mut fb = Framebuffer::new(s.width(), s.height());
        let base = {
            s.composite(&mut fb);
            fb.px().to_vec()
        };
        // Stand a figure on every visible tile in the near corner: at least one
        // must change the frame.
        let mut figs = Vec::new();
        for y in 0..s.map.h {
            for x in 0..s.map.w {
                if s.is_visible(x, y) {
                    figs.push(crate::people::Figure {
                        x: x as f32 + 0.5,
                        y: y as f32 + 0.5,
                        pose: crate::people::POSE_STAND,
                        tone: 3,
                        child: false,
                        prop: crate::people::PROP_NONE,
                        dir: 1,
                        open: false,
                        lift: 0,
                        frame: 0,
                    });
                }
            }
        }
        s.set_people(&figs, 0.0);
        s.flush_dirty();
        s.composite(&mut fb);
        let changed = fb.px().iter().zip(base.iter()).filter(|(a, b)| a != b).count();
        assert!(changed > 0, "a crowd of residents changed no pixel");
    }

    #[test]
    fn solid_features_take_the_night_tint() {
        // B45c: solid (tagged) pixels once skipped the ambient light, so park
        // lots and plazas stayed noon-bright at night.
        let lut = crate::light::lut([0.2, 0.25, 0.6]);
        let s = palette::with_alpha(palette::rgb(120, 200, 80), TAG_BUILT);
        let out = night_pixel(s, &lut, &[]);
        assert_eq!(palette::alpha(out), 0xFF);
        // Dark, but not black: it is the *night* version of the colour.
        assert!(palette::g(out) < 110 && palette::r(out) < 70, "{:x}", out);
        // B58b item 7: the material still shows through the moonlight, so a
        // green park stays green-dominant instead of turning navy like the ground
        // it stands on.
        assert!(
            palette::g(out) > palette::r(out) + 20,
            "green lost at night: {:x}",
            out
        );
        // Emissive kit lights keep their colour.
        let e = palette::with_alpha(palette::rgb(255, 214, 138), 0xFE);
        assert_eq!(palette::r(night_pixel(e, &lut, &[])), 255);
    }

    /// B64: a pale surface (a concrete lot, plaza paving) must not glow next to
    /// the ground at night, must be untouched by day, and must not pop while
    /// the light moves through dusk.
    #[test]
    fn pale_ground_takes_the_night_like_the_rest() {
        let lum = |c: Color| {
            0.30 * palette::r(c) as f32 + 0.59 * palette::g(c) as f32 + 0.11 * palette::b(c) as f32
        };
        let night = crate::light::lut([0.16, 0.22, 0.57]);
        let grass = night_pixel(palette::with_alpha(palette::rgb(66, 135, 56), 0xFF), &night, &[]);
        for c in [
            palette::rgb(189, 188, 173),
            palette::rgb(215, 215, 220),
            palette::rgb(170, 175, 190),
            palette::rgb(240, 238, 230),
        ] {
            let out = night_pixel(palette::with_alpha(c, 0xFF), &night, &[]);
            assert!(
                lum(out) < lum(grass) * 1.35 + 6.0,
                "pale {:x} glows at night: {:x} vs ground {:x}",
                c,
                out,
                grass
            );
        }
        // A painted fountain's water goes dark with the lakes.
        let basin = night_pixel(palette::with_alpha(palette::rgb(60, 150, 200), TAG_BUILT), &night, &[]);
        assert!(lum(basin) < 40.0, "fountain water glows at night: {:x}", basin);
        // Noon: the identity ambient leaves every colour as it was.
        let noon = crate::light::lut([1.0, 1.0, 1.0]);
        let c = palette::with_alpha(palette::rgb(189, 188, 173), 0xFF);
        assert_eq!(night_pixel(c, &noon, &[]), c);
        // Dusk: a smooth ramp, no pops.
        let mut last: Option<Color> = None;
        for i in 0..=40 {
            let k = 1.0 - i as f32 * 0.02;
            let l = crate::light::lut([k * 0.8 + 0.2 * 0.16, k * 0.8 + 0.2 * 0.22, k * 0.8 + 0.2 * 0.57]);
            let out = night_pixel(c, &l, &[]);
            if let Some(p) = last {
                for ch in [palette::r, palette::g, palette::b] {
                    assert!((ch(out) as i32 - ch(p) as i32).abs() <= 8, "dusk pop at step {i}");
                }
            }
            last = Some(out);
        }
    }

    /// Two materials of the same brightness must not come out one colour at
    /// night (B58b item 7: "all buildings uniformly blue").
    #[test]
    fn night_keeps_materials_apart() {
        let lut = crate::light::lut([0.16, 0.22, 0.57]);
        let brick = night_pixel(palette::with_alpha(palette::rgb(150, 95, 80), TAG_BUILT), &lut, &[]);
        let slate = night_pixel(palette::with_alpha(palette::rgb(95, 100, 130), TAG_BUILT), &lut, &[]);
        assert!(
            palette::r(brick) > palette::r(slate) + 15,
            "brick {:x} and slate {:x} read the same at night",
            brick,
            slate
        );
        assert!(
            palette::b(slate) > palette::r(slate) + 6,
            "slate must stay cool: {:x}",
            slate
        );
    }
}

/// B53: the gameday host: the stadium, or the biggest park when the city has no
/// stadium. Returns its index in `city`.
pub fn gameday_host(city: &[CityObj]) -> Option<usize> {
    use crate::city::{LandmarkKind, ObjKind};
    if let Some(i) = city
        .iter()
        .position(|o| matches!(o.kind, ObjKind::Landmark(LandmarkKind::Stadium)))
    {
        return Some(i);
    }
    city.iter()
        .enumerate()
        .filter(|(_, o)| matches!(o.kind, ObjKind::Park))
        .max_by_key(|(_, o)| o.w as i64 * o.h as i64)
        .map(|(i, _)| i)
}

/// B62: where a festival goes: the civic square, else the biggest park.
pub fn festival_host(city: &[CityObj]) -> Option<usize> {
    use crate::city::{LandmarkKind, ObjKind};
    if let Some(i) = city
        .iter()
        .position(|o| matches!(o.kind, ObjKind::Landmark(LandmarkKind::Plaza)))
    {
        return Some(i);
    }
    city.iter()
        .enumerate()
        .filter(|(_, o)| matches!(o.kind, ObjKind::Park) && o.w >= 3 && o.h >= 3)
        .max_by_key(|(_, o)| o.w as i64 * o.h as i64)
        .map(|(i, _)| i)
}

/// B87: where the composite's time goes, pass by pass, for the benches.
///
/// Set `CITYLIVE_PASS_TIME=1` and call [`report_pass_times`] (the spike's
/// `--water-bench` does) and every `composite_rect` adds its own per-pass
/// nanoseconds to a thread-local table. Off by default: one relaxed load and
/// one branch per pass.
pub mod pass_time {
    use std::cell::RefCell;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// The passes, in the order `composite_rect` runs them.
    pub const NAMES: [&str; 24] = [
        "sky",
        "static blit",
        "cast shadows",
        "mask above land",
        "trees",
        "ending ground",
        "water overlay",
        "nightlife",
        "cloud shadows",
        "street/tech/camp",
        "(unused)",
        "spec/civic over",
        "cloud band",
        "rain",
        "ambient wash",
        "spec under",
        "move shadows",
        "actors",
        "traffic",
        "parked",
        "sites+rigs",
        "people",
        "boat haze",
        "mover draw",
    ];

    pub static ON: AtomicBool = AtomicBool::new(false);

    thread_local! {
        static T: RefCell<[u64; 24]> = const { RefCell::new([0u64; 24]) };
    }

    /// Turn the clock on from the environment, once.
    pub fn enable_from_env() -> bool {
        let on = std::env::var_os("CITYLIVE_PASS_TIME").is_some();
        ON.store(on, Ordering::Relaxed);
        on
    }

    #[inline]
    pub fn on() -> bool {
        ON.load(Ordering::Relaxed)
    }

    #[inline]
    pub fn add(i: usize, ns: u64) {
        T.with(|t| t.borrow_mut()[i] += ns);
    }

    /// A stopwatch that only runs when the clock is on.
    pub struct Lap {
        on: bool,
        t: std::time::Instant,
    }

    impl Lap {
        pub fn new() -> Self {
            let on = on();
            Self { on, t: std::time::Instant::now() }
        }

        /// Charge the time since the last lap to pass `i`, and start again.
        pub fn mark(&mut self, i: usize) {
            if self.on {
                add(i, self.t.elapsed().as_nanos() as u64);
                self.t = std::time::Instant::now();
            }
        }
    }

    impl Default for Lap {
        fn default() -> Self {
            Self::new()
        }
    }

    pub fn take() -> [u64; 24] {
        T.with(|t| std::mem::take(&mut *t.borrow_mut()))
    }
}

/// B87: print the per-pass composite cost collected since the last call, over
/// `frames` frames, and reset it.
pub fn report_pass_times(frames: usize) {
    if !pass_time::on() {
        eprintln!("pass time: off (CITYLIVE_PASS_TIME={:?})", std::env::var("CITYLIVE_PASS_TIME"));
        return;
    }
    let t = pass_time::take();
    let n = frames.max(1) as f64;
    let total: u64 = t.iter().sum();
    println!("  composite passes, ms/frame over {frames} frames:");
    for (i, name) in pass_time::NAMES.iter().enumerate() {
        if t[i] == 0 {
            continue;
        }
        println!(
            "    {name:<18} {:>7.3} ms  ({:>4.1} %)",
            t[i] as f64 / 1e6 / n,
            t[i] as f64 * 100.0 / total.max(1) as f64
        );
    }
    println!("    {:<18} {:>7.3} ms", "sum", total as f64 / 1e6 / n);
}

/// B53: tailgaters along the front of a gameday host (a stadium lot or a park).
fn tailgate_row(fb: &mut Framebuffer, cam: &Camera, o: &CityObj, holiday: &crate::holiday::HolidayLook) {
    for i in 0..3u32 {
        let gx = o.x as f32 + 1.0 + i as f32 * (o.w as f32 / 3.0);
        let gy = (o.y + o.h) as f32 + 1.0;
        crate::holiday::tailgate(
            fb,
            cam,
            gx,
            gy,
            o.level,
            holiday,
            (o.x as u32).wrapping_mul(31) ^ (o.y as u32) ^ i,
        );
    }
}
