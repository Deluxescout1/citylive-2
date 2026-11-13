//! The Windows run loop's platform half: the hidden helper window (display
//! changes, Explorer restarts, sleep and resume, the tray icon), the monitor
//! heads, and the "is anything looking at the wallpaper" check.

use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE, HWND, LPARAM, LRESULT, POINT, WPARAM,
};
use windows_sys::Win32::Media::timeBeginPeriod;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::{
    CreateMutexW, GetCurrentProcess, SetPriorityClass, SetProcessInformation,
    BELOW_NORMAL_PRIORITY_CLASS, PROCESS_POWER_THROTTLING_CURRENT_VERSION,
    PROCESS_POWER_THROTTLING_EXECUTION_SPEED, PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION,
    PROCESS_POWER_THROTTLING_STATE, ProcessPowerThrottling,
};
use windows_sys::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows_sys::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NOTIFYICONDATAW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
    DispatchMessageW, GetCursorPos, LoadIconW, LoadImageW, MsgWaitForMultipleObjectsEx,
    PeekMessageW, RegisterClassExW, RegisterWindowMessageW, SetForegroundWindow, TrackPopupMenu,
    TranslateMessage, HWND_MESSAGE, IDI_APPLICATION, IMAGE_ICON, LR_DEFAULTSIZE, MF_CHECKED,
    MF_DISABLED, MF_GRAYED, MF_SEPARATOR, MF_STRING, MSG, MWMO_INPUTAVAILABLE, PM_REMOVE,
    QS_ALLINPUT, TPM_BOTTOMALIGN, TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON, WM_APP,
    WM_CLOSE, WM_DISPLAYCHANGE, WM_ENDSESSION, WM_LBUTTONDBLCLK, WM_POWERBROADCAST, WM_QUERYENDSESSION,
    WM_QUIT, WM_RBUTTONUP, WNDCLASSEXW,
};

use super::desktop::{self, Kind, Target};
use super::monitors::{self, MonitorInfo};
use super::surface::{Placement, Surface};
use super::util::{hex, last_error, wide};
use crate::{layout, Head, RunStats, TrayAction, WallpaperConfig, WallpaperState, TERMINATING};

const HELPER_CLASS: &str = "CityLiveHelper";
const WM_TRAY: u32 = WM_APP + 1;
const TRAY_ID: u32 = 1;

const F_DISPLAY: u32 = 1;
const F_TASKBAR: u32 = 2;
const F_SUSPEND: u32 = 4;
const F_RESUME: u32 = 8;
static FLAGS: AtomicU32 = AtomicU32::new(0);
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);
static PAUSED: AtomicBool = AtomicBool::new(false);
static TRAY_TX: Mutex<Option<Sender<TrayAction>>> = Mutex::new(None);
static TRAY_DISPLAY: Mutex<Option<Arc<RwLock<sim::story::TrayStatus>>>> = Mutex::new(None);

const ID_SETTINGS: usize = 1;
const ID_PAUSE: usize = 2;
const ID_TIMELAPSE: usize = 3;
const ID_CHRONICLE: usize = 4;
const ID_WORLDS: usize = 5;
const ID_AUTOSTART: usize = 6;
const ID_QUIT: usize = 7;

/// Options the command line sets before the wallpaper starts.
#[derive(Clone, Debug)]
pub struct Options {
    /// `legacy`, `progman`, `toplevel` or `auto` (see `desktop::forced_mode`).
    pub attach: Option<String>,
    /// Pause when ordinary windows cover every monitor, not only for
    /// full-screen apps.
    pub pause_when_covered: bool,
    /// Never pause (a diagnostic switch).
    pub never_pause: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            attach: None,
            pause_when_covered: true,
            never_pause: false,
        }
    }
}

static OPTIONS: OnceLock<Options> = OnceLock::new();

pub fn set_options(o: Options) {
    let _ = OPTIONS.set(o);
}

fn options() -> Options {
    OPTIONS.get().cloned().unwrap_or_default()
}

