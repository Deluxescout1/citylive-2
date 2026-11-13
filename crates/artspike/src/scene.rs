//! The B25 scene: a boulevard with a tree median crossing a side street,
//! the four showcase buildings + glass tower around the corner, filler
//! blocks for density, day/night. Layout is in tile space; the camera
//! anchors the intersection near frame centre, and fillers extend to cover
//! whatever frame is requested (1920×1080 renders → union benches).

use render::framebuffer::Rect;
use render::{Framebuffer, palette::Color, palette};

use crate::buildings;
use crate::col::{Mode, c};
use crate::geom::{self, Grid, corridor_quad};
use crate::parts::{self, FACE_L, FACE_R};

/// A building bounding box on screen (for the 2× zoom crops).
pub struct Crop {
    pub name: &'static str,
    pub rect: Rect,
}

/// World anchors (tile space).
pub const IX: f32 = 58.0; // cross-street centre
pub const IY: f32 = 74.0; // boulevard centre
const WORLD: i32 = 150;

// boulevard cross-section (runs along +x)
const BV_S_NWALK: f32 = 71.0; // north sidewalk 71..72
const BV_RD_N0: f32 = 72.0; // north roadway 72..73.5
const BV_RD_N1: f32 = 73.5;
const BV_MED0: f32 = 73.5; // median 73.5..74.5
const BV_MED1: f32 = 74.5;
const BV_RD_S0: f32 = 74.5; // south roadway 74.5..76
const BV_RD_S1: f32 = 76.0;
const BV_S_SWALK: f32 = 77.0; // south sidewalk 76..77

// cross street cross-section (runs along +y)
const CX_WALK_W: f32 = 56.0; // west sidewalk 56..57
const CX_RD0: f32 = 57.0; // roadway 57..59
const CX_RD1: f32 = 59.0;
const CX_WALK_E: f32 = 60.0; // east sidewalk 59..60

pub struct Scene {
    pub g: Grid,
    pub mode: Mode,
    pub seed: u64,
}

impl Scene {
    /// Camera: the intersection anchor lands at (w/2, h*0.56).
    pub fn new(tw: i32, w: usize, h: usize, mode: Mode, seed: u64) -> Self {
        let hw = tw / 2;
        let hh = tw / 4;
        let ox = w as i32 / 2 - ((IX - IY) * hw as f32) as i32;
        let oy = (h as f32 * 0.56) as i32 - ((IX + IY) * hh as f32) as i32;
        Scene { g: Grid::new(tw, ox, oy), mode, seed }
    }

    fn visible(&self, fb: &Framebuffer, r: Rect) -> bool {
        r.intersects(&fb.bounds())
    }

    /// Screen bbox of a box (footprint + height).
    fn box_rect(&self, x: f32, y: f32, w: f32, d: f32, h: i32) -> Rect {
        let mut r = Rect::from_points(&[
            self.g.p(x, y, h),
            self.g.p(x + w, y, h),
            self.g.p(x + w, y + d, h),
            self.g.p(x, y + d, h),
        ]);
        let (_bx, by) = self.g.pi_f(x + w, y + d, 0);
        r.y1 = r.y1.max(by + 2);
        r
    }

