//! Versioned binary save.
//!
//! Hand-rolled rather than serde+bincode: the state is a handful of flat arrays
//! of `u8`/`i32`/`f64`, a format is ~150 lines, it adds no dependencies (and so
//! no build time), and it is byte-exact by construction, which is what the
//! save/load round-trip test wants. Layout is little-endian, versioned, and
//! refuses to load a file from a different version instead of guessing.
//!
//! Everything the city is made of goes in: the tile overrides the sim applied to
//! the generated terrain (graded lanes, pads, pavements, fields), the lots with
//! their construction stage, the fields, and the clock. Everything else is
//! regenerated from `seed`.

use std::io::{self, Read, Write};
use std::path::Path;

use map::{Kind, Tile, TileMap, generate};

use crate::{Field, Lot, LotKind, Pace, Sim, Stage, people};

const MAGIC: &[u8; 4] = b"CLIS";
/// v1 is the original; v2 adds the wear map (D45), and a v1 save therefore loads
/// with fresh (unworn) ground — what an old city looks like the moment the
/// villagers start walking. v3 adds each building's `storeys` and `style`
/// (B26b item 7): a reloaded city keeps its skyline and its district colours
/// instead of dropping back to the kind's base height, which is the owner's live
/// save, so v2 must keep loading. v7 adds the residents (B43/D65); an older
/// save gets its residents generated on load from the city it already has.
/// v8 adds the world's biome (D66): a v7 save was a temperate world, and loads
/// as one, because the terrain is regenerated from the seed and a seed that now
/// rolls a desert would otherwise move the ground under a saved city.
/// v9 adds the city's name (D68) and the story feed (B48a/D65): an older save
/// gets its name from `(seed, biome)` on load, the same way the founders
/// would have named it, and an empty feed. v10 (B48) adds the lives behind the
/// roster and the scenes still to tell. v11 (B48) adds which feed lines are
/// milestones. v13 adds the technology path and what its crews have installed
/// (B49/D72): an older save gets the path its seed gives it and nothing
/// installed, like a town that has not reached its era yet.
/// v12 adds the world's ending (B50/D65): its kind, length, next land and how
/// far the edits have got. An older save gets a plan from its seed on load.
/// v14 adds the economy, the government, the pollution and the incidents
/// (B52): one block, `write_civics` / `read_civics`. An older save starts with
/// a calm economy and no mayor, and the first election is scheduled afresh.
/// v15 (B58) adds the transport that grows with the city: the second track, the
/// station building, the yard's sidings, the bus lines, the subway works, the
/// airfield and the harbour. An older save replans them from its map on load.
/// v16 (B51) adds the chronicle: the world's typed history entries (the
/// written history is generated from them). An older save backfills it from
/// its kept feed lines on load.
/// v17 adds the town grid's planner state (alignment and the opened, skipped
/// and reclaimed cells). Without it a reloaded town forgot its grid and its
/// street-grid growth stalled. An older save starts the grid afresh.
/// v18 (B88) adds the rest of the growth state a reload used to reset: the
/// planning step counter (it seeds every decision's rng) and the clock phase,
/// the style districts, the planner's current block plan, the planning
/// counters, the street-upgrade queues, the cached land values and walking
/// routes, and the street furniture and the crews still at work. Without them a
/// reloaded city drew different districts and plans than one that never stopped.
/// v19 (B80) is the countryside: farmland tiles carry what they grow in the top
/// three bits of the tile's `aux` (already saved), and the farmstead lots use the
/// new `Farmhouse` kind (code 50). The layout of the file is unchanged, so every
/// older save still loads — it simply has no farms, as it never did.
const VERSION: u32 = 19;
const VERSION_MIN: u32 = 1;

fn kind_to_u8(k: Kind) -> u8 {
    match k {
        Kind::Grass => 0,
        Kind::Meadow => 1,
        Kind::Upland => 2,
        Kind::DryGrass => 3,
        Kind::Scree => 4,
        Kind::Rock => 5,
        Kind::Sand => 6,
        Kind::Road => 7,
        Kind::Sidewalk => 8,
        Kind::Water => 9,
        Kind::Dirt => 10,
        Kind::Gravel => 11,
        Kind::Paved => 12,
        Kind::Bridge => 13,
        Kind::Field => 14,
    }
}

fn kind_from_u8(v: u8) -> io::Result<Kind> {
    Ok(match v {
        0 => Kind::Grass,
        1 => Kind::Meadow,
        2 => Kind::Upland,
        3 => Kind::DryGrass,
        4 => Kind::Scree,
        5 => Kind::Rock,
        6 => Kind::Sand,
        7 => Kind::Road,
        8 => Kind::Sidewalk,
        9 => Kind::Water,
        10 => Kind::Dirt,
        11 => Kind::Gravel,
        12 => Kind::Paved,
        13 => Kind::Bridge,
        14 => Kind::Field,
        _ => return Err(io::Error::new(io::ErrorKind::InvalidData, "bad kind")),
    })
}

/// B62: the save code of a lot kind (exposed for the landmark tests).
pub(crate) fn lot_kind_code(k: LotKind) -> u8 {
    lot_kind_to_u8(k)
}

/// B62: the lot kind a save code means.
pub(crate) fn lot_kind_from_code(v: u8) -> io::Result<LotKind> {
    lot_kind_from_u8(v)
}

fn lot_kind_to_u8(k: LotKind) -> u8 {
    match k {
        LotKind::House => 0,
        LotKind::Church => 1,
        LotKind::Store => 2,
        LotKind::Shop => 3,
        LotKind::Barn => 4,
        LotKind::Farmhouse => 50,
        LotKind::Townhouse => 5,
        LotKind::Apartment => 6,
        LotKind::Office => 7,
        LotKind::Tower => 8,
        LotKind::Industry => 9,
        LotKind::Park => 10,
        LotKind::Hall => 11,
        // B35: 12 + the landmark index, so the old numbers stay valid.
        LotKind::Landmark(crate::landmarks::LandmarkKind::Castle) => 28,
        // B52: the civic centre's own codes (12 + their index would collide).
        LotKind::Landmark(crate::landmarks::LandmarkKind::Courthouse) => 29,
        LotKind::Landmark(crate::landmarks::LandmarkKind::PoliceStation) => 30,
        LotKind::Landmark(crate::landmarks::LandmarkKind::PostOffice) => 31,
        LotKind::Landmark(k) => 12 + landmark_to_u8(k),
        LotKind::LogCabin => 26,
        LotKind::Cemetery => 27,
        // B59: car culture places, appended so every older code still means
        // what it did.
        LotKind::CarPark => 40,
        LotKind::CarParkDeck => 41,
        LotKind::GasStation => 42,
        LotKind::ChargingHub => 43,
        LotKind::DriveThrough => 44,
        LotKind::CarWash => 45,
        LotKind::Dealership => 46,
        LotKind::Motel => 47,
    }
}

fn landmark_to_u8(k: crate::landmarks::LandmarkKind) -> u8 {
    use crate::landmarks::LandmarkKind::*;
    match k {
        Cathedral => 0,
        Station => 1,
        School => 2,
        Library => 3,
        FireStation => 4,
        Hospital => 5,
        Museum => 6,
        Stadium => 7,
        SignatureTower => 8,
        Lighthouse => 9,
        Port => 10,
        FerrisWheel => 11,
        Plaza => 12,
        Zoo => 13,
        // 12 + 14 would be the log cabin's code; the castle has its own.
        Castle => 14,
        Courthouse => 15,
        PoliceStation => 16,
        PostOffice => 17,
    }
}

fn landmark_from_u8(v: u8) -> Option<crate::landmarks::LandmarkKind> {
    use crate::landmarks::LandmarkKind::*;
    Some(match v {
        0 => Cathedral,
        1 => Station,
        2 => School,
        3 => Library,
        4 => FireStation,
        5 => Hospital,
        6 => Museum,
        7 => Stadium,
        8 => SignatureTower,
        9 => Lighthouse,
        10 => Port,
        11 => FerrisWheel,
        12 => Plaza,
        13 => Zoo,
        14 => Castle,
        15 => Courthouse,
        16 => PoliceStation,
        17 => PostOffice,
        _ => return None,
    })
}

fn lot_kind_from_u8(v: u8) -> io::Result<LotKind> {
    Ok(match v {
        0 => LotKind::House,
        1 => LotKind::Church,
        2 => LotKind::Store,
        3 => LotKind::Shop,
        4 => LotKind::Barn,
        50 => LotKind::Farmhouse,
        5 => LotKind::Townhouse,
        6 => LotKind::Apartment,
        7 => LotKind::Office,
        8 => LotKind::Tower,
        9 => LotKind::Industry,
        10 => LotKind::Park,
        11 => LotKind::Hall,
        26 => LotKind::LogCabin,
        27 => LotKind::Cemetery,
        28 => LotKind::Landmark(crate::landmarks::LandmarkKind::Castle),
        29 => LotKind::Landmark(crate::landmarks::LandmarkKind::Courthouse),
        30 => LotKind::Landmark(crate::landmarks::LandmarkKind::PoliceStation),
        31 => LotKind::Landmark(crate::landmarks::LandmarkKind::PostOffice),
        // B59: car culture places.
        40 => LotKind::CarPark,
        41 => LotKind::CarParkDeck,
        42 => LotKind::GasStation,
        43 => LotKind::ChargingHub,
        44 => LotKind::DriveThrough,
        45 => LotKind::CarWash,
        46 => LotKind::Dealership,
        47 => LotKind::Motel,
        v if v >= 12 => match landmark_from_u8(v - 12) {
            Some(k) => LotKind::Landmark(k),
            None => return Err(io::Error::new(io::ErrorKind::InvalidData, "bad landmark kind")),
        },
        _ => return Err(io::Error::new(io::ErrorKind::InvalidData, "bad lot kind")),
    })
}

fn pace_to_u8(p: Pace) -> u8 {
    match p {
        Pace::Day => 0,
        Pace::Week => 1,
        Pace::Month => 2,
    }
}

fn pace_from_u8(v: u8) -> io::Result<Pace> {
    Ok(match v {
        0 => Pace::Day,
        1 => Pace::Week,
        2 => Pace::Month,
        _ => return Err(io::Error::new(io::ErrorKind::InvalidData, "bad pace")),
    })
}

