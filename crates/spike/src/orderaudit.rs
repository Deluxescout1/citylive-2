//! B100/D95: do buildings share ground, and are they drawn in the right order?
//!
//! `CITYLIVE_ORDER_AUDIT=1 spike --sim --sim-days N --seed S --biome B ...`
//! builds the same city a still would, then measures:
//!
//! * **lot overlap**: two lots share a tile;
//! * **wall overlap**: two buildings' real wall footprints (`KitSprite::foot`)
//!   share ground, or a footprint reaches outside its own lot;
//! * **sideways**: a sprite's opaque pixels reach past the screen width of its
//!   own lot's diamond (an overhang the neighbour's ground has to absorb);
//! * **order**: for every pair of sprites that both draw a pixel in the same
//!   place, the nearer one (by footprint) must be the one painted last. The
//!   scene reports the order it really paints in (`Scene::paint_rank`).

use render::iso::HEIGHT_UNIT;
use render::palette::TRANSPARENT;
use render::scene::KitSprite;
use render::{CityObj, ObjKind, Scene};

/// One building as the checker sees it.
struct B<'a> {
    i: usize,
    /// Lot rectangle, tiles: x0, y0, x1, y1 (exclusive).
    lot: [i32; 4],
    /// Wall footprint, absolute tiles.
    foot: [f32; 4],
    /// Sprite placement on screen and the sprite itself.
    sx: i32,
    sy: i32,
    k: Option<&'a KitSprite>,
    /// Opaque screen bounds.
    r: [i32; 4],
}

#[derive(Default)]
pub struct Report {
    pub buildings: usize,
    pub engine_art: usize,
    pub lot_overlap: usize,
    pub wall_overlap: usize,
    pub wall_outside_lot: usize,
    pub sideways: usize,
    pub sideways_max: i32,
    /// Sprites that draw ground-level pixels (paving, walls, porches) outside
    /// their own lot, measured on a fresh render with its depth plane.
    pub ext_px: usize,
    /// Ground painted over a building's pixels (tiles and props in front of it)
    /// and the buildings with more than a few such pixels.
    pub ground_over_px: usize,
    pub buried: usize,
    /// Flat ground (a park, a plaza) against buildings beside it.
    pub ground_pairs: usize,
    pub ground_order_wrong: usize,
    pub ground_order_wrong_px: usize,
    pub ground_over: usize,
    pub ground_over_max: i32,
    pub pairs_overlapping_px: usize,
    pub order_wrong: usize,
    pub order_wrong_px: usize,
    pub order_wrong_samples: Vec<String>,
}

impl Report {
    pub fn add(&mut self, o: &Report) {
        self.buildings += o.buildings;
        self.engine_art += o.engine_art;
        self.lot_overlap += o.lot_overlap;
        self.wall_overlap += o.wall_overlap;
        self.wall_outside_lot += o.wall_outside_lot;
        self.sideways += o.sideways;
        self.sideways_max = self.sideways_max.max(o.sideways_max);
        self.ext_px += o.ext_px;
        self.ground_over_px += o.ground_over_px;
        self.buried += o.buried;
        self.ground_pairs += o.ground_pairs;
        self.ground_order_wrong += o.ground_order_wrong;
        self.ground_order_wrong_px += o.ground_order_wrong_px;
        self.ground_over += o.ground_over;
        self.ground_over_max = self.ground_over_max.max(o.ground_over_max);
        self.pairs_overlapping_px += o.pairs_overlapping_px;
        self.order_wrong += o.order_wrong;
        self.order_wrong_px += o.order_wrong_px;
    }
}

fn flat(kind: ObjKind) -> bool {
    matches!(kind, ObjKind::Park | ObjKind::Cemetery | ObjKind::CarPark)
}

