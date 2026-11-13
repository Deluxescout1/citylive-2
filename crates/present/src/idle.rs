//! Away pacing (B53 item 3, D68): while the owner is away (his session is idle) the
//! city is QUIETER: a calmer pace, and fewer people and cars out. The city is
//! busier while he is active. It never runs faster when he is away.
//!
//! The only signal is the local session's idle hint from logind
//! (`org.freedesktop.login1.Session.IdleHint`, with `IdleSinceHintMonotonic`
//! for how long).  Nothing is sent anywhere and nothing is read but that one
//! property of this user's own session.  The desktop sets the hint (screen
//! saver / lock); if logind or the system bus is missing, the watch reports
//! "unknown" and the pace stays normal.
//!
//! The calm is gentle by design: nothing happens for the first [`IDLE_AFTER`];
//! after that the pace eases linearly down to [`MIN_CALM`] times normal by
//! [`IDLE_FULL`] away, and it returns to normal straight away when activity
//! returns.  It is a settings switch (`quiet_when_away`, off by default).

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Idle time before the city goes quieter.
pub const IDLE_AFTER: Duration = Duration::from_secs(5 * 60);
/// Idle time at which the full calm applies.
pub const IDLE_FULL: Duration = Duration::from_secs(30 * 60);
/// The calmest pace while away: 0.6x normal (never faster than normal).
pub const MIN_CALM: f64 = 0.6;
/// How often the watch re-reads the session hint.
pub const POLL: Duration = Duration::from_secs(10);

/// B62 / D68: while away the city is quieter, not only slower. The share of
/// residents and vehicles drawn falls to this at full calm (never to nothing:
/// a city asleep still has its night shift).
pub const MIN_CROWD: f32 = 0.5;

/// The share of people and vehicles to show for this away time, in
/// `MIN_CROWD..=1.0`; 1.0 when active, unknown, or the switch is off. Follows
/// the same ramp as [`quiet_factor`].
pub fn crowd_factor(enabled: bool, idle_for: Option<Duration>) -> f32 {
    let calm = quiet_factor(enabled, idle_for);
    let ramp = ((1.0 - calm) / (1.0 - MIN_CALM)) as f32;
    1.0 - (1.0 - MIN_CROWD) * ramp.clamp(0.0, 1.0)
}

/// Keep `share` (0..=1) of a list, a stable subset: the same items stay as long
/// as the order does, and the share changes in sixteenths so a thinning crowd
/// loses a few figures at a time rather than reshuffling.
pub fn thin<T>(items: &mut Vec<T>, share: f32) {
    let keep = (share.clamp(0.0, 1.0) * 16.0).round() as usize;
    if keep >= 16 {
        return;
    }
    let mut i = 0usize;
    items.retain(|_| {
        let k = (i * 7) % 16 < keep;
        i += 1;
        k
    });
}

/// The sim pace multiplier for this away time, always in `MIN_CALM..=1.0`.
/// Active, unknown, under [`IDLE_AFTER`], or the switch off: 1.0.
pub fn quiet_factor(enabled: bool, idle_for: Option<Duration>) -> f64 {
    if !enabled {
        return 1.0;
    }
    let Some(d) = idle_for else {
        return 1.0;
    };
    if d < IDLE_AFTER {
        return 1.0;
    }
    let span = (IDLE_FULL - IDLE_AFTER).as_secs_f64();
    let ramp = ((d - IDLE_AFTER).as_secs_f64() / span).clamp(0.0, 1.0);
    1.0 - (1.0 - MIN_CALM) * ramp
}

/// Turn the raw logind reading into "idle for how long".  `since_us` is the
/// monotonic microsecond stamp of when the hint went true; `now_us` is the
/// monotonic clock now.  A missing or future stamp counts as just-idle.
pub fn idle_from_hint(hint: bool, since_us: u64, now_us: u64) -> Option<Duration> {
    if !hint {
        return None;
    }
    if since_us == 0 || since_us > now_us {
        return Some(Duration::ZERO);
    }
    Some(Duration::from_micros(now_us - since_us))
}

