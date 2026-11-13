//! The sim's transport → the renderer's fixtures and bodies (B58).
//!
//! One conversion, used by the wallpaper, the spike and the shot tools, so the
//! trains the wallpaper draws and the trains a handback shows can never be two
//! different cities. `sim` knows nothing about pixels and `render` knows nothing
//! about the sim; this crate is the seam.

use render::rail::{
    AirportLook, Body, Fixtures, HarbourLook, Kind, SubwayLook, Transit,
};
use sim::rail::{MoverKind, Zone};

fn zone(z: Option<Zone>) -> Option<(i32, i32, i32, i32)> {
    z.map(|z| (z.x, z.y, z.w, z.h))
}

/// The static transport the sim has planned, in the renderer's form.
pub fn fixtures(s: &sim::Sim) -> Fixtures {
    let tr = &s.rail;
    let subway = tr.subway.as_ref().zip(s.subway_works()).map(|(w, (stage, _))| SubwayLook {
        line: w.line.clone(),
        entrances: w.stations.iter().map(|st| st.entrance).collect(),
        grates: w.grates.clone(),
        stage,
    });
    let airport = tr.airport.as_ref().map(|ap| AirportLook {
        stage: ap.stage.index(),
        runway: ap.runway.clone(),
        width: ap.width.max(1),
        paved: ap.runway_tiles(),
        taxiway: ap.taxiway.clone(),
        terminal: (ap.terminal.x, ap.terminal.y, ap.terminal.w, ap.terminal.h),
        apron: (ap.apron.x, ap.apron.y, ap.apron.w, ap.apron.h),
        hangar: ap.hangar.map(|h| (h.x, h.y, h.w, h.h)),
        tower: ap.tower,
        windsock: ap.windsock,
        access: ap.access.clone(),
        parked: ap.parked.clone(),
    });
    let harbour = tr.harbour.as_ref().map(|h| HarbourLook {
        quay: h.quay.clone(),
        water_dir: h.water_dir,
        piers: h.piers.clone(),
        moored: h.moored.clone(),
        shed: h.shed,
        cranes: h.cranes.clone(),
    });
    let (platforms, canopy, station_build, station_kind) = match &tr.station_build {
        Some(b) => (
            b.platforms.clone(),
            b.canopy.clone(),
            Some((b.building.x, b.building.y, b.building.w, b.building.h)),
            b.kind.index(),
        ),
        None => (Vec::new(), Vec::new(), None, 0),
    };
    Fixtures {
        track: tr.line.clone(),
        crossings: s.level_crossings(),
        station: tr.station,
        station_platform: tr.station_platform,
        tram_line: tr.tram_line.clone(),
        tram_stops: tr.tram_stops.clone(),
        yard: zone(tr.yard),
        marina: zone(tr.marina),
        port: zone(tr.port),
        containers: tr.containers.clone(),
        transit: Transit {
            twin: tr.twin.clone(),
            platforms,
            canopy,
            station_build,
            station_kind,
            sidings: tr.sidings.clone(),
            parked_wagons: tr.parked_wagons.clone(),
            tram_sides: tr.tram_stop_sides.clone(),
            bus_stops: s.bus_stops(),
            depot: zone(tr.depot),
            electrified: tr.electrified,
            subway,
            airport,
            harbour,
            jetties: s
                .rail
                .isles
                .jetties
                .iter()
                .map(|j| render::jetty::JettyLook {
                    deck: j.deck[..j.visible_len()].to_vec(),
                    dir: j.dir,
                    level: j.level,
                    shore: j.shore,
                })
                .collect(),
        },
    }
}