/// Per-process setup that has to happen before any window exists: per-monitor
/// v2 DPI awareness (so the pixels are real pixels on every monitor), a 1 ms
/// timer (so 30 fps is 30 fps), and no EcoQoS / timer throttling (a process
/// with no visible foreground window is otherwise treated as background work
/// and its timers are stretched). The priority is below normal: a wallpaper
/// must never compete with the user's work.
pub fn init_process() {
    // SAFETY: documented process-wide settings calls with valid arguments.
    unsafe {
        if SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) == 0 {
            // Already set by the manifest: not an error.
            println!("win: DPI awareness already set (manifest)");
        }
        timeBeginPeriod(1);
        let state = PROCESS_POWER_THROTTLING_STATE {
            Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
            ControlMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED
                | PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION,
            StateMask: 0,
        };
        let ok = SetProcessInformation(
            GetCurrentProcess(),
            ProcessPowerThrottling,
            &state as *const _ as *const core::ffi::c_void,
            std::mem::size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
        );
        if ok == 0 {
            println!("win: power throttling opt-out not available ({})", last_error());
        }
        SetPriorityClass(GetCurrentProcess(), BELOW_NORMAL_PRIORITY_CLASS);
    }
}

unsafe extern "system" fn helper_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let taskbar = TASKBAR_CREATED.load(Ordering::Relaxed);
    if taskbar != 0 && msg == taskbar {
        FLAGS.fetch_or(F_TASKBAR, Ordering::Relaxed);
        return 0;
    }
    match msg {
        WM_DISPLAYCHANGE => {
            FLAGS.fetch_or(F_DISPLAY, Ordering::Relaxed);
            0
        }
        WM_POWERBROADCAST => {
            // PBT_APMSUSPEND 4; PBT_APMRESUMESUSPEND 7; PBT_APMRESUMEAUTOMATIC 0x12.
            match wp {
                4 => {
                    FLAGS.fetch_or(F_SUSPEND, Ordering::Relaxed);
                }
                7 | 0x12 => {
                    FLAGS.fetch_or(F_RESUME, Ordering::Relaxed);
                }
                _ => {}
            }
            1
        }
        WM_QUERYENDSESSION => 1,
        WM_ENDSESSION => {
            if wp != 0 {
                TERMINATING.store(true, Ordering::Relaxed);
            }
            0
        }
        WM_CLOSE => {
            TERMINATING.store(true, Ordering::Relaxed);
            0
        }
        WM_TRAY => {
            let ev = lp as u32 & 0xFFFF;
            if ev == WM_RBUTTONUP {
                unsafe { tray_menu(hwnd) };
            } else if ev == WM_LBUTTONDBLCLK {
                send_action(TrayAction::OpenSettings);
            }
            0
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}

fn send_action(a: TrayAction) {
    if let Ok(g) = TRAY_TX.lock() {
        if let Some(tx) = g.as_ref() {
            let _ = tx.send(a);
        }
    }
}

fn status_lines() -> (String, String) {
    let g = TRAY_DISPLAY.lock().ok();
    let s = g
        .as_ref()
        .and_then(|g| g.as_ref())
        .and_then(|d| d.read().ok().map(|s| (s.city.clone(), s.line.clone())));
    s.unwrap_or_default()
}

unsafe fn tray_menu(hwnd: HWND) {
    let (city, line) = status_lines();
    let menu = unsafe { CreatePopupMenu() };
    if menu.is_null() {
        return;
    }
    let add = |flags: u32, id: usize, text: &str| {
        let t = wide(text);
        unsafe { AppendMenuW(menu, flags, id, t.as_ptr()) };
    };
    let title = if city.is_empty() {
        "CityLive".to_string()
    } else {
        city
    };
    add(MF_STRING | MF_DISABLED | MF_GRAYED, 0, &title);
    if !line.is_empty() {
        add(MF_STRING | MF_DISABLED | MF_GRAYED, 0, &line);
    }
    unsafe { AppendMenuW(menu, MF_SEPARATOR, 0, null()) };
    add(MF_STRING, ID_SETTINGS, "Settings...");
    add(
        MF_STRING | if PAUSED.load(Ordering::Relaxed) { MF_CHECKED } else { 0 },
        ID_PAUSE,
        "Pause animation",
    );
    add(MF_STRING, ID_TIMELAPSE, "60-second time-lapse");
    add(MF_STRING, ID_CHRONICLE, "Open the chronicle");
    add(MF_STRING, ID_WORLDS, "Open the worlds folder");
    add(
        MF_STRING | if super::autostart::is_enabled() { MF_CHECKED } else { 0 },
        ID_AUTOSTART,
        "Start with Windows",
    );
    unsafe { AppendMenuW(menu, MF_SEPARATOR, 0, null()) };
    add(MF_STRING, ID_QUIT, "Quit CityLive");
    let mut pt = POINT { x: 0, y: 0 };
    unsafe {
        GetCursorPos(&mut pt);
        // Required so the menu closes when the user clicks elsewhere.
        SetForegroundWindow(hwnd);
        let id = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN,
            pt.x,
            pt.y,
            0,
            hwnd,
            null(),
        );
        DestroyMenu(menu);
        match id as usize {
            ID_SETTINGS => send_action(TrayAction::OpenSettings),
            ID_PAUSE => send_action(TrayAction::TogglePause),
            ID_TIMELAPSE => send_action(TrayAction::Timelapse),
            ID_CHRONICLE => send_action(TrayAction::OpenChronicle),
            ID_WORLDS => send_action(TrayAction::OpenWorlds),
            ID_AUTOSTART => {
                let now = super::autostart::is_enabled();
                match super::autostart::set(!now) {
                    Ok(()) => println!("tray: start with Windows {}", if now { "off" } else { "on" }),
                    Err(e) => eprintln!("tray: cannot change start with Windows: {e}"),
                }
            }
            ID_QUIT => send_action(TrayAction::Quit),
            _ => {}
        }
    }
}

