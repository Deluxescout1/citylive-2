//! B32 integration: turn sim lots into `buildkit` building sprites and hand
//! them to the renderer.
//!
//! `render` cannot depend on `buildkit` (buildkit depends on render for the
//! palette), so the pixels cross the boundary in `render::scene::KitSprite`.
//! This module owns the mapping (a lot → a `BuildingSpec`), the sprite cache
//! keyed by (spec, light bucket) — the brief's rule — and the depth-free blit
//! the scene does at the lot's tile origin.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};

use crate::{BuildingFunction, BuildingSpec, BuildingStyle, ConstructionStage, Era, LandmarkKind, Light};
use render::city::{CityObj, ObjKind, Stage};
use render::scene::KitSprite;

/// B73c: the before/after switch. `CITYLIVE_KIT=legacy` draws the old hand-made
/// catalogue (and quantised seeds) so a report can measure the same city before
/// and after the generator, without two builds.
static KIT_LEGACY: AtomicU8 = AtomicU8::new(2);

/// Force the hand-made catalogue on or off (the report's two passes).
pub fn set_kit_legacy(v: bool) {
    KIT_LEGACY.store(v as u8, Ordering::Relaxed);
}

fn kit_legacy() -> bool {
    match KIT_LEGACY.load(Ordering::Relaxed) {
        0 => false,
        1 => true,
        _ => {
            let v = std::env::var("CITYLIVE_KIT").map(|s| s == "legacy").unwrap_or(false);
            KIT_LEGACY.store(v as u8, Ordering::Relaxed);
            v
        }
    }
}

/// The most sprites we will hold. A day-180 city needs ~220 with the spec
/// quantisation in `spec_for` (measured); the cap is headroom for a longer run.
///
/// **Never mix systems:** past the cap a lot reuses the nearest sprite of the
/// same function (see `fallback`), never the engine's own art — mixing the two
/// is what made H32's downtown look like floating roofs (B32b).
const CACHE_CAP: usize = 4096;

/// B73b: the kit also stops on bytes, not only on entries. A day-60 downtown is
/// ~110 generated towers and each is its own sprite (~40-80 KB); the two or
/// three light buckets the wallpaper keeps would otherwise grow past the memory
/// ceiling. Past the cap the cache reuses the nearest sprite of the same
/// function (below), never the engine's art.
/// B73c: every family is generated per lot now, so a grown city is ~1200
/// distinct sprites where it used to be ~430. At 1x an apartment block is
/// ~60 KB, a house ~15 KB, and the wallpaper's own city (day 60) needs ~70 MB;
/// the cap sits just above that and the reuse fallback stays for denser worlds.
const CACHE_BYTES: usize = 96 * 1024 * 1024;

/// What a cached sprite was built for, so an overflowing cache can reuse a
/// close match instead of falling back to the engine's art.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct EntryKey {
    function: BuildingFunction,
    style: BuildingStyle,
    storeys: u8,
    footprint: (u8, u8),
    /// B50: the ruin code the sprite was made for (0 = intact).
    ruin: u8,
}

/// The lots already placed, in a coarse grid so the neighbour rule costs O(1)
/// per lot instead of scanning the whole city (B73c makes every family
/// neighbour-checked, so the old full scan was 1.4 M comparisons a rebuild).
#[derive(Default)]
struct Placed {
    cells: HashMap<(i32, i32), Vec<(i32, i32, i32, i32, crate::genkit::DesignKey)>>,
}

impl Placed {
    const CELL: i32 = 4;

    fn insert(&mut self, x: i32, y: i32, w: i32, h: i32, key: crate::genkit::DesignKey) {
        let c = (x.div_euclid(Self::CELL), y.div_euclid(Self::CELL));
        self.cells.entry(c).or_default().push((x, y, w, h, key));
    }

    /// Does a design that touches this lot (gap ≤ 1 tile) already wear `key`?
    fn clash(&self, x: i32, y: i32, w: i32, h: i32, key: crate::genkit::DesignKey) -> bool {
        let (cx, cy) = (x.div_euclid(Self::CELL), y.div_euclid(Self::CELL));
        for dy in -1..=1 {
            for dx in -1..=1 {
                let Some(list) = self.cells.get(&(cx + dx, cy + dy)) else {
                    continue;
                };
                for &(nx, ny, nw, nh, nk) in list {
                    if nk != key {
                        continue;
                    }
                    let gap_x = (x - (nx + nw)).max(nx - (x + w));
                    let gap_y = (y - (ny + nh)).max(ny - (y + h));
                    if gap_x <= 1 && gap_y <= 1 {
                        return true;
                    }
                }
            }
        }
        false
    }
}

/// Sprite cache: one entry per (spec, light bucket).
#[derive(Default)]
pub struct KitCache {
    map: HashMap<(BuildingSpec, u8, u8), u16>,
    /// The land's building style (D66 item 4).
    biome: crate::style::BiomeStyle,
    keys: Vec<EntryKey>,
    /// The spec each cached sprite was built for (report / test helper).
    specs: Vec<BuildingSpec>,
    sprites: Vec<KitSprite>,
    /// Bytes held by the sprite pixels, kept as they are pushed.
    bytes_held: usize,
    /// B76 r3: sprites whose skyline is mostly not snow surface.
    bare_roofs: Vec<(crate::BuildingFunction, Option<crate::BuildingType>, usize, usize)>,
    /// B76 r3: sprites with no snow surface (see `CacheReport::unsnowed`).
    unsnowed: usize,
    /// Draws served from the cache and sprites actually rasterised.
    pub hits: usize,
    pub builds: usize,
    /// Lots that had to reuse another lot's sprite because the cache was full.
    pub reused: usize,
}

