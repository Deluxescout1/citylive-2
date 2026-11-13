//! Real-sky astronomy for the night sky (B45 / D67).
//!
//! Ports the old CityLive's astronomy (city.js "real night sky", "the Milky Way
//! + Andromeda", "ECLIPSES", "PLANET EPHEMERIS", "METEORS", "a GREAT COMET",
//! "THE STAR FIELD") to Rust, reusing their data tables and maths. Covers:
//!
//! * the embedded bright-star catalogue ([`crate::starcat`], generated);
//! * the Milky Way dab field, built once from a FIXED seed (the band is the
//!   same sky every night, on every screen);
//! * planets Mercury–Saturn from the JPL/Standish Keplerian elements (the old
//!   code had no Mercury; its elements complete the set here);
//! * solar/lunar eclipses from Meeus (Sun ch.25, Moon ch.47, nutation ch.22,
//!   topocentric ch.40, lunar shadow ch.54) — **topocentric for the observer**,
//!   which is what makes an eclipse local (see `topo_ra_dec`);
//! * meteor showers on their real dates, with the old slot state machine so a
//!   streak is deterministic from the wall clock;
//! * a great comet that visits on a rare season schedule and drifts against
//!   the stars night to night.
//!
//! Everything here is pure; no I/O, no wall clock, no allocation on the hot
//! path. The sky layer caches projections at most once a minute (B45 cost rule).

use crate::model::julian_from_ms;
use crate::moon::RaDec;
use crate::season::date_from_ms;

const DEG: f64 = std::f64::consts::PI / 180.0;

#[inline]
fn rev(x: f64) -> f64 {
    let x = x % 360.0;
    if x < 0.0 { x + 360.0 } else { x }
}

// ---------------------------------------------------------------- the catalogue

pub use crate::starcat::{NamedStar, Star, CATALOG, LINKS, NAMED};

impl Star {
    /// RA in hours (0..24).
    #[inline]
    pub fn ra_hours(&self) -> f64 {
        self.ra_ms_h as f64 / 1000.0
    }
    /// Declination in degrees.
    #[inline]
    pub fn dec_deg(&self) -> f64 {
        self.dec_cd as f64 / 100.0
    }
    /// Apparent magnitude.
    #[inline]
    pub fn mag(&self) -> f64 {
        self.mag_dm as f64 / 10.0
    }
    /// B-V colour index.
    #[inline]
    pub fn bv(&self) -> f64 {
        self.ci_cc as f64 / 50.0
    }
}

/// B-V colour index → display RGB. A blue-black ramp through white to orange-red
/// (approximates the blackbody hue of a star's photosphere; the old code used
/// four fixed buckets, this is the same idea as one smooth curve).
pub fn star_colour(bv: f64) -> (u8, u8, u8) {
    // bv -0.3 (hot blue) .. +2.0 (cool red)
    let t = (bv + 0.3).clamp(0.0, 2.3) / 2.3; // 0 = blue, 1 = red
    // Anchor colours: B  (155,190,255) — A (200,225,255) — F (240,245,255)
    //                   G (255,248,225) — K (255,220,180) — M (255,178,130)
    let anchors: [(f64, u8, u8, u8); 5] = [
        (0.00, 155, 196, 255),
        (0.18, 214, 230, 255),
        (0.38, 255, 250, 240),
        (0.68, 255, 224, 178),
        (1.00, 255, 172, 122),
    ];
    if t >= 1.0 {
        let (_, r, g, b) = anchors[4];
        return (r, g, b);
    }
    for w in anchors.windows(2) {
        if t >= w[0].0 && t <= w[1].0 {
            let u = if w[1].0 == w[0].0 { 0.0 } else { (t - w[0].0) / (w[1].0 - w[0].0) };
            return (
                (w[0].1 as f64 + (w[1].1 as f64 - w[0].1 as f64) * u).round() as u8,
                (w[0].2 as f64 + (w[1].2 as f64 - w[0].2 as f64) * u).round() as u8,
                (w[0].3 as f64 + (w[1].3 as f64 - w[0].3 as f64) * u).round() as u8,
            );
        }
    }
    let (_, r, g, b) = anchors[0];
    (r, g, b)
}

// ---------------------------------------------------------------- the Milky Way

/// Galactic → equatorial rotation (J2000), the old code's `MW_M` verbatim:
/// equatorial = Mᵀ · galactic. Verified anchors in the JS: the galactic centre
/// (ℓ=0,b=0) → RA 17.76h Dec −29°, the NGP → Dec +27.13°, M31 → RA 0.712h.
const MW_M: [[f64; 3]; 3] = [
    [-0.054_875_560_4, -0.873_437_090_2, -0.483_835_015_5],
    [0.494_109_427_9, -0.444_829_630_0, 0.746_982_244_5],
    [-0.867_666_149_0, -0.198_076_373_4, 0.455_983_776_2],
];

pub fn gal_to_eq(l_deg: f64, b_deg: f64) -> RaDec {
    let (l, b) = (l_deg * DEG, b_deg * DEG);
    let gx = b.cos() * l.cos();
    let gy = b.cos() * l.sin();
    let gz = b.sin();
    let ex = MW_M[0][0] * gx + MW_M[1][0] * gy + MW_M[2][0] * gz;
    let ey = MW_M[0][1] * gx + MW_M[1][1] * gy + MW_M[2][1] * gz;
    let ez = MW_M[0][2] * gx + MW_M[1][2] * gy + MW_M[2][2] * gz;
    let dec = ez.clamp(-1.0, 1.0).asin() / DEG;
    let mut ra = ey.atan2(ex) / DEG;
    if ra < 0.0 {
        ra += 360.0;
    }
    RaDec { ra: ra / 15.0, dec }
}

/// One dab of the Milky Way: a soft point of unresolved starlight at a fixed
/// sky position, with its weight (0..~0.35), size class and colour class.
/// Compact (8 bytes) so ~2 300 of them cost ~18 KB, built once.
#[derive(Clone, Copy)]
pub struct MwDab {
    /// RA, milli-hours.
    pub ra_ms_h: u16,
    /// Dec, centi-degrees.
    pub dec_cd: i16,
    /// Weight ×1000 (the old `w` field).
    pub w_th: u16,
    /// Size in px (1..=5).
    pub sz: u8,
    /// Colour class: 0 = blue-white starlight, 1 = warm-gold core cloud,
    /// 2 = rose H-alpha nebula.
    pub class: u8,
}

/// One point of the luminous spine (the river's bed, stroked as ribbons).
#[derive(Clone, Copy)]
pub struct MwSpine {
    pub ra_ms_h: u16,
    pub dec_cd: i16,
    /// Half-width weight ×1000 (the old `w`).
    pub w_th: u16,
    /// How close to the galactic centre (0..1): gold there, blue-white outward.
    pub core_th: u16,
}

/// The Milky Way's fixed shape: ~240 spine points + ~2 300 dabs, built once
/// from the FIXED seed (the old `rng(0x4D575759)` "MWWY" — identical on every
/// screen), never per-life. Ported from the old `buildMilkyWay`.
pub fn milky_way() -> &'static (Vec<MwSpine>, Vec<MwDab>) {
    use std::sync::OnceLock;
    static MW: OnceLock<(Vec<MwSpine>, Vec<MwDab>)> = OnceLock::new();
    MW.get_or_init(build_milky_way)
}

/// The old JS `rng` (mulberry32), ported bit-for-bit so the dab field is the
/// same sky the old CityLive drew.
struct JsRng(u32);
impl JsRng {
    fn new(seed: u32) -> Self {
        JsRng(seed)
    }
    fn next_f64(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x6D2B_79F5);
        let mut t = (self.0 ^ (self.0 >> 15)).wrapping_mul(1 | self.0);
        t = t.wrapping_add((t ^ (t >> 7)).wrapping_mul(61 | t)) ^ t;
        ((t ^ (t >> 14)) as f64) / 4_294_967_296.0
    }
}