/// Sentinel for "active or unknown" in the shared cell.
const NOT_IDLE: u64 = u64::MAX;

/// A background reader of the logind idle hint.  Dropping it stops the thread
/// at its next poll.
pub struct IdleWatch {
    /// Idle time in microseconds, or [`NOT_IDLE`].
    shared: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
}

impl IdleWatch {
    /// Start the reader thread.
    pub fn spawn() -> Self {
        let shared = Arc::new(AtomicU64::new(NOT_IDLE));
        let stop = Arc::new(AtomicBool::new(false));
        let (s, st) = (shared.clone(), stop.clone());
        std::thread::Builder::new()
            .name("idle-watch".into())
            .spawn(move || run(s, st))
            .ok();
        Self { shared, stop }
    }

    /// How long the session has been idle, or `None` when active or unknown.
    pub fn idle_for(&self) -> Option<Duration> {
        match self.shared.load(Ordering::Relaxed) {
            NOT_IDLE => None,
            us => Some(Duration::from_micros(us)),
        }
    }
}

impl Drop for IdleWatch {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn run(shared: Arc<AtomicU64>, stop: Arc<AtomicBool>) {
    // Give up for good after a few failures: no logind here means no signal.
    let mut failures = 0u32;
    while !stop.load(Ordering::Relaxed) {
        match query() {
            Some(idle) => {
                failures = 0;
                let v = match idle {
                    Some(d) => d.as_micros().min(NOT_IDLE as u128 - 1) as u64,
                    None => NOT_IDLE,
                };
                shared.store(v, Ordering::Relaxed);
            }
            None => {
                failures += 1;
                shared.store(NOT_IDLE, Ordering::Relaxed);
                if failures >= 3 {
                    return;
                }
            }
        }
        // Sleep in short slices so a dropped watch ends promptly.
        let mut slept = Duration::ZERO;
        while slept < POLL && !stop.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(500));
            slept += Duration::from_millis(500);
        }
    }
}

/// Windows: the time since the last keyboard or mouse input
/// (`GetLastInputInfo`), the same "away from the desk" signal.
#[cfg(windows)]
fn query() -> Option<Option<Duration>> {
    use windows_sys::Win32::System::SystemInformation::GetTickCount;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
    let mut li = LASTINPUTINFO {
        cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
        dwTime: 0,
    };
    // SAFETY: `li` is a valid LASTINPUTINFO with its size set.
    if unsafe { GetLastInputInfo(&mut li) } == 0 {
        return None;
    }
    let idle_ms = unsafe { GetTickCount() }.wrapping_sub(li.dwTime) as u64;
    Some((idle_ms >= 60_000).then(|| Duration::from_millis(idle_ms)))
}

/// One read of this session's idle hint.  `Some(None)` = active,
/// `Some(Some(d))` = idle for `d`, `None` = logind unavailable.
#[cfg(target_os = "linux")]
fn query() -> Option<Option<Duration>> {
    let conn = zbus::blocking::Connection::system().ok()?;
    let pid = std::process::id();
    let reply = conn
        .call_method(
            Some("org.freedesktop.login1"),
            "/org/freedesktop/login1",
            Some("org.freedesktop.login1.Manager"),
            "GetSessionByPID",
            &(pid,),
        )
        .ok()?;
    let path: zbus::zvariant::OwnedObjectPath = reply.body().deserialize().ok()?;
    let get = |name: &str| -> Option<zbus::zvariant::OwnedValue> {
        conn.call_method(
            Some("org.freedesktop.login1"),
            path.as_str(),
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &("org.freedesktop.login1.Session", name),
        )
        .ok()?
        .body()
        .deserialize()
        .ok()
    };
    let hint: bool = get("IdleHint")?.try_into().ok()?;
    if !hint {
        return Some(None);
    }
    let since: u64 = get("IdleSinceHintMonotonic")
        .and_then(|v| v.try_into().ok())
        .unwrap_or(0);
    Some(idle_from_hint(true, since, monotonic_us()))
}

