//! Rail, trams, boats and planes (B31): placeholder sprites plus the **static**
//! track, station platform, yard, marina pontoon and container stacks.
//!
//! The static parts are baked into the scene's static layer (they never move, so
//! a dirty-rect composite must not erase them); the movers are drawn per frame
//! with the depth plane, like the traffic.

use crate::framebuffer::{Framebuffer, Sprite};
use crate::iso::{Camera, HEIGHT_UNIT};
use crate::palette::{self, pal, Color};
use map::TileMap;

/// What is moving, mirrored from `sim::rail::MoverKind`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Train,
    Tram,
    Boat,
    Ship,
    Plane,
    /// A bus on its line (B58).
    Bus,
    /// A harbour ferry.
    Ferry,
    /// An aircraft on the runway: taxi, take-off or landing.
    Aircraft,
    /// A level crossing's barrier arms and lights.
    Crossing,
    /// A container crane's trolley and its load.
    Crane,
    /// A worker on the cut-and-cover (B58: the subway is built visibly).
    Crew,
    /// B79: a person, a gull, a towel on the shore (`set` is the shore kind
    /// low nibble, variant high; `lift` the height in px).
    Shore,
}

/// One moving thing, ready to draw.
#[derive(Clone, Copy, Debug)]
pub struct Body {
    /// Position of the head unit in tile space.
    pub x: f32,
    pub y: f32,
    pub dir: (f32, f32),
    pub kind: Kind,
    /// Trailing units (wagons, a tram's second car).
    pub cars: u8,
    /// Route points *behind* the head, so the cars follow the track.
    pub trail: [(f32, f32); 8],
    /// Pixels above the ground (planes).
    pub lift: i32,
    /// Lit (planes blink at night, trains carry lights).
    pub night: bool,
    /// Seconds since the mover appeared, for blinking.
    pub age: f32,
    /// The set: 0 diesel passenger, 1 electric passenger, 2 freight (B58).
    pub set: u8,
    /// A continuous 0..1 phase (a barrier's arm, a crane's trolley).
    pub phase: f32,
}

/// Static things that belong in the baked layer.
#[derive(Default, Clone)]
pub struct Fixtures {
    /// Rail tiles, in order (the line).
    pub track: Vec<(i32, i32)>,
    /// Level crossings (draw the sleepers across the street).
    pub crossings: Vec<(i32, i32)>,
    pub station: Option<(i32, i32)>,
    /// The platform beside the track: (x, y, side) (B31b).
    pub station_platform: Option<(i32, i32, u8)>,
    /// The tram avenue and its stops (B31b): rails in the street, kerbs at the
    /// stops.
    pub tram_line: Vec<(i32, i32)>,
    pub tram_stops: Vec<(i32, i32)>,
    pub yard: Option<(i32, i32, i32, i32)>,
    pub marina: Option<(i32, i32, i32, i32)>,
    pub port: Option<(i32, i32, i32, i32)>,
    pub containers: Vec<(i32, i32, u8)>,
    /// Everything else the city's transport builds (B58): the second track, the
    /// station, the bus and tram stops, the subway works, the airfield and the
    /// harbour. One value, so a change is one comparison.
    pub transit: Transit,
}

/// The transport fixtures that are not part of the railway itself (B58). All
/// primitives, so the whole thing compares cheaply and clones into the static
/// bake.
#[derive(Default, Clone, PartialEq, Debug)]
pub struct Transit {
    pub twin: Vec<(i32, i32)>,
    pub platforms: Vec<(i32, i32, u8)>,
    pub canopy: Vec<(i32, i32)>,
    pub station_build: Option<(i32, i32, i32, i32)>,
    pub station_kind: u8,
    pub sidings: Vec<(i32, i32)>,
    pub parked_wagons: Vec<((i32, i32), u8)>,
    pub tram_sides: Vec<u8>,
    pub bus_stops: Vec<(i32, i32, u8)>,
    pub depot: Option<(i32, i32, i32, i32)>,
    pub electrified: bool,
    pub subway: Option<SubwayLook>,
    pub airport: Option<AirportLook>,
    pub harbour: Option<HarbourLook>,
    /// B79: the islands' jetties.
    pub jetties: Vec<crate::jetty::JettyLook>,
}

#[derive(Default, Clone, PartialEq, Debug)]
pub struct SubwayLook {
    pub line: Vec<(i32, i32)>,
    pub entrances: Vec<(i32, i32, u8)>,
    pub grates: Vec<(i32, i32)>,
    /// 0 hoardings, 1 open cut, 2 deck, 3 entrances, 4 open.
    pub stage: u8,
}

#[derive(Default, Clone, PartialEq, Debug)]
pub struct AirportLook {
    /// 0 grass strip, 1 paved, 2 international.
    pub stage: u8,
    /// The centre line, in take-off order.
    pub runway: Vec<(i32, i32)>,
    /// How many rows of pavement (B58c: two).
    pub width: i32,
    /// Every pavement tile (all rows).
    pub paved: Vec<(i32, i32)>,
    pub taxiway: Vec<(i32, i32)>,
    pub terminal: (i32, i32, i32, i32),
    pub apron: (i32, i32, i32, i32),
    pub hangar: Option<(i32, i32, i32, i32)>,
    pub tower: Option<(i32, i32, u8)>,
    pub windsock: (i32, i32),
    pub access: Vec<(i32, i32)>,
    pub parked: Vec<(i32, i32)>,
}

#[derive(Default, Clone, PartialEq, Debug)]
pub struct HarbourLook {
    /// The straight quay, in order along the shore.
    pub quay: Vec<(i32, i32)>,
    /// Where the water lies from the quay.
    pub water_dir: (i32, i32),
    /// The piers, each a straight run out into the water.
    pub piers: Vec<Vec<(i32, i32)>>,
    pub moored: Vec<(i32, i32)>,
    /// The fish market shed [x, y, w, h]; (0,0,0,0) for none.
    pub shed: (i32, i32, i32, i32),
    /// Small cranes on the quay.
    pub cranes: Vec<(i32, i32)>,
}

impl Fixtures {
    pub fn is_empty(&self) -> bool {
        self.track.is_empty()
            && self.tram_line.is_empty()
            && self.station.is_none()
            && self.port.is_none()
            && self.transit == Transit::default()
    }
}