impl Sim {
    /// Write the state atomically: a temp file in the same directory, then a
    /// rename, so a crash never leaves a half-written save.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("tmp");
        {
            let mut f = io::BufWriter::new(std::fs::File::create(&tmp)?);
            self.write_to(&mut f)?;
            f.flush()?;
        }
        std::fs::rename(&tmp, path)
    }

    fn write_to(&self, w: &mut impl Write) -> io::Result<()> {
        w.write_all(MAGIC)?;
        w.write_all(&VERSION.to_le_bytes())?;
        w.write_all(&self.seed.to_le_bytes())?;
        w.write_all(&self.time.to_le_bytes())?;
        w.write_all(&[pace_to_u8(self.pace)])?;
        w.write_all(&self.map.w.to_le_bytes())?;
        w.write_all(&self.map.h.to_le_bytes())?;
        w.write_all(&self.founding.0.to_le_bytes())?;
        w.write_all(&self.founding.1.to_le_bytes())?;
        // D66: the land. Written so a reload keeps its biome whatever the seed's
        // roll would be today.
        w.write_all(&[self.map.biome.id()])?;

        w.write_all(&(self.overrides.len() as u32).to_le_bytes())?;
        for (&(x, y), t) in &self.overrides {
            w.write_all(&x.to_le_bytes())?;
            w.write_all(&y.to_le_bytes())?;
            w.write_all(&t.corner)?;
            w.write_all(&[kind_to_u8(t.kind)])?;
            w.write_all(&t.aux.to_le_bytes())?;
        }

        w.write_all(&(self.lots.len() as u32).to_le_bytes())?;
        for l in &self.lots {
            for v in [l.x, l.y, l.w, l.h] {
                w.write_all(&v.to_le_bytes())?;
            }
            w.write_all(&[l.level, lot_kind_to_u8(l.kind), l.stage.index() as u8])?;
            w.write_all(&[l.front])?;
            // v3: the height and the district style (B26b item 7).
            w.write_all(&[l.storeys, l.style])?;
            w.write_all(&l.seed.to_le_bytes())?;
            w.write_all(&l.stage_end.to_le_bytes())?;
            w.write_all(&l.started.to_le_bytes())?;
        }

        w.write_all(&(self.fields.len() as u32).to_le_bytes())?;
        for f in &self.fields {
            for v in [f.x, f.y, f.w, f.h] {
                w.write_all(&v.to_le_bytes())?;
            }
        }

        // B31b: the railway and everything planned with it.
        let r = &self.rail;
        for v in [r.line.len() as u32, r.tram_line.len() as u32] {
            w.write_all(&v.to_le_bytes())?;
        }
        for &(x, y) in &r.line {
            w.write_all(&x.to_le_bytes())?;
            w.write_all(&y.to_le_bytes())?;
        }
        for &(x, y) in &r.tram_line {
            w.write_all(&x.to_le_bytes())?;
            w.write_all(&y.to_le_bytes())?;
        }
        let z = |z: Option<crate::rail::Zone>| z.unwrap_or(crate::rail::Zone { x: 0, y: 0, w: 0, h: 0 });
        for zone in [z(r.station.map(|(x, y)| crate::rail::Zone { x, y, w: 1, h: 1 })), z(r.yard), z(r.marina), z(r.port)] {
            for v in [zone.x, zone.y, zone.w, zone.h] {
                w.write_all(&v.to_le_bytes())?;
            }
        }
        w.write_all(&(r.station.is_some() as u8).to_le_bytes())?;
        w.write_all(&(r.containers.len() as u32).to_le_bytes())?;
        for &(x, y, pick) in &r.containers {
            w.write_all(&x.to_le_bytes())?;
            w.write_all(&y.to_le_bytes())?;
            w.write_all(&[pick])?;
        }
        // v5: the platform beside the track and the tram stops (B31b).
        match r.station_platform {
            Some((px, py, side)) => {
                w.write_all(&[1u8])?;
                w.write_all(&px.to_le_bytes())?;
                w.write_all(&py.to_le_bytes())?;
                w.write_all(&[side])?;
            }
            None => w.write_all(&[0u8])?,
        }
        w.write_all(&(r.tram_stops.len() as u32).to_le_bytes())?;
        for &(x, y) in &r.tram_stops {
            w.write_all(&x.to_le_bytes())?;
            w.write_all(&y.to_le_bytes())?;
        }
        w.write_all(&r.tick.to_le_bytes())?;
        w.write_all(&r.clock.to_le_bytes())?;
        w.write_all(&[r.planned as u8])?;

        // D45: only the worn tiles are stored (a sparse list), so a village
        // save stays small.
        let worn: Vec<(i32, i32, u8)> = self
            .map
            .wear
            .iter()
            .enumerate()
            .filter(|(_, w)| **w > 0)
            .map(|(i, w)| {
                let (x, y) = ((i as i32) % self.map.w, (i as i32) / self.map.w);
                (x, y, *w)
            })
            .collect();
        w.write_all(&(worn.len() as u32).to_le_bytes())?;
        for (x, y, v) in worn {
            w.write_all(&x.to_le_bytes())?;
            w.write_all(&y.to_le_bytes())?;
            w.write_all(&[v])?;
        }
        // B43: the residents. Names are table indices, so a reload keeps the
        // same people byte for byte, family links included.
        let people = &self.people.list;
        w.write_all(&(people.len() as u32).to_le_bytes())?;
        for p in people {
            w.write_all(&[p.first, p.last])?;
            w.write_all(&p.age.to_le_bytes())?;
            w.write_all(&[match p.kind {
                people::AgeGroup::Adult => 0,
                people::AgeGroup::Child => 1,
                people::AgeGroup::Elder => 2,
            }])?;
            let z = p.home.unwrap_or((i32::MIN, i32::MIN));
            let j = p.job.unwrap_or((i32::MIN, i32::MIN));
            for v in [z.0, z.1, j.0, j.1, p.spouse.map(|v| v as i32).unwrap_or(-1), p.parent.map(|v| v as i32).unwrap_or(-1)] {
                w.write_all(&v.to_le_bytes())?;
            }
            w.write_all(&[p.job_kind.to_u8(), p.tone, p.jitter])?;
        }
        // v9: the city's name (D68) and the story feed (B48a/D65). The name is
        // what the place calls itself; the feed is the ring the tray reads.
        let name = self.city_name.as_bytes();
        w.write_all(&(name.len() as u32).to_le_bytes())?;
        w.write_all(name)?;
        w.write_all(&[self.wx_kind])?;
        w.write_all(&self.wx_at.to_le_bytes())?;
        w.write_all(&[self.holidays_told])?;
        let n = self.story.len();
        w.write_all(&(n as u32).to_le_bytes())?;
        for (l, _) in self.story.iter_flagged() {
            w.write_all(&l.time.to_le_bytes())?;
            let t = l.text.as_bytes();
            w.write_all(&(t.len() as u32).to_le_bytes())?;
            w.write_all(t)?;
        }
        // (B48's v10 lives and v11 story flags stand above.)
        // v10 (B48): the lives behind the roster — when each was born and
        // whether they are still here — the day the life pass last ran, and
        // the scenes still to be told or drawn.
        w.write_all(&(self.people.list.len() as u32).to_le_bytes())?;
        for p in &self.people.list {
            w.write_all(&p.born.to_le_bytes())?;
            w.write_all(&[p.life.to_u8()])?;
        }
        w.write_all(&self.people.last_day.to_le_bytes())?;
        w.write_all(&(self.people.scenes.len() as u32).to_le_bytes())?;
        for sc in &self.people.scenes {
            w.write_all(&[sc.kind.to_u8(), sc.open as u8, sc.told as u8, sc.front])?;
            w.write_all(&sc.start.to_le_bytes())?;
            w.write_all(&sc.dur.to_le_bytes())?;
            w.write_all(&sc.at.0.to_le_bytes())?;
            w.write_all(&sc.at.1.to_le_bytes())?;
            w.write_all(&sc.seed.to_le_bytes())?;
            let t = sc.line.as_bytes();
            w.write_all(&(t.len() as u32).to_le_bytes())?;
            w.write_all(t)?;
        }
        // v11 (B48): which story lines are milestones (kept) and which roll.
        let flagged = self.story.iter_flagged();
        w.write_all(&(flagged.len() as u32).to_le_bytes())?;
        for (_, kept) in flagged {
            w.write_all(&[kept as u8])?;
        }
        // v12 (B50): the ending.
        let e = &self.ending;
        w.write_all(&[e.kind.id()])?;
        w.write_all(&e.world_days.to_le_bytes())?;
        w.write_all(&e.next_seed.to_le_bytes())?;
        w.write_all(&[e.next_biome.id(), e.site_set as u8, e.rebirth_taken as u8])?;
        w.write_all(&e.site.0.to_le_bytes())?;
        w.write_all(&e.site.1.to_le_bytes())?;
        for v in e.aux {
            w.write_all(&v.to_le_bytes())?;
        }
        w.write_all(&e.told.to_le_bytes())?;
        for c in e.cur {
            w.write_all(&c.to_le_bytes())?;
        }
        let fl = e.final_line.as_bytes();
        w.write_all(&(fl.len() as u32).to_le_bytes())?;
        w.write_all(fl)?;
        w.write_all(&(e.props.len() as u32).to_le_bytes())?;
        for (&(x, y), p) in &e.props {
            w.write_all(&x.to_le_bytes())?;
            w.write_all(&y.to_le_bytes())?;
            w.write_all(&prop_to_bytes(*p))?;
        }
        // v13 (B49): the world's technology path and everything its crews
        // installed.
        write_tech(w, &self.tech)?;
        // v14 (B52): the economy, the government, the pollution, the incidents.
        write_civics(w, &self.civics)?;
        // v15 (B58): the transport that grows with the city. The line and the
        // station were already saved; this is everything planned around them —
        // the second track, the station building, the yard's sidings, the bus
        // lines, the subway works, the airfield and the harbour. Without it a
        // reload would re-site an airport in a grown city (or lose it).
        let r = &self.rail;
        w.write_all(&[r.electrified as u8])?;
        match r.metro_at {
            Some(t) => {
                w.write_all(&[1u8])?;
                w.write_all(&t.to_le_bytes())?;
            }
            None => w.write_all(&[0u8])?,
        }
        w_pts(w, &r.twin)?;
        match &r.station_build {
            Some(b) => {
                w.write_all(&[1u8, b.kind.index()])?;
                w_zone(w, b.building)?;
                w.write_all(&(b.platforms.len() as u32).to_le_bytes())?;
                for &(x, y, side) in &b.platforms {
                    w.write_all(&x.to_le_bytes())?;
                    w.write_all(&y.to_le_bytes())?;
                    w.write_all(&[side])?;
                }
                w_pts(w, &b.canopy)?;
                w_pts(w, &b.forecourt)?;
            }
            None => w.write_all(&[0u8, 0u8])?,
        }
        w_pts(w, &r.sidings)?;
        w.write_all(&(r.parked_wagons.len() as u32).to_le_bytes())?;
        for &((x, y), k) in &r.parked_wagons {
            w.write_all(&x.to_le_bytes())?;
            w.write_all(&y.to_le_bytes())?;
            w.write_all(&[k])?;
        }
        w.write_all(&(r.tram_stop_sides.len() as u32).to_le_bytes())?;
        w.write_all(&r.tram_stop_sides)?;
        w.write_all(&(r.bus_lines.len() as u32).to_le_bytes())?;
        for l in &r.bus_lines {
            w_pts(w, &l.route)?;
            w_pts(w, &l.stops)?;
            w.write_all(&(l.sides.len() as u32).to_le_bytes())?;
            w.write_all(&l.sides)?;
        }
        match r.depot {
            Some(z) => {
                w.write_all(&[1u8])?;
                w_zone(w, z)?;
            }
            None => w.write_all(&[0u8])?,
        }
        match &r.subway {
            Some(sb) => {
                w.write_all(&[1u8])?;
                w.write_all(&sb.started.to_le_bytes())?;
                w_pts(w, &sb.line)?;
                w.write_all(&(sb.stations.len() as u32).to_le_bytes())?;
                for st in &sb.stations {
                    w.write_all(&st.at.0.to_le_bytes())?;
                    w.write_all(&st.at.1.to_le_bytes())?;
                    w.write_all(&st.entrance.0.to_le_bytes())?;
                    w.write_all(&st.entrance.1.to_le_bytes())?;
                    w.write_all(&[st.entrance.2])?;
                }
                w_pts(w, &sb.grates)?;
            }
            None => w.write_all(&[0u8])?,
        }
        match &r.airport {
            Some(ap) => {
                w.write_all(&[1u8, ap.stage.index(), ap.width.max(1) as u8])?;
                w_pts(w, &ap.runway)?;
                w_pts(w, &ap.taxiway)?;
                w_zone(w, ap.terminal)?;
                w_zone(w, ap.apron)?;
                match ap.hangar {
                    Some(z) => {
                        w.write_all(&[1u8])?;
                        w_zone(w, z)?;
                    }
                    None => w.write_all(&[0u8])?,
                }
                match ap.tower {
                    Some((x, y, side)) => {
                        w.write_all(&[1u8])?;
                        w.write_all(&x.to_le_bytes())?;
                        w.write_all(&y.to_le_bytes())?;
                        w.write_all(&[side])?;
                    }
                    None => w.write_all(&[0u8])?,
                }
                w.write_all(&ap.gate.0.to_le_bytes())?;
                w.write_all(&ap.gate.1.to_le_bytes())?;
                w.write_all(&ap.windsock.0.to_le_bytes())?;
                w.write_all(&ap.windsock.1.to_le_bytes())?;
                w_pts(w, &ap.access)?;
                w_pts(w, &ap.parked)?;
                w.write_all(&ap.paved_at.to_le_bytes())?;
            }
            None => w.write_all(&[0u8, 0u8])?,
        }
        match &r.harbour {
            Some(h) => {
                // v12: the quay, its water side, the piers as lists, the shed
                // and the quay cranes (B58c straightened the harbour).
                w.write_all(&[1u8])?;
                w_pts(w, &h.quay)?;
                w.write_all(&h.water_dir.0.to_le_bytes())?;
                w.write_all(&h.water_dir.1.to_le_bytes())?;
                w.write_all(&(h.piers.len() as u32).to_le_bytes())?;
                for pier in &h.piers {
                    w_pts(w, pier)?;
                }
                w_pts(w, &h.ferry)?;
                w_pts(w, &h.moored)?;
                for v in [h.shed.0, h.shed.1, h.shed.2, h.shed.3] {
                    w.write_all(&v.to_le_bytes())?;
                }
                w_pts(w, &h.cranes)?;
            }
            None => w.write_all(&[0u8])?,
        }
        // ===== v16 (B51): the chronicle. Last in the file, so every older
        // block keeps its place. =====
        write_chronicle(w, &self.chronicle)?;
        // ===== v17: the town grid's planner state. =====
        write_blocks(w, &self.blocks)?;
        // ===== v18 (B88): the rest of the growth state. =====
        write_growth(w, self)?;
        Ok(())
    }

    /// Load a save. The terrain is regenerated from the seed and the recorded
    /// tile overrides are replayed on top.
    pub fn load(path: &Path) -> io::Result<Sim> {
        let mut f = io::BufReader::new(std::fs::File::open(path)?);
        let mut magic = [0u8; 4];
        f.read_exact(&mut magic)?;
        if &magic != MAGIC {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "not a city save"));
        }
        let version = read_u32(&mut f)?;
        if !(VERSION_MIN..=VERSION).contains(&version) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("save version {version}, expected {VERSION_MIN}..={VERSION}"),
            ));
        }
        let seed = read_u64(&mut f)?;
        let time = read_f64(&mut f)?;
        let mut b = [0u8; 1];
        f.read_exact(&mut b)?;
        let pace = pace_from_u8(b[0])?;
        let w = read_i32(&mut f)?;
        let h = read_i32(&mut f)?;
        let fx = read_i32(&mut f)?;
        let fy = read_i32(&mut f)?;
        if !(16..=4096).contains(&w) || !(16..=4096).contains(&h) {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "bad map size"));
        }
        // D66: v8 records the land; everything older was temperate.
        let biome = if version >= 8 {
            let mut bb = [0u8; 1];
            f.read_exact(&mut bb)?;
            map::Biome::from_id(bb[0])
        } else {
            map::Biome::Temperate
        };

        let map = map::generate_with(seed, w, h, Some(biome)).map_err(|e| {
            io::Error::new(io::ErrorKind::InvalidData, format!("bad map in save: {e}"))
        })?;
        let mut sim = Sim::new(seed, pace, w, h);
        sim.map = map;
        // D66: the land the world holds, restored from the save (not re-rolled
        // from the seed, which `Sim::new` already did).
        sim.biome = biome;
        sim.time = time;
        sim.founding = (fx, fy);
        sim.overrides.clear();
        sim.lots.clear();
        sim.fields.clear();
        sim.changes.clear();

        let n = read_u32(&mut f)? as usize;
        for _ in 0..n {
            let x = read_i32(&mut f)?;
            let y = read_i32(&mut f)?;
            let mut c = [0u8; 4];
            f.read_exact(&mut c)?;
            f.read_exact(&mut b)?;
            let kind = kind_from_u8(b[0])?;
            let aux = read_u16(&mut f)?;
            let t = Tile {
                corner: c,
                kind,
                prop: map::Prop::None,
                aux,
            };
            sim.map.set(x, y, t);
            sim.overrides.insert((x, y), t);
        }

        let n = read_u32(&mut f)? as usize;
        for _ in 0..n {
            let x = read_i32(&mut f)?;
            let y = read_i32(&mut f)?;
            let lw = read_i32(&mut f)?;
            let lh = read_i32(&mut f)?;
            let mut hdr = [0u8; 3];
            f.read_exact(&mut hdr)?;
            let mut front = [0u8; 1];
            f.read_exact(&mut front)?;
            let (storeys, style) = if version >= 3 {
                let mut hs = [0u8; 2];
                f.read_exact(&mut hs)?;
                (hs[0], hs[1])
            } else {
                // v1/v2 had no height or style: use the kind's base height and
                // the style of the district the plot stands in (a v2 save
                // reloads looking like the town it was).
                (lot_kind_from_u8(hdr[1])?.base_storeys().max(1), 0)
            };
            let seed_lot = read_u32(&mut f)?;
            let stage_end = read_f64(&mut f)?;
            let started = read_f64(&mut f)?;
            let stage = Stage::ALL
                .get(hdr[2] as usize)
                .copied()
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "bad stage"))?;
            sim.lots.push(Lot {
                x,
                y,
                w: lw,
                h: lh,
                level: hdr[0],
                kind: lot_kind_from_u8(hdr[1])?,
                storeys,
                style,
                seed: seed_lot,
                front: front[0],
                stage,
                stage_end,
                started,
            });
        }

        let n = read_u32(&mut f)? as usize;
        for _ in 0..n {
            sim.fields.push(Field {
                x: read_i32(&mut f)?,
                y: read_i32(&mut f)?,
                w: read_i32(&mut f)?,
                h: read_i32(&mut f)?,
            });
        }
        // B31b: rail (v4+). Older saves load with no railway and plan one at the
        // next town-era decision, which is the old behaviour.
        if version >= 4 {
            let n_line = read_u32(&mut f)? as usize;
            let n_tram = read_u32(&mut f)? as usize;
            let mut line = Vec::with_capacity(n_line.min(1 << 16));
            for _ in 0..n_line {
                line.push((read_i32(&mut f)?, read_i32(&mut f)?));
            }
            let mut tram = Vec::with_capacity(n_tram.min(1 << 16));
            for _ in 0..n_tram {
                tram.push((read_i32(&mut f)?, read_i32(&mut f)?));
            }
            let mut zones = [crate::rail::Zone { x: 0, y: 0, w: 0, h: 0 }; 4];
            for zone in zones.iter_mut() {
                *zone = crate::rail::Zone {
                    x: read_i32(&mut f)?,
                    y: read_i32(&mut f)?,
                    w: read_i32(&mut f)?,
                    h: read_i32(&mut f)?,
                };
            }
            let mut has_station = [0u8; 1];
            f.read_exact(&mut has_station)?;
            let n_cont = read_u32(&mut f)? as usize;
            let mut containers = Vec::with_capacity(n_cont.min(1 << 16));
            for _ in 0..n_cont {
                let x = read_i32(&mut f)?;
                let y = read_i32(&mut f)?;
                let mut b = [0u8; 1];
                f.read_exact(&mut b)?;
                containers.push((x, y, b[0]));
            }
            if version >= 5 {
                let mut has_platform = [0u8; 1];
                f.read_exact(&mut has_platform)?;
                if has_platform[0] != 0 {
                    let px = read_i32(&mut f)?;
                    let py = read_i32(&mut f)?;
                    let mut side = [0u8; 1];
                    f.read_exact(&mut side)?;
                    sim.rail.station_platform = Some((px, py, side[0]));
                } else {
                    sim.rail.station_platform = None;
                }
                let n_stops = read_u32(&mut f)? as usize;
                let mut stops = Vec::with_capacity(n_stops.min(1 << 16));
                for _ in 0..n_stops {
                    stops.push((read_i32(&mut f)?, read_i32(&mut f)?));
                }
                sim.rail.tram_stops = stops;
            }
            if version >= 5 {
                sim.rail.tick = read_u64(&mut f)?;
            }
            let clock = read_f64(&mut f)?;
            let mut planned = [0u8; 1];
            f.read_exact(&mut planned)?;
            sim.rail.line = line;
            sim.rail.tram_line = tram;
            sim.rail.station = (has_station[0] != 0).then_some((zones[0].x, zones[0].y));
            let nz = |z: crate::rail::Zone| (z.w > 0 && z.h > 0).then_some(z);
            sim.rail.yard = nz(zones[1]);
            sim.rail.marina = nz(zones[2]);
            sim.rail.port = nz(zones[3]);
            sim.rail.containers = containers;
            sim.rail.clock = clock;
            sim.rail.planned = planned[0] != 0;
            sim.rail.next_train = clock + 30.0;
            sim.rail.next_boat = clock + 90.0;
            sim.rail.next_plane = clock + 150.0;
            sim.rail.next_ship = clock + 60.0;
        }

        // Resume the clock exactly where it stopped.
        // D45: the wear map, when the save has one.
        if version >= 2 {
            let n = read_u32(&mut f)? as usize;
            for _ in 0..n {
                let x = read_i32(&mut f)?;
                let y = read_i32(&mut f)?;
                let mut b = [0u8; 1];
                f.read_exact(&mut b)?;
                if sim.map.in_bounds(x, y) {
                    sim.map.wear[(y * sim.map.w + x) as usize] = b[0];
                    if map::WearStage::of(b[0]) >= map::WearStage::Footpath {
                        sim.worn.insert((x, y));
                    }
                }
            }
        }

        // B43/D65: the residents, when the save has them.
        if version >= 7 {
            let n = read_u32(&mut f)? as usize;
            for _ in 0..n {
                let mut nm = [0u8; 2];
                f.read_exact(&mut nm)?;
                let age = read_u16(&mut f)?;
                let mut k = [0u8; 1];
                f.read_exact(&mut k)?;
                let kind = match k[0] {
                    0 => people::AgeGroup::Adult,
                    1 => people::AgeGroup::Child,
                    2 => people::AgeGroup::Elder,
                    _ => return Err(io::Error::new(io::ErrorKind::InvalidData, "bad person kind")),
                };
                let hx = read_i32(&mut f)?;
                let hy = read_i32(&mut f)?;
                let jx = read_i32(&mut f)?;
                let jy = read_i32(&mut f)?;
                let sp = read_i32(&mut f)?;
                let pa = read_i32(&mut f)?;
                let mut tail = [0u8; 3];
                f.read_exact(&mut tail)?;
                sim.people.list.push(people::Person {
                    first: nm[0],
                    last: nm[1],
                    age,
                    kind,
                    home: (hx != i32::MIN).then_some((hx, hy)),
                    job: (jx != i32::MIN).then_some((jx, jy)),
                    job_kind: people::JobKind::from_u8(tail[0]),
                    spouse: (sp >= 0).then_some(sp as u32),
                    parent: (pa >= 0).then_some(pa as u32),
                    tone: tail[1],
                    jitter: tail[2],
                    born: 0,
                    life: people::Life::Alive,
                });
            }
        }

        release_landforms(&mut sim, seed, w, h, biome, version < 12);
        sim.step_index = (sim.time / crate::PLANNING_STEP) as u64;
        sim.next_decision = (sim.time / crate::DECISION_PERIOD).floor() * crate::DECISION_PERIOD
            + crate::DECISION_PERIOD;
        // B41b: a save holds a *city*, not a settlement.
        // `Sim::new` spawned a
        // party (found() runs before the map is replaced), so a loaded save
        // would otherwise walk a new camp into a grown town. Any save with a
        // house in it is past its settlement: strike the camp and leave the
        // ground to the village.
        let grown = sim.lots.iter().any(|l| !matches!(l.kind, crate::LotKind::LogCabin))
            || sim.lane_tiles().len() > 3;
        if grown {
            sim.strike_camp(sim.time);
        }
        // B43: an older save has no residents of its own; the city it recorded
        // is enough to grow them, exactly as a live city would have.
        if version < 7 {
            sim.people.last_day = (sim.time / 86400.0).floor() as i64;
            sim.sync_people();
        }
        // v9: the name and the feed. A v8-and-younger save has no feed yet and
        // a city that has not told its naming: give it the name the land gives
        // a city of its seed, exactly as the founding would have.
        let mut story_rows: Vec<(crate::story::Line, bool)> = Vec::new();
        if version >= 9 {
            let n = read_u32(&mut f)? as usize;
            if n > 256 {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "city name too long"));
            }
            let mut bytes = vec![0u8; n];
            f.read_exact(&mut bytes)?;
            sim.city_name = String::from_utf8_lossy(&bytes).into_owned();
            let mut b = [0u8; 1];
            f.read_exact(&mut b)?;
            sim.wx_kind = b[0];
            sim.wx_at = read_f64(&mut f)?;
            f.read_exact(&mut b)?;
            sim.holidays_told = b[0];
            let n = read_u32(&mut f)? as usize;
            if n > 10_000 {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "story too long"));
            }
            for _ in 0..n {
                let t = read_f64(&mut f)?;
                let tl = read_u32(&mut f)? as usize;
                if tl > 4096 {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "story line too long"));
                }
                let mut tb = vec![0u8; tl];
                f.read_exact(&mut tb)?;
                story_rows.push((crate::story::Line { time: t, text: String::from_utf8_lossy(&tb).into_owned() }, true));
            }
        } else {
            sim.city_name = crate::story::city_name(seed, biome);
        }
        // v10 (B48): the lives. An older save has the roster but not the
        // birthdays: derive them from the ages so the city keeps its people,
        // and start the life pass from today rather than replaying the past.
        let today = (sim.time / 86400.0).floor() as i64;
        if version >= 10 {
            let n = read_u32(&mut f)? as usize;
            if n != sim.people.list.len() {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "roster size mismatch"));
            }
            for p in sim.people.list.iter_mut() {
                p.born = read_i32(&mut f)?;
                let mut b = [0u8; 1];
                f.read_exact(&mut b)?;
                p.life = people::Life::from_u8(b[0]);
            }
            sim.people.last_day = read_u64(&mut f)? as i64;
            let ns = read_u32(&mut f)? as usize;
            if ns > 100_000 {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "too many scenes"));
            }
            for _ in 0..ns {
                let mut h = [0u8; 4];
                f.read_exact(&mut h)?;
                let start = read_f64(&mut f)?;
                let dur = read_f64(&mut f)?;
                let ax = read_i32(&mut f)?;
                let ay = read_i32(&mut f)?;
                let seed = read_u32(&mut f)?;
                let tl = read_u32(&mut f)? as usize;
                if tl > 4096 {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "scene line too long"));
                }
                let mut tb = vec![0u8; tl];
                f.read_exact(&mut tb)?;
                sim.people.scenes.push(crate::life::LifeScene {
                    kind: crate::life::SceneKind::from_u8(h[0]),
                    open: h[1] != 0,
                    told: h[2] != 0,
                    front: h[3],
                    start,
                    dur,
                    at: (ax, ay),
                    seed,
                    line: String::from_utf8_lossy(&tb).into_owned(),
                });
            }
        } else {
            for p in sim.people.list.iter_mut() {
                p.born = (today - (p.age as f64 * people::LIFE_YEAR_DAYS) as i64) as i32;
            }
            sim.people.last_day = today;
        }
        // v11: which lines roll. Older feeds were all milestones.
        if version >= 11 {
            let n = read_u32(&mut f)? as usize;
            if n != story_rows.len() {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "story flags mismatch"));
            }
            for row in story_rows.iter_mut() {
                let mut b = [0u8; 1];
                f.read_exact(&mut b)?;
                row.1 = b[0] != 0;
            }
        }
        if version >= 9 {
            sim.story = crate::story::Story::from_flagged(story_rows);
        }
        sim.people.recount();
        // B50: the ending. The plan is a function of the seed and the land;
        // a v12 save then overrides it with how far it had got.
        sim.ending = crate::ending::State::plan(seed, &sim.map);
        if version >= 12 {
            let mut b = [0u8; 1];
            f.read_exact(&mut b)?;
            let kind = crate::ending::Ending::from_id(b[0]);
            let world_days = read_f64(&mut f)?;
            let next_seed = read_u64(&mut f)?;
            let mut b3 = [0u8; 3];
            f.read_exact(&mut b3)?;
            let sx = f32::from_le_bytes(read_u32(&mut f)?.to_le_bytes());
            let sy = f32::from_le_bytes(read_u32(&mut f)?.to_le_bytes());
            let mut st = crate::ending::State::new(kind, world_days, next_seed, map::Biome::from_id(b3[0]));
            st.site_set = b3[1] != 0;
            st.rebirth_taken = b3[2] != 0;
            st.site = (sx, sy);
            for a in st.aux.iter_mut() {
                *a = f32::from_le_bytes(read_u32(&mut f)?.to_le_bytes());
            }
            st.told = read_u32(&mut f)?;
            for c in st.cur.iter_mut() {
                *c = read_u32(&mut f)?;
            }
            let n = read_u32(&mut f)? as usize;
            if n > 4096 {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "final line too long"));
            }
            let mut bytes = vec![0u8; n];
            f.read_exact(&mut bytes)?;
            st.final_line = String::from_utf8_lossy(&bytes).into_owned();
            let np = read_u32(&mut f)? as usize;
            if np > 1_000_000 {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "too many props"));
            }
            for _ in 0..np {
                let x = read_i32(&mut f)?;
                let y = read_i32(&mut f)?;
                let mut pb = [0u8; 4];
                f.read_exact(&mut pb)?;
                st.props.insert((x, y), prop_from_bytes(pb));
            }
            sim.ending = st;
        } else {
            // A world that began before endings existed: the end is at least
            // a few days off, whatever its seed would have said.
            let tm = sim.ending.kind.timing();
            let earliest = (sim.time + tm.warn + 3.0 * 86400.0) / 86400.0;
            if sim.ending.world_days < earliest {
                sim.ending.world_days = earliest;
            }
        }
        // B49: the technology path (v13). An older save never reached the
        // eras, so it keeps the path its seed draws and nothing installed.
        if version >= 13 {
            read_tech(&mut f, &mut sim)?;
        }
        // B52: the civics block (v14). An older save keeps the calm default.
        if version >= 14 {
            sim.civics = read_civics(&mut f)?;
        }
        sim.restore_ending();
        // v15 (B58): the transport that grows with the city.
        if version >= 15 {
            read_transit(&mut f, &mut sim)?;
        }
        // ===== v16 (B51): the chronicle. An older save backfills it from its
        // kept feed lines, so every world has a history. =====
        if version >= 16 {
            sim.chronicle = read_chronicle(&mut f)?;
        } else {
            sim.chron_backfill();
        }
        // ===== v17: the town grid's planner state. =====
        if version >= 17 {
            read_blocks(&mut f, &mut sim.blocks)?;
        }
        // ===== v18 (B88): the rest of the growth state. =====
        // The one-shot flags guard the live run only; a loaded city re-derives
        // them, so it never re-tells a line its feed already has (B48a). A v18
        // save carries them exactly instead (a milestone crossed in the last
        // hour but not yet told must still be told after the reload).
        if version >= 18 {
            read_growth(&mut f, &mut sim)?;
        } else {
            sim.refresh_firsts();
        }
        if version < 18 {
            sim.thin_signs();
        }
        // B66: the tiles above were written straight into the map, so the reach
        // table (which land the streets can reach) is rebuilt on first use.
        sim.reach.invalidate();
        // B78: the years on the buildings are derived, not saved — compute them
        // now, or a loaded city would look brand new until the next sim hour.
        // B88: a v18 save whose city had not seeded its districts yet is a city
        // the live run had not aged yet either: running the pass now would seed
        // them from the wrong step counter and restyle plots an hour early.
        if version < 18 || sim.districts.is_some() {
            sim.ageing_step();
        }
        Ok(sim)
    }
}

