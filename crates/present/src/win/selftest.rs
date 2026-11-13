//! `--selftest`, `--diag` and `--show-pattern`: diagnosing the Windows build
//! from text.
//!
//! `--diag` prints what the machine looks like (Windows build, monitors, DPI,
//! the desktop's window tree, paths) and exits. `--selftest` adds a headless
//! run of the real wallpaper loop (world, sim, sky, weather, animation and the
//! slice into per-monitor buffers, without any window) with timings, CPU and
//! memory, a GDI blit benchmark, a network check, and writes a picture. It
//! prints PASS / WARN / FAIL lines and a final `SELFTEST RESULT`, and saves the
//! same text to `selftest.txt` beside the log. `--show-pattern` attaches to the
//! desktop for a few seconds and paints a test pattern, so the attach can be
//! checked by eye (the desktop icons must stay on top of it).

use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use std::time::{Duration, Instant};

use windows_sys::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject,
    GdiFlush, GetDC, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    HGDIOBJ, SRCCOPY,
};
use windows_sys::Win32::System::Registry::{
    RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ,
};
use windows_sys::Win32::UI::HiDpi::{
    GetAwarenessFromDpiAwarenessContext, GetThreadDpiAwarenessContext,
};

use super::desktop::{self, Kind};
use super::monitors::{self, MonitorInfo};
use super::surface::{Placement, Surface};
use super::util::{from_wide, wide};
use crate::{layout, plat, Head, WallpaperConfig, WallpaperState};

/// What `--selftest` was asked to do.
#[derive(Clone, Debug)]
pub struct Args {
    /// Seconds of the headless wallpaper run.
    pub secs: f64,
    /// `real` (this machine's monitors), `triple` (3 heads, 7040x1440) or `single`.
    pub layout: String,
    /// Where the picture and report go (default: the data folder).
    pub out: Option<PathBuf>,
    pub attach: Option<String>,
}

impl Default for Args {
    fn default() -> Self {
        Args {
            secs: 15.0,
            layout: "real".into(),
            out: None,
            attach: None,
        }
    }
}

struct Report {
    lines: Vec<String>,
    fails: u32,
    warns: u32,
}

impl Report {
    fn new() -> Self {
        Report {
            lines: Vec::new(),
            fails: 0,
            warns: 0,
        }
    }
    fn say(&mut self, s: String) {
        println!("{s}");
        self.lines.push(s);
    }
    fn info(&mut self, what: &str, detail: impl AsRef<str>) {
        self.say(format!("INFO  {what}: {}", detail.as_ref()));
    }
    fn pass(&mut self, what: &str, detail: impl AsRef<str>) {
        self.say(format!("PASS  {what}: {}", detail.as_ref()));
    }
    fn warn(&mut self, what: &str, detail: impl AsRef<str>) {
        self.warns += 1;
        self.say(format!("WARN  {what}: {}", detail.as_ref()));
    }
    fn fail(&mut self, what: &str, detail: impl AsRef<str>) {
        self.fails += 1;
        self.say(format!("FAIL  {what}: {}", detail.as_ref()));
    }
    fn raw(&mut self, s: &str) {
        self.say(format!("      {s}"));
    }
}

fn reg_string(sub: &str, name: &str) -> Option<String> {
    let (s, n) = (wide(sub), wide(name));
    let mut buf = vec![0u16; 256];
    let mut size = (buf.len() * 2) as u32;
    // SAFETY: `size` is the buffer's byte length.
    let rc = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            s.as_ptr(),
            n.as_ptr(),
            RRF_RT_REG_SZ,
            null_mut(),
            buf.as_mut_ptr() as *mut _,
            &mut size,
        )
    };
    (rc == 0).then(|| from_wide(&buf))
}

fn reg_dword(sub: &str, name: &str) -> Option<u32> {
    let (s, n) = (wide(sub), wide(name));
    let mut v = 0u32;
    let mut size = 4u32;
    // SAFETY: `size` is the byte length of `v`.
    let rc = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            s.as_ptr(),
            n.as_ptr(),
            RRF_RT_REG_DWORD,
            null_mut(),
            &mut v as *mut u32 as *mut _,
            &mut size,
        )
    };
    (rc == 0).then_some(v)
}

fn writable(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let f = dir.join(".write-test");
    std::fs::write(&f, b"x").map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&f);
    Ok(())
}