/// Draw the static rail/station/port fixtures on a tile, if any (called from the
/// static rebuild, before the buildings are drawn on top).
#[allow(clippy::too_many_arguments)]
pub fn draw_fixture_tile(
    fb: &mut Framebuffer,
    cam: &Camera,
    m: &TileMap,
    x: i32,
    y: i32,
    f: &Fixtures,
    glow: f32,
) {
    let t = m.at(x, y);
    let z = t.min_level() as i32 * HEIGHT_UNIT;
    let tr = &f.transit;
    // --- water: the marina pontoon, the harbour piers and its boats ---
    if t.is_water() {
        let in_marina = f.marina.is_some_and(|(px, py, pw, ph)| {
            x >= px && x < px + pw && y >= py && y < py + ph
        });
        if in_marina {
            let deck = palette::rgb(0xa8, 0x88, 0x5e);
            let post = palette::rgb(0x5a, 0x46, 0x32);
            let plank = palette::shade(palette::rgb(0xa8, 0x88, 0x5e), 1.14);
            let lift = z + 2;
            // B66: the pontoon is **one** deck, not a raft of separate tiles.
            // The old 0.12 inset left a quarter-tile of water between every
            // deck tile, so a 4x4 marina drew as a grid of brown diamonds
            // floating on the sea. The deck now fills its tile; only the
            // water-facing edges keep a seam.
            // A 3 % overlap on every side: adjacent decks share their edge with
            // no seam of water between them.
            let e = -0.03;
            let inset = [
                cam.project(x as f32 + e, y as f32 + e, lift),
                cam.project(x as f32 + 1.0 - e, y as f32 + e, lift),
                cam.project(x as f32 + 1.0 - e, y as f32 + 1.0 - e, lift),
                cam.project(x as f32 + e, y as f32 + 1.0 - e, lift),
            ];
            fb.fill_poly(&inset, deck);
            let a = cam.project(x as f32 + 0.02, y as f32 + 0.02, lift);
            let b = cam.project(x as f32 + 0.98, y as f32 + 0.02, lift);
            fb.line(a, b, post);
            // Plank courses along the deck, so it reads as timber.
            for k in 1..4 {
                let v = k as f32 / 4.0;
                let p0 = cam.project(x as f32 + 0.02, y as f32 + v, lift);
                let p1 = cam.project(x as f32 + 0.98, y as f32 + v, lift);
                fb.line(p0, p1, palette::shade(deck, 0.92));
            }
            // A gangway onto the shore: the pontoon has to touch land (the sim
            // will not site one out at sea), and this is the join you see.
            for (dx, dy) in [(1i32, 0i32), (-1, 0), (0, 1), (0, -1)] {
                let (nx, ny) = (x + dx, y + dy);
                if m.in_bounds(nx, ny) && !m.at(nx, ny).is_water() {
                    let (u0, v0) = match (dx, dy) {
                        (1, _) => (0.98, 0.30),
                        (-1, _) => (0.02, 0.70),
                        (_, 1) => (0.30, 0.98),
                        _ => (0.70, 0.02),
                    };
                    let (pu, pv) = if dx != 0 { (u0 + dx as f32 * 0.02, v0) } else { (u0, v0 + dy as f32 * 0.02) };
                    let g0 = cam.project(x as f32 + pu, y as f32 + pv, lift);
                    let g1 = cam.project(x as f32 + pu + dx as f32 * 0.10, y as f32 + pv + dy as f32 * 0.10, lift - 1);
                    fb.line(g0, g1, plank);
                    fb.line((g0.0, g0.1 + 1.0), (g1.0, g1.1 + 1.0), plank);
                }
            }
            if (x + y) % 2 == 0 {
                let (sx, sy) = cam.project(x as f32 + 0.35, y as f32 + 0.85, lift + 1);
                let hull = palette::rgb(0xf4, 0xf2, 0xea);
                let trim = palette::rgb(0x8a, 0x8e, 0x96);
                for i in 0..5 {
                    fb.put_blended(sx as i32 + i, sy as i32, hull, 1.0);
                    fb.put_blended(sx as i32 + i, sy as i32 + 1, if i == 0 || i == 4 { trim } else { hull }, 1.0);
                }
                fb.put_blended(sx as i32 + 2, sy as i32 - 1, trim, 0.8);
            }
            return;
        }
        // The harbour's piers, boats and quay are drawn as a whole in
        // `rebuild_static` (B58c), so nothing of it is per-tile here.
        return;
    }
    let c = cam.corners(x, y, t.corner.map(|v| v as f32));
    let on_track = f.track.contains(&(x, y));
    let on_twin = tr.twin.contains(&(x, y));
    let on_siding = tr.sidings.contains(&(x, y));
    let in_port = f.port.is_some_and(|(px, py, pw, ph)| {
        x >= px && x < px + pw && y >= py && y < py + ph
    });
    let in_yard = f.yard.is_some_and(|(px, py, pw, ph)| {
        x >= px && x < px + pw && y >= py && y < py + ph
    });
    if on_track || on_twin || on_siding {
        let track: &[(i32, i32)] = if on_track {
            &f.track
        } else if on_twin {
            &tr.twin
        } else {
            &tr.sidings
        };
        let cross = on_track && f.crossings.contains(&(x, y));
        draw_track(fb, cam, m, x, y, track, cross);
        if cross {
            // B83: a level crossing reads as rail in a road — the crossbuck
            // signs, not the two full-width white bars that made it look like a
            // zebra crossing (that is what the owner's phone shot showed).
            let along_x = track.contains(&(x + 1, y)) || track.contains(&(x - 1, y));
            draw_crossbucks(fb, cam, x, y, along_x, z);
        }
        // The catenary: a mast every third tile of the main line.
        if tr.electrified && on_track {
            let idx = f.track.iter().position(|p| *p == (x, y)).unwrap_or(0);
            if idx % 3 == 0 {
                let along_x = f.track.contains(&(x + 1, y)) || f.track.contains(&(x - 1, y));
                crate::transit::draw_catenary_tile(fb, cam, m, x, y, &f.track, along_x);
            }
        }
        return;
    }
    // --- the station: platforms, canopy and the booking hall ---
    if let Some(&(_, _, side)) = tr.platforms.iter().find(|p| (p.0, p.1) == (x, y)) {
        let canopy = tr.canopy.contains(&(x, y));
        crate::transit::draw_platform_tile(fb, cam, m, x, y, side, canopy, glow);
        if canopy {
            // The roof goes up after the platform's own furniture, and both are
            // on this tile, so the painter order is right (B58: the first pass
            // drew the columns and never the roof).
            crate::transit::draw_canopy_tile(fb, cam, m, x, y, side, glow);
        }
        return;
    }
    if let Some(zone) = tr.station_build {
        if x >= zone.0 && x < zone.0 + zone.2 && y >= zone.1 && y < zone.1 + zone.3 {
            // Per tile, so each tile's own ground is painted before its walls
            // (one-pass drawing let the ground of the next tile erase them).
            // The roof sits at a constant height, so a flat site gives one
            // continuous roof.
            crate::transit::draw_station_tile(fb, cam, m, x, y, zone, tr.station_kind, glow);
            return;
        }
    }
    // --- the airfield is drawn as a whole (surfaces, then its buildings) in
    // `rebuild_static`: a runway made of per-tile quads gets a green seam
    // between every tile, and a terminal made of per-tile boxes is a field of
    // slabs (B58c).
    // --- the subway works ---
    if let Some(sub) = &tr.subway {
        if let Some(i) = sub.line.iter().position(|p| *p == (x, y)) {
            let along_x = sub.line.windows(2).any(|w| w[0].1 == w[1].1);
            let entrance = sub.entrances.iter().any(|(ex, ey, _)| (*ex, *ey) == (x, y));
            let grate = sub.grates.contains(&(x, y));
            let _ = i;
            crate::transit::draw_subway_tile(fb, cam, m, x, y, along_x, sub.stage, false, grate, glow);
            return;
        }
        if let Some(&(_, _, side)) = sub.entrances.iter().find(|(ex, ey, _)| (*ex, *ey) == (x, y)) {
            let _ = side;
            crate::transit::draw_subway_tile(fb, cam, m, x, y, false, sub.stage, true, false, glow);
            return;
        }
        if sub.grates.contains(&(x, y)) {
            crate::transit::draw_subway_tile(fb, cam, m, x, y, false, sub.stage.max(3), false, true, glow);
            return;
        }
    }
    // --- the bus and tram stops: the shelter stands on the pavement tile ---
    let side_vec = |s: u8| -> (i32, i32) {
        match s {
            0 => (1, 0),
            1 => (-1, 0),
            2 => (0, 1),
            _ => (0, -1),
        }
    };
    if let Some(&(bx, by, side)) = tr
        .bus_stops
        .iter()
        .find(|&&(sx, sy, side)| (sx + side_vec(side).0, sy + side_vec(side).1) == (x, y))
    {
        let _ = (bx, by);
        crate::transit::draw_bus_stop_tile(fb, cam, m, x, y, side, true, glow);
        return;
    }
    let mut tram_shelter = None;
    for (i, &(sx, sy)) in f.tram_stops.iter().enumerate() {
        let s = tr.tram_sides.get(i).copied().unwrap_or(0);
        let (dx, dy) = side_vec(s);
        if (sx + dx, sy + dy) == (x, y) {
            tram_shelter = Some(s);
            break;
        }
    }
    if let Some(side) = tram_shelter {
        crate::transit::draw_shelter_tile(fb, cam, m, x, y, side, glow);
        return;
    }
    // --- the bus depot: a shed and a parked bus on an apron ---
    if let Some((dx, dy, dw, dh)) = tr.depot {
        if x >= dx && x < dx + dw && y >= dy && y < dy + dh {
            let gravel = palette::rgb(0x7a, 0x74, 0x6a);
            fb.fill_poly(&c, gravel);
            // The shed on the back row, the buses in front.
            if y == dy && x + 1 < dx + dw {
                crate::transit::draw_depot_shed(fb, cam, m, x, y);
            } else if y == dy + 1 && x == dx + 1 {
                crate::transit::draw_parked_bus(fb, cam, m, x, y);
            }
            return;
        }
    }
    if in_port {
        let quay = palette::rgb(0x8a, 0x86, 0x80);
        fb.fill_poly(&c, quay);
        return;
    }
    if let Some(i) = tr.parked_wagons.iter().position(|(p, _)| *p == (x, y)) {
        crate::transit::draw_parked_wagon(fb, cam, m, x, y, tr.parked_wagons[i].1);
        return;
    }
    // --- the tram rails in the avenue and a kerb at each stop ---
    let on_tram = f.tram_line.contains(&(x, y));
    if on_tram {
        let along_x = f.tram_line.contains(&(x + 1, y)) || f.tram_line.contains(&(x - 1, y));
        let steel = palette::rgb(0xb4, 0xb0, 0xa6);
        let (a, b) = if along_x {
            (
                cam.project(x as f32 + 0.0, y as f32 + 0.34, z + 1),
                cam.project(x as f32 + 1.0, y as f32 + 0.34, z + 1),
            )
        } else {
            (
                cam.project(x as f32 + 0.34, y as f32 + 0.0, z + 1),
                cam.project(x as f32 + 0.34, y as f32 + 1.0, z + 1),
            )
        };
        fb.line(a, b, steel);
        let (a2, b2) = if along_x {
            (
                cam.project(x as f32, y as f32 + 0.66, z + 1),
                cam.project(x as f32 + 1.0, y as f32 + 0.66, z + 1),
            )
        } else {
            (
                cam.project(x as f32 + 0.66, y as f32, z + 1),
                cam.project(x as f32 + 0.66, y as f32 + 1.0, z + 1),
            )
        };
        fb.line(a2, b2, steel);
        if f.tram_stops.contains(&(x, y)) {
            let kerb = palette::rgb(0xd0, 0xcc, 0xc0);
            let (ka, kb) = if along_x {
                ((0.15, 0.05), (0.85, 0.25))
            } else {
                ((0.05, 0.15), (0.25, 0.85))
            };
            let top = [
                cam.project(x as f32 + ka.0, y as f32 + ka.1, z + 2),
                cam.project(x as f32 + kb.0, y as f32 + ka.1, z + 2),
                cam.project(x as f32 + kb.0, y as f32 + kb.1, z + 2),
                cam.project(x as f32 + ka.0, y as f32 + kb.1, z + 2),
            ];
            fb.fill_poly(&top, kerb);
            let pole = if along_x {
                cam.project(x as f32 + 0.5, y as f32 + 0.15, z + 2)
            } else {
                cam.project(x as f32 + 0.15, y as f32 + 0.5, z + 2)
            };
            fb.line(pole, (pole.0, pole.1 - 12.0), palette::rgb(0x3a, 0x3e, 0x44));
            fb.put_blended(pole.0 as i32, pole.1 as i32 - 13, palette::rgb(0xe0, 0x5a, 0x3a), 1.0);
        }
    }
    if in_yard {
        let gravel = palette::rgb(0x7a, 0x74, 0x6a);
        fb.fill_poly(&c, gravel);
        return;
    }
}

