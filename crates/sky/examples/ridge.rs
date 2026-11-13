//! Scratch: print one ridge layer's skyline as ASCII, to eyeball the profile.
//!
//! ```text
//! cargo run -p sky --example ridge
//! ```

fn main() {
    let w = 960;
    for (name, seed, amp, peaks) in [
        ("far", 0x51 ^ 0x9E37, 46.0f32, 2.6f32),
        ("mid", 0x52 ^ 0x9E37, 30.0, 3.4),
        ("near", 0x53 ^ 0x9E37, 16.0, 4.4),
    ] {
        let cols = 96;
        let hs: Vec<f32> = (0..cols)
            .map(|i| sky::backdrop::ridge_profile(i * w / cols, w, seed, amp, peaks))
            .collect();
        println!("{name} (amp {amp}, {peaks} peaks):");
        for row in (1..=12).rev() {
            let line: String = hs
                .iter()
                .map(|&h| if h * 12.0 / amp >= row as f32 { '#' } else { ' ' })
                .collect();
            println!("{line}");
        }
    }
}
