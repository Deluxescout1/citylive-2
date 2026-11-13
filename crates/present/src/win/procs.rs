//! Other programs on the desktop: who owns a window, which other wallpaper
//! programs are running, and (only on the user's click in the settings app)
//! replacing the old Electron CityLive.

use std::path::PathBuf;
use std::ptr::null_mut;

use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegDeleteValueW, RegEnumValueW, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER, KEY_READ,
    KEY_SET_VALUE,
};
use windows_sys::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, TerminateProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    PROCESS_TERMINATE,
};

use super::util::{from_wide, wide};
use crate::wincore::{other_wallpaper_app, OtherApp};

/// The full path of a process's executable, if it can be read.
pub fn exe_path(pid: u32) -> Option<String> {
    // SAFETY: the handle is closed again; the buffer length is passed.
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return None;
        }
        let mut buf = vec![0u16; 1024];
        let mut n = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, 0, buf.as_mut_ptr(), &mut n);
        CloseHandle(h);
        (ok != 0).then(|| String::from_utf16_lossy(&buf[..n as usize]))
    }
}

/// `name.exe` from a full path.
pub fn file_name(path: &str) -> String {
    path.rsplit(['\\', '/']).next().unwrap_or(path).to_string()
}

/// "exe name (pid N)" for the log.
pub fn describe_pid(pid: u32) -> String {
    match exe_path(pid) {
        Some(p) => format!("{} (pid {pid})", file_name(&p)),
        None => format!("pid {pid}"),
    }
}

/// A running program that is a wallpaper of another make.
#[derive(Clone, Debug)]
pub struct Other {
    pub kind: OtherApp,
    pub label: &'static str,
    pub pid: u32,
    pub path: String,
}

fn all_pids() -> Vec<u32> {
    let mut out = Vec::new();
    // SAFETY: standard toolhelp walk; the snapshot handle is closed.
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return out;
        }
        let mut e: PROCESSENTRY32W = std::mem::zeroed();
        e.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut ok = Process32FirstW(snap, &mut e);
        while ok != 0 {
            out.push(e.th32ProcessID);
            ok = Process32NextW(snap, &mut e);
        }
        CloseHandle(snap);
    }
    out
}

/// Every other wallpaper program that is running now.
pub fn scan_other_wallpapers() -> Vec<Other> {
    let me = std::process::id();
    let mut out = Vec::new();
    for pid in all_pids() {
        if pid == me {
            continue;
        }
        let Some(path) = exe_path(pid) else { continue };
        if let Some((kind, label)) = other_wallpaper_app(&file_name(&path), &path) {
            out.push(Other { kind, label, pid, path });
        }
    }
    out
}

/// Is this process one of those other wallpaper programs (so a window of it
/// must not count as a full-screen application)?
pub fn is_other_wallpaper(pid: u32) -> bool {
    exe_path(pid)
        .map(|p| other_wallpaper_app(&file_name(&p), &p).is_some())
        .unwrap_or(false)
}

/// Stop the old CityLive (every process of its install) and remove its start-up
/// entries from the per-user Run key and the Startup folder. Called only from a
/// click in the settings app. Returns what it did, line by line.
pub fn replace_old_citylive() -> Vec<String> {
    let mut log = Vec::new();
    for o in scan_other_wallpapers().into_iter().filter(|o| o.kind == OtherApp::OldCityLive) {
        // SAFETY: the handle is closed again.
        let ok = unsafe {
            let h = OpenProcess(PROCESS_TERMINATE, 0, o.pid);
            if h.is_null() {
                false
            } else {
                let r = TerminateProcess(h, 0) != 0;
                CloseHandle(h);
                r
            }
        };
        log.push(format!("stop pid {} ({}): {}", o.pid, o.path, if ok { "stopped" } else { "failed" }));
    }
    // The Run key: any value that starts the old program.
    let sub = wide("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
    let mut key: HKEY = null_mut();
    // SAFETY: standard registry enumeration; names are collected first, then deleted.
    unsafe {
        if RegOpenKeyExW(HKEY_CURRENT_USER, sub.as_ptr(), 0, KEY_READ | KEY_SET_VALUE, &mut key) == 0 {
            let mut doomed: Vec<Vec<u16>> = Vec::new();
            let mut i = 0u32;
            loop {
                let mut name = vec![0u16; 256];
                let mut nlen = name.len() as u32;
                let mut data = vec![0u8; 2048];
                let mut dlen = data.len() as u32;
                let mut ty = 0u32;
                let rc = RegEnumValueW(
                    key,
                    i,
                    name.as_mut_ptr(),
                    &mut nlen,
                    null_mut(),
                    &mut ty,
                    data.as_mut_ptr(),
                    &mut dlen,
                );
                if rc != 0 {
                    break;
                }
                i += 1;
                let value: Vec<u16> = data[..dlen as usize]
                    .chunks_exact(2)
                    .map(|c| u16::from_le_bytes([c[0], c[1]]))
                    .collect();
                let text = from_wide(&value).to_ascii_lowercase().replace('/', "\\");
                if text.contains("\\programs\\citylive\\citylive.exe") {
                    log.push(format!("start-up entry removed: {}", from_wide(&name)));
                    name.truncate(nlen as usize);
                    name.push(0);
                    doomed.push(name);
                }
            }
            for n in doomed {
                RegDeleteValueW(key, n.as_ptr());
            }
            RegCloseKey(key);
        }
    }
    // The Startup folder: a shortcut called CityLive*.lnk (ours is "Citylive 2").
    if let Some(appdata) = std::env::var_os("APPDATA") {
        let dir = PathBuf::from(appdata).join("Microsoft\\Windows\\Start Menu\\Programs\\Startup");
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                let n = e.file_name().to_string_lossy().to_ascii_lowercase();
                if n.starts_with("citylive") && !n.starts_with("citylive 2") && n.ends_with(".lnk") {
                    let ok = std::fs::remove_file(e.path()).is_ok();
                    log.push(format!("startup shortcut {}: {}", e.path().display(), if ok { "removed" } else { "failed" }));
                }
            }
        }
    }
    if log.is_empty() {
        log.push("the old CityLive was not running and has no start-up entry".to_string());
    }
    log
}