/// The track itself on a tile, embedded in asphalt at a level crossing.
fn draw_track(
    fb: &mut Framebuffer,
    cam: &Camera,
    m: &TileMap,
    x: i32,
    y: i32,
    track: &[(i32, i32)],
    crossing: bool,
) {
    let z = m.at(x, y).min_level() as i32 * HEIGHT_UNIT;
    let c = cam.corners(x, y, m.at(x, y).corner.map(|v| v as f32));
    let salt = ((x as u32) << 8) ^ (y as u32);
    let along_x = track.contains(&(x + 1, y)) || track.contains(&(x - 1, y));
    let along_y = track.contains(&(x, y - 1)) || track.contains(&(x, y + 1));
    let diagonal = !(along_x ^ along_y);
    // The bed: ballast with a paler shoulder, and a sleeper rhythm every couple
    // of pixels — at 1x a real sleeper pitch (30 to the tile) is smaller than a
    // pixel, so the rhythm is what the eye reads, not the sleepers.
    if !crossing && !diagonal {
        let ballast = palette::rgb(0x7d, 0x79, 0x73);
        let shoulder = palette::rgb(0x96, 0x92, 0x8a);
        let sleeper = palette::rgb(0x5a, 0x4c, 0x40);
        let t = m.at(x, y).diagonal();
        for tri in [(0usize, 1usize, 2usize), (0, 2, 3)] {
            crate::tex::diamond_tri_px(fb, &c, tri, |px, py, u, v| {
                let (along, across) = if along_x { (u, v) } else { (v, u) };
                let a = (across - 0.5).abs();
                if a > 0.40 {
                    return palette::TRANSPARENT;
                }
                let mut col = if a > 0.33 { shoulder } else { ballast };
                if a < 0.30 && (along * 13.0).fract() < 0.44 {
                    col = sleeper;
                }
                crate::tex::grain_px(col, px, py, salt)
            });
        }
        let _ = t;
    }
    // The two rails: a dark web under a lit head, so they read as steel on the
    // sleepers and never as two painted white lines.
    let perp = if along_x && !along_y {
        (-0.8944f32, 0.4472f32)
    } else if along_y && !along_x {
        (0.8944f32, 0.4472f32)
    } else {
        (-0.7071f32, 0.7071f32)
    };
    let (a, b) = if along_x && !along_y {
        (
            cam.project(x as f32 + 0.06, y as f32 + 0.5, z),
            cam.project(x as f32 + 0.94, y as f32 + 0.5, z),
        )
    } else if along_y && !along_x {
        (
            cam.project(x as f32 + 0.5, y as f32 + 0.06, z),
            cam.project(x as f32 + 0.5, y as f32 + 0.94, z),
        )
    } else {
        (
            cam.project(x as f32 + 0.2, y as f32 + 0.2, z),
            cam.project(x as f32 + 0.8, y as f32 + 0.8, z),
        )
    };
    for off in [2.6f32, -2.6f32] {
        let a0 = (a.0 + perp.0 * off, a.1 + perp.1 * off);
        let a1 = (b.0 + perp.0 * off, b.1 + perp.1 * off);
        // A crossing's rails are flush: a thinner, darker web (the road is over
        // the sleepers there).
        fb.line((a0.0, a0.1 + 1.0), (a1.0, a1.1 + 1.0), palette::rgb(0x3a, 0x38, 0x36));
        fb.line(a0, a1, if crossing { palette::rgb(0x9a, 0x96, 0x8c) } else { palette::rgb(0xb4, 0xb0, 0xa6) });
    }
}

