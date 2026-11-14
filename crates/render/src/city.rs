//! City objects the renderer draws on top of the terrain: plots at every
//! construction stage, civic buildings, barns.
//!
//! The renderer must not depend on `sim`, so it takes this flat struct; the
//! spike converts. Anything the sim puts *into the map* (lanes, pavements,
//! fields, walls, bridges) needs no conversion at all.
//!
//! B9: village houses are **detached with yards** — the footprint is inset
//! inside the lot (≈65 % of it), set back from the street, with a front path —
//! and the roofs are pitched with a ridge, a lit and a shaded slope and 1 px
//! eaves, because at 1× a flat box reads as a dark slab.

use crate::framebuffer::Framebuffer;
use crate::iso::{Camera, HEIGHT_UNIT, Pt};
use crate::palette::{self, Color, pal};
use crate::tex;

mod places;
mod site;

pub use places::casters as place_casters;

/// Art detail multiplier (2 at 64-px tiles).
const D: i32 = map::iso_consts::DETAIL;

/// B62: where the square's clock tower stands, in tiles: the far (back) corner.
pub fn clock_tower_spot(x0: f32, y0: f32, w: f32, h: f32) -> (f32, f32) {
    (x0 + w * 0.17 + 0.2, y0 + h * 0.17 + 0.2)
}

/// Mirrors `sim::landmarks::LandmarkKind` without the dependency (B35).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LandmarkKind {
    Cathedral,
    Station,
    School,
    Library,
    FireStation,
    Hospital,
    Museum,
    Stadium,
    SignatureTower,
    Lighthouse,
    Port,
    FerrisWheel,
    Plaza,
    /// B62: the zoo (a paddocked park, engine art).
    Zoo,
    /// B62: a castle on the high ground, for old or fantasy-leaning worlds.
    Castle,
    /// B52: the civic centre's other buildings.
    Courthouse,
    PoliceStation,
    PostOffice,
}

impl LandmarkKind {
    /// Landmarks the engine draws itself when the kit has no sprite for them
    /// (the civic square is not a building at all).
    pub fn is_open_space(self) -> bool {
        matches!(self, LandmarkKind::Plaza)
    }
}

/// Mirrors `sim::LotKind` without the dependency.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ObjKind {
    House,
    Church,
    Store,
    Shop,
    Barn,
    /// B26's placeholder massing: boxes with window rows per storey.
    Townhouse,
    Apartment,
    Office,
    Tower,
    Industry,
    Park,
    /// The graveyard (B48): a small walled lawn with headstones.
    Cemetery,
    Hall,
    /// A landmark (B35): a big plot, drawn from the building kit.
    Landmark(LandmarkKind),
    /// The settlers' log cabin (B39): engine art, logs → walls → roof.
    LogCabin,
    /// B59: the places cars need. All engine art, drawn inside the plot.
    CarPark,
    CarParkDeck,
    GasStation,
    ChargingHub,
    DriveThrough,
    CarWash,
    Dealership,
    Motel,
}

impl ObjKind {
    /// B59: one of the places cars need (their parked cars stand on the plot).
    pub fn is_car_place(self) -> bool {
        car_place_kind(self).is_some()
    }

    /// The B26 types are drawn as massing, not as the cottage kit.
    pub fn is_massing(self) -> bool {
        matches!(
            self,
            ObjKind::Townhouse
                | ObjKind::Apartment
                | ObjKind::Office
                | ObjKind::Tower
                | ObjKind::Industry
                | ObjKind::Park
                | ObjKind::Cemetery
                | ObjKind::Hall
                | ObjKind::Landmark(_)
                // B59: the car places are drawn as their own plots, in the
                // static layer like a park.
                | ObjKind::CarPark
                | ObjKind::CarParkDeck
                | ObjKind::GasStation
                | ObjKind::ChargingHub
                | ObjKind::DriveThrough
                | ObjKind::CarWash
                | ObjKind::Dealership
                | ObjKind::Motel
        )
    }
}

/// Construction stage, in order. `Finished` is a standing building.
#[derive(Clone, Copy, PartialEq, Eq, Debug, PartialOrd, Ord)]
pub enum Stage {
    Staked,
    Graded,
    Foundation,
    Framing,
    Scaffold,
    Roofed,
    Finished,
}

impl Stage {
    pub const ALL: [Stage; 7] = [
        Stage::Staked,
        Stage::Graded,
        Stage::Foundation,
        Stage::Framing,
        Stage::Scaffold,
        Stage::Roofed,
        Stage::Finished,
    ];

    pub fn from_index(i: u8) -> Stage {
        match i {
            0 => Stage::Staked,
            1 => Stage::Graded,
            2 => Stage::Foundation,
            3 => Stage::Framing,
            4 => Stage::Scaffold,
            5 => Stage::Roofed,
            _ => Stage::Finished,
        }
    }
}

pub const STAGE_NAMES: [&str; 7] = [
    "staked",
    "graded",
    "foundation",
    "framing",
    "scaffold",
    "roofed",
    "finished",
];

/// B78 (D84): what the years, the weather and the street have done to one
/// building. Computed by the sim (`sim::ageing`) from the lot's own age and the
/// town's fortunes; drawn into the static layer by `crate::wear`, so it costs
/// nothing per frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Wear {
    /// 0 = crisp and new, 1 = a few years, 2 = old, 3 = run-down.
    pub grime: u8,
    /// A wet land: moss and lichen on the roof, greener streaks.
    pub wet: bool,
    /// 0 = a blank wall, 1 = spray tags, 2 = a painted mural.
    pub art: u8,
    /// Which tag or mural: an invented piece, per wall.
    pub art_seed: u32,
    /// Plywood over every window: the building is empty and waiting.
    pub boarded: bool,
    /// A shop with its shutters down and a TO LET board.
    pub shut: bool,
    /// Scaffolding and a crew: a renovation under way.
    pub scaffold: bool,
    /// Completed renovations (0..=3): the building has been repainted, so the
    /// sprite is regenerated with a new paint scheme (and the odd extension).
    pub repaint: u8,
    /// Completed shop turnovers (0..=3): a new fascia, a new business look.
    pub fascia: u8,
}

impl Wear {
    /// Does anything show at all? (A city of new buildings costs nothing.)
    pub fn is_plain(&self) -> bool {
        *self == Wear::default()
    }
}

#[derive(Clone, Copy, Debug)]
pub struct CityObj {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub level: u8,
    pub kind: ObjKind,
    /// Storeys above ground (B26); the cottage kit ignores it.
    pub storeys: u8,
    /// District style id: colour and window rhythm of the massing.
    pub style: u8,
    pub stage: Stage,
    /// Colour scheme / size jitter, so no two lots look identical.
    pub seed: u32,
    /// Which side the street is on: 0 = +x, 1 = +y, 2 = -x, 3 = -y.
    pub front: u8,
    /// Sim time in seconds, so the site crews can move.
    pub t: f32,
    /// Index+1 into the scene's pre-rendered building sprites (B33 buildkit);
    /// 0 means "draw with the engine's own art" (construction stages, parks,
    /// and any lot the kit cache could not hold).
    pub kit: u16,
    /// B78: the years on this building, and what the street did to it.
    pub wear: Wear,
}

/// How far the walls are inset from the lot edge, in tiles (setback + yard).
const INSET: f32 = 0.13;

impl CityObj {
    /// B76 r3: how warm this roof runs from inside (0..1) — a factory, a tower
    /// with a plant room, or one house block in ten. A warm roof thaws first, so
    /// its load is thinner and wet paving shows through.
    pub fn heat(&self) -> f32 {
        match self.kind {
            ObjKind::Industry => 0.40 + 0.3 * self.h(0x5a),
            ObjKind::Tower | ObjKind::Office => 0.16 + 0.25 * self.h(0x5a),
            _ => {
                let r = self.h(0x5a);
                if r < 0.07 { 0.28 + 3.0 * r } else { 0.0 }
            }
        }
    }

    pub fn diagonal(&self) -> i32 {
        self.x + self.w - 1 + self.y + self.h - 1
    }

    /// B61d: the same lot on screen — every field that the renderer's output
    /// depends on. `t` is excluded on purpose: it is sim time, it moves every
    /// frame, and the one reader (`life`, the construction crew) is not drawn.
    pub fn same_lot(&self, other: &Self) -> bool {
        self.x == other.x
            && self.y == other.y
            && self.w == other.w
            && self.h == other.h
            && self.level == other.level
            && self.kind == other.kind
            && self.storeys == other.storeys
            && self.style == other.style
            && self.stage == other.stage
            && self.seed == other.seed
            && self.front == other.front
            && self.kit == other.kit
            && self.wear == other.wear
    }

    /// The built footprint inside the lot (the rest is yard).
    fn footprint(&self) -> (f32, f32, f32, f32) {
        let i = INSET;
        (
            self.x as f32 + i,
            self.y as f32 + i,
            (self.x + self.w) as f32 - i,
            (self.y + self.h) as f32 - i,
        )
    }

    /// Wall height in pixels. Cottages stay low so the roof dominates.
    pub fn wall_height(&self) -> i32 {
        if self.kind.is_massing() {
            // A tower needs height that reads against its footprint (B26b).
            // B32: heights double with the tile (a storey was 4 px on 32-px
            // art, so it is 8 px here) — otherwise every building would look
            // half as tall against its plot as it did before.
            let per = if self.storeys >= 8 { 10 } else { 8 };
            return (self.storeys as i32 * per + 4).max(12);
        }
        match self.kind {
            ObjKind::Church => 32,
            ObjKind::Hall => 36,
            ObjKind::Store | ObjKind::Shop => 24,
            ObjKind::Barn => 22,
            // A cottage is clearly lower than it is wide (D41).
            ObjKind::House => {
                // A few two-storey houses (B13 item 2); the rest are cottages.
                let two_storey = self.seed % 11 < 2;
                let base = 18 + (self.seed % 3) as i32 * 2;
                if two_storey { base + 16 } else { base }
            }
            // Handled by the massing path above; give them their box height.
            _ => (self.storeys as i32 * 8 + 4).max(12),
        }
    }

    /// The finish (materials, style, lighting) a wall/roof pass needs.
    fn finish(&self, wall: Color, roof: Color, glow: f32, hour: f32, windows: bool) -> Finish {
        Finish {
            wall,
            roof,
            style: if matches!(self.kind, ObjKind::Hall) { 0 } else { self.style },
            seed: self.seed,
            glow,
            hour,
            obj: crate::windows::obj_key(self.x, self.y),
            floor_px: 8,
            windows,
        }
    }

    /// Cheap stable hash in 0..1 for style choices.
    fn h(&self, salt: u32) -> f32 {
        let mut x = self.seed ^ salt.wrapping_mul(0x9E37_79B9);
        x ^= x >> 15;
        x = x.wrapping_mul(0x2545_F491);
        x ^= x >> 13;
        (x & 0xFFFF) as f32 / 65535.0
    }

    /// Is this plot a building site right now?
    pub fn active(&self) -> bool {
        matches!(
            self.stage,
            Stage::Foundation | Stage::Framing | Stage::Scaffold | Stage::Roofed
        )
    }

    /// Is this plot a fenced building site at this stage (B43c)? Parks, the
    /// civic square and the settlers' cabin have their own art.
    pub fn is_site(&self) -> bool {
        self.stage != Stage::Finished
            && !matches!(
                self.kind,
                ObjKind::Park | ObjKind::LogCabin | ObjKind::Landmark(LandmarkKind::Plaza)
            )
    }

    /// Two or three tiny workers and a cart at the kerb while the plot is being
    /// built (B7 item 9 / D33). Cheap: a handful of 1x2 px figures.
    #[allow(dead_code)]
    fn life(&self, fb: &mut Framebuffer, cam: &Camera) {
        if !self.active() {
            return;
        }
        let base = self.level as i32 * HEIGHT_UNIT;
        let (x0, y0, x1, y1) = (self.x as f32, self.y as f32, (self.x + self.w) as f32, (self.y + self.h) as f32);
        let n = 2 + (self.seed % 2) as i32;
        // Workers walk a short loop along the frontage; the phase comes from
        // sim time so they move slowly without any per-actor state.
        for i in 0..n {
            let phase = self.t * 0.09 + i as f32 * 0.37 + self.seed as f32 * 0.11;
            let f = 0.5 + 0.42 * phase.sin();
            let (wx, wy) = match self.front {
                0 => (x1 + 0.1, y0 + (y1 - y0) * f),
                1 => (x0 + (x1 - x0) * f, y1 + 0.1),
                2 => (x0 - 0.1, y0 + (y1 - y0) * f),
                _ => (x0 + (x1 - x0) * f, y0 - 0.1),
            };
            let q = cam.project(wx, wy, base + 1);
            let shirt = pal::PEOPLE_SHIRT[(self.seed as usize + i as usize) % pal::PEOPLE_SHIRT.len()];
            fb.fill_rect(q.0 as i32, q.1 as i32 - 3, 1, 2, shirt);
            fb.fill_rect(q.0 as i32, q.1 as i32 - 1, 1, 1, pal::PEOPLE_LEGS);
        }
        // The cart drives slowly up and down the frontage.
        let slide = (self.t * 0.07 + self.seed as f32 * 0.3).sin() * 0.9;
        let (cx, cy) = match self.front {
            0 => (x1 + 0.35 + slide, (y0 + y1) * 0.5),
            1 => ((x0 + x1) * 0.5 + slide, y1 + 0.35),
            2 => (x0 - 0.35 - slide, (y0 + y1) * 0.5),
            _ => ((x0 + x1) * 0.5 + slide, y0 - 0.35),
        };
        let q = cam.project(cx, cy, base + 1);
        let (px, py) = (q.0 as i32, q.1 as i32);
        fb.fill_rect(px - 2, py - 3, 4, 3, palette::shade(pal::CAR_BODY[1], 0.95));
        fb.fill_rect(px - 2, py - 1, 1, 1, pal::CAR_WHEEL);
        fb.fill_rect(px + 1, py - 1, 1, 1, pal::CAR_WHEEL);
    }

