fn main() {
    let (lat, lon) = (41.0, -75.0);
    let base = 1_760_000_000_000i64; // arbitrary date
    for h in [12, 18, 19, 20, 21, 23, 1, 5, 6, 7] {
        let ms = base + h * 3_600_000;
        let st = sky::state(ms, lat, lon);
        let p = sky::palette::palette(&st);
        println!("{:02}:00 {:?} alt={:.1} tint={:.2?} glow={:.2} stars={:.2} zen={:#08x} hor={:#08x}",
            h, st.phase, st.sun_alt, p.ambient_tint, p.window_glow, p.star_alpha, p.zenith, p.horizon);
    }
}