/// The crossbuck signs at a level crossing: a white X on a post, on the kerb
/// side of the road at each approach. Two of them is what makes a crossing read
/// as a railway in a street.
fn draw_crossbucks(fb: &mut Framebuffer, cam: &Camera, x: i32, y: i32, along_x: bool, z: i32) {
    // Beside the *road*, which runs across the track: the signs stand where a
    // driver on the approach can see them.
    for side in [1i32, -1] {
        // On the near kerb, a little before the crossing: a sign in the middle
        // of the carriageway is a sign a driver would hit.
        let (fx, fy) = if along_x {
            (x as f32 + 0.5, y as f32 + 0.38 + side as f32 * 0.30)
        } else {
            (x as f32 + 0.38 + side as f32 * 0.30, y as f32 + 0.5)
        };
        let base = cam.project(fx, fy, z);
        let top = (base.0, base.1 - 11.0);
        fb.line(base, top, palette::rgb(0x6a, 0x6a, 0x66));
        let (cx, cy) = (top.0 as i32, top.1 as i32 - 3);
        fb.line((cx as f32 - 4.0, cy as f32 - 2.0), (cx as f32 + 4.0, cy as f32 + 2.0), palette::rgb(0xf2, 0xf0, 0xe8));
        fb.line((cx as f32 - 4.0, cy as f32 + 2.0), (cx as f32 + 4.0, cy as f32 - 2.0), palette::rgb(0xf2, 0xf0, 0xe8));
    }
}