    pub fn draw(&self, fb: &mut Framebuffer, cam: &Camera, glow: f32) {
        self.draw_with_snow(fb, cam, glow, 0.0)
    }



    /// As [`Self::draw`], with snow lying on the roofs (B27 item 2): a simple
    /// whitening of the top faces, which is all the placeholder massing needs.
    pub fn draw_with_snow(&self, fb: &mut Framebuffer, cam: &Camera, glow: f32, snow: f32) {
        // D66: a plot's domesticated ground (park lawn, pond, gravel, yard) wears
        // the world's land — a desert park is an oasis garden of sand and palms,
        // a tundra park is snow under dark conifers, not a lime lawn in either.
        let look = crate::biome::look(map::Biome::Temperate);
        let season = crate::terrain::SeasonLook::default();
        self.draw_in_land(fb, cam, glow, self.t, snow, &season, look);
    }

    /// The whole drawing, in a chosen land's colours (D66).
    pub fn draw_in_land(
        &self,
        fb: &mut Framebuffer,
        cam: &Camera,
        glow: f32,
        hour: f32,
        snow: f32,
        season: &crate::terrain::SeasonLook,
        look: &'static crate::biome::Ground,
    ) {
        // The hour is what decides which windows are lit (B58b item 7).
        let base = self.level as i32 * HEIGHT_UNIT;
        let p = |x: f32, y: f32, z: i32| cam.project(x, y, base + z);
        let (bx0, by0, bx1, by1) = self.footprint();
        let (lx0, ly0, lx1, ly1) = (
            self.x as f32,
            self.y as f32,
            (self.x + self.w) as f32,
            (self.y + self.h) as f32,
        );

        // B60/D74: the old D41 fixed offset shadow is gone. The one the sun
        // throws comes from the renderer's height field (`shadow::bake_mask`),
        // so it turns and stretches with the hour like every other shadow.
        let mut wall = palette::jitter(pal::WALLS[(self.seed as usize) % pal::WALLS.len()], 0.5, 0.05);
        let mut roof = palette::jitter(pal::ROOFS[(self.seed as usize + 3) % pal::ROOFS.len()], 0.5, 0.05);
        if self.kind == ObjKind::Barn {
            // B80: a barn is red (or weathered brown, now and then white), under
            // a grey tin or a dark shingle roof.
            let pick = self.seed % 7;
            wall = palette::jitter(
                match pick {
                    0 => palette::rgb(0x8a, 0x5a, 0x3a),
                    1 => palette::rgb(0xe6, 0xe2, 0xd6),
                    _ => palette::rgb(0xa8, 0x38, 0x2e),
                },
                0.5,
                0.05,
            );
            roof = palette::jitter(
                if self.seed % 3 == 0 { palette::rgb(0x62, 0x5e, 0x5a) } else { palette::rgb(0x8c, 0x92, 0x98) },
                0.5,
                0.05,
            );
        }

        // B43c: a plot being built has trampled soil, an orange fence and a
        // gate instead of a lawn; the lawn is laid when it is finished.
        let site_geo = (self.is_site()).then(|| {
            site::Geo::of(self, (lx0, ly0, lx1, ly1), (bx0, by0, bx1, by1))
        });
        if let Some(g) = &site_geo {
            site::before(fb, &p, g, self.stage);
        }

        // B65: the yard is *ground* — a figure standing on it must be drawn
        // over it (B43) — while the walls below are a solid: the depth key
        // moves up a step before they are drawn.
        fb.set_depth_key(crate::actors::static_key(fb.slice_diag_or(self.diagonal()), false));
        // --- the yard: grass, a little darker than open ground, and a path ---
        if site_geo.is_none() && !matches!(self.stage, Stage::Staked) && self.kind != ObjKind::Barn {
            let yard = [
                p(lx0, ly0, 0),
                p(lx1, ly0, 0),
                p(lx1, ly1, 0),
                p(lx0, ly1, 0),
            ];
            // B76: a winter lawn is dormant like the fields round it.
            let mut grass_c = palette::shade(look.grass[3], 0.94);
            if season.grass_dry > 0.02 {
                let dry = palette::rgb(0x9c, 0x8c, 0x58);
                grass_c = palette::mix(grass_c, dry, (0.85 * season.grass_dry).min(0.9));
            }
            fb.fill_poly(&yard, grass_c);
            // Path from the door to the street.
            let (dx, dy) = match self.front {
                0 => ((lx1 + 0.02), (ly0 + ly1) * 0.5),
                1 => ((lx0 + lx1) * 0.5, (ly1 + 0.02)),
                2 => ((lx0 - 0.02), (ly0 + ly1) * 0.5),
                _ => ((lx0 + lx1) * 0.5, (ly0 - 0.02)),
            };
            let door_side = match self.front {
                0 => (bx1, (by0 + by1) * 0.5),
                1 => ((bx0 + bx1) * 0.5, by1),
                2 => (bx0, (by0 + by1) * 0.5),
                _ => ((bx0 + bx1) * 0.5, by0),
            };
            fb.line(p(door_side.0, door_side.1, 0), p(dx, dy, 0), pal::SIDEWALK[1]);
        }

        // Everything from here up stands: the walls, the roof, the lamps.
        fb.set_depth_key(crate::actors::static_key(fb.slice_diag_or(self.diagonal()), true));
        // B38: night light on the civic spaces and the houses.
        match self.kind {
            ObjKind::Landmark(LandmarkKind::Plaza) | ObjKind::Park => {
                let n = if matches!(self.kind, ObjKind::Park) { 2 } else { 4 };
                for i in 0..n {
                    let t = (i as f32 + 0.5) / n as f32;
                    let (lx, ly) = if n == 4 {
                        let (fx, fy) = [(0.22f32, 0.22f32), (0.78, 0.22), (0.22, 0.78), (0.78, 0.78)][i];
                        (fx, fy)
                    } else {
                        (0.5, t)
                    };
                    crate::holiday::park_lamp(
                        fb,
                        cam,
                        self.x as f32 + self.w as f32 * lx,
                        self.y as f32 + self.h as f32 * ly,
                        self.level,
                        glow,
                    );
                }
            }
            ObjKind::House | ObjKind::Townhouse | ObjKind::Store | ObjKind::Shop => {
                crate::holiday::porch_light(
                    fb, cam, self.x, self.y, self.w, self.h, self.level, self.front, glow, self.seed,
                );
            }
            _ => {}
        }
        if self.kind.is_massing() {
            let open = matches!(self.kind, ObjKind::Park)
                || matches!(self.kind, ObjKind::Landmark(LandmarkKind::Plaza))
                // B59: a car place draws its apron and bays even while the
                // trade is going in, so a forecourt never reads as a cottage.
                || car_place_kind(self.kind).is_some();
            if open || self.stage == Stage::Finished {
                self.massing(fb, &p, (bx0, by0, bx1, by1), glow, hour, snow, season, look);
                return;
            }
            if self.stage == Stage::Roofed {
                // The shell is up and glazed, the flat cap poured; the kit's
                // finished building replaces it when the scaffold is gone.
                let h = self.wall_height();
                let fin = Finish {
                    wall: palette::jitter(pal::STYLE_CONCRETE, self.h(0x2c), 0.05),
                    roof: palette::shade(pal::CONCRETE, 0.92),
                    style: 5,
                    seed: self.seed,
                    glow,
                    hour,
                    obj: crate::windows::obj_key(self.x, self.y),
                    floor_px: if self.storeys >= 8 { 10 } else { 8 },
                    windows: false,
                };
                walls(fb, &p, (bx0, by0, bx1, by1), 0, h, &fin);
                if let Some(g) = &site_geo {
                    site::glazing(fb, &p, g, glow);
                    site::finishing(fb, &p, g);
                    site::after(fb, &p, g, self.stage);
                }
                return;
            }
            // Earlier stages: the shared site art below (steel frame).
        }
        // The settlers' log cabin has its own art for every stage (B39/B40):
        // the cottage kit below would draw it as a brick cottage.
        if matches!(self.kind, ObjKind::LogCabin) {
            let h = self.wall_height();
            self.draw_cabin(fb, &p, (lx0, ly0, lx1, ly1), h, snow, self.heat());
            return;
        }
        let sg = site_geo.as_ref();
        match self.stage {
            Stage::Staked => {
                if let Some(g) = sg {
                    site::staked(fb, &p, g);
                }
            }
            Stage::Graded => {
                if let Some(g) = sg {
                    site::excavation(fb, &p, g);
                }
            }
            Stage::Foundation => {
                if let Some(g) = sg {
                    site::slab(fb, &p, g);
                }
            }
            Stage::Framing => {
                if let Some(g) = sg {
                    site::slab_with(fb, &p, g, g.steel);
                    let hf = site::frame_height(g);
                    site::frame(fb, &p, g, 4, hf);
                    if !g.steel {
                        site::rafters(fb, &p, g, hf);
                    }
                }
            }
            Stage::Scaffold => {
                if let Some(g) = sg {
                    site::enclosure(fb, &p, g, wall, roof, glow);
                }
            }
            Stage::Roofed => {
                let h = self.wall_height();
                let fin = self.finish(wall, roof, glow, hour, true);
                walls(fb, &p, (bx0, by0, bx1, by1), 0, h, &fin);
                self.roof(fb, &p, (bx0, by0, bx1, by1), h, snow, &fin);
                if let Some(g) = sg {
                    site::finishing(fb, &p, g);
                }
            }
            Stage::Finished => {
                let h = self.wall_height();
                let barn = self.kind == ObjKind::Barn;
                let fin = self.finish(wall, roof, glow, hour, !barn);
                if barn {
                    self.silo(fb, &p, (bx0, by0, bx1, by1), snow);
                }
                walls(fb, &p, (bx0, by0, bx1, by1), 0, h, &fin);
                self.roof(fb, &p, (bx0, by0, bx1, by1), h, snow, &fin);
                if barn {
                    self.barn_trim(fb, &p, (bx0, by0, bx1, by1), h);
                } else {
                    self.details(fb, &p, (bx0, by0, bx1, by1), h, &fin);
                }
            }
        }
        if let Some(g) = sg {
            site::after(fb, &p, g, self.stage);
        }
    }

    /// B80: the grain silo behind a barn: a tall cylinder with a domed cap.
    fn silo(
        &self,
        fb: &mut Framebuffer,
        p: &impl Fn(f32, f32, i32) -> (f32, f32),
        (x0, y0, _x1, _y1): (f32, f32, f32, f32),
        snow: f32,
    ) {
        let (sx, sy) = p(x0 + 0.30, y0 + 0.30, 0);
        let (sx, sy) = (sx as i32, sy as i32);
        let r = 7;
        let hgt = 50;
        let tone = palette::rgb(0xb4, 0xb8, 0xb8);
        for dx in -r..=r {
            // Lit on the left, dark on the right, a band of hoops every 6 px.
            let f = (dx as f32 + r as f32) / (2.0 * r as f32);
            let k = 1.14 - 0.52 * f;
            for dy in 0..hgt {
                let mut c = palette::shade(tone, k);
                if dy % 7 == 0 {
                    c = palette::shade(c, 0.82);
                }
                fb.put(sx + dx, sy - dy, c);
            }
        }
        // The dome.
        for dy in 0..6 {
            let half = ((r * r) as f32 * (1.0 - (dy as f32 / 6.0).powi(2))).sqrt() as i32;
            for dx in -half..=half {
                let f = (dx as f32 + r as f32) / (2.0 * r as f32);
                let mut c = palette::shade(palette::rgb(0x8e, 0x94, 0x9a), 1.2 - 0.5 * f);
                if snow > 0.05 {
                    c = palette::mix(c, palette::rgb(0xec, 0xf3, 0xfa), 0.85 * snow);
                }
                fb.put(sx + dx, sy - hgt - dy, c);
            }
        }
        fb.put(sx, sy - hgt - 6, pal::OUTLINE);
    }

