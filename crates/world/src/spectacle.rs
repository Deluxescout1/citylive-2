//! Spectacle (B62, D75): the moments worth glancing at, as pure schedules.
//!
//! Nothing here draws. A schedule says *when* something happens (a street
//! festival, a parade, fireworks, the Friday night market, a hot-air balloon
//! morning, a rainbow, a side-monitor moment) from the calendar day, the city's
//! seed and the sky; the renderer turns an [`Active`] reading into pictures.
//! Everything is a function of its arguments, so a still frame and a test both
//! reproduce, and a day on which nothing is scheduled costs nothing at all.
//!
//! Days are civil day numbers (`model::days_from_civil`, 0 = 1970-01-01, a
//! Thursday). Hours are local, 0..24.

/// What can be scheduled for a day.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventKind {
    /// Stalls, a stage with a band, crowds and bunting on the square.
    StreetFestival,
    /// Floats and a marching band along the main avenue.
    Parade,
    /// A shell display over the waterfront or the main park.
    Fireworks,
    /// Lanterns and stalls along the square, every Friday evening.
    NightMarket,
}

/// One scheduled thing: its kind and when it starts and ends (local hours).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slot {
    pub kind: EventKind,
    pub start: f32,
    pub end: f32,
}

impl Slot {
    pub fn contains(&self, hour: f32) -> bool {
        hour >= self.start && hour < self.end
    }

    /// 0..1 through the slot.
    pub fn progress(&self, hour: f32) -> f32 {
        ((hour - self.start) / (self.end - self.start)).clamp(0.0, 0.9999)
    }
}

/// 0 = Sunday .. 6 = Saturday, from a civil day number.
pub fn weekday(day: i64) -> u32 {
    (day + 4).rem_euclid(7) as u32
}

/// A small, stable mixer (SplitMix64 finaliser).
pub fn mix(seed: u64, a: i64, salt: u64) -> u64 {
    let mut z = seed ^ (a as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ salt.wrapping_mul(0xD6E8_FEB8_6659_FD93);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn unit(h: u64) -> f32 {
    (h >> 40) as f32 / (1u64 << 24) as f32
}

/// The week a day belongs to (weeks start on Sunday).
fn week_of(day: i64) -> i64 {
    (day + 4).div_euclid(7)
}

/// Is this Saturday a festival Saturday? About every other week, so a month
/// holds two (sometimes three, sometimes one).
fn festival_saturday(seed: u64, day: i64) -> bool {
    weekday(day) == 6 && mix(seed, week_of(day), 0xFE57) % 2 == 0
}

/// Is this Sunday the month's parade? One Sunday in four, never the day after
/// a festival Saturday that also has the fireworks (the avenue is swept).
fn parade_sunday(seed: u64, day: i64) -> bool {
    weekday(day) == 0 && mix(seed, week_of(day), 0x9A4D) % 4 == 0
}

/// Everything scheduled on a day, in time order. Deterministic.
pub fn day_slots(seed: u64, day: i64) -> Vec<Slot> {
    let mut v = Vec::new();
    if festival_saturday(seed, day) {
        // Late morning to evening; the crowds peak mid afternoon.
        v.push(Slot { kind: EventKind::StreetFestival, start: 11.0, end: 19.0 });
        // The finale: the same night, a twenty-minute show (the renderer also
        // waits for the glow, so it is a fixed late slot).
        v.push(Slot { kind: EventKind::Fireworks, start: 21.0, end: 21.0 + 1200.0 / 3600.0 });
    }
    if parade_sunday(seed, day) {
        v.push(Slot { kind: EventKind::Parade, start: 13.0, end: 15.0 });
        // The parade Sunday ends with a short show over the park.
        v.push(Slot { kind: EventKind::Fireworks, start: 21.0, end: 21.25 });
    }
    if weekday(day) == 5 {
        v.push(Slot { kind: EventKind::NightMarket, start: 18.0, end: 23.0 });
    }
    v.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap());
    v
}

/// What is happening at an hour of a day.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Active {
    pub festival: Option<f32>,
    pub parade: Option<f32>,
    /// Seconds since the show began (a show is 15-20 minutes).
    pub fireworks: Option<f32>,
    pub night_market: Option<f32>,
}

impl Active {
    pub fn any(&self) -> bool {
        self.festival.is_some()
            || self.parade.is_some()
            || self.fireworks.is_some()
            || self.night_market.is_some()
    }
}

