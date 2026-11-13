//! The layout of the car-culture places (B59b): where the bays, the pumps, the
//! lane and the building of a plot are, in plot-local coordinates.
//!
//! The sim parks cars by it and the renderer paints by it, so the bay a car
//! stands in and the lines painted round it are one definition (the two crates
//! cannot see each other; both see this one).
//!
//! Local frame: `t` runs along the frontage, `d` runs inward from the kerb, both
//! in tiles. The plot is `al` long along the street and `dl` deep.

/// Frontage one bay takes, in tiles: a tile is about twelve metres, a bay about
/// four, so three cars to the tile's edge.
pub const BAY_PITCH: f32 = 1.0 / 3.0;
/// How deep a bay is: a car is 0.39 of a tile long.
pub const BAY_DEPTH: f32 = 0.45;
/// Height of one deck of the multi-storey car park, in screen pixels.
pub const DECK_PX: f32 = 14.0;
/// Decks in the multi-storey car park.
pub const DECKS: u8 = 3;

/// How an apartment block parks its residents' cars (B74 note).
///
/// A flat block's own yard is a strip too thin for a car, so parking is one of
/// two things, decided by how tall the block is (a field both the sim and the
/// renderer carry): a **kerbside row** of marked bays along the block's own
/// street for a low-rise block, or an **undercroft** with its mouth on the
/// frontage for a tall one. Both open straight onto the road, so the parking is
/// always connected; neither takes a tile from anything else.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AptParking {
    /// Marked bays along the kerb in front of the block.
    Kerbside,
    /// A garage under the building, its mouth on the street side.
    Undercroft,
}

/// Which side of a plot is its street: 0 = +x, 1 = +y, 2 = -x, 3 = -y. `front`
/// is the side the lot was staked facing; if that side has nothing to park on
/// (a courtyard, or a street one tile further out), the other three are tried in
/// order, so a kerb row always lands on a street. `streetlike` says whether a
/// tile is a lane or a pavement (the sim and the renderer each pass their own).
pub fn street_side(
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    front: u8,
    streetlike: impl Fn(i32, i32) -> bool,
) -> u8 {
    for k in 0..4u8 {
        let side = (front + k) % 4;
        // The whole ring on that side, corners included: a lane that touches
        // only the corner still gives the block a street to face.
        let found = match side {
            0 => (y - 1..=y + h).any(|ty| streetlike(x + w, ty)),
            2 => (y - 1..=y + h).any(|ty| streetlike(x - 1, ty)),
            1 => (x - 1..=x + w).any(|tx| streetlike(tx, y + h)),
            _ => (x - 1..=x + w).any(|tx| streetlike(tx, y - 1)),
        };
        if found {
            return side;
        }
    }
    front % 4
}