    pub fn render(&self, fb: &mut Framebuffer) -> Vec<Crop> {
        let mut crops = Vec::new();
        let g = self.g;
        let mode = self.mode;
        let s = g.s;

        // ---------------- ground: grass everywhere ----------------
        for ty in 0..WORLD {
            for tx in 0..WORLD {
                let q = geom::tile_quad(&g, tx, ty);
                if !self.visible(fb, geom::quad_bounds(&q)) {
                    continue;
                }
                parts::grass_tile(fb, &g, tx, ty, mode);
            }
        }

        // ---------------- road corridors (full length, over grass) ----------
        let x_run0 = 2.0f32;
        let x_run1 = (WORLD - 2) as f32;
        let y_run0 = 2.0f32;
        let y_run1 = (WORLD - 2) as f32;

        // boulevard roadways
        for (a, b) in [(BV_RD_N0, BV_RD_N1), (BV_RD_S0, BV_RD_S1)] {
            let q = corridor_quad(&g, x_run0, a, x_run1, b, 0);
            parts::asphalt(fb, &g, &q, mode);
            // edge lines
            let edge = mode.surf(sh(c::LANE_LINE, 1.05));
            parts::lane_dashes_x(fb, &g, &q, 1, edge, mode);
            parts::lane_dashes_x(fb, &g, &q, q.rows() - 1, edge, mode);
        }
        // boulevard centre dashes are painted with the median below
        // cross-street roadway
        {
            let q = corridor_quad(&g, CX_RD0, y_run0, CX_RD1, y_run1, 0);
            parts::asphalt(fb, &g, &q, mode);
            let cols = q.top_len();
            parts::lane_dashes_y(fb, &g, &q, cols / 2, c::LANE_LINE, mode);
        }
        // intersection patch
        {
            let q = corridor_quad(&g, CX_WALK_W, BV_S_NWALK, CX_WALK_E, BV_S_SWALK, 1);
            parts::asphalt(fb, &g, &q, mode);
        }

        // sidewalks
        for (a, b, curb) in [(BV_S_NWALK, BV_RD_N0, 1), (BV_RD_S1, BV_S_SWALK, 0)] {
            let q = corridor_quad(&g, x_run0, a, x_run1, b, 1);
            parts::sidewalk(fb, &g, &q, mode, if curb == 1 { q.rows() - 1 } else { 0 });
        }
        for (a, b, curb) in [(CX_WALK_W, CX_RD0, 1), (CX_RD1, CX_WALK_E, 0)] {
            let q = corridor_quad(&g, a, y_run0, b, y_run1, 1);
            parts::sidewalk(fb, &g, &q, mode, if curb == 1 { q.rows() - 1 } else { 0 });
        }

        // median: grass strip with curb edge, trees + lamps (两段, skipping the intersection)
        for (mx0, mx1) in [(x_run0, CX_WALK_W - 0.25), (CX_WALK_E + 0.25, x_run1)] {
            let q = corridor_quad(&g, mx0, BV_MED0, mx1, BV_MED1, 1);
            let qq = geom::quad_bounds(&q);
            if !self.visible(fb, qq) {
                continue;
            }
            q.paint(fb, &mut |u, v, x, y| {
                let edge = v == 0 || v == q.rows() - 1 || u < s || u > q.top_len() - s;
                if edge {
                    mode.surf(c::CURB)
                } else {
                    let jit = 0.9 + 0.15 * crate::col::hash01(x, y);
                    mode.surf(sh(c::GRASS_B, jit))
                }
            });
            // boulevard centre dashes sit on the roadways at the median edges
            let qn = corridor_quad(&g, mx0, BV_RD_N0, mx1, BV_RD_N1, 0);
            let qs = corridor_quad(&g, mx0, BV_RD_S0, mx1, BV_RD_S1, 0);
            parts::lane_dashes_x(fb, &g, &qn, qn.rows() - 1, c::CENTER_LINE, mode);
            parts::lane_dashes_x(fb, &g, &qs, 1, c::CENTER_LINE, mode);
        }

        // crosswalks around the intersection + stop bars + manholes
        parts::stop_bar(fb, &g, CX_RD0, BV_RD_N0 - 0.4, CX_RD1, BV_RD_N0 - 0.15, mode);
        parts::stop_bar(fb, &g, CX_RD0, BV_RD_S1 + 0.15, CX_RD1, BV_RD_S1 + 0.4, mode);
        parts::stop_bar(fb, &g, CX_RD0 - 0.4, BV_RD_N0, CX_RD0 - 0.15, BV_RD_S1, mode);
        parts::stop_bar(fb, &g, CX_RD1 + 0.15, BV_RD_N0, CX_RD1 + 0.4, BV_RD_S1, mode);

        parts::crosswalk_y(fb, &g, CX_RD0 - 0.2, BV_RD_N0, CX_RD1 + 0.2, BV_RD_N0 + 1.0, mode);
        parts::crosswalk_y(fb, &g, CX_RD0 - 0.2, BV_RD_S1 - 1.0, CX_RD1 + 0.2, BV_RD_S1, mode);
        parts::crosswalk_x(fb, &g, CX_RD1 + 0.1, BV_RD_N0 - 0.15, CX_RD1 + 1.1, BV_RD_S1 + 0.15, mode);
        parts::crosswalk_x(fb, &g, CX_RD0 - 1.1, BV_RD_N0 - 0.15, CX_RD0 - 0.1, BV_RD_S1 + 0.15, mode);
        parts::manhole(fb, &g, IX - 1.0, IY - 0.4, mode);
        parts::manhole(fb, &g, IX + 1.2, IY + 0.5, mode);
        parts::manhole(fb, &g, IX + 0.1, IY + 1.2, mode);
        parts::manhole(fb, &g, IX + 6.4, BV_RD_N0 + 0.7, mode);
        parts::manhole(fb, &g, IX - 6.4, BV_RD_S1 - 0.7, mode);

        // showcase lot surfaces: driveway strip for the house
        {
            let q = corridor_quad(&g, 66.2, 77.0, 68.0, 84.5, 1);
            parts::paint_flat(&q, fb, mode, c::DRIVEWAY, 1.0);
        }

        // ---------------- objects, painter-sorted ----------------
        let mut objs: Vec<(i32, Box<dyn FnOnce(&mut Framebuffer)>)> = Vec::new();
        let key = |x: f32, y: f32| ((x + y) * 100.0) as i32;

        // showcase buildings
        {
            let r = self.box_rect(62.6, 78.6, 3.2, 3.2, buildings::RES_FLOOR * 2 * s + 7 * s + 2).expanded(8 * s);
            crops.push(Crop { name: "house", rect: r });
        }
        let seed = self.seed;
        objs.push((key(65.6, 81.6), Box::new(move |fb| {
            buildings::clapboard_house(fb, &Scene::gs(g), mode, 63.0, 79.0, seed + 1);
        })));

        {
            let r = self.box_rect(60.4, 68.4, 2.6, 2.4, (buildings::SHOP_GROUND + 2 * buildings::SHOP_FLOOR) * s + 12 * s).expanded(6 * s);
            crops.push(Crop { name: "shop", rect: r });
        }
        objs.push((key(62.9, 70.8), Box::new(move |fb| {
            buildings::brick_shop(fb, &Scene::gs(g), mode, 60.5, 68.6, seed + 2);
        })));

        {
            let r = self.box_rect(53.2, 77.0, 2.4, 2.2, 4 * buildings::RES_FLOOR * s + 6 * s).expanded(6 * s);
            crops.push(Crop { name: "townhouse", rect: r });
        }
        objs.push((key(55.6, 79.2), Box::new(move |fb| {
            buildings::stucco_townhouse(fb, &Scene::gs(g), mode, 53.4, 77.2, seed + 3);
        })));

        {
            let fl10 = buildings::SHOP_GROUND * s + 10 * buildings::TOWER_FLOOR * s;
            let r = self.box_rect(64.0, 66.6, 2.8, 2.6, fl10 + 4 * s).expanded(6 * s);
            crops.push(Crop { name: "office", rect: r });
        }
        objs.push((key(66.8, 69.2), Box::new(move |fb| {
            buildings::glass_office(fb, &Scene::gs(g), mode, 64.2, 66.8, 10, seed + 4);
        })));

        {
            let fl_t = 3 * 8 * s + 14 * buildings::TOWER_FLOOR * s + 7 * buildings::TOWER_FLOOR * s + 20 * s;
            let r = self.box_rect(59.2, 57.5, 3.2, 3.0, fl_t).expanded(8 * s);
            crops.push(Crop { name: "tower", rect: r });
        }
        objs.push((key(62.4, 60.5), Box::new(move |fb| {
            buildings::glass_tower(fb, &Scene::gs(g), mode, 59.2, 57.5, seed + 5);
        })));

        // fillers
        self.fillers(&mut objs);

        // yard details: fence, front walkway, garden, mailbox
        objs.push((key(66.0, 79.5), Box::new(move |fb| {
            let g = Scene::gs(g);
            // Walkway from house porch steps to front sidewalk
            parts::walkway(fb, &g, mode, 64.0, 77.0, 64.6, 79.6);
            // Picket fence with front gate opening
            parts::fence_x(fb, &g, mode, 62.4, 64.0, 77.0, c::FENCE);
            parts::fence_x(fb, &g, mode, 64.6, 68.0, 77.0, c::FENCE);
            parts::fence_y(fb, &g, mode, 77.0, 84.0, 62.4, c::FENCE);
            // Gardens along foundation
            parts::garden(fb, &g, mode, 62.8, 77.4, 63.8, 78.8);
            parts::garden(fb, &g, mode, 64.8, 77.4, 65.8, 78.8);
            // Mailbox at front walk
            parts::mailbox(fb, &g, mode, 64.7, 76.7);
        })));

        // median trees + flowerbeds + boulevard lamps
        let mut tx = 6.0f32;
        while tx < (WORLD - 6) as f32 {
            // Strictly skip the cross-street intersection (56.0..60.0)
            if tx < CX_WALK_W - 1.2 || tx > CX_WALK_E + 1.2 {
                let k = key(tx, BV_MED1);
                let sp = crate::col::hash2(tx as i32, 7) % 3;
                objs.push((k, Box::new(move |fb| {
                    let g = Scene::gs(g);
                    match sp {
                        0 => parts::tree_oak(fb, &g, mode, tx, BV_MED1 - 0.5, tx as u64, 1.4),
                        1 => parts::tree_birch(fb, &g, mode, tx, BV_MED1 - 0.5, tx as u64, 1.4),
                        _ => parts::tree_pine(fb, &g, mode, tx, BV_MED1 - 0.5, tx as u64, 1.4),
                    }
                    // Flower bed in median
                    parts::garden(fb, &g, mode, tx + 0.8, BV_MED0 + 0.15, tx + 2.0, BV_MED1 - 0.15);
                })));
            }
            tx += 2.8;
        }

        // Park benches near the intersection
        objs.push((key(CX_WALK_W - 2.0, BV_MED1), Box::new(move |fb| {
            parts::park_bench(fb, &Scene::gs(g), mode, CX_WALK_W - 2.0, (BV_MED0 + BV_MED1) / 2.0, 'x');
        })));
        objs.push((key(CX_WALK_E + 2.0, BV_MED1), Box::new(move |fb| {
            parts::park_bench(fb, &Scene::gs(g), mode, CX_WALK_E + 2.0, (BV_MED0 + BV_MED1) / 2.0, 'x');
        })));
        objs.push((key(61.8, BV_S_SWALK), Box::new(move |fb| {
            parts::park_bench(fb, &Scene::gs(g), mode, 61.8, BV_S_SWALK - 0.35, 'x');
        })));

        // Fire hydrants at street corners
        objs.push((key(CX_WALK_E + 0.5, BV_S_SWALK), Box::new(move |fb| {
            parts::fire_hydrant(fb, &Scene::gs(g), mode, CX_WALK_E + 0.5, BV_S_SWALK - 0.35);
        })));
        objs.push((key(CX_WALK_W - 0.5, BV_S_NWALK), Box::new(move |fb| {
            parts::fire_hydrant(fb, &Scene::gs(g), mode, CX_WALK_W - 0.5, BV_S_NWALK + 0.35);
        })));
        // lamps on both sidewalks
        let mut lx = 8.0f32;
        while lx < (WORLD - 8) as f32 {
            if (lx - IX).abs() > 3.0 {
                let k1 = key(lx, BV_S_NWALK);
                objs.push((k1, Box::new(move |fb| {
                    parts::lamp_post(fb, &Scene::gs(g), mode, lx, BV_S_NWALK + 0.25);
                })));
                let lx2 = lx + 3.5;
                let k2 = key(lx2, BV_S_SWALK);
                objs.push((k2, Box::new(move |fb| {
                    parts::lamp_post(fb, &Scene::gs(g), mode, lx2, BV_S_SWALK - 0.25);
                })));
            }
            lx += 7.0;
        }
        // cross-street lamps + corner sign
        objs.push((key(CX_WALK_W, 80.0), Box::new(move |fb| {
            parts::lamp_post(fb, &Scene::gs(g), mode, CX_WALK_W + 0.3, 80.5);
        })));
        objs.push((key(CX_RD1, 71.4), Box::new(move |fb| {
            parts::street_sign(fb, &Scene::gs(g), mode, CX_RD1 + 0.4, BV_S_NWALK + 0.4);
        })));

        // people on the sidewalks
        let shirts = [
            palette::rgb(0xe2, 0x6c, 0x60),
            palette::rgb(0x78, 0x96, 0xdc),
            palette::rgb(0xf0, 0xd8, 0x78),
            palette::rgb(0x8c, 0xc8, 0x96),
            palette::rgb(0xdc, 0xdc, 0xe2),
            palette::rgb(0xbe, 0x82, 0xc8),
        ];
        let people_at = [
            (CX_WALK_W + 0.6, 74.9, 0u64),
            (CX_RD1 + 0.5, 73.0, 1),
            (65.0, BV_S_SWALK - 0.3, 2),
            (62.2, 71.6, 3),
            (57.2, 79.8, 4),
            (70.5, BV_S_NWALK + 0.6, 5),
            (49.0, BV_S_SWALK - 0.3, 6),
            (IX + 0.4, BV_RD_N0 + 0.6, 7),
        ];
        for (px, py, i) in people_at {
            let k = key(px, py);
            objs.push((k, Box::new(move |fb| {
                parts::person(fb, &Scene::gs(g), mode, px, py, shirts[(i as usize) % 6], seed * 7 + i);
            })));
        }

        // cars: two on the boulevard, one in the driveway
        objs.push((key(52.0, BV_RD_S0 + 0.75), Box::new(move |fb| {
            parts::car(fb, &Scene::gs(g), mode, 52.0, BV_RD_S0 + 0.75, 'x', palette::rgb(0xd6, 0x52, 0x4a), 1);
        })));
        objs.push((key(66.0, BV_RD_N0 + 0.7), Box::new(move |fb| {
            parts::car(fb, &Scene::gs(g), mode, 66.0, BV_RD_N0 + 0.7, 'x', palette::rgb(0x60, 0xa4, 0xd6), 2);
        })));
        objs.push((key(67.1, 81.5), Box::new(move |fb| {
            parts::car(fb, &Scene::gs(g), mode, 67.1, 81.5, 'x', palette::rgb(0xee, 0xee, 0xe8), 3);
        })));

        // corner tree by the house + a pine behind the shop
        objs.push((key(68.4, 84.0), Box::new(move |fb| {
            parts::tree_oak(fb, &Scene::gs(g), mode, 68.4, 78.6, 42, 1.2);
        })));

        // draw everything in painter order
        objs.sort_by_key(|(k, _)| *k);
        for (_k, draw) in objs {
            draw(fb);
        }

        crops
    }