    /// B80: the big double door and the hayloft door of a barn, white-trimmed.
    fn barn_trim(
        &self,
        fb: &mut Framebuffer,
        p: &impl Fn(f32, f32, i32) -> (f32, f32),
        (x0, y0, x1, y1): (f32, f32, f32, f32),
        h: i32,
    ) {
        let trim = palette::rgb(0xf0, 0xec, 0xe0);
        let dark = palette::rgb(0x4a, 0x2a, 0x22);
        // The +y face (the left wall on screen): a double door with an X on each leaf.
        let xc = (x0 + x1) * 0.5;
        let dw = ((x1 - x0) * 0.22).max(0.18);
        let dh = (h as f32 * 0.72) as i32;
        let a = p(xc - dw, y1, 0);
        let b = p(xc + dw, y1, 0);
        let c = p(xc + dw, y1, dh);
        let d = p(xc - dw, y1, dh);
        fb.fill_poly(&[a, b, c, d], dark);
        for (u, v) in [(a, c), (b, d), (a, d), (b, c), (d, c), (a, b)] {
            fb.line(u, v, trim);
        }
        let m0 = p(xc, y1, 0);
        let m1 = p(xc, y1, dh);
        fb.line(m0, m1, trim);
        // The hayloft door above it.
        let hl = p(xc - dw * 0.5, y1, h - 5);
        let hr = p(xc + dw * 0.5, y1, h - 5);
        let hl2 = p(xc - dw * 0.5, y1, h - 1);
        let hr2 = p(xc + dw * 0.5, y1, h - 1);
        let _ = (hl, hr, hl2, hr2);
        // The +x face: a small hayloft window.
        let yc = (y0 + y1) * 0.5;
        let w0 = p(x1, yc - 0.12, h * 2 / 3);
        let w1 = p(x1, yc + 0.12, h * 2 / 3);
        let w2 = p(x1, yc + 0.12, h * 2 / 3 + 5);
        let w3 = p(x1, yc - 0.12, h * 2 / 3 + 5);
        fb.fill_poly(&[w0, w1, w2, w3], dark);
        fb.line(w0, w1, trim);
        fb.line(w3, w2, trim);
    }

    /// B26 placeholder massing: a clear box, a flat cap and a window row per
    /// storey, coloured by district. It exists so the city's shape reads before
    /// the real kit (B27); every parameter is data.
    /// B59/B59b: the places cars need, drawn inside their plot (D72 car
    /// culture) by [`places`]: every line, lane, pump and building is laid out
    /// by the same `map::carplaces` definition the sim parks its cars by.
    fn draw_place(
        &self,
        fb: &mut Framebuffer,
        p: &impl Fn(f32, f32, i32) -> (f32, f32),
        _rect: (f32, f32, f32, f32),
        glow: f32,
        hour: f32,
        look: &'static crate::biome::Ground,
    ) {
        places::draw(self, fb, p, glow, hour, look);
    }

    /// B26 placeholder massing: a clear box, a flat cap and a window row per
    /// storey, coloured by district. It exists so the city's shape reads before
    /// the real kit (B27); every parameter is data.
    fn massing(
        &self,
        fb: &mut Framebuffer,
        p: &impl Fn(f32, f32, i32) -> (f32, f32),
        (x0, y0, x1, y1): (f32, f32, f32, f32),
        glow: f32,
        hour: f32,
        snow: f32,
        season: &crate::terrain::SeasonLook,
        look: &'static crate::biome::Ground,
    ) {
        // A real index, not a truncated fraction: every building gets its own
        // palette entry (B26b: all towers came out the same pale grey).
        let pick = (self.h(0x2c) * 997.0) as usize;
        let (wall, roof) = if matches!(self.kind, ObjKind::Hall) {
            // A civic building is stone/brick with a pale coping, whatever the
            // district around it looks like.
            (pal::STYLE_BRICK, pal::SIDEWALK[0])
        } else if self.storeys <= 4 {
            // Low-rise town buildings take the **cottage kit's** palette of
            // walls and roofs: a high street of brick, stucco, clapboard and
            // slate is what makes a town read at 1x (D40). Brick walls with a
            // brick roof read as one brown crate.
            let w = if self.style == 1 || self.style == 3 {
                pal::STYLE_STUCCO
            } else {
                pal::WALLS[pick % pal::WALLS.len()]
            };
            let r = pal::ROOFS[(pick + 3) % pal::ROOFS.len()];
            (
                palette::jitter(w, 0.5, 0.045),
                palette::jitter(r, 0.5, 0.05),
            )
        } else if self.storeys >= 8 {
            // Downtown towers: glass, pale concrete or bronze, by seed. The
            // crown is a dark coping, never the wall colour: a glass-coloured
            // flat cap reads as a sheet of sky floating over the city.
            let w = [pal::STYLE_GLASS, pal::STYLE_CONCRETE, pal::STYLE_BRONZE][pick % 3];
            let coping = palette::mix(pal::STYLE_STEEL, pal::OUTLINE, 0.35);
            (palette::jitter(w, 0.5, 0.05), palette::shade(coping, 1.0))
        } else {
            // Mid-rise: brick or stucco with a dark coping.
            let (w, r) = style_colours(self.style);
            let w = if self.style == 3 { pal::STYLE_CONCRETE } else { w };
            (palette::jitter(w, 0.5, 0.05), palette::shade(r, 0.82))
        };
        let h = self.wall_height();
        let heat = self.heat();
        if matches!(self.kind, ObjKind::Park) {
            self.draw_park(fb, p, (x0, y0, x1, y1), snow, season, look);
            return;
        }
        // B59: car culture places (a car park, a forecourt, a drive-through…).
        if car_place_kind(self.kind).is_some() {
            self.draw_place(fb, p, (x0, y0, x1, y1), glow, hour, look);
            return;
        }
        if matches!(self.kind, ObjKind::Cemetery) {
            self.draw_cemetery(fb, p, (x0, y0, x1, y1), look);
            return;
        }
        if let ObjKind::Landmark(LandmarkKind::Plaza) = self.kind {
            self.draw_plaza(fb, p, (x0, y0, x1, y1), look);
            return;
        }
        // B62: the zoo and the castle are engine art (the kit has neither).
        if let ObjKind::Landmark(LandmarkKind::Zoo) = self.kind {
            let mut lawn = palette::mix(look.grass[3], look.grass[5], 0.15);
        // B76: a park lawn in winter is dormant too.
        if season.grass_dry > 0.02 {
            lawn = palette::mix(lawn, palette::rgb(0x9c, 0x8c, 0x58), (0.85 * season.grass_dry).min(0.9));
        }
            let o0 = p(0.0, 0.0, 0);
            let cam0 = crate::iso::Camera::new(o0.0 as i32, o0.1 as i32);
            crate::spectacle::zoo(fb, &cam0, self.x as f32, self.y as f32, self.w as f32, self.h as f32, 0, self.seed, lawn);
            return;
        }
        // B52b: the government district is engine art in the world's style.
        if let ObjKind::Landmark(k @ (LandmarkKind::Courthouse | LandmarkKind::PoliceStation | LandmarkKind::PostOffice)) = self.kind {
            use crate::spectacle::CivicBld;
            let o0 = p(0.0, 0.0, 0);
            let cam0 = crate::iso::Camera::new(o0.0 as i32, o0.1 as i32);
            let which = match k {
                LandmarkKind::Courthouse => CivicBld::Courthouse,
                LandmarkKind::PoliceStation => CivicBld::Police,
                _ => CivicBld::PostOffice,
            };
            crate::spectacle::civic_building(fb, &cam0, which, self.x as f32, self.y as f32, self.w as f32, self.h as f32, 0, self.seed, look.biome, glow);
            return;
        }
        if let ObjKind::Landmark(LandmarkKind::Castle) = self.kind {
            let ground = palette::mix(look.grass[3], look.grass[4], 0.4);
            let o0 = p(0.0, 0.0, 0);
            let cam0 = crate::iso::Camera::new(o0.0 as i32, o0.1 as i32);
            crate::spectacle::castle(fb, &cam0, self.x as f32, self.y as f32, self.w as f32, self.h as f32, 0, self.seed, ground);
            return;
        }
        let _ = snow;
        let fin = Finish {
            wall,
            roof,
            // B58b item 7: a glass tower gets the curtain-wall texture (mullions,
            // spandrels, sky in the panes) whichever district it stands in, so a
            // tower's face reads as glass by day instead of a black slab.
            style: if matches!(self.kind, ObjKind::Hall) {
                0
            } else if self.storeys >= 8 && wall == pal::STYLE_GLASS {
                3
            } else {
                self.style
            },
            seed: self.seed,
            glow,
            hour,
            obj: crate::windows::obj_key(self.x, self.y),
            floor_px: 8,
            windows: true,
        };
        // Low-rise town buildings keep pitched roofs: a high street of gables
        // and eaves reads as a town at 1x, a row of flat caps reads as crates.
        if self.storeys <= 4 && !matches!(self.kind, ObjKind::Tower) {
            walls(fb, p, (x0, y0, x1, y1), 0, h, &fin);
            self.roof(fb, p, (x0, y0, x1, y1), h, snow, &fin);
            return;
        }
        // Mid-rise and towers: a shaft, a set-back upper stage and a crown, so
        // a downtown cluster has a skyline instead of one height (D40).
        let (top, h2) = if self.storeys >= 8 {
            let shaft = (h as f32 * 0.78) as i32;
            (shaft, h)
        } else {
            (h, h)
        };
        walls(fb, p, (x0, y0, x1, y1), 0, top, &fin);
        if h2 > top {
            let i = 0.22;
            let (ax0, ay0, ax1, ay1) = (x0 + i, y0 + i, x1 - i, y1 - i);
            walls(fb, p, (ax0, ay0, ax1, ay1), top, h2, &fin);
            let cap = [p(ax0, ay0, h2), p(ax1, ay0, h2), p(ax1, ay1, h2), p(ax0, ay1, h2)];
            // B76 r3: the load lies on the roof plane itself, with a drift along
            // the parapet and wet patches on a roof that warms from inside.
            snowy_cap(fb, &cap, palette::shade(roof, 1.10), snow, heat, self.seed);
            parapet(fb, &cap, roof, snow);
        } else {
            let cap = [p(x0, y0, h), p(x1, y0, h), p(x1, y1, h), p(x0, y1, h)];
            snowy_cap(fb, &cap, palette::shade(roof, 1.06), snow, heat, self.seed);
            parapet(fb, &cap, roof, snow);
        }
        // The tallest towers carry a mast, so downtown has landmarks in it.
        if self.storeys >= 14 {
            let (cx, cy) = ((x0 + x1) * 0.5, (y0 + y1) * 0.5);
            let base = p(cx, cy, h);
            fb.line((base.0, base.1), (base.0, base.1 - 12.0), pal::SCAFFOLD);
            fb.put(base.0 as i32, base.1 as i32 - 13, pal::WINDOW_HALO);
        }
        // The town hall carries a small clock tower (D24), so the civic heart
        // of the town is recognisable at 1x.
        if matches!(self.kind, ObjKind::Hall) {
            let (cx, cy) = ((x0 + x1) * 0.5, (y0 + y1) * 0.5);
            let r = 0.7;
            walls(fb, p, (cx - r, cy - r, cx + r, cy + r), h, h + 14, &fin);
            let clock = [
                p(cx - 0.3, cy - r, h + 14),
                p(cx + 0.3, cy - r, h + 14),
                p(cx + 0.3, cy - r, h + 8),
                p(cx - 0.3, cy - r, h + 8),
            ];
            fb.fill_poly(&clock, pal::WINDOW_LIT);
        }
        // A rooftop unit on the tallest kinds.
        if self.storeys >= 6 {
            let u = [
                p(x0 + 0.3, y0 + 0.3, h + 4),
                p(x1 - 0.3, y0 + 0.3, h + 4),
                p(x1 - 0.3, y1 - 0.3, h + 4),
                p(x0 + 0.3, y1 - 0.3, h + 4),
            ];
            fb.fill_poly(&u, snow_white(palette::shade(roof, 1.15), snow));
            fb.line(u[0], u[1], palette::shade(roof, 0.8));
        }
    }