fn build_milky_way() -> (Vec<MwSpine>, Vec<MwDab>) {
    let mut r = JsRng::new(0x4D57_5759); // "MWWY"
                                                    // triangular ≈ gaussian, tight to the equator
    let gauss = |r: &mut JsRng, s: f64| (r.next_f64() + r.next_f64() - 1.0) * s;

    let mut spine: Vec<MwSpine> = Vec::with_capacity(244);
    let mut dabs: Vec<MwDab> = Vec::with_capacity(2400);

    // The ridge: ordered by galactic longitude so adjacent points are
    // strokeable; fatter near the Sagittarius core.
    let mut l = 0.0f64;
    while l < 360.0 {
        let d0 = l.min(360.0 - l);
        let bulge = if d0 < 40.0 { 1.0 - d0 / 40.0 } else { 0.0 };
        let e = gal_to_eq(l, gauss(&mut r, 0.9));
        let w = 0.10 + 0.10 * bulge;
        spine.push(MwSpine {
            ra_ms_h: (e.ra * 1000.0).round() as u16,
            dec_cd: (e.dec * 100.0).round() as i16,
            w_th: (w * 1000.0).round() as u16,
            core_th: (core_factor(e.ra, e.dec) * 1000.0).round() as u16,
        });
        l += 1.5;
    }

    // The Great Rift (three braided dust filaments) and the dark cloudlets.
    let rift_k = |l: f64, b: f64| -> f64 {
        let e1 = (l * 0.31).sin() * 1.6 + (l * 0.83).sin() * 0.9;
        let e2 = (l * 0.47 + 2.0).sin() * 1.2 + (l * 1.1).sin() * 0.6;
        if l <= 66.0 && (b > -7.0 + e1 && b < 1.0 + e2) {
            0.0
        } else if (24.0..=58.0).contains(&l) && (b > 2.6 + e2 * 0.5 && b < 5.8 + e1 * 0.5) {
            0.25
        } else if (62.0..=84.0).contains(&l) && (b > -4.6 + e1 && b < 1.8 + e2) {
            0.15
        } else {
            1.0
        }
    };
    struct Cloud {
        l: f64,
        b: f64,
        rad: f64,
    }
    let mut dkp: Vec<Cloud> = Vec::with_capacity(26);
    for _ in 0..26 {
        dkp.push(Cloud {
            l: r.next_f64() * 360.0,
            b: gauss(&mut r, 7.0),
            rad: 1.5 + r.next_f64() * 3.0,
        });
    }
    let dark_k = |l: f64, b: f64| -> f64 {
        for p in &dkp {
            let mut dl = (l - p.l).abs();
            if dl > 180.0 {
                dl = 360.0 - dl;
            }
            if dl * dl + (b - p.b) * (b - p.b) < p.rad * p.rad {
                return 0.25;
            }
        }
        1.0
    };
    let clump_k = |l: f64, b: f64| {
        0.62 + 0.38 * (l * 0.9 + b * 0.8).sin() * (l * 0.23 + 1.7).sin() + 0.18 * (l * 2.1 + b * 1.9).sin()
    };

    let push = |dabs: &mut Vec<MwDab>, ra: f64, dec: f64, w: f64, sz: u8, class: u8| {
        dabs.push(MwDab {
            ra_ms_h: (ra * 1000.0).round() as u16,
            dec_cd: (dec * 100.0).round() as i16,
            w_th: (w * 1000.0).round() as u16,
            sz,
            class,
        });
    };

    for li in (0..360).step_by(1) {
        let l = li as f64;
        let d02 = l.min(360.0 - l);
        let bulge2 = if d02 < 50.0 { 1.0 - d02 / 50.0 } else { 0.0 };
        let cloud = 1.0
            + 1.5 * bulge2
            + if (18.0..=32.0).contains(&l) { 0.9 } else { 0.0 } // Scutum star cloud
            + if (66.0..=86.0).contains(&l) { 0.8 } else { 0.0 }; // Cygnus star cloud

        // layer 1 — HAZE: the continuous cloud body.
        for h2 in 0..4u8 {
            let bh = gauss(&mut r, 10.0) * (1.0 + 0.8 * bulge2);
            let mut wh = (0.050 + 0.070 * cloud) * (1.0 - bh.abs() / 22.0).max(0.10);
            let rk = rift_k(l, bh);
            wh *= 0.35 + 0.65 * rk;
            wh *= clump_k(l, bh).max(0.45);
            if r.next_f64() < 0.10 {
                continue;
            }
            let e = gal_to_eq(l + r.next_f64() - 0.5, bh);
            // The old tiers (5/4/3 px on a full-height sky) scaled to our
            // ~30 % band: 3/3/2/2.
            let sz = [3u8, 3, 2, 2][h2 as usize];
            let class = if bulge2 > 0.35 && r.next_f64() < 0.55 { 1 } else { 0 };
            push(&mut dabs, e.ra, e.dec, wh, sz, class);
        }
        // layer 2 — GRAIN: the fine mottle of millions of unresolved stars.
        let n = if li % 2 == 0 { 5 } else { 4 };
        for _ in 0..n {
            let b = gauss(&mut r, 8.0) * (1.0 + 0.8 * bulge2) + gauss(&mut r, 4.0);
            let mut wgt = (0.08 + 0.10 * cloud) * (1.0 - b.abs() / 24.0).max(0.12);
            let rk = rift_k(l, b);
            wgt *= 0.08 + 0.92 * rk;
            wgt *= dark_k(l, b) * clump_k(l, b).max(0.25);
            if r.next_f64() < 0.12 {
                continue;
            }
            let e = gal_to_eq(l + r.next_f64() - 0.5, b);
            let sz = if r.next_f64() < 0.30 { 2 } else { 1 };
            let class = if bulge2 > 0.3 && r.next_f64() < 0.6 { 1 } else { 0 };
            push(&mut dabs, e.ra, e.dec, wgt, sz, class);
        }
        // layer 3 — STAR SPRINKLE: pinpricks feathering past the band.
        for _ in 0..2 {
            let bs = gauss(&mut r, 18.0) + gauss(&mut r, 8.0);
            let e = gal_to_eq(l + r.next_f64() - 0.5, bs);
            push(&mut dabs, e.ra, e.dec, 0.035 + 0.03 * r.next_f64(), 1, 0);
        }
    }
    // bright knots & clumps riding the band.
    for _ in 0..90 {
        let kl = r.next_f64() * 360.0;
        let kd = kl.min(360.0 - kl);
        let kk = gal_to_eq(kl, gauss(&mut r, 7.0));
        let w = 0.20 + 0.15 * r.next_f64() * if kd < 55.0 { 1.5 } else { 1.0 };
        let sz = if r.next_f64() < 0.4 { 3 } else { 2 };
        let class = if kd < 55.0 && r.next_f64() < 0.55 { 1 } else { 0 };
        push(&mut dabs, kk.ra, kk.dec, w, sz, class);
    }
    // Emission nebulae at their real homes (Lagoon/Omega/Eagle near the core,
    // North America by Deneb, Heart & Soul in Cassiopeia) — rose H-alpha.
    const NEBS: [(f64, f64); 8] = [
        (6.0, -1.5),
        (15.0, -0.8),
        (17.0, 0.8),
        (85.0, -1.2),
        (134.0, 0.9),
        (137.0, 1.2),
        (353.0, 1.4),
        (265.0, -1.4),
    ];
    for &(nl, nb) in NEBS.iter() {
        for np in 0..5 {
            let e = gal_to_eq(nl + gauss(&mut r, 1.4), nb + gauss(&mut r, 1.1));
            let sz = if np == 0 { 3 } else { 2 };
            push(&mut dabs, e.ra, e.dec, 0.10 + 0.10 * r.next_f64(), sz, 2);
        }
    }
    // Embedded star clusters: tight sparkling knots sitting in the band.
    for _ in 0..22 {
        let cl = r.next_f64() * 360.0;
        let cb = gauss(&mut r, 6.0);
        for _ in 0..(4 + (r.next_f64() * 3.0) as usize) {
            let e = gal_to_eq(cl + gauss(&mut r, 0.9), cb + gauss(&mut r, 0.7));
            push(&mut dabs, e.ra, e.dec, 0.26 + 0.18 * r.next_f64(), 1, 0);
        }
    }
    (spine, dabs)
}

/// How close a sky position is to the galactic centre (Sagittarius A*:
/// RA 17.76h, Dec −28.94°) — the flattened disc distance, falloff widened the
/// way the old code did (the core never climbs high at mid-northern latitudes).
fn core_factor(ra_h: f64, dec_deg: f64) -> f64 {
    const GC_RA: f64 = 17.76;
    const GC_DEC: f64 = -28.94;
    let mut d_ra = (ra_h - GC_RA) * 15.0;
    while d_ra > 180.0 {
        d_ra -= 360.0;
    }
    while d_ra < -180.0 {
        d_ra += 360.0;
    }
    let d_de = dec_deg - GC_DEC;
    let gd = (d_ra * d_ra * 0.35 + d_de * d_de).sqrt();
    (1.0 - gd / 105.0).clamp(0.0, 1.0)
}

// ---------------------------------------------------------------- planets