impl KitCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// A cache whose sprites wear one land's vernacular (D66).
    pub fn with_style(biome: crate::style::BiomeStyle) -> Self {
        Self {
            biome,
            ..Self::default()
        }
    }

    /// The sprite index (1-based, for `CityObj::kit`) for one lot, building it
    /// on first use.
    pub fn index_for(&mut self, o: &CityObj, light: &Light) -> u16 {
        self.index_for_ruin(o, light, 0)
    }

    /// As [`Self::index_for`], for a building the world's ending has done
    /// something to (B50): `ruin` is a `sim::ending::R_*` code.
    pub fn index_for_ruin(&mut self, o: &CityObj, light: &Light, ruin: u8) -> u16 {
        self.index_for_in(o, light, ruin, None)
    }

    /// The one path both attach functions and the single-lot callers take.
    /// `chosen` is the city so far, for the neighbour rule: B73b's towers may
    /// not share shaft+facade+material with a tower that touches them.
    fn index_for_in(
        &mut self,
        o: &CityObj,
        light: &Light,
        ruin: u8,
        mut chosen: Option<&mut Placed>,
    ) -> u16 {
        let Some(mut spec) = spec_for(o) else {
            return 0;
        };
        spec.biome = self.biome;
        if let Some(map) = chosen.as_deref_mut() {
            if crate::genkit::key_of(&spec).is_some() {
                let clash = |k: crate::genkit::DesignKey| map.clash(o.x, o.y, o.w, o.h, k);
                if let Some(seed) = crate::genkit::nudge_seed(&spec, clash) {
                    spec.seed = seed;
                }
            }
            if let Some(k) = crate::genkit::key_of(&spec) {
                map.insert(o.x, o.y, o.w, o.h, k);
            }
        }
        let key = (spec.clone(), light.bucket, ruin);
        if let Some(i) = self.map.get(&key) {
            self.hits += 1;
            return i + 1;
        }
        if self.sprites.len() >= CACHE_CAP || self.bytes_held >= CACHE_BYTES {
            // Full: reuse the nearest sprite of the same function, so the city
            // stays one art system (never a mix of kit sprites and engine art).
            self.reused += 1;
            let want = EntryKey {
                function: spec.function,
                style: spec.style,
                storeys: spec.storeys,
                footprint: spec.footprint,
                ruin,
            };
            return self.fallback(want).map(|i| i + 1).unwrap_or(0);
        }
        let mut s = crate::Sprite::new(1, 1);
        crate::draw(&spec, light, &mut s);
        // buildkit blends its cast shadow onto a transparent sprite, which
        // leaves pixels with alpha 0 but a dark colour. The engine's blitter is
        // 1-bit alpha, so those must become transparent (the engine draws its
        // own ground shadow for kit buildings).
        let top = s
            .px
            .iter()
            .zip(s.depth.iter())
            .filter(|(c, _)| (**c >> 24) >= 0x80)
            .map(|(_, d)| *d)
            .max()
            .unwrap_or(0);
        let px: Vec<u32> = s
            .px
            .into_iter()
            .map(|c| if (c >> 24) < 0x80 { 0 } else { c })
            .collect();
        // B36b: at night the kit's lit windows and neon must survive the ambient
        // tint, or the whole city goes flat blue. They are written with alpha
        // 0xFE, which the renderer treats as "emissive: do not tint me".
        // B38: after midnight most homes are dark (D25). The late bucket keeps a
        // third of the warm panes, chosen deterministically per pixel.
        let late = light.bucket == 3;
        let px: Vec<u32> = if light.bucket == Light::NIGHT_BUCKET || late {
            px.into_iter()
                .map(|c| {
                    if c == 0 {
                        return c;
                    }
                    // B99d: a pixel the catalogue already tagged as a lamp keeps
                    // its own colour; the pass below is for what it painted as
                    // plain material.
                    if (c >> 24) == 0xFE {
                        return c;
                    }
                    // B38: structural steel (the dark towers' cross-bracing) is
                    // painted nearly white by the catalogue and read as glowing
                    // zigzags at night. It is not a light: darken it.
                    let (r, g, b) = ((c >> 16) & 0xFF, (c >> 8) & 0xFF, c & 0xFF);
                    let max = r.max(g).max(b);
                    let min = r.min(g).min(b);
                    if min > 110 && max - min < 45 {
                        let dim = |v: u32| (v * 42) / 100;
                        return (0xFF << 24) | (dim(r) << 16) | (dim(g) << 8) | dim(b);
                    }
                    match night_light(c) {
                        Some(NightLight::Warm) => {
                            if late && (c.wrapping_mul(2654435761) >> 16) % 3 != 0 {
                                // Dark pane at 3 am: the same colour as an unlit
                                // window, so it does not glow.
                                return 0x1a2232;
                            }
                            (c & 0x00FF_FFFF) | (0xFE << 24)
                        }
                        // Cool panes: a glass tower where *every* pane is lit
                        // reads as a white slab with grey zigzags (B38). Keep a
                        // quarter of them lit, the rest dark glass.
                        Some(NightLight::Cool) => {
                            if (c.wrapping_mul(2246822519) >> 16) % 4 != 0 {
                                return 0x1a2232;
                            }
                            let dim = |v: u32| ((v * 55) / 100).min(255);
                            let r = dim((c >> 16) & 0xFF);
                            let g = dim((c >> 8) & 0xFF);
                            let b = dim(c & 0xFF);
                            (0xFE << 24) | (r << 16) | (g << 8) | b
                        }
                        Some(NightLight::Neon) => (c & 0x00FF_FFFF) | (0xFE << 24),
                        None => {
                            // B99c round 2: a surface under an open night sky is
                            // never black. A big unlit roof came out at two or
                            // three percent and, beside lit water, read as a hole
                            // punched in the frame. Each face keeps a floor of the
                            // sky's own colour, written emissive so the ambient
                            // tint does not multiply it away again.
                            let r = (c >> 16) & 0xFF;
                            let g = (c >> 8) & 0xFF;
                            let b = c & 0xFF;
                            if r.max(g).max(b) < 74 {
                                return (0xFE << 24) | (0x22 << 16) | (0x2a << 8) | 0x3c;
                            }
                            c
                        }
                    }
                })
                .collect()
        } else {
            px
        };
        // Crop to the drawn pixels: the catalogue sizes its sprite box for the
        // worst case (a 250-px tower's shadow reach), which leaves large empty
        // margins that would otherwise be kept in memory for every sprite.
        let mut px = px;
        if ruin != 0 {
            crate::ruin::apply(&mut px, s.w, s.h, s.ox, s.oy, ruin, o.seed);
        }
        // B76 r3: the surface tags travel with the pixels (cropped to the same
        // box, packed two to a byte, so they cost an eighth of the pixels).
        let (surf_tags, _, _, _, _) = crop_plane(&px, &s.surf, s.w, s.h, s.ox, s.oy);
        let surf = render::snow::pack(&surf_tags);
        let (px, w, h, ox, oy) = crop(&px, s.w, s.h, s.ox, s.oy);
        // B72: the panes' pixels move with the crop.
        let (dx, dy) = (s.ox - ox, s.oy - oy);
        let panes: Vec<render::scene::KitSpritePane> = s
            .win
            .iter()
            .map(|p| render::scene::KitSpritePane {
                px: p.px.iter().map(|(x, y)| ((x - dx as i16), (y - dy as i16))).collect(),
                warm: p.warm,
            })
            .collect();
        let foot = KitSprite::footprint_of(&px, w, h, ox, oy);
        let kit = KitSprite {
            w,
            h,
            ox,
            oy,
            px: std::sync::Arc::new(px),
            top,
            foot,
            roof: s.roof,
            panes,
            surf,
        };
        let i = self.sprites.len() as u16;
        self.bytes_held += kit.px.len() * 4 + kit.surf.len();
        // B76 r3: how much of the sprite's *skyline* is a snow surface. A column's
        // first drawn pixel is the top of whatever stands there (a ridge, a deck,
        // a spire); if most of those are untagged, this roof will stay bare.
        let (kw, kh) = (kit.w.max(1), kit.h);
        let mut tops = 0usize;
        let mut bare = 0usize;
        for x in 0..kw {
            for y in 0..kh {
                let j = y * kw + x;
                if kit.px[j] == 0 {
                    continue;
                }
                tops += 1;
                let t = kit.surf_at(x, y);
                if !matches!(
                    t,
                    render::snow::FLAT
                        | render::snow::SLOPE_GENTLE
                        | render::snow::SLOPE_MEDIUM
                        | render::snow::SLOPE_STEEP
                        | render::snow::RIDGE
                        | render::snow::LEDGE
                        | render::snow::EQUIP_TOP
                        | render::snow::FOLIAGE
                ) {
                    bare += 1;
                }
                break;
            }
        }
        if tops > 0 && bare * 10 > tops * 4 {
            self.bare_roofs.push((spec.function, spec.building_type, bare, tops));
        }
        let snowed = (0..kw * kh).any(|j| {
            matches!(
                kit.surf_at(j % kw, j / kw),
                render::snow::FLAT
                    | render::snow::SLOPE_GENTLE
                    | render::snow::SLOPE_MEDIUM
                    | render::snow::SLOPE_STEEP
                    | render::snow::EQUIP_TOP
                    | render::snow::LEDGE
            )
        });
        if !snowed {
            self.unsnowed += 1;
        }
        self.sprites.push(kit);
        self.keys.push(EntryKey {
            function: spec.function,
            style: spec.style,
            storeys: spec.storeys,
            footprint: spec.footprint,
            ruin,
        });
        self.specs.push(spec.clone());
        self.map.insert(key, i);
        self.builds += 1;
        i + 1
    }

    /// The closest cached sprite to `want`: same function and style first, then
    /// same function. `None` only if the cache holds nothing of that function.
    fn fallback(&self, want: EntryKey) -> Option<u16> {
        let same = |k: &EntryKey| k.function == want.function && k.ruin == want.ruin;
        let mut best: Option<(i32, usize)> = None;
        for (i, k) in self.keys.iter().enumerate() {
            if !same(k) {
                continue;
            }
            // Lower is better: style mismatch costs 1000, storey difference 1.
            let mut cost = (k.storeys as i32 - want.storeys as i32).abs();
            if k.style != want.style {
                cost += 1000;
            }
            if k.footprint != want.footprint {
                cost += 100;
            }
            if best.map(|(c, _)| cost < c).unwrap_or(true) {
                best = Some((cost, i));
            }
        }
        best.map(|(_, i)| i as u16)
    }

    /// Fill in every lot's `kit` index.
    pub fn attach(&mut self, objects: &mut [CityObj], light: &Light) {
        let mut chosen = Placed::default();
        for o in objects.iter_mut() {
            o.kit = self.index_for_in(o, light, 0, Some(&mut chosen));
        }
    }

    /// As [`Self::attach`], with the ending's ruin code for each object, in
    /// the same order (B50). A short list means "intact" for the rest.
    pub fn attach_ruined(&mut self, objects: &mut [CityObj], light: &Light, ruin: &[u8]) {
        let mut chosen = Placed::default();
        for (i, o) in objects.iter_mut().enumerate() {
            o.kit =
                self.index_for_in(o, light, ruin.get(i).copied().unwrap_or(0), Some(&mut chosen));
        }
    }

    /// The spec a cached sprite was built for (test/report helper).
    pub fn spec_of(&self, i: usize) -> Option<&BuildingSpec> {
        self.specs.get(i)
    }

    /// A cached sprite (test/report helper).
    pub fn sprite_of(&self, i: usize) -> Option<&KitSprite> {
        self.sprites.get(i)
    }

    /// The footprint a cached sprite was built for (test/report helper).
    pub fn footprint_of(&self, i: usize) -> (u8, u8) {
        self.keys.get(i).map(|k| k.footprint).unwrap_or((1, 1))
    }

    /// Hand the sprites to a scene.
    pub fn into_sprites(self) -> Vec<KitSprite> {
        self.sprites
    }

    /// How many sprites are cached so far.
    pub fn sprite_count(&self) -> usize {
        self.sprites.len()
    }

    /// Copies of the sprites added since `from` (B34: a growing city only needs
    /// the new ones appended to the scene, not the whole set handed over again).
    pub fn sprites_from(&self, from: usize) -> Vec<KitSprite> {
        self.sprites.iter().skip(from).cloned().collect()
    }

    /// Bytes held by the sprite pixels (for the memory report).
    pub fn bytes(&self) -> usize {
        self.bytes_held
    }

    /// B76 r3: sprites with no surface snow can lie on (see
    /// [`CacheReport::unsnowed`]).
    pub fn unsnowed(&self) -> usize {
        self.unsnowed
    }

    /// B76 r3: the cached sprites whose skyline is mostly bare roof.
    pub fn bare_roofs(
        &self,
    ) -> &[(crate::BuildingFunction, Option<crate::BuildingType>, usize, usize)] {
        &self.bare_roofs
    }
}