/// The movers to draw this frame: trains, trams, buses, boats, ferries, ships,
/// aircraft, planes, the cranes' trolleys — plus the level-crossing barriers
/// and the subway's crews, which are movers too so they animate on a dirty rect
/// like everything else (D71).
pub fn bodies(s: &sim::Sim, night: bool) -> Vec<Body> {
    let mut out: Vec<Body> = Vec::with_capacity(s.transport().len() + 8);
    for m in s.transport() {
        let ((mut x, mut y), mut dir) = m.boat_pose();
        if let Some(h) = s.rest_heading(m) {
            dir = h;
            let (sx, sy) = s.rest_shift(m);
            x += sx;
            y += sy;
        }
        // Trains and buses are longer than the four-car trail the sprite layer
        // used to carry: eight units follow the head (B58).
        let mut trail = [(x, y); 8];
        for (i, slot) in trail.iter_mut().enumerate() {
            *slot = match m.kind {
                MoverKind::Ferry | MoverKind::Bus => m.car_pose((i as f32 + 1.0) * 0.9),
                MoverKind::Boat => m.car_pose((i as f32 + 1.0) * 1.9),
                _ => m.car_pose((i as f32 + 1.0) * 0.62),
            };
        }
        out.push(Body {
            x,
            y,
            dir,
            kind: match m.kind {
                MoverKind::Train => Kind::Train,
                MoverKind::Tram => Kind::Tram,
                MoverKind::Bus => Kind::Bus,
                MoverKind::Boat => Kind::Boat,
                MoverKind::Ferry => Kind::Ferry,
                MoverKind::Ship => Kind::Ship,
                MoverKind::Plane => Kind::Plane,
                MoverKind::Aircraft => Kind::Aircraft,
                MoverKind::Crane => Kind::Crane,
            },
            cars: m.cars,
            trail,
            lift: m.lift,
            night,
            age: (s.time % 1000.0) as f32,
            set: m.set,
            phase: m.phase,
        });
    }
    // B79: boats lying at the jetties, and the life on the shore.
    let mut moored = Vec::new();
    s.moored_boats(&mut moored);
    for m in moored {
        out.push(Body {
            x: m.x,
            y: m.y,
            dir: m.dir,
            kind: Kind::Boat,
            cars: 0,
            trail: [(m.x, m.y); 8],
            lift: 1,
            night,
            age: (s.time % 1000.0) as f32,
            set: m.set,
            phase: 0.0,
        });
    }
    let mut shore = Vec::new();
    s.shore_items(&mut shore);
    for it in shore {
        out.push(Body {
            x: it.x,
            y: it.y,
            dir: (it.dir.0 as f32, it.dir.1 as f32),
            kind: Kind::Shore,
            cars: 0,
            trail: [(it.x, it.y); 8],
            lift: it.lift,
            night,
            age: 0.0,
            set: it.kind as u8 | (it.var << 4),
            phase: it.phase,
        });
    }
    // The barriers: a body only while a crossing is closing, shut or opening, so
    // an open crossing costs nothing at all.
    for ((cx, cy), phase) in s.crossings_live() {
        if phase < 0.02 {
            continue;
        }
        out.push(Body {
            x: cx as f32 + 0.5,
            y: cy as f32 + 0.5,
            dir: (1.0, 0.0),
            kind: Kind::Crossing,
            cars: 0,
            trail: [(cx as f32, cy as f32); 8],
            lift: 0,
            night,
            age: (s.time % 100.0) as f32,
            set: 0,
            phase,
        });
    }
    // The subway's crews: they walk the cut while it is open (B58).
    if let (Some(w), Some((stage, _))) = (&s.rail.subway, s.subway_works()) {
        if (1..=2).contains(&stage) && w.line.len() > 4 {
            for k in 0..3 {
                let span = w.line.len() as f32;
                let t = ((s.time * 0.02 + k as f64 * 0.31) as f32).rem_euclid(1.0);
                let i = (t * span) as usize;
                let (x, y) = w.line[i.min(w.line.len() - 1)];
                let (nx, ny) = w.line[(i + 1).min(w.line.len() - 1)];
                out.push(Body {
                    x: x as f32 + 0.5,
                    y: y as f32 + 0.35,
                    dir: ((nx - x).signum() as f32, (ny - y).signum() as f32),
                    kind: Kind::Crew,
                    cars: 0,
                    trail: [(x as f32, y as f32); 8],
                    lift: 0,
                    night,
                    age: (s.time % 10.0) as f32,
                    set: 0,
                    phase: 0.0,
                });
            }
        }
    }
    out
}