/// The kerb row of an apartment block: one bay per street tile beside the block,
/// taken on the first side (starting at `front`) that has any, and falling back
/// to a corner when that is all the block has. `streetlike` is the caller's test
/// (the sim and the renderer each have their own map type). The bay is returned
/// in **tile space** (its centre and the nose of the car), because a corner bay
/// cannot be written as a point on one side's frame.
pub fn apt_kerb_bays(
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    front: u8,
    streetlike: impl Fn(i32, i32) -> bool,
) -> Vec<(f32, f32, Nose)> {
    for k in 0..4u8 {
        let side = (front + k) % 4;
        let f = Frame::new(x, y, w, h, side);
        let (al, _) = f.size();
        let mut kept: Vec<(f32, f32, Nose)> = Vec::new();
        let n = (al * 4.0).floor().max(1.0) as i32;
        for i in 0..n {
            let t = (i as f32 + 0.5) / n as f32 * al;
            let (px, py) = f.at(t, -0.30);
            let (tx, ty) = (px.floor() as i32, py.floor() as i32);
            if streetlike(tx, ty) {
                // Stand the car in the street tile, a step in from its kerb, so
                // it is never drawn half inside the plot or half in the next
                // tile's lane markings.
                let (mut bx, mut by) = (tx as f32 + 0.5, ty as f32 + 0.5);
                let (cx, cy) = (x as f32 + w as f32 * 0.5, y as f32 + h as f32 * 0.5);
                let (dx, dy) = (cx - bx, cy - by);
                let d = (dx * dx + dy * dy).sqrt().max(0.001);
                bx += dx / d * 0.30;
                by += dy / d * 0.30;
                kept.push((bx, by, Nose::Fwd));
            }
        }
        if !kept.is_empty() {
            return kept;
        }
    }
    // The block's only street is a corner (a terrace end): park on it.
    let mut ring: Vec<(i32, i32)> = Vec::new();
    for tx in x..x + w {
        ring.push((tx, y - 1));
        ring.push((tx, y + h));
    }
    for ty in y..y + h {
        ring.push((x - 1, ty));
        ring.push((x + w, ty));
    }
    for (cx, cy) in [(x - 1, y - 1), (x + w, y - 1), (x - 1, y + h), (x + w, y + h)] {
        ring.push((cx, cy));
    }
    let (lx, ly) = (x as f32 + w as f32 * 0.5, y as f32 + h as f32 * 0.5);
    let mut out = Vec::new();
    for (tx, ty) in ring {
        if !streetlike(tx, ty) {
            continue;
        }
        let (mut bx, mut by) = (tx as f32 + 0.5, ty as f32 + 0.5);
        let (dx, dy) = (lx - bx, ly - by);
        let d = (dx * dx + dy * dy).sqrt().max(0.001);
        bx += dx / d * 0.30;
        by += dy / d * 0.30;
        out.push((bx, by, Nose::Fwd));
        if out.len() >= 2 {
            break;
        }
    }
    out
}

/// Which parking a block of this many storeys gets. Five storeys is the break:
/// below it there is room to step out of the car, above it the block is dense
/// enough that the city digs down.
pub fn apt_style(storeys: u8) -> AptParking {
    if storeys >= 5 {
        AptParking::Undercroft
    } else {
        AptParking::Kerbside
    }
}

/// Bays of one apartment block, in the plot's own frame. Kerbside bays sit just
/// **outside** the kerb (`d` negative) on the lane the plot fronts; undercroft
/// bays sit just inside the frontage on the ramp into the garage. The sim parks
/// cars here and the renderer marks the kerb row with the same numbers.
pub fn apt_bays(al: f32, dl: f32, front: u8, style: AptParking) -> Vec<LBay> {
    let mut out = Vec::new();
    match style {
        AptParking::Kerbside => {
            // A bay every half tile along the frontage, from the first to the
            // last, but never the whole frontage: the drive to the door has to
            // stay clear (the brief: the entrance is part of the access rule).
            let pitch = 0.5f32;
            let n = ((al - 0.5) / pitch).floor().max(1.0) as i32;
            let start = (al - (n as f32 - 1.0) * pitch) * 0.5;
            for i in 0..n {
                out.push(LBay {
                    t: start + i as f32 * pitch,
                    d: -0.30,
                    nose: Nose::Fwd,
                    level: 0,
                });
            }
        }
        AptParking::Undercroft => {
            // The mouth: two cars on the ramp, nose in, at the middle of the
            // frontage, plus the mouth's tile itself (the lintel the renderer
            // paints over them).
            let mid = al * 0.5;
            let _ = dl;
            out.push(LBay { t: mid - 0.45, d: 0.55, nose: Nose::In, level: 0 });
            out.push(LBay { t: mid + 0.45, d: 0.55, nose: Nose::In, level: 0 });
        }
    }
    let _ = front;
    out
}

/// How many cars an apartment block of this many flats should hold: roughly one
/// space per two flats (the brief's "one space per one or two flats"), and the
/// undercroft is the only style with room for that many.
pub fn apt_capacity(flats: usize, style: AptParking, al: f32) -> usize {
    let ideal = (flats + 1) / 2;
    match style {
        AptParking::Undercroft => ideal.max(4),
        // The kerb row is limited by the frontage: half a tile a bay.
        AptParking::Kerbside => ideal.min(((al - 0.5) / 0.5).floor().max(1.0) as usize),
    }
}

/// The shape of a place's plot (the render crate and the sim each have their
/// own kind enums; this is what they share).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Shape {
    Park,
    Deck,
    Gas,
    Charge,
    Drive,
    Wash,
    Dealer,
    Motel,
}

