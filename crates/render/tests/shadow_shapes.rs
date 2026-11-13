//! B68: the shadow of every kind of caster, checked against the sun.
//!
//! Direction and length per caster type: a box or a kit house starts at its
//! wall and reaches `height / tan(altitude)`, a tree is a canopy on a trunk
//! line (not a column), a hip roof's ridge shows in its outline, a person or a
//! car drags the swept footprint along the same light, and a pole is a line.

use map::{Tile, TileMap};
use render::framebuffer::Framebuffer;
use render::iso::Camera;
use render::light::{self, LightState, Orb};
use render::palette;
use render::shadow::{HeightField, bake_mask};

const G: f32 = 2.0 * map::iso_consts::HEIGHT_UNIT as f32;

fn sun(alt: f64, az: f64) -> LightState {
    let mut ls = LightState::default();
    ls.sun = Orb::new(alt, az);
    ls.dir_strength = 1.0;
    ls
}

fn field() -> HeightField {
    HeightField::new(&TileMap::fill(40, 40, Tile::flat(2), 1))
}

fn at(f: &HeightField, m: &[u8], x: f32, y: f32) -> u8 {
    m[(y * f.n as f32) as usize * f.w + (x * f.n as f32) as usize]
}

/// The farthest distance (tiles) from `(x, y)` along `(dx, dy)` still shadowed.
fn reach(f: &HeightField, m: &[u8], x: f32, y: f32, dx: f32, dy: f32) -> f32 {
    let mut far = 0.0;
    let mut d = 0.05;
    while d < 12.0 {
        if at(f, m, x + dx * d, y + dy * d) > 128 {
            far = d;
        }
        d += 0.04;
    }
    far
}

/// For every azimuth and altitude the shadow of a box lies on the far side of
/// the sun and is `height / tan(alt)` long (the same for every azimuth).
#[test]
fn a_box_shadow_follows_the_sun_at_every_azimuth_and_altitude() {
    let mut f = field();
    let h = 60.0;
    f.stamp_rect(19.0, 19.0, 20.0, 20.0, G + h);
    for alt in [15.0f64, 30.0, 50.0] {
        let want = h / ((alt as f32).to_radians().tan() * 64.0);
        for az in (0..360).step_by(45) {
            let mut mask = Vec::new();
            let c = bake_mask(&f, &sun(alt, az as f64), &mut mask).unwrap();
            let away = (-c.toward.0, -c.toward.1);
            let edge = 0.5 / away.0.abs().max(away.1.abs());
            let far = reach(&f, &mask, 19.5, 19.5, away.0, away.1);
            assert!(
                (far - (edge + want)).abs() < 0.3 + want * 0.15,
                "alt {alt} az {az}: reach {far:.2}, want {:.2}",
                edge + want
            );
            // Nothing on the sunny side, beyond the box itself.
            assert_eq!(
                at(&f, &mask, 19.5 - away.0 * (edge + 0.4), 19.5 - away.1 * (edge + 0.4)),
                0,
                "alt {alt} az {az}"
            );
        }
    }
}

/// A shadow starts at the foot of the wall: the sample just outside the box on
/// the far side is already dark.
#[test]
fn a_shadow_touches_its_caster() {
    let mut f = field();
    f.stamp_rect(19.0, 19.0, 20.0, 20.0, G + 40.0);
    let mut mask = Vec::new();
    let c = bake_mask(&f, &sun(30.0, 120.0), &mut mask).unwrap();
    let away = (-c.toward.0, -c.toward.1);
    let edge = 0.5 / away.0.abs().max(away.1.abs());
    assert!(at(&f, &mask, 19.5 + away.0 * (edge + 0.12), 19.5 + away.1 * (edge + 0.12)) > 200);
}

/// A taller tower throws a proportionally longer shadow than a house.
#[test]
fn a_tower_throws_a_longer_shadow_than_a_house() {
    let mut f = field();
    f.stamp_rect(10.0, 19.0, 11.0, 20.0, G + 24.0);
    f.stamp_rect(29.0, 19.0, 30.0, 20.0, G + 120.0);
    let mut mask = Vec::new();
    let c = bake_mask(&f, &sun(25.0, 200.0), &mut mask).unwrap();
    let a = (-c.toward.0, -c.toward.1);
    let house = reach(&f, &mask, 10.5, 19.5, a.0, a.1);
    let tower = reach(&f, &mask, 29.5, 19.5, a.0, a.1);
    assert!(tower > 3.0 * house, "tower {tower:.2} vs house {house:.2}");
}

