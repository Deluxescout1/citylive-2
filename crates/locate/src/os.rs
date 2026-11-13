//! The OS location services. Linux: GeoClue2 over D-Bus. Windows:
//! `Windows.Devices.Geolocation`. Anywhere else: none (the IP lookup runs).

use crate::OsLocator;

/// The location service of this OS, if it has one.
pub fn system_locator() -> Option<Box<dyn OsLocator + Send>> {
    #[cfg(target_os = "linux")]
    {
        Some(Box::new(geoclue::GeoClue))
    }
    #[cfg(target_os = "windows")]
    {
        Some(Box::new(winloc::WindowsLocation))
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        None
    }
}

/// Run `f` on its own thread and give up after `timeout` (the D-Bus and WinRT
/// calls can hang; the caller must not).
#[allow(dead_code)]
fn with_timeout<T: Send + 'static>(
    timeout: std::time::Duration,
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(timeout).map_err(|_| "location service timed out".to_string())?
}

#[cfg(target_os = "linux")]
mod geoclue {
    use super::with_timeout;
    use crate::{OsFix, OsLocator};
    use std::time::{Duration, Instant};
    use zbus::blocking::{Connection, Proxy};
    use zbus::zvariant::OwnedObjectPath;

    pub struct GeoClue;

    const SERVICE: &str = "org.freedesktop.GeoClue2";
    /// Accuracy level CITY: a city is all a wallpaper sky needs.
    const ACCURACY_CITY: u32 = 4;

    fn fix(deadline: Instant) -> Result<OsFix, String> {
        fn e(what: &str, err: impl std::fmt::Display) -> String {
            format!("GeoClue {what}: {err}")
        }
        let conn = Connection::system().map_err(|x| e("no system bus", x))?;
        let manager = Proxy::new(
            &conn,
            SERVICE,
            "/org/freedesktop/GeoClue2/Manager",
            "org.freedesktop.GeoClue2.Manager",
        )
        .map_err(|x| e("manager", x))?;
        let client_path: OwnedObjectPath = manager.call("GetClient", &()).map_err(|x| e("GetClient", x))?;
        let client = Proxy::new(&conn, SERVICE, client_path, "org.freedesktop.GeoClue2.Client")
            .map_err(|x| e("client", x))?;
        client.set_property("DesktopId", "citylive-iso").map_err(|x| e("DesktopId", x))?;
        client
            .set_property("RequestedAccuracyLevel", ACCURACY_CITY)
            .map_err(|x| e("RequestedAccuracyLevel", x))?;
        // GeoClue answers "access denied" here when location services are off
        // or this app is not allowed; that is a normal failure, not a retry loop.
        client.call_method("Start", &()).map_err(|x| e("Start", x))?;
        let result = (|| loop {
            let loc: OwnedObjectPath = client.get_property("Location").map_err(|x| e("Location", x))?;
            if loc.as_str() != "/" {
                let lp = Proxy::new(&conn, SERVICE, loc, "org.freedesktop.GeoClue2.Location")
                    .map_err(|x| e("location", x))?;
                let lat: f64 = lp.get_property("Latitude").map_err(|x| e("Latitude", x))?;
                let lon: f64 = lp.get_property("Longitude").map_err(|x| e("Longitude", x))?;
                // "Description" is often a street or "Network" for a
                // city-level fix; use it only when it looks like a name.
                let desc: String = lp.get_property("Description").unwrap_or_default();
                let name = Some(desc).filter(|d| !d.is_empty() && d != "Network" && d.len() < 80);
                return Ok((lat, lon, name));
            }
            if Instant::now() >= deadline {
                return Err("GeoClue gave no location in time".to_string());
            }
            std::thread::sleep(Duration::from_millis(200));
        })();
        let _ = client.call_method("Stop", &());
        result
    }

    impl OsLocator for GeoClue {
        fn name(&self) -> &'static str {
            "GeoClue"
        }
        fn locate(&self, timeout: Duration) -> Result<OsFix, String> {
            let deadline = Instant::now() + timeout;
            with_timeout(timeout + Duration::from_secs(2), move || fix(deadline))
        }
    }
}

#[cfg(target_os = "windows")]
mod winloc {
    use super::with_timeout;
    use crate::{OsFix, OsLocator};
    use std::time::Duration;
    use windows::Devices::Geolocation::Geolocator;
    use windows::Foundation::TimeSpan;

    pub struct WindowsLocation;

    impl OsLocator for WindowsLocation {
        fn name(&self) -> &'static str {
            "Windows Location"
        }
        fn locate(&self, timeout: Duration) -> Result<OsFix, String> {
            with_timeout(timeout + Duration::from_secs(2), move || {
                let err = |what: &str, e: windows::core::Error| format!("Windows Location {what}: {e}");
                let geo = Geolocator::new().map_err(|e| err("create", e))?;
                // Ask for a position up to ten minutes old, waiting at most
                // `timeout`. Windows shows no prompt from here: when location
                // access is off for desktop apps this fails with "access
                // denied" and the IP lookup takes over.
                let max_age = TimeSpan { Duration: 10 * 60 * 10_000_000 };
                let wait = TimeSpan { Duration: timeout.as_millis() as i64 * 10_000 };
                let pos = geo
                    .GetGeopositionAsyncWithAgeAndTimeout(max_age, wait)
                    .map_err(|e| err("request", e))?
                    .get()
                    .map_err(|e| err("position", e))?;
                let p = pos
                    .Coordinate()
                    .and_then(|c| c.Point())
                    .and_then(|p| p.Position())
                    .map_err(|e| err("coordinate", e))?;
                Ok((p.Latitude, p.Longitude, None))
            })
        }
    }
}
