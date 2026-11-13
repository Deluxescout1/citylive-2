//! Small Win32 helpers shared by the Windows backend.

use windows_sys::Win32::Foundation::{GetLastError, HWND, RECT};
use windows_sys::Win32::UI::WindowsAndMessaging::{GetClassNameW, GetWindowRect};

/// A NUL-terminated UTF-16 string for the wide Win32 calls.
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// A UTF-16 buffer (up to the first NUL) as a `String`.
pub fn from_wide(buf: &[u16]) -> String {
    let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..n])
}

pub fn last_error() -> u32 {
    // SAFETY: no arguments, reads the thread's last-error value.
    unsafe { GetLastError() }
}

pub fn class_name(hwnd: HWND) -> String {
    let mut buf = [0u16; 256];
    // SAFETY: the buffer length is passed as given.
    let n = unsafe { GetClassNameW(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
    if n <= 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buf[..n as usize])
}

pub fn window_rect(hwnd: HWND) -> Option<RECT> {
    let mut r = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    // SAFETY: `r` is a valid out-parameter.
    (unsafe { GetWindowRect(hwnd, &mut r) } != 0).then_some(r)
}

pub fn fmt_rect(r: &RECT) -> String {
    format!(
        "({},{}) {}x{}",
        r.left,
        r.top,
        r.right - r.left,
        r.bottom - r.top
    )
}

/// A window handle as a short hex string for the log.
pub fn hex(h: HWND) -> String {
    format!("0x{:X}", h as usize)
}