/// The machine and the desktop, without drawing anything.
fn diag(r: &mut Report, attach: Option<&str>) -> Vec<MonitorInfo> {
    let cv = "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion";
    r.info(
        "windows",
        format!(
            "{} {} build {}.{}",
            reg_string(cv, "ProductName").unwrap_or_else(|| "?".into()),
            reg_string(cv, "DisplayVersion").unwrap_or_default(),
            reg_string(cv, "CurrentBuildNumber").unwrap_or_else(|| "?".into()),
            reg_dword(cv, "UBR").unwrap_or(0)
        ),
    );
    let build: u32 = reg_string(cv, "CurrentBuildNumber")
        .and_then(|b| b.parse().ok())
        .unwrap_or(0);
    if build >= 22000 {
        r.pass("windows 11", format!("build {build}"));
    } else if build > 0 {
        r.warn("windows 11", format!("build {build} is not Windows 11 (22000+); not the target"));
    }
    if build >= 26100 {
        r.info("layout", "24H2 or later: the desktop is expected in the Progman layout");
    }
    r.info("cpu threads", format!("{}", std::thread::available_parallelism().map(|n| n.get()).unwrap_or(0)));
    // SAFETY: both calls only read the thread's DPI context.
    let aw = unsafe { GetAwarenessFromDpiAwarenessContext(GetThreadDpiAwarenessContext()) };
    // DPI_AWARENESS: 0 unaware, 1 system, 2 per-monitor.
    if aw == 2 {
        r.pass("dpi awareness", "per-monitor (real pixels on every monitor)");
    } else {
        r.fail("dpi awareness", format!("value {aw}, expected 2 (per-monitor)"));
    }
    r.info("exe", std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default());
    for (what, dir) in [
        ("config folder", plat::config_dir()),
        ("data folder", plat::data_dir()),
        ("cache folder", plat::cache_dir()),
    ] {
        match writable(&dir) {
            Ok(()) => r.pass(what, format!("{} (writable)", dir.display())),
            Err(e) => r.fail(what, format!("{}: {e}", dir.display())),
        }
    }
    r.info("config file", crate::config::config_path().display().to_string());
    r.info("save file", crate::default_save_path().display().to_string());
    r.info("log file", super::log::log_file("citylive").display().to_string());
    r.info(
        "start with windows",
        match super::autostart::registered() {
            Some(c) => format!("on ({c})"),
            None => "off".to_string(),
        },
    );

    let mons = monitors::monitors();
    if mons.is_empty() {
        r.fail("monitors", "none found");
    } else {
        r.pass("monitors", format!("{} found", mons.len()));
    }
    let (mut minx, mut miny, mut maxx, mut maxy) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for m in &mons {
        r.info(
            "monitor",
            format!(
                "{} at ({},{}) {}x{} work {:?} dpi {} ({}%){}",
                m.name,
                m.x,
                m.y,
                m.w,
                m.h,
                m.work,
                m.dpi,
                m.dpi * 100 / 96,
                if m.primary { " primary" } else { "" }
            ),
        );
        minx = minx.min(m.x);
        miny = miny.min(m.y);
        maxx = maxx.max(m.x + m.w);
        maxy = maxy.max(m.y + m.h);
    }
    if !mons.is_empty() {
        r.info("virtual desktop", format!("{}x{} from ({minx},{miny})", maxx - minx, maxy - miny));
        let specs: Vec<layout::OutputSpec> = mons
            .iter()
            .map(|m| layout::OutputSpec::new(&m.name, m.x, m.y, m.w as u32, m.h as u32))
            .collect();
        match layout::layout(&specs, &[], &[], None) {
            Some(l) => r.info(
                "city layout",
                format!(
                    "scale {}x, internal framebuffer {}x{} (founded on head {})",
                    l.scale, l.fb_w, l.fb_h, l.focus_index
                ),
            ),
            None => r.fail("city layout", "no layout for these monitors"),
        }
    }

    // The desktop's windows: what is there before and after the 0x052C message.
    r.info("desktop", "window tree before the WorkerW message:");
    for l in desktop::probe() {
        r.raw(&l);
    }
    let force = desktop::forced_mode(attach);
    let t = desktop::find_target(force, true);
    r.info("desktop", "window tree after it:");
    for l in desktop::probe() {
        r.raw(&l);
    }
    match t.kind {
        Kind::TopLevel => r.fail(
            "desktop attach",
            "no WorkerW / Progman layer found: the wallpaper would have to be a plain window",
        ),
        k => r.pass(
            "desktop attach",
            format!("{} parent={} below={}", k.name(), super::util::hex(t.parent), super::util::hex(t.below)),
        ),
    }
    let others = super::procs::scan_other_wallpapers();
    if others.is_empty() {
        r.info("other wallpaper programs", "none running");
    }
    for o in &others {
        r.warn("other wallpaper program", format!("{} pid {} {}", o.label, o.pid, o.path));
    }
    let hidden = monitors::hidden_reason(&mons, true);
    r.info(
        "visibility now",
        hidden.clone().map(|h| format!("hidden: {h}")).unwrap_or_else(|| "visible".into()),
    );
    if !mons.is_empty() {
        r.info("covered monitors", format!("{:?}", monitors::covered_monitors(&mons)));
    }
    mons
}