/// Crop a plane to the pixels a sprite actually drew, keeping the anchor's
/// place relative to the image. Returns `(plane, w, h, ox, oy)`.
fn crop_plane(px: &[u32], plane: &[u8], w: usize, h: usize, ox: i32, oy: i32) -> (Vec<u8>, usize, usize, i32, i32) {
    let (mut x0, mut y0, mut x1, mut y1) = (w as i32, h as i32, -1i32, -1i32);
    for y in 0..h {
        for x in 0..w {
            if px[y * w + x] != 0 {
                x0 = x0.min(x as i32);
                x1 = x1.max(x as i32);
                y0 = y0.min(y as i32);
                y1 = y1.max(y as i32);
            }
        }
    }
    if x1 < x0 || y1 < y0 {
        return (vec![0], 1, 1, 0, 0);
    }
    let (nw, nh) = ((x1 - x0 + 1) as usize, (y1 - y0 + 1) as usize);
    let mut out = vec![0u8; nw * nh];
    for y in 0..nh {
        let src = (y0 as usize + y) * w + x0 as usize;
        out[y * nw..(y + 1) * nw].copy_from_slice(&plane[src..src + nw]);
    }
    (out, nw, nh, ox - x0, oy - y0)
}

/// Crop a sprite to the pixels it actually drew, keeping the anchor's place
/// relative to the image. Returns `(px, w, h, ox, oy)`.
fn crop(px: &[u32], w: usize, h: usize, ox: i32, oy: i32) -> (Vec<u32>, usize, usize, i32, i32) {
    let (mut x0, mut y0, mut x1, mut y1) = (w as i32, h as i32, -1i32, -1i32);
    for y in 0..h {
        for x in 0..w {
            if px[y * w + x] != 0 {
                x0 = x0.min(x as i32);
                x1 = x1.max(x as i32);
                y0 = y0.min(y as i32);
                y1 = y1.max(y as i32);
            }
        }
    }
    if x1 < x0 || y1 < y0 {
        // Nothing drawn: a 1x1 transparent sprite keeps the index valid.
        return (vec![0], 1, 1, 0, 0);
    }
    let (nw, nh) = ((x1 - x0 + 1) as usize, (y1 - y0 + 1) as usize);
    let mut out = vec![0u32; nw * nh];
    for y in 0..nh {
        let src = (y0 as usize + y) * w + x0 as usize;
        out[y * nw..(y + 1) * nw].copy_from_slice(&px[src..src + nw]);
    }
    (out, nw, nh, ox - x0, oy - y0)
}

