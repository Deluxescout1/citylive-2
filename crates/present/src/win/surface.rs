//! One monitor's wallpaper surface: a child window of the desktop and the DIB
//! the city is sliced into.
//!
//! The DIB section is the canvas, like the shm buffer on Wayland: the slice is
//! written straight into its memory, and only the damaged rectangles are copied
//! to the window with `BitBlt` (the DIB is 32-bit BGRX, which is exactly the
//! `0xAARRGGBB` word the framebuffer already holds, so there is no conversion).
//! `WM_PAINT` copies the whole thing back when Windows asks for a repaint.

use std::ptr::{null, null_mut};
use std::sync::Once;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, EndPaint,
    GdiFlush, GetDC, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    DIB_RGB_COLORS, HBITMAP, HDC, HGDIOBJ, MapWindowPoints, PAINTSTRUCT, SRCCOPY,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetParent, GetWindowLongPtrW, IsWindow,
    RegisterClassExW, SetParent, SetWindowLongPtrW, SetWindowPos, ShowWindow, GWL_EXSTYLE, GWL_STYLE,
    GWLP_USERDATA, HTTRANSPARENT, LWA_ALPHA, SetLayeredWindowAttributes, WS_EX_LAYERED, HWND_BOTTOM, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOZORDER,
    SWP_SHOWWINDOW, SW_SHOWNOACTIVATE, WM_ERASEBKGND, WM_NCHITTEST, WM_PAINT,
    WNDCLASSEXW, WS_CHILD, WS_CLIPSIBLINGS, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_POPUP,
    WS_VISIBLE,
};

use super::util::{last_error, wide};

const CLASS: &str = "CityLiveWallpaper";

/// How a surface is attached to the desktop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    /// A child of this window (the WorkerW behind the icons, or Progman).
    Child(HWND),
    /// A top-level window pushed to the bottom of the z-order (only a
    /// fallback: it covers the desktop icons).
    TopLevelBottom,
}

struct Inner {
    hwnd: HWND,
    memdc: HDC,
    bmp: HBITMAP,
    old: HGDIOBJ,
    bits: *mut u32,
    w: i32,
    h: i32,
    /// Headless surfaces (the self-test) keep their pixels here and have no
    /// window or GDI objects.
    mem: Vec<u32>,
}

/// A monitor's window and its pixel canvas.
pub struct Surface {
    inner: Box<Inner>,
}

fn register_class() -> bool {
    static ONCE: Once = Once::new();
    static mut OK: bool = false;
    ONCE.call_once(|| {
        let name = wide(CLASS);
        // SAFETY: a plain window class with our own window procedure.
        unsafe {
            let wc = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                style: 0,
                lpfnWndProc: Some(proc),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: GetModuleHandleW(null()),
                hIcon: null_mut(),
                hCursor: null_mut(),
                hbrBackground: null_mut(),
                lpszMenuName: null(),
                lpszClassName: name.as_ptr(),
                hIconSm: null_mut(),
            };
            OK = RegisterClassExW(&wc) != 0;
            if !OK {
                eprintln!("win: RegisterClassExW({CLASS}) failed: {}", last_error());
            }
        }
    });
    // SAFETY: written once inside `call_once`, read after it.
    unsafe { OK }
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_ERASEBKGND => 1,
        // Never take the mouse: clicks belong to the desktop and its icons.
        WM_NCHITTEST => HTTRANSPARENT as LRESULT,
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = unsafe { std::mem::zeroed() };
            let hdc = unsafe { BeginPaint(hwnd, &mut ps) };
            let p = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *const Inner;
            if !p.is_null() && !hdc.is_null() {
                let inner = unsafe { &*p };
                let r = ps.rcPaint;
                let (w, h) = (r.right - r.left, r.bottom - r.top);
                if w > 0 && h > 0 {
                    unsafe {
                        BitBlt(hdc, r.left, r.top, w, h, inner.memdc, r.left, r.top, SRCCOPY);
                    }
                }
            }
            unsafe { EndPaint(hwnd, &ps) };
            0
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}

