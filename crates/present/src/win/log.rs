//! Logging for a program with no console.
//!
//! The wallpaper and the settings app are GUI-subsystem programs, so
//! `println!` and `eprintln!` would go nowhere. [`init`] points both standard
//! handles at a pipe; a thread stamps every line with the time and writes it to
//! `%LOCALAPPDATA%\CityLive\<name>.log`, and to the console or redirect the
//! program was started from, if there is one. Everything the program prints is
//! therefore in the log without any call site changing. Panics are written to
//! the log too (the release build aborts on panic).

use std::io::Write;
use std::path::PathBuf;
use std::ptr::{null, null_mut};
use std::sync::Mutex;

use windows_sys::Win32::Foundation::{
    CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, ReadFile, WriteFile, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::Console::{
    AttachConsole, GetStdHandle, SetStdHandle, ATTACH_PARENT_PROCESS, STD_ERROR_HANDLE,
    STD_OUTPUT_HANDLE,
};
use windows_sys::Win32::System::Pipes::CreatePipe;

use super::util::wide;
use crate::plat;

struct Live {
    write: HANDLE,
    orig_out: HANDLE,
    orig_err: HANDLE,
    thread: Option<std::thread::JoinHandle<()>>,
}

// SAFETY: raw handles are just integers owned by this module.
unsafe impl Send for Live {}

static LIVE: Mutex<Option<Live>> = Mutex::new(None);
static PATH: Mutex<Option<PathBuf>> = Mutex::new(None);

/// The log file for `name` (`citylive`, `citylive-settings`).
pub fn log_file(name: &str) -> PathBuf {
    plat::data_dir().join(format!("{name}.log"))
}

fn stamp() -> String {
    use windows_sys::Win32::System::SystemInformation::GetLocalTime;
    // SAFETY: fills a struct we own.
    let t = unsafe {
        let mut t = std::mem::zeroed();
        GetLocalTime(&mut t);
        t
    };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
        t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond, t.wMilliseconds
    )
}

fn valid(h: HANDLE) -> bool {
    !h.is_null() && h != INVALID_HANDLE_VALUE
}

fn write_handle(h: HANDLE, data: &[u8]) {
    let mut done = 0u32;
    // SAFETY: `data` is a valid slice for the length given.
    unsafe {
        WriteFile(h, data.as_ptr(), data.len() as u32, &mut done, null_mut());
    }
}

/// Start logging. `console`: also attach to the parent console (for a program
/// run from a terminal with a visible result, like `--selftest`).
pub fn init(name: &str, console: bool) {
    let path = log_file(name);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // Keep one old log: a session's evidence survives the next start.
    if std::fs::metadata(&path).map(|m| m.len() > 2_000_000).unwrap_or(false) {
        let _ = std::fs::rename(&path, path.with_extension("old.log"));
    }
    let file = std::fs::OpenOptions::new().create(true).append(true).open(&path);
    let Ok(mut file) = file else {
        return;
    };
    if let Ok(mut p) = PATH.lock() {
        *p = Some(path.clone());
    }

    // SAFETY: standard handle plumbing; every handle is checked.
    unsafe {
        let orig_out = GetStdHandle(STD_OUTPUT_HANDLE);
        let orig_err = GetStdHandle(STD_ERROR_HANDLE);
        let mut sink: HANDLE = if valid(orig_out) { orig_out } else { null_mut() };
        if console && !valid(orig_out) && AttachConsole(ATTACH_PARENT_PROCESS) != 0 {
            let con = wide("CONOUT$");
            let h = CreateFileW(
                con.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                null(),
                OPEN_EXISTING,
                0,
                null_mut(),
            );
            if valid(h) {
                sink = h;
            }
        }
        let (mut r, mut w): (HANDLE, HANDLE) = (null_mut(), null_mut());
        if CreatePipe(&mut r, &mut w, null(), 1 << 16) == 0 {
            return;
        }
        SetStdHandle(STD_OUTPUT_HANDLE, w);
        SetStdHandle(STD_ERROR_HANDLE, w);
        let sink_addr = sink as usize;
        let read_addr = r as usize;
        let thread = std::thread::Builder::new()
            .name("log".into())
            .spawn(move || {
                let r = read_addr as HANDLE;
                let sink = sink_addr as HANDLE;
                let mut pending: Vec<u8> = Vec::new();
                let mut buf = [0u8; 8192];
                loop {
                    let mut n = 0u32;
                    let ok = ReadFile(r, buf.as_mut_ptr() as *mut _, buf.len() as u32, &mut n, null_mut());
                    if ok == 0 || n == 0 {
                        break;
                    }
                    pending.extend_from_slice(&buf[..n as usize]);
                    while let Some(i) = pending.iter().position(|&b| b == b'\n') {
                        let line: Vec<u8> = pending.drain(..=i).collect();
                        let text = String::from_utf8_lossy(&line);
                        let text = text.trim_end_matches(['\r', '\n']);
                        let stamped = format!("{} {}\r\n", stamp(), text);
                        let _ = file.write_all(stamped.as_bytes());
                        if !sink.is_null() {
                            write_handle(sink, format!("{text}\r\n").as_bytes());
                        }
                    }
                    let _ = file.flush();
                }
                if !pending.is_empty() {
                    let _ = writeln!(file, "{} {}", stamp(), String::from_utf8_lossy(&pending));
                }
                let _ = file.flush();
                CloseHandle(r);
            })
            .ok();
        if let Ok(mut l) = LIVE.lock() {
            *l = Some(Live {
                write: w,
                orig_out,
                orig_err,
                thread,
            });
        }
    }

    // A panic writes straight to the file (the pipe thread may not get to run
    // before the abort).
    std::panic::set_hook(Box::new(|info| {
        let bt = std::backtrace::Backtrace::force_capture();
        let msg = format!("{} PANIC: {info}\r\n{bt}\r\n", stamp());
        if let Ok(p) = PATH.lock() {
            if let Some(p) = p.as_ref() {
                if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
                    let _ = f.write_all(msg.as_bytes());
                }
            }
        }
    }));
}

/// Write the banner every log starts a session with.
pub fn banner(what: &str) {
    println!("==== {what} {} starting ====", env!("CARGO_PKG_VERSION"));
    println!(
        "exe: {}",
        std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default()
    );
    println!("args: {:?}", std::env::args().skip(1).collect::<Vec<_>>());
    println!("pid: {}", std::process::id());
}

/// Flush and stop the logging thread (call before exiting).
pub fn finish() {
    let live = LIVE.lock().ok().and_then(|mut l| l.take());
    if let Some(mut l) = live {
        // SAFETY: restoring the handles saved in `init`, then closing the pipe
        // so the reader sees end-of-file.
        unsafe {
            SetStdHandle(STD_OUTPUT_HANDLE, l.orig_out);
            SetStdHandle(STD_ERROR_HANDLE, l.orig_err);
            CloseHandle(l.write);
        }
        if let Some(t) = l.thread.take() {
            let _ = t.join();
        }
    }
}