    /// The settlers' log cabin (B39/D62, B40): a cleared pad with logs laid out,
    /// then log walls rising course by course, then a shingled roof. A compact
    /// little building on the plot — not a slab the size of the plot.
    fn draw_cabin(
        &self,
        fb: &mut Framebuffer,
        p: &impl Fn(f32, f32, i32) -> (f32, f32),
        (x0, y0, x1, y1): (f32, f32, f32, f32),
        _h: i32,
        snow: f32,
        heat: f32,
    ) {
        let log = palette::rgb(0x96, 0x6c, 0x44);
        let log_dark = palette::rgb(0x6a, 0x4a, 0x30);
        let log_lit = palette::rgb(0xc0, 0x94, 0x60);
        // The cabin sits on the plot with a margin, so the pad shows round it.
        let (cx0, cy0, cx1, cy1) = (x0 + 0.3, y0 + 0.32, x1 - 0.3, y1 - 0.32);
        let z = match self.stage {
            Stage::Staked => 0,
            Stage::Graded => 2,
            Stage::Foundation => 5,
            Stage::Framing => 9,
            Stage::Scaffold => 13,
            _ => 15,
        };
        if matches!(self.stage, Stage::Staked | Stage::Graded) {
            // A cleared earth pad with the first logs squared and laid out.
            let pad = [p(x0, y0, 0), p(x1, y0, 0), p(x1, y1, 0), p(x0, y1, 0)];
            fb.fill_poly(&pad, palette::shade(pal::DIRT[3], 0.98));
            let pad2 = [
                p(x0 + 0.12, y0 + 0.12, 0),
                p(x1 - 0.12, y0 + 0.12, 0),
                p(x1 - 0.12, y1 - 0.12, 0),
                p(x0 + 0.12, y1 - 0.12, 0),
            ];
            fb.fill_poly(&pad2, palette::shade(pal::DIRT[4], 1.03));
        }
        if z > 0 {
            // The near face (+y), lit; the right face (+x), in shade.
            let front = [
                p(cx0, cy1, z),
                p(cx1, cy1, z),
                p(cx1, cy1, 0),
                p(cx0, cy1, 0),
            ];
            fb.fill_poly(&front, log);
            let side = [
                p(cx1, cy0, z),
                p(cx1, cy1, z),
                p(cx1, cy1, 0),
                p(cx1, cy0, 0),
            ];
            fb.fill_poly(&side, log_dark);
            // Log courses: dark joints every 3 px, so it reads as timber.
            let mut zz = 3;
            while zz < z {
                fb.line(p(cx0, cy1, zz), p(cx1, cy1, zz), log_dark);
                fb.line(p(cx1, cy0, zz), p(cx1, cy1, zz), palette::shade(log_dark, 0.8));
                zz += 3;
            }
            fb.line(p(cx0, cy1, z), p(cx1, cy1, z), log_lit);
            fb.line(p(cx1, cy0, z), p(cx1, cy1, z), log_dark);
            // The log ends at the corners.
            for (cx, cy) in [(cx0, cy1), (cx1, cy1), (cx1, cy0)] {
                let q = p(cx, cy, z);
                fb.put(q.0 as i32, q.1 as i32, log_lit);
            }
            if matches!(self.stage, Stage::Roofed | Stage::Finished) {
                // A gable roof: ridge along the plot, the near slope lit.
                let cym = (cy0 + cy1) * 0.5;
                let rz = z + 6;
                let a = p(cx0 - 0.06, cy1 + 0.06, z);
                let b = p(cx1 + 0.06, cy1 + 0.06, z);
                let ra = p(cx0 - 0.06, cym, rz);
                let rb = p(cx1 + 0.06, cym, rz);
                // B76 r3: the cabin's two slopes take the load like any roof,
                // with the ridge line kept dark so the gable still reads.
                let lit = if snow > 0.02 {
                    palette::mix(
                        palette::shade(pal::ROOFS[0], 1.15),
                        palette::rgb(0xf2, 0xf6, 0xfd),
                        (0.95 * snow * (1.0 - 0.3 * heat)).min(0.96),
                    )
                } else {
                    palette::shade(pal::ROOFS[0], 1.15)
                };
                let shade = if snow > 0.02 {
                    palette::mix(
                        palette::shade(pal::ROOFS[0], 0.66),
                        palette::rgb(0xdc, 0xe6, 0xf2),
                        (0.95 * snow * (1.0 - 0.3 * heat)).min(0.96),
                    )
                } else {
                    palette::shade(pal::ROOFS[0], 0.66)
                };
                fb.fill_poly(&[a, b, rb, ra], lit);
                fb.fill_poly(
                    &[
                        p(cx1 + 0.06, cy0 - 0.06, z),
                        p(cx0 - 0.06, cy0 - 0.06, z),
                        ra,
                        rb,
                    ],
                    shade,
                );
                fb.line(ra, rb, pal::OUTLINE);
                // A stone chimney on the far slope.
                let ch = p(cx1 - 0.15, cy0 + 0.2, rz);
                fb.fill_rect(ch.0 as i32 - 1, ch.1 as i32 - 4, 3, 5, pal::WALL_STONE[1]);
                fb.fill_rect(ch.0 as i32 - 1, ch.1 as i32 - 5, 3, 1, pal::WALL_STONE[0]);
            }
            if self.stage == Stage::Finished {
                // A door and one lit window in the near face.
                let d = p((cx0 + cx1) * 0.5, cy1, 0);
                fb.fill_rect(d.0 as i32 - 1, d.1 as i32 - 6, 4, 6, pal::DOOR);
                let w = p(cx0 + 0.22, cy1, z / 2);
                fb.fill_rect(w.0 as i32 - 1, w.1 as i32 - 2, 3, 3, pal::WINDOW_LIT);
            } else if matches!(self.stage, Stage::Framing | Stage::Scaffold) {
                // Scaffolding poles at the corners while the walls go up.
                for (cx, cy) in [(cx0, cy1), (cx1, cy1)] {
                    let q = p(cx, cy, 0);
                    fb.line((q.0, q.1), (q.0, q.1 - (z + 6) as f32), pal::SCAFFOLD);
                }
            }
        }
    }

    /// The civic square (B35/D55): a paved plaza with a central fountain, a ring
    /// of formal trees, benches and a bandstand corner — the thing the
    /// boulevards lead to.
    fn draw_plaza(
        &self,
        fb: &mut Framebuffer,
        p: &impl Fn(f32, f32, i32) -> (f32, f32),
        (x0, y0, x1, y1): (f32, f32, f32, f32),
        look: &'static crate::biome::Ground,
    ) {
        let (cx, cy) = ((x0 + x1) * 0.5, (y0 + y1) * 0.5);
        let (w, h) = (x1 - x0, y1 - y0);
        // Paving: pale slabs over the whole plot, with a darker kerb ring.
        let pad = [p(x0, y0, 0), p(x1, y0, 0), p(x1, y1, 0), p(x0, y1, 0)];
        tex::fill_poly_px(fb, &pad, |px, py, u, v| {
            // Slabs 8 px square, jointed, with a per-slab tint.
            let ju = (u * (w * 5.0)).fract();
            let jv = (v * (h * 5.0)).fract();
            let slab = tex::hash2((u * w * 5.0) as i32, (v * h * 5.0) as i32, 0x71);
            let c = palette::jitter(pal::SIDEWALK[0], slab, 0.05);
            if ju < 0.10 || jv < 0.10 {
                palette::shade(c, 0.88)
            } else {
                palette::shade(c, 1.0 + (tex::hash2(px, py, 0x72) - 0.5) * 0.06)
            }
        });
        // Two crossing bands of darker setts, meeting at the fountain.
        for k in 0..2 {
            let o = 0.10 + k as f32 * 0.06;
            fb.line(p(x0 + 0.2, cy + o, 0), p(x1 - 0.2, cy + o, 0), palette::shade(pal::SIDEWALK[1], 0.86));
            fb.line(p(cx + o, y0 + 0.2, 0), p(cx + o, y1 - 0.2, 0), palette::shade(pal::SIDEWALK[1], 0.86));
        }
        // The fountain: a stone basin with water and a spray.
        let r = (w.min(h) * 0.18).clamp(0.5, 1.2);
        let n = 20;
        let mut basin: Vec<(f32, f32)> = Vec::new();
        let mut water: Vec<(f32, f32)> = Vec::new();
        for i in 0..n {
            let a = i as f32 / n as f32 * std::f32::consts::TAU;
            basin.push(p(cx + a.cos() * (r + 0.10), cy + a.sin() * (r + 0.10), 0));
            water.push(p(cx + a.cos() * r, cy + a.sin() * r, 0));
        }
        fb.fill_poly(&basin, look.wall_stone[1]);
        fb.fill_poly(&water, look.water[1]);
        let q = p(cx, cy, 0);
        fb.line((q.0, q.1 - 1.0), (q.0, q.1 - 5.0), palette::mix(look.water[0], look.rapids, 0.5));
        fb.put(q.0 as i32, q.1 as i32 - 6, look.rapids);
        // B62: the clock tower stands in the far corner of the square.
        {
            let o0 = p(0.0, 0.0, 0);
            let cam0 = crate::iso::Camera::new(o0.0 as i32, o0.1 as i32);
            let (tx, ty) = clock_tower_spot(x0, y0, w, h);
            crate::spectacle::clock_tower(fb, &cam0, tx, ty, 0);
        }
        // Formal trees in the four corners and halfway along each side.
        for (fx, fy) in [
            (0.86, 0.14),
            (0.14, 0.86),
            (0.86, 0.86),
            (0.5, 0.12),
            (0.5, 0.88),
            (0.12, 0.5),
            (0.88, 0.5),
        ] {
            self.park_tree_big(fb, p, x0 + w * fx, y0 + h * fy, ((fx * 10.0) as u8) % 4 + 1, look);
        }
        // Benches along the two near sides.
        let bench = palette::shade(look.trunk, 0.8);
        for f in [0.3f32, 0.7] {
            let b = p(x0 + w * f, y1 - 0.18, 0);
            fb.fill_rect(b.0 as i32, b.1 as i32, 5, 1, bench);
            let b2 = p(x1 - 0.18, y0 + h * f, 0);
            fb.fill_rect(b2.0 as i32, b2.1 as i32, 1, 3, bench);
        }
    }