/// Write the town grid's planner state (v17): ready, origin, then the opened,
/// skipped and reclaimed cells (count, then i32 pairs).
fn write_blocks(w: &mut impl Write, b: &crate::blocks::Blocks) -> io::Result<()> {
    w.write_all(&[b.is_ready() as u8])?;
    w.write_all(&b.ox.to_le_bytes())?;
    w.write_all(&b.oy.to_le_bytes())?;
    for set in [&b.open, &b.skip, &b.reclaimed] {
        w.write_all(&(set.len() as u32).to_le_bytes())?;
        for &(i, j) in set.iter() {
            w.write_all(&i.to_le_bytes())?;
            w.write_all(&j.to_le_bytes())?;
        }
    }
    w.write_all(&b.opened.to_le_bytes())?;
    Ok(())
}

fn read_blocks(r: &mut impl Read, b: &mut crate::blocks::Blocks) -> io::Result<()> {
    let mut ready = [0u8; 1];
    r.read_exact(&mut ready)?;
    let ox = read_i32(r)?;
    let oy = read_i32(r)?;
    if ready[0] != 0 {
        b.restore_alignment(ox, oy);
    }
    for k in 0..3 {
        let n = read_u32(r)?;
        if n > 1_000_000 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "grid cell count"));
        }
        let mut set = std::collections::BTreeSet::new();
        for _ in 0..n {
            let i = read_i32(r)?;
            let j = read_i32(r)?;
            set.insert((i, j));
        }
        match k {
            0 => b.open = set,
            1 => b.skip = set,
            _ => b.reclaimed = set,
        }
    }
    b.opened = read_u32(r)?;
    Ok(())
}

