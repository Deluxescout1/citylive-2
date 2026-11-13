fn main() {
    for seed in [2u64, 4] {
        let vp = map::world_for_viewport(960, 540);
        let mut s = sim::Sim::new(seed, sim::Pace::Month, vp.w, vp.h);
        let (fx, fy) = map::screen_to_tile(&vp, 480.0, 270.0);
        s.set_focus(fx, fy);
        s.run_to(8.0 * 86400.0);
        let mut n = 0;
        for y in 0..s.map.h { for x in 0..s.map.w {
            let t = s.map.at(x, y);
            if t.kind == map::Kind::Bridge {
                if n < 4 {
                    let sx = vp.origin.0 + (x - y) * 16;
                    let sy = vp.origin.1 + (x + y) * 8;
                    println!("seed {seed}: bridge at tile {x},{y} -> screen {sx},{sy} level {}", t.corner[0]);
                }
                n += 1;
            }
        }}
        println!("seed {seed}: {n} bridge tiles, {} lanes", s.lane_tiles().len());
    }
}
