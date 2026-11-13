//! Growth report: the numbers behind the frames (B26b).
//!
//! ```text
//! cargo run --release -p sim --example growth -- 7 30          # the day table
//! cargo run --release -p sim --example growth -- 7 60 daily   # every day
//! ```
//!
//! Columns: era, population, lots by kind, street tiles and what they are
//! surfaced with, the share of the developed area that is street (brief B26b
//! item 1: <= 30 %), and the paved share inside the built-up area (item 2:
//! >= 80 % by day 16).

use sim::{District, Era, LotKind, Pace};

fn main() {
    let mut args = std::env::args().skip(1);
    let seed: u64 = args.next().and_then(|a| a.parse().ok()).unwrap_or(7);
    let days: u32 = args.next().and_then(|a| a.parse().ok()).unwrap_or(30);
    let every: u32 = if args.next().as_deref() == Some("daily") { 1 } else { 0 };
    // Optional framebuffer size (`7040 1440` = the owner's desktop); default 960x540.
    let fw: i32 = std::env::var("GROWTH_FB_W").ok().and_then(|v| v.parse().ok()).unwrap_or(960);
    let fh: i32 = std::env::var("GROWTH_FB_H").ok().and_then(|v| v.parse().ok()).unwrap_or(540);
    let vp = map::world_for_viewport(fw, fh);
    // The same founding the frames use: the middle of the framebuffer (D35).
    let focus = map::screen_to_tile(&vp, fw as f32 / 2.0, fh as f32 / 2.0);
    // GROWTH_LOAD=<save>: start from a saved city instead (days are then counted
    // from the save's own clock).
    let loaded = std::env::var("GROWTH_LOAD").ok();
    let mut s = match &loaded {
        Some(p) => sim::Sim::load(std::path::Path::new(p)).expect("load save"),
        None => sim::Sim::with_focus(seed, Pace::Month, vp.w, vp.h, focus),
    };
    let t0 = if loaded.is_some() { s.time } else { 0.0 };
    println!(
        "world {}x{} = {} tiles, seed {seed}",
        vp.w,
        vp.h,
        vp.w * vp.h
    );
    println!(
        "day  era        pop  bld  house  town  apt  off  tow  ind  shop  park | \
         road  asph  grvl  dirt  walk | lots  dev%  pave%  box | districts"
    );
    for day in 0..=days {
        if day > 0 {
            // GROWTH_RELOAD_HOURS=h: save and reload every h sim hours, the way a
            // wallpaper restart (deploy, login) does, to see whether a reload
            // costs growth.
            match std::env::var("GROWTH_RELOAD_HOURS").ok().and_then(|v| v.parse::<f64>().ok()) {
                Some(h) if h > 0.0 => {
                    let end = t0 + day as f64 * 86400.0;
                    let mut t = t0 + (day as f64 - 1.0) * 86400.0;
                    let tmp = std::env::temp_dir().join(format!("growth-reload-{}.sav", std::process::id()));
                    while t < end {
                        t = (t + h * 3600.0).min(end);
                        s.run_to(t);
                        s.save(&tmp).expect("save");
                        s = sim::Sim::load(&tmp).expect("load");
                    }
                    let _ = std::fs::remove_file(&tmp);
                }
                _ => s.run_to(t0 + day as f64 * 86400.0),
            }
        }
        if every == 0 && day % 2 != 0 && day != days && day != 8 && day != 16 && day != 24 {
            continue;
        }
        let n = |k: LotKind| s.lots.iter().filter(|l| l.kind == k).count();
        let mut grid_road = 0;
        let mut asph = 0;
        let mut grvl = 0;
        let mut dirt = 0;
        let mut walk = 0;
        let mut road = 0;
        for y in 0..s.map.h {
            for x in 0..s.map.w {
                match s.map.at(x, y).kind {
                    map::Kind::Paved | map::Kind::Road => asph += 1,
                    map::Kind::Gravel => grvl += 1,
                    map::Kind::Dirt => dirt += 1,
                    map::Kind::Sidewalk => walk += 1,
                    map::Kind::Bridge => road += 1,
                    _ => {}
                }
                if s.map.at(x, y).kind.is_lane() && s.on_grid(x, y) {
                    grid_road += 1;
                }
            }
        }
        road += asph + grvl + dirt;
        let lot_tiles: i32 = s.lots.iter().map(|l| l.w * l.h).sum();
        let _parks: i32 = s
            .lots
            .iter()
            .filter(|l| l.kind == LotKind::Park)
            .map(|l| l.w * l.h)
            .sum();
        let dev = road + lot_tiles;
        // Built-up box: every finished lot, plus the streets inside its bounds.
        let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for l in s.lots.iter().filter(|l| l.finished() && l.kind != LotKind::Park) {
            x0 = x0.min(l.x - 1);
            y0 = y0.min(l.y - 1);
            x1 = x1.max(l.x + l.w + 1);
            y1 = y1.max(l.y + l.h + 1);
        }
        let (mut inbox, mut pavebox) = (0, 0);
        if x0 <= x1 {
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let k = s.map.at(x, y).kind;
                    if k.is_lane() || k == map::Kind::Sidewalk {
                        inbox += 1;
                        if k == map::Kind::Paved || k == map::Kind::Road {
                            pavebox += 1;
                        }
                    }
                }
            }
        }
        let mut per_district = [0i32; 6];
        for l in &s.lots {
            if l.kind == LotKind::Park {
                continue;
            }
            let d = s.district_at(l.x, l.y);
            per_district[match d {
                District::Core => 0,
                District::European => 1,
                District::Suburb => 2,
                District::Downtown => 3,
                District::HighEnd => 4,
                District::Industrial => 5,
            }] += 1;
        }
        let storeys = if s.lots.is_empty() {
            0.0
        } else {
            s.lots.iter().map(|l| l.storeys as f32).sum::<f32>() / s.lots.len() as f32
        };
        println!(
            "{day:>3}  {:<10} {:>4} {:>4} {:>6} {:>5} {:>4} {:>4} {:>4} {:>4} {:>5} {:>5} | \
             {:>4} {:>5} {:>5} {:>5} {:>5} | {:>4} {:>4} {:>5} {:>3}x{:<3}| {} {} {} {} {} {} (st {storeys:.1}, grid {grid_road})",
            s.era().name(),
            s.population(),
            s.lots.len(),
            n(LotKind::House),
            n(LotKind::Townhouse),
            n(LotKind::Apartment),
            n(LotKind::Office),
            n(LotKind::Tower),
            n(LotKind::Industry),
            n(LotKind::Shop) + n(LotKind::Store),
            n(LotKind::Park),
            road,
            asph,
            grvl,
            dirt,
            walk,
            lot_tiles,
            dev,
            if inbox > 0 { pavebox * 100 / inbox } else { 0 },
            x1 - x0,
            y1 - y0,
            per_district[0],
            per_district[1],
            per_district[2],
            per_district[3],
            per_district[4],
            per_district[5],
        );
        if every == 1 {
            println!(
                "      road share {:.1}% of the built box ({:.1}% of road+lot), paved {}%",
                s.road_share() * 100.0,
                road as f32 / (road + lot_tiles).max(1) as f32 * 100.0,
                if inbox > 0 { pavebox * 100 / inbox } else { 0 }
            );
        }
    }
    let (road, lots) = s.street_ratio();
    println!(
        "road share: {:.1} % of the built-up area, {:.1} % of road+lot; {} paved tiles upgraded; {} footpaths paved",
        s.road_share() * 100.0,
        road as f32 / (road + lots).max(1) as f32 * 100.0,
        s.stats.upgraded,
        s.stats.paths_paved
    );
    println!(
        "try_lot successes {} / deleted by the D41 pass {} (net {})",
        s.stats.dev_built,
        s.stats.dev_tried,
        s.stats.dev_built as i64 - s.stats.dev_tried as i64
    );
    println!("try_lot refusals [bounds, occupied, relief, lane-adj, other-lot, gap, frontage, earthwork] = {:?}", s.lot_tries());
    let (tried, stood, refuse) = s.grid_refusals();
    println!("plots tried {tried} stood {stood} refused {refuse:?} skips {:?}", s.grid_skips());
    let (origin, open, skip, plan, at) = s.grid_report();
    let pitch = 9;
    println!(
        "grid origin {origin:?} pitch {pitch}: {open} cells open, {skip} skipped, last plan {plan} lots ({at} staked)"
    );
    if s.era() == Era::Metropolis {
        println!("metropolis reached");
    }
}
