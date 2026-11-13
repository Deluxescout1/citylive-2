//! The monitors, and whether the wallpaper can currently be seen on them.

use std::ptr::null_mut;

use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM, RECT, TRUE};
use windows_sys::Win32::Graphics::Dwm::{
    DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS,
};
use windows_sys::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW,
};

const MONITORINFOF_PRIMARY: u32 = 1;
use windows_sys::Win32::System::Threading::GetCurrentProcessId;
use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows_sys::Win32::UI::Shell::SHQueryUserNotificationState;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetAncestor, GetForegroundWindow, GetLayeredWindowAttributes, GetWindow,
    GetWindowLongW, GetWindowThreadProcessId, IsIconic, IsWindowVisible, GA_ROOT, GWL_EXSTYLE,
    GW_OWNER, LWA_ALPHA, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT,
};

use super::util::{class_name, from_wide};

/// One physical monitor, in virtual-screen pixels (the process is per-monitor
/// DPI aware, so these are real pixels).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MonitorInfo {
    /// `DISPLAY1`, `DISPLAY2`... (the device name without the `\\.\` prefix).
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    /// The area not taken by the taskbar.
    pub work: (i32, i32, i32, i32),
    pub primary: bool,
    /// Effective DPI (96 = 100 %).
    pub dpi: u32,
}

unsafe extern "system" fn each_monitor(h: HMONITOR, _dc: HDC, _r: *mut RECT, lp: LPARAM) -> BOOL {
    // SAFETY: `lp` is the `&mut Vec<MonitorInfo>` passed by `monitors`.
    let list = unsafe { &mut *(lp as *mut Vec<MonitorInfo>) };
    let mut mi: MONITORINFOEXW = unsafe { std::mem::zeroed() };
    mi.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    // SAFETY: cbSize says which structure this is.
    if unsafe { GetMonitorInfoW(h, &mut mi as *mut MONITORINFOEXW as *mut MONITORINFO) } == 0 {
        return TRUE;
    }
    let r = mi.monitorInfo.rcMonitor;
    let wk = mi.monitorInfo.rcWork;
    let (mut dx, mut dy) = (96u32, 96u32);
    // SAFETY: out-parameters are locals.
    unsafe { GetDpiForMonitor(h, MDT_EFFECTIVE_DPI, &mut dx, &mut dy) };
    let raw = from_wide(&mi.szDevice);
    list.push(MonitorInfo {
        name: crate::wincore::head_name(&raw),
        x: r.left,
        y: r.top,
        w: r.right - r.left,
        h: r.bottom - r.top,
        work: (wk.left, wk.top, wk.right, wk.bottom),
        primary: mi.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
        dpi: dx,
    });
    TRUE
}

/// Every attached monitor, primary first, then left to right.
pub fn monitors() -> Vec<MonitorInfo> {
    let mut list: Vec<MonitorInfo> = Vec::new();
    // SAFETY: the callback only appends to `list`.
    unsafe {
        EnumDisplayMonitors(
            null_mut(),
            std::ptr::null(),
            Some(each_monitor),
            &mut list as *mut Vec<MonitorInfo> as LPARAM,
        );
    }
    list.sort_by_key(|m| (!m.primary, m.x, m.y));
    list
}

/// A window that counts as a real application window in front of the desktop.
#[derive(Clone)]
pub struct AppWindow {
    pub hwnd: HWND,
    pub class: String,
    pub pid: u32,
    pub exe: String,
    pub rect: RECT,
}

impl AppWindow {
    pub fn describe(&self) -> String {
        format!(
            "window {} class '{}' {}x{} at ({},{}) of {}",
            super::util::hex(self.hwnd),
            self.class,
            self.rect.right - self.rect.left,
            self.rect.bottom - self.rect.top,
            self.rect.left,
            self.rect.top,
            super::procs::describe_pid(self.pid)
        )
    }
}

/// Is `hwnd` a window a person can actually see and use in front of the
/// desktop? Rejects: invisible, minimised, cloaked (hidden UWP/virtual-desktop
/// windows), zero-alpha or click-through layered windows, tool/no-activate
/// windows, child windows, windows owned by Progman/WorkerW, the desktop and
/// shell windows, our own, and windows of other wallpaper programs.
pub fn real_app_window(hwnd: HWND) -> Option<AppWindow> {
    if hwnd.is_null() {
        return None;
    }
    // SAFETY: read-only window queries; out-parameters are locals.
    unsafe {
        if IsWindowVisible(hwnd) == 0 || IsIconic(hwnd) != 0 {
            return None;
        }
        if GetAncestor(hwnd, GA_ROOT) != hwnd {
            return None;
        }
        let ex = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
        if ex & (WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE) != 0 {
            return None;
        }
        if ex & WS_EX_LAYERED != 0 {
            let (mut key, mut alpha, mut flags) = (0u32, 255u8, 0u32);
            if GetLayeredWindowAttributes(hwnd, &mut key, &mut alpha, &mut flags) != 0
                && flags & LWA_ALPHA != 0
                && alpha < 16
            {
                return None;
            }
        }
        let mut cloaked = 0u32;
        DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED as u32, &mut cloaked as *mut u32 as *mut _, 4);
        if cloaked != 0 {
            return None;
        }
        let owner = GetWindow(hwnd, GW_OWNER);
        if !owner.is_null() {
            let oc = class_name(owner);
            if matches!(oc.as_str(), "Progman" | "WorkerW") {
                return None;
            }
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == GetCurrentProcessId() {
            return None;
        }
        let class = class_name(hwnd);
        if matches!(
            class.as_str(),
            "Progman"
                | "WorkerW"
                | "Shell_TrayWnd"
                | "Shell_SecondaryTrayWnd"
                | "SHELLDLL_DefView"
                | "Windows.UI.Core.CoreWindow"
                | "XamlExplorerHostIslandWindow"
        ) {
            return None;
        }
        if super::procs::is_other_wallpaper(pid) {
            return None;
        }
        let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        let hr = DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS as u32,
            &mut r as *mut RECT as *mut _,
            std::mem::size_of::<RECT>() as u32,
        );
        if hr != 0 {
            r = super::util::window_rect(hwnd)?;
        }
        let exe = super::procs::exe_path(pid)
            .map(|p| super::procs::file_name(&p))
            .unwrap_or_default();
        Some(AppWindow { hwnd, class, pid, exe, rect: r })
    }
}