fn district_to_u8(d: crate::District) -> u8 {
    use crate::District::*;
    match d {
        Core => 0,
        European => 1,
        Suburb => 2,
        Downtown => 3,
        HighEnd => 4,
        Industrial => 5,
    }
}

fn district_from_u8(v: u8) -> io::Result<crate::District> {
    use crate::District::*;
    Ok(match v {
        0 => Core,
        1 => European,
        2 => Suburb,
        3 => Downtown,
        4 => HighEnd,
        5 => Industrial,
        _ => return Err(io::Error::new(io::ErrorKind::InvalidData, "bad district")),
    })
}

fn role_to_u8(r: crate::blocks::Role) -> u8 {
    use crate::blocks::Role::*;
    match r {
        Low => 0,
        Mid => 1,
        High => 2,
        Work => 3,
        Green => 4,
    }
}

fn role_from_u8(v: u8) -> io::Result<crate::blocks::Role> {
    use crate::blocks::Role::*;
    Ok(match v {
        0 => Low,
        1 => Mid,
        2 => High,
        3 => Work,
        4 => Green,
        _ => return Err(io::Error::new(io::ErrorKind::InvalidData, "bad plan role")),
    })
}

fn era_to_u8(e: crate::Era) -> u8 {
    e as u8
}

fn era_from_u8(v: u8) -> Option<crate::Era> {
    use crate::Era::*;
    [Settlement, Village, Town, City, Metropolis].get(v as usize).copied()
}

fn job_kind_to_u8(k: crate::streetworks::JobKind) -> u8 {
    use crate::streetworks::JobKind::*;
    match k {
        Paint => 0,
        Signs => 1,
        Lamps => 2,
        Lights => 3,
    }
}

fn job_kind_from_u8(v: u8) -> io::Result<crate::streetworks::JobKind> {
    use crate::streetworks::JobKind::*;
    Ok(match v {
        0 => Paint,
        1 => Signs,
        2 => Lamps,
        3 => Lights,
        _ => return Err(io::Error::new(io::ErrorKind::InvalidData, "bad street job")),
    })
}

fn put_u32(w: &mut impl Write, v: u32) -> io::Result<()> {
    w.write_all(&v.to_le_bytes())
}

fn put_pt(w: &mut impl Write, p: (i32, i32)) -> io::Result<()> {
    w.write_all(&p.0.to_le_bytes())?;
    w.write_all(&p.1.to_le_bytes())
}

fn get_pt(r: &mut impl Read) -> io::Result<(i32, i32)> {
    Ok((read_i32(r)?, read_i32(r)?))
}

/// A count read from a save, refused when it is absurd.
fn get_count(r: &mut impl Read, max: u32, what: &'static str) -> io::Result<usize> {
    let n = read_u32(r)?;
    if n > max {
        return Err(io::Error::new(io::ErrorKind::InvalidData, what));
    }
    Ok(n as usize)
}


fn put_f32pair(w: &mut impl Write, p: (f32, f32)) -> io::Result<()> {
    put_f32(w, p.0)?;
    put_f32(w, p.1)
}

fn get_f32pair(r: &mut impl Read) -> io::Result<(f32, f32)> {
    Ok((get_f32(r)?, get_f32(r)?))
}

/// The settlement phase: the party on its legs, the camp, the felling.
fn write_settlers(w: &mut impl Write, s: &crate::settlers::Settlers) -> io::Result<()> {
    put_u32(w, s.party.len() as u32)?;
    for m in &s.party {
        w.write_all(&[m.kind, m.state])?;
        put_f32pair(w, m.from)?;
        put_f32pair(w, m.to)?;
        w.write_all(&m.t0.to_le_bytes())?;
        w.write_all(&m.dur.to_le_bytes())?;
        w.write_all(&m.busy_until.to_le_bytes())?;
        put_pt(w, m.job)?;
        put_u32(w, m.load)?;
    }
    match &s.camp {
        Some(c) => {
            w.write_all(&[1, c.fire_ok as u8])?;
            w_pts(w, &c.tents)?;
            put_pt(w, c.fire)?;
            w.write_all(&c.lit.to_le_bytes())?;
        }
        None => w.write_all(&[0])?,
    }
    put_pt(w, s.target)?;
    w.write_all(&[s.arrived as u8, s.stumps_pulled as u8, s.road_laid as u8, s.struck as u8])?;
    w.write_all(&s.arrived_at.to_le_bytes())?;
    for v in [s.timber, s.felled, s.cabins] {
        put_u32(w, v)?;
    }
    match s.first_cabin_day {
        Some(d) => {
            w.write_all(&[1])?;
            w.write_all(&d.to_le_bytes())?;
        }
        None => w.write_all(&[0])?,
    }
    put_u32(w, s.piles.len() as u32)?;
    for p in &s.piles {
        put_pt(w, (p.x, p.y))?;
        w.write_all(&[p.kind])?;
    }
    put_u32(w, s.falling.len() as u32)?;
    for f in &s.falling {
        put_pt(w, (f.x, f.y))?;
        w.write_all(&f.t0.to_le_bytes())?;
        w.write_all(&f.dur.to_le_bytes())?;
        w.write_all(&[f.dir as u8, f.size, f.variant])?;
    }
    write_tiles(w, &s.worn_trail)
}

fn read_settlers(f: &mut impl Read) -> io::Result<crate::settlers::Settlers> {
    use crate::settlers::*;
    let mut s = Settlers::default();
    let n = get_count(f, 256, "too many settlers")?;
    for _ in 0..n {
        let kind = get_u8(f)?;
        let state = get_u8(f)?;
        let from = get_f32pair(f)?;
        let to = get_f32pair(f)?;
        let (t0, dur, busy_until) = (read_f64(f)?, read_f64(f)?, read_f64(f)?);
        let job = get_pt(f)?;
        let load = read_u32(f)?;
        s.party.push(Member { kind, from, to, t0, dur, state, busy_until, job, load });
    }
    s.camp = if get_u8(f)? != 0 {
        let fire_ok = get_u8(f)? != 0;
        let tents = r_pts(f)?;
        let fire = get_pt(f)?;
        let lit = read_f64(f)?;
        Some(Camp { tents, fire, lit, fire_ok })
    } else {
        None
    };
    s.target = get_pt(f)?;
    let mut b = [0u8; 4];
    f.read_exact(&mut b)?;
    s.arrived = b[0] != 0;
    s.stumps_pulled = b[1] != 0;
    s.road_laid = b[2] != 0;
    s.struck = b[3] != 0;
    s.arrived_at = read_f64(f)?;
    s.timber = read_u32(f)?;
    s.felled = read_u32(f)?;
    s.cabins = read_u32(f)?;
    s.first_cabin_day = if get_u8(f)? != 0 { Some(read_f64(f)?) } else { None };
    let n = get_count(f, 1 << 18, "too many piles")?;
    for _ in 0..n {
        let p = get_pt(f)?;
        s.piles.push(LogPile { x: p.0, y: p.1, kind: get_u8(f)? });
    }
    let n = get_count(f, 1 << 16, "too many falling trees")?;
    for _ in 0..n {
        let p = get_pt(f)?;
        let (t0, dur) = (read_f64(f)?, read_f64(f)?);
        let mut b = [0u8; 3];
        f.read_exact(&mut b)?;
        s.falling.push(Falling { x: p.0, y: p.1, t0, dur, dir: b[0] as i8, size: b[1], variant: b[2] });
    }
    s.worn_trail = read_tiles(f)?;
    Ok(s)
}