/// Gantry cranes along the port quay (B31b): a mast, a boom over the water and a
/// trolley, one every third quay tile that touches water.
pub fn draw_cranes(fb: &mut Framebuffer, cam: &Camera, m: &TileMap, port: (i32, i32, i32, i32)) {
    let (px, py, pw, ph) = port;
    let mut quay = 0;
    for ty in py..py + ph {
        for tx in px..px + pw {
            let water = [(1, 0), (-1, 0), (0, 1), (0, -1)]
                .iter()
                .find(|(dx, dy)| m.at(tx + dx, ty + dy).is_water())
                .copied();
            let Some(dir) = water else { continue };
            quay += 1;
            if quay % 3 != 1 {
                continue;
            }
            let z = m.at(tx, ty).min_level() as i32 * HEIGHT_UNIT;
            let base = cam.project(tx as f32 + 0.5, ty as f32 + 0.5, z + 2);
            let top = (base.0, base.1 - 20.0);
            let steel = palette::rgb(0xe8, 0x9a, 0x22);
            let dark = palette::rgb(0x3a, 0x38, 0x34);
            let pale = palette::rgb(0xf2, 0xc8, 0x58);
            // Two legs and the top beam.
            fb.line((base.0 - 5.0, base.1), (top.0 - 2.0, top.1), dark);
            fb.line((base.0 + 5.0, base.1), (top.0 + 2.0, top.1), dark);
            fb.line((top.0 - 3.0, top.1), (top.0 + 3.0, top.1), steel);
            // The boom reaches out over the water, with a slight sag.
            let reach = (dir.0 as f32 * 10.0, dir.1 as f32 * 5.0);
            let tip = (top.0 + reach.0 + 3.0, top.1 + reach.1 - 2.0);
            fb.line((top.0 - 2.0, top.1), tip, pale);
            fb.line((top.0 + 2.0, top.1), tip, steel);
            // B58: the trolley and its container are drawn per frame from the
            // sim's crane cycle (a mover), so the mast here is only the mast.
            // A cab on the quay side.
            fb.put_blended((base.0 - 1.0) as i32, (base.1 - 4.0) as i32, pale, 1.0);
        }
    }
}

/// Containers: small coloured boxes standing on the quay.
pub fn draw_container(fb: &mut Framebuffer, cam: &Camera, m: &TileMap, x: i32, y: i32, pick: u8) {
    let t = m.at(x, y);
    let z = t.min_level() as i32 * HEIGHT_UNIT;
    let colours = [
        palette::rgb(0xc0, 0x4a, 0x3a),
        palette::rgb(0x3a, 0x6a, 0xa8),
        palette::rgb(0x3a, 0x8a, 0x5a),
        palette::rgb(0xc8, 0x9a, 0x32),
        palette::rgb(0x8a, 0x8a, 0x92),
    ];
    let c = colours[(pick as usize) % colours.len()];
    let (x0, y0) = (x as f32 + 0.15, y as f32 + 0.15);
    let (x1, y1) = (x as f32 + 0.85, y as f32 + 0.85);
    let h = 4;
    let top = [
        cam.project(x0, y0, z + h),
        cam.project(x1, y0, z + h),
        cam.project(x1, y1, z + h),
        cam.project(x0, y1, z + h),
    ];
    fb.fill_poly(&top, palette::shade(c, 1.12));
    let left = [
        cam.project(x0, y1, z + h),
        cam.project(x1, y1, z + h),
        cam.project(x1, y1, z),
        cam.project(x0, y1, z),
    ];
    fb.fill_poly(&left, palette::shade(c, 0.86));
    let right = [
        cam.project(x1, y0, z + h),
        cam.project(x1, y1, z + h),
        cam.project(x1, y1, z),
        cam.project(x1, y0, z),
    ];
    fb.fill_poly(&right, palette::shade(c, 0.68));
}

/// Movers, with their sprites.
pub struct MoverRenderer {
    /// The locomotive of each set: 0 diesel passenger, 1 electric passenger,
    /// 2 freight.
    train: Vec<[Sprite; 4]>,
    /// The passenger coach behind the locomotive.
    coach: [Sprite; 4],
    /// The four wagon kinds a freight train is made of (B58: mixed wagons).
    wagon: Vec<[Sprite; 4]>,
    tram: [Sprite; 4],
    plane: Sprite,
    /// Two airliners: the grass-strip size and the regional one.
    aircraft: [Sprite; 2],
    bus: [Sprite; 4],
}

impl MoverRenderer {
    /// B61f: the bytes the mover sprites hold (for the memory report).
    pub fn bytes(&self) -> usize {
        let set = |v: &Vec<[Sprite; 4]>| v.iter().flatten().map(|s| s.bytes()).sum::<usize>();
        let q = |v: &[Sprite; 4]| v.iter().map(|s| s.bytes()).sum::<usize>();
        set(&self.train)
            + q(&self.coach)
            + set(&self.wagon)
            + q(&self.tram)
            + self.plane.bytes()
            + self.aircraft.iter().map(|a| a.bytes()).sum::<usize>()
            + q(&self.bus)
            + crate::boats::bytes()
    }

    pub fn new() -> Self {
        Self {
            train: (0..3).map(|set| std::array::from_fn(|d| train_sprite(set, d))).collect(),
            coach: std::array::from_fn(|d| coach_sprite(d)),
            wagon: (0..4).map(|k| std::array::from_fn(|d| wagon_sprite(k, d))).collect(),
            tram: std::array::from_fn(|d| vehicle_sprite(Kind::Tram, d)),
            plane: plane_sprite(),
            aircraft: [aircraft_sprite(false), aircraft_sprite(true)],
            bus: std::array::from_fn(|d| bus_sprite(d)),
        }
    }