    /// Filler city blocks around the showcase lots (extend to the frame).
    fn fillers(&self, objs: &mut Vec<(i32, Box<dyn FnOnce(&mut Framebuffer)>)>) {
        let g = self.g;
        let mode = self.mode;
        let seed = self.seed;
        let key = |x: f32, y: f32| ((x + y) * 100.0) as i32;

        // lot bands: (x0, y0, x1, y1). Rows along both sides of the
        // boulevard plus background blocks. Pitch ~3.5 tiles per lot.
        let mut bands: Vec<(f32, f32, f32, f32)> = Vec::new();
        // north of boulevard: two rows (tower/shop/office lots kept clear)
        bands.push((6.0, 64.2, CX_WALK_W - 5.2, 70.6));
        bands.push((66.6, 63.0, (WORLD - 6) as f32, 70.6));
        bands.push((6.0, 57.0, CX_WALK_W - 0.5, 63.6));
        bands.push((CX_WALK_E + 5.6, 56.5, (WORLD - 6) as f32, 61.4));
        // background suburbs further north
        bands.push((10.0, 40.0, (WORLD - 12) as f32, 55.0));
        bands.push((18.0, 26.0, (WORLD - 18) as f32, 38.0));
        // south of boulevard
        bands.push((6.0, 77.6, CX_WALK_W - 2.6, 96.0));
        bands.push((68.8, 77.6, (WORLD - 6) as f32, 96.0));
        bands.push((CX_WALK_W - 2.6, 85.0, 68.8, 96.0));
        bands.push((16.0, 97.0, (WORLD - 12) as f32, 112.0));

        // showcase lots to keep clear
        let keep: [(f32, f32, f32, f32); 6] = [
            (62.0, 76.5, 69.0, 85.0),  // house + yard + driveway
            (59.8, 68.0, 63.6, 71.6),  // shop
            (52.8, 76.6, 56.2, 80.2),  // townhouse
            (63.6, 66.2, 67.4, 70.0),  // office
            (58.8, 57.0, 63.0, 61.2),  // tower
            (30.0, 30.0, 31.0, 31.0),  // dummy for array shape stability
        ];

        let mut rng = map::rng::Rng::new(seed ^ 0x5EED);
        for (bx0, by0, bx1, by1) in bands {
            let mut ly = by0;
            while ly + 3.4 < by1 {
                let mut lx = bx0;
                while lx + 3.4 < bx1 {
                    let lot = (lx, ly, lx + 3.4, ly + 3.4);
                    // skip cross street and showcase lots
                    let crosses = lot.0 < CX_WALK_E && lot.2 > CX_WALK_W;
                    let blocked = keep.iter().any(|k| lot.0 < k.2 && lot.2 > k.0 && lot.1 < k.3 && lot.3 > k.1);
                    if !crosses && !blocked && !rng.chance(0.08) {
                        let cx = lot.0 + 0.3 + rng.f32() * 0.7;
                        let cy = lot.1 + 0.3 + rng.f32() * 0.7;
                        let lseed = rng.next_u64();
                        let south = ly > BV_MED0;
                        let near_blv = (ly - 57.0).abs() < 14.0 || (ly - 77.0).abs() < 10.0;
                        let pickv = rng.f32();
                        let k = key(cx + 2.0, cy + 2.0);
                        objs.push((k, Box::new(move |fb| {
                            let g = Scene::gs(g);
                            if south && pickv < 0.62 {
                                buildings::filler_house(fb, &g, mode, cx, cy, lseed);
                            } else if near_blv && pickv < 0.72 {
                                buildings::filler_shop(fb, &g, mode, cx, cy, lseed);
                            } else if pickv < 0.88 {
                                buildings::filler_house(fb, &g, mode, cx, cy, lseed);
                            } else {
                                buildings::filler_mid(fb, &g, mode, cx, cy, lseed);
                            }
                        })));
                        // street trees on the boulevard rows
                        if near_blv && rng.chance(0.45) {
                            let tx2 = lx + 1.6 + rng.f32();
                            let ty2 = if south { BV_S_SWALK + 0.8 } else { BV_S_NWALK - 0.8 };
                            let tseed = rng.next_u64();
                            let sp = rng.usize(3);
                            let k = key(tx2, ty2);
                            objs.push((k, Box::new(move |fb| {
                                let g = Scene::gs(g);
                                match sp {
                                    0 => parts::tree_oak(fb, &g, mode, tx2, ty2, tseed, 1.0),
                                    1 => parts::tree_birch(fb, &g, mode, tx2, ty2, tseed, 1.0),
                                    _ => parts::tree_pine(fb, &g, mode, tx2, ty2, tseed, 1.1),
                                }
                            })));
                        }
                    }
                    lx += 3.5;
                }
                ly += 3.6;
            }
        }
        let _ = (FACE_L, FACE_R);
    }

    /// Copy of the grid (Grid is Copy; helper for closures).
    fn gs(g: Grid) -> Grid {
        g
    }

    pub fn sh(c0: Color, f: f32) -> Color {
        palette::shade(c0, f)
    }
}

#[inline]
fn sh(c0: Color, f: f32) -> Color {
    palette::shade(c0, f)
}