/// v18 (B88): everything else growth depends on that a reload used to reset.
fn write_growth(w: &mut impl Write, s: &Sim) -> io::Result<()> {
    write_settlers(w, &s.settlers)?;
    // The clock phase and the rng stream: every decision's rng is seeded from
    // `step_index`, which counts the finer settlement steps too.
    w.write_all(&s.step_index.to_le_bytes())?;
    w.write_all(&s.acc.to_le_bytes())?;
    w.write_all(&s.next_decision.to_le_bytes())?;
    w.write_all(&[s.aimed_at_river as u8])?;
    w.write_all(&(s.trip_cursor as u64).to_le_bytes())?;
    w.write_all(&s.family_story_at.to_le_bytes())?;
    w.write_all(&s.last_dt.to_le_bytes())?;
    // The town is in a slump (nobody is hired; the layoffs have been told).
    w.write_all(&[s.people.slump as u8])?;
    // Which one-shot story lines have been told (a reload must neither repeat
    // one nor drop one that was about to be told).
    let f = &s.firsts;
    w.write_all(&[
        f.arrived as u8,
        f.first_cabin as u8,
        f.first_track as u8,
        f.named as u8,
        f.first_paved as u8,
        f.first_shop as u8,
        f.first_office as u8,
        f.first_tower as u8,
        f.sw_done,
    ])?;
    put_u32(w, f.landmarks_done)?;
    put_u32(w, s.pop_milestone as u32)?;
    // The counters the planner reads (`stats.farms`, `stats.bridges`, ...).
    let st = &s.stats;
    w.write_all(&st.decisions.to_le_bytes())?;
    for v in [
        st.lanes,
        st.lane_tiles,
        st.upgraded,
        st.bridges,
        st.paths_paved,
        st.dev_tried,
        st.dev_built,
        st.farms,
        st.cells,
    ] {
        put_u32(w, v)?;
    }
    // The style districts (seeded from the rng on first use).
    match &s.districts {
        Some(d) => {
            w.write_all(&[1])?;
            put_u32(w, d.len() as u32)?;
            for &(p, k) in d {
                put_pt(w, p)?;
                w.write_all(&[district_to_u8(k)])?;
            }
        }
        None => w.write_all(&[0])?,
    }
    // The land-value cache (refreshed every sixth decision, so it is stale by
    // design: a reload must not refresh it early).
    put_u32(w, s.land.len() as u32)?;
    for &v in &s.land {
        put_f32(w, v)?;
    }
    // The paving queues.
    put_u32(w, s.upgrade_queue.len() as u32)?;
    for &p in &s.upgrade_queue {
        put_pt(w, p)?;
    }
    put_u32(w, s.upgrade_due.len() as u32)?;
    for &(p, t) in &s.upgrade_due {
        put_pt(w, p)?;
        w.write_all(&t.to_le_bytes())?;
    }
    // The tiles that back off hourly (not the same as "worn enough to show").
    write_tiles(w, &s.worn)?;
    // The cached walking routes.
    put_u32(w, s.routes.len() as u32)?;
    for r in &s.routes {
        w_pts(w, r)?;
    }
    // The grid's current block plan and the planner's counters.
    let b = &s.blocks;
    put_u32(w, b.plan.len() as u32)?;
    for p in &b.plan {
        for v in [p.x, p.y, p.w, p.h] {
            w.write_all(&v.to_le_bytes())?;
        }
        w.write_all(&[role_to_u8(p.role)])?;
    }
    put_u32(w, b.at as u32)?;
    match b.cell {
        Some(c) => {
            w.write_all(&[1])?;
            put_pt(w, c)?;
        }
        None => w.write_all(&[0])?,
    }
    put_u32(w, b.tried)?;
    put_u32(w, b.stood)?;
    for v in b.refuse {
        put_u32(w, v)?;
    }
    for v in b.skip_why {
        put_u32(w, v)?;
    }
    // The street furniture and the crews at work.
    let sw = &s.streetworks;
    for set in [
        &sw.paint,
        &sw.lamps,
        &sw.lights,
        &sw.painted_done,
        &sw.lamps_done,
        &sw.signs_done,
        &sw.lights_done,
    ] {
        write_tiles(w, set)?;
    }
    put_u32(w, sw.signs.len() as u32)?;
    for &(x, y, k) in &sw.signs {
        put_pt(w, (x, y))?;
        w.write_all(&[k])?;
    }
    put_u32(w, sw.sign_plan.len() as u32)?;
    for (&p, &k) in &sw.sign_plan {
        put_pt(w, p)?;
        w.write_all(&[k])?;
    }
    put_u32(w, sw.jobs.len() as u32)?;
    for j in &sw.jobs {
        w.write_all(&[job_kind_to_u8(j.kind)])?;
        w_pts(w, &j.path)?;
        put_u32(w, j.i as u32)?;
        w.write_all(&j.next_at.to_le_bytes())?;
    }
    w.write_all(&[sw.halted_in.map_or(255, era_to_u8)])?;
    w.write_all(&[s.tech.halted_in.map_or(255, era_to_u8)])?;
    // The car layer: the highway (planned once), the interchange legs and the
    // clock that spaces the car places out.
    let c = &s.cars;
    w.write_all(&[c.planned_highway as u8])?;
    w.write_all(&c.next_at.to_le_bytes())?;
    for v in [c.trips, c.no_bay, c.departures, c.no_route] {
        put_u32(w, v)?;
    }
    w_pts(w, &c.highway)?;
    w_pts(w, &c.ramps)?;
    Ok(())
}

fn read_growth(f: &mut impl Read, s: &mut Sim) -> io::Result<()> {
    s.settlers = read_settlers(f)?;
    s.step_index = read_u64(f)?;
    s.acc = read_f64(f)?;
    s.next_decision = read_f64(f)?;
    s.aimed_at_river = get_u8(f)? != 0;
    s.trip_cursor = read_u64(f)? as usize;
    s.family_story_at = read_f64(f)?;
    s.last_dt = read_f64(f)?;
    s.people.slump = get_u8(f)? != 0;
    let mut fb = [0u8; 9];
    f.read_exact(&mut fb)?;
    s.firsts = crate::Firsts {
        arrived: fb[0] != 0,
        first_cabin: fb[1] != 0,
        first_track: fb[2] != 0,
        named: fb[3] != 0,
        first_paved: fb[4] != 0,
        first_shop: fb[5] != 0,
        first_office: fb[6] != 0,
        first_tower: fb[7] != 0,
        sw_done: fb[8],
        landmarks_done: read_u32(f)?,
    };
    s.pop_milestone = read_u32(f)? as usize;
    let decisions = read_u64(f)?;
    let mut v = [0u32; 9];
    for x in v.iter_mut() {
        *x = read_u32(f)?;
    }
    let old = std::mem::take(&mut s.stats);
    s.stats = crate::Stats {
        decisions,
        lanes: v[0],
        lane_tiles: v[1],
        upgraded: v[2],
        bridges: v[3],
        paths_paved: v[4],
        dev_tried: v[5],
        dev_built: v[6],
        farms: v[7],
        cells: v[8],
        ..old
    };
    s.districts = if get_u8(f)? != 0 {
        let n = get_count(f, 64, "too many districts")?;
        let mut d = Vec::with_capacity(n);
        for _ in 0..n {
            let p = get_pt(f)?;
            d.push((p, district_from_u8(get_u8(f)?)?));
        }
        Some(d)
    } else {
        None
    };
    let n = get_count(f, 1 << 24, "land cache too big")?;
    s.land = Vec::with_capacity(n);
    for _ in 0..n {
        s.land.push(get_f32(f)?);
    }
    let n = get_count(f, 1 << 20, "too many queued tiles")?;
    s.upgrade_queue = (0..n).map(|_| get_pt(f)).collect::<io::Result<_>>()?;
    let n = get_count(f, 1 << 20, "too many due tiles")?;
    s.upgrade_due = Vec::with_capacity(n);
    for _ in 0..n {
        let p = get_pt(f)?;
        s.upgrade_due.push((p, read_f64(f)?));
    }
    s.worn = read_tiles(f)?;
    let n = get_count(f, 1 << 16, "too many routes")?;
    s.routes = (0..n).map(|_| r_pts(f)).collect::<io::Result<_>>()?;
    let b = &mut s.blocks;
    let n = get_count(f, 1 << 12, "block plan too long")?;
    b.plan.clear();
    for _ in 0..n {
        let (x, y, w, h) = (read_i32(f)?, read_i32(f)?, read_i32(f)?, read_i32(f)?);
        b.plan.push(crate::blocks::PlanLot {
            x,
            y,
            w,
            h,
            role: role_from_u8(get_u8(f)?)?,
        });
    }
    b.at = read_u32(f)? as usize;
    b.cell = if get_u8(f)? != 0 { Some(get_pt(f)?) } else { None };
    b.tried = read_u32(f)?;
    b.stood = read_u32(f)?;
    for r in b.refuse.iter_mut() {
        *r = read_u32(f)?;
    }
    for r in b.skip_why.iter_mut() {
        *r = read_u32(f)?;
    }
    let sw = &mut s.streetworks;
    sw.paint = read_tiles(f)?;
    sw.lamps = read_tiles(f)?;
    sw.lights = read_tiles(f)?;
    sw.painted_done = read_tiles(f)?;
    sw.lamps_done = read_tiles(f)?;
    sw.signs_done = read_tiles(f)?;
    sw.lights_done = read_tiles(f)?;
    let n = get_count(f, 1 << 18, "too many signs")?;
    sw.signs.clear();
    for _ in 0..n {
        let p = get_pt(f)?;
        sw.signs.push((p.0, p.1, get_u8(f)?));
    }
    let n = get_count(f, 1 << 18, "too many planned signs")?;
    sw.sign_plan.clear();
    for _ in 0..n {
        let p = get_pt(f)?;
        sw.sign_plan.insert(p, get_u8(f)?);
    }
    let n = get_count(f, 128, "too many street crews")?;
    sw.jobs.clear();
    for _ in 0..n {
        let kind = job_kind_from_u8(get_u8(f)?)?;
        let path = r_pts(f)?;
        let i = read_u32(f)? as usize;
        let next_at = read_f64(f)?;
        sw.jobs.push(crate::streetworks::Job { kind, path, i, next_at });
    }
    sw.halted_in = era_from_u8(get_u8(f)?);
    s.tech.halted_in = era_from_u8(get_u8(f)?);
    s.cars.planned_highway = get_u8(f)? != 0;
    s.cars.next_at = read_f64(f)?;
    s.cars.trips = read_u32(f)?;
    s.cars.no_bay = read_u32(f)?;
    s.cars.departures = read_u32(f)?;
    s.cars.no_route = read_u32(f)?;
    s.cars.highway = r_pts(f)?;
    s.cars.ramps = r_pts(f)?;
    Ok(())
}

/// Write the chronicle (B51): count, then per entry the sim time, the topic,
/// the number and the text.
fn write_chronicle(w: &mut impl Write, c: &crate::chronicle::Chronicle) -> io::Result<()> {
    w.write_all(&(c.entries.len() as u32).to_le_bytes())?;
    for e in &c.entries {
        w.write_all(&e.time.to_le_bytes())?;
        w.write_all(&[e.topic as u8])?;
        w.write_all(&e.n.to_le_bytes())?;
        let t = e.text.as_bytes();
        w.write_all(&(t.len() as u32).to_le_bytes())?;
        w.write_all(t)?;
    }
    Ok(())
}

fn read_chronicle(f: &mut impl Read) -> io::Result<crate::chronicle::Chronicle> {
    let n = read_u32(f)? as usize;
    if n > crate::chronicle::CHRONICLE_CAP + 8 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "chronicle too long"));
    }
    let mut c = crate::chronicle::Chronicle::default();
    for _ in 0..n {
        let time = read_f64(f)?;
        let mut tb = [0u8; 1];
        f.read_exact(&mut tb)?;
        let num = read_i32(f)?;
        let len = read_u32(f)? as usize;
        if len > crate::chronicle::TEXT_CAP * 2 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "chronicle text too long"));
        }
        let mut text = vec![0u8; len];
        f.read_exact(&mut text)?;
        c.entries.push(crate::chronicle::Entry {
            time,
            topic: crate::chronicle::Topic::from_u8(tb[0]),
            n: num,
            text: String::from_utf8_lossy(&text).into_owned(),
        });
    }
    Ok(c)
}

