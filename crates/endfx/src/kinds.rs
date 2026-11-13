//! B66: what the sim's city looks like to the renderer — in one place.
//!
//! The mapping from a sim lot (`sim::LotKind`, `sim::landmarks::LandmarkKind`)
//! to the renderer's object (`render::ObjKind`, `render::city::LandmarkKind`)
//! used to be copied into `present`, `present_x11`, `present_gnome`, the spike
//! and a dozen `tools/*` shots. Every new kind broke a tool at merge time, so
//! the table lives here once: this crate already depends on both `sim` and
//! `render`, and everything that needs the table can depend on this crate.
//!
//! Nothing here draws or decides anything; it is the translation, and only the
//! translation.

use render::CityObj;
use render::ObjKind;
use render::city::LandmarkKind as RK;
use sim::LotKind;
use sim::landmarks::LandmarkKind as SK;

/// The renderer's kind for a sim lot kind.
pub fn lot(k: LotKind) -> ObjKind {
    match k {
        LotKind::House => ObjKind::House,
        LotKind::Church => ObjKind::Church,
        LotKind::Store => ObjKind::Store,
        LotKind::Shop => ObjKind::Shop,
        LotKind::Barn => ObjKind::Barn,
        // B80: the farmhouse is an ordinary house to the renderer; its lot style
        // (6) asks the kit for the farmhouse.
        LotKind::Farmhouse => ObjKind::House,
        LotKind::Townhouse => ObjKind::Townhouse,
        LotKind::Apartment => ObjKind::Apartment,
        LotKind::Office => ObjKind::Office,
        LotKind::Tower => ObjKind::Tower,
        LotKind::Industry => ObjKind::Industry,
        LotKind::Park => ObjKind::Park,
        LotKind::Cemetery => ObjKind::Cemetery,
        LotKind::Hall => ObjKind::Hall,
        LotKind::Landmark(k) => ObjKind::Landmark(landmark(k)),
        LotKind::LogCabin => ObjKind::LogCabin,
        LotKind::CarPark => ObjKind::CarPark,
        LotKind::CarParkDeck => ObjKind::CarParkDeck,
        LotKind::GasStation => ObjKind::GasStation,
        LotKind::ChargingHub => ObjKind::ChargingHub,
        LotKind::DriveThrough => ObjKind::DriveThrough,
        LotKind::CarWash => ObjKind::CarWash,
        LotKind::Dealership => ObjKind::Dealership,
        LotKind::Motel => ObjKind::Motel,
    }
}

/// The renderer's kind for a sim landmark kind.
pub fn landmark(k: SK) -> RK {
    match k {
        SK::Cathedral => RK::Cathedral,
        SK::Station => RK::Station,
        SK::School => RK::School,
        SK::Library => RK::Library,
        SK::FireStation => RK::FireStation,
        SK::Hospital => RK::Hospital,
        SK::Museum => RK::Museum,
        SK::Stadium => RK::Stadium,
        SK::SignatureTower => RK::SignatureTower,
        SK::Lighthouse => RK::Lighthouse,
        SK::Port => RK::Port,
        SK::FerrisWheel => RK::FerrisWheel,
        SK::Plaza => RK::Plaza,
        SK::Zoo => RK::Zoo,
        SK::Castle => RK::Castle,
        SK::Courthouse => RK::Courthouse,
        SK::PoliceStation => RK::PoliceStation,
        SK::PostOffice => RK::PostOffice,
    }
}

/// B78: what one lot wears, in the renderer's own shape.
pub fn wear_look(w: &sim::ageing::Wear) -> render::Wear {
    render::Wear {
        grime: w.grime,
        wet: w.wet,
        art: w.art,
        art_seed: w.art_seed,
        boarded: w.boarded,
        shut: w.shut,
        scaffold: w.scaffold,
        repaint: w.repaint,
        fascia: w.fascia,
    }
}

/// One sim lot as a renderer object, at sim time `t` (seconds).
pub fn object(l: &sim::Lot, t: f64) -> CityObj {
    CityObj {
        x: l.x,
        y: l.y,
        w: l.w,
        h: l.h,
        level: l.level,
        kind: lot(l.kind),
        storeys: l.storeys,
        style: l.style,
        stage: render::BuildStage::from_index(l.stage.index() as u8),
        seed: l.seed,
        front: l.front,
        t: t as f32,
        kit: 0,
        wear: render::Wear::default(),
    }
}

/// Every finished-and-unfinished lot of a sim as renderer objects, in lot
/// order (the order the ending's ruin codes are indexed by).
pub fn objects(s: &sim::Sim) -> Vec<CityObj> {
    s.lots
        .iter()
        .enumerate()
        .map(|(i, l)| {
            let mut o = object(l, s.time);
            // B78: the years on this building (the tick's own answer; empty
            // before the first tick, which is a city of new buildings).
            if let Some(w) = s.lot_wear(i) {
                o.wear = wear_look(w);
            }
            o
        })
        .collect()
}


/// B72 (D78): which of every lot's windows are lit at `hour`, keyed by the lot's
/// anchor tile — the plan the renderer's window pass reads.
pub fn window_plan(s: &sim::Sim, hour: f32) -> Vec<(i32, i32, u64)> {
    let bits = s.window_plan(hour as f64, s.is_weekend());
    s.lots
        .iter()
        .enumerate()
        .map(|(i, l)| (l.x, l.y, bits.get(i).copied().unwrap_or(0)))
        .collect()
}

/// B59/B74: the cars a sim is showing parked (bays, kerb rows, driveways, a
/// drive-through queue, a dealership's stock), as traffic bodies the scene
/// draws. The live wallpaper has its own copy of this (B66's table predates
/// this module); stills had **none** until B74's parking pass needed them, so a
/// car park in a handback shot was always empty. B75 added `style`/`signal` to
/// the body; a parked car has neither (its lights are off and no signal shows).
pub fn parked_bodies(s: &sim::Sim) -> Vec<render::traffic::Body> {
    use sim::traffic::VehicleKind;
    s.car_views()
        .into_iter()
        .map(|c| render::traffic::Body {
            x: c.x,
            y: c.y,
            dir: c.dir,
            kind: match c.kind {
                VehicleKind::Cart => render::traffic::Kind::Cart,
                VehicleKind::Car => render::traffic::Kind::Car,
                VehicleKind::Van => render::traffic::Kind::Van,
                VehicleKind::Taxi => render::traffic::Kind::Taxi,
                VehicleKind::Bus => render::traffic::Kind::Bus,
                VehicleKind::Truck => render::traffic::Kind::Truck,
                VehicleKind::Ev => render::traffic::Kind::Ev,
                VehicleKind::Emergency => render::traffic::Kind::Emergency,
                VehicleKind::Bike => render::traffic::Kind::Bike,
            },
            variant: c.variant,
            style: render::traffic::Style::from_u8(c.style),
            // A parked car keeps its lights off except the brake lights of a
            // queue — the light model does the rest (D74). No indicator shows.
            lights: false,
            blink: 0.0,
            lift: c.lift,
            brake: c.braking,
            signal: 0,
        })
        .collect()
}