/// Does what we paint really reach the screen? Attach to the desktop on the
/// primary monitor, paint a known colour, let DWM compose, and read one pixel
/// back from the screen DC in the lower right (no icons there). Two colours,
/// so a coincidence is not a pass. The window is removed again afterwards.
fn screen_check(r: &mut Report, mons: &[MonitorInfo], attach: Option<&str>) {
    use windows_sys::Win32::Graphics::Dwm::DwmFlush;
    use windows_sys::Win32::Graphics::Gdi::GetPixel;
    let Some(m) = mons.iter().find(|m| m.primary).or(mons.first()) else {
        return;
    };
    let t = desktop::find_target(desktop::forced_mode(attach), true);
    let placement = if t.kind == Kind::TopLevel || t.parent.is_null() {
        Placement::TopLevelBottom
    } else {
        Placement::Child(t.parent)
    };
    let mut s = match Surface::create(placement, m.x, m.y, m.w, m.h) {
        Ok(s) => s,
        Err(e) => {
            r.fail("screen check", format!("cannot create the window: {e}"));
            return;
        }
    };
    if matches!(t.kind, Kind::Progman) && !t.below.is_null() {
        s.place_below(t.below);
    }
    let (px, py) = (m.x + m.w * 4 / 5, m.y + m.h * 3 / 5);
    // The reliable check: the window is visible (so every parent is), its parent
    // is the desktop layer we meant, and its clip region is not empty.
    s.pixels_mut().fill(0x00ff_00ff);
    s.present_all();
    pump_for(Duration::from_millis(200));
    // SAFETY: read-only window and DC queries; the DC is released.
    let (visible, parent_class, clip) = unsafe {
        use windows_sys::Win32::Foundation::RECT;
        use windows_sys::Win32::Graphics::Gdi::GetClipBox;
        use windows_sys::Win32::UI::WindowsAndMessaging::{GetParent, IsWindowVisible};
        let v = IsWindowVisible(s.hwnd()) != 0;
        let pc = super::util::class_name(GetParent(s.hwnd()));
        let dc = GetDC(s.hwnd());
        let mut rc = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        let kind = GetClipBox(dc, &mut rc);
        ReleaseDC(s.hwnd(), dc);
        (v, pc, (kind, rc.right - rc.left, rc.bottom - rc.top))
    };
    // GetClipBox: 0 error, 1 NULLREGION, 2 SIMPLEREGION, 3 COMPLEXREGION.
    let clip_ok = clip.0 >= 2 && clip.1 > 0 && clip.2 > 0;
    r.info(
        "our window",
        format!(
            "visible={visible}, parent class '{parent_class}', clip region kind {} {}x{}",
            clip.0, clip.1, clip.2
        ),
    );
    // The pixel read-back is informational only: GetPixel on the screen DC is
    // unreliable on a basic display adapter (reads the wrong layer).
    for (name, rgb) in [("magenta", (255u32, 0u32, 255u32)), ("green", (0, 200, 0))] {
        let word = rgb.0 << 16 | rgb.1 << 8 | rgb.2;
        s.pixels_mut().fill(word);
        s.present_all();
        pump_for(Duration::from_millis(150));
        // SAFETY: DwmFlush waits for the next composition; the screen DC is
        // released; GetPixel reads one pixel.
        let got = unsafe {
            DwmFlush();
            pump_for(Duration::from_millis(100));
            let dc = GetDC(null_mut());
            let c = GetPixel(dc, px, py);
            ReleaseDC(null_mut(), dc);
            c
        };
        let (gr, gg, gb) = (got & 255, (got >> 8) & 255, (got >> 16) & 255);
        r.info(
            "screen pixel (informational)",
            format!("painted {name} {:?}, read ({gr},{gg},{gb}) at ({px},{py})", rgb),
        );
    }
    drop(s);
    let parent_ok = parent_class == "WorkerW" || parent_class == "Progman";
    if visible && clip_ok && parent_ok {
        r.pass(
            "window reaches the desktop",
            format!("visible child of {parent_class}, non-empty clip ({})", t.kind.name()),
        );
    } else {
        r.fail(
            "window reaches the desktop",
            format!(
                "visible={visible}, parent '{parent_class}', clip {clip:?} in mode {}; try --attach progman|legacy|progman-workerw",
                t.kind.name()
            ),
        );
    }
}

