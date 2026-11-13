//! B47 — the living sky in the wallpaper's own loop: the three feeds, their
//! gates, and the per-frame pass that draws them.
//!
//! The data layer is `world`'s (B44): the Kp index, the ISS's sub-satellite
//! point and the aircraft around the city. The art is `sky::live`'s. This module
//! is only the plumbing: fetch on the feed's own bucket (never on the render
//! thread), turn the sky's state into the gates, and hand the drawing pass the
//! scene and the composite.
//!
//! Cost discipline (D71): a fetch is a thread on a bucket; a frame is the
//! rectangles the layers swept, restored from the scene before they draw again.
//! With nothing up, [`LiveSky::live`] stays false and only the fetch buckets run.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use render::{Framebuffer, Rect};
use sky::live::{self, AuroraInk, AuroraSky, Band, Track};
use world::aurora::Aurora;
use world::iss::Iss;

/// The state of the three layers: what the feeds last said, when they were last
/// asked, and the scratch the drawing pass reuses (no allocation on the hot
/// path).
pub struct LiveSky {
    /// The current Kp reading.
    pub aurora: Aurora,
    aurora_rx: Arc<Mutex<Option<Aurora>>>,
    aurora_at: Instant,
    /// The last two ISS readings: the pair dead-reckons a smooth trail between
    /// the feed's 25-second fixes.
    pub iss_prev: Option<Iss>,
    pub iss_now: Option<Iss>,
    iss_rx: Arc<Mutex<Option<Option<Iss>>>>,
    iss_at: Instant,
    /// The aircraft around the city, and the fetch bucket.
    flights_rx: Arc<Mutex<Option<Vec<world::flights::Flight>>>>,
    flights_fetch_at: Instant,
    /// One track per aircraft, dead-reckoned between fixes.
    pub tracks: Vec<Track>,
    /// The rectangles the layers painted last frame, restored before the next.
    pub painted: Vec<Rect>,
    /// Scratch: the fake schedule and its poses.
    pub fakes: Vec<live::FakeFlight>,
    /// Whether anything was drawn last frame (the loop's wake-up gate).
    pub live: bool,
    /// What the aurora's ink was when the sky was last told about it, so the
    /// sky only rebuilds when it really changed.
    last_ink: Option<AuroraInk>,
}

impl Default for LiveSky {
    fn default() -> Self {
        Self {
            aurora: Aurora::default(),
            aurora_rx: Arc::new(Mutex::new(None)),
            aurora_at: Instant::now(),
            iss_prev: None,
            iss_now: None,
            iss_rx: Arc::new(Mutex::new(None)),
            iss_at: Instant::now(),
            flights_rx: Arc::new(Mutex::new(None)),
            // Ask straight away: the first fix of the day is what the sky needs
            // to be honest.
            flights_fetch_at: Instant::now() - Duration::from_secs(60),
            tracks: Vec::new(),
            painted: Vec::new(),
            fakes: Vec::new(),
            live: false,
            last_ink: None,
        }
    }
}

/// The switches, from the settings file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Gates {
    pub aurora: bool,
    pub iss: bool,
    pub flights: bool,
}

impl Gates {
    pub fn none(&self) -> bool {
        !self.aurora && !self.iss && !self.flights
    }
}

/// Everything the drawing pass needs to know about the place and the moment.
#[derive(Clone, Copy, Debug)]
pub struct SkyView {
    pub band: Band,
    /// The sim/sky clock: the ISS's geometry and the fake schedule follow it, so
    /// a pinned `--time` render is the sky of that hour.
    pub sky_ms: i64,
    /// The animation clock (wall time): everything that moves follows this, so a
    /// pinned clock still gets live motion (D71, as the meteors do).
    pub wall_ms: i64,
    pub lat: f64,
    pub lon: f64,
    /// The aurora's inputs: Kp, the level at this latitude, the sun, cloud, Moon.
    pub aurora: AuroraSky,
    /// The air temperature at the ground (the contrail's lapse starts here).
    pub ground_c: f32,
    /// Does the city have an airfield? Then its own low traffic is in the sky.
    pub has_airfield: bool,
    /// Is it dark enough for nav lights and strobes?
    pub night: bool,
    pub gates: Gates,
}

fn cache(name: &str) -> std::path::PathBuf {
    crate::plat::cache_dir().join(name)
}

