//! Start with Windows: the per-user `Run` key. No admin rights, and the user
//! can see and remove it in Task Manager > Startup apps.

use std::path::PathBuf;
use std::ptr::{null, null_mut};

use windows_sys::Win32::System::Registry::{
    RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ,
};

use super::util::{from_wide, wide};

const SUBKEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
// Not "CityLive": the old Electron app used that name for its own entry.
const VALUE: &str = "Citylive 2";

/// The wallpaper program: the `citylive.exe` next to this one (the settings app
/// calls this too), or this program itself.
pub fn wallpaper_exe() -> Option<PathBuf> {
    let me = std::env::current_exe().ok()?;
    let dir = me.parent()?;
    for name in ["citylive.exe", "citylive-iso.exe"] {
        let p = dir.join(name);
        if p.exists() {
            return Some(p);
        }
    }
    Some(me)
}

/// The value as stored: the quoted path (spaces in `C:\Users\Some One\...`).
fn command() -> Option<String> {
    wallpaper_exe().map(|p| format!("\"{}\"", p.display()))
}

/// The command currently registered, if any.
pub fn registered() -> Option<String> {
    let sub = wide(SUBKEY);
    let val = wide(VALUE);
    let mut buf = vec![0u16; 1024];
    let mut size = (buf.len() * 2) as u32;
    // SAFETY: `size` is the buffer's byte length.
    let rc = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            sub.as_ptr(),
            val.as_ptr(),
            RRF_RT_REG_SZ,
            null_mut(),
            buf.as_mut_ptr() as *mut _,
            &mut size,
        )
    };
    (rc == 0).then(|| from_wide(&buf))
}

/// Is the wallpaper registered to start with Windows?
pub fn is_enabled() -> bool {
    registered().is_some()
}

/// Turn start-with-Windows on or off.
pub fn set(enabled: bool) -> Result<(), String> {
    let sub = wide(SUBKEY);
    let val = wide(VALUE);
    if enabled {
        let cmd = command().ok_or("cannot find the program's own path")?;
        let data = wide(&cmd);
        // SAFETY: `data` is NUL-terminated and its byte length is passed.
        let rc = unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                sub.as_ptr(),
                val.as_ptr(),
                REG_SZ,
                data.as_ptr() as *const _,
                (data.len() * 2) as u32,
            )
        };
        if rc != 0 {
            return Err(format!("RegSetKeyValueW failed ({rc})"));
        }
        Ok(())
    } else {
        // SAFETY: plain call; ERROR_FILE_NOT_FOUND (2) just means "already off".
        let rc = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, sub.as_ptr(), val.as_ptr()) };
        if rc != 0 && rc != 2 {
            return Err(format!("RegDeleteKeyValueW failed ({rc})"));
        }
        let _ = null::<u8>();
        Ok(())
    }
}
