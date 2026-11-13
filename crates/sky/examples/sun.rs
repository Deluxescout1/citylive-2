//! Scratch: sun and moon positions for a fixed time, so screenshots can be
//! checked against what should be in the sky (H20 review asked whether the moon
//! was up in `H20-night.png`).
//!
//! ```text
//! cargo run -p sky --example sun -- 22:00 2026-11-09
//! ```

use sky::{clock_ms, palette, state};

const LAT: f64 = 41.0; // Greenwich, London
const LON: f64 = -75.0;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let times: Vec<String> = if args.is_empty() {
        vec!["06:50".into(), "11:40".into(), "16:00".into(), "16:20".into(), "17:10".into(), "22:00".into()]
    } else {
        args.iter().filter(|a| a.contains(':')).cloned().collect()
    };
    let date = args.iter().find(|a| a.contains('-')).cloned();
    for t in times {
        let ms = clock_ms(Some(&t), date.as_deref(), LON);
        let s = state(ms, LAT, LON);
        let p = palette::palette(&s);
        println!(
            "{t}  sun alt {:6.2}° az {:6.1}°   moon alt {:6.2}° az {:6.1}° illum {:.2} phase {:.2}  phase {:?}  stars {:.2}",
            s.sun_alt, s.sun_az, s.moon_alt, s.moon_az, s.moon_illum, s.moon_phase, s.phase, p.star_alpha
        );
    }
}
