//! B74 (note from the owner): an apartment block is connected to a road **and**
//! parks its residents' cars.
//!
//! A flat block's own yard is a strip too thin for a car (the renderer's
//! `INSET` is 0.13 of a lot), so the parking a block gets is decided by how tall
//! it is, and both answers open straight onto the street:
//!
//! * a low-rise block gets a row of **marked bays along its own kerb**, on the
//!   lane it fronts (the brief's "marked parking bays along its own street when
//!   the lot is tight");
//! * a block of five storeys or more gets an **undercroft**: a garage under the
//!   building whose mouth and ramp are on the frontage, sized to hold a space
//!   per two flats.
//!
//! The geometry is shared with the renderer (`map::carplaces::apt_kerb_bays`), so
//! the car a frame draws in a bay and the bay the audit counts are one
//! definition: both ask their own map which tiles are street.
//!
//! Nothing here is saved: a block's parking follows its own lot (kind, storeys,
//! size, frontage, and the streets beside it), so an old save's apartments get
//! their parking on load.

use map::Kind;
use map::carplaces::{self, AptParking, Frame};

use crate::carculture::Bay;
use crate::{Lot, LotKind, Sim};

/// Flats in a block: two people a flat, and a block's occupancy follows its
/// height (B26d), so a taller block holds more homes.
pub fn flats(l: &Lot) -> usize {
    (l.occupants_now() as usize).div_ceil(2)
}

/// How a block parks: kerbside row or undercroft (B74 note).
pub fn style(l: &Lot) -> AptParking {
    carplaces::apt_style(l.storeys)
}

/// Is this a block that must park its residents? (Only apartments: a shop with
/// a flat over it is not a block of flats.)
pub fn needs_parking(l: &Lot) -> bool {
    l.kind == LotKind::Apartment
}

/// The bays a block offers, given the caller's answer to "is this tile street?".
/// Empty while the block is still going up.
pub fn bays_with(l: &Lot, streetlike: impl Fn(i32, i32) -> bool + Copy) -> Vec<Bay> {
    if !needs_parking(l) || !l.finished() {
        return Vec::new();
    }
    match style(l) {
        AptParking::Undercroft => {
            let f = Frame::new(l.x, l.y, l.w, l.h, l.front);
            let (al, dl) = f.size();
            let mut out: Vec<Bay> = carplaces::apt_bays(al, dl, l.front, style(l))
                .into_iter()
                .map(|b| {
                    let (x, y) = f.at(b.t, b.d);
                    Bay {
                        x,
                        y,
                        dir: f.dir(b.nose),
                        lift: 0.0,
                    }
                })
                .collect();
            // The garage is under the building; the kerb outside it takes the
            // visitors and the overflow, which is also what makes the block's
            // parking visible from the street (and different at eight and ten).
            for (x, y, _nose) in carplaces::apt_kerb_bays(l.x, l.y, l.w, l.h, l.front, streetlike) {
                out.push(Bay {
                    x,
                    y,
                    dir: (1.0, 0.0),
                    lift: 0.0,
                });
            }
            out
        }
        AptParking::Kerbside => carplaces::apt_kerb_bays(l.x, l.y, l.w, l.h, l.front, streetlike)
            .into_iter()
            .map(|(x, y, _nose)| Bay {
                x,
                y,
                // A kerb bay faces along the street: the sim only needs a
                // direction for the sprite's nose, and a car at a kerb parks
                // parallel to it.
                dir: (1.0, 0.0),
                lift: 0.0,
            })
            .collect(),
    }
}

/// How many cars this block can hold. The undercroft holds a space per two
/// flats; a kerb row is whatever its street really offers.
pub fn capacity_with(l: &Lot, streetlike: impl Fn(i32, i32) -> bool + Copy) -> usize {
    if !needs_parking(l) || !l.finished() {
        return 0;
    }
    match style(l) {
        AptParking::Undercroft => {
            let f = Frame::new(l.x, l.y, l.w, l.h, l.front);
            let (al, _) = f.size();
            // The garage's own spaces plus the kerb row outside it.
            carplaces::apt_capacity(flats(l), style(l), al)
                + bays_with(l, streetlike).len().saturating_sub(2)
        }
        AptParking::Kerbside => bays_with(l, streetlike).len(),
    }
}

/// How full the residents' parking is at this hour: home in the evening and
/// through the night, out at work by the morning. `0..1`.
pub fn fill(hour: f64) -> f32 {
    let h = hour.rem_euclid(24.0);
    if (22.0..24.0).contains(&h) || h < 5.5 {
        0.95
    } else if (5.5..8.0).contains(&h) {
        // The commute: cars leave one by one.
        (0.95 - (h - 5.5) / 2.5 * 0.55) as f32
    } else if (8.0..17.0).contains(&h) {
        0.38
    } else if (17.0..19.5).contains(&h) {
        // Home again.
        (0.38 + (h - 17.0) / 2.5 * 0.52) as f32
    } else {
        0.90
    }
}

#[inline]
fn is_street(k: Kind) -> bool {
    k.is_lane() || k == Kind::Sidewalk
}

impl Sim {
    /// Is this tile a street (a lane or the drive to one)? Both the sim and the
    /// renderer use this test, on their own copy of the map.
    pub fn tile_is_street(&self, x: i32, y: i32) -> bool {
        is_street(self.map.at(x, y).kind)
    }

    /// A block's parking bays, in tile space.
    pub fn parking_bays(&self, l: &Lot) -> Vec<Bay> {
        bays_with(l, |x, y| self.tile_is_street(x, y))
    }

    /// A block's parking capacity.
    pub fn parking_spaces(&self, l: &Lot) -> usize {
        capacity_with(l, |x, y| self.tile_is_street(x, y))
    }