    /// Draw the movers. `only_planes` splits the pass in two: the sky overflight
    /// belongs in the sky band, everything on the ground is clipped to the land
    /// (B58d: a train at the back of the map was painting over the sky).
    pub fn draw(
        &self,
        fb: &mut Framebuffer,
        cam: &Camera,
        map: &TileMap,
        bodies: &[Body],
        bands: &crate::bands::Bands,
        depth: &[u16],
        only_planes: bool,
        haze: Option<&[f32]>,
        under: &[Color],
    ) {
        let fb_w = fb.w;
        // The static layer's colour under a screen point (what a distant boat
        // fades towards): the same in a damaged part and in the whole frame.
        let under_at = |sx: i32, sy: i32| -> Color {
            if sx < 0 || sy < 0 || sx as usize >= fb_w {
                return 0;
            }
            under.get(sy as usize * fb_w + sx as usize).copied().unwrap_or(0)
        };
        // B61e: one mover, by list index; the band index below keeps a damaged
        // rectangle on its own rows.
        let clip = fb.clip();
        let mut one = |bi: usize| {
            let b = &bodies[bi];
            if (b.kind == Kind::Plane) != only_planes {
                return;
            }
            match b.kind {
                Kind::Plane => {
                    // In the sky band: no depth test, it is in front of the hills.
                    let z = b.lift;
                    let (sx, sy) = cam.project(b.x, b.y, z);
                    let px = sx as i32 - self.plane.w as i32 / 2;
                    let py = sy as i32 - self.plane.h as i32 / 2;
                    if b.night {
                        let on = ((b.age * 2.0) as i32) % 2 == 0;
                        let c = if on { pal::EMERGENCY_RED } else { pal::HEADLIGHT };
                        fb.put_blended(px, py + 1, c, 1.0);
                    } else {
                        for i in 1..26 {
                            fb.put_blended(px - i, py + 1, palette::rgb(0xe8, 0xec, 0xf2), 0.35 - i as f32 * 0.01);
                        }
                    }
                    fb.blit_sprite(&self.plane, px, py);
                }
                Kind::Aircraft => {
                    // The airliner: on the ground it sits on the runway, in the
                    // air its shadow falls on the runway below it.
                    let (sx, sy) = cam.project(b.x, b.y, b.lift);
                    let large = b.set > 0;
                    let s = &self.aircraft[if large { 1 } else { 0 }];
                    let key = crate::actors::actor_key(b.x.floor() as i32, b.y.floor() as i32);
                    if b.lift > 3 {
                        let (gx, gy) = cam.project(b.x, b.y, 0);
                        let r = 6 + (b.lift / 18);
                        for k in 0..r {
                            let w = r - k;
                            fb.put_blended(gx as i32 - w, gy as i32 + k, palette::rgb(0x20, 0x22, 0x28), 0.16);
                            fb.put_blended(gx as i32 + w, gy as i32 + k, palette::rgb(0x20, 0x22, 0x28), 0.16);
                        }
                    }
                    fb.blit_sprite_depth(s, sx as i32 - s.ax, sy as i32 - s.ay, key, depth);
                    if b.night {
                        let c = if ((b.age * 2.0) as i32) % 2 == 0 { palette::rgb(0xff, 0x40, 0x30) } else { palette::rgb(0x40, 0xff, 0x60) };
                        fb.put_blended((sx + 8.0) as i32, sy as i32, c, 1.0);
                    }
                }
                Kind::Boat | Kind::Ship | Kind::Ferry => {
                    // B91: the boat kit. The set byte carries the kind (low five
                    // bits) and the variant (top three).
                    let class = match b.kind {
                        Kind::Boat => crate::boats::Class::Boat,
                        Kind::Ship => crate::boats::Class::Ship,
                        _ => crate::boats::Class::Ferry,
                    };
                    let (bk, var) = crate::boats::BoatKind::decode(class, b.set);
                    let z = map.ground_px(b.x, b.y);
                    let (sx, sy) = cam.project(b.x, b.y, z);
                    let key = crate::actors::actor_key(b.x.floor() as i32, b.y.floor() as i32);
                    // A tug's tow: the trailing unit is a barge of the same set.
                    for i in (0..b.cars as usize).rev() {
                        let (cx, cy) = b.trail.get(i).copied().unwrap_or((b.x, b.y));
                        let z2 = map.ground_px(cx, cy);
                        let (px, py) = cam.project(cx, cy, z2);
                        let k = crate::actors::actor_key(cx.floor() as i32, cy.floor() as i32);
                        crate::boats::draw(
                            fb,
                            &crate::boats::Pose {
                                kind: crate::boats::BoatKind::Barge,
                                variant: var,
                                dir: b.dir,
                                sx: px as i32,
                                sy: py as i32 + crate::boats::bob(b.age, cx + cy, crate::boats::BoatKind::Barge),
                                night: b.night,
                                age: b.age,
                                moving: b.lift == 0,
                                depth: Some((k, depth)),
                                haze,
                                haze_to: under_at(px as i32, py as i32),
                            },
                        );
                    }
                    crate::boats::draw(
                        fb,
                        &crate::boats::Pose {
                            kind: bk,
                            variant: var,
                            dir: b.dir,
                            sx: sx as i32,
                            sy: sy as i32 + crate::boats::bob(b.age, b.x + b.y, bk),
                            night: b.night,
                            age: b.age,
                            moving: b.lift == 0,
                            depth: Some((key, depth)),
                            haze,
                            haze_to: under_at(sx as i32, sy as i32),
                        },
                    );
                }
                Kind::Train | Kind::Tram | Kind::Bus => {
                    let d = dir_index(b.dir);
                    let head: &Sprite = match b.kind {
                        Kind::Train => &self.train[(b.set as usize).min(2)][d],
                        Kind::Tram => &self.tram[d],
                        Kind::Bus => &self.bus[d],
                        _ => &self.tram[d],
                    };
                    let z = map.ground_px(b.x, b.y);
                    let (sx, sy) = cam.project(b.x, b.y, z);
                    let key = crate::actors::actor_key(b.x.floor() as i32, b.y.floor() as i32);
                    // The units behind the head: coaches on a passenger set,
                    // mixed wagons on a freight, a second car on a tram.
                    for i in (0..b.cars as usize).rev() {
                        let (cx, cy) = b.trail.get(i).copied().unwrap_or((b.x, b.y));
                        let z2 = map.ground_px(cx, cy);
                        let (px, py) = cam.project(cx, cy, z2);
                        let k = crate::actors::actor_key(cx.floor() as i32, cy.floor() as i32);
                        let unit: &Sprite = match b.kind {
                            Kind::Train => {
                                if b.set == 2 {
                                    &self.wagon[(i + b.x as usize) % 4][d]
                                } else {
                                    &self.coach[d]
                                }
                            }
                            Kind::Tram => &self.tram[d],
                            _ => head,
                        };
                        fb.blit_sprite_depth(unit, px as i32 - unit.ax, py as i32 - unit.ay, k, depth);
                    }
                    fb.blit_sprite_depth(head, sx as i32 - head.ax, sy as i32 - head.ay, key, depth);
                    if b.night {
                        // Vehicle lights along the way (trains, buses, trams).
                        let fx = sx + b.dir.0 * 6.0;
                        let fy = sy + b.dir.1 * 3.0;
                        fb.put_blended(fx as i32, fy as i32, pal::HEADLIGHT, 0.9);
                        if b.kind == Kind::Train {
                            // The carriage windows glow at night.
                            for i in 0..b.cars as usize {
                                if let Some(&(cx, cy)) = b.trail.get(i) {
                                    let z2 = map.ground_px(cx, cy);
                                    let (px, py) = cam.project(cx, cy, z2);
                                    fb.put_blended(px as i32, py as i32 - 4, palette::rgb(0xff, 0xe0, 0x90), 0.75);
                                }
                            }
                        }
                    }
                }
                Kind::Crossing => {
                    crate::transit::draw_crossing(fb, cam, map, b.x.floor() as i32, b.y.floor() as i32, b.dir, b.phase, b.age);
                }
                Kind::Crane => {
                    crate::transit::draw_crane_trolley(fb, cam, map, b.x.floor() as i32, b.y.floor() as i32, b.dir, b.phase);
                }
                Kind::Crew => {
                    crate::transit::draw_crew(fb, cam, map, b.x, b.y, ((b.age * 3.0) as i32) % 2 == 0);
                }
                Kind::Shore => {
                    crate::shore::draw(fb, cam, map, b.x, b.y, b.dir, b.set, b.lift, b.phase, b.night, depth, haze, under);
                }
            }
        };
        if bands.is_empty() {
            for i in 0..bodies.len() {
                one(i);
            }
        } else {
            // A train's carriages trail behind the head, and a plane is in the
            // sky: the pad covers a set's length above and below.
            bands.for_each(clip, 80, |i| one(i as usize));
        }
    }
}