/// `--diag`: print and exit.
pub fn run_diag(attach: Option<&str>) -> i32 {
    let mut r = Report::new();
    let mons = diag(&mut r, attach);
    screen_check(&mut r, &mons, attach);
    save_report(&r, None);
    if r.fails > 0 {
        1
    } else {
        0
    }
}

fn save_report(r: &Report, out: Option<&Path>) {
    let dir = out.map(|p| p.to_path_buf()).unwrap_or_else(plat::data_dir);
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(dir.join("selftest.txt"), r.lines.join("\r\n"));
}

/// Write a 32-bit top-down BMP of `w` x `h` pixels cut from `px` (row stride
/// `stride`).
fn write_bmp(path: &Path, px: &[u32], stride: usize, x0: usize, y0: usize, w: usize, h: usize) -> std::io::Result<()> {
    let mut data = Vec::with_capacity(54 + w * h * 4);
    data.extend_from_slice(b"BM");
    data.extend_from_slice(&((54 + w * h * 4) as u32).to_le_bytes());
    data.extend_from_slice(&[0; 4]);
    data.extend_from_slice(&54u32.to_le_bytes());
    data.extend_from_slice(&40u32.to_le_bytes());
    data.extend_from_slice(&(w as i32).to_le_bytes());
    data.extend_from_slice(&(-(h as i32)).to_le_bytes());
    data.extend_from_slice(&1u16.to_le_bytes());
    data.extend_from_slice(&32u16.to_le_bytes());
    data.extend_from_slice(&[0; 24]);
    for y in 0..h {
        let row = &px[(y0 + y) * stride + x0..(y0 + y) * stride + x0 + w];
        for p in row {
            data.extend_from_slice(&p.to_le_bytes());
        }
    }
    std::fs::write(path, data)
}

/// The monitors the headless run pretends to have.
fn headless_monitors(kind: &str, real: &[MonitorInfo]) -> Vec<MonitorInfo> {
    let mk = |name: &str, x: i32, w: i32, h: i32| MonitorInfo {
        name: name.to_string(),
        x,
        y: 0,
        w,
        h,
        work: (x, 0, x + w, h),
        primary: x == 0,
        dpi: 96,
    };
    match kind {
        // The reference desk: 2560 + 2560 + 1920 = 7040 wide, 1440 tall.
        "triple" => vec![mk("DISPLAY1", 0, 2560, 1440), mk("DISPLAY2", 2560, 2560, 1440), mk("DISPLAY3", 5120, 1920, 1080)],
        "single" => vec![mk("DISPLAY1", 0, 1920, 1080)],
        _ if !real.is_empty() => real.to_vec(),
        _ => vec![mk("DISPLAY1", 0, 1920, 1080)],
    }
}