/// The five naked-eye planets the old CityLive drew, plus Mercury (its
/// Standish elements complete the table here — the old code skipped it).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Planet {
    Mercury,
    Venus,
    Mars,
    Jupiter,
    Saturn,
}

pub const PLANETS: [Planet; 5] = [
    Planet::Mercury,
    Planet::Venus,
    Planet::Mars,
    Planet::Jupiter,
    Planet::Saturn,
];

impl Planet {
    /// JPL/Standish Keplerian elements, epoch J2000, rates per century
    /// (valid 1800–2050, accurate to a few arcmin).
    fn elements(self) -> Elements {
        match self {
            Self::Mercury => Elements {
                a: 0.38709927, a_r: 0.00000037, e: 0.20563593, e_r: 0.00001906,
                i: 7.00497902, i_r: -0.00594749, l: 252.25032350, l_r: 149472.67411175,
                w: 77.45779628, w_r: 0.16047689, o: 48.33076593, o_r: -0.12534081,
            },
            Self::Venus => Elements {
                a: 0.72333566, a_r: 0.00000390, e: 0.00677672, e_r: -0.00004107,
                i: 3.39467605, i_r: -0.00078890, l: 181.97909950, l_r: 58517.81538729,
                w: 131.60246718, w_r: 0.00268329, o: 76.67984255, o_r: -0.27769418,
            },
            Self::Mars => Elements {
                a: 1.52371034, a_r: 0.00001847, e: 0.09339410, e_r: 0.00007882,
                i: 1.84969142, i_r: -0.00813131, l: -4.55343205, l_r: 19140.30268499,
                w: -23.94362959, w_r: 0.44441088, o: 49.55953891, o_r: -0.29257343,
            },
            Self::Jupiter => Elements {
                a: 5.20288700, a_r: -0.00011607, e: 0.04838624, e_r: -0.00013253,
                i: 1.30439695, i_r: -0.00183714, l: 34.39644051, l_r: 3034.74612775,
                w: 14.72847983, w_r: 0.21252668, o: 100.47390909, o_r: 0.20469106,
            },
            Self::Saturn => Elements {
                a: 9.53667594, a_r: -0.00125060, e: 0.05386179, e_r: -0.00050991,
                i: 2.48599187, i_r: 0.00193609, l: 49.95424423, l_r: 1222.49362201,
                w: 92.59887831, w_r: -0.41897216, o: 113.66242448, o_r: -0.28867794,
            },
        }
    }

    /// Earth's own elements (for the geocentric subtraction).
    fn earth() -> Elements {
        Elements {
            a: 1.00000261, a_r: 0.00000562, e: 0.01671123, e_r: -0.00004392,
            i: -0.00001531, i_r: -0.01294668, l: 100.46457166, l_r: 35999.37244981,
            w: 102.93768193, w_r: 0.32327364, o: 0.0, o_r: 0.0,
        }
    }

    /// The old code's display colours (Mercury grey-white like the JS's Venus
    /// tone family).
    pub fn colour(self) -> (u8, u8, u8) {
        match self {
            Self::Mercury => (216, 216, 210),
            Self::Venus => (255, 248, 232),
            Self::Mars => (255, 120, 80),
            Self::Jupiter => (255, 226, 182),
            Self::Saturn => (240, 222, 166),
        }
    }
}

#[derive(Clone, Copy)]
struct Elements {
    a: f64, a_r: f64, e: f64, e_r: f64, i: f64, i_r: f64,
    l: f64, l_r: f64, w: f64, w_r: f64, o: f64, o_r: f64,
}

/// Heliocentric rectangular coordinates (AU) at T centuries from J2000.
/// Verbatim port of the old `planetHelio`.
fn planet_helio(el: Elements, t: f64) -> [f64; 3] {
    let a = el.a + el.a_r * t;
    let e = el.e + el.e_r * t;
    let inc = (el.i + el.i_r * t) * DEG;
    let l = el.l + el.l_r * t;
    let wl = el.w + el.w_r * t;
    let om = (el.o + el.o_r * t) * DEG;
    let mut m = l - wl;
    m = ((m % 360.0) + 360.0) % 360.0;
    if m > 180.0 {
        m -= 360.0;
    }
    let m = m * DEG;
    let mut ecc_anom = m;
    for _ in 0..8 {
        ecc_anom -= (ecc_anom - e * ecc_anom.sin() - m) / (1.0 - e * ecc_anom.cos());
    }
    let xp = a * (ecc_anom.cos() - e);
    let yp = a * (1.0 - e * e).sqrt() * ecc_anom.sin();
    let wp = wl * DEG - om;
    let (cw, sw) = (wp.cos(), wp.sin());
    let (co, so) = (om.cos(), om.sin());
    let (ci, si) = (inc.cos(), inc.sin());
    [
        (cw * co - sw * so * ci) * xp + (-sw * co - cw * so * ci) * yp,
        (cw * so + sw * co * ci) * xp + (-sw * so + cw * co * ci) * yp,
        (sw * si) * xp + (cw * si) * yp,
    ]
}

/// Geocentric apparent RA/Dec for a planet at `ms` (the old `planetRaDec`).
pub fn planet_ra_dec(p: Planet, ms: i64) -> RaDec {
    let jd = julian_from_ms(ms);
    let t = (jd - 2_451_545.0) / 36_525.0;
    let ph = planet_helio(p.elements(), t);
    let ea = planet_helio(Planet::earth(), t);
    let (gx, gy, gz) = (ph[0] - ea[0], ph[1] - ea[1], ph[2] - ea[2]);
    let ec = 23.43928 * DEG;
    let eqy = gy * ec.cos() - gz * ec.sin();
    let eqz = gy * ec.sin() + gz * ec.cos();
    let mut ra = eqy.atan2(gx) / DEG / 15.0;
    if ra < 0.0 {
        ra += 24.0;
    }
    RaDec {
        ra,
        dec: eqz.atan2((gx * gx + eqy * eqy).sqrt()) / DEG,
    }
}

// ---------------------------------------------------------------- the Sun (Meeus 25)

/// Geocentric apparent ecliptic longitude (deg) and distance (AU) of the Sun.
/// Port of the old `ecSunPos` (Meeus ch.25, ~1 arcsec).
pub fn sun_pos(jd: f64) -> (f64, f64) {
    let t = (jd - 2_451_545.0) / 36_525.0;
    let l0 = 280.466_46 + 36_000.769_83 * t + 0.000_303_2 * t * t;
    let m = 357.529_11 + 35_999.050_29 * t - 0.000_153_7 * t * t;
    let e = 0.016_708_634 - 0.000_042_037 * t - 0.000_000_126_7 * t * t;
    let c = (1.914_602 - 0.004_817 * t - 0.000_014 * t * t) * (m * DEG).sin()
        + (0.019_993 - 0.000_101 * t) * (2.0 * m * DEG).sin()
        + 0.000_289 * (3.0 * m * DEG).sin();
    let nu = m + c;
    let r = 1.000_001_018 * (1.0 - e * e) / (1.0 + e * (nu * DEG).cos());
    let om = 125.04 - 1934.136 * t;
    (rev(l0 + c - 0.005_69 - 0.004_78 * (om * DEG).sin()), r)
}

/// The Sun's apparent equatorial position — the comet's tail points away
/// from it (port of the old `sunRaDecApprox`).
pub fn sun_ra_dec(ms: i64) -> RaDec {
    let jd = julian_from_ms(ms);
    let (lon, _dist) = sun_pos(jd);
    let eps = nutation(jd).1;
    ecl_to_eq(lon, 0.0, eps)
}

// ---------------------------------------------------------------- nutation & time

/// Nutation in longitude (deg) + the true obliquity (deg).
/// Port of the old `ecNutation` (Meeus ch.22, low-precision terms).
fn nutation(jd: f64) -> (f64, f64) {
    let t = (jd - 2_451_545.0) / 36_525.0;
    let om = 125.044_52 - 1_934.136_261 * t;
    let ls = 280.466_5 + 36_000.769_8 * t;
    let lm = 218.316_5 + 481_267.881_3 * t;
    let dpsi = (-17.20 * (om * DEG).sin()
        - 1.32 * (2.0 * ls * DEG).sin()
        - 0.23 * (2.0 * lm * DEG).sin()
        + 0.21 * (2.0 * om * DEG).sin())
        / 3600.0;
    let deps = (9.20 * (om * DEG).cos()
        + 0.57 * (2.0 * ls * DEG).cos()
        + 0.10 * (2.0 * lm * DEG).cos()
        - 0.09 * (2.0 * om * DEG).cos())
        / 3600.0;
    let e0 = (23.0 * 3600.0 + 26.0 * 60.0 + 21.448 - 46.8150 * t - 0.000_59 * t * t + 0.001_813 * t * t * t)
        / 3600.0;
    (dpsi, e0 + deps)
}