/// A pitched roof is in the outline: its shadow reaches further than the shadow
/// of a box that only goes up to the eave, and the ridge line's shadow runs
/// further than the eave's.
#[test]
fn a_hip_roof_shows_its_ridge_in_the_shadow() {
    let mut f = field();
    // 2 x 1 footprint, eave 20 px, ridge 44 px, ridge along x.
    f.stamp_hip(18.0, 19.0, 20.0, 20.0, G, 20.0, 44.0);
    let mut mask = Vec::new();
    let c = bake_mask(&f, &sun(30.0, 45.0), &mut mask).unwrap();
    let (ex, ey) = (-c.toward.0, -c.toward.1);
    let ridge = reach(&f, &mask, 19.0, 19.5, ex, ey);
    let mut flat = field();
    flat.stamp_rect(18.0, 19.0, 20.0, 20.0, G + 20.0);
    let mut m2 = Vec::new();
    bake_mask(&flat, &sun(30.0, 45.0), &mut m2);
    let boxy = reach(&flat, &m2, 19.0, 19.5, ex, ey);
    assert!(ridge > boxy + 0.1, "roof {ridge:.2} vs eave box {boxy:.2}");
    // Higher at the ridge line than at the eaves: the roof is not a slab.
    assert!(f.height_above_ground(19.0, 19.5) > f.height_above_ground(19.0, 19.06) + 8.0);
}

/// A tree is a crown on a trunk line: the crown's shadow sits about its height
/// away from the trunk, with a lit gap between, not a column from the base.
#[test]
fn a_tree_is_a_crown_on_a_trunk() {
    let mut f = field();
    let tile = Tile {
        prop: map::Prop::Tree { size: 1, species: map::TreeSpecies::ALL[0], variant: 0 },
        ..Tile::flat(2)
    };
    let mut m = TileMap::fill(40, 40, Tile::flat(2), 1);
    m.set(20, 20, tile);
    f.rebuild(&m, &[], &[], &[]);
    let mut mask = Vec::new();
    let c = bake_mask(&f, &sun(45.0, 90.0), &mut mask).unwrap();
    let (ex, ey) = (-c.toward.0, -c.toward.1);
    let rise = render::terrain::prop_rise(&tile) as f32;
    let crown = rise * 0.64 / 64.0; // tiles from the trunk to the crown's shadow centre at 45 degrees
    let mid = at(&f, &mask, 20.5 + ex * crown, 20.5 + ey * crown);
    assert!(mid > 200, "the crown's shadow is missing: {mid}");
    // Between the trunk and the crown's shadow the ground is not solid dark.
    let gap = (0..8)
        .map(|i| at(&f, &mask, 20.5 + ex * (0.12 + i as f32 * 0.01), 20.5 + ey * (0.12 + i as f32 * 0.01)))
        .min()
        .unwrap();
    assert!(gap < 200, "a column, not a crown: {gap}");
    // And it lies where the sun says: nothing on the sunny side.
    assert_eq!(at(&f, &mask, 20.5 - ex * crown, 20.5 - ey * crown), 0);
}

/// A kit sprite's wall footprint is read off its silhouette.
#[test]
fn the_footprint_is_read_from_the_silhouette() {
    let (w, h, ox, oy) = (160usize, 120usize, 64i32, 50i32);
    let mut px = vec![palette::TRANSPARENT; w * h];
    let proj = |x: f32, y: f32, z: f32| {
        ((ox as f32 + (x - y) * 32.0).round() as i32, (oy as f32 + (x + y) * 16.0 - z).round() as i32)
    };
    for zi in 0..=30 {
        for t in 0..=40 {
            let tt = t as f32 / 40.0;
            for (a, b) in [(1.2 * tt, 0.0), (1.2, 1.0 * tt), (1.2 * tt, 1.0), (0.0, 1.0 * tt)] {
                let (sx, sy) = proj(a, b, zi as f32);
                px[sy as usize * w + sx as usize] = palette::rgb(100, 100, 100);
            }
        }
    }
    let f = render::scene::KitSprite::footprint_of(&px, w, h, ox, oy);
    assert!(f[0].abs() < 0.1 && f[1].abs() < 0.1, "{f:?}");
    assert!((f[2] - 1.2).abs() < 0.1 && (f[3] - 1.0).abs() < 0.1, "{f:?}");
}