/// How long a full-monitor BitBlt and a 256x256 one take (memory to memory:
/// the cost of GDI's copy itself, not of the screen).
fn gdi_bench(r: &mut Report, mons: &[MonitorInfo]) {
    let Some(m) = mons.iter().max_by_key(|m| m.w * m.h) else {
        return;
    };
    // SAFETY: a private DIB and a compatible bitmap, both freed at the end.
    unsafe {
        let screen = GetDC(null_mut());
        let src = CreateCompatibleDC(screen);
        let dst = CreateCompatibleDC(screen);
        let mut bmi: BITMAPINFO = std::mem::zeroed();
        bmi.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: m.w,
            biHeight: -m.h,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            biSizeImage: 0,
            biXPelsPerMeter: 0,
            biYPelsPerMeter: 0,
            biClrUsed: 0,
            biClrImportant: 0,
        };
        let mut bits: *mut core::ffi::c_void = null_mut();
        let dib = CreateDIBSection(src, &bmi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        let target = CreateCompatibleBitmap(screen, m.w, m.h);
        if dib.is_null() || target.is_null() {
            r.warn("gdi blit", "could not create the benchmark bitmaps");
        } else {
            let o1 = SelectObject(src, dib as HGDIOBJ);
            let o2 = SelectObject(dst, target as HGDIOBJ);
            let time = |w: i32, h: i32, n: u32| -> f64 {
                let t = Instant::now();
                for _ in 0..n {
                    BitBlt(dst, 0, 0, w, h, src, 0, 0, SRCCOPY);
                    GdiFlush();
                }
                t.elapsed().as_secs_f64() * 1000.0 / n as f64
            };
            let full = time(m.w, m.h, 20);
            let small = time(256.min(m.w), 256.min(m.h), 200);
            let msg = format!("{}x{} full copy {:.2} ms, 256x256 copy {:.3} ms", m.w, m.h, full, small);
            if full < 25.0 {
                r.pass("gdi blit", msg);
            } else {
                r.warn("gdi blit", format!("{msg} (slow: a full-monitor copy should be well under 25 ms)"));
            }
            SelectObject(src, o1);
            SelectObject(dst, o2);
        }
        if !dib.is_null() {
            DeleteObject(dib as HGDIOBJ);
        }
        if !target.is_null() {
            DeleteObject(target as HGDIOBJ);
        }
        DeleteDC(src);
        DeleteDC(dst);
        ReleaseDC(null_mut(), screen);
    }
}

#[cfg(feature = "net")]
fn net_check(r: &mut Report) {
    use world::HttpClient;
    let t = Instant::now();
    let c = world::UreqClient::new();
    match c.get("https://api.open-meteo.com/v1/forecast?latitude=0&longitude=0&current=temperature_2m") {
        Ok(body) if body.contains("temperature_2m") => {
            r.pass("network (https)", format!("Open-Meteo answered in {:.0} ms", t.elapsed().as_secs_f64() * 1000.0))
        }
        Ok(_) => r.warn("network (https)", "answered, but not the expected data"),
        Err(e) => r.warn("network (https)", format!("{e} (live weather will fall back to the cache or a clear sky)")),
    }
}

#[cfg(not(feature = "net"))]
fn net_check(r: &mut Report) {
    r.info("network (https)", "not built in (the net feature is off)");
}