/// Apparent Greenwich sidereal time (deg), port of the old `ecGAST`.
fn gast(jd: f64) -> f64 {
    let t = (jd - 2_451_545.0) / 36_525.0;
    let (dpsi, eps) = nutation(jd);
    let th = 280.460_618_37
        + 360.985_647_366_29 * (jd - 2_451_545.0)
        + 0.000_387_933 * t * t
        - t * t * t / 38_710_000.0;
    rev(th + dpsi * (eps * DEG).cos())
}

/// Ecliptic (lon, lat deg) → equatorial RA (deg) / Dec (deg).
fn ecl_to_eq(lon: f64, lat: f64, eps_deg: f64) -> RaDec {
    let (sl, cl) = ((lon * DEG).sin(), (lon * DEG).cos());
    let (sb, cb) = ((lat * DEG).sin(), (lat * DEG).cos());
    let (se, ce) = ((eps_deg * DEG).sin(), (eps_deg * DEG).cos());
    RaDec {
        ra: rev((sl * ce - (sb / cb) * se).atan2(cl) / DEG) / 15.0,
        dec: (sb * ce + cb * se * sl).asin() / DEG,
    }
}

// ---------------------------------------------------------------- the Moon (Meeus 47)

/// Moon table 47.A — D, M, M', F, Σl (1e-6 deg), Σr (1e-3 km).
#[rustfmt::skip]
const MOON_LR: [[i32; 6]; 60] = [
    [0,0,1,0,6288774,-20905355],[2,0,-1,0,1274027,-3699111],[2,0,0,0,658314,-2955968],
    [0,0,2,0,213618,-569925],[0,1,0,0,-185116,48888],[0,0,0,2,-114332,-3149],
    [2,0,-2,0,58793,246158],[2,-1,-1,0,57066,-152138],[2,0,1,0,53322,-170733],
    [2,-1,0,0,45758,-204586],[0,1,-1,0,-40923,-129620],[1,0,0,0,-34720,108743],
    [0,1,1,0,-30383,104755],[2,0,0,-2,15327,10321],[0,0,1,2,-12528,0],
    [0,0,1,-2,10980,79661],[4,0,-1,0,10675,-34782],[0,0,3,0,10034,-23210],
    [4,0,-2,0,8548,-21636],[2,1,-1,0,-7888,24208],[2,1,0,0,-6766,30824],
    [1,0,-1,0,-5163,-8379],[1,1,0,0,4987,-16675],[2,-1,1,0,4036,-12831],
    [2,0,2,0,3994,-10445],[4,0,0,0,3861,-11650],[2,0,-3,0,3665,14403],
    [0,1,-2,0,-2689,-7003],[2,0,-1,2,-2602,0],[2,-1,-2,0,2390,10056],
    [1,0,1,0,-2348,6322],[2,-2,0,0,2236,-9884],[0,1,2,0,-2120,5751],
    [0,2,0,0,-2069,0],[2,-2,-1,0,2048,-4950],[2,0,1,-2,-1773,4130],
    [2,0,0,2,-1595,0],[4,-1,-1,0,1215,-3958],[0,0,2,2,-1110,0],
    [3,0,-1,0,-892,3258],[2,1,1,0,-810,2616],[4,-1,-2,0,759,-1897],
    [0,2,-1,0,-713,-2117],[2,2,-1,0,-700,2354],[2,1,-2,0,691,0],
    [2,-1,0,-2,596,0],[4,0,1,0,549,-1423],[0,0,4,0,537,-1117],
    [4,-1,0,0,520,-1571],[1,0,-2,0,-487,-1739],[2,1,0,-2,-399,0],
    [0,0,2,-2,-381,-4421],[1,1,1,0,351,0],[3,0,-2,0,-340,0],
    [4,0,-3,0,330,0],[2,-1,2,0,327,0],[0,2,1,0,-323,1165],
    [1,1,-1,0,299,0],[2,0,3,0,294,0],[2,0,-1,-2,0,8752],
];

/// Moon table 47.B — D, M, M', F, Σb (1e-6 deg).
#[rustfmt::skip]
const MOON_B: [[i32; 5]; 60] = [
    [0,0,0,1,5128122],[0,0,1,1,280602],[0,0,1,-1,277693],[2,0,0,-1,173237],
    [2,0,-1,1,55413],[2,0,-1,-1,46271],[2,0,0,1,32573],[0,0,2,1,17198],
    [2,0,1,-1,9266],[0,0,2,-1,8822],[2,-1,0,-1,8216],[2,0,-2,-1,4324],
    [2,0,1,1,4200],[2,1,0,-1,-3359],[2,-1,-1,1,2463],[2,-1,0,1,2211],
    [2,-1,-1,-1,2065],[0,1,-1,-1,-1870],[4,0,-1,-1,1828],[0,1,0,1,-1794],
    [0,0,0,3,-1749],[0,1,-1,1,-1565],[1,0,0,1,-1491],[0,1,1,1,-1475],
    [0,1,1,-1,-1410],[0,1,0,-1,-1344],[1,0,0,-1,-1335],[0,0,3,1,1107],
    [4,0,0,-1,1021],[4,0,-1,1,833],[0,0,1,-3,777],[4,0,-2,1,671],
    [2,0,0,-3,607],[2,0,2,-1,596],[2,-1,1,-1,491],[2,0,-2,1,-451],
    [0,0,3,-1,439],[2,0,2,1,422],[2,0,-3,-1,421],[2,1,-1,1,-366],
    [2,1,0,1,-351],[4,0,0,1,331],[2,-1,1,1,315],[2,-2,0,-1,302],
    [0,0,1,3,-283],[2,1,1,-1,-229],[1,1,0,-1,223],[1,1,0,1,223],
    [0,1,-2,-1,-220],[2,1,-1,-1,-220],[1,0,1,1,-185],[2,-1,-2,-1,181],
    [0,1,2,1,-177],[4,0,-2,-1,176],[4,-1,-1,-1,166],[1,0,1,-1,-164],
    [4,0,1,-1,132],[1,0,-1,-1,-119],[4,-1,0,-1,115],[2,-2,0,1,107],
];

/// High-precision Moon: ecliptic lon (deg), lat (deg), distance (km).
/// Port of the old `ecMoonPos` (Meeus ch.47, 60 longitude/distance + 60
/// latitude terms). Used by the eclipse maths; the night sky's cheap Moon
/// position stays `crate::moon::ra_dec` (B12's reviewed behaviour).
pub fn moon_precise(jd: f64) -> (f64, f64, f64) {
    let t = (jd - 2_451_545.0) / 36_525.0;
    let (t2, t3, t4) = (t * t, t * t * t, t * t * t * t);
    let lp = 218.316_447_7 + 481_267.881_234_21 * t - 0.001_578_6 * t2 + t3 / 538_841.0 - t4 / 65_194_000.0;
    let d = 297.850_192_1 + 445_267.111_403_4 * t - 0.001_881_9 * t2 + t3 / 545_868.0 - t4 / 113_065_000.0;
    let m = 357.529_109_2 + 35_999.050_290_9 * t - 0.000_153_6 * t2 + t3 / 24_490_000.0;
    let mp = 134.963_396_4 + 477_198.867_505_5 * t + 0.008_741_4 * t2 + t3 / 69_699.0 - t4 / 14_712_000.0;
    let f = 93.272_095_0 + 483_202.017_523_3 * t - 0.003_653_9 * t2 - t3 / 3_526_000.0 + t4 / 863_310_000.0;
    let a1 = 119.75 + 131.849 * t;
    let a2 = 53.09 + 479_264.290 * t;
    let a3 = 313.45 + 481_266.484 * t;
    let ecc = 1.0 - 0.002_516 * t - 0.000_007_4 * t2;
    let (mut sl, mut sr, mut sb) = (0.0f64, 0.0f64, 0.0f64);
    for &row in MOON_LR.iter() {
        let arg = (row[0] as f64) * d + (row[1] as f64) * m + (row[2] as f64) * mp + (row[3] as f64) * f;
        let fct = if row[1] == 0 {
            1.0
        } else if row[1].abs() == 1 {
            ecc
        } else {
            ecc * ecc
        };
        sl += (row[4] as f64) * fct * (arg * DEG).sin();
        sr += (row[5] as f64) * fct * (arg * DEG).cos();
    }
    for &row in MOON_B.iter() {
        let arg = (row[0] as f64) * d + (row[1] as f64) * m + (row[2] as f64) * mp + (row[3] as f64) * f;
        let fct = if row[1] == 0 {
            1.0
        } else if row[1].abs() == 1 {
            ecc
        } else {
            ecc * ecc
        };
        sb += (row[4] as f64) * fct * (arg * DEG).sin();
    }
    sl += 3958.0 * (a1 * DEG).sin()
        + 1962.0 * ((lp - f) * DEG).sin()
        + 318.0 * (a2 * DEG).sin();
    sb += -2235.0 * (lp * DEG).sin()
        + 382.0 * (a3 * DEG).sin()
        + 175.0 * ((a1 - f) * DEG).sin()
        + 175.0 * ((a1 + f) * DEG).sin()
        + 127.0 * ((lp - mp) * DEG).sin()
        - 115.0 * ((lp + mp) * DEG).sin();
    (rev(lp + sl / 1e6), sb / 1e6, 385_000.56 + sr / 1000.0)
}