    /// A park is not an empty green square (B26c item 3): mown lawn, a gravel
    /// path across it, trees round the edge and a pond or a fountain in the
    /// middle. All of it is placeholder-level drawing from the lot's own data
    /// (size and seed), so it costs nothing to keep.
    ///
    /// B58b item 4: *what kind* of park comes from the lot's `style`, so a
    /// district's greens are a lawn, a playground, a pitch, a paved square,
    /// allotments or the big pond park instead of six copies of one design.
    fn draw_park(
        &self,
        fb: &mut Framebuffer,
        p: &impl Fn(f32, f32, i32) -> (f32, f32),
        (x0, y0, x1, y1): (f32, f32, f32, f32),
        snow: f32,
        season: &crate::terrain::SeasonLook,
        look: &'static crate::biome::Ground,
    ) {
        let kind = map::ParkKind::of_style(self.style);
        let (cx, cy) = ((x0 + x1) * 0.5, (y0 + y1) * 0.5);
        let (w, h) = (x1 - x0, y1 - y0);
        let small = w.min(h) < 4.0;
        // The surface: grass for the green kinds, paving for the plaza.
        let pad = [p(x0, y0, 0), p(x1, y0, 0), p(x1, y1, 0), p(x0, y1, 0)];
        // An oasis lawn in a desert, snow in a tundra, jungle grass in the
        // tropics: the land's own soft ground, mown.
        let mut lawn = palette::mix(look.grass[3], look.grass[5], 0.15);
        // B76: a park lawn in winter is dormant too.
        if season.grass_dry > 0.02 {
            lawn = palette::mix(lawn, palette::rgb(0x9c, 0x8c, 0x58), (0.85 * season.grass_dry).min(0.9));
        }
        let paving = palette::mix(pal::SIDEWALK[0], look.wall_stone[1], 0.25);
        let base = if kind == map::ParkKind::Plaza { paving } else { lawn };
        // B76: a park under snow is white ground, not a green square in a white
        // city. The paths keep a little more of their colour (they are walked).
        let base = if snow > 0.02 {
            palette::mix(base, palette::rgb(0xec, 0xf3, 0xfa), (0.90 * snow).clamp(0.0, 0.94))
        } else {
            base
        };
        // Mottled, not a flat slab of colour: a tundra park is snow with its own
        // texture, and a flat white diamond read as a pasted card in review.
        tex::fill_poly_px(fb, &pad, |px, py, u, v| {
            let n = tex::hash2(px / 2, py / 3, 0x6D);
            if kind == map::ParkKind::Plaza {
                // Slabs with joints, so a paved square reads as paving.
                let ju = (u * (w * 5.0)).fract();
                let jv = (v * (h * 5.0)).fract();
                if ju < 0.08 || jv < 0.08 {
                    return palette::shade(base, 0.86);
                }
                return palette::jitter(base, tex::hash2((u * w * 5.0) as i32, (v * h * 5.0) as i32, 0x72), 0.05);
            }
            if n > 0.80 {
                palette::shade(base, 1.05)
            } else if n < 0.18 {
                palette::shade(base, 0.95)
            } else {
                base
            }
        });
        // Mowing stripes: alternating bands across a lawn (a pitch is marked
        // instead).
        if kind != map::ParkKind::Plaza && !small {
            let stripes = 5;
            for k in 0..stripes {
                if k % 2 == 0 {
                    continue;
                }
                let t0 = k as f32 / stripes as f32;
                let t1 = (k + 1) as f32 / stripes as f32;
                let a = p(x0 + (x1 - x0) * t0, y0, 0);
                let b = p(x0 + (x1 - x0) * t1, y0, 0);
                let c2 = p(x0 + (x1 - x0) * t1, y1, 0);
                let d2 = p(x0 + (x1 - x0) * t0, y1, 0);
                fb.fill_poly(&[a, b, c2, d2], palette::shade(base, 1.03));
            }
        }
        let i = 0.06;
        let rim = [
            p(x0 + i, y0 + i, 0),
            p(x1 - i, y0 + i, 0),
            p(x1 - i, y1 - i, 0),
            p(x0 + i, y1 - i, 0),
        ];
        fb.fill_poly(&rim, palette::shade(base, 1.04));
        // A gravel path: a cross for a small square, a ring walk for a big park.
        let path_c = palette::mix(look.wall_stone[1], look.dirt[5], 0.45);
        // A park path is a 3-px gravel band, not a hairline (B58b: a 1-px line
        // read as a scratch at 1x).
        let path = |fb: &mut Framebuffer, a: (f32, f32), b: (f32, f32)| {
            fb.line(a, b, path_c);
            fb.line((a.0, a.1 + 1.0), (b.0, b.1 + 1.0), palette::shade(path_c, 0.94));
            fb.line((a.0, a.1 + 2.0), (b.0, b.1 + 2.0), palette::shade(path_c, 1.04));
        };
        let bench = palette::shade(look.trunk, 0.8);
        let tree_ring = |me: &Self, fb: &mut Framebuffer, skip_front: bool| {
            let n = if small { 2 } else { 4 };
            for k in 0..n {
                let f = (k as f32 + 1.0) / (n as f32 + 1.0);
                me.park_tree_big(fb, p, x0 + w * f, y0 + 0.40, k as u8 + 1, look);
                if !(skip_front && k + 1 == n) {
                    me.park_tree_big(fb, p, x0 + 0.40, y0 + h * f, k as u8 + 5, look);
                }
            }
            me.park_tree_big(fb, p, x1 - 0.5, y1 - 0.4, 9, look);
        };
        match kind {
            map::ParkKind::Pond => {
                // The big park: a cross of paths and a pond, with trees all round
                // so nothing hides the water.
                path(fb, p(x0 + 0.4, cy, 0), p(x1 - 0.4, cy, 0));
                path(fb, p(cx, y0 + 0.4, 0), p(cx, y1 - 0.4, 0));
                let r = map::park_pond(self.w, self.h, kind).map(|p| p.2).unwrap_or(1.0);
                // A frozen pond in a tundra: ice, not a hole of open water.
                let water = if look.tex == crate::biome::GroundTex::Snow {
                    palette::mix(look.sand[4], look.water[1], 0.30)
                } else {
                    palette::shade(look.water[1], 0.82)
                };
                // A sandy bank: water in a park needs an edge you can see.
                let bank = palette::mix(look.wall_stone[1], look.dirt[4], 0.45);
                let n = 22;
                let mut bank_poly: Vec<(f32, f32)> = Vec::new();
                let mut pond: Vec<(f32, f32)> = Vec::new();
                for k in 0..n {
                    let a = k as f32 / n as f32 * std::f32::consts::TAU;
                    // A wobble on the radius: a park pond is an old, hand-dug
                    // shape, not a circle struck with dividers.
                    let wob = 1.0 + (a * 3.0 + 1.7).sin() * 0.10 + (a * 5.0).sin() * 0.06;
                    bank_poly.push(p(cx + a.cos() * (r + 0.18) * wob, cy + a.sin() * (r + 0.18) * wob, 0));
                    pond.push(p(cx + a.cos() * r * wob, cy + a.sin() * r * wob, 0));
                }
                fb.fill_poly(&bank_poly, bank);
                fb.fill_poly(&pond, water);
                let q = p(cx - r * 0.3, cy - r * 0.25, 0);
                fb.put(q.0 as i32, q.1 as i32, palette::shade(look.water[0], 1.25));
                tree_ring(self, fb, false);
            }
            map::ParkKind::Lawn => {
                // The classic square: cross paths, a drinking fountain, benches.
                path(fb, p(x0 + 0.3, cy, 0), p(x1 - 0.3, cy, 0));
                let q = p(cx, cy, 0);
                let (qx, qy) = (q.0 as i32, q.1 as i32);
                for dy in -2..=1 {
                    for dx in -3..=3 {
                        let a = (dx as f32 / 3.0).powi(2) + ((dy + 1) as f32 / 2.0).powi(2);
                        if a <= 1.0 {
                            fb.put(
                                qx + dx,
                                qy + dy,
                                if a > 0.55 {
                                    palette::shade(look.wall_stone[1], 1.05)
                                } else {
                                    look.water[1]
                                },
                            );
                        }
                    }
                }
                fb.put(qx, qy - 2, palette::shade(look.water[0], 1.4));
                fb.put(qx, qy - 3, look.wall_stone[1]);
                tree_ring(self, fb, false);
                let b = p(x0 + w * 0.62, y1 - 0.28, 0);
                fb.fill_rect(b.0 as i32, b.1 as i32, 3, 1, bench);
            }
            map::ParkKind::Playground => {
                // A hard-wearing play surface inside the green, a swing frame and
                // a slide, with a bench for the parents.
                let sand = palette::mix(look.sand[3], look.dirt[4], 0.35);
                let px0 = cx - w * 0.28;
                let py0 = cy - h * 0.24;
                let px1 = cx + w * 0.28;
                let py1 = cy + h * 0.24;
                let pit = [p(px0, py0, 0), p(px1, py0, 0), p(px1, py1, 0), p(px0, py1, 0)];
                fb.fill_poly(&pit, sand);
                fb.line(p(px0, py0, 0), p(px1, py0, 0), palette::shade(sand, 0.86));
                // Swings: two posts and a crossbar, two chains and seats.
                let sx = px0 + (px1 - px0) * 0.28;
                let sy = py0 + (py1 - py0) * 0.45;
                let post = palette::rgb(0x8a, 0x8e, 0x92);
                let (ga, gb) = (p(sx - 0.28, sy, 0), p(sx + 0.28, sy, 0));
                let hgt = 13.0;
                fb.line(ga, (ga.0, ga.1 - hgt), post);
                fb.line(gb, (gb.0, gb.1 - hgt), post);
                fb.line((ga.0, ga.1 - hgt), (gb.0, gb.1 - hgt), palette::shade(post, 0.9));
                for (k, c) in [(0.35f32, palette::rgb(0xc8, 0x4a, 0x42)), (0.68, palette::rgb(0x3e, 0x6c, 0xc0))] {
                    let ax = ga.0 + (gb.0 - ga.0) * k;
                    let ay = ga.1 + (gb.1 - ga.1) * k - hgt;
                    fb.line((ax, ay), (ax, ay + hgt * 0.55), palette::shade(post, 0.7));
                    fb.fill_rect(ax as i32 - 1, (ay + hgt * 0.55) as i32, 3, 1, c);
                }
                // Slide: a lit ladder and a run down to the sand.
                let lx = px1 - (px1 - px0) * 0.26;
                let ly = py0 + (py1 - py0) * 0.42;
                let base = p(lx, ly, 0);
                fb.line(base, (base.0, base.1 - 10.0), palette::shade(post, 1.05));
                fb.line(
                    (base.0, base.1 - 10.0),
                    (base.0 - 11.0, base.1 - 2.0),
                    palette::rgb(0xd8, 0xd2, 0xc4),
                );
                fb.line(
                    (base.0, base.1 - 9.0),
                    (base.0 - 11.0, base.1 - 1.0),
                    palette::shade(palette::rgb(0xd8, 0xd2, 0xc4), 0.8),
                );
                // A fence round the playground: posts every ~3 tiles.
                let fence = palette::shade(look.trunk, 0.75);
                let posts = ((px1 - px0).max(1.0) * 2.0) as i32;
                for k in 0..=posts {
                    let t = k as f32 / posts.max(1) as f32;
                    let q = p(px0 + (px1 - px0) * t, py1, 0);
                    fb.put(q.0 as i32, q.1 as i32, fence);
                }
                tree_ring(self, fb, false);
                let b = p(x0 + w * 0.30, y1 - 0.26, 0);
                fb.fill_rect(b.0 as i32, b.1 as i32, 3, 1, bench);
            }
            map::ParkKind::Sports => {
                // A pitch: mown, chalked lines, a centre circle and two goals.
                let m = 0.22;
                let ch = (x0 + m, y0 + m, x1 - m, y1 - m);
                let chalk = palette::rgb(0xe8, 0xea, 0xe2);
                let c0 = p(ch.0, ch.1, 0);
                let c1 = p(ch.2, ch.1, 0);
                let c2 = p(ch.2, ch.3, 0);
                let c3 = p(ch.0, ch.3, 0);
                fb.line(c0, c1, chalk);
                fb.line(c1, c2, chalk);
                fb.line(c2, c3, chalk);
                fb.line(c3, c0, chalk);
                let mid = p(cx, ch.1, 0);
                let mid2 = p(cx, ch.3, 0);
                fb.line(mid, mid2, palette::shade(chalk, 0.92));
                // Centre circle, drawn as a squashed ring.
                let rr = (w.min(h) * 0.13).clamp(0.5, 1.2);
                for k in 0..16 {
                    let a = k as f32 / 16.0 * std::f32::consts::TAU;
                    let q = p(cx + a.cos() * rr, cy + a.sin() * rr, 0);
                    fb.put(q.0 as i32, q.1 as i32, chalk);
                }
                // Goals at both ends: two posts and a net line.
                for (gx, back) in [(ch.0 + 0.05, -1.0f32), (ch.2 - 0.05, 1.0)] {
                    let a = p(gx, cy - h * 0.10, 0);
                    let b = p(gx, cy + h * 0.10, 0);
                    fb.line((a.0, a.1 - 3.0), (b.0, b.1 - 3.0), chalk);
                    fb.line((a.0, a.1), (a.0, a.1 - 3.0), chalk);
                    fb.line((b.0, b.1), (b.0, b.1 - 3.0), chalk);
                    let nb = p(gx + back * 0.18, cy + h * 0.10, 0);
                    let nt = p(gx + back * 0.18, cy - h * 0.10, 0);
                    fb.line((a.0, a.1 - 3.0), (nt.0, nt.1 - 3.0), palette::shade(chalk, 0.8));
                    fb.line((b.0, b.1 - 3.0), (nb.0, nb.1 - 3.0), palette::shade(chalk, 0.8));
                }
                if !small {
                    tree_ring(self, fb, false);
                }
            }
            map::ParkKind::Plaza => {
                // A civic square: paving ringed by trees, planters and benches.
                let planter = palette::shade(look.wall_stone[2], 1.02);
                for k in 0..3 {
                    let t = (k as f32 + 1.0) / 4.0;
                    let q = p(x0 + w * t, cy, 0);
                    let (qx, qy) = (q.0 as i32, q.1 as i32);
                    fb.fill_rect(qx - 3, qy - 1, 7, 2, planter);
                    fb.fill_rect(qx - 2, qy - 3, 5, 2, look.bush[0]);
                    fb.put(qx, qy - 4, look.flowers[0]);
                }
                // A statue on a plinth at the centre.
                let q = p(cx, cy, 0);
                let (qx, qy) = (q.0 as i32, q.1 as i32);
                fb.fill_rect(qx - 2, qy - 3, 5, 3, palette::shade(look.wall_stone[1], 1.05));
                fb.fill_rect(qx - 1, qy - 9, 3, 6, palette::shade(look.wall_stone[0], 1.1));
                fb.put(qx, qy - 11, palette::shade(look.wall_stone[0], 1.25));
                tree_ring(self, fb, false);
                let b = p(x0 + w * 0.5, y1 - 0.30, 0);
                fb.fill_rect(b.0 as i32 - 1, b.1 as i32, 4, 1, bench);
            }
            map::ParkKind::Garden => {
                // Allotments: beds in rows, a shed, a path down the middle.
                path(fb, p(cx, y0 + 0.3, 0), p(cx, y1 - 0.3, 0));
                let beds = 3;
                let soil = palette::shade(look.dirt[4], 0.96);
                let crop = look.leaf[1];
                for k in 0..beds {
                    let t = (k as f32 + 0.5) / beds as f32;
                    for side in [0.0f32, 0.55] {
                        let bx0 = x0 + 0.22 + w * side * 0.55;
                        let bx1 = bx0 + w * 0.26;
                        let by = y0 + h * (0.12 + t * 0.66);
                        let bh = h * 0.13;
                        let bed = [
                            p(bx0, by, 0),
                            p(bx1, by, 0),
                            p(bx1, by + bh, 0),
                            p(bx0, by + bh, 0),
                        ];
                        fb.fill_poly(&bed, soil);
                        // Rows of seedlings.
                        for r in 0..3 {
                            let f = (r as f32 + 0.5) / 3.0;
                            let a = p(bx0 + 0.03, by + bh * f, 0);
                            let b = p(bx1 - 0.03, by + bh * f, 0);
                            fb.line(a, b, palette::mix(soil, crop, 0.55));
                        }
                    }
                }
                // A shed at the back corner.
                let s = p(x1 - 0.9, y0 + 0.8, 0);
                let (sx, sy) = (s.0 as i32, s.1 as i32);
                // Walls, a gable roof, a dark door and a window: a tool shed.
                fb.fill_rect(sx - 4, sy - 7, 9, 7, palette::shade(pal::TIMBER, 0.92));
                fb.fill_rect(sx - 4, sy - 7, 9, 1, palette::shade(pal::TIMBER, 1.10));
                fb.fill_poly(
                    &[
                        (sx as f32 - 6.0, sy as f32 - 7.0),
                        (sx as f32 + 6.0, sy as f32 - 7.0),
                        (sx as f32 + 1.0, sy as f32 - 13.0),
                        (sx as f32 - 1.0, sy as f32 - 13.0),
                    ],
                    palette::shade(pal::ROOFS[2], 1.05),
                );
                fb.fill_rect(sx - 1, sy - 5, 3, 5, palette::shade(pal::DOOR, 0.85));
                if !small {
                    tree_ring(self, fb, true);
                }
            }
        }
    }