/// `--selftest`.
pub fn run_selftest(mut cfg: WallpaperConfig, args: Args) -> i32 {
    let mut r = Report::new();
    r.say(format!("CityLive {} selftest", env!("CARGO_PKG_VERSION")));
    let real = diag(&mut r, args.attach.as_deref());
    screen_check(&mut r, &real, args.attach.as_deref());
    gdi_bench(&mut r, &real);
    net_check(&mut r);

    // The headless run: the real loop, no windows.
    let mons = headless_monitors(&args.layout, &real);
    r.info(
        "render run",
        format!(
            "{} monitor(s) ({}), {:.0} s, world seed 7, weather clear",
            mons.len(),
            args.layout,
            args.secs
        ),
    );
    let out_dir = args.out.clone().unwrap_or_else(plat::data_dir);
    let _ = std::fs::create_dir_all(&out_dir);
    let save = out_dir.join("selftest.sav");
    let _ = std::fs::remove_file(&save);
    cfg.save_path = save.clone();
    cfg.seed = 7;
    cfg.seed_forced = true;
    cfg.weather = Some("clear".to_string());
    cfg.seconds = args.secs;
    cfg.stats = true;
    cfg.sim_days = cfg.sim_days.max(20.0);
    cfg.tray_display = None;
    cfg.tray_rx = None;
    let fps = cfg.fps;

    let t_setup = Instant::now();
    let mut state = WallpaperState::new(cfg.clone());
    for m in &mons {
        state.heads.push(Head {
            name: m.name.clone(),
            surface: Some(Surface::headless(m.w, m.h)),
            logical_pos: (m.x, m.y),
            logical_size: (m.w as u32, m.h as u32),
            scale_factor: 1,
            configured: true,
            frame_pending: false,
            frame_requested_at: None,
            last_drawn_seq: 0,
        });
    }
    state.check_geometry_change();
    let started = Instant::now();
    let mut shot_done = false;
    let mut shot_info = String::new();
    let mut first_frame_ms: Option<f64> = None;
    let out_for_shot = out_dir.clone();
    let secs = args.secs;
    let res = crate::run_loop(
        cfg,
        state,
        &mut |st, timeout| {
            if first_frame_ms.is_none() && st.current_union.is_some() && st.frame_seq > 1 {
                first_frame_ms = Some(t_setup.elapsed().as_secs_f64() * 1000.0);
            }
            if !shot_done && started.elapsed().as_secs_f64() > secs * 0.8 && st.current_union.is_some() {
                shot_done = true;
                let (w, h) = (st.current_fb.w, st.current_fb.h);
                let px = st.current_fb.px();
                let n = px.len().max(1);
                let (mut lum, mut dark) = (0u64, 0usize);
                for p in px.iter().step_by(7) {
                    let (rr, gg, bb) = ((p >> 16) & 255, (p >> 8) & 255, p & 255);
                    let l = (rr * 3 + gg * 6 + bb) as u64 / 10;
                    lum += l;
                    if l < 8 {
                        dark += 1;
                    }
                }
                let samples = (n / 7).max(1) as u64;
                let (cw, ch) = (w.min(1920), h.min(1080));
                let x0 = (w - cw) / 2;
                let y0 = h - ch;
                let path = out_for_shot.join("selftest.bmp");
                let saved = write_bmp(&path, px, w, x0, y0, cw, ch);
                shot_info = format!(
                    "framebuffer {w}x{h}, mean luminance {}/255, {:.1}% black pixels; picture {} ({:?})",
                    lum / samples,
                    dark as f64 * 100.0 / samples as f64,
                    path.display(),
                    saved.map(|_| "written").map_err(|e| e.to_string())
                );
                if lum / samples < 10 || dark as f64 / samples as f64 > 0.9 {
                    shot_info.push_str(" -- LOOKS BLANK");
                }
            }
            // No window messages here: just wait for the next frame.
            std::thread::sleep(timeout.min(Duration::from_millis(4)));
            Ok(())
        },
        &mut || {},
    );
    let _ = std::fs::remove_file(&save);
    match res {
        Ok(s) => {
            if let Some(ms) = first_frame_ms {
                if ms < 20_000.0 {
                    r.pass("first frame", format!("{ms:.0} ms after start (world founding included)"));
                } else {
                    r.warn("first frame", format!("{ms:.0} ms after start (slow)"));
                }
            } else {
                r.fail("first frame", "no frame was produced");
            }
            r.info(
                "frames",
                format!(
                    "{} loop ticks, {} frames committed in {:.1} s = {:.1} fps (cap {fps})",
                    s.total_ticks, s.committed_frames, s.elapsed_secs, s.avg_fps
                ),
            );
            if s.committed_frames > 0 {
                r.pass("render loop", "frames were produced");
            } else {
                r.fail("render loop", "no frame was committed");
            }
            r.info(
                "cost",
                format!(
                    "sim {:.3} ms, composite {:.3} ms, present (slice) {:.3} ms per frame",
                    s.avg_sim_ms, s.avg_comp_ms, s.avg_present_ms
                ),
            );
            let cpu = s.cpu_pct_one_core;
            let msg = format!("{cpu:.1}% of one core over {:.0} s ({:.2} CPU seconds)", s.elapsed_secs, s.cpu_seconds);
            if cpu < 25.0 {
                r.pass("cpu", msg);
            } else {
                r.warn("cpu", format!("{msg} (high: the budget is a few percent)"));
            }
            let mb = s.rss_peak_kb as f64 / 1024.0;
            let msg = format!("peak working set {mb:.0} MB");
            if mb < 700.0 {
                r.pass("memory", msg);
            } else {
                r.warn("memory", format!("{msg} (high)"));
            }
        }
        Err(e) => r.fail("render loop", format!("error: {e}")),
    }
    if shot_info.is_empty() {
        r.fail("picture", "no picture was taken");
    } else if shot_info.contains("LOOKS BLANK") {
        r.fail("picture", shot_info);
    } else {
        r.pass("picture", shot_info);
    }
    let verdict = if r.fails == 0 { "PASS" } else { "FAIL" };
    r.say(format!(
        "SELFTEST RESULT: {verdict} ({} failure(s), {} warning(s)); report: {}",
        r.fails,
        r.warns,
        out_dir.join("selftest.txt").display()
    ));
    save_report(&r, Some(&out_dir));
    if r.fails == 0 {
        0
    } else {
        1
    }
}