impl Surface {
    /// Create a surface for the monitor rectangle `(x, y, w, h)` given in
    /// virtual-screen pixels. Returns the reason on failure.
    pub fn create(placement: Placement, x: i32, y: i32, w: i32, h: i32) -> Result<Surface, String> {
        if w <= 0 || h <= 0 {
            return Err(format!("bad size {w}x{h}"));
        }
        if !register_class() {
            return Err("window class registration failed".into());
        }
        // SAFETY: standard GDI / window creation; every handle is checked and
        // released again on the failure paths and in Drop.
        unsafe {
            let screen = GetDC(null_mut());
            let memdc = CreateCompatibleDC(screen);
            ReleaseDC(null_mut(), screen);
            if memdc.is_null() {
                return Err(format!("CreateCompatibleDC failed ({})", last_error()));
            }
            let mut bmi: BITMAPINFO = std::mem::zeroed();
            bmi.bmiHeader = BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h, // top-down, row 0 first, like the framebuffer
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
            let bmp = CreateDIBSection(memdc, &bmi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
            if bmp.is_null() || bits.is_null() {
                let e = last_error();
                DeleteDC(memdc);
                return Err(format!("CreateDIBSection {w}x{h} failed ({e})"));
            }
            let old = SelectObject(memdc, bmp as HGDIOBJ);

            let class = wide(CLASS);
            let title = wide("CityLive wallpaper");
            let hinst = GetModuleHandleW(null());
            let make = |style: u32, ex: u32, parent: HWND, px: i32, py: i32| {
                CreateWindowExW(
                    ex,
                    class.as_ptr(),
                    title.as_ptr(),
                    style,
                    px,
                    py,
                    w,
                    h,
                    parent,
                    null_mut(),
                    hinst,
                    null(),
                )
            };
            let hwnd = match placement {
                Placement::Child(p) => {
                    // The way wallpaper programs have always done it: a hidden
                    // top-level window, re-parented to the desktop layer, then
                    // turned into a child. (Creating the child directly in
                    // Explorer's window can be refused.)
                    let mut pt = POINT { x, y };
                    MapWindowPoints(null_mut(), p, &mut pt, 1);
                    let top = make(WS_POPUP, WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE, null_mut(), x, y);
                    let mut hwnd = null_mut();
                    if !top.is_null() {
                        let _ = SetParent(top, p);
                        let as_child = (WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS) as isize;
                        SetWindowLongPtrW(top, GWL_STYLE, as_child);
                        // Windows 11 24H2 composes Progman through DirectComposition:
                        // a child only shows if it is a layered window (fully opaque).
                        SetWindowLongPtrW(top, GWL_EXSTYLE, WS_EX_LAYERED as isize);
                        SetLayeredWindowAttributes(top, 0, 255, LWA_ALPHA);
                        if GetParent(top) == p {
                            hwnd = top;
                        } else {
                            eprintln!(
                                "win: SetParent did not take ({}); trying a direct child",
                                last_error()
                            );
                            DestroyWindow(top);
                        }
                    }
                    if hwnd.is_null() {
                        hwnd = make(WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS, WS_EX_LAYERED, p, pt.x, pt.y);
                        if !hwnd.is_null() {
                            SetLayeredWindowAttributes(hwnd, 0, 255, LWA_ALPHA);
                        }
                    }
                    if !hwnd.is_null() {
                        SetWindowPos(
                            hwnd,
                            null_mut(),
                            pt.x,
                            pt.y,
                            w,
                            h,
                            SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED | SWP_SHOWWINDOW,
                        );
                    }
                    hwnd
                }
                Placement::TopLevelBottom => make(
                    WS_POPUP | WS_VISIBLE,
                    WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                    null_mut(),
                    x,
                    y,
                ),
            };
            if hwnd.is_null() {
                let e = last_error();
                SelectObject(memdc, old);
                DeleteObject(bmp as HGDIOBJ);
                DeleteDC(memdc);
                return Err(format!("CreateWindowExW failed ({e}) placement={placement:?} at ({x},{y}) {w}x{h}"));
            }
            let mut inner = Box::new(Inner {
                hwnd,
                memdc,
                bmp,
                old,
                bits: bits as *mut u32,
                w,
                h,
                mem: Vec::new(),
            });
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, &mut *inner as *mut Inner as isize);
            if placement == Placement::TopLevelBottom {
                SetWindowPos(hwnd, HWND_BOTTOM, x, y, w, h, SWP_NOACTIVATE | SWP_SHOWWINDOW);
            } else {
                ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            }
            Ok(Surface { inner })
        }
    }