// ---------------------------------------------------------------- eclipses

const EC_AU: f64 = 149_597_870.7;
const EARTH_R: f64 = 6378.14;

/// ⚠ THE LOAD-BEARING FUNCTION of the solar eclipse: without it, every
/// location on Earth sees the same eclipse (the old code's comment, kept
/// because the bug it warns about is the easy one to reintroduce).
/// Geocentric → TOPOCENTRIC for the observer (Meeus ch.40).
#[allow(clippy::too_many_arguments)]
fn topo_ra_dec(ra_deg: f64, dec_deg: f64, dist_km: f64, jd: f64, lat: f64, lon: f64) -> (f64, f64, f64) {
    let u = (0.996_647_19 * (lat * DEG).tan()).atan();
    let rho_sin = 0.996_647_19 * u.sin();
    let rho_cos = u.cos();
    let sin_pi = EARTH_R / dist_km;
    let h = rev(gast(jd) + lon - ra_deg);
    let a = (dec_deg * DEG).cos() * (h * DEG).sin();
    let b = (dec_deg * DEG).cos() * (h * DEG).cos() - rho_cos * sin_pi;
    let c = (dec_deg * DEG).sin() - rho_sin * sin_pi;
    let q = (a * a + b * b + c * c).sqrt();
    (rev(ra_deg + h - a.atan2(b) / DEG), (c / q).asin() / DEG, q)
}

fn ang_sep(ra1: f64, dec1: f64, ra2: f64, dec2: f64) -> f64 {
    let c = (dec1 * DEG).sin() * (dec2 * DEG).sin() + (dec1 * DEG).cos() * (dec2 * DEG).cos() * ((ra1 - ra2) * DEG).cos();
    c.clamp(-1.0, 1.0).acos() / DEG
}

fn alt_of(ra_deg: f64, dec_deg: f64, jd: f64, lat: f64, lon: f64) -> f64 {
    let h = rev(gast(jd) + lon - ra_deg);
    let sin_alt = (dec_deg * DEG).sin() * (lat * DEG).sin()
        + (dec_deg * DEG).cos() * (lat * DEG).cos() * (h * DEG).cos();
    sin_alt.asin() / DEG
}

/// A solar eclipse for THIS observer at THIS instant: how much of the Sun is
/// hidden and from which side (`mag` = fraction of the diameter, `obsc` =
/// fraction of the area — what the light follows).
#[derive(Debug, Clone, Copy)]
pub struct SolarEclipse {
    pub mag: f64,
    pub obsc: f64,
    pub sun_alt: f64,
    pub total: bool,
    pub annular: bool,
    pub visible: bool,
    /// Which side the Moon is on, in units of the Sun's radius (so the dark
    /// disc slides the way it really does, not always left-to-right).
    pub dx: f64,
    pub dy: f64,
    /// The Moon's apparent radius in units of the Sun's (why some eclipses
    /// are annular).
    pub moon_ratio: f64,
}

/// A lunar eclipse: the Moon is inside Earth's shadow, so the umbral magnitude
/// is the SAME for everyone who can see it; only whether the Moon is up is
/// local.
#[derive(Debug, Clone, Copy)]
pub struct LunarEclipse {
    /// Umbral magnitude (1.0 = totality).
    pub umb_mag: f64,
    /// Penumbral magnitude.
    pub pen_mag: f64,
    pub moon_alt: f64,
    pub total: bool,
    pub partial: bool,
    pub visible: bool,
}

/// Both eclipse states for the instant — the Moon series is ~120 trig terms,
/// far too much per frame; the sky layer computes this at most once a minute.
#[derive(Debug, Clone, Copy)]
pub struct Eclipses {
    pub solar: SolarEclipse,
    pub lunar: LunarEclipse,
}

impl Eclipses {
    pub fn at(ms: i64, lat: f64, lon: f64) -> Self {
        Self {
            solar: solar_eclipse_at(ms, lat, lon),
            lunar: lunar_eclipse_at(ms, lat, lon),
        }
    }
}

pub fn solar_eclipse_at(ms: i64, lat: f64, lon: f64) -> SolarEclipse {
    let jd = julian_from_ms(ms);
    let eps = nutation(jd).1;
    let (sun_lon, sun_dist) = sun_pos(jd);
    let (moon_lon, moon_lat, moon_dist) = moon_precise(jd);
    let s_eq = ecl_to_eq(sun_lon, 0.0, eps);
    let m_eq = ecl_to_eq(moon_lon, moon_lat, eps);
    let (s_ra, s_dec, s_q) = topo_ra_dec(s_eq.ra * 15.0, s_eq.dec, sun_dist * EC_AU, jd, lat, lon);
    let (m_ra, m_dec, m_q) = topo_ra_dec(m_eq.ra * 15.0, m_eq.dec, moon_dist, jd, lat, lon);
    let r_sun = 959.63 / (sun_dist * s_q) / 3600.0;
    let r_moon = (0.272_481 * (EARTH_R / (moon_dist * m_q))).asin() / DEG;
    let d = ang_sep(s_ra, s_dec, m_ra, m_dec);
    let alt = alt_of(s_ra, s_dec, jd, lat, lon);
    let mut o = SolarEclipse {
        mag: 0.0,
        obsc: 0.0,
        sun_alt: alt,
        total: false,
        annular: false,
        visible: false,
        dx: 0.0,
        dy: 0.0,
        moon_ratio: r_moon / r_sun,
    };
    if d >= r_sun + r_moon {
        return o;
    }
    o.mag = (r_sun + r_moon - d) / (2.0 * r_sun);
    if d <= (r_moon - r_sun).abs() {
        o.obsc = if r_moon >= r_sun { 1.0 } else { (r_moon * r_moon) / (r_sun * r_sun) };
        o.total = r_moon >= r_sun;
        o.annular = r_moon < r_sun;
    } else {
        let (a, b) = (r_sun, r_moon);
        let c1 = (((d * d + a * a - b * b) / (2.0 * d * a)).clamp(-1.0, 1.0)).acos();
        let c2 = (((d * d + b * b - a * a) / (2.0 * d * b)).clamp(-1.0, 1.0)).acos();
        o.obsc = (a * a * (c1 - (2.0 * c1).sin() / 2.0) + b * b * (c2 - (2.0 * c2).sin() / 2.0))
            / (std::f64::consts::PI * a * a);
    }
    o.dx = (((m_ra - s_ra + 540.0) % 360.0) - 180.0) * (s_dec * DEG).cos() / r_sun;
    o.dy = (m_dec - s_dec) / r_sun;
    o.visible = o.obsc > 0.0 && alt > -0.5;
    o
}