/// Paint one monitor's pattern: a white frame and diagonals, a grid, and
/// `marker` white squares in the lower middle (so a screenshot says which
/// attach mode it shows).
fn paint_pattern(px: &mut [u32], w: usize, h: usize, colour: u32, marker: usize) {
    for y in 0..h {
        for x in 0..w {
            let edge = x < 12 || y < 12 || x >= w - 12 || y >= h - 12;
            let diag = (x * h / w).abs_diff(y) < 6 || ((w - 1 - x) * h / w).abs_diff(y) < 6;
            let grid = x % 120 < 2 || y % 120 < 2;
            px[y * w + x] = if edge || diag {
                0x00ff_ffff
            } else if grid {
                colour | 0x00_40_40_40
            } else {
                colour
            };
        }
    }
    let (cx, cy) = (w / 2, h * 3 / 4);
    for i in 0..marker {
        let x0 = cx + i * 90;
        for y in cy..(cy + 60).min(h) {
            for x in x0..(x0 + 60).min(w) {
                px[y * w + x] = 0x00ff_ffff;
            }
        }
    }
}

fn pump_for(d: Duration) {
    let end = Instant::now() + d;
    while Instant::now() < end {
        // SAFETY: a standard message loop so the windows stay responsive.
        unsafe {
            use windows_sys::Win32::UI::WindowsAndMessaging::{
                DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE,
            };
            let mut msg: MSG = std::mem::zeroed();
            while PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Show the pattern on every monitor for `secs` using one attach mode.
fn show_mode(mons: &[MonitorInfo], force: Option<Kind>, secs: f64, marker: usize) {
    let t = desktop::find_target(force, true);
    println!(
        "pattern: mode {} parent={} below={} ({marker} white square(s) on screen)",
        t.kind.name(),
        super::util::hex(t.parent),
        super::util::hex(t.below)
    );
    let placement = if t.kind == Kind::TopLevel || t.parent.is_null() {
        Placement::TopLevelBottom
    } else {
        Placement::Child(t.parent)
    };
    let mut surfaces: Vec<Surface> = Vec::new();
    for m in mons {
        match Surface::create(placement, m.x, m.y, m.w, m.h) {
            Ok(mut s) => {
                if t.kind == Kind::Progman && !t.below.is_null() {
                    s.place_below(t.below);
                }
                let base: [u32; 4] = [0x00_20_60_c0, 0x00_30_a0_50, 0x00_c0_60_20, 0x00_a0_30_a0];
                let colour = base[surfaces.len() % base.len()];
                paint_pattern(s.pixels_mut(), m.w as usize, m.h as usize, colour, marker);
                s.present_all();
                println!("pattern:   {} -> window {}", m.name, super::util::hex(s.hwnd()));
                surfaces.push(s);
            }
            Err(e) => eprintln!("pattern:   {}: {e}", m.name),
        }
    }
    println!(
        "pattern: {} surface(s) shown for {secs:.0} s. The icons and the taskbar must stay visible above it.",
        surfaces.len()
    );
    pump_for(Duration::from_secs_f64(secs));
    drop(surfaces);
}

/// `--show-pattern [secs]`: attach to the desktop, paint a test pattern on
/// every monitor for a few seconds, then detach. For checking by eye that the
/// attach works and the desktop icons stay on top. With `--attach cycle` it
/// goes through every mode in turn (1 to 4 white squares).
pub fn run_pattern(secs: f64, attach: Option<&str>) -> i32 {
    super::host::init_process();
    let mons = monitors::monitors();
    if attach == Some("cycle") {
        let modes = [Kind::Legacy, Kind::Progman, Kind::ProgmanWorkerW, Kind::TopLevel];
        for (i, k) in modes.iter().enumerate() {
            show_mode(&mons, Some(*k), secs, i + 1);
        }
    } else {
        show_mode(&mons, desktop::forced_mode(attach), secs, 1);
    }
    println!("pattern: done");
    0
}