impl Default for MoverRenderer {
    fn default() -> Self {
        Self::new()
    }
}

pub fn dir_index(dir: (f32, f32)) -> usize {
    crate::traffic::dir_index(dir)
}

fn vehicle_sprite(kind: Kind, dir: usize) -> Sprite {
    // B32: every mover is an isometric box anchored on its ground point, the
    // same primitive the traffic uses, so a train car sits centred on the rails
    // (H31b note: cars were drawn beside/across the track).
    let (len, wid, hgt) = match kind {
        Kind::Train => (36.0, 10.0, 12.0),
        Kind::Tram => (32.0, 9.0, 11.0),
        Kind::Bus => (30.0, 9.5, 11.0),
        Kind::Boat => (18.0, 7.0, 4.0),
        Kind::Ferry => (26.0, 10.0, 6.0),
        Kind::Ship => (46.0, 16.0, 10.0),
        _ => (20.0, 4.0, 4.0),
    };
    let (body, roof): (Color, Color) = match kind {
        Kind::Train => (palette::rgb(0x6a, 0x2a, 0x2a), palette::rgb(0x9a, 0x44, 0x3a)),
        Kind::Tram => (palette::rgb(0xc8, 0x9a, 0x38), palette::rgb(0xe8, 0xc4, 0x60)),
        Kind::Bus => (palette::rgb(0xc8, 0x8a, 0x2a), palette::rgb(0xe8, 0xb0, 0x50)),
        Kind::Boat => (palette::rgb(0xe0, 0xdc, 0xd0), palette::rgb(0xf6, 0xf4, 0xec)),
        Kind::Ferry => (palette::rgb(0xe8, 0xe4, 0xd8), palette::rgb(0x2e, 0x5a, 0x8a)),
        Kind::Ship => (palette::rgb(0x3a, 0x44, 0x52), palette::rgb(0x62, 0x6e, 0x7c)),
        _ => (palette::rgb(0xdc, 0xde, 0xe2), palette::rgb(0xf0, 0xf2, 0xf6)),
    };
    let spec = crate::traffic::BoxSpec {
        len,
        wid,
        hgt,
        body,
        roof,
        glass: pal::CAR_GLASS,
        windows: matches!(kind, Kind::Train | Kind::Tram | Kind::Bus),
        wheels: false,
        iso: false,
        cabin: false,
    };
    crate::traffic::iso_box_sprite(dir, &spec)
}

fn sput(s: &mut Sprite, x: i32, y: i32, c: Color) {
    if x >= 0 && y >= 0 {
        s.set(x as usize, y as usize, c);
    }
}

/// The locomotive of a set (B58): a diesel in green/cream, an electric in
/// blue/grey with a pantograph, or a heavy freight engine in red/black.
fn train_sprite(set: usize, dir: usize) -> Sprite {
    let (body, roof) = match set {
        0 => (palette::rgb(0x2e, 0x5a, 0x3a), palette::rgb(0xd8, 0xd2, 0xb0)),
        1 => (palette::rgb(0x28, 0x48, 0x7a), palette::rgb(0xb8, 0xc4, 0xd4)),
        _ => (palette::rgb(0x7a, 0x24, 0x22), palette::rgb(0x3a, 0x38, 0x34)),
    };
    let spec = crate::traffic::BoxSpec {
        len: 40.0,
        wid: 10.0,
        hgt: 13.0,
        body,
        roof,
        glass: palette::rgb(0x22, 0x2c, 0x38),
        windows: true,
        wheels: false,
        iso: false,
        cabin: false,
    };
    let mut s = crate::traffic::iso_box_sprite(dir, &spec);
    if set == 1 {
        // The pantograph: a small diamond of steel above the roof, which is
        // what makes an electric set read as electric at 1×.
        let ax = s.ax;
        let ay = s.ay - 13;
        let steel = palette::rgb(0x9a, 0x9e, 0xa6);
        for k in 0..4 {
            sput(&mut s, ax as i32 + k, ay - k, steel);
            sput(&mut s, ax as i32 - k, ay - k, steel);
        }
    }
    s
}

