//! B42f: the city keeps clear of the belt under the land line.
//!
//! the owner's desktop, night: houses, roads and trees standing on the land line were
//! chopped flat by the horizon. Nothing may be built on the tiles whose screen
//! row falls within ~160 px below it — those tiles stay countryside — so this
//! builds the same frame the wallpaper does and looks at what the sim put there.

use sim::{Pace, Sim};

/// the owner's union and sky band: 7040x1440, 30 % sky (D53).
const W: i32 = 7040;
const H: i32 = 1440;
const SKY: i32 = 432;
const BELT: i32 = 160;

fn desk_sim(seed: u64) -> Sim {
    // The world is generated for the full framebuffer (what `present` does),
    // and the camera comes from the *land* band plus the sky shift — the same
    // two viewports the renderer uses.
    let world = map::world_for_viewport(W, H);
    let land = map::world_for_viewport(W, H - SKY);
    let (fx, fy) = map::screen_to_tile(&world, W as f32 * 0.5, H as f32 * 0.5);
    let mut sim = Sim::with_focus(seed, Pace::Month, world.w, world.h, (fx, fy));
    sim.set_viewport(land.origin.0, land.origin.1 + SKY, W, H);
    sim.set_horizon(SKY, BELT);
    sim
}

fn built_in_the_belt(sim: &Sim) -> Vec<String> {
    let mut out = Vec::new();
    for lot in &sim.lots {
        if sim.behind_horizon(lot.x, lot.y) || sim.behind_horizon(lot.x + lot.w - 1, lot.y + lot.h - 1) {
            out.push(format!("lot at {},{} ({}x{})", lot.x, lot.y, lot.w, lot.h));
        }
    }
    for y in 0..sim.map.h {
        for x in 0..sim.map.w {
            let t = sim.map.at(x, y);
            let built = t.kind.is_lane() || matches!(t.kind, map::Kind::Sidewalk);
            if built && sim.behind_horizon(x, y) {
                out.push(format!("{:?} at {x},{y}", t.kind));
            }
        }
    }
    out
}

#[test]
fn nothing_is_built_under_the_land_line() {
    // Three seeds and two ages: the founding day and a town-sized city.
    for seed in [2184655232852813596u64, 7, 424242] {
        for days in [1.0, 12.0] {
            let mut sim = desk_sim(seed);
            sim.run_to(days * 86400.0);
            let bad = built_in_the_belt(&sim);
            assert!(
                bad.is_empty(),
                "seed {seed} day {days}: {} thing(s) in the horizon belt, e.g. {:?}",
                bad.len(),
                &bad[..bad.len().min(6)]
            );
            // ...and the belt is not empty countryside either: the test would
            // pass trivially if the view were mis-set and the belt covered
            // everything (or nothing).
            let belt_tiles = (0..sim.map.h)
                .flat_map(|y| (0..sim.map.w).map(move |x| (x, y)))
                .filter(|&(x, y)| sim.behind_horizon(x, y))
                .count();
            assert!(
                belt_tiles > 40,
                "seed {seed}: only {belt_tiles} tiles in the belt — the view is wrong"
            );
            assert!(
                sim.lots.len() > 5,
                "seed {seed} day {days}: the city never grew ({} lots)",
                sim.lots.len()
            );
        }
    }
}

/// The belt is the sim's *view*, not its state: without a sky band nothing is
/// reserved, so a headless frame (and every existing test) behaves as before.
#[test]
fn without_a_sky_band_nothing_is_reserved() {
    let vp = map::world_for_viewport(1920, 1080);
    let (fx, fy) = map::screen_to_tile(&vp, 960.0, 540.0);
    let mut sim = Sim::with_focus(7, Pace::Month, vp.w, vp.h, (fx, fy));
    sim.set_viewport(vp.origin.0, vp.origin.1, 1920, 1080);
    sim.set_horizon(0, BELT);
    assert_eq!(sim.back_belt(), (0, 0));
    let mut in_belt = 0;
    for y in 0..sim.map.h {
        for x in 0..sim.map.w {
            if sim.behind_horizon(x, y) {
                in_belt += 1;
            }
        }
    }
    assert_eq!(in_belt, 0);
}
