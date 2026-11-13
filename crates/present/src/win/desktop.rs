//! Finding the window the wallpaper goes into (the WorkerW technique).
//!
//! Explorer owns a `Progman` window. Sending it the undocumented message
//! `0x052C` makes it split the desktop into layers: the icons (the
//! `SHELLDLL_DefView` window) and, behind them, a `WorkerW` window where a
//! wallpaper can live. How the windows are arranged depends on the Windows
//! build:
//!
//! * **Legacy** (Windows 8 to 11 23H2): after the message there is a top-level
//!   `WorkerW` that holds `SHELLDLL_DefView`, and a second, empty top-level
//!   `WorkerW` behind it. The wallpaper window becomes a child of the second one.
//! * **Progman** (Windows 11 24H2 and later): `SHELLDLL_DefView` and the
//!   `WorkerW` are both children of `Progman`. The wallpaper window becomes a
//!   child of `Progman`, placed just below `SHELLDLL_DefView`.
//!
//! Nothing here is assumed: [`probe`] describes what is actually there, and the
//! caller can force a mode with `--attach` / `CITYLIVE_ATTACH` while diagnosing.

use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, FindWindowExW, FindWindowW, GetWindow, GetWindowThreadProcessId, IsWindowVisible,
    SendMessageTimeoutW, GetWindowLongW, GWL_EXSTYLE, GWL_STYLE, GW_CHILD, GW_HWNDNEXT, SMTO_NORMAL,
};

use super::util::{class_name, fmt_rect, hex, wide, window_rect};

/// Which arrangement was found (or forced).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Child of the empty top-level WorkerW behind the icons.
    Legacy,
    /// Child of Progman, below SHELLDLL_DefView.
    Progman,
    /// Child of the WorkerW that lives inside Progman (24H2 variant).
    ProgmanWorkerW,
    /// Nothing usable: a top-level window at the bottom of the z-order.
    TopLevel,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Legacy => "legacy (WorkerW)",
            Kind::Progman => "progman (24H2)",
            Kind::ProgmanWorkerW => "progman-workerw (24H2, inside the WorkerW)",
            Kind::TopLevel => "top-level fallback",
        }
    }
}

/// What the wallpaper window should be parented to.
#[derive(Clone, Copy, Debug)]
pub struct Target {
    pub kind: Kind,
    /// The parent window (null for `TopLevel`).
    pub parent: HWND,
    /// The window to sit just below (`SHELLDLL_DefView` for `Progman`).
    pub below: HWND,
}

/// The user's override: `auto` (default), `legacy`, `progman`,
/// `progman-workerw` or `toplevel`. Automatic: legacy if the top-level WorkerW
/// exists, else the WorkerW inside Progman, else Progman itself.
pub fn forced_mode(arg: Option<&str>) -> Option<Kind> {
    let v = arg
        .map(|s| s.to_string())
        .or_else(|| std::env::var("CITYLIVE_ATTACH").ok())?;
    match v.to_lowercase().as_str() {
        "legacy" | "workerw" => Some(Kind::Legacy),
        "progman" | "24h2" => Some(Kind::Progman),
        "progman-workerw" | "pw" => Some(Kind::ProgmanWorkerW),
        "toplevel" | "top-level" | "bottom" => Some(Kind::TopLevel),
        _ => None,
    }
}

fn find(class: &str, parent: HWND, after: HWND) -> HWND {
    let c = wide(class);
    // SAFETY: NUL-terminated class name; null handles mean "any".
    unsafe { FindWindowExW(parent, after, c.as_ptr(), null()) }
}

pub fn progman() -> HWND {
    let c = wide("Progman");
    // SAFETY: NUL-terminated class name.
    unsafe { FindWindowW(c.as_ptr(), null()) }
}

/// Ask Progman to create the WorkerW layer. Harmless when it already exists.
pub fn spawn_workerw(progman: HWND) {
    let mut result = 0usize;
    // SAFETY: a documented-by-practice message with a timeout, so a hung
    // Explorer cannot hang us.
    unsafe {
        SendMessageTimeoutW(progman, 0x052C, 0, 0, SMTO_NORMAL, 1000, &mut result);
        SendMessageTimeoutW(progman, 0x052C, 0xD, 0, SMTO_NORMAL, 1000, &mut result);
        SendMessageTimeoutW(progman, 0x052C, 0xD, 1, SMTO_NORMAL, 1000, &mut result);
    }
}

struct Search {
    /// The top-level window that holds SHELLDLL_DefView (a WorkerW, or Progman).
    defview_host: HWND,
}

unsafe extern "system" fn each_top(hwnd: HWND, lp: LPARAM) -> BOOL {
    // SAFETY: `lp` is the `&mut Search` the caller passed.
    let s = unsafe { &mut *(lp as *mut Search) };
    if !find("SHELLDLL_DefView", hwnd, null_mut()).is_null() {
        s.defview_host = hwnd;
        return 0;
    }
    1
}