/// The monotonic clock in microseconds (the clock logind stamps idle with).
#[cfg(target_os = "linux")]
fn monotonic_us() -> u64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `ts` is a valid, writable timespec; CLOCK_MONOTONIC always exists.
    unsafe {
        libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts);
    }
    ts.tv_sec as u64 * 1_000_000 + ts.tv_nsec as u64 / 1000
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn off_or_active_is_normal_pace() {
        assert_eq!(quiet_factor(false, Some(Duration::from_secs(3600))), 1.0);
        assert_eq!(quiet_factor(true, None), 1.0);
    }

    #[test]
    fn nothing_happens_before_the_threshold() {
        assert_eq!(quiet_factor(true, Some(Duration::from_secs(0))), 1.0);
        assert_eq!(quiet_factor(true, Some(Duration::from_secs(299))), 1.0);
    }

    #[test]
    fn eases_linearly_down_then_holds_the_floor() {
        let at = |m: u64| quiet_factor(true, Some(Duration::from_secs(m * 60)));
        assert!((at(5) - 1.0).abs() < 1e-9, "starts at 1.0");
        // Halfway between 5 and 30 minutes (17.5 min) is halfway to the floor: 0.8x.
        assert!((quiet_factor(true, Some(Duration::from_secs(1050))) - 0.8).abs() < 1e-9);
        assert!((at(30) - MIN_CALM).abs() < 1e-9, "full calm at 30 min");
        assert!((at(600) - MIN_CALM).abs() < 1e-9, "never below the floor");
    }

    #[test]
    fn never_faster_and_monotonic() {
        let mut prev = 1.0;
        for s in (0..=3600).step_by(60) {
            let f = quiet_factor(true, Some(Duration::from_secs(s)));
            assert!(f <= 1.0 + 1e-12, "never faster than normal");
            assert!(f >= MIN_CALM - 1e-12, "never calmer than the floor");
            assert!(f <= prev + 1e-12, "the calm only deepens while away");
            prev = f;
        }
        assert!(MIN_CALM >= 0.5, "gentle: at least half speed");
    }

    #[test]
    fn hint_conversion() {
        assert_eq!(idle_from_hint(false, 100, 200), None);
        assert_eq!(
            idle_from_hint(true, 1_000_000, 61_000_000),
            Some(Duration::from_secs(60))
        );
        // No stamp, or a stamp in the future: just-idle, not a crash.
        assert_eq!(idle_from_hint(true, 0, 5), Some(Duration::ZERO));
        assert_eq!(idle_from_hint(true, 9, 5), Some(Duration::ZERO));
    }

    /// B62/D68: fewer people and cars while away; none of it when he is here.
    #[test]
    fn the_crowd_thins_while_away_and_is_whole_otherwise() {
        assert_eq!(crowd_factor(false, Some(Duration::from_secs(3600))), 1.0);
        assert_eq!(crowd_factor(true, None), 1.0);
        assert_eq!(crowd_factor(true, Some(Duration::from_secs(100))), 1.0);
        let full = crowd_factor(true, Some(Duration::from_secs(30 * 60)));
        assert!((full - MIN_CROWD).abs() < 1e-5, "{full}");
        let mut prev = 1.0;
        for s in (0..=3600).step_by(60) {
            let f = crowd_factor(true, Some(Duration::from_secs(s)));
            assert!(f <= prev + 1e-6 && f >= MIN_CROWD - 1e-6);
            prev = f;
        }
    }

    #[test]
    fn thinning_keeps_a_stable_share_of_the_list() {
        let base: Vec<u32> = (0..1000).collect();
        let mut half = base.clone();
        thin(&mut half, 0.5);
        assert!((480..=520).contains(&half.len()), "{}", half.len());
        // The same items stay when the same share is asked again (no reshuffle).
        let mut again = base.clone();
        thin(&mut again, 0.5);
        assert_eq!(half, again);
        // A thinner crowd is a subset of a fuller one.
        let mut fewer = base.clone();
        thin(&mut fewer, 0.375);
        assert!(fewer.iter().all(|x| half.contains(x)));
        let mut all = base.clone();
        thin(&mut all, 1.0);
        assert_eq!(all.len(), 1000);
    }
}