/// Which way a parked car points.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Nose {
    /// Away from the kerb, into the plot.
    In,
    /// Toward the kerb.
    Out,
    /// Along the frontage, `t` increasing.
    Fwd,
    /// Along the frontage, `t` decreasing.
    Back,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LBay {
    pub t: f32,
    pub d: f32,
    pub nose: Nose,
    /// Which deck (0 for everything that is not the multi-storey).
    pub level: u8,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub t0: f32,
    pub t1: f32,
    pub d0: f32,
    pub d1: f32,
}

impl Rect {
    pub fn new(t0: f32, t1: f32, d0: f32, d1: f32) -> Self {
        Self { t0, t1, d0, d1 }
    }
    pub fn centre(&self) -> (f32, f32) {
        ((self.t0 + self.t1) * 0.5, (self.d0 + self.d1) * 0.5)
    }
    fn hits(&self, t: f32, d: f32, pad: f32) -> bool {
        t > self.t0 - pad && t < self.t1 + pad && d > self.d0 - pad && d < self.d1 + pad
    }
}

/// The plot placed on the map: its corner, size and the side its street is on.
/// `front`: 0 = street on +x, 1 = +y, 2 = -x, 3 = -y.
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub front: u8,
}

impl Frame {
    pub fn new(x: i32, y: i32, w: i32, h: i32, front: u8) -> Self {
        Self { x: x as f32, y: y as f32, w: w as f32, h: h as f32, front }
    }

    /// `(along, face)`: the unit step along the frontage (always positive) and
    /// the unit vector from the plot out to its street.
    pub fn axes(&self) -> ((f32, f32), (f32, f32)) {
        match self.front {
            0 => ((0.0, 1.0), (1.0, 0.0)),
            1 => ((1.0, 0.0), (0.0, 1.0)),
            2 => ((0.0, 1.0), (-1.0, 0.0)),
            _ => ((1.0, 0.0), (0.0, -1.0)),
        }
    }

    /// Length of the frontage and depth of the plot.
    pub fn size(&self) -> (f32, f32) {
        let (along, _) = self.axes();
        if along.0 != 0.0 { (self.w, self.h) } else { (self.h, self.w) }
    }

    /// Local `(t, d)` to a tile-space point.
    pub fn at(&self, t: f32, d: f32) -> (f32, f32) {
        let (along, face) = self.axes();
        let (_, dl) = self.size();
        let inward = (-face.0, -face.1);
        let o = if inward.0 < 0.0 || inward.1 < 0.0 { dl - d } else { d };
        (
            self.x + along.0 * t + inward.0.abs() * o,
            self.y + along.1 * t + inward.1.abs() * o,
        )
    }

    /// The unit direction a nose points, in tile space.
    pub fn dir(&self, n: Nose) -> (f32, f32) {
        let (along, face) = self.axes();
        match n {
            Nose::In => (-face.0, -face.1),
            Nose::Out => face,
            Nose::Fwd => along,
            Nose::Back => (-along.0, -along.1),
        }
    }
}

/// Rows of a nose-in car park: `(d, nose)` for each bank, cars pointing at the
/// aisle they back out of.
pub fn bank_rows(dl: f32) -> Vec<(f32, Nose)> {
    let c = dl * 0.5;
    if dl >= 3.1 {
        // Edge bank, aisle, a back-to-back double bank, aisle, edge bank.
        vec![
            (c - 1.275, Nose::In),
            (c - 0.225, Nose::Out),
            (c + 0.225, Nose::In),
            (c + 1.275, Nose::Out),
        ]
    } else if dl >= 1.6 {
        let aisle = (dl - 0.9 - 0.3).clamp(0.6, 0.75);
        let o = aisle * 0.5 + BAY_DEPTH * 0.5;
        vec![(c - o, Nose::In), (c + o, Nose::Out)]
    } else {
        vec![(c, Nose::In)]
    }
}

/// How many bays fit along `len` tiles of bank at `pitch`, and where.
fn along_bank(len: f32, margin: f32, pitch: f32) -> Vec<f32> {
    let n = ((len - 2.0 * margin) / pitch).floor().max(1.0) as i32;
    let step = (len - 2.0 * margin) / n as f32;
    (0..n).map(|i| margin + (i as f32 + 0.5) * step).collect()
}