pub fn lunar_eclipse_at(ms: i64, lat: f64, lon: f64) -> LunarEclipse {
    let jd = julian_from_ms(ms);
    let eps = nutation(jd).1;
    let (sun_lon, sun_dist) = sun_pos(jd);
    let (moon_lon, moon_lat, moon_dist) = moon_precise(jd);
    let d_lon =
        (((moon_lon - rev(sun_lon + 180.0) + 540.0) % 360.0) - 180.0) * (moon_lat * DEG).cos();
    let d_lat = moon_lat;
    let d = (d_lon * d_lon + d_lat * d_lat).sqrt();
    let pi_m = (EARTH_R / moon_dist).asin() / DEG;
    let pi_s = 8.794 / 3600.0 / sun_dist;
    let s_s = 959.63 / 3600.0 / sun_dist;
    let s_m = (0.272_481 * (pi_m * DEG).sin()).asin() / DEG;
    // 1.02 = the atmosphere's enlargement of the shadow.
    let f_pen = 1.02 * (0.998_340 * pi_m + pi_s + s_s);
    let f_umb = 1.02 * (0.998_340 * pi_m + pi_s - s_s);
    let m_eq = ecl_to_eq(moon_lon, moon_lat, eps);
    let alt = alt_of(m_eq.ra * 15.0, m_eq.dec, jd, lat, lon);
    let umb = (f_umb + s_m - d) / (2.0 * s_m);
    let pen = (f_pen + s_m - d) / (2.0 * s_m);
    LunarEclipse {
        umb_mag: umb,
        pen_mag: pen,
        moon_alt: alt,
        total: umb >= 1.0,
        partial: umb > 0.0 && umb < 1.0,
        visible: pen > 0.0 && alt > -0.5,
    }
}

// ---------------------------------------------------------------- meteor showers

/// The major annual showers — real peak windows + radiants + relative
/// strength (city.js `METEOR_SHOWERS`, verbatim).
pub struct Shower {
    pub name: &'static str,
    pub month: u8,
    pub d0: u8,
    pub d1: u8,
    /// Radiant RA (hours).
    pub ra: f64,
    /// Radiant Dec (deg).
    pub dec: f64,
    /// Relative strength.
    pub s: f64,
}

#[rustfmt::skip]
pub const SHOWERS: [Shower; 6] = [
    Shower { name: "QUADRANTID", month: 1,  d0: 1,  d1: 5,  ra: 15.33, dec: 49.0, s: 1.0 },
    Shower { name: "LYRID",      month: 4,  d0: 21, d1: 23, ra: 18.13, dec: 34.0, s: 0.45 },
    Shower { name: "PERSEID",    month: 8,  d0: 11, d1: 14, ra: 3.20,  dec: 58.0, s: 1.0 },
    Shower { name: "ORIONID",    month: 10, d0: 20, d1: 22, ra: 6.33,  dec: 16.0, s: 0.5 },
    Shower { name: "LEONID",     month: 11, d0: 16, d1: 18, ra: 10.13, dec: 22.0, s: 0.45 },
    Shower { name: "GEMINID",    month: 12, d0: 12, d1: 15, ra: 7.47,  dec: 33.0, s: 1.1 },
];

/// The shower whose window contains the calendar day of `ms`, if any.
pub fn current_shower(ms: i64) -> Option<&'static Shower> {
    let d = date_from_ms(ms);
    SHOWERS
        .iter()
        .find(|s| s.month == d.month && d.day >= s.d0 && d.day <= s.d1)
}

// The meteor slot machine (city.js 2503): every `MSLOT` of wall clock, up to
// 3 streaks (shower) or 1 (sporadic) are rolled from a hash of the slot index
// — deterministic, so every screen agrees and a render at a fixed instant is
// reproducible. A streak lives 850 ms at the head of its slot.
const METEOR_LIFE_MS: i64 = 850;

/// One meteor streak alive at `wall_ms`.
#[derive(Clone, Copy)]
pub struct Meteor {
    /// Head position in band units: x 0..1 across the width, y 0..1 down the
    /// sky band.
    pub x01: f32,
    pub y01: f32,
    /// Travel progress 0..1 through the streak's life.
    pub prog: f32,
    /// Travel distance in band-width units.
    pub travel: f32,
    /// 1 = part of the active shower (all streaks point away from the
    /// radiant), 0 = sporadic (random angle, from the same hash).
    pub shower: bool,
    /// Sporadic direction angle (radians); shower streaks ignore this.
    pub angle: f32,
    /// Peak brightness 0..1.
    pub bright: f32,
}

/// The streaks alive at `wall_ms` (at most 3), deterministic in the wall
/// clock. `ms_sky` decides which shower's window we are in. Port of the old
/// slot machine: all of a slot's streaks share the head-of-slot life window,
/// but each has its own hash (position, rate, direction).
pub fn meteors_now(wall_ms: i64, ms_sky: i64) -> Vec<Meteor> {
    let shower = current_shower(ms_sky);
    let slot_ms: i64 = if shower.is_some() { 3_200 } else { 20_000 };
    let slot = wall_ms.div_euclid(slot_ms);
    let into_slot = wall_ms - slot * slot_ms;
    if into_slot > METEOR_LIFE_MS {
        return Vec::new();
    }
    let count = if shower.is_some() { 3 } else { 1 };
    let mut out = Vec::with_capacity(count);
    for k in 0..count {
        // The old `midx = floor(now/MSLOT) + mk*997`.
        let idx = slot + k as i64 * 997;
        let (draw_at, y_roll, travel_roll, angle_roll) = meteor_hash(idx);
        // The old code: `if(mr()>(shower?0.85:0.5)) continue` — a streak is
        // drawn when the first roll is under the rate.
        let rate = if shower.is_some() { 0.85 } else { 0.5 };
        if draw_at > rate {
            continue;
        }
        out.push(Meteor {
            x01: y_roll as f32,
            y01: (0.04 + 0.32 * travel_roll) as f32,
            prog: into_slot as f32 / METEOR_LIFE_MS as f32,
            // The old `travel = 26+mr()*34` px on a ~400-px screen — kept as
            // a fraction of the band width so it scales.
            travel: (0.045 + 0.050 * angle_roll) as f32,
            shower: shower.is_some(),
            angle: (angle_roll * std::f64::consts::TAU) as f32,
            bright: if shower.is_some() { 0.9 } else { 0.55 },
        });
    }
    out
}

/// How long until the meteor layer next needs a look (the next slot boundary,
/// or the end of the current life window while a streak may still run) —
/// feeds the idle scheduler so a quiet night still sleeps.
pub fn next_meteor_due(wall_ms: i64, ms_sky: i64) -> i64 {
    let shower = current_shower(ms_sky);
    let slot_ms: i64 = if shower.is_some() { 3_200 } else { 20_000 };
    let into = wall_ms.rem_euclid(slot_ms);
    let to_next_slot = slot_ms - into;
    if into < METEOR_LIFE_MS {
        (METEOR_LIFE_MS - into).min(to_next_slot).max(1)
    } else {
        to_next_slot.max(1)
    }
}

/// Four cheap deterministic rolls from one 32-bit hash (the old code chained
/// `rng((midx*40503+13)>>>0)`; this is the same idea with one mix).
fn meteor_hash(idx: i64) -> (f64, f64, f64, f64) {
    let mut x = (idx as u32).wrapping_mul(40_503).wrapping_add(13);
    let next = |x: &mut u32| {
        *x = (*x ^ (*x >> 16)).wrapping_mul(0x045D_9F3B);
        *x = (*x ^ (*x >> 16)).wrapping_mul(0x045D_9F3B);
        *x ^= *x >> 16;
        (*x >> 8) as f64 / 16_777_216.0
    };
    let a = next(&mut x);
    let b = next(&mut x);
    let c = next(&mut x);
    let d = next(&mut x);
    (a, b, c, d)
}

// ---------------------------------------------------------------- a great comet

/// The apparitions (year-month) the great comet visits — the old
/// `COMET_SEASON`, reused verbatim: it visited April 2026 and returns October
/// 2027.
pub const COMET_SEASONS: [&str; 2] = ["2026-4", "2027-10"];

/// The comet's RA/Dec tonight, or `None` outside its seasons. Port of the old
/// `cometNow`: a fixed entry point per apparition (hashed from the month) and
/// a steady drift of ~1.4°/day, so it moves against the stars night to night.
pub fn comet_now(ms: i64) -> Option<RaDec> {
    let d = date_from_ms(ms);
    let key = format!("{}-{}", d.year, d.month);
    if !COMET_SEASONS.contains(&key.as_str()) {
        return None;
    }
    let seed = (d.year as i64 * 12 + d.month as i64)
        .wrapping_mul(2_654_435_761);
    let h = fract_hash(seed as u32);
    let ra0 = h * 24.0;
    let dec0 = -20.0 + fract_hash((seed >> 32) as u32 ^ 0x9E37) * 70.0;
    let day = d.day as f64 + d.hour as f64 / 24.0;
    Some(RaDec {
        ra: (ra0 + day * 0.093) % 24.0,
        dec: (dec0 - day * 0.55).clamp(-85.0, 85.0),
    })
}

fn fract_hash(x: u32) -> f64 {
    let mut x = x;
    x ^= x >> 16;
    x = x.wrapping_mul(0x045D_9F3B);
    x ^= x >> 16;
    x = x.wrapping_mul(0x045D_9F3B);
    x ^= x >> 16;
    (x >> 8) as f64 / 16_777_216.0
}

