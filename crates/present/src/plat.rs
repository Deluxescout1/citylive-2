//! The few things that differ by operating system and have nothing to do with
//! drawing: where files live, the local clock, opening a folder, and the
//! signal / console-close handler. The Linux answers are exactly what the code
//! returned before the Windows port (B95); the Windows ones follow the
//! conventions in `DECISIONS.md` D92: settings in `%APPDATA%\CityLive`, the
//! saves, cache and log in `%LOCALAPPDATA%\CityLive`.

use std::path::{Path, PathBuf};

/// Local civil time plus the zone's offset from UTC.
#[derive(Clone, Copy, Debug)]
pub struct LocalTime {
    pub year: i32,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    /// Offset from UTC now, in milliseconds (DST included).
    pub utc_offset_ms: i64,
}

#[cfg(target_os = "linux")]
pub fn local_time() -> LocalTime {
    // SAFETY: plain libc time calls on a zeroed `tm` we own.
    let t: libc::time_t = unsafe { libc::time(std::ptr::null_mut()) };
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe {
        libc::localtime_r(&t, &mut tm);
    }
    LocalTime {
        year: tm.tm_year + 1900,
        month: (tm.tm_mon + 1) as u8,
        day: tm.tm_mday as u8,
        hour: tm.tm_hour as u8,
        utc_offset_ms: tm.tm_gmtoff as i64 * 1000,
    }
}

#[cfg(windows)]
pub fn local_time() -> LocalTime {
    use windows_sys::Win32::System::SystemInformation::GetLocalTime;
    use windows_sys::Win32::System::Time::GetTimeZoneInformation;
    const TIME_ZONE_ID_DAYLIGHT: u32 = 2;
    // SAFETY: both calls only fill structures we own.
    unsafe {
        let mut st = std::mem::zeroed();
        GetLocalTime(&mut st);
        let mut tz = std::mem::zeroed();
        let id = GetTimeZoneInformation(&mut tz);
        // Bias is minutes to ADD to local time to get UTC (so west of UTC is
        // positive); the daylight bias applies while DST is in force.
        let mut bias = tz.Bias;
        if id == TIME_ZONE_ID_DAYLIGHT {
            bias += tz.DaylightBias;
        } else {
            bias += tz.StandardBias;
        }
        LocalTime {
            year: st.wYear as i32,
            month: st.wMonth as u8,
            day: st.wDay as u8,
            hour: st.wHour as u8,
            utc_offset_ms: -(bias as i64) * 60_000,
        }
    }
}

/// `%NAME%` as a non-empty path.
#[cfg(windows)]
fn env_dir(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// The settings folder: `%APPDATA%\CityLive`.
#[cfg(windows)]
pub fn config_dir() -> PathBuf {
    env_dir("APPDATA")
        .or_else(|| env_dir("USERPROFILE").map(|p| p.join("AppData").join("Roaming")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("CityLive")
}

/// The per-machine data folder: `%LOCALAPPDATA%\CityLive` (saves, cache, log).
#[cfg(windows)]
pub fn data_dir() -> PathBuf {
    env_dir("LOCALAPPDATA")
        .or_else(|| env_dir("USERPROFILE").map(|p| p.join("AppData").join("Local")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("CityLive")
}

/// The log file (Windows): `%LOCALAPPDATA%\CityLive\citylive.log`.
#[cfg(windows)]
pub fn log_path() -> PathBuf {
    data_dir().join("citylive.log")
}

/// The cache folder (weather, feeds, location).
#[cfg(target_os = "linux")]
pub fn cache_dir() -> PathBuf {
    PathBuf::from(
        std::env::var("XDG_CACHE_HOME")
            .map(|d| format!("{d}/citylive-iso"))
            .unwrap_or_else(|_| "/tmp/citylive-iso".to_string()),
    )
}

#[cfg(windows)]
pub fn cache_dir() -> PathBuf {
    data_dir().join("cache")
}

/// Open a file or folder with the desktop's default handler.
#[cfg(target_os = "linux")]
pub fn open_path(path: &Path) {
    if let Err(e) = std::process::Command::new("xdg-open").arg(path).spawn() {
        eprintln!("xdg-open {}: {e}", path.display());
    }
}

#[cfg(windows)]
pub fn open_path(path: &Path) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    if let Err(e) = std::process::Command::new("explorer.exe")
        .arg(path)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
    {
        eprintln!("explorer {}: {e}", path.display());
    }
}

/// Ctrl-C, console close, logoff and shutdown all end the run cleanly (the
/// state is saved on the way out).
#[cfg(windows)]
pub fn setup_signal_handlers() {
    use std::sync::atomic::Ordering;
    use windows_sys::Win32::System::Console::SetConsoleCtrlHandler;
    unsafe extern "system" fn handler(_ctrl: u32) -> i32 {
        crate::TERMINATING.store(true, Ordering::Relaxed);
        1
    }
    // SAFETY: registering a handler that only stores an atomic.
    unsafe {
        SetConsoleCtrlHandler(Some(handler), 1);
    }
}
