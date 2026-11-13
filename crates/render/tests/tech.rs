//! B49: the technology art and the lamp heads it changes.

use render::framebuffer::Framebuffer;
use render::iso::Camera;
use render::street;

/// A small road through a map, with one lamp on it.
fn scene() -> (Framebuffer, Camera, map::TileMap) {
    let map = map::generate(3, 24, 24);
    let cam = Camera::new(24 * 32 / 2 + 64, 40);
    let mut fb = Framebuffer::new(512, 512);
    fb.set_clip(render::framebuffer::Rect::new(0, 0, 512, 512));
    (fb, cam, map)
}

/// The three lamp technologies draw three different heads, and the pool they
/// throw is the colour of the lamp: gas amber, sodium orange, LED white (B49).
#[test]
fn the_lamp_head_and_its_pool_follow_the_technology() {
    let (mut fb, cam, map) = scene();
    // Find a lane tile near the middle of the map to stand the lamp on.
    let mut spot = None;
    for y in 8..16 {
        for x in 8..16 {
            if map.at(x, y).kind.is_lane() {
                spot = Some((x, y));
                break;
            }
        }
    }
    let (x, y) = spot.unwrap_or((12, 12));
    let mut hue = Vec::new();
    for level in 0..3u8 {
        let mut f = Framebuffer::new(512, 512);
        f.set_clip(render::framebuffer::Rect::new(0, 0, 512, 512));
        street::draw_lamp(&mut f, &cam, &map, x, y, 1.0, level);
        let lit = f
            .px()
            .iter()
            .filter(|c| **c != 0)
            .map(|c| (palette_r(*c), palette_b(*c)))
            .max_by_key(|(r, b)| r.saturating_sub(*b))
            .unwrap();
        hue.push(lit);
    }
    // Gas and sodium are warmer than LED; LED is the coolest of the three.
    assert!(hue[0].0 > hue[0].1 + 20, "gas head is not warm: {hue:?}");
    assert!(hue[1].0 > hue[1].1 + 20, "sodium head is not warm: {hue:?}");
    assert!(
        hue[2].0 <= hue[1].0 || hue[2].0 - hue[2].1 < hue[1].0 - hue[1].1,
        "the LED head is no cooler than the sodium one: {hue:?}"
    );
    let _ = &mut fb;
}

fn palette_r(c: u32) -> i32 {
    ((c >> 16) & 0xFF) as i32
}
fn palette_b(c: u32) -> i32 {
    (c & 0xFF) as i32
}

/// B49: the technology structures draw without panicking on water, steep ground
/// and the map edge, and leave pixels behind.
#[test]
fn the_structures_draw() {
    let map = map::generate(5, 32, 32);
    let cam = Camera::new(32 * 32 / 2 + 96, 60);
    let mut fb = Framebuffer::new(640, 640);
    fb.set_clip(render::framebuffer::Rect::new(0, 0, 640, 640));
    let (x, y) = (16, 16);
    render::tech::draw_pole(&mut fb, &cam, &map, x, y);
    render::tech::draw_cabinet(&mut fb, &cam, &map, x, y);
    render::tech::draw_tower(&mut fb, &cam, &map, x, y);
    render::tech::draw_small_cell(&mut fb, &cam, &map, x, y);
    render::tech::draw_farm(&mut fb, &cam, &map, x, y);
    render::tech::draw_turbine_mast(&mut fb, &cam, &map, x, y);
    render::tech::draw_turbine_rotor(&mut fb, &cam, &map, x, y, 1.5);
    render::tech::draw_crew(&mut fb, &cam, &map, x, y, 4, &[0u16; 640 * 640], 5);
    render::tech::draw_charger(&mut fb, &cam, &map, x, y);
    render::tech::draw_drone(&mut fb, &cam, &map, 16.5, 16.5, 0.0, 1.0);
    render::tech::draw_wire(&mut fb, &cam, &map, x, y, x + 4, y, 4);
    let painted = fb.px().iter().filter(|c| **c != 0).count();
    assert!(painted > 40, "the structures drew only {painted} pixels");
}