/// A person and a car drag their swept footprint along the light: the far end
/// of the shadow is where `ground_offset` says, and it is darker, cooler
/// ground rather than grey paint.
#[test]
fn an_actor_shadow_runs_away_from_the_sun_by_height_over_tan() {
    let cam = Camera::new(200, 40);
    for (alt, az) in [(20.0f64, 90.0f64), (35.0, 200.0), (15.0, 280.0)] {
        let ls = sun(alt, az);
        let s = ls.actor().unwrap();
        for height in [11.0f32, 26.0] {
            let mut fb = Framebuffer::new(400, 240);
            let grass = palette::rgb(110, 170, 80);
            for p in fb.px_mut().iter_mut() {
                *p = grass;
            }
            let (x, y) = (6.0f32, 6.0f32);
            light::draw_actor_shadow(&mut fb, &cam, &[], 0, x, y, 0, 0.04, 0.04, height, &s, false);
            let (off_x, off_y) = ls.ground_offset(height).unwrap();
            let tip = cam.project(x + off_x, y + off_y, 0);
            let foot = cam.project(x, y, 0);
            let mut far = (0.0f32, (0i32, 0i32));
            let mut dark = 0;
            for yy in 0..240 {
                for xx in 0..400 {
                    let c = fb.px()[yy * 400 + xx];
                    if c != grass {
                        dark += 1;
                        let d = ((xx as f32 - foot.0).powi(2) + (yy as f32 - foot.1).powi(2)).sqrt();
                        if d > far.0 {
                            far = (d, (xx as i32, yy as i32));
                        }
                        assert!(palette::r(c) <= palette::r(grass) && palette::b(c) <= palette::b(grass));
                        // cooler: blue keeps more of itself than red does
                        assert!(
                            palette::b(c) as f32 * palette::r(grass) as f32
                                >= palette::r(c) as f32 * palette::b(grass) as f32 - 1.0
                        );
                    }
                }
            }
            assert!(dark > 0);
            let want = ((tip.0 - foot.0).powi(2) + (tip.1 - foot.1).powi(2)).sqrt();
            assert!(
                (far.0 - want).abs() < 4.0 + want * 0.1,
                "alt {alt} az {az} h {height}: far {:.1} want {want:.1}",
                far.0
            );
            assert!((far.1.0 as f32 - foot.0) * (tip.0 - foot.0) >= -1.5, "wrong side horizontally");
        }
    }
}

/// A figure standing where the static shadow already is is not darkened twice.
#[test]
fn a_shadow_is_not_cast_twice() {
    let cam = Camera::new(200, 40);
    let ls = sun(25.0, 180.0);
    let s = ls.actor().unwrap();
    let grass = palette::rgb(110, 170, 80);
    let mk = || {
        let mut fb = Framebuffer::new(400, 240);
        for p in fb.px_mut().iter_mut() {
            *p = grass;
        }
        fb
    };
    let mut a = mk();
    let mut b = mk();
    let full = vec![255u8; 400 * 240];
    light::draw_actor_shadow_in(&mut a, &cam, &[], &[], 0, 6.0, 6.0, 0, 0.1, 0.1, 20.0, &s, false);
    light::draw_actor_shadow_in(&mut b, &cam, &[], &full, 0, 6.0, 6.0, 0, 0.1, 0.1, 20.0, &s, false);
    assert!(a.px().iter().any(|&c| c != grass));
    assert!(b.px().iter().all(|&c| c == grass), "already in shade: nothing more to darken");
}

/// A pole's shadow is a line from its foot along the light, the right way round.
#[test]
fn a_pole_throws_a_line_away_from_the_sun() {
    let mut layer = Framebuffer::new(300, 200);
    for p in layer.px_mut().iter_mut() {
        *p = palette::with_alpha(palette::rgb(90, 90, 90), 0xFF);
    }
    for az in [90.0f64, 270.0] {
        let ls = sun(25.0, az);
        let c = ls.caster().unwrap();
        let mut plane = vec![0u8; 300 * 200];
        render::shadow::stamp_posts(&mut plane, &layer, &c, &[(150.0, 100.0, 24.0)]);
        let hits: Vec<(i32, i32)> = (0..plane.len())
            .filter(|&i| plane[i] > 0)
            .map(|i| ((i % 300) as i32, (i / 300) as i32))
            .collect();
        assert!(hits.len() > 20);
        let far = hits.iter().max_by_key(|p| (p.0 - 150).pow(2) + (p.1 - 100).pow(2)).unwrap();
        // Morning sun (east, screen left) throws it screen-right, evening left.
        if az == 90.0 {
            assert!(far.0 > 150 + 20, "{far:?}");
        } else {
            assert!(far.0 < 150 - 20, "{far:?}");
        }
        let len = (((far.0 - 150).pow(2) + (far.1 - 100).pow(2)) as f32).sqrt();
        let (tx, ty) = ls.ground_offset(24.0).unwrap();
        let want = (((tx - ty) * 32.0).powi(2) + ((tx + ty) * 16.0).powi(2)).sqrt();
        assert!((len - want).abs() < 3.0, "len {len} want {want}");
    }
}