// ---------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;
    use crate::moon;

    const SITE: (f64, f64) = (41.0, -75.0);
    /// UT milliseconds for a UTC date-time (no chrono needed).
    fn ut(y: i64, mo: u32, d: u32, h: u32, mi: u32) -> i64 {
        crate::model::days_from_civil(y as i32, mo as i32, d as i32) * 86_400_000
            + (h as i64 * 3600 + mi as i64 * 60) * 1000
    }

    // ---- catalogue ----

    #[test]
    fn catalogue_ranges_and_named_present() {
        for s in CATALOG.iter() {
            assert!(s.ra_ms_h <= 24_000, "ra out of range");
            assert!(s.dec_cd >= -9000 && s.dec_cd <= 9000, "dec out of range");
        }
        // 500..1500 stars to mag ~4.5 (the brief's envelope).
        assert!((500..=1500).contains(&CATALOG.len()), "{} stars", CATALOG.len());
        assert_eq!(NAMED.len(), 47);
        for n in NAMED.iter() {
            let s = CATALOG[n.idx as usize];
            assert!(s.mag() < 3.5, "{} not a bright star", n.name);
        }
        // Brightest first in the old table: Sirius must be there and bright.
        let sirius = &NAMED.iter().find(|n| n.name == "Sirius").unwrap();
        assert!(CATALOG[sirius.idx as usize].mag() < -1.0);
    }

    #[test]
    fn star_colour_runs_blue_to_red() {
        let hot = star_colour(-0.2);
        let cool = star_colour(1.8);
        assert!(hot.2 > hot.0, "hot star should be blue: {hot:?}");
        assert!(cool.0 > cool.2, "cool star should be red: {cool:?}");
        let vega = star_colour(0.0);
        assert!(vega.2 >= vega.0, "A-type star should be white-blue");
    }

    #[test]
    fn polaris_altitude_matches_latitude() {
        // Polaris at the test site: alt ≈ lat ± 1.5°, az within 2° of north.
        let polaris = &NAMED.iter().find(|n| n.name == "Polaris").unwrap();
        let s = CATALOG[polaris.idx as usize];
        let lst = moon::lst_hours(ut(2026, 8, 12, 3, 0), SITE.1);
        let (alt, az) = moon::alt_az(s.ra_hours(), s.dec_deg(), lst, SITE.0);
        assert!((alt - SITE.0).abs() < 1.5, "Polaris alt {alt}");
        let az_n = if az > 180.0 { 360.0 - az } else { az };
        assert!(az_n < 2.0, "Polaris az {az}");
    }

    #[test]
    fn vega_transits_at_lst_equals_ra() {
        // At transit, alt = 90 - |lat - dec|.
        let vega = CATALOG[NAMED.iter().find(|n| n.name == "Vega").unwrap().idx as usize];
        let lst = vega.ra_hours();
        let (alt, _) = moon::alt_az(vega.ra_hours(), vega.dec_deg(), lst, SITE.0);
        let want = 90.0 - (SITE.0 - vega.dec_deg()).abs();
        assert!((alt - want).abs() < 0.01, "alt {alt} want {want}");
    }

    // ---- Milky Way ----

    #[test]
    fn galactic_centre_maps_to_sagittarius() {
        // The JS's verified anchors.
        let gc = gal_to_eq(0.0, 0.0);
        assert!((gc.ra - 17.76).abs() < 0.05, "gc ra {}", gc.ra);
        assert!((gc.dec - -28.94).abs() < 0.15, "gc dec {}", gc.dec);
        let ngp = gal_to_eq(0.0, 90.0);
        assert!((ngp.dec - 27.13).abs() < 0.1, "ngp dec {}", ngp.dec);
        let m31 = gal_to_eq(121.2, -21.6);
        assert!((m31.ra - 0.712).abs() < 0.05, "m31 ra {}", m31.ra);
        assert!((m31.dec - 41.27).abs() < 0.2, "m31 dec {}", m31.dec);
    }

    #[test]
    fn milky_way_is_built_once_and_stable() {
        let (spine, dabs) = milky_way();
        assert!((230..=250).contains(&spine.len()), "spine {}", spine.len());
        assert!((3000..=4500).contains(&dabs.len()), "dabs {}", dabs.len());
        // Deterministic: same pointer (OnceLock) and same first dab.
        let a = dabs[0];
        let (..) = milky_way();
        assert_eq!(milky_way().1[0].ra_ms_h, a.ra_ms_h);
        // The band runs all the way round the sky.
        let min_ra = dabs.iter().map(|d| d.ra_ms_h).min().unwrap();
        let max_ra = dabs.iter().map(|d| d.ra_ms_h).max().unwrap();
        assert!(min_ra < 1000 && max_ra > 23000, "band should wrap the sky");
        // H-alpha nebulae exist.
        assert!(dabs.iter().any(|d| d.class == 2));
        // Weights in range.
        for d in dabs.iter() {
            assert!(d.w_th <= 500);
        }
    }

    // ---- planets ----

    #[test]
    fn planets_stay_within_their_elongation_bounds() {
        // Physical invariants over a sample of dates: Mercury is never more
        // than ~28.5° from the Sun, Venus never more than ~47.5°.
        for day in [ut(2026, 1, 1, 0, 0), ut(2026, 8, 12, 3, 0), ut(2027, 3, 20, 0, 0)] {
            let sun = sun_ra_dec(day);
            for p in PLANETS {
                let rd = planet_ra_dec(p, day);
                let sep = ang_sep(rd.ra * 15.0, rd.dec, sun.ra * 15.0, sun.dec);
                match p {
                    Planet::Mercury => assert!(sep <= 29.0, "Mercury {sep:.1}° from the Sun"),
                    Planet::Venus => assert!(sep <= 48.0, "Venus {sep:.1}° from the Sun"),
                    _ => assert!(sep < 180.0, "{p:?} sep {sep}"),
                }
            }
        }
    }

    #[test]
    fn planet_positions_move_smoothly() {
        // RA/Dec of each planet changes by less than a degree a day (retrograde
        // included) — a broken Kepler solve jumps instead.
        for p in PLANETS {
            let a = planet_ra_dec(p, ut(2026, 11, 1, 0, 0));
            let b = planet_ra_dec(p, ut(2026, 11, 2, 0, 0));
            let dra = ((a.ra - b.ra + 12.0).rem_euclid(24.0) - 12.0).abs() * 15.0;
            assert!(dra < 1.3, "{p:?} moved {dra}°/day");
            assert!((a.dec - b.dec).abs() < 1.0, "{p:?} dec moved {}°/day", (a.dec - b.dec).abs());
        }
    }

    #[test]
    fn jupiter_saturn_are_where_the_almanac_has_them() {
        // JPL's Keplerian elements (the table above) at J2000.0: the
        // heliocentric distance must sit between perihelion and aphelion
        // a(1∓e) (a broken element row or Kepler solve moves it outside).
        let t = 0.0f64;
        let norm = |v: [f64; 3]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        for (p, a, e) in [
            (Planet::Mercury, 0.38710, 0.20564),
            (Planet::Jupiter, 5.20289, 0.04839),
            (Planet::Saturn, 9.53668, 0.05386),
        ] {
            let r = norm(planet_helio(p.elements(), t));
            assert!(
                r >= a * (1.0 - e) - 0.01 && r <= a * (1.0 + e) + 0.01,
                "{p:?} r={r} outside [{}, {}]",
                a * (1.0 - e),
                a * (1.0 + e)
            );
        }
    }

    // ---- Meeus sun/moon vs the book ----

    #[test]
    fn sun_matches_meeus_example_25a() {
        // Meeus 25.a: 1992-10-13 0h TD → apparent RA 13h13m31.4s = 198.3808°,
        // Dec −7.785 0° (the JS ignores ΔT; arcmin tolerance).
        let jd = 2_448_908.5;
        let (lon, _dist) = sun_pos(jd);
        let eps = nutation(jd).1;
        let eq = ecl_to_eq(lon, 0.0, eps);
        let ra_deg = eq.ra * 15.0;
        assert!((ra_deg - 198.3808).abs() < 0.02, "sun ra {ra_deg}");
        assert!((eq.dec - -7.785).abs() < 0.02, "sun dec {}", eq.dec);
    }

    #[test]
    fn moon_matches_meeus_example_47a() {
        // Meeus 47.a: 1992-04-12 0h TD → λ 133.162 7°, β −3.229 1°,
        // Δ 368 409.7 km (tolerance from dropping ΔT).
        let (lon, lat, dist) = moon_precise(2_448_724.5);
        assert!((lon - 133.1627).abs() < 0.01, "moon lon {lon}");
        assert!((lat - -3.2291).abs() < 0.01, "moon lat {lat}");
        assert!((dist - 368_409.7).abs() < 30.0, "moon dist {dist}");
    }

    #[test]
    fn precise_moon_agrees_with_the_cheap_series() {
        // The night sky's cheap Moon (world::moon::ra_dec, the B12 port) and
        // the Meeus Moon must agree to ~1.5° so the eclipse state matches the
        // drawn Moon.
        for ms in [ut(2026, 8, 12, 3, 0), ut(2025, 3, 14, 7, 0)] {
            let cheap = moon::ra_dec(ms);
            let jd = julian_from_ms(ms);
            let (lon, lat, _) = moon_precise(jd);
            let precise = ecl_to_eq(lon, lat, nutation(jd).1);
            let sep = ang_sep(cheap.ra * 15.0, cheap.dec, precise.ra * 15.0, precise.dec);
            assert!(sep < 1.5, "moon series disagree by {sep:.2}°");
        }
    }

    // ---- eclipses vs published circumstances ----

    #[test]
    fn total_solar_2024_buffalo_test_site_houston() {
        // Published circumstances (NASA/Espenak, validated the same way the
        // old JS was): 2024-04-08 — Buffalo TOTAL around 19:20 UTC,
        // the test site (41 N, 75 W) ≈ 90 % obscured (19:26 UTC), Houston ≈ 93 % (18:40 UTC).
        let buffalo = solar_eclipse_at(ut(2024, 4, 8, 19, 21), 42.8864, -78.8784);
        assert!(buffalo.total, "Buffalo should be total: {buffalo:?}");
        assert!(buffalo.obsc > 0.97, "Buffalo obsc {}", buffalo.obsc);
        let site = solar_eclipse_at(ut(2024, 4, 8, 19, 26), 41.0, -75.0);
        assert!(
            (site.obsc - 0.90).abs() < 0.06,
            "site obsc {}",
            site.obsc
        );
        assert!(site.visible && !site.total);
        let houston = solar_eclipse_at(ut(2024, 4, 8, 18, 40), 29.7604, -95.3698);
        assert!(
            (houston.obsc - 0.93).abs() < 0.06,
            "Houston obsc {}",
            houston.obsc
        );
    }

    #[test]
    fn total_lunar_2025_03_14() {
        // Published: greatest 06:59 UTC, umbral magnitude 1.148, visible from
        // the Americas. (The 60-term series + 4-term nutation land ~0.03
        // high on the magnitude — arcmin of position; tolerance covers it.)
        let t = ut(2025, 3, 14, 6, 59);
        let e = lunar_eclipse_at(t, 41.0, -75.0);
        assert!(e.visible, "moon alt {}", e.moon_alt);
        assert!((e.umb_mag - 1.148).abs() < 0.10, "umbMag {}", e.umb_mag);
        assert!(e.total);
        // The same instant over Tokyo: the same eclipse, but the Moon is down
        // — the local part of a lunar eclipse is only whether it is up.
        let tokyo = lunar_eclipse_at(t, 35.6762, 139.6503);
        assert!((tokyo.umb_mag - e.umb_mag).abs() < 1e-6);
        assert!(!tokyo.visible, "moon should be below the horizon in Tokyo");
        // Six hours earlier: not yet in shadow.
        let early = lunar_eclipse_at(t - 6 * 3_600_000, 41.0, -75.0);
        assert!(early.umb_mag < 0.0, "early umbMag {}", early.umb_mag);
    }

    #[test]
    fn partial_lunar_2026_08_28() {
        // Published: greatest 04:12:52 UTC, umbral magnitude 0.9319; the Moon
        // is high over the US east coast (it sets before Tokyo's evening).
        let t = ut(2026, 8, 28, 4, 13);
        let e = lunar_eclipse_at(t, 41.0, -75.0);
        assert!(e.visible, "moon alt {}", e.moon_alt);
        assert!(e.moon_alt > 30.0, "moon should be high: {}", e.moon_alt);
        assert!((e.umb_mag - 0.932).abs() < 0.08, "umbMag {}", e.umb_mag);
        assert!(e.partial && !e.total);
    }

    #[test]
    fn ordinary_days_have_no_eclipse() {
        for (mo, d) in [(6u32, 15u32), (9, 1)] {
            let t = ut(2026, mo, d, 12, 0);
            assert_eq!(solar_eclipse_at(t, 41.0, -75.0).obsc, 0.0);
            assert!(lunar_eclipse_at(t, 41.0, -75.0).pen_mag < 0.0);
        }
    }

    // ---- showers & meteors ----

    #[test]
    fn showers_match_their_real_dates() {
        assert_eq!(current_shower(ut(2026, 8, 12, 3, 0)).unwrap().name, "PERSEID");
        assert_eq!(current_shower(ut(2026, 12, 13, 3, 0)).unwrap().name, "GEMINID");
        assert!(current_shower(ut(2026, 7, 4, 3, 0)).is_none());
        // Perseid radiant: RA 3.2h, Dec +58 (Perseus).
        let p = current_shower(ut(2026, 8, 12, 3, 0)).unwrap();
        assert!((p.dec - 58.0).abs() < 0.5);
    }

    #[test]
    fn meteors_are_deterministic_and_slotted() {
        let wall = ut(2026, 8, 12, 3, 0);
        let a = meteors_now(wall, wall);
        let b = meteors_now(wall, wall);
        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(b.iter()) {
            assert_eq!((x.x01, x.y01, x.prog), (y.x01, y.y01, y.prog));
        }
        // During a shower, streaks are frequent across a whole night (≈ 3
        // rolls/slot at 85 % inside a 26 % life window ≈ 0.65 alive/sample).
        let mut alive = 0;
        let mut samples = 0;
        for i in 0..1000 {
            let m = meteors_now(wall + i * 977, wall);
            samples += 1;
            alive += m.len();
        }
        assert!(alive > samples / 2, "shower should be busy: {alive}/{samples}");
        // A quiet night is quiet: most samples empty.
        let quiet_day = ut(2026, 7, 4, 3, 0);
        let mut q_alive = 0;
        for i in 0..1000 {
            q_alive += meteors_now(quiet_day + i * 977, quiet_day).len();
        }
        assert!(q_alive < samples / 2, "sporadic rate too high: {q_alive}/{samples}");
        // Fields in range.
        for m in meteors_now(wall + 1234, wall) {
            assert!((0.0..=1.0).contains(&m.x01));
            assert!((0.0..=1.0).contains(&m.y01));
            assert!((0.0..=1.0).contains(&m.prog));
        }
    }

    #[test]
    fn next_meteor_due_is_bounded_by_the_slot() {
        let wall = ut(2026, 7, 4, 3, 0);
        let due = next_meteor_due(wall, wall);
        assert!(due >= 1 && due <= 20_000, "due {due}");
        let shower = ut(2026, 8, 12, 3, 0);
        let due = next_meteor_due(shower, shower);
        assert!(due >= 1 && due <= 3_200, "shower due {due}");
    }

    // ---- comet ----

    #[test]
    fn the_comet_visits_only_its_seasons() {
        // April 2026 and October 2027 (the old COMET_SEASON).
        let c = comet_now(ut(2026, 4, 20, 3, 0));
        assert!(c.is_some());
        let c = c.unwrap();
        assert!((0.0..24.0).contains(&c.ra));
        assert!((-85.0..=85.0).contains(&c.dec));
        assert!(comet_now(ut(2026, 11, 9, 3, 0)).is_none(), "today is not a season");
        assert!(comet_now(ut(2027, 10, 9, 3, 0)).is_some());
        // It drifts night to night (the whole point of the real track).
        let a = comet_now(ut(2026, 4, 10, 3, 0)).unwrap();
        let b = comet_now(ut(2026, 4, 20, 3, 0)).unwrap();
        let moved = ((a.ra - b.ra + 12.0).rem_euclid(24.0) - 12.0).abs() * 15.0
            + (a.dec - b.dec).abs();
        assert!(moved > 1.0, "comet did not drift: {moved}°");
    }

    #[test]
    fn gast_j2000_epoch_is_the_book_value() {
        // At J2000.0 the formula gives GMST 280.4606°.
        let g = gast(2_451_545.0);
        assert!((g - 280.4606).abs() < 0.01, "gast {g}");
    }
}