struct Cover<'a> {
    mons: &'a [MonitorInfo],
    covered: Vec<Option<AppWindow>>,
}

unsafe extern "system" fn each_window(hwnd: HWND, lp: LPARAM) -> BOOL {
    // SAFETY: `lp` is the `&mut Cover` passed by `covered_monitors`.
    let c = unsafe { &mut *(lp as *mut Cover) };
    let cls = class_name(hwnd);
    // EnumWindows goes top to bottom: everything from the desktop layer down
    // is under the icons and cannot hide the wallpaper.
    if matches!(cls.as_str(), "Progman" | "WorkerW") {
        return 0;
    }
    let Some(w) = real_app_window(hwnd) else {
        return TRUE;
    };
    for (i, m) in c.mons.iter().enumerate() {
        if c.covered[i].is_none()
            && crate::wincore::covers_work_area((w.rect.left, w.rect.top, w.rect.right, w.rect.bottom), m.work)
        {
            c.covered[i] = Some(w.clone());
        }
    }
    TRUE
}

/// For each monitor: the real, visible application window covering its work
/// area (so the wallpaper cannot be seen there), if any.
pub fn covering_windows(mons: &[MonitorInfo]) -> Vec<Option<AppWindow>> {
    let mut c = Cover { mons, covered: vec![None; mons.len()] };
    // SAFETY: the callback only reads window properties.
    unsafe {
        EnumWindows(Some(each_window), &mut c as *mut Cover as LPARAM);
    }
    c.covered
}

pub fn covered_monitors(mons: &[MonitorInfo]) -> Vec<bool> {
    covering_windows(mons).iter().map(|c| c.is_some()).collect()
}

static LAST_NOTE: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

/// Log a decision only when it changes (this runs once a second).
fn note(msg: String) {
    if let Ok(mut l) = LAST_NOTE.lock() {
        if *l != msg {
            println!("win: {msg}");
            *l = msg;
        }
    }
}

/// Why the wallpaper cannot be seen right now, or `None` when it can.
///
/// The shell's notification state is the main signal, but "busy" and "full
/// screen" are only believed when the foreground window is a real application
/// window covering a monitor (another wallpaper program's invisible
/// full-monitor window makes the shell say "busy" with nothing in front).
/// Ordinary windows count only when the foreground window is a real one and
/// real windows cover every monitor. The decisive window is logged.
pub fn hidden_reason(mons: &[MonitorInfo], pause_when_covered: bool) -> Option<String> {
    let mut state = 0i32;
    // SAFETY: out-parameter is a local.
    let hr = unsafe { SHQueryUserNotificationState(&mut state) };
    // SAFETY: no arguments.
    let fg_hwnd = unsafe { GetForegroundWindow() };
    let fg = real_app_window(fg_hwnd);
    let fg_covers_monitor = fg.as_ref().is_some_and(|w| {
        mons.iter().any(|m| {
            crate::wincore::covers_work_area(
                (w.rect.left, w.rect.top, w.rect.right, w.rect.bottom),
                (m.x, m.y, m.x + m.w, m.y + m.h),
            )
        })
    });
    if hr == 0 {
        if let Some(r) = crate::wincore::notification_reason(state, fg_covers_monitor) {
            let by = fg.as_ref().map(|w| w.describe()).unwrap_or_else(|| "no window".into());
            note(format!("shell state {state}: {r}; foreground {by}"));
            return Some(format!("{r} ({by})"));
        }
        if matches!(state, 2 | 3) {
            let fgd = if fg_hwnd.is_null() {
                "none".to_string()
            } else {
                let mut pid = 0u32;
                // SAFETY: out-parameter is a local.
                unsafe { GetWindowThreadProcessId(fg_hwnd, &mut pid) };
                format!(
                    "{} class '{}' of {} (not a visible application window, or not covering a monitor)",
                    super::util::hex(fg_hwnd),
                    class_name(fg_hwnd),
                    super::procs::describe_pid(pid)
                )
            };
            note(format!("shell state {state} (busy) ignored: foreground is {fgd}"));
        } else {
            note(format!("shell state {state}: nothing hides the wallpaper"));
        }
    }
    if pause_when_covered && fg.is_some() && !mons.is_empty() {
        let cov = covering_windows(mons);
        if cov.iter().all(|c| c.is_some()) {
            let who: Vec<String> = mons
                .iter()
                .zip(&cov)
                .filter_map(|(m, c)| c.as_ref().map(|w| format!("{}: {}", m.name, w.describe())))
                .collect();
            let msg = format!("windows cover every monitor ({})", who.join("; "));
            note(msg.clone());
            return Some(msg);
        }
    }
    None
}
