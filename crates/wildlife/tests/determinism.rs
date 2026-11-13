//! B22 test: two `Wildlife` instances from the same seed follow the same run.
//!
//! Public-API check (unit tests already cover the in-crate view): build the
//! layer twice on the same map, drive both with identical dt / time-of-day
//! sequences, and compare fingerprints. A different seed must diverge.

use map::{world_for_viewport, generate};
use wildlife::Wildlife;

fn run_wildlife(seed: u64, secs: usize, tod: f32) -> u64 {
    let vp = world_for_viewport(960, 540);
    let map = generate(7, vp.w, vp.h);
    let mut wl = Wildlife::new(seed, &map);
    let dt = 1.0 / 30.0;
    for i in 0..secs {
        // A slowly rotating clock so day/night masks flip mid-run (owls and
        // bats enter, land animals thin out) — the mask edge is the most
        // likely source of order-dependent non-determinism.
        wl.update(dt, &map, &[], (tod + i as f32 * dt).rem_euclid(24.0));
    }
    wl.state_fingerprint()
}

#[test]
fn same_seed_same_run() {
    let a = run_wildlife(0xC0FFEE, 6_000, 11.0);
    let b = run_wildlife(0xC0FFEE, 6_000, 11.0);
    assert_eq!(a, b, "same seed must replay the same run");
}

#[test]
fn different_seed_diverges() {
    let a = run_wildlife(0xC0FFEE, 6_000, 11.0);
    let b = run_wildlife(0xC0FFEE ^ 1, 6_000, 11.0);
    assert_ne!(a, b, "a different seed should produce a different run");
}