fn tray_data(hwnd: HWND, tip: &str) -> NOTIFYICONDATAW {
    // SAFETY: zeroed is a valid empty NOTIFYICONDATAW; fields are set below.
    let mut nid: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
    nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = TRAY_ID;
    nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    nid.uCallbackMessage = WM_TRAY;
    // SAFETY: resource id 1 is the app icon from the .rc; the fallback is stock.
    nid.hIcon = unsafe {
        let hinst = GetModuleHandleW(null());
        let h = LoadImageW(hinst, 1 as *const u16, IMAGE_ICON, 0, 0, LR_DEFAULTSIZE);
        if h.is_null() {
            LoadIconW(null_mut(), IDI_APPLICATION)
        } else {
            h
        }
    };
    let t: Vec<u16> = tip.encode_utf16().take(127).collect();
    for (i, c) in t.iter().enumerate() {
        nid.szTip[i] = *c;
    }
    nid
}

pub struct Host {
    helper: HWND,
    mutex: HANDLE,
    mons: Vec<MonitorInfo>,
    target: Option<Target>,
    last_mon_poll: Instant,
    last_vis_poll: Instant,
    last_tip: Instant,
    rebuild_at: Option<Instant>,
    /// A display change was announced: look at the monitors at this time and
    /// rebuild only if they really are different.
    check_at: Option<Instant>,
    hidden_reason: Option<String>,
    suspended: bool,
    tray_added: bool,
    opts: Options,
}

