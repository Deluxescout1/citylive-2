//! Scratch: how the desire paths build up over the days (D45).
//!
//! ```text
//! cargo run -p sim --example paths -- 7
//! ```

use map::WearStage;

fn main() {
    let seed: u64 = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(7);
    let vp = map::world_for_viewport(960, 540);
    let mut s = sim::Sim::with_focus(
        seed,
        sim::Pace::Month,
        vp.w,
        vp.h,
        (vp.w as f32 * 0.5, vp.h as f32 * 0.5),
    );
    println!("day  trampled footpath cart  lanes  paved-by-paths  decisions  max_us");
    for day in 0..=12 {
        if day > 0 {
            s.run_to(day as f64 * 86400.0);
        }
        let mut n = [0usize; 4];
        for w in &s.map.wear {
            match WearStage::of(*w) {
                WearStage::None => n[0] += 1,
                WearStage::Trampled => n[1] += 1,
                WearStage::Footpath => n[2] += 1,
                WearStage::CartTrack => n[3] += 1,
            }
        }
        println!(
            "{day:>3}  {:>8} {:>8} {:>4}  {:>5}  {:>13}  {:>9}  {:>6}",
            n[1],
            n[2],
            n[3],
            s.stats.lanes,
            s.stats.paths_paved,
            s.stats.decisions,
            s.stats.max_decision_us
        );
    }
}