/// The hand-made archetype the old `resolved_type` picked for a lot, so the
/// report's "before" pass draws the catalogue exactly as it was.
fn legacy_type(function: BuildingFunction, seed: u32) -> crate::BuildingType {
    use crate::BuildingType as T;
    let v = (seed % 4) as usize;
    match function {
        BuildingFunction::House => [
            T::NewEnglandCape,
            T::NewEnglandColonial,
            T::NewEnglandSaltbox,
            T::NewEnglandFarmhouse,
        ][v],
        BuildingFunction::Townhouse => [
            T::EuropeanHaussmann,
            T::EuropeanStuccoTownhouse,
            T::EuropeanCanalHouse,
            T::EuropeanOldTownHalfTimbered,
        ][v],
        BuildingFunction::Shop => [
            T::BrickShop2Storey,
            T::BrickBakeryCafe3Storey,
            T::BrickCornerStore4Storey,
            T::BrickBookstoreArcade,
        ][v],
        BuildingFunction::Apartment => T::APARTMENTS_8[(v + (seed as usize % 6)) % 8],
        BuildingFunction::Office => T::OFFICES_8[(v + (seed as usize % 6)) % 8],
        BuildingFunction::Industrial => [
            T::IndustrialBrickFactory,
            T::IndustrialWarehouse,
            T::IndustrialSilos,
            T::IndustrialPowerSubstation,
        ][v],
        BuildingFunction::Civic => [
            T::CivicSchool,
            T::CivicTownHall,
            T::CivicLibrary,
            T::CivicFireStation,
            T::CivicHospital,
        ][v % 5],
        // A tower keeps B73b's own split (signature or generated).
        BuildingFunction::Tower => T::TowerGenerated,
        BuildingFunction::Landmark => T::CivicGenerated,
    }
}

/// The kit's generated type for a function (B73c): every family but the
/// landmarks comes from the generator now.
fn generated_type(function: BuildingFunction) -> crate::BuildingType {
    use crate::BuildingType as T;
    match function {
        BuildingFunction::House => T::HouseGenerated,
        BuildingFunction::Townhouse => T::TownhouseGenerated,
        BuildingFunction::Shop => T::ShopGenerated,
        BuildingFunction::Apartment => T::ApartmentGenerated,
        BuildingFunction::Office => T::OfficeGenerated,
        BuildingFunction::Tower => T::TowerGenerated,
        BuildingFunction::Industrial => T::IndustrialGenerated,
        BuildingFunction::Civic => T::CivicGenerated,
        // A landmark is hand-made art; this is never reached for one.
        BuildingFunction::Landmark => T::CivicGenerated,
    }
}