    /// A surface with no window: the pixels live in memory and presenting is a
    /// no-op. The self-test uses it to run the real slice without a desktop.
    pub fn headless(w: i32, h: i32) -> Surface {
        let mut mem = vec![0u32; (w.max(1) * h.max(1)) as usize];
        Surface {
            inner: Box::new(Inner {
                hwnd: null_mut(),
                memdc: null_mut(),
                bmp: null_mut(),
                old: null_mut(),
                bits: mem.as_mut_ptr(),
                w,
                h,
                mem,
            }),
        }
    }

    pub fn hwnd(&self) -> HWND {
        self.inner.hwnd
    }

    pub fn size(&self) -> (i32, i32) {
        (self.inner.w, self.inner.h)
    }

    /// Is the window still there (Explorer restarting destroys it)?
    pub fn alive(&self) -> bool {
        if self.inner.hwnd.is_null() {
            return true; // headless
        }
        // SAFETY: IsWindow accepts any handle value.
        unsafe { IsWindow(self.inner.hwnd) != 0 }
    }

    /// Put the window just below `sibling` in its parent's z-order (so the
    /// desktop icons stay above it).
    pub fn place_below(&self, sibling: HWND) {
        if self.inner.hwnd.is_null() {
            return;
        }
        // SAFETY: both handles are windows; the flags keep it from activating.
        unsafe {
            SetWindowPos(
                self.inner.hwnd,
                sibling,
                0,
                0,
                0,
                0,
                windows_sys::Win32::UI::WindowsAndMessaging::SWP_NOMOVE
                    | windows_sys::Win32::UI::WindowsAndMessaging::SWP_NOSIZE
                    | SWP_NOACTIVATE,
            );
        }
    }

    /// The canvas, `w * h` pixels, row 0 first.
    pub fn pixels_mut(&mut self) -> &mut [u32] {
        let n = self.inner.w as usize * self.inner.h as usize;
        // SAFETY: the DIB section owns exactly `w * h` 32-bit pixels and lives
        // as long as `self`.
        unsafe { std::slice::from_raw_parts_mut(self.inner.bits, n) }
    }

    /// Copy the whole canvas to the window.
    pub fn present_all(&mut self) {
        let (w, h) = (self.inner.w, self.inner.h);
        self.blit(&[(0, 0, w, h)]);
    }

    /// Copy only these rectangles (canvas pixels) to the window.
    pub fn present_rects(&mut self, rects: &[render::Rect]) {
        let (w, h) = (self.inner.w, self.inner.h);
        let list: Vec<(i32, i32, i32, i32)> = rects
            .iter()
            .filter_map(|r| {
                let (x0, y0) = (r.x0.max(0), r.y0.max(0));
                let (x1, y1) = (r.x1.min(w), r.y1.min(h));
                (x1 > x0 && y1 > y0).then_some((x0, y0, x1 - x0, y1 - y0))
            })
            .collect();
        self.blit(&list);
    }

    fn blit(&mut self, rects: &[(i32, i32, i32, i32)]) {
        if rects.is_empty() || self.inner.hwnd.is_null() {
            return;
        }
        // SAFETY: the window DC is released before returning; GdiFlush makes
        // sure the batched BitBlts have read the DIB before we write the next
        // frame into it.
        unsafe {
            let dc = GetDC(self.inner.hwnd);
            if dc.is_null() {
                return;
            }
            for &(x, y, w, h) in rects {
                BitBlt(dc, x, y, w, h, self.inner.memdc, x, y, SRCCOPY);
            }
            ReleaseDC(self.inner.hwnd, dc);
            GdiFlush();
        }
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        if self.inner.hwnd.is_null() {
            return; // headless: nothing but the Vec
        }
        // SAFETY: tearing down what `create` made, in reverse order.
        unsafe {
            SetWindowLongPtrW(self.inner.hwnd, GWLP_USERDATA, 0);
            DestroyWindow(self.inner.hwnd);
            SelectObject(self.inner.memdc, self.inner.old);
            DeleteObject(self.inner.bmp as HGDIOBJ);
            DeleteDC(self.inner.memdc);
        }
    }
}
