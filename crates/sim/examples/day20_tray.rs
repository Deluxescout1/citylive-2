//! The tray's picture of a day-20 city (B48a item 4).
//!
//! ```text
//! cargo run --release -p sim --example day20_tray            # seed 40
//! cargo run --release -p sim --example day20_tray -- 7       # another world
//! ```
//!
//! Runs a world to day 20 at month pace (the same founding the wallpaper
//! uses) and prints exactly what the tray menu would show: the city's name
//! (its title), the headcount and era line, and the latest eight story lines,
//! newest first. The tray itself is a StatusNotifier menu, not a frame — this
//! is the model the menu is built from.

use sim::Pace;

fn main() {
    let seed: u64 = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(40);
    let vp = map::world_for_viewport(960, 540);
    let focus = map::screen_to_tile(&vp, 480.0, 270.0);
    let mut s = sim::Sim::with_focus(seed, Pace::Month, vp.w, vp.h, focus);
    s.run_to(20.0 * 86400.0);

    let days = s.days();
    let status = s.tray_status("", false);
    println!("day {days:.1}  seed {seed}  {}", s.biome.name());
    println!("the tray menu, day 20:");
    for line in status.as_menu() {
        println!("  {line}");
    }
    println!();
    println!("the full feed the city has told so far (oldest first):");
    for l in s.story.iter() {
        println!("  day {:5.1}  {}", l.time / 86400.0, l.text);
    }
}