/// The structure on the plot (the kiosk, the showroom, the tunnel, the rooms,
/// the stair core), `None` for a plain car park.
pub fn building(shape: Shape, al: f32, dl: f32) -> Option<Rect> {
    building_for(shape, al, dl, false)
}

/// As [`building`], for a plot whose street is on the far side of the camera
/// (`far`): a tall building stands at the street then, so the cars in front of
/// it are not hidden behind it (the motel).
pub fn building_for(shape: Shape, al: f32, dl: f32, far: bool) -> Option<Rect> {
    Some(match shape {
        Shape::Park => return None,
        Shape::Deck => Rect::new(0.15, 1.0, dl - 1.05, dl - 0.15),
        Shape::Gas => Rect::new(0.75, al - 0.75, dl - 0.95, dl - 0.25),
        Shape::Charge => Rect::new(al - 1.05, al - 0.3, dl - 0.65, dl - 0.2),
        Shape::Drive => Rect::new(0.8, al - 0.8, 0.95, dl - 0.55),
        Shape::Wash => Rect::new(0.45, al - 0.85, 0.45, dl - 0.45),
        Shape::Dealer => Rect::new(0.3, al - 0.3, dl - 1.0, dl - 0.2),
        Shape::Motel if far => Rect::new(0.35, al - 0.35, 0.12, 0.87),
        Shape::Motel => Rect::new(0.35, al - 0.35, dl - 0.85, dl - 0.1),
    })
}

/// The canopy over the pumps or the chargers, where there is one.
pub fn canopy(shape: Shape, al: f32, dl: f32) -> Option<Rect> {
    match shape {
        Shape::Gas => Some(Rect::new(0.45, al - 0.45, 0.45, 1.8)),
        Shape::Charge => Some(Rect::new(0.3, al - 0.3, 0.3, dl - 0.8)),
        _ => None,
    }
}

/// The pump island of a forecourt: `(t0, t1, d)` with `d` its centre line.
pub fn island(al: f32) -> (f32, f32, f32) {
    (0.75, al - 0.75, 1.125)
}

/// Centre line of the charger posts.
pub fn charger_line(dl: f32) -> f32 {
    let c = canopy(Shape::Charge, 3.0, dl).map(|r| r.centre().1).unwrap_or(1.0);
    c
}

/// The drive-through's lane as a polyline of `(t, d)` points (the first is at
/// the kerb, the last leaves the plot) and the distance along it of the
/// serving window.
pub fn drive_lane(al: f32, dl: f32) -> (Vec<(f32, f32)>, f32) {
    let m = 0.3;
    let pts = vec![(m, 0.0), (m, dl - m), (al - m, dl - m), (al - m, 0.0)];
    let b = building(Shape::Drive, al, dl).unwrap();
    let dw = (b.d0 + b.d1) * 0.5;
    // Down the first leg, across the back, then back toward the kerb until the
    // window.
    let s_win = (dl - m) + (al - 2.0 * m) + ((dl - m) - dw);
    (pts, s_win)
}

/// A point on a polyline at distance `s`, with the unit step direction there as
/// a nose.
pub fn along_lane(pts: &[(f32, f32)], s: f32) -> ((f32, f32), Nose) {
    let mut left = s.max(0.0);
    for w in pts.windows(2) {
        let (a, b) = (w[0], w[1]);
        let len = (b.0 - a.0).abs() + (b.1 - a.1).abs();
        let nose = if b.1 > a.1 + 1e-4 {
            Nose::In
        } else if b.1 < a.1 - 1e-4 {
            Nose::Out
        } else if b.0 > a.0 {
            Nose::Fwd
        } else {
            Nose::Back
        };
        if left <= len {
            let f = if len > 0.0 { left / len } else { 0.0 };
            return ((a.0 + (b.0 - a.0) * f, a.1 + (b.1 - a.1) * f), nose);
        }
        left -= len;
    }
    let n = pts.len();
    (pts[n - 1], Nose::Out)
}

/// Spacing of the cars in the drive-through queue (nose to tail).
pub const QUEUE_STEP: f32 = 0.5;

