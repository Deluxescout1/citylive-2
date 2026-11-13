//! The pure decisions of the Windows backend, kept free of Win32 so they are
//! tested on any machine (the Windows code itself can only be exercised on
//! Windows: see `win::selftest`).

/// A rectangle as (left, top, right, bottom), Win32 style.
pub type Edges = (i32, i32, i32, i32);

/// Does a window covering `win` hide (at least 98 %) of the monitor's work
/// area `work`? A maximised window covers it exactly; the slack forgives a
/// rounding pixel at a fractional DPI scale.
pub fn covers_work_area(win: Edges, work: Edges) -> bool {
    let area = ((work.2 - work.0).max(0) as i64) * ((work.3 - work.1).max(0) as i64);
    if area == 0 {
        return false;
    }
    let iw = (win.2.min(work.2) - win.0.max(work.0)).max(0) as i64;
    let ih = (win.3.min(work.3) - win.1.max(work.1)).max(0) as i64;
    iw * ih * 100 >= area * 98
}

/// The device name Windows gives a monitor (`\\.\DISPLAY1`) as the head name
/// the config's `include_outputs` / `ignore_outputs` use (`DISPLAY1`).
pub fn head_name(device: &str) -> String {
    device.trim_start_matches("\\\\.\\").to_string()
}

/// Another program that draws a wallpaper (or acts as a desktop layer).
/// `OldCityLive` is the previous Electron "CityLive" app (a `Programs\CityLive` install).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OtherApp {
    OldCityLive,
    WallpaperEngine,
    Lively,
    Other,
}

/// Recognise such a program from its executable name and full path.
pub fn other_wallpaper_app(exe: &str, path: &str) -> Option<(OtherApp, &'static str)> {
    let exe = exe.to_ascii_lowercase();
    let path = path.to_ascii_lowercase().replace('/', "\\");
    if exe == "citylive.exe" && path.contains("\\programs\\citylive\\") {
        return Some((OtherApp::OldCityLive, "the old CityLive (Electron)"));
    }
    match exe.as_str() {
        "wallpaper32.exe" | "wallpaper64.exe" | "wallpaperservice32.exe" => {
            Some((OtherApp::WallpaperEngine, "Wallpaper Engine"))
        }
        "lively.exe" | "livelywpf.exe" | "lively.player.webview2.exe" | "lively.player.vlc.exe" => {
            Some((OtherApp::Lively, "Lively Wallpaper"))
        }
        "weebp.exe" | "rainmeter.exe" => Some((OtherApp::Other, "another desktop app")),
        _ => None,
    }
}

/// What the shell's notification state (`QUERY_USER_NOTIFICATION_STATE`) means
/// for the wallpaper. Another wallpaper program, or any full-monitor window
/// parented into the desktop, can make the shell report "busy" (2) or "full
/// screen" (3) with nothing in front at all, so those two are only believed
/// when the foreground window is a real application window covering a monitor
/// (`fg_covers`). Locked / screen saver (1) and presentation mode (4) are
/// believed as they are.
pub fn notification_reason(state: i32, fg_covers: bool) -> Option<&'static str> {
    match state {
        1 => Some("session locked or screen saver"),
        2 if fg_covers => Some("a full-screen application is in front"),
        3 if fg_covers => Some("a full-screen game is in front"),
        4 => Some("presentation mode"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORK: Edges = (0, 0, 2560, 1400);

    #[test]
    fn a_maximised_window_covers_the_work_area() {
        assert!(covers_work_area((0, 0, 2560, 1400), WORK));
        assert!(covers_work_area((-8, -8, 2568, 1408), WORK));
        // A rounding pixel at 125 % scale.
        assert!(covers_work_area((0, 0, 2559, 1400), WORK));
    }

    #[test]
    fn a_floating_window_does_not() {
        assert!(!covers_work_area((200, 100, 2000, 1200), WORK));
        assert!(!covers_work_area((0, 0, 1280, 1400), WORK));
        // On the neighbouring monitor only.
        assert!(!covers_work_area((2560, 0, 5120, 1400), WORK));
    }

    #[test]
    fn an_empty_work_area_is_never_covered() {
        assert!(!covers_work_area((0, 0, 100, 100), (0, 0, 0, 0)));
    }

    #[test]
    fn head_names_drop_the_device_prefix() {
        assert_eq!(head_name("\\\\.\\DISPLAY2"), "DISPLAY2");
        assert_eq!(head_name("DISPLAY1"), "DISPLAY1");
    }

    #[test]
    fn only_real_foreground_windows_make_busy_count() {
        for st in [2, 3] {
            assert!(notification_reason(st, true).is_some());
            assert!(notification_reason(st, false).is_none(), "{st}");
        }
        assert!(notification_reason(1, false).is_some());
        assert!(notification_reason(4, false).is_some());
        for st in [0, 5, 6, 7] {
            assert!(notification_reason(st, true).is_none(), "{st}");
        }
    }

    #[test]
    fn other_wallpaper_programs_are_recognised() {
        let old = other_wallpaper_app("CityLive.exe", "C:/Users/N/AppData/Local/Programs/CityLive/CityLive.exe");
        assert_eq!(old.map(|o| o.0), Some(OtherApp::OldCityLive));
        // Citylive 2 itself (a different folder) is not the old one.
        assert!(other_wallpaper_app("citylive.exe", "c:\\users\\n\\appdata\\local\\citylive 2\\citylive.exe").is_none());
        assert_eq!(other_wallpaper_app("wallpaper64.exe", "").map(|o| o.0), Some(OtherApp::WallpaperEngine));
        assert_eq!(other_wallpaper_app("Lively.exe", "").map(|o| o.0), Some(OtherApp::Lively));
        assert!(other_wallpaper_app("notepad.exe", "").is_none());
    }
}