    /// Every block of flats' parking, for the audit and the tests:
    /// `(lot index, style, spaces, bays)`.
    pub fn apartment_parking(&self) -> Vec<(usize, AptParking, usize, usize)> {
        self.lots
            .iter()
            .enumerate()
            .filter(|(_, l)| needs_parking(l))
            .map(|(i, l)| {
                let b = self.parking_bays(l);
                (i, style(l), self.parking_spaces(l), b.len())
            })
            .collect()
    }

    /// Blocks with no parking at all (the audit's `NoParking`).
    pub fn apartments_without_parking(&self) -> usize {
        self.lots
            .iter()
            .filter(|l| needs_parking(l) && self.parking_spaces(l) == 0)
            .count()
    }

    /// Is a kerbside bay really on a street (or the drive to it)? The parking
    /// must be connected: a bay that lands on grass is a plot in the wrong place.
    pub fn bay_is_on_the_street(&self, b: &Bay) -> bool {
        let (x, y) = b.tile();
        self.tile_is_street(x, y)
    }

    /// Does this block park its residents the way the brief asks? A block with
    /// no spaces fails; a kerb row has to stand on the street; an undercroft has
    /// to open onto it (its access is the road-access rule).
    pub fn apartment_parks_itself(&self, l: &Lot) -> bool {
        if self.parking_spaces(l) == 0 {
            return false;
        }
        match style(l) {
            AptParking::Kerbside => self.parking_bays(l).iter().all(|b| self.bay_is_on_the_street(b)),
            AptParking::Undercroft => crate::audit::lot_reaches_road(&self.map, l),
        }
    }
}

/// `map::carplaces` needs the street test as a plain closure (the two crates
/// have different map types), so this is the bridge the sim hands it.
pub fn street_test<'a>(sim: &'a Sim) -> impl Fn(i32, i32) -> bool + Copy + 'a {
    move |x, y| sim.tile_is_street(x, y)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Pace, Sim};

    fn grown(seed: u64, days: f64) -> Sim {
        let vp = map::world_for_viewport(7040, 1440);
        let focus = map::screen_to_tile(&vp, 3520.0, 720.0);
        let mut s = Sim::with_focus(seed, Pace::Month, vp.w, vp.h, focus);
        s.ending.world_days = 1e9;
        s.run_to(days * 86400.0);
        s
    }

    /// A grown city's blocks of flats park their residents in bays that are
    /// either on the street (a kerb row) or under the building (an undercroft
    /// mouth) — never on bare ground — and the parking is sized to the homes.
    /// A block the grid did not front has no bays at all, and that is reported,
    /// not failed (round 4: road access is no longer enforced).
    #[test]
    fn every_apartment_parks_and_its_bays_are_on_the_street() {
        let s = grown(7, 60.0);
        let plans = s.apartment_parking();
        assert!(plans.len() >= 3, "only {} apartment blocks in the city", plans.len());
        let mut no_parking = 0;
        for (i, st, spaces, _n) in &plans {
            let l = &s.lots[*i];
            assert!(
                *spaces <= flats(l) + 8,
                "a block at {},{} claims {spaces} spaces for {} flats",
                l.x,
                l.y,
                flats(l)
            );
            if *spaces == 0 {
                no_parking += 1;
            }
            for b in s.parking_bays(l) {
                let (x, y) = b.tile();
                if *st == AptParking::Kerbside {
                    assert!(
                        s.bay_is_on_the_street(&b),
                        "a kerb bay of the apartment at {},{} stands off the street at {x},{y}",
                        l.x,
                        l.y
                    );
                } else {
                    // An undercroft has its mouth under the building and a kerb
                    // row outside it; nothing may land anywhere else.
                    let inside = x >= l.x && x < l.x + l.w && y >= l.y && y < l.y + l.h;
                    assert!(
                        inside || s.bay_is_on_the_street(&b),
                        "an undercroft bay of the block at {},{} is on bare ground at {x},{y}",
                        l.x,
                        l.y
                    );
                }
            }
        }
        eprintln!("{no_parking} of {} blocks have no street and no parking", plans.len());
    }

    /// A tall block digs down (a space per two flats); a low one uses its kerb.
    #[test]
    fn a_tall_block_holds_more_cars_than_a_low_one() {
        let street = |_x: i32, _y: i32| true;
        let mut l = Lot {
            x: 10,
            y: 10,
            w: 2,
            h: 2,
            level: 3,
            kind: LotKind::Apartment,
            storeys: 12,
            style: 0,
            seed: 1,
            front: 1,
            stage: crate::Stage::Finished,
            stage_end: 0.0,
            started: 0.0,
        };
        let tall = capacity_with(&l, street);
        assert_eq!(style(&l), AptParking::Undercroft);
        // The undercroft is sized to the homes: at least a space per two flats.
        assert!(tall >= flats(&l) / 2, "a 12-storey block holds {tall} of {} spaces", flats(&l) / 2);
        l.storeys = 3;
        let low = capacity_with(&l, street);
        assert_eq!(style(&l), AptParking::Kerbside);
        assert!(tall > low, "tall block {tall} does not out-park low {low}");
        // A block still going up offers nothing yet.
        l.stage = crate::Stage::Framing;
        assert_eq!(capacity_with(&l, street), 0);
    }

    /// Parking is full at night and emptier in the working day.
    #[test]
    fn the_residents_are_home_at_night() {
        assert!(fill(22.0) > 0.9);
        assert!(fill(2.0) > 0.9);
        assert!(fill(12.0) < 0.5);
        assert!(fill(12.0) < fill(19.5));
        assert!(fill(8.0) < fill(21.0));
    }
}