impl Host {
    pub fn new() -> Result<Host, String> {
        // SAFETY: standard class registration and window creation.
        unsafe {
            let name = wide("Local\\CityLiveWallpaper");
            let mutex = CreateMutexW(null(), 0, name.as_ptr());
            if !mutex.is_null() && GetLastError() == ERROR_ALREADY_EXISTS {
                CloseHandle(mutex);
                return Err("CityLive is already running (single instance)".to_string());
            }
            let hinst = GetModuleHandleW(null());
            let class = wide(HELPER_CLASS);
            let wc = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                style: 0,
                lpfnWndProc: Some(helper_proc),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: hinst,
                hIcon: null_mut(),
                hCursor: null_mut(),
                hbrBackground: null_mut(),
                lpszMenuName: null(),
                lpszClassName: class.as_ptr(),
                hIconSm: null_mut(),
            };
            if RegisterClassExW(&wc) == 0 {
                return Err(format!("RegisterClassExW(helper) failed ({})", last_error()));
            }
            // A hidden top-level window: message-only windows do not receive
            // broadcasts (display change, TaskbarCreated), so this is not one.
            let title = wide("CityLive");
            let helper = CreateWindowExW(
                0,
                class.as_ptr(),
                title.as_ptr(),
                0,
                0,
                0,
                0,
                0,
                null_mut(),
                null_mut(),
                hinst,
                null(),
            );
            let _ = HWND_MESSAGE;
            if helper.is_null() {
                return Err(format!("CreateWindowExW(helper) failed ({})", last_error()));
            }
            let tb = wide("TaskbarCreated");
            TASKBAR_CREATED.store(RegisterWindowMessageW(tb.as_ptr()), Ordering::Relaxed);
            let now = Instant::now();
            Ok(Host {
                helper,
                mutex,
                mons: Vec::new(),
                target: None,
                last_mon_poll: now,
                last_vis_poll: now,
                last_tip: now,
                rebuild_at: None,
                check_at: None,
                hidden_reason: None,
                suspended: false,
                tray_added: false,
                opts: options(),
            })
        }
    }

    pub fn set_tray(&mut self, tx: Sender<TrayAction>, display: Option<Arc<RwLock<sim::story::TrayStatus>>>) {
        if let Ok(mut g) = TRAY_TX.lock() {
            *g = Some(tx);
        }
        if let Ok(mut g) = TRAY_DISPLAY.lock() {
            *g = display;
        }
        self.add_tray();
    }

    fn add_tray(&mut self) {
        let nid = tray_data(self.helper, "CityLive");
        // SAFETY: `nid` is fully initialised.
        let ok = unsafe { Shell_NotifyIconW(NIM_ADD, &nid) } != 0;
        self.tray_added = ok;
        if !ok {
            eprintln!("win: tray icon not added ({})", last_error());
        }
    }

    fn update_tip(&mut self, state: &WallpaperState) {
        if !self.tray_added {
            return;
        }
        let (city, line) = status_lines();
        let mut tip = if city.is_empty() { "CityLive".to_string() } else { city };
        if !line.is_empty() {
            tip.push_str("\n");
            tip.push_str(&line);
        }
        if state.hidden {
            tip.push_str("\n(paused: not visible)");
        }
        let nid = tray_data(self.helper, &tip);
        // SAFETY: `nid` is fully initialised.
        unsafe { Shell_NotifyIconW(NIM_MODIFY, &nid) };
    }

    /// Rebuild every head: find the desktop window, enumerate the monitors,
    /// create a surface per monitor.
    pub fn rebuild(&mut self, state: &mut WallpaperState) {
        self.rebuild_at = None;
        state.heads.clear(); // drops the old surfaces (windows and DIBs)
        let force = desktop::forced_mode(self.opts.attach.as_deref());
        let target = desktop::find_target(force, true);
        println!(
            "win: desktop target: {} parent={} below={}",
            target.kind.name(),
            hex(target.parent),
            hex(target.below)
        );
        for l in desktop::probe() {
            println!("win:   {l}");
        }
        for o in super::procs::scan_other_wallpapers() {
            println!(
                "win: another wallpaper program is running: {} (pid {}, {}). It may draw over or under this one; \
                 the settings app can replace the old CityLive.",
                o.label, o.pid, o.path
            );
        }
        self.mons = monitors::monitors();
        for m in &self.mons {
            println!(
                "win: monitor {}: ({},{}) {}x{} dpi {}{}",
                m.name,
                m.x,
                m.y,
                m.w,
                m.h,
                m.dpi,
                if m.primary { " primary" } else { "" }
            );
        }
        let mut placement = if target.kind == Kind::TopLevel || target.parent.is_null() {
            Placement::TopLevelBottom
        } else {
            Placement::Child(target.parent)
        };
        let mut heads: Vec<Head> = Vec::new();
        for m in &self.mons {
            if !layout::included(&m.name, &state.config.include_outputs, &state.config.ignore_outputs) {
                println!("win: ignoring monitor '{}'", m.name);
                continue;
            }
            let mut res = Surface::create(placement, m.x, m.y, m.w, m.h);
            if let (Err(e), Placement::Child(_)) = (&res, placement) {
                eprintln!("win: child window failed ({e}); falling back to a top-level window");
                placement = Placement::TopLevelBottom;
                res = Surface::create(placement, m.x, m.y, m.w, m.h);
            }
            match res {
                Ok(s) => {
                    if target.kind == Kind::Progman && !target.below.is_null() {
                        s.place_below(target.below);
                    }
                    println!("win: surface for {} = {}", m.name, hex(s.hwnd()));
                    heads.push(Head {
                        name: m.name.clone(),
                        surface: Some(s),
                        logical_pos: (m.x, m.y),
                        logical_size: (m.w as u32, m.h as u32),
                        scale_factor: 1,
                        configured: true,
                        frame_pending: false,
                        frame_requested_at: None,
                        last_drawn_seq: 0,
                    });
                }
                Err(e) => eprintln!("win: cannot create the surface for {}: {e}", m.name),
            }
        }
        if placement == Placement::TopLevelBottom {
            eprintln!(
                "win: WARNING: running as a plain window at the bottom of the z-order; \
                 the desktop icons are covered. Use --attach legacy|progman to force a mode."
            );
        }
        println!("win: {} head(s) ready", heads.len());
        self.target = Some(target);
        state.heads = heads;
        state.check_geometry_change();
        state.repaint_all();
    }

    /// Did the monitors change in a way that needs new surfaces? (A moved
    /// taskbar changes the work area only: not worth a rebuild.)
    fn monitors_changed(&self) -> bool {
        let now = monitors::monitors();
        let key = |m: &MonitorInfo| (m.name.clone(), m.x, m.y, m.w, m.h, m.primary);
        now.iter().map(key).collect::<Vec<_>>() != self.mons.iter().map(key).collect::<Vec<_>>()
    }

    fn surfaces_ok(&self, state: &WallpaperState) -> bool {
        let Some(t) = self.target else { return false };
        if t.kind != Kind::TopLevel {
            // SAFETY: IsWindow accepts any handle.
            if unsafe { windows_sys::Win32::UI::WindowsAndMessaging::IsWindow(t.parent) } == 0 {
                return false;
            }
        }
        // The desktop layer may have been replaced or moved (theme or wallpaper
        // change, slideshow, Explorer restart): the parent we would pick now must
        // be the one we are in, and the parent must be visible.
        let force = desktop::forced_mode(self.opts.attach.as_deref());
        let now = desktop::find_target(force, false);
        if now.kind != t.kind || now.parent != t.parent {
            return false;
        }
        // SAFETY: IsWindowVisible accepts any handle.
        if t.kind != Kind::TopLevel
            && unsafe { windows_sys::Win32::UI::WindowsAndMessaging::IsWindowVisible(t.parent) } == 0
        {
            return false;
        }
        state.heads.iter().all(|h| {
            h.surface.as_ref().is_some_and(|s| {
                s.alive()
                    && (t.kind == Kind::TopLevel
                        // SAFETY: GetParent accepts any handle.
                        || unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetParent(s.hwnd()) }
                            == t.parent)
            })
        })
    }

    /// Wait up to `timeout` for window messages, handle them, then do the
    /// periodic checks.
    pub fn pump(&mut self, state: &mut WallpaperState, timeout: Duration) -> Result<(), String> {
        let ms = timeout.as_millis().min(1000) as u32;
        // SAFETY: the standard message loop; MSG is plain data.
        unsafe {
            MsgWaitForMultipleObjectsEx(0, null(), ms, QS_ALLINPUT, MWMO_INPUTAVAILABLE);
            let mut msg: MSG = std::mem::zeroed();
            while PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) != 0 {
                if msg.message == WM_QUIT {
                    TERMINATING.store(true, Ordering::Relaxed);
                    break;
                }
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        self.housekeeping(state);
        PAUSED.store(state.animation_paused, Ordering::Relaxed);
        Ok(())
    }

    fn housekeeping(&mut self, state: &mut WallpaperState) {
        let now = Instant::now();
        let flags = FLAGS.swap(0, Ordering::Relaxed);
        if flags & F_TASKBAR != 0 {
            println!("win: Explorer restarted; re-attaching");
            self.add_tray();
            self.rebuild_at = Some(now + Duration::from_millis(2000));
        }
        if flags & F_DISPLAY != 0 && self.check_at.is_none() {
            self.check_at = Some(now + Duration::from_millis(700));
        }
        if let Some(t) = self.check_at {
            if now >= t {
                self.check_at = None;
                if self.rebuild_at.is_none() && (self.monitors_changed() || !self.surfaces_ok(state)) {
                    println!("win: display change; rebuilding");
                    self.rebuild_at = Some(now);
                }
            }
        }
        if flags & F_SUSPEND != 0 {
            println!("win: going to sleep");
            self.suspended = true;
            state.hidden = true;
        }
        if flags & F_RESUME != 0 {
            println!("win: woke up");
            self.suspended = false;
            // The sim must not "catch up" on the whole sleep in one step.
            state.last_sim_step = now;
            self.rebuild_at = Some(now + Duration::from_millis(2500));
        }
        if let Some(t) = self.rebuild_at {
            if now >= t {
                self.rebuild(state);
            }
        }
        // The slow checks.
        if now.duration_since(self.last_mon_poll) >= Duration::from_secs(2) {
            self.last_mon_poll = now;
            if self.rebuild_at.is_none()
                && (self.target.is_none() || self.monitors_changed() || !self.surfaces_ok(state))
            {
                println!("win: monitors or desktop windows changed; rebuilding");
                self.rebuild_at = Some(now + Duration::from_millis(300));
            }
        }
        if now.duration_since(self.last_vis_poll) >= Duration::from_secs(1) {
            self.last_vis_poll = now;
            let reason = if self.suspended {
                Some("asleep".to_string())
            } else if self.opts.never_pause {
                None
            } else {
                // Fresh work areas (the taskbar may have moved or hidden).
                monitors::hidden_reason(&monitors::monitors(), self.opts.pause_when_covered)
            };
            let hidden = reason.is_some();
            if hidden != state.hidden {
                match &reason {
                    Some(r) => println!("win: hidden ({r}): no frames until it is visible again"),
                    None => println!("win: visible again"),
                }
                state.hidden = hidden;
                if !hidden {
                    state.repaint_all();
                }
                self.update_tip(state);
            }
            self.hidden_reason = reason;
        }
        if now.duration_since(self.last_tip) >= Duration::from_secs(30) {
            self.last_tip = now;
            self.update_tip(state);
        }
    }

    pub fn shutdown(&mut self, state_heads: &mut Vec<Head>) {
        state_heads.clear();
        if self.tray_added {
            let nid = tray_data(self.helper, "");
            // SAFETY: `nid` is fully initialised.
            unsafe { Shell_NotifyIconW(NIM_DELETE, &nid) };
            self.tray_added = false;
        }
        // SAFETY: both handles were created in `new`.
        unsafe {
            DestroyWindow(self.helper);
            CloseHandle(self.mutex);
        }
    }
}

/// Run the wallpaper on the Windows desktop until it is told to stop.
pub fn run(config: WallpaperConfig) -> Result<RunStats, Box<dyn std::error::Error>> {
    crate::setup_signal_handlers();
    init_process();
    // Frames, fps, CPU and memory every 10 s in the log: proof it is painting.
    let mut config = config;
    config.stats = true;
    let mut host = Host::new()?;
    let mut state = WallpaperState::new(config.clone());
    let (tx, rx) = std::sync::mpsc::channel();
    host.set_tray(tx, config.tray_display.clone());
    if state.tray_rx.is_none() {
        state.tray_rx = Some(Arc::new(Mutex::new(rx)));
    }
    host.rebuild(&mut state);
    // The host is needed again after the loop (to shut down), and the loop's
    // closure borrows it, so the heads are taken out of the state's reach by
    // the loop's own exit: dropping the state drops the surfaces.
    let host_cell = std::cell::RefCell::new(host);
    let res = crate::run_loop(
        config,
        state,
        &mut |st, t| host_cell.borrow_mut().pump(st, t),
        &mut || {},
    );
    let mut dummy: Vec<Head> = Vec::new();
    host_cell.borrow_mut().shutdown(&mut dummy);
    res
}