/// Write the technology state (B49/D72): the path, which advances are done, the
/// crews still out, and the furniture they have installed.
fn write_tech(w: &mut impl Write, t: &crate::tech::Tech) -> io::Result<()> {
    w.write_all(&(t.path.len() as u32).to_le_bytes())?;
    for s in &t.path {
        w.write_all(&[s.advance.index() as u8])?;
        w.write_all(&s.at.to_le_bytes())?;
    }
    let mut bits = 0u32;
    for (i, d) in t.done.iter().enumerate() {
        if *d && i < 32 {
            bits |= 1 << i;
        }
    }
    w.write_all(&bits.to_le_bytes())?;
    // The crews mid-job: a reload carries on along the same street.
    w.write_all(&(t.jobs.len() as u32).to_le_bytes())?;
    for j in &t.jobs {
        w.write_all(&[work_to_u8(j.work), j.stage as u8])?;
        w.write_all(&(j.path.len() as u32).to_le_bytes())?;
        for &(x, y) in &j.path {
            w.write_all(&x.to_le_bytes())?;
            w.write_all(&y.to_le_bytes())?;
        }
        w.write_all(&(j.i as u32).to_le_bytes())?;
        w.write_all(&j.next_at.to_le_bytes())?;
    }
    // Lamp heads that a crew has fitted (a tile not listed wears the baseline).
    w.write_all(&(t.lamps.len() as u32).to_le_bytes())?;
    for (&(x, y), &l) in &t.lamps {
        w.write_all(&x.to_le_bytes())?;
        w.write_all(&y.to_le_bytes())?;
        w.write_all(&[l])?;
    }
    write_tiles(w, &t.poles)?;
    write_tiles(w, &t.buried)?;
    write_tiles(w, &t.cells)?;
    write_tiles(w, &t.chargers)?;
    write_tiles(w, &t.farm)?;
    let towers: std::collections::BTreeSet<(i32, i32)> = t.towers.iter().copied().collect();
    let turbines: std::collections::BTreeSet<(i32, i32)> = t.turbines.iter().copied().collect();
    write_tiles(w, &towers)?;
    write_tiles(w, &turbines)?;
    w.write_all(&(t.solar.len() as u32).to_le_bytes())?;
    for (&(x, y), &s) in &t.solar {
        w.write_all(&x.to_le_bytes())?;
        w.write_all(&y.to_le_bytes())?;
        w.write_all(&[s])?;
    }
    w.write_all(&[
        t.catenary as u8,
        t.drones as u8,
        t.maglev as u8,
        t.solar_spreading as u8,
    ])?;
    Ok(())
}

fn read_tech(f: &mut impl Read, sim: &mut Sim) -> io::Result<()> {
    let n = read_u32(f)? as usize;
    if n > 64 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "tech path too long"));
    }
    let mut path = Vec::with_capacity(n);
    for _ in 0..n {
        let mut b = [0u8; 1];
        f.read_exact(&mut b)?;
        let advance = crate::tech::Advance::ALL
            .get(b[0] as usize)
            .copied()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "bad advance"))?;
        path.push(crate::tech::Stage {
            advance,
            at: read_f64(f)?,
        });
    }
    let bits = read_u32(f)?;
    let done = (0..path.len()).map(|i| bits & (1 << i) != 0).collect();
    let t = &mut sim.tech;
    t.path = path;
    t.done = done;
    let n = read_u32(f)? as usize;
    if n > 128 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "too many tech crews"));
    }
    t.jobs.clear();
    for _ in 0..n {
        let mut hdr = [0u8; 2];
        f.read_exact(&mut hdr)?;
        let work = work_from_u8(hdr[0])
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "bad tech work"))?;
        let np = read_u32(f)? as usize;
        if np > 4096 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "tech job too long"));
        }
        let mut p = Vec::with_capacity(np);
        for _ in 0..np {
            p.push((read_i32(f)?, read_i32(f)?));
        }
        let i = read_u32(f)? as usize;
        let next_at = read_f64(f)?;
        t.jobs.push(crate::tech::Job {
            work,
            stage: hdr[1] as usize,
            path: p,
            i,
            next_at,
        });
    }
    let n = read_u32(f)? as usize;
    if n > 1 << 16 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "too many lamps"));
    }
    t.lamps.clear();
    for _ in 0..n {
        let x = read_i32(f)?;
        let y = read_i32(f)?;
        let mut b = [0u8; 1];
        f.read_exact(&mut b)?;
        t.lamps.insert((x, y), b[0]);
    }
    t.poles = read_tiles(f)?;
    t.buried = read_tiles(f)?;
    t.cells = read_tiles(f)?;
    t.chargers = read_tiles(f)?;
    t.farm = read_tiles(f)?;
    t.towers = read_tiles(f)?.into_iter().collect();
    t.turbines = read_tiles(f)?.into_iter().collect();
    let n = read_u32(f)? as usize;
    if n > 1 << 16 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "too many solar roofs"));
    }
    t.solar.clear();
    for _ in 0..n {
        let x = read_i32(f)?;
        let y = read_i32(f)?;
        let mut b = [0u8; 1];
        f.read_exact(&mut b)?;
        t.solar.insert((x, y), b[0]);
    }
    let mut b = [0u8; 4];
    f.read_exact(&mut b)?;
    t.catenary = b[0] != 0;
    t.drones = b[1] != 0;
    t.maglev = b[2] != 0;
    t.solar_spreading = b[3] != 0;
    t.halted_in = None;
    Ok(())
}

fn write_tiles(w: &mut impl Write, set: &std::collections::BTreeSet<(i32, i32)>) -> io::Result<()> {
    w.write_all(&(set.len() as u32).to_le_bytes())?;
    for &(x, y) in set {
        w.write_all(&x.to_le_bytes())?;
        w.write_all(&y.to_le_bytes())?;
    }
    Ok(())
}

fn read_tiles(f: &mut impl Read) -> io::Result<std::collections::BTreeSet<(i32, i32)>> {
    let n = read_u32(f)? as usize;
    if n > 1 << 18 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "too many tech tiles"));
    }
    let mut set = std::collections::BTreeSet::new();
    for _ in 0..n {
        let x = read_i32(f)?;
        let y = read_i32(f)?;
        set.insert((x, y));
    }
    Ok(set)
}

fn work_to_u8(w: crate::tech::Work) -> u8 {
    use crate::tech::Work::*;
    match w {
        Lamp(level) => level.min(2),
        Pole => 3,
        Bury => 4,
        Tower => 5,
        Cell => 6,
        Solar => 7,
        Farm => 8,
        Turbine => 9,
        Charger => 10,
    }
}

fn work_from_u8(v: u8) -> Option<crate::tech::Work> {
    use crate::tech::Work::*;
    Some(match v {
        0..=2 => Lamp(v),
        3 => Pole,
        4 => Bury,
        5 => Tower,
        6 => Cell,
        7 => Solar,
        8 => Farm,
        9 => Turbine,
        10 => Charger,
        _ => return None,
    })
}

// --- v15: the transport block (B58) -----------------------------------

fn w_pts(w: &mut impl Write, v: &[(i32, i32)]) -> io::Result<()> {
    w.write_all(&(v.len() as u32).to_le_bytes())?;
    for &(x, y) in v {
        w.write_all(&x.to_le_bytes())?;
        w.write_all(&y.to_le_bytes())?;
    }
    Ok(())
}

fn w_zone(w: &mut impl Write, z: crate::rail::Zone) -> io::Result<()> {
    for v in [z.x, z.y, z.w, z.h] {
        w.write_all(&v.to_le_bytes())?;
    }
    Ok(())
}

fn r_pts(r: &mut impl Read) -> io::Result<Vec<(i32, i32)>> {
    let n = read_u32(r)? as usize;
    if n > 1 << 16 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "transit list too long"));
    }
    let mut v = Vec::with_capacity(n);
    for _ in 0..n {
        v.push((read_i32(r)?, read_i32(r)?));
    }
    Ok(v)
}

fn r_zone(r: &mut impl Read) -> io::Result<crate::rail::Zone> {
    Ok(crate::rail::Zone {
        x: read_i32(r)?,
        y: read_i32(r)?,
        w: read_i32(r)?,
        h: read_i32(r)?,
    })
}

fn r_byte(r: &mut impl Read) -> io::Result<u8> {
    let mut b = [0u8; 1];
    r.read_exact(&mut b)?;
    Ok(b[0])
}

fn read_transit(f: &mut impl Read, sim: &mut Sim) -> io::Result<()> {
    use crate::rail::{StationBuild, StationKind, Zone};
    use crate::transit::{Airport, AirportStage, BusLine, Harbour, Subway, SubwayStation};

    sim.rail.electrified = r_byte(f)? != 0;
    if r_byte(f)? != 0 {
        sim.rail.metro_at = Some(read_f64(f)?);
    }
    sim.rail.twin = r_pts(f)?;
    if r_byte(f)? != 0 {
        let kind = StationKind::from_index(r_byte(f)?);
        let building: Zone = r_zone(f)?;
        let n = read_u32(f)? as usize;
        if n > 4096 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "too many platforms"));
        }
        let mut platforms = Vec::with_capacity(n);
        for _ in 0..n {
            let x = read_i32(f)?;
            let y = read_i32(f)?;
            platforms.push((x, y, r_byte(f)?));
        }
        let canopy = r_pts(f)?;
        let forecourt = r_pts(f)?;
        sim.rail.station_build = Some(StationBuild {
            kind,
            building,
            platforms,
            canopy,
            forecourt,
        });
    } else {
        let _ = r_byte(f)?;
        sim.rail.station_build = None;
    }
    sim.rail.sidings = r_pts(f)?;
    let n = read_u32(f)? as usize;
    if n > 4096 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "too many wagons"));
    }
    sim.rail.parked_wagons.clear();
    for _ in 0..n {
        let x = read_i32(f)?;
        let y = read_i32(f)?;
        sim.rail.parked_wagons.push(((x, y), r_byte(f)?));
    }
    let n = read_u32(f)? as usize;
    if n > 4096 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "too many tram stops"));
    }
    sim.rail.tram_stop_sides.clear();
    for _ in 0..n {
        sim.rail.tram_stop_sides.push(r_byte(f)?);
    }
    let n = read_u32(f)? as usize;
    if n > 64 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "too many bus lines"));
    }
    sim.rail.bus_lines.clear();
    for _ in 0..n {
        let route = r_pts(f)?;
        let stops = r_pts(f)?;
        let ns = read_u32(f)? as usize;
        if ns > 4096 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "too many stop sides"));
        }
        let mut sides = Vec::with_capacity(ns);
        for _ in 0..ns {
            sides.push(r_byte(f)?);
        }
        sim.rail.bus_lines.push(BusLine {
            route,
            stops,
            sides,
        });
    }
    sim.rail.depot = if r_byte(f)? != 0 { Some(r_zone(f)?) } else { None };
    if r_byte(f)? != 0 {
        let started = read_f64(f)?;
        let line = r_pts(f)?;
        let n = read_u32(f)? as usize;
        if n > 4096 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "too many stations"));
        }
        let mut stations = Vec::with_capacity(n);
        for _ in 0..n {
            let at = (read_i32(f)?, read_i32(f)?);
            let ex = read_i32(f)?;
            let ey = read_i32(f)?;
            stations.push(SubwayStation {
                at,
                entrance: (ex, ey, r_byte(f)?),
            });
        }
        let grates = r_pts(f)?;
        sim.rail.subway = Some(Subway {
            started,
            line,
            stations,
            grates,
        });
    } else {
        sim.rail.subway = None;
    }
    if r_byte(f)? != 0 {
        let stage = AirportStage::from_index(r_byte(f)?);
        let width = r_byte(f)?.max(1) as i32;
        let runway = r_pts(f)?;
        let taxiway = r_pts(f)?;
        let terminal = r_zone(f)?;
        let apron = r_zone(f)?;
        let hangar = if r_byte(f)? != 0 { Some(r_zone(f)?) } else { None };
        let tower = if r_byte(f)? != 0 {
            let x = read_i32(f)?;
            let y = read_i32(f)?;
            Some((x, y, r_byte(f)?))
        } else {
            None
        };
        let gate = (read_i32(f)?, read_i32(f)?);
        let windsock = (read_i32(f)?, read_i32(f)?);
        let access = r_pts(f)?;
        let parked = r_pts(f)?;
        let paved_at = read_f64(f)?;
        sim.rail.airport = Some(Airport {
            stage,
            runway,
            width,
            taxiway,
            terminal,
            apron,
            hangar,
            tower,
            gate,
            windsock,
            access,
            parked,
            paved_at,
        });
    } else {
        let _ = r_byte(f)?;
        sim.rail.airport = None;
    }
    if r_byte(f)? != 0 {
        let quay = r_pts(f)?;
        let water_dir = (read_i32(f)?, read_i32(f)?);
        let n = read_u32(f)? as usize;
        if n > 64 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "too many piers"));
        }
        let mut piers = Vec::with_capacity(n);
        for _ in 0..n {
            piers.push(r_pts(f)?);
        }
        let ferry = r_pts(f)?;
        let moored = r_pts(f)?;
        let shed = (read_i32(f)?, read_i32(f)?, read_i32(f)?, read_i32(f)?);
        let cranes = r_pts(f)?;
        sim.rail.harbour = Some(Harbour {
            quay,
            water_dir,
            piers,
            ferry,
            moored,
            shed,
            cranes,
        });
    } else {
        sim.rail.harbour = None;
    }
    Ok(())
}