impl LiveSky {
    /// The aurora's ink for this sky, if anything is up.
    pub fn ink(&self, sky: &AuroraSky) -> Option<AuroraInk> {
        AuroraInk::of(sky)
    }

    /// Did the aurora's ink move enough to raise the scene's sky again?
    pub fn ink_changed(&mut self, ink: Option<AuroraInk>) -> bool {
        let changed = match (self.last_ink, ink) {
            (None, None) => false,
            (Some(a), Some(b)) => {
                (a.strength - b.strength).abs() > 0.06 || (a.red - b.red).abs() > 0.15
            }
            _ => true,
        };
        if changed {
            self.last_ink = ink;
        }
        changed
    }

    /// Ask the feeds whose bucket has turned, and take any answers waiting.
    /// The weather's discipline: fetch on a thread, land in a slot, never block
    /// the loop.
    #[allow(unused_variables)]
    pub fn poll(&mut self, gates: Gates, lat: f64, lon: f64, now: Instant) {
        #[cfg(feature = "net")]
        {
            if gates.aurora && now >= self.aurora_at {
                self.aurora_at =
                    now + Duration::from_millis(world::feeds::AURORA_REFRESH_MS as u64);
                let slot = self.aurora_rx.clone();
                std::thread::spawn(move || {
                    let client = world::UreqClient::new();
                    let a = world::aurora::fetch(&client, &cache("noaa-kp.json"));
                    if let Ok(mut g) = slot.lock() {
                        *g = Some(a);
                    }
                });
            }
            if gates.iss && now >= self.iss_at {
                self.iss_at = now + Duration::from_millis(world::feeds::ISS_REFRESH_MS as u64);
                let slot = self.iss_rx.clone();
                std::thread::spawn(move || {
                    let client = world::UreqClient::new();
                    let now_ms = super::wall_ms_now();
                    let fresh = world::iss::fetch(&client, now_ms, &cache("iss-now.json"));
                    if let Ok(mut g) = slot.lock() {
                        *g = Some(fresh);
                    }
                });
            }
            if gates.flights && now >= self.flights_fetch_at {
                self.flights_fetch_at =
                    now + Duration::from_millis(world::feeds::FLIGHTS_REFRESH_MS as u64);
                let slot = self.flights_rx.clone();
                std::thread::spawn(move || {
                    let client = world::UreqClient::new();
                    let list = world::flights::fetch(
                        &client,
                        lat,
                        lon,
                        world::flights::DEFAULT_RADIUS_NM,
                        &cache("airplanes-live.json"),
                    );
                    if let Ok(mut g) = slot.lock() {
                        *g = Some(list);
                    }
                });
            }
        }
        if let Ok(mut g) = self.aurora_rx.lock() {
            if let Some(a) = g.take() {
                self.aurora = a;
            }
        }
        if let Ok(mut g) = self.iss_rx.lock() {
            if let Some(i) = g.take() {
                if i.is_some() {
                    self.iss_prev = self.iss_now;
                    self.iss_now = i;
                }
            }
        }
        // Take the list out of the slot first, so the lock is not held while
        // the tracks are updated.
        let landed = self
            .flights_rx
            .lock()
            .ok()
            .and_then(|mut g| g.take());
        if let Some(list) = landed {
            self.ingest(&list);
        }
    }

    /// Turn one feed snapshot into tracks: a fix for an aircraft already
    /// tracked moves it, a new aircraft gets a track, and anything the feed
    /// stops mentioning is left to run out on its own clock.
    pub fn ingest(&mut self, flights: &[world::flights::Flight]) {
        let now_ms = super::wall_ms_now();
        for f in flights {
            let key = if f.hex.is_empty() {
                f.callsign.clone()
            } else {
                f.hex.clone()
            };
            match self.tracks.iter_mut().find(|t| t.key() == key) {
                Some(t) => t.advance(f, now_ms),
                None => self.tracks.push(Track::from_flight(&key, f, now_ms)),
            }
        }
        // Forget the ones that ran out long ago.
        self.tracks.retain(|t| !t.is_stale(now_ms));
        if self.tracks.len() > 32 {
            let extra = self.tracks.len() - 32;
            self.tracks.drain(..extra);
        }
    }