/// Work out where the wallpaper goes. `send` sends the 0x052C message first.
pub fn find_target(force: Option<Kind>, send: bool) -> Target {
    let progman = progman();
    if progman.is_null() {
        return Target {
            kind: Kind::TopLevel,
            parent: null_mut(),
            below: null_mut(),
        };
    }
    if send {
        spawn_workerw(progman);
    }
    let mut s = Search {
        defview_host: null_mut(),
    };
    // SAFETY: the callback only reads window classes.
    unsafe {
        EnumWindows(Some(each_top), &mut s as *mut Search as LPARAM);
    }
    // Legacy: the icons live in a top-level WorkerW and the target is the next
    // top-level WorkerW after it.
    let legacy_target = if !s.defview_host.is_null() && s.defview_host != progman {
        find("WorkerW", null_mut(), s.defview_host)
    } else {
        null_mut()
    };
    // 24H2: SHELLDLL_DefView is a child of Progman.
    let defview_in_progman = find("SHELLDLL_DefView", progman, null_mut());

    let legacy = (!legacy_target.is_null()).then_some(Target {
        kind: Kind::Legacy,
        parent: legacy_target,
        below: null_mut(),
    });
    let prog = Target {
        kind: Kind::Progman,
        parent: progman,
        below: defview_in_progman,
    };
    let inner_workerw = find("WorkerW", progman, null_mut());
    let prog_w = Target {
        kind: Kind::ProgmanWorkerW,
        parent: if inner_workerw.is_null() { progman } else { inner_workerw },
        below: null_mut(),
    };
    match force {
        Some(Kind::Legacy) => legacy.unwrap_or(prog),
        Some(Kind::Progman) => prog,
        Some(Kind::ProgmanWorkerW) => prog_w,
        Some(Kind::TopLevel) => Target {
            kind: Kind::TopLevel,
            parent: null_mut(),
            below: null_mut(),
        },
        // 24H2: the WorkerW inside Progman is where a child shows (verified on
        // build 26200); the layered child under DefView does not.
        None => match legacy {
            Some(t) => t,
            None if !inner_workerw.is_null() => prog_w,
            None if !defview_in_progman.is_null() => prog,
            None => Target {
                kind: Kind::TopLevel,
                parent: null_mut(),
                below: null_mut(),
            },
        },
    }
}

fn describe(h: HWND, depth: usize, out: &mut Vec<String>) {
    let pad = "  ".repeat(depth);
    let vis = unsafe { IsWindowVisible(h) } != 0;
    let r = window_rect(h).map(|r| fmt_rect(&r)).unwrap_or_default();
    // SAFETY: plain style queries.
    let (st, ex) = unsafe {
        (
            GetWindowLongW(h, GWL_STYLE) as u32,
            GetWindowLongW(h, GWL_EXSTYLE) as u32,
        )
    };
    out.push(format!(
        "{pad}{} class={} {} {} style=0x{st:08X} exstyle=0x{ex:08X}{}",
        hex(h),
        class_name(h),
        if vis { "visible" } else { "hidden" },
        r,
        if ex & 0x0008_0000 != 0 { " LAYERED" } else { "" }
    ));
}

/// A text description of the desktop's window tree, for the log and the
/// self-test: Progman and its children, and every top-level window that holds
/// SHELLDLL_DefView or is a WorkerW.
pub fn probe() -> Vec<String> {
    let mut out = Vec::new();
    let progman = progman();
    if progman.is_null() {
        out.push("Progman: NOT FOUND (is Explorer running?)".to_string());
        return out;
    }
    out.push("Progman:".to_string());
    describe(progman, 1, &mut out);
    // SAFETY: walking child windows with GetWindow.
    unsafe {
        let mut c = GetWindow(progman, GW_CHILD);
        while !c.is_null() {
            describe(c, 2, &mut out);
            let mut g = GetWindow(c, GW_CHILD);
            while !g.is_null() {
                describe(g, 3, &mut out);
                g = GetWindow(g, GW_HWNDNEXT);
            }
            c = GetWindow(c, GW_HWNDNEXT);
        }
    }
    out.push("top-level WorkerW windows:".to_string());
    let mut w = find("WorkerW", null_mut(), null_mut());
    while !w.is_null() {
        describe(w, 1, &mut out);
        let dv = find("SHELLDLL_DefView", w, null_mut());
        if !dv.is_null() {
            describe(dv, 2, &mut out);
        }
        w = find("WorkerW", null_mut(), w);
    }
    let mut pid = 0u32;
    // SAFETY: out-parameter is a local.
    unsafe { GetWindowThreadProcessId(progman, &mut pid) };
    out.push(format!("Progman owner process id: {pid}"));
    out
}