fn read_u16(r: &mut impl Read) -> io::Result<u16> {
    let mut b = [0u8; 2];
    r.read_exact(&mut b)?;
    Ok(u16::from_le_bytes(b))
}
fn read_u32(r: &mut impl Read) -> io::Result<u32> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b)?;
    Ok(u32::from_le_bytes(b))
}
fn read_i32(r: &mut impl Read) -> io::Result<i32> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b)?;
    Ok(i32::from_le_bytes(b))
}
fn read_u64(r: &mut impl Read) -> io::Result<u64> {
    let mut b = [0u8; 8];
    r.read_exact(&mut b)?;
    Ok(u64::from_le_bytes(b))
}
fn read_f64(r: &mut impl Read) -> io::Result<f64> {
    let mut b = [0u8; 8];
    r.read_exact(&mut b)?;
    Ok(f64::from_le_bytes(b))
}

fn prop_to_bytes(p: map::Prop) -> [u8; 4] {
    use map::Prop::*;
    match p {
        None => [0, 0, 0, 0],
        Tree { species, size, variant } => [1, species.id(), size, variant],
        Bush(a) => [2, a, 0, 0],
        Boulder(a) => [3, a, 0, 0],
        Meadow(a) => [4, a, 0, 0],
        TallGrass(a) => [5, a, 0, 0],
        Reeds(a) => [6, a, 0, 0],
        Log(a) => [7, a, 0, 0],
        Stone(a) => [8, a, 0, 0],
    }
}

fn prop_from_bytes(b: [u8; 4]) -> map::Prop {
    use map::Prop::*;
    match b[0] {
        1 => Tree { species: map::TreeSpecies::from_id(b[1]), size: b[2], variant: b[3] },
        2 => Bush(b[1]),
        3 => Boulder(b[1]),
        4 => Meadow(b[1]),
        5 => TallGrass(b[1]),
        6 => Reeds(b[1]),
        7 => Log(b[1]),
        8 => Stone(b[1]),
        _ => None,
    }
}

#[allow(dead_code)]
fn unused(_: &TileMap) {}

#[cfg(test)]
mod roundtrip_tests {
    use super::*;
    use crate::{Pace, Sim};

    /// Save -> load -> save is byte-identical on several seeds and ages, and a
    /// truncated or corrupt file is an error, never a panic.
    #[test]
    fn save_round_trips_and_bad_files_error() {
        let dir = std::env::temp_dir().join("citylive-bugsweep-save");
        std::fs::create_dir_all(&dir).unwrap();
        for (seed, days) in [(1u64, 0.05f64), (2, 1.0), (3, 8.0), (4, 30.0), (5, 60.0)] {
            let mut a = Sim::new(seed, Pace::Month, 41, 41);
            a.run_to(days * 86400.0);
            let p1 = dir.join(format!("a{seed}.sav"));
            let p2 = dir.join(format!("b{seed}.sav"));
            a.save(&p1).unwrap();
            let b = Sim::load(&p1).unwrap();
            b.save(&p2).unwrap();
            let x = std::fs::read(&p1).unwrap();
            let y = std::fs::read(&p2).unwrap();
            assert_eq!(x.len(), y.len(), "seed {seed} day {days}: size differs");
            assert!(x == y, "seed {seed} day {days}: re-save differs");
            // A corrupt header (absurd map size, absurd rail counts) is an
            // error, not a huge allocation or a multi-gigabyte map.
            let bad = dir.join("bad.sav");
            let mut huge = x.clone();
            huge[25..29].copy_from_slice(&i32::MAX.to_le_bytes());
            std::fs::write(&bad, &huge).unwrap();
            assert!(Sim::load(&bad).is_err(), "seed {seed}: absurd width loaded");
            // Truncations must be errors.
            let bad = dir.join("bad.sav");
            for cut in [0, 3, 8, 20, x.len() / 3, x.len() / 2, x.len() - 1] {
                std::fs::write(&bad, &x[..cut]).unwrap();
                assert!(Sim::load(&bad).is_err(), "seed {seed}: cut {cut} loaded");
            }
        }
    }
}

/// B46d: the loaded world was generated with landforms, but a city that stood
/// before they existed may already hold ground inside a massif's reach (a road,
/// a rail, a lot, a field, a building site, a change to the land). That massif
/// is given up (no sprite, smoothed on the drawing map, its ground restored), so
/// nothing changes for that city. Decided from the loaded state alone, so it is
/// the same on every load of a save.
///
/// Only a save older than v12 (written before landforms existed) is judged by the
/// whole skirt; a newer one was grown beside its massif, which the live city kept,
/// so only ground inside the footprint gives it up (a reload used to drop a
/// massif whenever the town had reached its skirt, so every restart changed the
/// land).
fn release_landforms(sim: &mut Sim, seed: u64, w: i32, h: i32, biome: map::Biome, pre_landform_save: bool) {
    if sim.map.landforms.is_empty() {
        return;
    }
    let mut occ: std::collections::HashSet<(i32, i32)> = std::collections::HashSet::new();
    // A tile the sim rewrote but left bare rock (a grade mark on the rim of a
    // landform) has not built on the massif: only a pre-landform save counts it.
    occ.extend(
        sim.overrides
            .iter()
            .filter(|(_, t)| pre_landform_save || t.kind != map::Kind::Rock)
            .map(|(k, _)| *k),
    );
    occ.extend(sim.changes.iter().copied());
    for l in &sim.lots {
        for y in l.y..l.y + l.h {
            for x in l.x..l.x + l.w {
                occ.insert((x, y));
            }
        }
    }
    for f in &sim.fields {
        for y in f.y..f.y + f.h {
            for x in f.x..f.x + f.w {
                occ.insert((x, y));
            }
        }
    }
    occ.extend(sim.rail.line.iter().copied());
    occ.extend(sim.rail.tram_line.iter().copied());
    occ.extend(sim.rail.tram_stops.iter().copied());
    occ.extend(sim.rail.containers.iter().map(|c| (c.0, c.1)));
    occ.extend(sim.rail.station);
    if let Some(p) = sim.rail.station_platform {
        occ.insert((p.0, p.1));
    }
    for z in [sim.rail.yard, sim.rail.marina, sim.rail.port].into_iter().flatten() {
        for y in z.y..z.y + z.h {
            for x in z.x..z.x + z.w {
                occ.insert((x, y));
            }
        }
    }
    if occ.is_empty() {
        return;
    }
    let Ok(original) = map::try_generate_unmarked(seed, w, h, Some(biome)) else {
        return;
    };
    let overrides = &sim.overrides;
    map::massif::release_occupied(
        &mut sim.map,
        &original,
        &|x, y| occ.contains(&(x, y)),
        &|x, y| overrides.contains_key(&(x, y)),
        pre_landform_save,
    );
}

#[cfg(test)]
mod landform_tests {
    use crate::{Pace, Sim};

    /// B46d: a city that already stands where a massif would be keeps it as plain
    /// terrain: no sprite, ground restored, the road still there; a save without
    /// anything built there keeps the landform.
    #[test]
    fn a_road_through_a_massif_gives_up_its_landform_on_load() {
        let dir = std::env::temp_dir().join("citylive-b46d-save");
        std::fs::create_dir_all(&dir).unwrap();
        let mut tried = 0;
        for seed in 1u64..40 {
            let mut a = Sim::new(seed, Pace::Month, 107, 107);
            if a.map.landforms.is_empty() {
                continue;
            }
            tried += 1;
            let before = a.map.landforms.len();
            let m = a.map.landforms[0];
            // Untouched: the save loads with its landforms.
            let p0 = dir.join(format!("clean{seed}.sav"));
            a.save(&p0).unwrap();
            let clean = Sim::load(&p0).unwrap();
            assert_eq!(clean.map.landforms.len(), before, "seed {seed}: a clean save lost a landform");
            // A road through the middle of it.
            let (x, y) = (m.cx as i32, m.cy as i32);
            let mut t = a.map.at(x, y);
            t.kind = map::Kind::Road;
            a.map.set(x, y, t);
            a.overrides.insert((x, y), t);
            let p1 = dir.join(format!("road{seed}.sav"));
            a.save(&p1).unwrap();
            let b = Sim::load(&p1).unwrap();
            assert!(
                b.map.landforms.iter().all(|l| (l.cx - m.cx).abs() > 0.1 || (l.cy - m.cy).abs() > 0.1),
                "seed {seed}: a landform still covers the road"
            );
            assert!(b.map.hills.iter().any(|l| l.cx == m.cx && l.cy == m.cy), "seed {seed}: not smoothed");
            assert_eq!(b.map.at(x, y).kind, map::Kind::Road, "seed {seed}: the road is gone");
            // Its footprint is no longer marked rock beyond the road tile.
            let original = map::try_generate_unmarked(seed, 107, 107, Some(b.map.biome)).unwrap();
            for dy in -2..=2 {
                for dx in -2..=2 {
                    if (dx, dy) != (0, 0) {
                        assert_eq!(
                            b.map.at(x + dx, y + dy).kind,
                            original.at(x + dx, y + dy).kind,
                            "seed {seed}: ground at {},{} was not restored",
                            x + dx,
                            y + dy
                        );
                    }
                }
            }
            if tried >= 2 {
                break;
            }
        }
        assert!(tried > 0, "no seed carried a landform");
    }

    /// A city that grew up beside its massif (in the skirt, outside the footprint)
    /// keeps the massif across a reload, as the live city did: a restart must not
    /// change the land.
    #[test]
    fn a_town_in_the_skirt_keeps_its_landform_across_a_reload() {
        let dir = std::env::temp_dir().join("citylive-b93-skirt-save");
        std::fs::create_dir_all(&dir).unwrap();
        let mut tried = 0;
        let mut checked = 0;
        for seed in 1u64..40 {
            let mut a = Sim::new(seed, Pace::Month, 107, 107);
            if a.map.landforms.is_empty() {
                continue;
            }
            tried += 1;
            let m = a.map.landforms[0];
            // A tile between the footprint and the edge of the skirt, and a grade
            // mark on a rock tile of the footprint (a rim tile the town graded
            // beside): neither is a building on the massif.
            let d = (m.radius + 0.5 + map::massif::reach(&m)) * 0.5;
            let (x, y) = ((m.cx + d) as i32, m.cy as i32);
            let (rx, ry) = (m.cx as i32, m.cy as i32);
            if !a.map.in_bounds(x, y) {
                continue;
            }
            // A neighbouring massif these tiles happen to touch may go; this one may not.
            let others = a.map.landforms.iter().skip(1).any(|o| {
                [(x, y), (rx, ry)].iter().any(|&(tx, ty)| {
                    let (dx, dy) = (tx as f32 + 0.5 - o.cx, ty as f32 + 0.5 - o.cy);
                    dx * dx + dy * dy <= (o.radius + 0.5).powi(2)
                })
            });
            if others {
                continue;
            }
            let mut t = a.map.at(x, y);
            t.kind = map::Kind::Road;
            a.map.set(x, y, t);
            a.overrides.insert((x, y), t);
            let mut r = a.map.at(rx, ry);
            r.kind = map::Kind::Rock;
            r.aux = 84;
            a.map.set(rx, ry, r);
            a.overrides.insert((rx, ry), r);
            let p = dir.join(format!("skirt{seed}.sav"));
            a.save(&p).unwrap();
            let b = Sim::load(&p).unwrap();
            checked += 1;
            assert!(
                b.map.landforms.iter().any(|l| l.cx == m.cx && l.cy == m.cy),
                "seed {seed}: the skirt road or the grade mark cost the massif"
            );
            if checked >= 3 {
                break;
            }
        }
        assert!(checked > 0, "no seed could be tested ({tried} carried a landform)");
    }
}


// --- B52: the economy, the government, the pollution, the incidents (v14) ----

fn put_f32(w: &mut impl Write, v: f32) -> io::Result<()> {
    w.write_all(&v.to_le_bytes())
}

fn get_f32(r: &mut impl Read) -> io::Result<f32> {
    Ok(f32::from_le_bytes(read_u32(r)?.to_le_bytes()))
}

fn get_u8(r: &mut impl Read) -> io::Result<u8> {
    let mut b = [0u8; 1];
    r.read_exact(&mut b)?;
    Ok(b[0])
}