    /// The graveyard (B48): a quiet lawn inside a low stone wall, rows of
    /// headstones, a cross, a gravel path to the gate and two dark conifers.
    /// Small on purpose: it has to read at 1x as "the cemetery", not as a park
    /// with grey bits.
    fn draw_cemetery(
        &self,
        fb: &mut Framebuffer,
        p: &impl Fn(f32, f32, i32) -> (f32, f32),
        (x0, y0, x1, y1): (f32, f32, f32, f32),
        look: &'static crate::biome::Ground,
    ) {
        let (cx, cy) = ((x0 + x1) * 0.5, (y0 + y1) * 0.5);
        let (w, h) = (x1 - x0, y1 - y0);
        let lawn = palette::shade(palette::mix(look.grass[3], look.grass[5], 0.2), 0.86);
        let pad = [p(x0, y0, 0), p(x1, y0, 0), p(x1, y1, 0), p(x0, y1, 0)];
        tex::fill_poly_px(fb, &pad, |px, py, _u, _v| {
            let n = tex::hash2(px / 2, py / 3, 0x6E);
            if n > 0.82 {
                palette::shade(lawn, 1.05)
            } else if n < 0.16 {
                palette::shade(lawn, 0.93)
            } else {
                lawn
            }
        });
        // The path: from the middle of the front edge up to the cross.
        let path_c = palette::mix(look.wall_stone[1], look.dirt[5], 0.5);
        let (gx, gy) = match self.front % 4 {
            0 => (x1, cy),
            1 => (cx, y1),
            2 => (x0, cy),
            _ => (cx, y0),
        };
        fb.line(p(gx, gy, 0), p(cx, cy, 0), path_c);
        let q = p(gx, gy, 0);
        fb.line(
            p(gx + (cx - gx) * 0.5 + 0.04, gy + (cy - gy) * 0.5 + 0.04, 0),
            p(cx + 0.04, cy + 0.04, 0),
            path_c,
        );
        let _ = q;
        // Rows of headstones, two deep, left clear down the middle for the path.
        let stone = palette::mix(look.wall_stone[0], pal::SIDEWALK[0], 0.4);
        let shade = palette::shade(stone, 0.72);
        let moss = palette::shade(lawn, 0.8);
        let spots = [
            (0.18, 0.28), (0.18, 0.72), (0.34, 0.28), (0.34, 0.72),
            (0.66, 0.28), (0.66, 0.72), (0.82, 0.28), (0.82, 0.72),
        ];
        for (i, (fx, fy)) in spots.iter().enumerate() {
            let (sx, sy) = if self.front % 2 == 0 { (*fy, *fx) } else { (*fx, *fy) };
            let t = p(x0 + w * sx, y0 + h * sy, 0);
            let (tx, ty) = (t.0 as i32, t.1 as i32);
            // Turf mound, then the stone: two wide, three tall, lit on the left.
            fb.put(tx, ty, moss);
            fb.put(tx + 1, ty, moss);
            let hgt = 4 + (self.seed as usize + i * 7) % 2;
            fb.fill_rect(tx - 1, ty - hgt as i32, 2, hgt as i32, stone);
            fb.fill_rect(tx + 1, ty - hgt as i32 + 1, 1, hgt as i32 - 1, shade);
            fb.put(tx - 1, ty - hgt as i32 - 1, stone);
            fb.put(tx, ty - hgt as i32 - 1, stone);
        }
        // A cross at the middle, taller than the stones.
        let c = p(cx, cy - h * 0.1, 0);
        let (qx, qy) = (c.0 as i32, c.1 as i32);
        let white = palette::shade(stone, 1.12);
        fb.fill_rect(qx, qy - 9, 2, 9, white);
        fb.fill_rect(qx - 2, qy - 6, 6, 2, white);
        fb.fill_rect(qx + 2, qy - 8, 1, 8, shade);
        // A low wall round the lot: two courses of stone, with a gap at the gate.
        let wall = palette::shade(look.wall_stone[1], 0.95);
        let wall_hi = palette::shade(look.wall_stone[1], 1.1);
        let i = 0.04;
        let edges = [
            ((x0 + i, y0 + i), (x1 - i, y0 + i)),
            ((x1 - i, y0 + i), (x1 - i, y1 - i)),
            ((x1 - i, y1 - i), (x0 + i, y1 - i)),
            ((x0 + i, y1 - i), (x0 + i, y0 + i)),
        ];
        for (k, (a, b)) in edges.iter().enumerate() {
            // Skip the front edge's middle for the gate: the front edge is
            // the one the path leaves by.
            let front_edge = match self.front % 4 {
                3 => 0,
                0 => 1,
                1 => 2,
                _ => 3,
            };
            let (pa, pb) = (p(a.0, a.1, 0), p(b.0, b.1, 0));
            let courses = |fb: &mut Framebuffer, a: (f32, f32), b: (f32, f32)| {
                for z in 0..4 {
                    fb.line(
                        p(a.0, a.1, z),
                        p(b.0, b.1, z),
                        if z == 3 { wall_hi } else if z == 1 { palette::shade(wall, 0.88) } else { wall },
                    );
                }
            };
            let _ = (pa, pb);
            if k == front_edge {
                let m1 = (a.0 + (b.0 - a.0) * 0.38, a.1 + (b.1 - a.1) * 0.38);
                let m2 = (a.0 + (b.0 - a.0) * 0.62, a.1 + (b.1 - a.1) * 0.62);
                courses(fb, *a, m1);
                courses(fb, m2, *b);
                // Gate posts, a little taller than the wall.
                for m in [m1, m2] {
                    let q = p(m.0, m.1, 0);
                    fb.fill_rect(q.0 as i32, q.1 as i32 - 6, 2, 6, wall_hi);
                }
            } else {
                courses(fb, *a, *b);
            }
        }
        // Two dark conifers at the back corners.
        let dark = palette::shade(look.leaf[2], 0.72);
        let mid = palette::shade(look.leaf[1], 0.8);
        for (tx, ty) in [(x0 + 0.34, y0 + 0.30), (x1 - 0.34, y0 + 0.30)] {
            let b = p(tx, ty, 0);
            let (bx, by) = (b.0 as i32, b.1 as i32);
            fb.fill_rect(bx, by - 2, 1, 2, look.trunk);
            for k in 0..11 {
                let half = if k < 2 { 1 } else if k < 8 { 2 } else { 1 };
                fb.fill_rect(bx - half, by - 3 - k, half * 2 + 1, 1, if k % 3 == 0 { mid } else { dark });
            }
        }
    }

    /// A park tree: a 2 px trunk and a round two-tone canopy, about 9 px wide
    /// so it reads at 1x next to the terrain's trees.
    fn park_tree_big(
        &self,
        fb: &mut Framebuffer,
        p: &impl Fn(f32, f32, i32) -> (f32, f32),
        x: f32,
        y: f32,
        v: u8,
        look: &'static crate::biome::Ground,
    ) {
        let q = p(x, y, 0);
        let (qx, qy) = (q.0 as i32, q.1 as i32);
        // B69b: the park's trees sway too. Drawn once into the sway layer's
        // sprite store; the lot keeps only the silhouette.
        if crate::sway::active() {
            let class = match look.biome {
                map::Biome::Desert => crate::sway::Class::Palm,
                map::Biome::Tundra => crate::sway::Class::Conifer,
                _ => crate::sway::Class::Broad,
            };
            let done = crate::sway::with_sink(|l| {
                let scratch = l.scratch(qx, qy);
                Self::park_tree_body(scratch, qx, qy, v, look);
                l.insert_with(fb, (crate::sway::PARK_KEY + qx, qy), (qx, qy), class, 0.9);
            });
            if done.is_some() {
                return;
            }
        }
        Self::park_tree_body(fb, qx, qy, v, look);
    }

    fn park_tree_body(
        fb: &mut Framebuffer,
        qx: i32,
        qy: i32,
        v: u8,
        look: &'static crate::biome::Ground,
    ) {
        let trunk = 10;
        // D66: a park is planted with the land's own trees — palms in an oasis
        // garden, spruces on the tundra, broadleaves anywhere else.
        match look.biome {
            map::Biome::Desert => {
                fb.fill_rect(qx, qy - trunk, 2, trunk, palette::shade(look.trunk, 1.05));
                let (lx, d) = (look.leaf[0], look.leaf[2]);
                for f in 0..6 {
                    let a = std::f32::consts::PI + (f as f32 + 0.5) / 6.0 * std::f32::consts::PI;
                    let (mut px, mut py) = (qx as f32, (qy - trunk) as f32);
                    for k in 0..9 {
                        px += a.cos() * 1.2;
                        py += a.sin() * 1.2 + (k as f32) * 0.12;
                        fb.put(px as i32, py as i32, if k % 3 == 0 { lx } else { d });
                    }
                }
                return;
            }
            map::Biome::Tundra => {
                fb.fill_rect(qx, qy - 4, 2, 5, look.trunk);
                let (l, m, d) = (look.needles[0], look.needles[1], look.needles[2]);
                let height = 22;
                for k in 0..height {
                    let t = k as f32 / height as f32;
                    let half = ((1.0 - t) * 7.0) as i32 + 1;
                    let yy = qy - 4 - k;
                    let c = if (k / 4) % 2 == 0 { m } else { l };
                    fb.fill_rect(qx - half, yy, half * 2 + 1, 1, c);
                    if half > 1 {
                        fb.fill_rect(qx + half / 2, yy, half, 1, d);
                    }
                }
                return;
            }
            _ => {}
        }
        fb.fill_rect(qx, qy - trunk, 3, trunk, look.trunk);
        let top = qy - trunk - 12;
        let (light, mid, dark) = (look.leaf[0], look.leaf[1], look.leaf[2]);
        let skew = (v % 3) as i32 - 1;
        for dy in 0..14 {
            let t = (dy as f32 + 0.5) / 14.0;
            let round = (1.0 - (2.0 * t - 1.0).powi(2)).max(0.0).sqrt();
            let half = (9.0 * round).max(1.0) as i32;
            let yy = top + dy;
            let x0 = qx - half + 1 + skew;
            let x1 = qx + half + skew;
            fb.fill_rect(x0, yy, (x1 - x0 + 1).max(1), 1, if t < 0.45 { light } else { mid });
            let df = qx + (half as f32 * 0.35) as i32 + skew;
            if x1 > df {
                fb.fill_rect(df, yy, (x1 - df).max(1), 1, dark);
            }
        }
    }