/// A passenger coach: the same body as the locomotive without the nose.
fn coach_sprite(dir: usize) -> Sprite {
    let spec = crate::traffic::BoxSpec {
        len: 34.0,
        wid: 10.0,
        hgt: 12.0,
        body: palette::rgb(0x8a, 0x3a, 0x32),
        roof: palette::rgb(0x6a, 0x6e, 0x76),
        glass: palette::rgb(0x22, 0x2c, 0x38),
        windows: true,
        wheels: false,
        iso: false,
        cabin: false,
    };
    crate::traffic::iso_box_sprite(dir, &spec)
}

/// One of the four freight wagons: a loaded flat, a tank, a box van, a hopper.
fn wagon_sprite(kind: usize, dir: usize) -> Sprite {
    let (body, roof) = match kind % 4 {
        0 => (palette::rgb(0x5a, 0x5e, 0x62), palette::rgb(0xc0, 0x6a, 0x3a)),
        1 => (palette::rgb(0x8a, 0x8e, 0x92), palette::rgb(0xa8, 0xac, 0xb0)),
        2 => (palette::rgb(0x7a, 0x3a, 0x2e), palette::rgb(0x8a, 0x86, 0x80)),
        _ => (palette::rgb(0x5a, 0x6a, 0x3a), palette::rgb(0x3a, 0x3e, 0x34)),
    };
    let spec = crate::traffic::BoxSpec {
        len: 30.0,
        wid: 9.5,
        hgt: if kind % 4 == 1 { 11.0 } else { 9.0 },
        body,
        roof,
        glass: palette::rgb(0x22, 0x2c, 0x38),
        windows: false,
        wheels: false,
        iso: false,
        cabin: false,
    };
    crate::traffic::iso_box_sprite(dir, &spec)
}

/// A city bus: yellow with a window strip and a route board (B58).
fn bus_sprite(dir: usize) -> Sprite {
    let mut s = vehicle_sprite(Kind::Bus, dir);
    let body = palette::rgb(0xc8, 0x8a, 0x2a);
    // A destination board over the windscreen.
    let (ax, ay) = (s.ax, s.ay);
    for k in 0..5 {
        sput(&mut s, ax as i32 + k + 3, ay - 10, palette::shade(body, 0.5));
    }
    s
}

/// The airliner that uses the airport (B58): a side-on airliner, larger than the
/// band's plane so it reads on a 64-px runway.
fn aircraft_sprite(large: bool) -> Sprite {
    let (w, h) = if large { (29usize, 15usize) } else { (21, 12) };
    let mut s = Sprite::new(w, h);
    let body = palette::rgb(0xec, 0xee, 0xf2);
    let shade = palette::rgb(0xc0, 0xc6, 0xd0);
    let trim = palette::rgb(0x2e, 0x5a, 0x9a);
    let wing = palette::rgb(0xb0, 0xb6, 0xc0);
    let cy = h / 2;
    for x in 2..w - 1 {
        s.set(x, cy, body);
        s.set(x, cy + 1, shade);
    }
    // Wings swept back from the middle.
    let mid = w / 2;
    for i in 0..(w / 4) {
        s.set(mid - i, cy - 1 - i / 2, wing);
        s.set(mid - i, cy + 2 + i / 2, wing);
        s.set(mid + i, cy - 1 - i / 2, wing);
        s.set(mid + i, cy + 2 + i / 2, wing);
    }
    // A blue cheat line and the cabin windows.
    for x in 4..w - 3 {
        s.set(x, cy + 1, trim);
        if x % 3 == 0 {
            s.set(x, cy, palette::rgb(0x28, 0x34, 0x44));
        }
    }
    // Tail fin and engines.
    for k in 0..4 {
        s.set(2, cy - 1 - k, trim);
        s.set(3, cy - 1 - k, trim);
    }
    s.set(mid - 3, cy + 2, shade);
    s.set(mid - 3, cy - 2, shade);
    s.set(mid + 3, cy + 2, shade);
    s.set(mid + 3, cy - 2, shade);
    s.set_anchor((w / 2) as i32, cy as i32 + 2);
    s
}

fn plane_sprite() -> Sprite {
    // An airliner at 2x the old size (B31b: at 1x a 9x5 airliner was a blink):
    // fuselage with a lit cabin strip, swept wings, tail fin and engine pods.
    let mut s = Sprite::new(19, 11);
    let body = palette::rgb(0xe8, 0xea, 0xee);
    let shade = palette::rgb(0xc0, 0xc4, 0xcc);
    let wing = palette::rgb(0xa8, 0xac, 0xb6);
    for x in 0..19 {
        s.set(x, 5, body);
        s.set(x, 6, shade);
    }
    for x in 6..13 {
        s.set(x, 4, body);
        s.set(x, 3, wing);
        s.set(x, 7, wing);
        s.set(x, 2, wing);
        s.set(x, 8, wing);
    }
    for x in 8..11 {
        s.set(x, 1, wing);
        s.set(x, 9, wing);
    }
    // Tail fin.
    s.set(1, 4, wing);
    s.set(1, 3, wing);
    s.set(2, 3, wing);
    // Nose.
    s.set(18, 5, palette::rgb(0x9a, 0x9e, 0xa6));
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_mover_has_a_sprite() {
        let r = MoverRenderer::new();
        for nest in r.train.iter().chain(r.wagon.iter()).chain(std::iter::once(&r.coach)) {
            for s in nest.iter() {
                let set = s.px.iter().filter(|c| **c != 0).count();
                assert!(set > 8, "sprite has only {set} pixels");
            }
        }
        for s in r.tram.iter().chain(r.bus.iter()) {
            let set = s.px.iter().filter(|c| **c != 0).count();
            assert!(set > 8, "sprite has only {set} pixels");
        }
        for a in &r.aircraft {
            assert!(a.px.iter().filter(|c| **c != 0).count() > 20);
        }
        for w in r.wagon.iter() {
            for s in w.iter() {
                assert!(s.px.iter().filter(|c| **c != 0).count() > 8);
            }
        }
        assert!(r.plane.px.iter().filter(|c| **c != 0).count() > 6);
    }
}