fn write_civics(w: &mut impl Write, c: &crate::civics::Civics) -> io::Result<()> {
    put_f32(w, c.mood)?;
    put_f32(w, c.vacancy)?;
    put_f32(w, c.shock)?;
    put_f32(w, c.shock_left)?;
    w.write_all(&[c.phase])?;
    w.write_all(&(c.companies.len() as u32).to_le_bytes())?;
    for co in &c.companies {
        w.write_all(&[co.a, co.b, co.kind, co.floors, co.closed as u8])?;
        w.write_all(&co.hq.0.to_le_bytes())?;
        w.write_all(&co.hq.1.to_le_bytes())?;
        put_f32(w, co.health)?;
        w.write_all(&co.closed_day.to_le_bytes())?;
    }
    put_f32(w, c.smog)?;
    w.write_all(&[c.cleanups])?;
    w.write_all(&c.cleanup_day.to_le_bytes())?;
    match c.mayor {
        Some(m) => {
            w.write_all(&[1, m.slate])?;
            w.write_all(&m.id.to_le_bytes())?;
            w.write_all(&m.since_day.to_le_bytes())?;
        }
        None => w.write_all(&[0; 10])?,
    }
    match c.election {
        Some(e) => {
            w.write_all(&[1, e.measure.map_or(255, |m| m.id()), e.told, e.cand[0].1, e.cand[1].1])?;
            w.write_all(&e.day.to_le_bytes())?;
            w.write_all(&e.cand[0].0.to_le_bytes())?;
            w.write_all(&e.cand[1].0.to_le_bytes())?;
        }
        None => w.write_all(&[0; 17])?,
    }
    w.write_all(&c.next_election.to_le_bytes())?;
    w.write_all(&[c.elections_held, c.parks_owed, c.fires_seen, c.blackouts_seen])?;
    w.write_all(&c.approved.to_le_bytes())?;
    w.write_all(&c.rejected.to_le_bytes())?;
    match c.fire {
        Some(f) => {
            w.write_all(&[1, f.dispatched as u8, f.told])?;
            w.write_all(&f.x.to_le_bytes())?;
            w.write_all(&f.y.to_le_bytes())?;
            w.write_all(&f.t0.to_le_bytes())?;
        }
        None => w.write_all(&[0; 19])?,
    }
    match c.blackout {
        Some(b) => {
            w.write_all(&[1, b.told])?;
            w.write_all(&b.cx.to_le_bytes())?;
            w.write_all(&b.cy.to_le_bytes())?;
            w.write_all(&b.r.to_le_bytes())?;
            w.write_all(&b.t0.to_le_bytes())?;
            w.write_all(&b.dur.to_le_bytes())?;
        }
        None => w.write_all(&[0; 30])?,
    }
    w.write_all(&c.next_fire.to_le_bytes())?;
    w.write_all(&c.next_blackout.to_le_bytes())?;
    w.write_all(&c.day.to_le_bytes())?;
    Ok(())
}

fn read_civics(f: &mut impl Read) -> io::Result<crate::civics::Civics> {
    use crate::civics::*;
    let bad = |m: &'static str| io::Error::new(io::ErrorKind::InvalidData, m);
    let mut c = Civics::default();
    c.mood = get_f32(f)?;
    c.vacancy = get_f32(f)?;
    c.shock = get_f32(f)?;
    c.shock_left = get_f32(f)?;
    c.phase = get_u8(f)?;
    let n = read_u32(f)? as usize;
    if n > 64 {
        return Err(bad("too many companies"));
    }
    for _ in 0..n {
        let mut b = [0u8; 5];
        f.read_exact(&mut b)?;
        let hq = (read_i32(f)?, read_i32(f)?);
        let health = get_f32(f)?;
        let closed_day = read_i32(f)?;
        c.companies.push(Company { a: b[0], b: b[1], kind: b[2], floors: b[3], closed: b[4] != 0, hq, health, closed_day });
    }
    c.smog = get_f32(f)?;
    c.cleanups = get_u8(f)?;
    c.cleanup_day = read_i32(f)?;
    let mut m = [0u8; 2];
    f.read_exact(&mut m)?;
    let (id, since) = (read_u32(f)?, read_i32(f)?);
    c.mayor = (m[0] != 0).then_some(Mayor { id, slate: m[1], since_day: since });
    let mut e = [0u8; 5];
    f.read_exact(&mut e)?;
    let (day, c0, c1) = (read_i32(f)?, read_u32(f)?, read_u32(f)?);
    c.election = (e[0] != 0).then(|| Election {
        day,
        cand: [(c0, e[3]), (c1, e[4])],
        measure: (e[1] != 255).then(|| Project::from_id(e[1])),
        told: e[2],
    });
    c.next_election = read_i32(f)?;
    let mut k = [0u8; 4];
    f.read_exact(&mut k)?;
    c.elections_held = k[0];
    c.parks_owed = k[1];
    c.fires_seen = k[2];
    c.blackouts_seen = k[3];
    c.approved = read_u32(f)?;
    c.rejected = read_u32(f)?;
    let mut fl = [0u8; 3];
    f.read_exact(&mut fl)?;
    let (x, y, t0) = (read_i32(f)?, read_i32(f)?, read_f64(f)?);
    c.fire = (fl[0] != 0).then_some(Fire { x, y, t0, dispatched: fl[1] != 0, told: fl[2] });
    let mut bl = [0u8; 2];
    f.read_exact(&mut bl)?;
    let (cx, cy, r) = (read_i32(f)?, read_i32(f)?, read_i32(f)?);
    let (t0, dur) = (read_f64(f)?, read_f64(f)?);
    c.blackout = (bl[0] != 0).then_some(Blackout { cx, cy, r, t0, dur, told: bl[1] });
    c.next_fire = read_i32(f)?;
    c.next_blackout = read_i32(f)?;
    c.day = read_i32(f)?;
    Ok(c)
}

#[cfg(test)]
mod reload_growth_tests {
    use crate::{Pace, Sim};

    /// the owner (2026-11-13): "I don't think the wallpaper is actually evolving over
    /// time". A loaded save struck the fresh party's camp but never marked the
    /// settlers arrived, and `decide` waits for that: a city loaded from a save
    /// never built again. A reloaded town must keep growing like a live one.
    #[test]
    fn a_reloaded_town_keeps_growing() {
        let dir = std::env::temp_dir().join("citylive-reload-growth");
        std::fs::create_dir_all(&dir).unwrap();
        let mut a = Sim::new(7, Pace::Month, 80, 80);
        a.run_to(2.0 * 86400.0);
        let lots_at_save = a.lots.len();
        assert!(lots_at_save > 4, "the town should have grown in two days ({lots_at_save} lots)");
        let p = dir.join("reload.sav");
        a.save(&p).unwrap();
        let mut b = Sim::load(&p).unwrap();
        assert!(b.settlers.arrived, "a loaded town is a settled one");
        b.run_to(5.0 * 86400.0);
        assert!(
            b.lots.len() > lots_at_save + 5,
            "a reloaded town stopped growing: {} lots at save, {} three days later",
            lots_at_save,
            b.lots.len()
        );
        let _ = std::fs::remove_file(&p);
    }

    /// v17: a town reloaded once it has a street grid keeps growing it about as
    /// fast as a town that never stopped (the grid planner state is saved).
    #[test]
    fn a_reloaded_town_keeps_its_grid() {
        let dir = std::env::temp_dir().join("citylive-reload-grid");
        std::fs::create_dir_all(&dir).unwrap();
        let mut live = Sim::new(7, Pace::Month, 120, 120);
        live.run_to(6.0 * 86400.0);
        assert!(live.blocks.is_ready(), "the town should have a grid by day 6");
        let p = dir.join("grid.sav");
        live.save(&p).unwrap();
        let mut b = Sim::load(&p).unwrap();
        assert!(b.blocks.is_ready(), "the grid alignment is lost on load");
        assert_eq!(b.blocks.open, live.blocks.open, "the opened cells are lost on load");
        live.run_to(10.0 * 86400.0);
        b.run_to(10.0 * 86400.0);
        let (nl, nb) = (live.lots.len() as f64, b.lots.len() as f64);
        assert!(nb >= nl * 0.8, "a reloaded town fell behind: {nb} lots against {nl}");
        let _ = std::fs::remove_file(&p);
    }

    fn fixture(name: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../handbacks/fixtures").join(name)
    }

    fn lane_count(s: &Sim) -> usize {
        s.map.tiles.iter().filter(|t| t.kind.is_lane()).count()
    }

    /// B88: a city saved and reloaded every few hours grows like one that never
    /// stopped: population, buildings and road tiles stay within 10 % at every
    /// day from the settlement on. (On a map without a massif in the town's way
    /// the two are byte-identical; `examples/reloadparity` checks that. A
    /// reload gives up a massif the town stands in, B46d, which moves a few
    /// tiles of terrain and is the only difference left.)
    #[test]
    fn reload_every_few_hours_is_the_same_city() {
        let dir = std::env::temp_dir().join("citylive-reload-parity");
        std::fs::create_dir_all(&dir).unwrap();
        let pb = dir.join("b.sav");
        let mut live = Sim::new(7, Pace::Month, 120, 120);
        let mut twin = Sim::new(7, Pace::Month, 120, 120);
        let mut t = 0.0;
        let mut next_day = 1.0;
        while t < 8.0 * 86400.0 {
            t += 3.0 * 3600.0;
            live.run_to(t);
            twin.run_to(t);
            twin.save(&pb).unwrap();
            twin = Sim::load(&pb).unwrap();
            if t / 86400.0 >= next_day {
                next_day += 1.0;
                for (what, a, b) in [
                    ("population", live.population() as f64, twin.population() as f64),
                    ("buildings", live.lots.len() as f64, twin.lots.len() as f64),
                    ("road tiles", lane_count(&live) as f64, lane_count(&twin) as f64),
                ] {
                    assert!(
                        (a - b).abs() <= 0.1 * a.max(10.0),
                        "{what} {b} against {a} at day {:.1}",
                        t / 86400.0
                    );
                }
            }
        }
        assert!(live.lots.len() > 60, "the town should have grown ({} lots)", live.lots.len());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// B88: a reload during the settlement (the camp is struck, the party is
    /// not saved) still ends up a town of the same size.
    #[test]
    fn reload_from_the_first_hour_is_within_ten_percent() {
        let dir = std::env::temp_dir().join("citylive-reload-early");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("c.sav");
        let mut live = Sim::new(7, Pace::Month, 120, 120);
        let mut twin = Sim::new(7, Pace::Month, 120, 120);
        let mut t = 0.0;
        while t < 8.0 * 86400.0 {
            t += 3600.0;
            live.run_to(t);
            twin.run_to(t);
            twin.save(&p).unwrap();
            twin = Sim::load(&p).unwrap();
        }
        let (a, b) = (live.population() as f64, twin.population() as f64);
        assert!((a - b).abs() <= 0.1 * a, "population {b} against {a}");
        let (a, b) = (live.lots.len() as f64, twin.lots.len() as f64);
        assert!((a - b).abs() <= 0.1 * a, "buildings {b} against {a}");
        let (a, b) = (lane_count(&live) as f64, lane_count(&twin) as f64);
        assert!((a - b).abs() <= 0.1 * a, "road tiles {b} against {a}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// B88: the owner's old save was 14 lots and no street at all. It must lay
    /// streets and grow into a town within ten sim days. (The fixture is also a
    /// v16 file, so this is an old-version load.)
    #[test]
    fn a_save_with_houses_and_no_lanes_lays_streets() {
        let mut s = Sim::load(&fixture("willowcote-0lanes.sav")).expect("the fixture loads");
        assert_eq!(lane_count(&s), 0, "the fixture has no street");
        let (pop0, lots0) = (s.population(), s.lots.len());
        let t0 = s.time;
        s.run_to(t0 + 10.0 * 86400.0);
        assert!(lane_count(&s) > 30, "no streets were laid: {} lane tiles", lane_count(&s));
        assert!(s.population() > pop0 + 100, "the town did not grow: {} people from {pop0}", s.population());
        assert!(s.lots.len() > lots0 + 20, "{} lots from {lots0}", s.lots.len());
    }

    /// Older versions keep loading: a v17 file is a v18 file without the growth
    /// block, and loads as the city it was (with the planner state it lacked
    /// rebuilt).
    #[test]
    fn a_v17_save_still_loads() {
        let dir = std::env::temp_dir().join("citylive-v17-load");
        std::fs::create_dir_all(&dir).unwrap();
        let mut a = Sim::new(7, Pace::Month, 120, 120);
        a.run_to(4.0 * 86400.0);
        let mut bytes = Vec::new();
        super::write_growth(&mut bytes, &a).unwrap();
        let mut full = Vec::new();
        a.write_to(&mut full).unwrap();
        full.truncate(full.len() - bytes.len());
        full[4..8].copy_from_slice(&17u32.to_le_bytes());
        let p = dir.join("v17.sav");
        std::fs::write(&p, &full).unwrap();
        let mut b = Sim::load(&p).expect("a v17 save loads");
        assert_eq!(b.lots.len(), a.lots.len());
        b.run_to(6.0 * 86400.0);
        assert!(b.lots.len() >= a.lots.len(), "the v17 city kept growing");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The old fixture format (v16) loads too.
    #[test]
    fn the_v16_fixture_loads() {
        let s = Sim::load(&fixture("willowcote-0lanes.sav")).unwrap();
        assert!(s.lots.len() >= 14);
    }
}