/// The events running at `hour`, each with its 0..1 progress.
pub fn active(seed: u64, day: i64, hour: f32) -> Active {
    let mut a = Active::default();
    for s in day_slots(seed, day) {
        if !s.contains(hour) {
            continue;
        }
        let p = Some(s.progress(hour));
        match s.kind {
            EventKind::StreetFestival => a.festival = p,
            EventKind::Parade => a.parade = p,
            EventKind::Fireworks => a.fireworks = Some((hour - s.start) * 3600.0),
            EventKind::NightMarket => a.night_market = p,
        }
    }
    a
}

/// Where the fireworks go up: over the water when the city has a waterfront,
/// else over the main park.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FireworkSite {
    Waterfront,
    Park,
}

/// The feed line announcing an event (once, when it starts).
pub fn feed_line(kind: EventKind, seed: u64, day: i64) -> String {
    let pick = |lines: &[&str]| lines[(mix(seed, day, kind as u64 + 1) % lines.len() as u64) as usize].to_string();
    match kind {
        EventKind::StreetFestival => pick(&[
            "Street festival on the square: stalls, a band on the stage, and half the town.",
            "The square is hung with bunting for today's festival.",
            "Festival day: the band is warming up and the stalls are open.",
        ]),
        EventKind::Parade => pick(&[
            "The parade is coming down the avenue: floats, a marching band, crowds on the kerbs.",
            "A parade along the main avenue this afternoon.",
        ]),
        EventKind::Fireworks => pick(&[
            "Fireworks tonight: people are gathering to watch.",
            "The fireworks begin: crowds are looking up.",
        ]),
        EventKind::NightMarket => pick(&[
            "The Friday night market has lit its lanterns.",
            "Night market on the square: lanterns, food stalls and slow strolling.",
        ]),
    }
}

// ---- the sky -------------------------------------------------------------

/// The slice of the real sky the sky spectacles care about.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkyNow {
    /// Degrees above the horizon, azimuth clockwise from north.
    pub sun_alt: f32,
    pub sun_az: f32,
    /// 0..1 cloud cover (already translated for the biome).
    pub cloud: f32,
    pub wind_kmh: f32,
    /// Where the wind comes *from*, degrees.
    pub wind_from: f32,
    /// Falling now.
    pub precip: bool,
    /// Thunder about.
    pub storm: bool,
    /// Minutes since the rain last stopped (large when it has not rained).
    pub since_rain_min: f32,
    /// True when the real Moon is in a lunar eclipse (any phase of the umbra).
    pub lunar_eclipse: bool,
}

impl Default for SkyNow {
    fn default() -> Self {
        Self {
            sun_alt: 30.0,
            sun_az: 180.0,
            cloud: 0.0,
            wind_kmh: 5.0,
            wind_from: 270.0,
            precip: false,
            storm: false,
            since_rain_min: 9999.0,
            lunar_eclipse: false,
        }
    }
}

/// A morning for hot-air balloons: the sun just up, the air calm and clear.
pub fn balloon_morning(s: &SkyNow, hour: f32) -> bool {
    hour < 12.0
        && (1.0..=34.0).contains(&s.sun_alt)
        && s.cloud < 0.45
        && s.wind_kmh < 14.0
        && !s.precip
        && !s.storm
}

/// One balloon's flight over a morning.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Flight {
    /// Local hour it lifts off.
    pub launch: f32,
    /// Minutes aloft, lift-off to touch-down.
    pub minutes: f32,
    /// A colour scheme id and a size (1.0 = the common envelope).
    pub scheme: u8,
    pub size: f32,
    /// 0..1 across the launch field's width (where in the field it starts).
    pub spot: f32,
}

/// The balloons that fly on a day (2..=4), launched in turn after sunrise.
/// `sunrise` is the local hour of sunrise.
pub fn balloon_flights(seed: u64, day: i64, sunrise: f32) -> Vec<Flight> {
    let n = 2 + (mix(seed, day, 0xBA11) % 3) as usize;
    let mut v = Vec::with_capacity(n);
    for i in 0..n {
        let h = mix(seed, day, 0xBA20 + i as u64);
        v.push(Flight {
            launch: sunrise + 0.25 + i as f32 * 0.22 + unit(h) * 0.08,
            minutes: 38.0 + unit(h >> 7) * 18.0,
            scheme: (h >> 13) as u8 % 6,
            size: 0.9 + unit(h >> 19) * 0.3,
            spot: unit(h >> 27),
        });
    }
    v
}