    /// Pitched roof: a ridge with a lit and a shaded slope, gable ends, a 2-px
    /// eave overhang and shingle courses (B32). Barns get a gambrel profile.
    fn roof(
        &self,
        fb: &mut Framebuffer,
        p: &impl Fn(f32, f32, i32) -> (f32, f32),
        (x0, y0, x1, y1): (f32, f32, f32, f32),
        h: i32,
        snow: f32,
        fin: &Finish,
    ) {
        // B76: a roof under snow. Every slope is whitened by the same rule the
        // ground uses, so the roof and the garden can never disagree.
        let roof = if snow > 0.02 {
            palette::mix(fin.roof, palette::rgb(0xe8, 0xf0, 0xf8), (0.85 * snow).clamp(0.0, 0.90))
        } else {
            fin.roof
        };
        let wall = fin.wall;
        let salt = self.seed;
        // A 2-px eave all round, and a pitch that reads at 1x.
        let (ex0, ey0, ex1, ey1) = (x0 - 0.04, y0 - 0.04, x1 + 0.04, y1 + 0.04);
        let rise = ((ex1 - ex0).max(ey1 - ey0) * 2.2).clamp(6.0, 14.0) as i32;
        let along_x = (ex1 - ex0) >= (ey1 - ey0);
        if self.kind == ObjKind::Barn {
            // Gambrel: a shallow upper slope and a steep lower one.
            let mid = 1 + rise / 3;
            if along_x {
                let ym = (ey0 + ey1) * 0.5;
                let ra = p(ex0, ym, h + rise);
                let rb = p(ex1, ym, h + rise);
                roof_slope(
                    fb,
                    [p(ex0, ey0, h + mid), p(ex1, ey0, h + mid), ra, rb],
                    palette::shade(roof, 1.06),
                    salt,
                );
                roof_slope(
                    fb,
                    [p(ex1, ey1, h), p(ex0, ey1, h), rb, ra],
                    palette::shade(roof, 0.74),
                    salt,
                );
                fb.fill_poly(
                    &[p(ex0, ey0, h), p(ex1, ey0, h), p(ex1, ey0, h + mid), p(ex0, ey0, h + mid)],
                    palette::shade(roof, 0.92),
                );
                fb.fill_poly(
                    &[p(ex1, ey1, h), p(ex0, ey1, h), p(ex0, ey1, h + mid), p(ex1, ey1, h + mid)],
                    palette::shade(roof, 0.66),
                );
            } else {
                let xm = (ex0 + ex1) * 0.5;
                let ra = p(xm, ey0, h + rise);
                let rb = p(xm, ey1, h + rise);
                roof_slope(
                    fb,
                    [p(ex0, ey0, h + mid), p(ex0, ey1, h + mid), ra, rb],
                    palette::shade(roof, 0.92),
                    salt,
                );
                roof_slope(
                    fb,
                    [p(ex1, ey0, h + mid), p(ex1, ey1, h + mid), ra, rb],
                    palette::shade(roof, 1.04),
                    salt,
                );
                fb.fill_poly(
                    &[p(ex0, ey1, h), p(ex1, ey1, h), p(ex1, ey1, h + mid), p(ex0, ey1, h + mid)],
                    palette::shade(roof, 0.68),
                );
            }
            return;
        }
        // House / shop / store / church body: a gable (or hip when the plot is
        // square and the seed says so).
        let hip = self.h(0x51) > 0.72 && (ex1 - ex0 - (ey1 - ey0)).abs() < 0.5;
        let (ra, rb);
        if along_x {
            let ym = (ey0 + ey1) * 0.5;
            ra = p(ex0 + if hip { 0.25 } else { 0.0 }, ym, h + rise);
            rb = p(ex1 - if hip { 0.25 } else { 0.0 }, ym, h + rise);
            // Shaded slope (towards the viewer) and lit slope (away).
            roof_slope(
                fb,
                [p(ex1, ey1, h), p(ex0, ey1, h), ra, rb],
                palette::shade(roof, 0.70),
                salt,
            );
            roof_slope(
                fb,
                [p(ex0, ey0, h), p(ex1, ey0, h), rb, ra],
                palette::shade(roof, 1.16),
                salt,
            );
            if !hip {
                fb.fill_poly(
                    &[p(ex0, ey0, h), p(ex0, ym, h + rise), p(ex0, ey1, h)],
                    palette::shade(wall, 0.90),
                );
                fb.fill_poly(
                    &[p(ex1, ey0, h), p(ex1, ym, h + rise), p(ex1, ey1, h)],
                    palette::shade(wall, 0.66),
                );
            }
            fb.line(ra, rb, pal::OUTLINE);
        } else {
            let xm = (ex0 + ex1) * 0.5;
            ra = p(xm, ey0 + if hip { 0.25 } else { 0.0 }, h + rise);
            rb = p(xm, ey1 - if hip { 0.25 } else { 0.0 }, h + rise);
            roof_slope(
                fb,
                [p(ex0, ey0, h), p(ex0, ey1, h), ra, rb],
                palette::shade(roof, 0.92),
                salt,
            );
            roof_slope(
                fb,
                [p(ex1, ey0, h), p(ex1, ey1, h), ra, rb],
                palette::shade(roof, 1.06),
                salt,
            );
            if !hip {
                fb.fill_poly(
                    &[p(ex0, ey0, h), p(xm, ey0, h + rise), p(ex1, ey0, h)],
                    palette::shade(wall, 0.88),
                );
                fb.fill_poly(
                    &[p(ex1, ey1, h), p(xm, ey1, h + rise), p(ex0, ey1, h)],
                    palette::shade(wall, 0.66),
                );
            }
            fb.line(ra, rb, pal::OUTLINE);
        }
        // 2-px silhouette outline: eaves plus the two visible wall corners.
        fb.line(p(ex0, ey1, h), p(ex1, ey1, h), pal::OUTLINE);
        fb.line(p(ex0, ey1, h), p(ex0, ey1, h - 2), pal::OUTLINE);
        fb.line(p(ex1, ey0, h), p(ex1, ey1, h), pal::OUTLINE);
        fb.line(p(ex1, ey1, h), p(ex1, ey1, h - 2), pal::OUTLINE);
        // Ridge cap: a lit tile row along the top.
        fb.line(
            (ra.0, ra.1 - 1.0),
            (rb.0, rb.1 - 1.0),
            palette::shade(roof, 1.28),
        );

        // Church steeple.
        if self.kind == ObjKind::Church {
            let ch = 0.32;
            let tower = [p(ex0, ey0, h), p(ex0 + ch, ey0, h), p(ex0 + ch, ey0 + ch, h), p(ex0, ey0 + ch, h)];
            fb.fill_poly(&tower, palette::shade(wall, 0.82));
            let spire = [
                p(ex0, ey0, h + 20),
                p(ex0 + ch * 0.5, ey0 + ch * 0.5, h + 38),
                p(ex0 + ch, ey0, h + 20),
            ];
            fb.fill_poly(&spire, palette::shade(roof, 1.0));
            let q = p(ex0 + ch * 0.5, ey0 + ch * 0.5, h + 38);
            fb.line((q.0, q.1), (q.0, q.1 - 8.0), pal::ROAD_LINE);
            fb.put(q.0 as i32, q.1 as i32 - 8, pal::WINDOW_LIT);
        }
        let _ = rise;
    }

    /// Doors, shop fronts, porches and awnings at 64-px detail (B32). The
    /// window grid itself is part of the wall texture.
    fn details(
        &self,
        fb: &mut Framebuffer,
        p: &impl Fn(f32, f32, i32) -> (f32, f32),
        (x0, y0, x1, y1): (f32, f32, f32, f32),
        h: i32,
        fin: &Finish,
    ) {
        let glow = fin.glow;
        let door_h = 8 * D;
        let door_w = 5 * D;
        // The street side of the plot.
        let (fx, fy, along) = match self.front {
            0 => (x1, (y0 + y1) * 0.5, 1),
            1 => ((x0 + x1) * 0.5, y1, 0),
            2 => (x0, (y0 + y1) * 0.5, 1),
            _ => ((x0 + x1) * 0.5, y0, 0),
        };
        // --- door: a recessed panel with a step and a lit fanlight ---------
        let d = p(fx, fy, 1);
        let (dx, dy) = (d.0 as i32, d.1 as i32);
        fb.fill_rect(dx - door_w / 2, dy - door_h, door_w, door_h, palette::shade(pal::DOOR, 1.12));
        fb.fill_rect(dx - door_w / 2 + 1, dy - door_h + 1, door_w - 2, door_h - 2, pal::DOOR);
        // Two panels and a brass handle.
        fb.fill_rect(dx - door_w / 2 + 2, dy - door_h + 3, door_w - 4, 2, palette::shade(pal::DOOR, 0.82));
        fb.put(dx + door_w / 2 - 2, dy - door_h / 2, pal::LAMP_GLOW);
        // A stone step, one pixel proud of the wall.
        fb.fill_rect(dx - door_w / 2 - 1, dy, door_w + 2, 2, palette::shade(pal::SIDEWALK[1], 0.92));
        // --- shop front: a wide glazed window beside the door --------------
        if matches!(self.kind, ObjKind::Store | ObjKind::Shop) {
            let a = p(if along == 1 { fx } else { x0 + 0.06 }, if along == 1 { y0 + 0.10 } else { fy }, 3);
            let b = p(if along == 1 { fx } else { x1 - 0.06 }, if along == 1 { y1 - 0.10 } else { fy }, 3);
            let c = p(if along == 1 { fx } else { x1 - 0.06 }, if along == 1 { y1 - 0.10 } else { fy }, h - 4);
            let e = p(if along == 1 { fx } else { x0 + 0.06 }, if along == 1 { y0 + 0.10 } else { fy }, h - 4);
            let glass = if glow > 0.5 {
                palette::mix(pal::WINDOW_LIT, pal::WINDOW_HALO, 0.35)
            } else {
                pal::WINDOW_DARK
            };
            fb.fill_poly(&[a, b, c, e], glass);
            // Mullions and a fascia band above the glazing.
            let m1 = p(if along == 1 { fx } else { (x0 + x1) * 0.5 }, if along == 1 { (y0 + y1) * 0.5 } else { fy }, 3);
            let m2 = p(if along == 1 { fx } else { (x0 + x1) * 0.5 }, if along == 1 { (y0 + y1) * 0.5 } else { fy }, h - 4);
            fb.line(m1, m2, palette::shade(pal::DOOR, 1.1));
            // Awning across the shop front.
            let aw = 0.16;
            let a2 = p(x0, y1, h + 2);
            let b2 = p(x1, y1, h + 2);
            let c2 = p(x1, y1 + aw, h - 4);
            let e2 = p(x0, y1 + aw, h - 4);
            let stripe = palette::shade(pal::ROOFS[0], 1.15);
            fb.fill_poly(&[a2, b2, c2, e2], stripe);
            // Stripes across the awning.
            for i in 1..4 {
                let t = i as f32 / 4.0;
                let pa = (a2.0 + (b2.0 - a2.0) * t, a2.1 + (b2.1 - a2.1) * t);
                let pb = (e2.0 + (c2.0 - e2.0) * t, e2.1 + (c2.1 - e2.1) * t);
                fb.line(pa, pb, palette::shade(stripe, 0.82));
            }
        }
        // --- porch light on some houses (B16) ------------------------------
        if glow > 0.4 && self.seed % 3 == 0 {
            let q = p(fx, fy, 6);
            fb.fill_rect(q.0 as i32 - 3, q.1 as i32 - 3, 6, 6, pal::WINDOW_HALO);
            fb.fill_rect(q.0 as i32 - 1, q.1 as i32 - 2, 3, 3, pal::WINDOW_LIT);
            fb.put(q.0 as i32, q.1 as i32 - 3, palette::shade(pal::WINDOW_LIT, 1.2));
        }
        // --- a porch canopy over the door on detached houses ---------------
        if self.kind == ObjKind::House && self.h(0x31) > 0.55 {
            let q = p(fx, fy, h - 2);
            let (qx, qy) = (q.0 as i32, q.1 as i32);
            fb.fill_rect(qx - door_w, qy - 2, door_w * 2, 2, palette::shade(pal::ROOFS[1], 1.1));
            fb.line(
                (qx as f32 - door_w as f32, qy as f32 - 2.0),
                (qx as f32 - door_w as f32, qy as f32 + 1.0),
                pal::TIMBER,
            );
            fb.line(
                (qx as f32 + door_w as f32, qy as f32 - 2.0),
                (qx as f32 + door_w as f32, qy as f32 + 1.0),
                pal::TIMBER,
            );
        }
    }
}

/// Two shaded walls plus a flat cap (used by the early stages).
/// Wall and roof colour per district style (B26 placeholder palette).
fn style_colours(style: u8) -> (Color, Color) {
    match style {
        1 => (pal::STYLE_STUCCO, pal::STYLE_TILE),   // European quarter
        2 => (pal::STYLE_SIDING, pal::STYLE_SHINGLE), // suburbs
        3 => (pal::STYLE_GLASS, pal::STYLE_STEEL),   // downtown
        4 => (pal::STYLE_VILLA, pal::STYLE_TILE),    // high-end
        5 => (pal::STYLE_BRICK, pal::STYLE_STEEL),   // industrial
        _ => (pal::WALLS[1], pal::ROOFS[0]),         // the old core
    }
}

/// Everything the wall and roof art needs that is not geometry (B32).
#[derive(Clone, Copy)]
struct Finish {
    wall: Color,
    roof: Color,
    /// District style: 0 brick core, 1 stucco, 2 siding, 3 glass, 4 villa,
    /// 5 industrial. Picks the surface texture.
    style: u8,
    seed: u32,
    /// 0..1 night lighting: how dark the night is (from the sky palette).
    glow: f32,
    /// Local hour, 0..24.
    hour: f32,
    /// B72: which city object this is (its panes are recorded against it).
    obj: u32,
    /// Pixels per storey, so window rows line up with the massing height.
    floor_px: i32,
    /// Punched windows on the walls (false while a building is a shell).
    windows: bool,
}

impl Finish {
    /// B72 (D78): the window grid on one wall face, from the face's own screen
    /// geometry. The panes are recorded against the object so the scene can
    /// switch a light without touching the static layer's build.
    fn face(&self, q: [Pt; 4], wpx: f32, hpx: f32, side: u8) -> crate::windows::Face {
        use crate::windows::{Face, FaceMap, Grid};
        // `tex::wall_quad_px` normalises a right-to-left quad, so the mapping
        // has to be normalised the same way.
        let flip = q[1].0 < q[0].0;
        let (tl, tr, bl, br) = if flip { (q[1], q[0], q[3], q[2]) } else { (q[0], q[1], q[2], q[3]) };
        let map = FaceMap {
            left: tl.0,
            dx: (tr.0 - tl.0).max(0.001),
            top_l: tl.1,
            top_r: tr.1,
            span_l: bl.1 - tl.1,
            span_r: br.1 - tr.1,
            flip,
            wpx,
            hpx,
        };
        let d = D as f32;
        let grid = if self.style == 3 {
            Grid::Glass {
                bay: 6.0 * d,
                floor: 8.0 * d,
            }
        } else {
            let floor = self.floor_px.max(6) as f32;
            let pitch = 14.0 * d;
            let ww = 8.0 * d;
            let wh = 10.0 * d;
            let n = (wpx / pitch).round().max(1.0);
            let top = (floor - wh).max(2.0) * 0.55;
            Grid::Punched {
                floor,
                ww,
                wh,
                top,
                n,
            }
        };
        Face {
            map,
            side,
            grid,
            wall: self.wall,
            seed: self.seed,
        }
    }
}

/// One vertical wall face, textured and shaded: `q` is `[top-left, top-right,
/// bottom-left, bottom-right]` in screen space.
///
/// B72: the face also carries the window grid, and its panes are handed to the
/// framebuffer's window sink so the scene can switch one light later without
/// rebuilding anything. Every pane is painted **dark** here: the lit ones are a
/// separate pass (`Scene::paint_windows`), which is what makes a light cheap.
fn wall_face(fb: &mut Framebuffer, q: [Pt; 4], f: &Finish, side: u8) {
    let wpx = ((q[1].0 - q[0].0).powi(2) + (q[1].1 - q[0].1).powi(2)).sqrt();
    let hpx = ((q[2].0 - q[0].0).powi(2) + (q[2].1 - q[0].1).powi(2)).sqrt();
    if wpx < 2.0 || hpx < 2.0 {
        return;
    }
    let face = f.face(q, wpx, hpx, side);
    let windows = f.windows;
    tex::wall_quad_px(fb, q[0], q[1], q[2], q[3], |px, py, u, v| {
        wall_px(f, &face, windows, px, py, u, v, wpx, hpx, side)
    });
    if windows {
        fb.push_face(f.obj, face);
    }
}

