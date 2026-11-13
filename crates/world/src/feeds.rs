//! Feed on/off switches and the shared-bucket refresh policy for the live
//! world feeds (B44 / D67): flights, aurora Kp, ISS, sports scores.
//!
//! Every poll follows the old CityLive `maybeFetch*` discipline (city.js):
//! one request per wall-clock bucket of the feed's interval, a bounded
//! retry gap when a fetch failed, and a graceful offline stand-in when the
//! network is down.

/// Per-feed on/off switches (D67: "each one toggleable in settings").
/// All feeds are ON by default, matching the old CityLive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Feeds {
    /// Real aircraft near the location (airplanes.live, B44).
    pub flights: bool,
    /// Planetary Kp / aurora (NOAA SWPC, B44).
    pub aurora: bool,
    /// Real ISS sub-satellite point (open-notify, B44).
    pub iss: bool,
    /// Live NFL scores (ESPN, B44).
    pub sports: bool,
}

impl Default for Feeds {
    fn default() -> Self {
        Self {
            flights: true,
            aurora: true,
            iss: true,
            sports: true,
        }
    }
}

impl Feeds {
    /// Every feed off (the "no live data" setting).
    pub const ALL_OFF: Self = Self {
        flights: false,
        aurora: false,
        iss: false,
        sports: false,
    };
}

/// Minimum wall-clock interval between refreshes of each feed, taken
/// verbatim from old `city.js`:
///
/// * `FL_BUCKET = 90000` — flights (B44 floors the refresh at 30 s; 90 s
///   satisfies it),
/// * `KP_BUCKET = 1800000` — aurora (30 minutes, per the brief),
/// * `25000` — ISS (old `maybeFetchISS`),
/// * `SCORE_BUCKET = 120000` — sports (old `maybeFetchScores`).
pub const FLIGHTS_REFRESH_MS: i64 = 90_000;
pub const AURORA_REFRESH_MS: i64 = 1_800_000;
pub const ISS_REFRESH_MS: i64 = 25_000;
pub const SPORTS_REFRESH_MS: i64 = 120_000;

/// Shared-bucket re-request policy (the old `maybeFetch*` gate).
///
/// At wall time `now_ms` with a feed interval `interval_ms`, a fetch is due
/// when the bucket `floor(now / interval)` has turned, **or** when there was
/// no successful fetch in *this* bucket yet and at least `retry_ms` have
/// passed since the last attempt (the old 25/60/8/15 s per-feed re-try gaps).
#[derive(Debug, Clone, Copy)]
pub struct Poller {
    bucket: i64,
    ok_bucket: i64,
    last_req_ms: i64,
}

impl Default for Poller {
    fn default() -> Self {
        Self::new()
    }
}

impl Poller {
    /// No fetches yet (the old JS initialised `*Bucket = -1`).
    pub fn new() -> Self {
        Self {
            bucket: -1,
            ok_bucket: -1,
            last_req_ms: 0,
        }
    }

    /// Should this feed fetch at `now_ms`?  Records the attempt on yes.
    pub fn due(&mut self, now_ms: i64, interval_ms: i64, retry_ms: i64) -> bool {
        let b = now_ms / interval_ms;
        if b != self.bucket || (self.ok_bucket != b && now_ms - self.last_req_ms > retry_ms) {
            self.bucket = b;
            self.last_req_ms = now_ms;
            true
        } else {
            false
        }
    }

    /// A fetch at `now_ms` succeeded (old `*OkBucket = bucket`), which
    /// suppresses in-bucket retries.
    pub fn ok(&mut self, now_ms: i64, interval_ms: i64) {
        self.ok_bucket = now_ms / interval_ms;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feeds_on_by_default() {
        let f = Feeds::default();
        assert!(
            f.flights && f.aurora && f.iss && f.sports,
            "all feeds default on (B44: off-switches, not opt-ins)"
        );
        assert_ne!(Feeds::default(), Feeds::ALL_OFF);
        assert!(!Feeds::ALL_OFF.flights && !Feeds::ALL_OFF.sports);
    }

    #[test]
    fn intervals() {
        assert!(FLIGHTS_REFRESH_MS >= 30_000, "B44 floors flights at 30 s");
        assert_eq!(AURORA_REFRESH_MS, 30 * 60_000, "aurora refresh is 30 min");
        assert_eq!(ISS_REFRESH_MS, 25_000);
        assert_eq!(SPORTS_REFRESH_MS, 120_000);
    }

    #[test]
    fn poller_fires_first_call_then_holds_bucket() {
        let mut p = Poller::new();
        assert!(p.due(100_000, 90_000, 25_000), "first call is due");
        assert!(!p.due(120_000, 90_000, 25_000), "same bucket, no success, retry gap not met");
        assert!(p.due(190_000, 90_000, 25_000), "new bucket is due");
    }

    #[test]
    fn poller_retries_within_bucket_after_gap() {
        let mut p = Poller::new();
        let t0 = 100_000; // bucket 1
        assert!(p.due(t0, 90_000, 25_000));
        assert!(!p.due(t0 + 10_000, 90_000, 25_000));
        assert!(!p.due(t0 + 20_000, 90_000, 25_000));
        assert!(
            p.due(t0 + 26_000, 90_000, 25_000),
            "retry gap exceeded -> due again in the same bucket"
        );
    }

    #[test]
    fn poller_ok_suppresses_in_bucket_retry() {
        let mut p = Poller::new();
        let t0 = 100_000;
        assert!(p.due(t0, 90_000, 25_000));
        p.ok(t0, 90_000);
        assert!(
            !p.due(t0 + 30_000, 90_000, 25_000),
            "in-bucket with a success: no re-fetch (old flOkBucket gate)"
        );
        assert!(p.due(t0 + 90_000, 90_000, 25_000), "next bucket turns");
    }
}