impl Flight {
    pub fn end(&self) -> f32 {
        self.launch + self.minutes / 60.0
    }

    /// 0..1 through the flight at `hour`, or `None` when it is not aloft.
    pub fn progress(&self, hour: f32) -> Option<f32> {
        (hour >= self.launch && hour < self.end())
            .then(|| (hour - self.launch) / (self.minutes / 60.0))
    }
}

/// Height of a balloon over its flight, 0..1: inflates and lifts in the first
/// sixth, cruises, sinks and touches down in the last quarter. Smooth.
pub fn balloon_lift(p: f32) -> f32 {
    let up = (p / 0.16).clamp(0.0, 1.0);
    let down = ((1.0 - p) / 0.24).clamp(0.0, 1.0);
    let s = |x: f32| x * x * (3.0 - 2.0 * x);
    s(up) * s(down)
}

/// A rainbow: strength 0..1, or 0 when conditions are wrong.
///
/// It needs the sun low and **behind** the viewer. The camera looks south, so
/// the sun has to be in the northern half of the sky (the antisolar point, the
/// middle of the bow, then falls on the southern sky the picture shows). It
/// shows while rain is falling through sun (thin cloud) or for about forty
/// minutes after it stops, then fades.
pub fn rainbow(s: &SkyNow) -> f32 {
    if s.sun_alt < 3.0 || s.sun_alt > 40.0 {
        return 0.0;
    }
    // Sun azimuth within 80 degrees of north.
    let d = ((s.sun_az + 180.0).rem_euclid(360.0) - 180.0).abs();
    if d > 80.0 {
        return 0.0;
    }
    let wet = if s.precip {
        // A sun shower: only where the cloud is broken.
        if s.cloud < 0.85 { 0.8 } else { 0.0 }
    } else if s.since_rain_min < 45.0 {
        1.0 - (s.since_rain_min / 45.0)
    } else {
        0.0
    };
    // A bow is brightest with the sun low (the higher the sun, the lower and
    // flatter the arc).
    let low = 1.0 - ((s.sun_alt - 3.0) / 37.0).clamp(0.0, 1.0) * 0.6;
    // The bow gets sharper toward being directly behind.
    let behind = 1.0 - d / 120.0;
    (wet * low * behind).clamp(0.0, 1.0)
}

/// Is today's storm a spectacular one (forked bolts that reach the ground, a
/// brief double flash)? About one thunderstorm in three.
pub fn spectacular_storm(seed: u64, day: i64) -> bool {
    mix(seed, day, 0x570A) % 3 == 0
}

// ---- side-monitor moments -------------------------------------------------

/// Small, gentle things placed on a side monitor, where the eyes rest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SideKind {
    FoxCrossing,
    KiteFlying,
    BalloonLanding,
    StreetMusician,
}

/// One side-monitor moment: starts at `start` (hour), lasts `seconds`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SideEvent {
    pub kind: SideKind,
    pub start: f32,
    pub seconds: f32,
    /// Which side head (0 = the first side zone) and where along it, 0..1.
    pub head: u8,
    pub spot: f32,
}

/// A few a day, at times that suit them (a fox at dawn or dusk, kites on a
/// breezy afternoon, a musician in the evening), never in rain.
pub fn side_events(seed: u64, day: i64, heads: u8) -> Vec<SideEvent> {
    if heads == 0 {
        return Vec::new();
    }
    let mut v = Vec::new();
    let mut add = |kind: SideKind, salt: u64, lo: f32, hi: f32, secs: f32| {
        let h = mix(seed, day, salt);
        // About three days in four have one of each kind.
        if h % 4 == 3 {
            return;
        }
        v.push(SideEvent {
            kind,
            start: lo + unit(h >> 8) * (hi - lo),
            seconds: secs,
            head: ((h >> 33) % heads as u64) as u8,
            spot: 0.2 + unit(h >> 20) * 0.6,
        });
    };
    add(SideKind::FoxCrossing, 0x51, 5.0, 7.5, 40.0);
    add(SideKind::FoxCrossing, 0x52, 19.0, 21.5, 40.0);
    add(SideKind::KiteFlying, 0x53, 13.0, 17.0, 150.0);
    add(SideKind::StreetMusician, 0x54, 18.0, 21.0, 240.0);
    // A balloon sets down on the side screen on balloon mornings only; the
    // caller drops it when the sky says no (`balloon_morning`).
    add(SideKind::BalloonLanding, 0x55, 7.0, 9.0, 120.0);
    v.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap());
    v
}