/// One pixel of a wall: material texture, then the window grid, with the face's
/// own shading baked in (left face lit, right face in shade).
#[allow(clippy::too_many_arguments)]
fn wall_px(
    f: &Finish,
    face: &crate::windows::Face,
    windows: bool,
    px: i32,
    py: i32,
    u: f32,
    v: f32,
    wpx: f32,
    hpx: f32,
    side: u8,
) -> Color {
    use crate::windows::Hit;
    let shade = if side == 0 { 0.98 } else { 0.74 };
    let up = u * wpx;
    let vp = v * hpx;
    let base = match f.style {
        3 => f.wall,
        5 => corrugated_px(f.wall, px, py, up, vp),
        2 => siding_px(f.wall, up, vp),
        1 | 4 => stucco_px(f.wall, px, py),
        _ => brick_px(f.wall, px, py, up, vp),
    };
    let mut c = palette::shade(base, shade);
    // A stone plinth grounds the wall (D41: definition at 1x).
    if vp > hpx - 3.0 * D as f32 {
        let plinth = palette::shade(pal::WALL_STONE[2], shade);
        c = if vp > hpx - 1.0 { palette::shade(plinth, 0.86) } else { plinth };
    }
    if !windows {
        return c;
    }
    // The window grid owns the pane pixels; the sills and their shadows are the
    // wall colour darkened, so they keep the texture.
    match face.pane_px(up, vp, false) {
        Hit::Ink(c2) => c2,
        Hit::Sill => palette::shade(pal::SIDEWALK[0], shade * 1.02),
        Hit::Dim(k) => palette::shade(c, k),
        Hit::Wall => c,
    }
}

/// One roof slope: a quad filled with shingle courses.
fn roof_slope(fb: &mut Framebuffer, q: [Pt; 4], base: Color, salt: u32) {
    let wpx = ((q[1].0 - q[0].0).powi(2) + (q[1].1 - q[0].1).powi(2)).sqrt();
    let hpx = ((q[2].0 - q[0].0).powi(2) + (q[2].1 - q[0].1).powi(2)).sqrt();
    if wpx < 2.0 || hpx < 2.0 {
        return;
    }
    tex::quad_px(fb, q[0], q[1], q[2], q[3], |px, py, u, v| {
        let _ = (px, py);
        shingle_px(base, u, v, wpx, hpx, salt)
    });
}

/// Shingles / slates: courses with staggered tabs and a shadow line under each.
fn shingle_px(base: Color, u: f32, v: f32, wpx: f32, hpx: f32, salt: u32) -> Color {
    let course_h = 3.0 * D as f32;
    let vp = v * hpx;
    let up = u * wpx;
    let course = (vp / course_h).floor();
    let cv = vp - course * course_h;
    let tab_w = 6.0 * D as f32;
    let stagger = if (course as i32) & 1 == 0 { 0.0 } else { tab_w * 0.5 };
    let bu = up + stagger;
    let tab = (bu / tab_w).floor();
    let cu = bu - tab * tab_w;
    let tint = tex::hash2(tab as i32, course as i32, salt);
    let c = palette::jitter(base, tint, 0.11);
    if cv < 1.0 {
        palette::shade(c, 0.78) // the shadow under the course above
    } else if cu < 0.9 {
        palette::shade(c, 0.88) // the tab joint
    } else if cv > course_h - 1.0 {
        palette::shade(c, 1.05) // the lit lower edge of the tab
    } else {
        c
    }
}

/// Brick: courses 4 px high with staggered joints and per-brick tint.
fn brick_px(base: Color, _px: i32, _py: i32, up: f32, vp: f32) -> Color {
    let course_h = 3.0 * D as f32;
    let course = (vp / course_h).floor();
    let cv = vp - course * course_h;
    let stagger = if (course as i32) & 1 == 0 { 0.0 } else { 3.5 * D as f32 };
    let brick_w = 7.0 * D as f32;
    let bu = up + stagger;
    let col = (bu / brick_w).floor();
    let cu = bu - col * brick_w;
    let tint = tex::hash2(col as i32, course as i32, 0xf1);
    let c = palette::jitter(base, tint, 0.10);
    if cv < 1.0 || cu < 0.9 {
        palette::shade(c, 0.80) // mortar joint
    } else if cv > course_h - 1.0 {
        palette::shade(c, 1.06) // the lit top of the brick
    } else {
        c
    }
}

/// Clapboard siding: horizontal boards with a shadow under each.
fn siding_px(base: Color, _up: f32, vp: f32) -> Color {
    let board = 3.0 * D as f32;
    let row = (vp / board).floor();
    let cv = vp - row * board;
    let tint = tex::hash1(row as u32 * 7 + 3);
    let c = palette::jitter(base, tint, 0.05);
    if cv < 1.0 {
        palette::shade(c, 0.84)
    } else if cv > board - 1.5 {
        palette::shade(c, 1.05)
    } else {
        c
    }
}

/// Stucco / render: grain with a few patches where the coat is uneven.
fn stucco_px(base: Color, px: i32, py: i32) -> Color {
    let patch = tex::hash2(px / 4, py / 4, 0xf3);
    let c = if patch > 0.90 {
        palette::shade(base, 1.05)
    } else if patch < 0.10 {
        palette::shade(base, 0.95)
    } else {
        base
    };
    let g = tex::hash2(px, py, 0xf4);
    if g > 0.90 {
        palette::shade(c, 1.04)
    } else if g < 0.10 {
        palette::shade(c, 0.96)
    } else {
        c
    }
}

/// Corrugated metal (warehouses): vertical ribs every 4 px.
fn corrugated_px(base: Color, _px: i32, _py: i32, up: f32, _vp: f32) -> Color {
    let rib = 4.0 * D as f32;
    let cu = up % rib;
    if cu < 1.0 {
        palette::shade(base, 0.86)
    } else if cu > rib - 1.5 {
        palette::shade(base, 1.07)
    } else {
        base
    }
}

/// A flat roof's parapet: a lit edge on the sunlit sides and a dark line on the
/// others, so a flat cap reads as a roof rather than a floating diamond (B32).
fn parapet(fb: &mut Framebuffer, cap: &[Pt; 4], roof: Color, snow: f32) {
    // cap order: (x0,y0) (x1,y0) (x1,y1) (x0,y1)
    // B76 r3: the coping is the lip the snow hangs over, so its two visible
    // lines turn white while the parapet's own shading stays.
    let lip = |c: Color| {
        if snow > 0.15 {
            palette::mix(c, palette::rgb(0xf0, 0xf6, 0xfd), (0.25 + 0.6 * snow).min(0.88))
        } else {
            c
        }
    };
    fb.line(cap[0], cap[1], lip(palette::shade(roof, 1.30)));
    fb.line(cap[1], cap[2], lip(palette::shade(roof, 0.72)));
    fb.line(cap[2], cap[3], lip(palette::shade(roof, 0.72)));
    fb.line(cap[3], cap[0], lip(palette::shade(roof, 1.18)));
    // A 1-px inset: the coping itself.
    let inset = 0.06;
    let c = |a: Pt, b: Pt, t: f32| (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
    let p0 = c(cap[0], cap[2], inset);
    let p1 = c(cap[1], cap[3], inset);
    let p2 = c(cap[2], cap[0], inset);
    let p3 = c(cap[3], cap[1], inset);
    fb.line(p0, p1, lip(palette::shade(roof, 1.16)));
    fb.line(p3, p2, lip(palette::shade(roof, 0.82)));
}

/// B76 r3: the load on a flat roof plane — an even layer with a brighter drift
/// along the parapet and a wet patch or two where the roof warms from inside.
fn snowy_cap(fb: &mut Framebuffer, cap: &[Pt; 4], base: Color, snow: f32, heat: f32, seed: u32) {
    if snow <= 0.02 {
        fb.fill_poly(cap, base);
        return;
    }
    let cover = (snow * (1.0 - 0.22 * heat)).clamp(0.0, 1.0);
    let face = palette::mix(base, palette::rgb(0xf4, 0xf8, 0xff), (0.96 * cover).min(0.97));
    fb.fill_poly(cap, face);
    if cover <= 0.12 {
        return;
    }
    // The drift: the load banks up where the roof meets the parapet, which on
    // this side of the diamond is the far two edges.
    let drift = palette::mix(face, palette::rgb(0xff, 0xff, 0xff), 0.55);
    fb.line(cap[0], cap[1], drift);
    fb.line(cap[0], cap[3], drift);
    // Melted back to wet paving, in a few patches, on a warm roof.
    if heat > 0.5 {
        let mut h = seed.wrapping_mul(0x9E37_79B1) | 1;
        for _ in 0..3 {
            h ^= h << 13;
            h ^= h >> 17;
            h ^= h << 5;
            let fx = ((h & 0xFF) as f32 / 255.0) * 0.5 + 0.2;
            let fy = (((h >> 8) & 0xFF) as f32 / 255.0) * 0.5 + 0.2;
            let cx = cap[0].0 + (cap[1].0 - cap[0].0) * fx + (cap[3].0 - cap[0].0) * fy;
            let cy = cap[0].1 + (cap[1].1 - cap[0].1) * fx + (cap[3].1 - cap[0].1) * fy;
            let wet = palette::mix(face, palette::rgb(0x6a, 0x6f, 0x74), 0.45 * heat);
            fb.fill_rect(cx as i32, cy as i32, 2, 1, wet);
            fb.fill_rect(cx as i32 + 1, cy as i32 + 1, 2, 1, wet);
        }
    }
}

/// Snow whitening for a top face (B27): a mix towards a cold white.
fn snow_white(c: Color, snow: f32) -> Color {
    if snow <= 0.01 {
        return c;
    }
    palette::mix(c, palette::rgb(0xea, 0xf0, 0xf6), (snow * 0.9).clamp(0.0, 0.92))
}

/// B59: which car place a plot is, if it is one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum CarPlace {
    CarPark,
    CarParkDeck,
    GasStation,
    ChargingHub,
    DriveThrough,
    CarWash,
    Dealership,
    Motel,
}

fn car_place_kind(k: ObjKind) -> Option<CarPlace> {
    Some(match k {
        ObjKind::CarPark => CarPlace::CarPark,
        ObjKind::CarParkDeck => CarPlace::CarParkDeck,
        ObjKind::GasStation => CarPlace::GasStation,
        ObjKind::ChargingHub => CarPlace::ChargingHub,
        ObjKind::DriveThrough => CarPlace::DriveThrough,
        ObjKind::CarWash => CarPlace::CarWash,
        ObjKind::Dealership => CarPlace::Dealership,
        ObjKind::Motel => CarPlace::Motel,
        _ => return None,
    })
}

fn walls(
    fb: &mut Framebuffer,
    p: &impl Fn(f32, f32, i32) -> (f32, f32),
    (x0, y0, x1, y1): (f32, f32, f32, f32),
    z0: i32,
    z1: i32,
    f: &Finish,
) {
    // The two visible faces: the +y face (screen-left, lit) and the +x face
    // (screen-right, in shade).
    let left = [p(x0, y1, z1), p(x1, y1, z1), p(x0, y1, z0), p(x1, y1, z0)];
    let right = [p(x1, y0, z1), p(x1, y1, z1), p(x1, y0, z0), p(x1, y1, z0)];
    wall_face(fb, left, f, 0);
    wall_face(fb, right, f, 1);
    let cap = [p(x0, y0, z1), p(x1, y0, z1), p(x1, y1, z1), p(x0, y1, z1)];
    fb.fill_poly(&cap, f.roof);
}

#[cfg(test)]
mod landmark_art_tests {
    use super::*;

    fn obj(kind: ObjKind, w: i32, h: i32) -> CityObj {
        CityObj {
            x: 10,
            y: 10,
            w,
            h,
            level: 4,
            kind,
            storeys: 3,
            style: 0,
            stage: Stage::Finished,
            seed: 9,
            front: 0,
            t: 0.0,
            kit: 0, wear: Default::default(),
        }
    }

    /// B35: the plaza is engine art — it must paint its plot.
    #[test]
    fn the_plaza_paints_its_plot() {
        // The lot at (10,10) projects to y = oy + 20*16 - 4*12, so the buffer has
        // to be big enough to hold it.
        let mut fb = Framebuffer::new(500, 500);
        fb.clear(0xFF00_0000);
        let cam = Camera::new(200, 40);
        let before = fb.px().to_vec();
        obj(ObjKind::Landmark(LandmarkKind::Plaza), 5, 5).draw(&mut fb, &cam, 0.0);
        let diff = before.iter().zip(fb.px()).filter(|(a, b)| a != b).count();
        assert!(diff > 2000, "the plaza painted only {diff} pixels");
    }
}
