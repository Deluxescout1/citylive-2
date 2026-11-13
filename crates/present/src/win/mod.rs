//! The Windows desktop backend (B95): the city as the real desktop wallpaper
//! behind the icons, on every monitor.
//!
//! * [`desktop`]: finds the window behind the icons (the WorkerW technique, for
//!   both the legacy layout and the Windows 11 24H2 one).
//! * [`surface`]: one child window and DIB canvas per monitor, damage-rectangle
//!   blits with GDI.
//! * [`monitors`]: the monitor list, and "is the wallpaper visible right now"
//!   (full-screen apps, lock screen, windows covering everything).
//! * [`host`]: the run loop's platform half: the helper window, the tray icon,
//!   monitor hot-plug, Explorer restarts, sleep and resume.
//! * [`log`]: the log file in `%LOCALAPPDATA%\CityLive`, since the programs have
//!   no console.
//! * [`autostart`]: start with Windows (the per-user Run key).
//! * [`selftest`]: `--selftest` and `--diag`, so a problem can be diagnosed from
//!   text.

pub mod autostart;
pub mod desktop;
pub mod host;
pub mod log;
pub mod monitors;
pub mod procs;
pub mod selftest;
pub mod surface;
pub mod util;

pub use host::{run, set_options, Options};
pub use surface::Surface;