impl SideEvent {
    /// 0..1 through the event at `hour`, or `None` when it is not on.
    pub fn progress(&self, hour: f32) -> Option<f32> {
        let len = self.seconds / 3600.0;
        (hour >= self.start && hour < self.start + len).then(|| (hour - self.start) / len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::days_from_civil;

    fn month(seed: u64, y: i32, m: i32) -> Vec<(i64, Vec<Slot>)> {
        let d0 = days_from_civil(y, m, 1);
        (0..30).map(|i| (d0 + i, day_slots(seed, d0 + i))).collect()
    }

    #[test]
    fn weekday_matches_the_calendar() {
        // 2026-11-10 is a Tuesday; 1970-01-01 a Thursday.
        assert_eq!(weekday(days_from_civil(2026, 11, 10)), 2);
        assert_eq!(weekday(0), 4);
        assert_eq!(weekday(days_from_civil(2026, 11, 8)), 0);
    }

    #[test]
    fn a_month_has_a_few_festivals_one_ish_parade_and_a_market_every_friday() {
        for seed in [1u64, 7, 42, 9001] {
            let mut total_fest = 0;
            let mut total_parade = 0;
            let mut total_fw = 0;
            for (y, m) in [(2026, 3), (2026, 4), (2026, 5), (2026, 6), (2026, 7), (2026, 8)] {
                let mo = month(seed, y, m);
                let fridays = mo.iter().filter(|(d, _)| weekday(*d) == 5).count();
                let markets = mo
                    .iter()
                    .filter(|(_, s)| s.iter().any(|x| x.kind == EventKind::NightMarket))
                    .count();
                assert_eq!(fridays, markets, "a market every Friday");
                total_fest += mo.iter().filter(|(_, s)| s.iter().any(|x| x.kind == EventKind::StreetFestival)).count();
                total_parade += mo.iter().filter(|(_, s)| s.iter().any(|x| x.kind == EventKind::Parade)).count();
                total_fw += mo.iter().filter(|(_, s)| s.iter().any(|x| x.kind == EventKind::Fireworks)).count();
            }
            // Six months: roughly 2/month festivals, 1/month parades, 3/month shows.
            assert!((6..=18).contains(&total_fest), "seed {seed}: {total_fest} festivals in 6 months");
            assert!((2..=12).contains(&total_parade), "seed {seed}: {total_parade} parades");
            assert!(total_fw >= total_fest, "every festival ends with fireworks");
        }
    }

    #[test]
    fn schedules_are_deterministic_and_seeded() {
        let d = days_from_civil(2026, 7, 4);
        assert_eq!(day_slots(5, d), day_slots(5, d));
        let a: Vec<_> = (0..200).map(|i| day_slots(5, d + i).len()).collect();
        let b: Vec<_> = (0..200).map(|i| day_slots(6, d + i).len()).collect();
        assert_ne!(a, b, "two cities do not party on the same days");
    }

    #[test]
    fn active_reads_progress_and_is_empty_when_nothing_is_on() {
        let seed = 3;
        let d = (0..60)
            .map(|i| days_from_civil(2026, 5, 1) + i)
            .find(|d| festival_saturday(seed, *d))
            .expect("a festival Saturday in sixty days");
        assert!(active(seed, d, 9.0).festival.is_none());
        let p = active(seed, d, 15.0).festival.unwrap();
        assert!((0.4..0.6).contains(&p), "{p}");
        let secs = active(seed, d, 21.1).fireworks.unwrap();
        assert!((300.0..420.0).contains(&secs), "{secs}");
        assert!(active(seed, d, 21.5).fireworks.is_none(), "the show is over by 21:20");
        // A Wednesday is empty.
        let wed = (0..14).map(|i| d + i).find(|x| weekday(*x) == 3).unwrap();
        for h in 0..24 {
            assert!(!active(seed, wed, h as f32 + 0.5).any());
        }
    }

    #[test]
    fn slots_never_overlap_themselves_and_stay_in_the_day() {
        for seed in 0..20u64 {
            for i in 0..100 {
                let s = day_slots(seed, 20_000 + i);
                for w in s.windows(2) {
                    if w[0].kind == w[1].kind {
                        assert!(w[0].end <= w[1].start);
                    }
                }
                for x in &s {
                    assert!(x.start >= 0.0 && x.end <= 24.0 && x.start < x.end);
                }
            }
        }
    }

    #[test]
    fn balloons_fly_on_calm_clear_mornings_only() {
        let calm = SkyNow { sun_alt: 12.0, cloud: 0.1, wind_kmh: 6.0, ..SkyNow::default() };
        assert!(balloon_morning(&calm, 7.5));
        assert!(!balloon_morning(&calm, 15.0), "not in the afternoon");
        assert!(!balloon_morning(&SkyNow { wind_kmh: 25.0, ..calm }, 7.5), "too windy");
        assert!(!balloon_morning(&SkyNow { cloud: 0.8, ..calm }, 7.5), "overcast");
        assert!(!balloon_morning(&SkyNow { precip: true, ..calm }, 7.5));
        assert!(!balloon_morning(&SkyNow { sun_alt: -3.0, ..calm }, 5.5), "before sunrise");
    }

    #[test]
    fn a_balloon_flight_rises_cruises_and_lands() {
        let f = balloon_flights(9, 20_500, 6.0);
        assert!((2..=4).contains(&f.len()));
        for fl in &f {
            assert!(fl.launch > 6.0 && fl.end() < 9.0);
            assert!(fl.progress(fl.launch - 0.01).is_none());
            assert!(fl.progress(fl.end() + 0.01).is_none());
            let mid = fl.progress(fl.launch + fl.minutes / 120.0).unwrap();
            assert!((mid - 0.5).abs() < 0.01);
        }
        assert_eq!(balloon_lift(0.0), 0.0);
        assert!(balloon_lift(0.5) > 0.99);
        assert!(balloon_lift(0.999) < 0.01, "touches down");
        // Smooth: no step larger than a few percent per 1/200 of the flight.
        let mut last = 0.0;
        for i in 0..=200 {
            let v = balloon_lift(i as f32 / 200.0);
            assert!((v - last).abs() < 0.08);
            last = v;
        }
    }

    #[test]
    fn a_rainbow_needs_wet_air_low_sun_behind_the_viewer() {
        let base = SkyNow { sun_alt: 15.0, sun_az: 320.0, cloud: 0.5, since_rain_min: 10.0, ..SkyNow::default() };
        assert!(rainbow(&base) > 0.3);
        assert_eq!(rainbow(&SkyNow { since_rain_min: 120.0, ..base }), 0.0, "dry air");
        assert_eq!(rainbow(&SkyNow { sun_az: 180.0, ..base }), 0.0, "sun in front of the camera");
        assert_eq!(rainbow(&SkyNow { sun_alt: 60.0, ..base }), 0.0, "sun too high");
        assert_eq!(rainbow(&SkyNow { sun_alt: 1.0, ..base }), 0.0, "sun on the horizon");
        assert_eq!(rainbow(&SkyNow { precip: true, cloud: 0.95, ..base }), 0.0, "heavy rain, no sun");
        assert!(rainbow(&SkyNow { precip: true, cloud: 0.5, ..base }) > 0.3, "sun shower");
        // It fades as the rain recedes.
        let a = rainbow(&SkyNow { since_rain_min: 5.0, ..base });
        let b = rainbow(&SkyNow { since_rain_min: 40.0, ..base });
        assert!(a > b && b > 0.0);
    }

    #[test]
    fn side_moments_are_rare_gentle_and_on_a_real_head() {
        assert!(side_events(1, 20_000, 0).is_empty());
        let mut per_day = Vec::new();
        for d in 20_000..20_100 {
            let ev = side_events(11, d, 2);
            per_day.push(ev.len());
            for e in &ev {
                assert!(e.head < 2 && (0.0..=1.0).contains(&e.spot));
                assert!(e.seconds <= 300.0, "gentle: no long events");
                match e.kind {
                    SideKind::FoxCrossing => assert!(!(8.0..18.5).contains(&e.start)),
                    SideKind::KiteFlying => assert!((12.0..18.0).contains(&e.start)),
                    SideKind::StreetMusician => assert!((17.5..21.5).contains(&e.start)),
                    SideKind::BalloonLanding => assert!((6.5..9.5).contains(&e.start)),
                }
            }
        }
        let avg = per_day.iter().sum::<usize>() as f32 / per_day.len() as f32;
        assert!((2.0..=5.0).contains(&avg), "a few a day, got {avg}");
    }

    #[test]
    fn feed_lines_exist_for_every_kind() {
        for k in [EventKind::StreetFestival, EventKind::Parade, EventKind::Fireworks, EventKind::NightMarket] {
            assert!(feed_line(k, 3, 20_000).len() > 20);
        }
    }
}