    /// Where the ISS is now, dead-reckoned between the last two fixes.
    pub fn iss_at(&self, now_ms: i64) -> Option<Iss> {
        match (self.iss_prev, self.iss_now) {
            (Some(p), Some(c)) => Some(Iss::dead_reckon(&p, &c, now_ms)),
            (None, Some(c)) => Some(c),
            _ => None,
        }
    }

    /// Draw the three layers into the composite and report what was swept.
    ///
    /// `scene` is only read to put the previous frame's pixels back, and for the
    /// sky layer the bare-sky test needs. Nothing is drawn when the switches are
    /// off or nothing is up.
    pub fn draw(
        &mut self,
        scene: &render::Scene,
        fb: &mut Framebuffer,
        v: &SkyView,
    ) -> Vec<Rect> {
        let mut damage = Vec::new();
        // The previous frame's pixels, back from the scene — even when every
        // switch has just been turned off, because that is what erases them.
        live::restore(scene, fb, &mut self.painted, &mut damage);
        if !v.band.usable() || v.gates.none() {
            self.live = false;
            return damage;
        }

        let mut painted = false;
        let sky_fb = scene.sky();
        // Order matters for the occlusion test, not for the eye. The small,
        // bright things go down first, against the *bare* sky; the aurora goes
        // last, and its own test then skips their pixels — so an aircraft or the
        // station is never hidden behind a curtain of light that is actually
        // 400 km below it.
        if v.gates.flights {
            let mut n = 0;
            for t in &self.tracks {
                if let Some(p) = t.pose(v.band, v.wall_ms, v.ground_c, v.has_airfield) {
                    if let Some(r) = live::draw_plane(fb, sky_fb, v.band, &p, v.wall_ms, v.night) {
                        self.painted.push(r);
                        damage.push(r);
                        painted = true;
                    }
                    n += 1;
                    if n >= 8 {
                        break;
                    }
                }
            }
            // Nothing from the feed (or it is offline): the schedule stands in.
            if n == 0 {
                let hour = sky::local_hour(v.sky_ms, v.lon);
                live::fake_flights(hour, &mut self.fakes);
                for i in 0..self.fakes.len() {
                    if let Some(p) = live::fake_pose(&self.fakes[i], v.band, v.ground_c) {
                        if let Some(r) =
                            live::draw_plane(fb, sky_fb, v.band, &p, v.wall_ms, v.night)
                        {
                            self.painted.push(r);
                            damage.push(r);
                            painted = true;
                        }
                    }
                }
            }
        }
        if v.gates.iss {
            if let Some(iss) = self.iss_at(v.wall_ms) {
                let look = iss.look(v.lat, v.lon);
                let sun_sub = live::sun_alt_at(v.sky_ms, iss.lat, iss.lon);
                let a = live::iss_alpha(look.alt, v.aurora.sun_alt, sun_sub);
                if a > 0.02 && look.alt > 5.0 {
                    let x = v.band.x(look.az);
                    let y = v.band.y(look.alt);
                    if let Some(r) = live::draw_iss(fb, sky_fb, v.band, x, y, a) {
                        self.painted.push(r);
                        damage.push(r);
                        painted = true;
                    }
                }
            }
        }
        if v.gates.aurora {
            if let Some(ink) = self.ink(&v.aurora) {
                if let Some(r) = live::draw_aurora(fb, sky_fb, v.band, v.wall_ms, &ink) {
                    self.painted.push(r);
                    damage.push(r);
                    painted = true;
                }
            }
        }
        self.live = painted;
        damage
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// B93: landing an ISS fix must not reset the fetch bucket. It used to set
    /// the next fetch to "now", so every answer started the next request at
    /// once and the feed was asked back to back instead of every 25 s.
    #[test]
    fn landing_an_iss_fix_keeps_the_fetch_bucket() {
        let mut ls = LiveSky::default();
        let now = Instant::now();
        let due = now + Duration::from_secs(25);
        ls.iss_at = due;
        let fix = Iss { lat: 10.0, lon: 20.0, ts: 1_000 };
        *ls.iss_rx.lock().unwrap() = Some(Some(fix));
        let gates = Gates { aurora: false, iss: false, flights: false };
        ls.poll(gates, 0.0, 0.0, now);
        assert!(ls.iss_now.is_some(), "the fix should have landed");
        assert_eq!(ls.iss_at, due, "landing a fix must not make the next fetch due");
    }
}
