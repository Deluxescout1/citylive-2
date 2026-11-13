//! B88: where does a reloaded city part ways with one that never stopped?
//!
//! ```text
//! cargo run --release -p sim --example reloadparity -- 7 8 [start_day]
//! ```
//! A live sim and a twin that is saved and reloaded every sim hour run side by
//! side from `start_day` (default 2, after the settlement); each hour the two
//! saves are compared byte for byte. Prints the first hours that differ and the
//! first differing offsets.

use sim::{Pace, Sim};

fn main() {
    let mut args = std::env::args().skip(1);
    let seed: u64 = args.next().and_then(|a| a.parse().ok()).unwrap_or(7);
    let days: f64 = args.next().and_then(|a| a.parse().ok()).unwrap_or(8.0);
    let start: f64 = args.next().and_then(|a| a.parse().ok()).unwrap_or(2.0);
    let (fw, fh) = (7040, 1440);
    let vp = map::world_for_viewport(fw, fh);
    let focus = map::screen_to_tile(&vp, fw as f32 / 2.0, fh as f32 / 2.0);
    let mut live = Sim::with_focus(seed, Pace::Month, vp.w, vp.h, focus);
    live.run_to(start * 86400.0);
    let dir = std::env::temp_dir().join(format!("reloadparity-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (pa, pb) = (dir.join("a.sav"), dir.join("b.sav"));
    live.save(&pa).unwrap();
    let mut twin = Sim::load(&pa).unwrap();
    let mut shown = 0;
    let mut t = start * 86400.0;
    while t < days * 86400.0 {
        t += 3600.0;
        live.run_to(t);
        twin.run_to(t);
        live.save(&pa).unwrap();
        twin.save(&pb).unwrap();
        let (a, b) = (std::fs::read(&pa).unwrap(), std::fs::read(&pb).unwrap());
        if a != b && shown < 6 {
            shown += 1;
            let first = a.iter().zip(b.iter()).position(|(x, y)| x != y);
            let n = a.iter().zip(b.iter()).filter(|(x, y)| x != y).count();
            println!(
                "hour {:.2}d: saves differ, len {} vs {}, first diff at {:?} of {}, {} bytes differ; lots {} vs {}",
                t / 86400.0,
                a.len(),
                b.len(),
                first,
                a.len().min(b.len()),
                n,
                live.lots.len(),
                twin.lots.len()
            );
        }
        if a != b && shown == 1 {
            let mut k = 0;
            for y in 0..live.map.h {
                for x in 0..live.map.w {
                    let (ta, tb) = (live.map.at(x, y), twin.map.at(x, y));
                    if ta.kind != tb.kind || ta.corner != tb.corner || ta.aux != tb.aux {
                        if k < 12 {
                            println!("  tile {x},{y}: live {:?} aux {} vs twin {:?} aux {}", ta.kind, ta.aux, tb.kind, tb.aux);
                        }
                        k += 1;
                    }
                    if live.map.wear(x, y) != twin.map.wear(x, y) {
                        k += 1000000;
                    }
                }
            }
            println!("  {k} tile diffs (+1e6 per wear diff)");
            for (i, (la, lb)) in live.lots.iter().zip(twin.lots.iter()).enumerate() {
                if la != lb {
                    println!("  lot {i}: {la:?} vs {lb:?}");
                }
            }
            if let Ok(d) = std::env::var("PARITY_KEEP") {
                std::fs::write(format!("{d}/live.sav"), &a).unwrap();
                std::fs::write(format!("{d}/twin.sav"), &b).unwrap();
            }
            shown += 1;
        }
        twin =Sim::load(&pb).unwrap();
    }
    println!(
        "end: lots {} vs {}, pop {} vs {}",
        live.lots.len(),
        twin.lots.len(),
        live.population(),
        twin.population()
    );
    let _ = std::fs::remove_dir_all(&dir);
}