/// Every bay of a plot. The order matters to the callers that show only the
/// first few (the drive-through's queue from the window back, the forecourt's
/// first pump).
pub fn bays(shape: Shape, al: f32, dl: f32) -> Vec<LBay> {
    bays_for(shape, al, dl, false)
}

/// The depth of the motel's row of guests' cars from the kerb.
pub fn motel_car_d(b: Rect, far: bool) -> f32 {
    if far { b.d1 + 0.42 } else { b.d0 - 0.4 }
}

/// As [`bays`], knowing which way round the street is (see [`building_for`]).
pub fn bays_for(shape: Shape, al: f32, dl: f32, far: bool) -> Vec<LBay> {
    let mut out = Vec::new();
    match shape {
        Shape::Park | Shape::Deck => {
            let core = building(Shape::Deck, al, dl).filter(|_| shape == Shape::Deck);
            let levels = if shape == Shape::Deck { DECKS } else { 1 };
            for level in 0..levels {
                for (d, nose) in bank_rows(dl) {
                    for t in along_bank(al, 0.2, BAY_PITCH) {
                        if core.is_some_and(|c| c.hits(t, d, 0.25)) {
                            continue;
                        }
                        out.push(LBay { t, d, nose, level });
                    }
                }
            }
        }
        Shape::Gas => {
            let (t0, t1, dc) = island(al);
            let n = (((t1 - t0) / 0.75).floor() as i32).max(1);
            for side in [-1.0f32, 1.0] {
                for i in 0..n {
                    let t = t0 + (i as f32 + 0.5) * (t1 - t0) / n as f32;
                    out.push(LBay { t, d: dc + side * 0.4, nose: Nose::Fwd, level: 0 });
                }
            }
            // The cars waiting for a pump stand in front of the kerb line's
            // first bay in the same direction, so a busy forecourt queues.
        }
        Shape::Charge => {
            let c = canopy(Shape::Charge, al, dl).unwrap();
            let pc = charger_line(dl);
            let _ = c;
            for t in along_bank(al, 0.4, 0.5) {
                out.push(LBay { t, d: pc - 0.3, nose: Nose::In, level: 0 });
                out.push(LBay { t, d: pc + 0.3, nose: Nose::Out, level: 0 });
            }
        }
        Shape::Drive => {
            let (pts, s_win) = drive_lane(al, dl);
            for i in 0..12 {
                let s = s_win - i as f32 * QUEUE_STEP;
                if s < 0.7 {
                    break;
                }
                let ((t, d), nose) = along_lane(&pts, s);
                out.push(LBay { t, d, nose, level: 0 });
            }
        }
        Shape::Wash => {
            // The tunnel is entered from the +t end (the end the camera sees),
            // so the queue stands in front of its mouth, nose toward it.
            let b = building(Shape::Wash, al, dl).unwrap();
            let dc = (b.d0 + b.d1) * 0.5;
            out.push(LBay { t: (b.t1 + 0.36).min(al - 0.25), d: dc, nose: Nose::Back, level: 0 });
            out.push(LBay { t: (b.t1 + 0.76).min(al - 0.14), d: dc, nose: Nose::Back, level: 0 });
        }
        Shape::Dealer => {
            let b = building(Shape::Dealer, al, dl).unwrap();
            let rows = [0.5f32, 0.5 + 0.8];
            for &d in &rows {
                if d + BAY_DEPTH * 0.5 > b.d0 - 0.1 {
                    continue;
                }
                for t in along_bank(al, 0.35, 0.5) {
                    out.push(LBay { t, d, nose: Nose::Out, level: 0 });
                }
            }
        }
        Shape::Motel => {
            let b = building_for(Shape::Motel, al, dl, far).unwrap();
            for t in motel_doors(al) {
                out.push(LBay { t, d: motel_car_d(b, far), nose: if far { Nose::Out } else { Nose::In }, level: 0 });
            }
        }
    }
    out
}

