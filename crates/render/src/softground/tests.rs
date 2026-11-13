use super::*;

fn world(seed: u64) -> TileMap {
    map::generate(seed, 96, 96)
}

fn corner_dz(m: &TileMap, vx: i32, vy: i32) -> Vec<(i32, i32, usize, i8)> {
    let mut out = Vec::new();
    for i in 0..4 {
        let (tx, ty) = (vx - CX[i], vy - CY[i]);
        if tx >= 0 && ty >= 0 && tx < m.w && ty < m.h {
            out.push((tx, ty, i, m.soft_dz(tx, ty)[i]));
        }
    }
    out
}

#[test]
fn smoothing_is_deterministic_and_bounded() {
    let mut a = world(5);
    let mut b = world(5);
    compute(&mut a, &|_, _| false);
    compute(&mut b, &|_, _| false);
    assert_eq!(a.soft.dz, b.soft.dz);
    let mut moved = 0;
    for d in a.soft.dz.iter().flatten() {
        assert!(d.abs() <= 6, "half a level at most, got {d}");
        if *d != 0 {
            moved += 1;
        }
    }
    assert!(moved > 100, "the terrain should have something to smooth, moved {moved}");
}

/// Slope tiles cover their footprint: every vertex is drawn at one height by
/// every tile that touches it, so no gap or overlap opens between tiles; a
/// vertex that any hard tile (road, water, lot) or step touches does not move.
#[test]
fn tiles_share_corners_and_hard_tiles_stay_put() {
    for seed in [3u64, 5, 9] {
        let mut m = world(seed);
        // Lay a flat road across the middle and a lot beside it.
        for x in 10..80 {
            let mut t = m.at(x, 40);
            t.kind = Kind::Paved;
            t.corner = [t.corner[0]; 4];
            m.set(x, 40, t);
        }
        let occ = |x: i32, y: i32| (20..24).contains(&x) && (50..54).contains(&y);
        compute(&mut m, &occ);
        for vy in 0..=m.h {
            for vx in 0..=m.w {
                let tiles = corner_dz(&m, vx, vy);
                if tiles.iter().all(|t| t.3 == 0) {
                    continue;
                }
                // Every tile on a moved vertex is drawn at the same height.
                let heights: Vec<f32> = tiles
                    .iter()
                    .map(|&(tx, ty, i, d)| {
                        m.at(tx, ty).corner[i] as f32 + d as f32 / crate::iso::HEIGHT_UNIT as f32
                    })
                    .collect();
                for h in &heights {
                    assert!((h - heights[0]).abs() < 1e-4, "gap at vertex ({vx},{vy})");
                }
                // ... and none of them is a road, a shore, a rock face or a lot.
                for &(tx, ty, _, _) in &tiles {
                    let t = m.at(tx, ty);
                    assert!(soft_kind(t.kind) && !occ(tx, ty), "hard tile moved at ({tx},{ty})");
                }
            }
        }
        // The road tiles themselves are exactly as the sim left them.
        for x in 10..80 {
            assert_eq!(m.soft_dz(x, 40), [0; 4]);
            assert!(m.soft_lit(x, 40).is_none());
        }
    }
}

#[test]
fn recompute_reports_only_what_changed() {
    let mut m = world(5);
    let first = compute(&mut m, &|_, _| false);
    assert!(!first.is_empty());
    assert!(compute(&mut m, &|_, _| false).is_empty());
    // A new lot changes the ground around it and nowhere far away.
    let changed = compute(&mut m, &|x, y| (30..33).contains(&x) && (30..33).contains(&y));
    assert!(changed.iter().all(|&(x, y)| (18..45).contains(&x) && (18..45).contains(&y)));
}

#[test]
fn recompute_time() {
    let mut m = world(5);
    let t0 = std::time::Instant::now();
    for _ in 0..20 {
        compute(&mut m, &|_, _| false);
    }
    eprintln!("softground::compute 96x96: {:?} per call", t0.elapsed() / 20);
}