/// The buildkit spec for a lot, or `None` when the engine's own art should draw
/// it (parks, construction sites, barns).
pub fn spec_for(o: &CityObj) -> Option<BuildingSpec> {
    // B35: tall buildings and landmarks go up through the kit's construction
    // stages (excavation, foundation, framing with a tower crane, cladding);
    // cottages keep the engine's own stage art, which is cheaper and reads fine.
    let stage = kit_stage(o.stage);
    // B43c: every plot under construction uses the engine's site art (frame,
    // scaffold, cranes), so a tall tower and a cottage read alike.
    if stage != ConstructionStage::Finished {
        return None;
    }
    use crate::BuildingType as T;
    use render::city::LandmarkKind as R;
    let (function, landmark, forced) = match o.kind {
        // B39: the settlers' log cabin is engine art (the catalogue has none).
        ObjKind::LogCabin => return None,
        ObjKind::Park | ObjKind::Cemetery => return None,
        // B59: the car places are engine art too (a forecourt, a car park).
        ObjKind::CarPark
        | ObjKind::CarParkDeck
        | ObjKind::GasStation
        | ObjKind::ChargingHub
        | ObjKind::DriveThrough
        | ObjKind::CarWash
        | ObjKind::Dealership
        | ObjKind::Motel => return None,
        ObjKind::Landmark(R::Plaza) => return None,
        // B62: the zoo and the castle are engine art.
        ObjKind::Landmark(R::Zoo | R::Castle | R::Courthouse | R::PoliceStation | R::PostOffice) => return None,
        // The civic four are catalogue buildings, not "landmark" archetypes.
        ObjKind::Landmark(R::School) => (BuildingFunction::Civic, None, Some(T::CivicSchool)),
        ObjKind::Landmark(R::Library) => (BuildingFunction::Civic, None, Some(T::CivicLibrary)),
        ObjKind::Landmark(R::FireStation) => {
            (BuildingFunction::Civic, None, Some(T::CivicFireStation))
        }
        ObjKind::Landmark(R::Hospital) => (BuildingFunction::Civic, None, Some(T::CivicHospital)),
        // B52: the government district. The kit's civic catalogue has no court,
        // police station or post office of its own, so each wears the nearest
        // archetype and the renderer's civic dressing (columns, flags, the blue
        // lamp, the mail box) makes them read as what they are.
        ObjKind::Landmark(R::Courthouse) => (BuildingFunction::Civic, None, Some(T::CivicTownHall)),
        ObjKind::Landmark(R::PoliceStation) => (BuildingFunction::Civic, None, Some(T::CivicFireStation)),
        ObjKind::Landmark(R::PostOffice) => (BuildingFunction::Civic, None, Some(T::CivicLibrary)),
        ObjKind::Landmark(R::Cathedral) => {
            (BuildingFunction::Landmark, Some(LandmarkKind::Cathedral), None)
        }
        ObjKind::Landmark(R::Station) => (
            BuildingFunction::Landmark,
            Some(LandmarkKind::TrainStation),
            None,
        ),
        ObjKind::Landmark(R::Museum) => (BuildingFunction::Landmark, Some(LandmarkKind::Museum), None),
        ObjKind::Landmark(R::Stadium) => {
            (BuildingFunction::Landmark, Some(LandmarkKind::Stadium), None)
        }
        ObjKind::Landmark(R::SignatureTower) => (
            BuildingFunction::Landmark,
            Some(LandmarkKind::SignatureTower),
            None,
        ),
        ObjKind::Landmark(R::Lighthouse) => (
            BuildingFunction::Landmark,
            Some(LandmarkKind::Lighthouse),
            None,
        ),
        ObjKind::Landmark(R::Port) => (
            BuildingFunction::Landmark,
            Some(LandmarkKind::ContainerPort),
            None,
        ),
        ObjKind::Landmark(R::FerrisWheel) => (
            BuildingFunction::Landmark,
            Some(LandmarkKind::FerrisWheel),
            None,
        ),
        // B80: a farmstead's house (lot style 6) is the kit's farmhouse.
        ObjKind::House if o.style == 6 => (BuildingFunction::House, None, Some(T::NewEnglandFarmhouse)),
        ObjKind::House => (BuildingFunction::House, None, None),
        ObjKind::Townhouse => (BuildingFunction::Townhouse, None, None),
        ObjKind::Apartment => (BuildingFunction::Apartment, None, None),
        ObjKind::Shop | ObjKind::Store => (BuildingFunction::Shop, None, None),
        ObjKind::Office => (BuildingFunction::Office, None, None),
        ObjKind::Tower => (BuildingFunction::Tower, None, None),
        // B80: a barn is the engine's own art (red walls, gambrel roof, silo).
        ObjKind::Barn => return None,
        ObjKind::Industry => (BuildingFunction::Industrial, None, None),
        // B73c: the village church and the hall keep their program; the kit
        // generates their architecture.
        ObjKind::Church => (BuildingFunction::Civic, None, Some(T::CivicChurch)),
        ObjKind::Hall => (BuildingFunction::Civic, None, Some(T::CivicTownHall)),
    };
    let style = match o.style {
        1 => BuildingStyle::European,
        2 => BuildingStyle::Suburban,
        3 => BuildingStyle::ModernGlass,
        4 => BuildingStyle::HighEnd,
        5 => BuildingStyle::Industrial,
        _ => {
            if o.storeys >= 8 {
                BuildingStyle::ModernGlass
            } else if o.storeys <= 2 {
                BuildingStyle::NewEngland
            } else {
                BuildingStyle::BrickMainStreet
            }
        }
    };
    let era = match o.storeys {
        0 | 1 => Era::Village,
        2..=4 => Era::Town,
        5..=9 => Era::City,
        _ => Era::Metropolis,
    };
    // B78 (D84): age now comes from the sim — the lot's own years, not a dice
    // roll. `Wear::grime` is what shows (the streak/moss pass in `render::wear`);
    // `repaint` and `fascia` change the *design*: a renovated building comes back
    // with a new paint scheme, an extension or a loft (a new seed), and a shop
    // that changed hands gets a new fascia and sign.
    // The sprite itself stays crisp: all the weathering (streaks, soot, moss,
    // paint fade) happens once, in `render::wear`, so an aged city is one system
    // rather than two that disagree.
    let age = 0.0f32;
    // Every generated family gets the lot's own seed (position, height and the
    // sim's seed folded in), so two lots are two buildings; `attach*` steps a
    // clashing neighbour's seed (see `genkit::nudge_seed`). The street theme is
    // a coarser hash — the block the lot stands on — so a row shares a paint and
    // a roof family but no two houses are the same building (brief §4).
    let legacy = kit_legacy();
    let is_tower = function == BuildingFunction::Tower;
    let seed = if legacy && !is_tower {
        (o.seed as u64 % 6) * 0x9E37_79B9
    } else {
        (o.seed as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
            ^ ((o.x as u64) << 22)
            ^ ((o.y as u64) << 7)
            ^ ((o.storeys as u64) << 44)
    };
    // A repaint or a new fascia is a new design for the same lot: fold the count
    // into the seed so the kit's generator picks a different scheme (and, for a
    // house, now and then a slightly different massing — the loft/extension).
    let seed = seed
        ^ (o.wear.repaint as u64).wrapping_mul(0x2545_F491_4F6C_DD1D)
        ^ (o.wear.fascia as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    let block = (o.x.div_euclid(6), o.y.div_euclid(5));
    let theme = (crate::color::hash2(block.0, block.1) % 251) as u8;
    // The archetype a lot draws: hand-made art for the landmarks and the rare
    // signature towers, the kit for everything else (B73c).
    let building_type = if legacy {
        if is_tower {
            let h = crate::color::hash_seed(o.seed as u64, 0x51);
            if h % 14 == 0 {
                Some(crate::BuildingType::TOWERS_16[(h / 14) as usize % 16])
            } else {
                None
            }
        } else {
            forced.or_else(|| {
                if landmark.is_some() {
                    None
                } else {
                    // The pre-B73c catalogue: every family a hand-made
                    // archetype, chosen the way `resolved_type` used to.
                    Some(legacy_type(function, o.seed))
                }
            })
        }
    } else if is_tower {
        let h = crate::color::hash_seed(o.seed as u64, 0x51);
        if h % 14 == 0 {
            Some(crate::BuildingType::TOWERS_16[(h / 14) as usize % 16])
        } else {
            None
        }
    } else if landmark.is_some() {
        None
    } else {
        Some(forced.unwrap_or_else(|| generated_type(function)))
    };
    Some(BuildingSpec {
        // The catalogue draws each archetype at its own size and ignores
        // `spec.footprint` (B32b finding), so a shallow lot — a 2x1 house plot —
        // must be given a shallow archetype or the building overhangs its
        // neighbours. 1.15x1.05 and 2.10x1.05 archetypes are the shallow ones.
        building_type,
        function,
        style,
        footprint: (o.w.clamp(1, 255) as u8, o.h.clamp(1, 255) as u8),
        storeys: o.storeys.max(1),
        era,
        age,
        seed,
        variant: if legacy { (o.seed % 4) as u8 } else { theme },
        landmark_kind: landmark,
        stage,
        // Set by the cache that asks for the sprite (D66 item 4).
        biome: crate::style::BiomeStyle::Temperate,
    })
}

/// What kind of night light a catalogue pixel is (B36b).
#[derive(Clone, Copy, PartialEq, Eq)]
enum NightLight {
    Warm,
    Cool,
    Neon,
}

/// Classify a night pixel: a warm tungsten pane (and its shaded variants), a cool
/// LED/TV pane, or a neon sign. Everything else is wall and gets tinted.
fn night_light(c: u32) -> Option<NightLight> {
    let (r, g, b) = ((c >> 16) & 0xFF, (c >> 8) & 0xFF, c & 0xFF);
    // Warm tungsten: 0xffd27e and the shades the catalogue paints around it.
    // The bar is high on purpose: tan walls and brass trim must not glow (B38:
    // the dark towers' grey cross-bracing lit up at night).
    if r >= 0xd0 && g >= 0x90 && g <= 0xe8 && b <= 0x9c && r > b + 0x48 {
        return Some(NightLight::Warm);
    }
    // Cool LED/TV: 0xc8e6ff and its shades — bright and strongly blue, not a
    // blue-grey steel member.
    if b >= 0xd8 && g >= 0xc0 && r >= 0x98 && b > r + 0x30 {
        return Some(NightLight::Cool);
    }
    // Neon signs (color.rs NEON_*).
    const NEON: [(u32, u32, u32); 4] = [
        (0xff, 0x3e, 0xc8),
        (0x26, 0xe0, 0xff),
        (0xff, 0x9a, 0x3c),
        (0x36, 0xe6, 0x78),
    ];
    if NEON.contains(&(r, g, b)) {
        return Some(NightLight::Neon);
    }
    None
}

/// The kit's construction stage for a sim stage (B35/D33).
fn kit_stage(s: Stage) -> ConstructionStage {
    match s {
        Stage::Staked | Stage::Graded => ConstructionStage::Excavation,
        Stage::Foundation => ConstructionStage::Foundation,
        Stage::Framing | Stage::Scaffold => ConstructionStage::Framing,
        Stage::Roofed => ConstructionStage::Cladding,
        Stage::Finished => ConstructionStage::Finished,
    }
}

pub fn set_city(scene: &mut render::Scene, objects: Vec<CityObj>, light: &Light) -> CacheReport {
    set_city_in(scene, objects, light, crate::style::BiomeStyle::Temperate)
}

/// As [`set_city`], in a land's vernacular (D66 item 4).
pub fn set_city_in(
    scene: &mut render::Scene,
    mut objects: Vec<CityObj>,
    light: &Light,
    biome: crate::style::BiomeStyle,
) -> CacheReport {
    let mut cache = KitCache::with_style(biome);
    cache.attach(&mut objects, light);
    let report = CacheReport {
        sprites: cache.sprites.len(),
        bytes: cache.bytes(),
        hits: cache.hits,
        builds: cache.builds,
        reused: cache.reused,
        unsnowed: cache.unsnowed(),
        bare_roofs: cache.bare_roofs().to_vec(),
    };
    // One rebuild for sprites + city (B34: this used to be three).
    scene.set_city_with_sprites(cache.into_sprites(), objects);
    report
}

/// As [`set_city_in`], with the ending's ruin code for each lot (B50).
pub fn set_city_ruined(
    scene: &mut render::Scene,
    mut objects: Vec<CityObj>,
    light: &Light,
    biome: crate::style::BiomeStyle,
    ruin: &[u8],
) -> CacheReport {
    let mut cache = KitCache::with_style(biome);
    cache.attach_ruined(&mut objects, light, ruin);
    let report = CacheReport {
        sprites: cache.sprites.len(),
        bytes: cache.bytes(),
        hits: cache.hits,
        builds: cache.builds,
        reused: cache.reused,
        unsnowed: cache.unsnowed(),
        bare_roofs: cache.bare_roofs().to_vec(),
    };
    scene.set_city_with_sprites(cache.into_sprites(), objects);
    report
}

/// What the cache did, for the handback and the debug line.
#[derive(Clone, Debug, Default)]
pub struct CacheReport {
    pub sprites: usize,
    pub bytes: usize,
    pub hits: usize,
    pub builds: usize,
    /// Lots that reused another lot's sprite (cache full).
    pub reused: usize,
    /// B76 r3: sprites with no surface snow can lie on at all (their roof is a
    /// spire, a lattice or bare structure). The report keeps them honest.
    pub unsnowed: usize,
    /// B76 r3: sprites whose skyline is mostly not a snow surface — the roofs
    /// that would stay bare in winter. `(function, type, bare tops, tops)`.
    pub bare_roofs: Vec<(crate::BuildingFunction, Option<crate::BuildingType>, usize, usize)>,
}

/// The buildkit light for the scene's ambient state.
/// The window glow at which the evening's materials switch (see [`light_for`]).
/// Two steps only: a sprite set per step costs memory, and the frame's own
/// ambient runs continuously between them.
pub const DUSK_GLOW: f32 = 0.30;
pub const NIGHT_GLOW: f32 = 0.93;

pub fn light_for(tint: [f32; 3], glow: f32, hour: f32) -> Light {
    let small_hours = (0.0..5.0).contains(&hour);
    // B99d: the night skin waits for the sky to be genuinely dark (the sun down
    // about 6 degrees), not for the glow to pass a half: at 0.72 the sun was
    // still up in a November evening and the city had already switched over.
    let bucket = if glow > NIGHT_GLOW && !small_hours {
        Light::NIGHT_BUCKET
    } else if glow > NIGHT_GLOW {
        // B38: 00:00-05:00 — night, but most windows have gone dark.
        3
    } else if glow > DUSK_GLOW {
        Light::SUNSET_BUCKET
    } else {
        Light::DAY_BUCKET
    };
    // B99d: night *materials* belong to the night buckets, not to "the glow has
    // passed a half". The old rule flipped a tower's glass to its night skin at
    // glow 0.5 while the sky was still an hour off sunset, and the dusk ambient
    // then turned that dark skin flat brown. `night` carries the fade itself, so
    // the glass darkens and the panes come on across the buckets instead of
    // snapping.
    let night = match bucket {
        Light::DAY_BUCKET => 0.0,
        Light::SUNSET_BUCKET => 0.30,
        _ => 1.0,
    };
    Light {
        bucket,
        is_night: bucket == Light::NIGHT_BUCKET || bucket == 3,
        night,
        tint,
        hour,
    }
}

#[cfg(test)]
mod evening_tests {
    use super::*;

    /// B99d: night *materials* belong to the night buckets. The old rule flipped
    /// them at glow 0.5, which is still an hour before sunset in November, and
    /// the dusk ambient then turned the dark skin flat brown.
    #[test]
    fn night_materials_wait_for_the_night_bucket() {
        let day = light_for([1.0, 1.0, 1.0], 0.05, 12.0);
        assert_eq!(day.bucket, Light::DAY_BUCKET);
        assert_eq!(day.night, 0.0);
        assert!(!day.is_night);

        // An hour before sunset: the dusk bucket, lit a little, not night-skinned.
        let dusk = light_for([1.0, 1.0, 1.0], 0.60, 16.0);
        assert_eq!(dusk.bucket, Light::SUNSET_BUCKET);
        assert!(!dusk.is_night, "dusk must not wear the night skin");
        assert!(dusk.night > 0.0 && dusk.night < 1.0, "dusk is part-way: {}", dusk.night);

        // After sunset: the night bucket.
        let night = light_for([1.0, 1.0, 1.0], 0.99, 18.0);
        assert_eq!(night.bucket, Light::NIGHT_BUCKET);
        assert!(night.is_night);
        assert_eq!(night.night, 1.0);
    }

    #[test]
    fn the_night_factor_never_goes_backwards() {
        let mut prev = -1.0;
        let mut glow = 0.0f32;
        while glow <= 1.0 {
            let l = light_for([1.0, 1.0, 1.0], glow, 21.0);
            assert!(l.night >= prev, "night went backwards at glow {glow}: {prev} -> {}", l.night);
            prev = l.night;
            glow += 0.02;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use render::city::Stage;
    use render::scene::Scene;

    fn lot(x: i32, y: i32, w: i32, h: i32, kind: ObjKind, storeys: u8, style: u8, seed: u32) -> CityObj {
        CityObj {
            x,
            y,
            w,
            h,
            level: 4,
            kind,
            storeys,
            style,
            stage: Stage::Finished,
            seed,
            front: 0,
            t: 0.0,
            kit: 0, wear: Default::default(),
        }
    }

    /// A day-180-ish downtown: towers, offices, apartments, shops and houses,
    /// plus a park and two sites (which must keep the engine's stage art).
    fn city() -> Vec<CityObj> {
        let mut v = Vec::new();
        let mut seed = 1u32;
        // Laid out inside the visible diamond of the 1920x1080 test scene.
        for i in 0..7 {
            let x = 20 + i * 4;
            v.push(lot(x, 20, 3, 3, ObjKind::Tower, 14 + (i % 6) as u8, 3, seed));
            seed += 7;
            v.push(lot(x, 24, 2, 2, ObjKind::Office, 8 + (i % 4) as u8, 3, seed));
            seed += 5;
            v.push(lot(x, 27, 2, 2, ObjKind::Apartment, 5, 1, seed));
            seed += 3;
            v.push(lot(x, 30, 2, 2, ObjKind::Shop, 3, 0, seed));
            seed += 2;
            v.push(lot(x, 33, 2, 1, ObjKind::House, 2, 2, seed));
            seed += 1;
        }
        v.push(lot(22, 34, 2, 2, ObjKind::Park, 0, 0, 99));
        let mut site = lot(28, 34, 2, 2, ObjKind::House, 2, 2, 42);
        site.stage = Stage::Framing;
        v.push(site);
        let mut site2 = lot(34, 34, 3, 3, ObjKind::Tower, 12, 3, 43);
        site2.stage = Stage::Foundation;
        v.push(site2);
        v
    }

    /// B32b: the regression was a *mix* of art systems — most lots fell back to
    /// the engine's massing (whose flat cap floated above the kit buildings),
    /// while a few drew a kit sprite. Every finished building must get exactly
    /// one kit sprite, and nothing else may be drawn for it.
    ///
    /// B43c: every plot under construction draws the engine's site art.
    #[test]
    fn every_finished_lot_gets_exactly_one_sprite() {
        let mut objs = city();
        let mut cache = KitCache::new();
        let light = crate::Light::day();
        cache.attach(&mut objs, &light);
        assert_eq!(cache.reused, 0, "the cache overflowed for a normal city");
        let mut engine_art = 0usize;
        for o in &objs {
            if o.stage == Stage::Finished && o.kind != ObjKind::Park {
                assert!(o.kit != 0, "finished lot at {},{} got no kit sprite", o.x, o.y);
            } else {
                // B43c: every site, tall or short, is the engine's site art.
                assert_eq!(o.kit, 0, "a site or park must keep the engine's art");
                engine_art += 1;
            }
        }
        let sprites = cache.into_sprites();
        let with_sprite = objs.iter().filter(|o| o.kit != 0).count();

        let mut scene = Scene::new(render::SceneConfig {
            width: 1920,
            height: 1080,
            seed: 3,
            mode: render::SceneMode::Sim,
            cars: 0,
            peds: 0,
            buildings: 0,
            rain: false,
        });
        scene.set_kit_sprites(sprites.clone());
        scene.set_city(objs);
        scene.flush_dirty();
        // One blit per lot with a sprite; the engine's own art only for the
        // park and the short site.
        assert_eq!(
            scene.stats.kit_draws, with_sprite,
            "expected {with_sprite} kit blits, got {}",
            scene.stats.kit_draws
        );
        assert_eq!(
            scene.stats.engine_draws, engine_art,
            "engine art must only draw the park and short sites, got {}",
            scene.stats.engine_draws
        );
    }

    /// No sprite may carry a detached roof. The catalogue's H32 rule was a
    /// column scan; B73c's parts (an eave, a chimney, a porch roof, a civic
    /// spire) make a column scan ambiguous, so the rule is stated as what it
    /// means: everything a building draws above the ground is one connected
    /// piece (8-connected, so an eave that touches its wall on the diagonal is
    /// still attached), and no more than a few stray pixels are loose.
    #[test]
    fn no_sprite_has_a_floating_roof() {
        let objs = city();
        let light = crate::Light::day();
        let mut specs: Vec<BuildingSpec> = Vec::new();
        for o in &objs {
            if let Some(sp) = spec_for(o) {
                if specs.iter().any(|x| x == &sp) {
                    continue;
                }
                specs.push(sp);
            }
        }
        assert!(specs.len() > 10, "the fixture must cover every family");
        for (i, spec) in specs.iter().enumerate() {
            let mut s = crate::Sprite::new(1, 1);
            crate::draw(spec, &light, &mut s);
            let (w, h) = (s.w, s.h);
            // The building itself: above the lot's ground plot (depth 0).
            let on = |x: usize, y: usize| s.px[y * w + x] != 0 && s.depth[y * w + x] > 0;
            let mut seen = vec![false; w * h];
            let mut components: Vec<(usize, Vec<(usize, usize)>)> = Vec::new();
            for y in 0..h {
                for x in 0..w {
                    if seen[y * w + x] || !on(x, y) {
                        continue;
                    }
                    let mut stack = vec![(x, y)];
                    seen[y * w + x] = true;
                    let mut px = Vec::new();
                    while let Some((cx, cy)) = stack.pop() {
                        px.push((cx, cy));
                        for dy in -1i32..=1 {
                            for dx in -1i32..=1 {
                                let (nx, ny) = (cx as i32 + dx, cy as i32 + dy);
                                if nx < 0 || ny < 0 || nx as usize >= w || ny as usize >= h {
                                    continue;
                                }
                                let (nx, ny) = (nx as usize, ny as usize);
                                if !seen[ny * w + nx] && on(nx, ny) {
                                    seen[ny * w + nx] = true;
                                    stack.push((nx, ny));
                                }
                            }
                        }
                    }
                    components.push((px.len(), px));
                }
            }
            components.sort_by(|a, b| b.0.cmp(&a.0));
            let total: usize = components.iter().map(|(n, _)| n).sum();
            // A separate piece is allowed only if it stands on the ground in
            // its own right (a detached garage or a garden outbuilding); a part
            // that hangs in the air attached to nothing is a detached roof.
            let floating: Vec<(usize, (usize, usize))> = components
                .iter()
                .skip(1)
                .filter(|(n, px)| *n >= 8 && !px.iter().any(|&(x, y)| s.depth[y * w + x] <= 8))
                .map(|(n, px)| (*n, px[0]))
                .collect();
            assert!(
                floating.is_empty(),
                "spec {i}: {total} drawn px, floating parts {floating:?} ({spec:?})"
            );
            assert!(total > 200, "spec {i} drew only {total} px ({spec:?})");
        }
    }

    /// The sprite's anchor must be the footprint's tile origin: a building's
    /// lowest drawn pixel has to sit at or above the anchor row, and the anchor
    /// column has to fall inside the drawn footprint.
    #[test]
    fn sprite_anchor_is_the_lot_origin() {
        let mut objs = city();
        let mut cache = KitCache::new();
        cache.attach(&mut objs, &crate::Light::day());
        let footprints: Vec<(u8, u8)> = (0..cache.sprites.len()).map(|i| cache.footprint_of(i)).collect();
        let sprites = cache.into_sprites();
        for (i, s) in sprites.iter().enumerate() {
            let (fw, fh) = footprints[i];
            assert!((0..s.w as i32).contains(&s.ox), "sprite {i}: ox {} outside 0..{}", s.ox, s.w);
            assert!((0..=s.h as i32).contains(&s.oy), "sprite {i}: oy {} outside 0..{}", s.oy, s.h);
            // The anchor is the footprint's near (north) corner, so the drawn
            // ground diamond reaches at most (fw + fh) * 16 px below it — and
            // nothing at all above it.
            let bottom = (0..s.h).rev().find(|y| (0..s.w).any(|x| s.px[y * s.w + x] != 0));
            if let Some(b) = bottom {
                let limit = (fw as i32 + fh as i32) * 16 + 6;
                assert!(
                    b as i32 <= s.oy + limit,
                    "sprite {i}: draws {} px below its anchor (limit {limit})",
                    b as i32 - s.oy
                );
            }
        }
    }
}


#[cfg(test)]
mod landmark_tests {
    use super::*;
    use render::city::{LandmarkKind as R, ObjKind, Stage};

    fn lot(kind: ObjKind, w: i32, h: i32, storeys: u8) -> CityObj {
        CityObj {
            x: 20,
            y: 20,
            w,
            h,
            level: 4,
            kind,
            storeys,
            style: 0,
            stage: Stage::Finished,
            seed: 7,
            front: 0,
            t: 0.0,
            kit: 0, wear: Default::default(),
        }
    }

    /// B35: every landmark must produce a spec and a sprite with pixels in it.
    #[test]
    fn every_landmark_draws() {
        let kinds = [
            (R::Cathedral, 3, 4, 4),
            (R::Station, 5, 3, 2),
            (R::School, 4, 3, 3),
            (R::Library, 3, 3, 3),
            (R::FireStation, 3, 3, 2),
            (R::Hospital, 5, 4, 6),
            (R::Museum, 4, 3, 3),
            (R::Stadium, 6, 5, 4),
            (R::SignatureTower, 3, 3, 22),
            (R::Lighthouse, 2, 2, 3),
            (R::Port, 5, 4, 1),
            (R::FerrisWheel, 3, 3, 4),
        ];
        let mut cache = KitCache::new();
        for (k, w, h, st) in kinds {
            let o = lot(ObjKind::Landmark(k), w, h, st);
            let spec = spec_for(&o).unwrap_or_else(|| panic!("{k:?}: no spec"));
            let mut s = crate::Sprite::new(1, 1);
            crate::draw(&spec, &crate::Light::day(), &mut s);
            let set = s.px.iter().filter(|c| **c != 0).count();
            assert!(set > 40, "{k:?}: sprite has only {set} pixels ({}x{})", s.w, s.h);
            let _ = cache.index_for(&o, &crate::Light::day());
        }
        assert_eq!(cache.sprite_count(), 12, "one sprite per landmark");
        // The plaza is engine art, not a kit building.
        assert!(spec_for(&lot(ObjKind::Landmark(R::Plaza), 5, 5, 0)).is_none());
    }
}

#[cfg(test)]
mod plaza_scene_test {
    use super::*;
    use render::city::{LandmarkKind as R, ObjKind, Stage};

    #[test]
    fn the_plaza_reaches_the_frame() {
        let mut scene = render::Scene::new(render::SceneConfig {
            width: 1920,
            height: 1080,
            seed: 3,
            mode: render::SceneMode::Sim,
            cars: 0,
            peds: 0,
            buildings: 0,
            rain: false,
        });
        let cx = scene.viewport.w / 2;
        let cy = scene.viewport.h / 2;
        let objs = vec![CityObj {
            x: cx - 2,
            y: cy - 2,
            w: 5,
            h: 5,
            level: scene.map.at(cx, cy).max_level(),
            kind: ObjKind::Landmark(R::Plaza),
            storeys: 0,
            style: 0,
            stage: Stage::Finished,
            seed: 7,
            front: 0,
            t: 0.0,
            kit: 0, wear: Default::default(),
        }];
        let mut before = render::Framebuffer::new(1920, 1080);
        scene.composite(&mut before);
        scene.set_city(objs);
        let mut after = render::Framebuffer::new(1920, 1080);
        scene.composite(&mut after);
        let diff = before.px().iter().zip(after.px()).filter(|(a, b)| a != b).count();
        assert!(diff > 2000, "the plaza changed only {diff} pixels");
    }
}

#[cfg(test)]
mod scene_landmark_test {
    use super::*;
    use render::city::{LandmarkKind as R, ObjKind, Stage};

    #[test]
    fn a_landmark_reaches_the_frame() {
        let mut scene = render::Scene::new(render::SceneConfig {
            width: 1920,
            height: 1080,
            seed: 3,
            mode: render::SceneMode::Sim,
            cars: 0,
            peds: 0,
            buildings: 0,
            rain: false,
        });
        let mut before = render::Framebuffer::new(1920, 1080);
        scene.composite(&mut before);
        let cx = scene.viewport.w / 2;
        let cy = scene.viewport.h / 2;
        let mut objs = vec![CityObj {
            x: cx - 2,
            y: cy - 2,
            w: 4,
            h: 3,
            level: scene.map.at(cx, cy).max_level(),
            kind: ObjKind::Landmark(R::School),
            storeys: 3,
            style: 0,
            stage: Stage::Finished,
            seed: 7,
            front: 0,
            t: 0.0,
            kit: 0, wear: Default::default(),
        }];
        let mut cache = KitCache::new();
        cache.attach(&mut objs, &crate::Light::day());
        assert_ne!(objs[0].kit, 0, "no sprite index");
        assert_eq!(scene.stats.engine_draws, 0, "engine drew nothing before the city");
        scene.set_city_with_sprites(cache.into_sprites(), objs);
        let mut after = render::Framebuffer::new(1920, 1080);
        scene.composite(&mut after);
        let diff = before.px().iter().zip(after.px()).filter(|(a, b)| a != b).count();
        assert!(diff > 400, "the landmark changed only {diff} pixels");
    }
}

#[cfg(test)]
mod dump_cross {
    #[test]
    fn dump_crossbraced() {
        use crate::types::*;
        let spec = BuildingSpec {
            building_type: Some(BuildingType::TowerCrossBraced),
            function: BuildingFunction::Tower,
            landmark_kind: None,
            style: BuildingStyle::ModernGlass,
            footprint: (3, 3),
            storeys: 14,
            era: Era::City,
            stage: ConstructionStage::Finished,
            age: 0.0,
            seed: 0x9E37_79B9,
            variant: 1,
            biome: crate::style::BiomeStyle::Temperate,
        };
        let mut sp = crate::Sprite::new(1, 1);
        crate::draw(&spec, &crate::light_for([1.0,1.0,1.0], 1.0, 21.0), &mut sp);
        let mut v: Vec<(u32, usize)> = Vec::new();
        for c in sp.px.iter().filter(|c| **c != 0) {
            if let Some(x) = v.iter_mut().find(|(k, _)| k == c) { x.1 += 1; } else { v.push((*c, 1)); }
        }
        v.sort_by_key(|(c, _)| {
            let l = ((c >> 16) & 0xFF) + ((c >> 8) & 0xFF) + (c & 0xFF);
            std::cmp::Reverse(l)
        });
        for (c, n) in v.iter().take(10) {
            eprintln!("#{:06x} x{}", c & 0xFFFFFF, n);
        }
    }
}