/// Where the motel's room doors (and so its cars) are along the frontage.
pub fn motel_doors(al: f32) -> Vec<f32> {
    along_bank(al, 0.55, 0.62)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_bay_is_inside_the_plot() {
        for shape in [
            Shape::Park, Shape::Deck, Shape::Gas, Shape::Charge, Shape::Drive, Shape::Wash,
            Shape::Dealer, Shape::Motel,
        ] {
            for (al, dl) in [(3.0, 3.0), (4.0, 4.0), (4.0, 3.0), (5.0, 2.0), (3.0, 2.0), (2.0, 5.0)] {
                for b in bays(shape, al, dl) {
                    assert!(b.t > 0.1 && b.t < al - 0.1, "{shape:?} {al}x{dl} t {}", b.t);
                    assert!(b.d > 0.1 && b.d < dl - 0.1, "{shape:?} {al}x{dl} d {}", b.d);
                }
            }
        }
    }

    #[test]
    fn no_two_bays_overlap_on_a_level() {
        for shape in [Shape::Park, Shape::Deck, Shape::Gas, Shape::Charge, Shape::Dealer, Shape::Motel] {
            let (al, dl) = match shape {
                Shape::Motel => (5.0, 2.0),
                Shape::Dealer => (4.0, 3.0),
                Shape::Gas | Shape::Charge => (3.0, 3.0),
                _ => (4.0, 4.0),
            };
            let v = bays(shape, al, dl);
            for (i, a) in v.iter().enumerate() {
                for b in &v[i + 1..] {
                    if a.level != b.level {
                        continue;
                    }
                    let near = (a.t - b.t).abs() < 0.3 && (a.d - b.d).abs() < 0.3;
                    assert!(!near, "{shape:?}: {a:?} and {b:?}");
                }
            }
        }
    }

    #[test]
    fn the_queue_follows_the_lane() {
        let (pts, s_win) = drive_lane(3.0, 3.0);
        let v = bays(Shape::Drive, 3.0, 3.0);
        assert!(v.len() >= 8, "a 3x3 drive-through queues at least eight cars");
        // Nose to tail: successive cars are one step apart along the lane.
        let (p0, _) = along_lane(&pts, s_win);
        assert!((v[0].t - p0.0).abs() < 1e-4 && (v[0].d - p0.1).abs() < 1e-4);
    }

    #[test]
    fn frame_maps_the_kerb_side_to_the_street() {
        for front in 0..4u8 {
            let f = Frame::new(10, 20, 4, 4, front);
            let (_, face) = f.axes();
            // The kerb (d = 0) is nearer the street than the back (d = dl).
            let k = f.at(2.0, 0.0);
            let b = f.at(2.0, 4.0);
            let dk = (k.0 - b.0) * face.0 + (k.1 - b.1) * face.1;
            assert!(dk > 3.9, "front {front}");
        }
    }

    #[test]
    fn an_apartment_pays_for_itself_on_the_kerb_or_under_the_building() {
        for front in 0..4u8 {
            let f = Frame::new(10, 20, 2, 2, front);
            let (al, dl) = f.size();
            // Low-rise: a kerb row outside the plot, on the street side.
            let k = apt_bays(al, dl, front, AptParking::Kerbside);
            assert!(k.len() >= 2, "front {front}: {} kerb bays", k.len());
            let (_, face) = f.axes();
            for b in &k {
                assert!(b.d < 0.0);
                // The bay is one step *towards* the street from the kerb.
                let (px, py) = f.at(b.t, b.d);
                let (kx, ky) = f.at(b.t, 0.0);
                let towards = (px - kx) * face.0 + (py - ky) * face.1;
                assert!(towards > 0.1, "front {front}: a kerb bay is not on the street side");
            }
            // Tall: two mouth bays, inside the frontage.
            let u = apt_bays(al, dl, front, AptParking::Undercroft);
            assert_eq!(u.len(), 2, "front {front}: an undercroft has two mouth bays");
            for b in &u {
                assert!(b.d > 0.0);
                assert_eq!(b.nose, Nose::In);
            }
        }
        assert_eq!(apt_style(4), AptParking::Kerbside);
        assert_eq!(apt_style(5), AptParking::Undercroft);
        assert!(apt_capacity(24, AptParking::Undercroft, 2.0) == 12);
        assert!(apt_capacity(24, AptParking::Kerbside, 2.0) <= 3);
    }
}