pub fn audit(scene: &Scene, label: &str) -> Report {
    let cam = scene.camera();
    let kit = scene.kit_sprites();
    let objs = scene.city_objects();
    let mut rep = Report::default();
    let mut bs: Vec<B> = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        if flat(o.kind) {
            continue;
        }
        let base = o.level as i32 * HEIGHT_UNIT;
        let (px, py) = cam.project(o.x as f32, o.y as f32, base);
        let (sx, sy) = (px as i32, py as i32);
        let k = (o.kit != 0).then(|| kit.get(o.kit as usize - 1)).flatten();
        let lot = [o.x, o.y, o.x + o.w, o.y + o.h];
        let (foot, r) = match k {
            Some(k) => {
                let f = [
                    o.x as f32 + k.foot[0],
                    o.y as f32 + k.foot[1],
                    o.x as f32 + k.foot[2],
                    o.y as f32 + k.foot[3],
                ];
                let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
                for yy in 0..k.h {
                    for xx in 0..k.w {
                        if k.px[yy * k.w + xx] != TRANSPARENT {
                            x0 = x0.min(xx as i32);
                            x1 = x1.max(xx as i32 + 1);
                            y0 = y0.min(yy as i32);
                            y1 = y1.max(yy as i32 + 1);
                        }
                    }
                }
                (
                    f,
                    [
                        sx - k.ox + x0,
                        sy - k.oy + y0,
                        sx - k.ox + x1,
                        sy - k.oy + y1,
                    ],
                )
            }
            None => (
                [lot[0] as f32, lot[1] as f32, lot[2] as f32, lot[3] as f32],
                [0, 0, 0, 0],
            ),
        };
        if k.is_some() {
            rep.buildings += 1;
        } else {
            rep.engine_art += 1;
        }
        bs.push(B { i, lot, foot, sx, sy, k, r });
    }

    // Ground (tiles, props, terrain) painted over a building's pixels: read off
    // the static layer's depth plane (an odd key is ground, an even one a wall).
    if let Some(depth) = scene.static_layer().depth() {
        let lw = scene.static_layer().w as i32;
        let lh = scene.static_layer().h as i32;
        for b in &bs {
            let Some(k) = b.k else { continue };
            let mut covered = 0usize;
            for j in 0..k.h {
                for i in 0..k.w {
                    if k.px[j * k.w + i] == TRANSPARENT {
                        continue;
                    }
                    let (x, y) = (b.sx - k.ox + i as i32, b.sy - k.oy + j as i32);
                    if x < 0 || y < 0 || x >= lw || y >= lh {
                        continue;
                    }
                    let key = depth[(y * lw + x) as usize];
                    if key != 0 && key % 2 == 1 {
                        covered += 1;
                    }
                }
            }
            rep.ground_over_px += covered;
            if covered > 8 {
                rep.buried += 1;
                if std::env::var("CITYLIVE_ORDER_VERBOSE").is_ok() {
                    println!("   buried {covered}px lot {:?} sprite rect {:?}", b.lot, b.r);
                }
            }
        }
    }

    // Ground contact outside the lot, from a fresh render's depth plane.
    {
        let style = buildkit::BiomeStyle::of(scene.biome());
        let light = buildkit::light_for([1.0, 1.0, 1.0], 0.0, 12.0);
        let mut cache: std::collections::HashMap<(buildkit::BuildingSpec, u8), i32> = Default::default();
        for b in &bs {
            let o = &objs[b.i];
            if b.k.is_none() {
                continue;
            }
            let Some(mut sp) = buildkit::spec_for(o) else { continue };
            sp.biome = style;
            let key = (sp.clone(), 0u8);
            let over = *cache.entry(key).or_insert_with(|| {
                let mut spr = buildkit::Sprite::new(1, 1);
                buildkit::draw(&sp, &light, &mut spr);
                render::scene::lot_overhang(
                    spr.w,
                    spr.h,
                    spr.ox,
                    spr.oy,
                    sp.footprint.0.max(1) as i32,
                    sp.footprint.1.max(1) as i32,
                    |i| (spr.px[i] >> 24) >= 0x80 && spr.depth[i] <= 10,
                )
            });
            if over > 1 {
                rep.ground_over += 1;
                rep.ground_over_max = rep.ground_over_max.max(over);
                if std::env::var("CITYLIVE_ORDER_VERBOSE").is_ok() {
                    println!("   ground {over}px kind {:?} {}x{} storeys {} style {} lot {:?} design {}", o.kind, o.w, o.h, o.storeys, o.style, b.lot, buildkit::genkit::design_of(&sp).map(|d| format!("{:?}/{:?}", d.family, d.plan)).unwrap_or(format!("hand-made {:?}", sp.resolved_type())));
                }
            }
        }
    }

    // Sideways: the sprite's opaque columns against the lot diamond's width.
    for b in &bs {
        if b.k.is_none() {
            continue;
        }
        let (lx, _) = cam.project(b.lot[0] as f32, b.lot[3] as f32, 0);
        let (rx, _) = cam.project(b.lot[2] as f32, b.lot[1] as f32, 0);
        let over = ((lx as i32) - b.r[0]).max(b.r[2] - rx as i32);
        if over > 1 {
            if std::env::var("CITYLIVE_ORDER_VERBOSE").is_ok() {
                let o = &objs[b.i];
                if let Some(sp) = buildkit::spec_for(o) {
                    if let Some(d) = buildkit::genkit::design_of(&sp) {
                        println!("      design w {:.2} d {:.2} bw {:.2} bd {:.2} family {:?} porch {} garage {} bay {} roof {:?} plan {:?} dormers {}", d.w, d.d, d.bw, d.bd, d.family, d.porch, d.garage, d.bay, d.roof, d.plan, d.dormers);
                    } else {
                        println!("      no genkit design (hand-made / vernacular) spec {:?}", sp.function);
                    }
                }
                println!("   sideways {over}px kind {:?} {}x{} storeys {} style {} stage {:?} lot {:?} foot {:.2?}", o.kind, o.w, o.h, o.storeys, o.style, o.stage, b.lot, b.foot);
            }
            rep.sideways += 1;
            rep.sideways_max = rep.sideways_max.max(over);
        }
        let f = b.foot;
        let eps = 0.03;
        if f[0] < b.lot[0] as f32 - eps
            || f[1] < b.lot[1] as f32 - eps
            || f[2] > b.lot[2] as f32 + eps
            || f[3] > b.lot[3] as f32 + eps
        {
            rep.wall_outside_lot += 1;
        }
    }

    // Pairs by sweep on screen x.
    let mut order: Vec<usize> = (0..bs.len()).collect();
    order.sort_by_key(|&j| bs[j].lot[0] - bs[j].lot[1]);
    for a_i in 0..bs.len() {
        let a = &bs[a_i];
        for b_i in (a_i + 1)..bs.len() {
            let b = &bs[b_i];
            // lots
            if a.lot[0] < b.lot[2] && b.lot[0] < a.lot[2] && a.lot[1] < b.lot[3] && b.lot[1] < a.lot[3] {
                rep.lot_overlap += 1;
            }
            let e = 0.03;
            let walls = a.foot[0] < b.foot[2] - e
                && b.foot[0] < a.foot[2] - e
                && a.foot[1] < b.foot[3] - e
                && b.foot[1] < a.foot[3] - e;
            if walls && a.k.is_some() && b.k.is_some() {
                rep.wall_overlap += 1;
            }
            // Pixels.
            let (Some(ka), Some(kb)) = (a.k, b.k) else { continue };
            let x0 = a.r[0].max(b.r[0]);
            let x1 = a.r[2].min(b.r[2]);
            let y0 = a.r[1].max(b.r[1]);
            let y1 = a.r[3].min(b.r[3]);
            if x0 >= x1 || y0 >= y1 {
                continue;
            }
            // Who is nearer? Separation of the lots along an axis decides
            // (lots never overlap). Diagonal neighbours have no depth relation.
            let a_front = a.lot[0] >= b.lot[2] || a.lot[1] >= b.lot[3];
            let b_front = b.lot[0] >= a.lot[2] || b.lot[1] >= a.lot[3];
            let nearer_is_a = match (a_front, b_front) {
                (true, false) => Some(true),
                (false, true) => Some(false),
                _ => None,
            };
            let (mut n, mut wrong) = (0usize, 0usize);
            let mut ext_px = 0usize;
            let (mut rka, mut rkb) = (0u64, 0u64);
            let mut last_x = i32::MIN;
            let mut first_bad: Option<(i32, i32, u64, u64)> = None;
            for y in y0..y1 {
                for x in x0..x1 {
                    let (ax, ay) = (x - (a.sx - ka.ox), y - (a.sy - ka.oy));
                    let (bx, by) = (x - (b.sx - kb.ox), y - (b.sy - kb.oy));
                    if ax < 0 || ay < 0 || bx < 0 || by < 0 {
                        continue;
                    }
                    let (ax, ay, bx, by) = (ax as usize, ay as usize, bx as usize, by as usize);
                    if ax >= ka.w || ay >= ka.h || bx >= kb.w || by >= kb.h {
                        continue;
                    }
                    if ka.px[ay * ka.w + ax] != TRANSPARENT && kb.px[by * kb.w + bx] != TRANSPARENT {
                        n += 1;
                        if last_x != x {
                            last_x = x;
                            rka = scene.paint_rank(a.i, x);
                            rkb = scene.paint_rank(b.i, x);
                        }
                        // A pixel where either sprite is leaning out past its own
                        // lot (bit 31 clear = an overhang piece) is a sideways
                        // overhang, counted there, not an ordering fault.
                        let real = (rka & (1 << 31)) != 0 && (rkb & (1 << 31)) != 0;
                        if !real {
                            ext_px += 1;
                        } else if let Some(na) = nearer_is_a {
                            if (rka > rkb) != na {
                                wrong += 1;
                                if first_bad.is_none() {
                                    first_bad = Some((x, y, rka >> 32, rkb >> 32));
                                }
                            }
                        }
                    }
                }
            }
            if n == 0 {
                continue;
            }
            rep.ext_px += ext_px;
            rep.pairs_overlapping_px += 1;
            if wrong > 0 {
                rep.order_wrong += 1;
                rep.order_wrong_px += wrong;
                if rep.order_wrong_samples.len() < 12 {
                    let (n_, f_) = if nearer_is_a == Some(true) { (a, b) } else { (b, a) };
                    rep.order_wrong_samples.push(format!(
                        "{label}: nearer lot {:?} foot {:.2?} painted under farther lot {:?} foot {:.2?} ({} of {} px) first at {:?} (ranks a{} b{})",
                        n_.lot, n_.foot, f_.lot, f_.foot, wrong, n, first_bad, if nearer_is_a == Some(true) { "near" } else { "far" }, ""
                    ));
                }
            }
        }
    }
    let _ = &order;

    // Flat ground (parks, cemeteries, plazas, car parks) against the buildings
    // beside it: where a building's pixel falls inside the ground's diamond,
    // the nearer of the two must be painted last.
    for (pi, p) in objs.iter().enumerate() {
        if !(flat(p.kind) || matches!(p.kind, ObjKind::Landmark(render::city::LandmarkKind::Plaza))) {
            continue;
        }
        let pl = [p.x, p.y, p.x + p.w, p.y + p.h];
        let c = |x: i32, y: i32| {
            let (a, b) = cam.project(x as f32, y as f32, p.level as i32 * HEIGHT_UNIT);
            (a as i32, b as i32)
        };
        let (top, right, bottom, left) = (c(pl[0], pl[1]), c(pl[2], pl[1]), c(pl[2], pl[3]), c(pl[0], pl[3]));
        for b in &bs {
            let Some(k) = b.k else { continue };
            if b.r[2] <= left.0 || b.r[0] >= right.0 || b.r[3] <= top.1 || b.r[1] >= bottom.1 {
                continue;
            }
            let b_front = b.lot[0] >= pl[2] || b.lot[1] >= pl[3];
            let p_front = pl[0] >= b.lot[2] || pl[1] >= b.lot[3];
            let near_b = match (b_front, p_front) {
                (true, false) => true,
                (false, true) => false,
                _ => continue,
            };
            let mut wrong = 0usize;
            let mut seen = 0usize;
            for yy in b.r[1].max(top.1)..b.r[3].min(bottom.1) {
                for xx in b.r[0].max(left.0)..b.r[2].min(right.0) {
                    let (ax, ay) = (xx - (b.sx - k.ox), yy - (b.sy - k.oy));
                    if ax < 0 || ay < 0 || ax as usize >= k.w || ay as usize >= k.h {
                        continue;
                    }
                    if k.px[ay as usize * k.w + ax as usize] == TRANSPARENT {
                        continue;
                    }
                    // Inside the diamond (|dx| / 32 + |dy| / 16 <= ...): the
                    // diamond is centred on the top-to-bottom midpoint.
                    let (mx, my) = ((left.0 + right.0) / 2, (top.1 + bottom.1) / 2);
                    let (hw, hh) = ((right.0 - left.0) / 2, (bottom.1 - top.1) / 2);
                    if hw == 0 || hh == 0 {
                        continue;
                    }
                    let inside = ((xx - mx).abs() as f32 / hw as f32) + ((yy - my).abs() as f32 / hh as f32) <= 1.0;
                    if !inside {
                        continue;
                    }
                    seen += 1;
                    let (rb, rp) = (scene.paint_rank(b.i, xx), scene.paint_rank(pi, xx));
                    // Bit 31 clear: the building's pixel is an overhang piece,
                    // leaning out past its own lot (the sideways metric).
                    if rb & (1 << 31) == 0 {
                        rep.ext_px += 1;
                        continue;
                    }
                    if (rb > rp) != near_b {
                        wrong += 1;
                    }
                }
            }
            if seen > 0 {
                rep.ground_pairs += 1;
            }
            if wrong > 0 {
                rep.ground_order_wrong += 1;
                rep.ground_order_wrong_px += wrong;
                if rep.order_wrong_samples.len() < 12 {
                    rep.order_wrong_samples.push(format!(
                        "{label}: {:?} {:?} vs building lot {:?} (sprite rect {:?}): {} of {} px painted in the wrong order",
                        p.kind, pl, b.lot, b.r, wrong, seen
                    ));
                }
            }
        }
    }
    rep
}

pub fn print(label: &str, r: &Report) {
    println!(
        "AUDIT {label}: buildings {} engine {} | lot-overlap {} wall-overlap {} wall-outside-lot {} sideways {} (max {} px) ground-over {} (max {} px) | sprites sharing pixels {} order-wrong {} ({} px) [overhang px {}] ground-over-building {} px / {} buildings | park/plaza pairs {} wrong {} ({} px)",
        r.buildings,
        r.engine_art,
        r.lot_overlap,
        r.wall_overlap,
        r.wall_outside_lot,
        r.sideways,
        r.sideways_max,
        r.ground_over,
        r.ground_over_max,
        r.pairs_overlapping_px,
        r.order_wrong,
        r.order_wrong_px,
        r.ext_px,
        r.ground_over_px,
        r.buried,
        r.ground_pairs,
        r.ground_order_wrong,
        r.ground_order_wrong_px
    );
    for s in &r.order_wrong_samples {
        println!("   {s}");
    }
}

#[allow(dead_code)]
fn _unused(_: &CityObj) {}
