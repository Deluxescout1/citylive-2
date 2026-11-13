//! B63 — one shared-memory frame per head for the Plasma wallpaper plugin (D71).
//!
//! **Why not PPM files.** `--export-frames` (B42e) writes a base PPM plus
//! 128-row strips into a directory and lets QML show them with `FolderListModel`
//! + `Image`. Measured on the owner's desk at 30 fps that is 350–420 MB/s of PPM
//! written, 180 MB/s read back by plasmashell (33 % of a core), ~66 lost strips
//! a minute (the writer deletes a strip before plasmashell opens it, so the
//! older base shows through = flicker), and the frame is *upscaled on the CPU*
//! to screen resolution before it is encoded, then decoded again in Qt. This
//! module replaces all of that with one mmapped file per head that holds the
//! frame at its **internal** resolution, double-buffered, published with a
//! seqlock. No file is ever deleted while running, so the race cannot happen;
//! the reader scales with the GPU, so the CPU upscale and the PPM encode/decode
//! both disappear; only the rows a frame actually changed are copied, so an idle
//! desktop copies nothing.
//!
//! # File layout (`<shm-dir>/<head-name>.frame`)
//!
//! ```text
//! 0     magic           [u8; 8]   "CLISOFR1"
//! 8     version         u32       4 (3 = the row-table layout, still read)
//! 12    header_bytes    u32       53248 (v3: 4096)
//! 16    width           u32       internal pixels (the head's slice)
//! 20    height          u32
//! 24    stride          u32       bytes per pixel row = width * 4
//! 28    scale           u32       integer upscale the screen applies (D61)
//! 32    screen_w        u32       the head's size in Qt logical pixels (the
//!                                 item stretches the slice to its own size)
//! 36    screen_h        u32
//! 40    fps             u32       the rate the writer publishes at
//! 44    buffers         u32       1 (B61: one buffer, seqlock retry)
//! 48    seq             u64       published frame number (0 = nothing yet)
//! 56    published       u32       which buffer holds frame `seq`
//! 60    reserved        u32       0
//! 64    buf[0].offset   u64       file offset of buffer 0 (page aligned)
//! 72    buf[0].seq      u64       2*frame while complete, 2*frame+1 while writing
//! 80    buf[0].bytes    u32       stride * height
//! 84    pad             u32       0
//! 88    prev_seq        u64       v4: `seq` of the publish before this one
//!                                 (0 for the first); inside the seqlock window
//! 96    buf[1]          --        reserved (was the second buffer; zeroed)
//! 120   pad             u64
//! 128   damage[0].count u32       rows this buffer wrote, 0xFFFF = the whole head
//! 132   damage[0].pad   u32
//! 136   damage[0].rows[24]  { y0: u32, y1: u32 }   y0..y1, half open
//! 328   damage[1].count u32
//! 332   damage[1].pad   u32
//! 336   damage[1].rows[24]
//! 576   rects.count     u32       v4: rectangles this publish changed;
//!                                 0xFFFFFFFF = the whole head
//! 580   rects.pad       u32
//! 584   rects.rect[3072]  { x0, y0, x1, y1: u32 }   half open, head pixels
//! 49736 (end of the table)
//! 53248 buffer 0        stride * height bytes, XRGB8888 / BGRA little-endian
//!                       (v3 files: 4096)
//! ```
//!
//! All integers are little-endian host order (this is a same-machine transport;
//! the C++ side static_asserts the offsets). Pixels are `0x00RRGGBB` words,
//! exactly what the renderer keeps in its framebuffer and what Qt's
//! `Format_RGB32` reads, so neither side converts anything.
//!
//! # Seqlock protocol
//!
//! Writer, frame `n`:
//! 1. (B61: there is one buffer, so it is always `buf[0]`;)
//! 2. store that buffer's `seq = 2n+1` (odd = being written);
//! 3. copy this frame's changed rows into it;
//! 4. store the buffer's "damaged rows" table (part of the header the reader
//!    copies — see below), then the buffer's `seq = 2n` (Release);
//! 5. store `published = <buffer>` then `seq = n` (Release) — this is the
//!    publish. No file is created, deleted or renamed at this point.
//!
//! Reader:
//! 1. load `seq` (Acquire); equal to the last frame it showed → nothing to do;
//! 2. read `published`; load that buffer's `seq` and require it to be exactly
//!    `2 * seq` — an odd value or a stale one means the writer is inside it, so
//!    keep the previous frame;
//! 3. copy the rows the header's damage table lists for that buffer (a full
//!    copy when it says 0xFFFF, or when the table is unusable);
//! 4. load that buffer's `seq` again; if it changed, the copy raced a writer and
//!    is torn: throw it away and keep the previous frame.
//!
//! Damaged rows reach the reader in a per-buffer table (B61): up to
//! [`MAX_DAMAGE_SPANS`] half-open row ranges, or `DAMAGE_FULL`, which means "the
//! whole head changed". The reader keeps its image between frames and copies
//! only those rows, so the most expensive part of the reader — the per-frame
//! copy of a 14.7 MB head — scales with the damage instead of the frame.
//!
//! Damage (v4, B86). A publish names **rectangles** (not rows: a downpour
//! touches every row and a small part of every row, so row copies moved
//! ~425 MB/s per head and rectangles move ~20). The table holds this publish's
//! changes against the buffer as the writer found it, i.e. against publish
//! `prev_seq`: the scene's damaged rectangles, the pixels the weather overlay
//! painted last time (put back now) and the pixels it paints now. More than
//! `MAX_DAMAGE_RECTS` are merged (`coalesce`); a table that would cover more than
//! half the head, or a full recomposite, is `RECTS_FULL`.
//!
//! There is **no history** in the table. The reader copies the table only when
//! its image is exactly one publish behind (`prev_seq == the last seq it took`);
//! otherwise (it skipped a publish, dropped a torn one, was just opened) it copies
//! the whole head. A reader that is always caught up needs only this publish's
//! damage, and one that is not never trusts the table.
//!
//! The v3 row table at 128 is still filled (this publish's rows, no history) and a
//! reader still accepts version 3 (rows plus the `seq != seen + 1` skip rule), so
//! a newer reader does not break on an older wallpaper.
//!
//! A size change (a head resized) recreates the file under `.<name>.frame.new`
//! and renames it over the old one; readers notice the inode changed and re-map.
//! File names are never deleted while the wallpaper runs.

use render::Rect;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

/// `CLISOFR1`: CityLive Iso FRame, version 1.
pub const MAGIC: [u8; 8] = *b"CLISOFR1";
/// Bumped only when the layout below changes incompatibly.
///
/// 2 (B61): the per-buffer damaged-row table at `OFF_DAMAGE`.
/// 3 (B61): one buffer instead of two (`buffers` is validated on both sides).
/// 4 (B86): the rectangle damage table at `OFF_RECTS` and `prev_seq` at
/// `OFF_PREV_SEQ`. Every v3 field keeps its offset, and the writer still fills the
/// v3 row table (for this frame only), so a reader may take either.
pub const VERSION: u32 = 4;
/// The previous layout. Readers still accept it (rows + the `seq != seen + 1`
/// skip rule), so a reader built from this source does not break on a file an
/// older wallpaper wrote.
pub const VERSION_V3: u32 = 3;
/// Room for the rectangle table, every buffer page-aligned (B86: was one page).
pub const HEADER_BYTES: usize = 53248;
/// One buffer (B61, D14's memory ceiling): the writer never writes a buffer a
/// reader can accept, because it marks the buffer's `seq` odd (being written)
/// before it touches a pixel and the reader requires the exact `2 * n` value.
/// A reader that arrives mid-copy throws the copy away and takes the next
/// frame, so the cost of one buffer is an occasional skipped frame, not a torn
/// picture — and it halves the shared memory (37 MB, not 75 MB, on three
/// heads). The damaged-row table is simpler too: the buffer holds frame `n-1`,
/// so it only has to carry this frame's rows.
pub const BUFFERS: usize = 1;

// Field offsets. The C++ reader (`plasma/frames/cityframe.h`) static_asserts the
// same numbers, so a mismatch is a compile error on the desktop rather than a
// corrupted picture.
pub const OFF_MAGIC: usize = 0;
pub const OFF_VERSION: usize = 8;
pub const OFF_HEADER_BYTES: usize = 12;
pub const OFF_WIDTH: usize = 16;
pub const OFF_HEIGHT: usize = 20;
pub const OFF_STRIDE: usize = 24;
pub const OFF_SCALE: usize = 28;
pub const OFF_SCREEN_W: usize = 32;
pub const OFF_SCREEN_H: usize = 36;
pub const OFF_FPS: usize = 40;
pub const OFF_BUFFERS: usize = 44;
pub const OFF_SEQ: usize = 48;
pub const OFF_PUBLISHED: usize = 56;
pub const OFF_BUF0: usize = 64;
/// Size of one buffer-table entry: offset, seq, bytes, pad.
pub const BUF_ENTRY: usize = 32;
/// First byte of the per-buffer damaged-row tables (B61).
pub const OFF_DAMAGE: usize = 128;
/// Row ranges a damage table can hold; more means "the whole head".
pub const MAX_DAMAGE_SPANS: usize = 24;
/// `damage[i].count` when the whole head was written.
pub const DAMAGE_FULL: u32 = 0xFFFF;

/// B86: `prev_seq` — the `seq` of the publish before this one (0 for the first).
/// It lives in the padding of the buffer entry (64 + 24).
pub const OFF_PREV_SEQ: usize = 88;
/// B86: first byte of the rectangle damage table. After the two v3 row tables
/// (which end at 528), on a cache line.
pub const OFF_RECTS: usize = 576;
/// Rectangles a publish can name; more are merged (see [`coalesce`]).
///
/// Why 3072: a downpour on one 2560-wide head is ~1000 (rain) to ~1900 (storm)
/// rain-streak boxes a frame, and the boxes the previous frame painted are
/// restored in the same frame, so twice that is the raw list. The copy budget
/// (about 1 MB a frame) is only met by naming drops, not bands, so the table
/// has to hold on the order of a couple of thousand rectangles; merging only
/// trims it. 16 bytes each is 48 KB of header, written once per frame.
pub const MAX_DAMAGE_RECTS: usize = 3072;
/// What one more rectangle costs the reader, in pixels of copy: a separate
/// `memcpy` loop and a separate upload entry. Merging two boxes is worth it when
/// the bounding box adds less than this.
pub const RECT_COST_PX: i64 = 64;
/// B86: the export asks the weather for rain streaks cut into slices of this many
/// rows (`Fx::overlay_rects_fine`), so a leaning one-pixel streak is not boxed as
/// a big mostly empty rectangle.
pub const STREAK_SLICE_ROWS: i32 = 32;
/// One rectangle in the table: `x0, y0, x1, y1`, half open, head pixels, u32.
pub const RECT_ENTRY: usize = 16;
/// `rects.count` when the whole head changed.
pub const RECTS_FULL: u32 = 0xFFFF_FFFF;
/// More than this share (percent) of the head named by rectangles is a full copy:
/// per-rectangle bookkeeping stops paying for itself.
pub const FULL_PERCENT: i64 = 50;
/// Bytes one buffer's damage entry takes: count, pad, then the rows.
pub const DAMAGE_ENTRY: usize = 8 + MAX_DAMAGE_SPANS * 8;

/// `damage[i].count`.
fn off_damage_count(i: usize) -> usize {
    OFF_DAMAGE + i * DAMAGE_ENTRY
}

/// `damage[i].rows[k].y0`.
fn off_damage_row(i: usize, k: usize) -> usize {
    OFF_DAMAGE + i * DAMAGE_ENTRY + 8 + k * 8
}

/// `rects.rect[k].x0`.
fn off_rect(k: usize) -> usize {
    OFF_RECTS + 8 + k * RECT_ENTRY
}

/// `buf[i]`'s `seq` field.
fn off_buf_seq(i: usize) -> usize {
    OFF_BUF0 + i * BUF_ENTRY + 8
}

/// `buf[i]`'s `bytes` field.
fn off_buf_bytes(i: usize) -> usize {
    OFF_BUF0 + i * BUF_ENTRY + 16
}

/// `buf[i]`'s `offset` field.
fn off_buf_offset(i: usize) -> usize {
    OFF_BUF0 + i * BUF_ENTRY
}

fn align_up(n: usize, a: usize) -> usize {
    n.div_ceil(a) * a
}

/// The size of the file for a `w`×`h` internal frame: header + two buffers.
pub fn file_size(w: u32, h: u32) -> usize {
    let buf_bytes = align_up(w as usize * 4 * h as usize, 4096);
    HEADER_BYTES + BUFFERS * buf_bytes
}

/// Z-order key of a point, so rectangles that are close in the plane are close in
/// the sorted list.
fn morton(x: u32, y: u32) -> u32 {
    fn spread(v: u32) -> u32 {
        let mut v = v & 0xFFFF;
        v = (v | (v << 8)) & 0x00FF_00FF;
        v = (v | (v << 4)) & 0x0F0F_0F0F;
        v = (v | (v << 2)) & 0x3333_3333;
        (v | (v << 1)) & 0x5555_5555
    }
    spread(x) | (spread(y) << 1)
}

fn area(r: &Rect) -> i64 {
    (r.x1 - r.x0) as i64 * (r.y1 - r.y0) as i64
}

/// The row ranges `rects` touch, merged (the v3 table).
pub fn rows_of(rects: &[Rect]) -> Vec<(usize, usize)> {
    let mut rows: Vec<(usize, usize)> =
        rects.iter().map(|r| (r.y0 as usize, r.y1 as usize)).collect();
    rows.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::with_capacity(rows.len());
    for (a0, b0) in rows {
        match merged.last_mut() {
            Some(last) if a0 <= last.1 => last.1 = last.1.max(b0),
            _ => merged.push((a0, b0)),
        }
    }
    merged
}

/// B86: turn a frame's damage into the rectangle table.
///
/// `damage` may be long, overlapping and in any order (a downpour is a thousand
/// small boxes, twice). Every rectangle must lie inside `0..w` x `0..h`. The
/// result **covers every input** (a reader copying it never misses a pixel it
/// should have) and has at most [`MAX_DAMAGE_RECTS`] entries; `None` means "copy
/// the whole head" (the table would not fit even after merging, or the
/// rectangles cover more than [`FULL_PERCENT`] of the head).
///
/// The merge walks the rectangles in Z-order of their centres and joins a
/// neighbour into the previous rectangle when that costs less than keeping both.
/// A rectangle's cost is its area plus a fixed [`RECT_COST_PX`] (what a separate
/// copy and a separate texture upload cost the reader, in pixels), so two drops a
/// few pixels apart become one box and two far apart stay two. The fixed cost
/// doubles until the table fits.
pub fn coalesce(mut rects: Vec<Rect>, w: i32, h: i32) -> Option<Vec<Rect>> {
    rects.retain(|r| !r.is_empty());
    if rects.is_empty() {
        return Some(rects);
    }
    rects.sort_unstable_by_key(|r| morton(((r.x0 + r.x1) / 2) as u32, ((r.y0 + r.y1) / 2) as u32));
    let total = w as i64 * h as i64;
    let mut fixed = RECT_COST_PX;
    loop {
        let mut out: Vec<Rect> = Vec::with_capacity(rects.len());
        for r in rects.iter() {
            if let Some(last) = out.last_mut() {
                let u = Rect::new(last.x0.min(r.x0), last.y0.min(r.y0), last.x1.max(r.x1), last.y1.max(r.y1));
                if area(&u) <= area(last) + area(r) + fixed {
                    *last = u;
                    continue;
                }
            }
            out.push(*r);
        }
        rects = out;
        if rects.len() <= MAX_DAMAGE_RECTS {
            break;
        }
        fixed *= 2;
        if fixed > total / 8 {
            return None;
        }
    }
    let covered: i64 = rects.iter().map(area).sum();
    if covered * 100 > total * FULL_PERCENT {
        return None;
    }
    Some(rects)
}

// ---------------------------------------------------------------------------
// Atomic access to the mapped header
// ---------------------------------------------------------------------------

/// # Safety
/// `map` must point at a mapped file of at least `off + 8` bytes and the offset
/// must be 8-byte aligned (every field we use is).
unsafe fn load_u64(map: *const u8, off: usize) -> u64 {
    unsafe { AtomicU64::from_ptr(map.add(off) as *mut u64).load(Ordering::Acquire) }
}

unsafe fn store_u64(map: *mut u8, off: usize, v: u64) {
    unsafe { AtomicU64::from_ptr(map.add(off) as *mut u64).store(v, Ordering::Release) }
}

unsafe fn load_u32(map: *const u8, off: usize) -> u32 {
    unsafe { AtomicU32::from_ptr(map.add(off) as *mut u32).load(Ordering::Acquire) }
}

unsafe fn store_u32(map: *mut u8, off: usize, v: u32) {
    unsafe { AtomicU32::from_ptr(map.add(off) as *mut u32).store(v, Ordering::Release) }
}

/// One head's frames. Reused every frame; the mapping never moves while the
/// head's internal size stays the same.
pub struct ShmWriter {
    path: PathBuf,
    map: *mut u8,
    len: usize,
    w: u32,
    h: u32,
    scale: u32,
    screen: (u32, u32),
    fps: u32,
    /// Last published frame number.
    seq: u64,
    /// Which buffer holds `seq`.
    published: u32,
    /// B61b: the pixels the overlay painted into this buffer last time, saved by
    /// rectangle, so the next write can put the scene's values back (no drop
    /// trails). A flat buffer plus the rectangles it came from, in order: a
    /// rectangle copy is a memcpy, where a per-pixel log of a 730k-pixel cloud
    /// shadow cost milliseconds.
    saved: Vec<u32>,
    saved_rects: Vec<Rect>,
    /// Has the buffer ever held a frame? The first publish copies the whole
    /// head (the rest of it is still zeros).
    valid: [bool; BUFFERS],
    buf_off: [usize; BUFFERS],
}

// The mapping is owned by this struct and only touched from the caller's
// thread; marking it `Send` lets it live inside the wallpaper state.
unsafe impl Send for ShmWriter {}

impl ShmWriter {
    /// Create (or recreate) `path` for a `w`×`h` internal frame.
    ///
    /// The file is built under a temporary name and renamed into place, so a
    /// reader either has the old file or a complete new one. The old file is not
    /// deleted: a reader still holding it keeps a valid (stale) mapping until it
    /// notices the inode change.
    pub fn open(
        path: &Path,
        w: u32,
        h: u32,
        scale: u32,
        screen: (u32, u32),
        fps: u32,
    ) -> io::Result<Self> {
        if w == 0 || h == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "a frame needs a non-empty size",
            ));
        }
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let len = file_size(w, h);
        let name = path
            .file_name()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no file name"))?;
        let tmp = path.with_file_name(format!(".{}.new", name.to_string_lossy()));
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&tmp)?;
        file.set_len(len as u64)?;
        let map = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                file.as_raw_fd(),
                0,
            )
        };
        if map == libc::MAP_FAILED {
            let _ = fs::remove_file(&tmp);
            return Err(io::Error::last_os_error());
        }
        let map = map as *mut u8;
        let buf_bytes = align_up(w as usize * 4 * h as usize, 4096);
        // One buffer (B61): every entry is the same page-aligned size, so a
        // second one would sit at HEADER_BYTES + buf_bytes.
        let mut buf_off = [0usize; BUFFERS];
        for (i, off) in buf_off.iter_mut().enumerate() {
            *off = HEADER_BYTES + i * buf_bytes;
        }
        unsafe {
            std::ptr::write_bytes(map, 0, HEADER_BYTES);
            std::ptr::copy_nonoverlapping(MAGIC.as_ptr(), map, MAGIC.len());
            store_u32(map, OFF_VERSION, VERSION);
            store_u32(map, OFF_HEADER_BYTES, HEADER_BYTES as u32);
            store_u32(map, OFF_WIDTH, w);
            store_u32(map, OFF_HEIGHT, h);
            store_u32(map, OFF_STRIDE, w * 4);
            store_u32(map, OFF_SCALE, scale.max(1));
            store_u32(map, OFF_SCREEN_W, screen.0);
            store_u32(map, OFF_SCREEN_H, screen.1);
            store_u32(map, OFF_FPS, fps);
            store_u32(map, OFF_BUFFERS, BUFFERS as u32);
            for (i, off) in buf_off.iter().enumerate() {
                store_u64(map, off_buf_offset(i), *off as u64);
                store_u64(map, off_buf_seq(i), 0);
                store_u32(map, off_buf_bytes(i), buf_bytes as u32);
            }
            store_u32(map, OFF_PUBLISHED, 0);
            store_u64(map, OFF_SEQ, 0);
        }
        fs::rename(&tmp, path)?;
        drop(file); // the mapping outlives the descriptor
        Ok(Self {
            path: path.to_path_buf(),
            map,
            len,
            w,
            h,
            scale: scale.max(1),
            screen,
            fps,
            seq: 0,
            published: 0,
            saved: Vec::new(),
            saved_rects: Vec::new(),
            valid: [false; BUFFERS],
            buf_off,
        })
    }

    /// Is this writer already set up for exactly this frame shape?
    pub fn matches(&self, w: u32, h: u32, scale: u32, screen: (u32, u32)) -> bool {
        self.w == w && self.h == h && self.scale == scale.max(1) && self.screen == screen
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// B61f: the mapped bytes this head's frame occupies (resident once the
    /// first frame is written).
    pub fn bytes(&self) -> usize {
        self.len
    }

    pub fn dims(&self) -> (u32, u32) {
        (self.w, self.h)
    }

    /// B93: does the buffer still hold pixels an overlay painted last time? Until
    /// the next publish puts the scene's own pixels back they stay on screen, so
    /// the caller must publish again even when nothing else changed.
    pub fn overlay_pending(&self) -> bool {
        !self.saved_rects.is_empty()
    }

    /// Last frame number published through this writer.
    pub fn seq(&self) -> u64 {
        self.seq
    }

    /// Keep the advertised rate in step with the config: the reader polls the
    /// header at this rate, so a change to `--fps` (or a live settings change)
    /// must reach it without recreating the file.
    pub fn set_fps(&mut self, fps: u32) {
        if fps != self.fps {
            self.fps = fps;
            unsafe { store_u32(self.map, OFF_FPS, fps) };
        }
    }

    /// Publish frame `seq`: copy the changed rows of `src` (the union
    /// framebuffer, `fb_w` wide) for the head at `crop` into the buffer that is
    /// not being read, then make it the current frame. Returns the bytes copied
    /// (0 when the frame is identical to the previous one, which is what makes
    /// an idle desktop free).
    ///
    /// `rects` are this frame's damaged rectangles **inside the head** (B61b:
    /// rectangles, not rows — a weather frame's drops are everywhere in rows and
    /// nowhere in columns, so a whole-row copy was the expensive part of a rainy
    /// frame). `full` means "the whole head changed". The rows the copy touched
    /// are what the header advertises to the reader, which copies whole rows
    /// into the image it keeps: a rectangle leaves the rest of its rows at the
    /// values they already had, so a row copy is still exactly right.
    pub fn publish(
        &mut self,
        seq: u64,
        src: &[u32],
        fb_w: usize,
        crop: &crate::slice::HeadCrop,
        rects: &[render::Rect],
        full: bool,
    ) -> usize {
        self.publish_with(seq, src, fb_w, crop, rects, full, &[], |_| {})
    }

    /// `publish`, plus an overlay painted into the head after the damaged
    /// rectangles are copied and before the frame is published (B61b: the
    /// weather particles). Every pixel the overlay writes is remembered with the
    /// value it replaced, so the next write into this buffer puts the scene's
    /// pixels back and no drop leaves a trail. The rows the overlay touched are
    /// named to the reader like any other damage, so the picture it shows is the
    /// composed one.
    pub fn publish_with(
        &mut self,
        seq: u64,
        src: &[u32],
        fb_w: usize,
        crop: &crate::slice::HeadCrop,
        rects: &[render::Rect],
        full: bool,
        overlay_rects: &[render::Rect],
        overlay: impl FnOnce(&mut PaintSurface),
    ) -> usize {
        let (w, h) = (self.w as usize, self.h as usize);
        if crop.w != w || crop.h != h {
            return 0; // caller should have re-opened: never write a wrong size
        }
        if crop.x + w > fb_w || (crop.y + h) * fb_w > src.len() {
            return 0; // a crop outside the framebuffer: refuse rather than read out of bounds
        }
        // One buffer (B61): the only buffer is always the target, and it holds
        // frame `prev_seq`, so this frame's damaged pixels are exactly what it
        // is missing.
        let target = 0usize;
        let whole = full || !self.valid[target];
        let head = render::Rect::new(0, 0, w as i32, h as i32);
        let overlay_active = !overlay_rects.is_empty();
        let rects: Vec<render::Rect> = if whole {
            vec![head]
        } else {
            rects
                .iter()
                .filter_map(|r| {
                    let r = r.intersect(&head);
                    (!r.is_empty()).then_some(r)
                })
                .collect()
        };

        let dst = unsafe {
            std::slice::from_raw_parts_mut(self.map.add(self.buf_off[target]) as *mut u32, w * h)
        };
        // Everything that differs between the buffer as it stands (frame
        // `prev_seq`) and the frame being written: this frame's scene rectangles,
        // the places the overlay painted last time (put back now) and the places
        // it paints this time. The reader copies exactly these (B86).
        let mut damage: Vec<render::Rect> =
            Vec::with_capacity(rects.len() + self.saved_rects.len() + overlay_rects.len());
        damage.extend(rects.iter().copied());
        let prev_seq = self.seq;
        unsafe {
            store_u64(self.map, off_buf_seq(target), 2 * seq + 1);
            // Inside the seqlock window: the reader requires the buffer's seq to
            // be 2*seq before and after it has read this.
            store_u64(self.map, OFF_PREV_SEQ, prev_seq);
        }
        // Undo what the overlay painted into this buffer last time it was
        // written, so the buffer holds the scene's own pixels again (frame
        // `n-1`, which is frame `n` wherever nothing changed). The rectangles
        // are restored in the order they were saved.
        {
            let mut pos = 0usize;
            for i in 0..self.saved_rects.len() {
                let r = self.saved_rects[i];
                damage.push(r); // these pixels change back
                for y in r.y0 as usize..r.y1 as usize {
                    let len = (r.x1 - r.x0) as usize;
                    let d = y * w + r.x0 as usize;
                    if pos + len <= self.saved.len() && d + len <= w * h {
                        dst[d..d + len].copy_from_slice(&self.saved[pos..pos + len]);
                    }
                    pos += len;
                }
            }
            self.saved.clear();
            self.saved_rects.clear();
        }
        let mut bytes = 0usize;
        for r in rects.iter() {
            let (x0, x1) = (r.x0 as usize, r.x1 as usize);
            let span = x1 - x0;
            for y in r.y0 as usize..r.y1 as usize {
                let s = (crop.y + y) * fb_w + crop.x + x0;
                let d = y * w + x0;
                dst[d..d + span].copy_from_slice(&src[s..s + span]);
            }
            bytes += span * (r.y1 - r.y0) as usize * 4;
        }
        // The overlay: save the pixels it is about to paint (so the next write
        // can put them back), then paint into them.
        if overlay_active {
            for r in overlay_rects.iter() {
                let r = r.intersect(&head);
                if r.is_empty() {
                    continue;
                }
                let len = (r.x1 - r.x0) as usize;
                for y in r.y0 as usize..r.y1 as usize {
                    let d = y * w + r.x0 as usize;
                    self.saved.extend_from_slice(&dst[d..d + len]);
                }
                self.saved_rects.push(r);
                damage.push(r);
            }
            let mut surface = PaintSurface { px: dst, w, h };
            overlay(&mut surface);
        }
        // B86: tell the reader which pixels of this buffer are new, as
        // rectangles. There is no history: a reader that is not exactly one
        // publish behind (`prev_seq != its last frame`) copies everything, so
        // this frame's damage is all it ever needs.
        let table: Option<Vec<render::Rect>> =
            if whole { None } else { coalesce(damage, w as i32, h as i32) };
        unsafe {
            match &table {
                None => {
                    store_u32(self.map, OFF_RECTS, RECTS_FULL);
                    store_u32(self.map, off_damage_count(target), DAMAGE_FULL);
                }
                Some(t) => {
                    for (k, r) in t.iter().enumerate() {
                        let o = off_rect(k);
                        store_u32(self.map, o, r.x0 as u32);
                        store_u32(self.map, o + 4, r.y0 as u32);
                        store_u32(self.map, o + 8, r.x1 as u32);
                        store_u32(self.map, o + 12, r.y1 as u32);
                    }
                    store_u32(self.map, OFF_RECTS, t.len() as u32);
                    // The v3 row table, for readers that speak it: this frame's
                    // rows, or FULL when they would not fit.
                    let rows = rows_of(t);
                    if rows.len() > MAX_DAMAGE_SPANS {
                        store_u32(self.map, off_damage_count(target), DAMAGE_FULL);
                    } else {
                        for (k, &(r0, r1)) in rows.iter().enumerate() {
                            store_u32(self.map, off_damage_row(target, k), r0 as u32);
                            store_u32(self.map, off_damage_row(target, k) + 4, r1 as u32);
                        }
                        store_u32(self.map, off_damage_count(target), rows.len() as u32);
                    }
                }
            }
            store_u64(self.map, off_buf_seq(target), 2 * seq);
            store_u32(self.map, OFF_PUBLISHED, target as u32);
            store_u64(self.map, OFF_SEQ, seq);
        }
        self.published = target as u32;
        self.valid[target] = true;
        self.seq = seq;
        bytes
    }

    /// Test hook: make the header claim a frame the buffers do not hold, or
    /// leave one buffer mid-write. Only used to prove the reader rejects both.
    #[cfg(test)]
    fn poke(&mut self, header_seq: u64, buf: usize, buf_seq: u64) {
        unsafe {
            store_u64(self.map, off_buf_seq(buf), buf_seq);
            store_u64(self.map, OFF_SEQ, header_seq);
        }
    }

    /// Test hook: which buffer is published.
    #[cfg(test)]
    fn published(&self) -> u32 {
        self.published
    }
}

impl Drop for ShmWriter {
    fn drop(&mut self) {
        unsafe {
            libc::munmap(self.map as *mut libc::c_void, self.len);
        }
        self.map = std::ptr::null_mut();
    }
}

/// The overlay's drawing surface (B61b): one head's pixels, in head-local
/// coordinates, recording every pixel it writes so the next frame can put the
/// scene's value back. No allocation (the log is the writer's own buffer).
pub struct PaintSurface<'a> {
    px: &'a mut [u32],
    w: usize,
    h: usize,
}

impl PaintSurface<'_> {
    pub fn width(&self) -> usize {
        self.w
    }

    pub fn height(&self) -> usize {
        self.h
    }

    #[inline]
    fn index(&self, x: i32, y: i32) -> Option<usize> {
        if x < 0 || y < 0 || x >= self.w as i32 || y >= self.h as i32 {
            return None;
        }
        Some(y as usize * self.w + x as usize)
    }

    /// One opaque pixel (a snow flake).
    pub fn put(&mut self, x: i32, y: i32, c: render::Color) {
        if let Some(i) = self.index(x, y) {
            self.px[i] = c;
        }
    }

    /// One alpha-blended pixel, the same maths the `Framebuffer` uses, so a drop
    /// looks the same in the shared buffer as it does in a layer-shell frame.
    pub fn put_blended(&mut self, x: i32, y: i32, c: render::Color, cov: f32) {
        if let Some(i) = self.index(x, y) {
            if cov <= 0.0 {
                return;
            }
            self.px[i] = render::palette::blend(self.px[i], c, cov);
        }
    }
}

impl weather_fx::Surface for PaintSurface<'_> {
    #[inline]
    fn world_bounds(&self) -> render::Rect {
        render::Rect::new(0, 0, self.w as i32, self.h as i32)
    }
    #[inline]
    fn put(&mut self, x: i32, y: i32, c: render::Color) {
        PaintSurface::put(self, x, y, c);
    }
    #[inline]
    fn put_blended(&mut self, x: i32, y: i32, c: render::Color, cov: f32) {
        PaintSurface::put_blended(self, x, y, c, cov);
    }
}

/// One frame as a reader sees it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub scale: u32,
    pub screen: (u32, u32),
    pub fps: u32,
    pub seq: u64,
    pub px: Vec<u32>,
}

/// The reader side of the protocol, mapped read-only.
///
/// The Plasma item in `plasma/frames/` is the real consumer; this one exists so
/// the protocol is tested in Rust (round trip, torn reads, resize, idle) and so
/// a headless rig can check what the writer produced.
pub struct FrameReader {
    path: PathBuf,
    map: *mut u8,
    len: usize,
    dev: u64,
    ino: u64,
    /// Last frame number handed out.
    seq: u64,
    /// Rows the last successful poll copied into `frame.px` (B61).
    copied_rows: usize,
    /// B86: pixels the last successful poll copied (rectangles: their area).
    copied_px: usize,
    /// Has `frame.px` ever been filled completely? Until it has, every poll
    /// copies the whole head, because the rows outside the damage are zeros.
    filled: bool,
    /// Frames rejected by the seqlock (a diagnostic: it should stay 0 when
    /// nothing is racing and climb when one is).
    pub rejected: u64,
    pub frame: Frame,
}

unsafe impl Send for FrameReader {}

fn map_ro(path: &Path) -> io::Result<(*mut u8, usize, u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    let file = File::open(path)?;
    let md = file.metadata()?;
    let len = md.len() as usize;
    if len < HEADER_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame file shorter than its header",
        ));
    }
    let map = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            len,
            libc::PROT_READ,
            libc::MAP_SHARED,
            file.as_raw_fd(),
            0,
        )
    };
    if map == libc::MAP_FAILED {
        return Err(io::Error::last_os_error());
    }
    Ok((map as *mut u8, len, md.dev(), md.ino()))
}

impl FrameReader {
    pub fn open(path: &Path) -> io::Result<Self> {
        let (map, len, dev, ino) = map_ro(path)?;
        let mut r = Self {
            path: path.to_path_buf(),
            map,
            len,
            dev,
            ino,
            seq: 0,
            rejected: 0,
            copied_rows: 0,
            copied_px: 0,
            filled: false,
            frame: Frame::default(),
        };
        // Validate now so a bad file is an error at open, not a panic later.
        r.validate()?;
        Ok(r)
    }

    /// Check every header field the protocol depends on against the file size.
    fn validate(&mut self) -> io::Result<()> {
        let hdr = self.header()?;
        let (w, h) = (hdr.width as usize, hdr.height as usize);
        for (i, off) in hdr.buf_off.iter().enumerate() {
            let bytes = hdr.buf_bytes[i] as usize;
            if bytes < w * 4 * h || off + bytes > self.len {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("buffer {i} does not fit the frame file"),
                ));
            }
        }
        self.frame.width = hdr.width;
        self.frame.height = hdr.height;
        self.frame.scale = hdr.scale;
        self.frame.screen = hdr.screen;
        self.frame.fps = hdr.fps;
        if self.frame.px.len() != w * h {
            self.frame.px = vec![0; w * h];
        }
        Ok(())
    }

    fn header(&self) -> io::Result<Header> {
        let map = self.map;
        let magic = unsafe { std::slice::from_raw_parts(map, 8) };
        if magic != MAGIC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "not a CityLive frame file (bad magic)",
            ));
        }
        let version = unsafe { load_u32(map, OFF_VERSION) };
        if version != VERSION && version != VERSION_V3 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("frame file version {version}, expected {VERSION} (or {VERSION_V3})"),
            ));
        }
        let buffers = unsafe { load_u32(map, OFF_BUFFERS) } as usize;
        if buffers != BUFFERS {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("frame file has {buffers} buffers, expected {BUFFERS}"),
            ));
        }
        let width = unsafe { load_u32(map, OFF_WIDTH) };
        let height = unsafe { load_u32(map, OFF_HEIGHT) };
        if width == 0 || height == 0 || width > 16_384 || height > 16_384 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("absurd frame size {width}x{height}"),
            ));
        }
        let mut buf_off = [0usize; BUFFERS];
        let mut buf_bytes = [0u32; BUFFERS];
        for i in 0..BUFFERS {
            buf_off[i] = unsafe { load_u64(map, off_buf_offset(i)) } as usize;
            buf_bytes[i] = unsafe { load_u32(map, off_buf_bytes(i)) };
            if buf_off[i] < HEADER_BYTES || buf_off[i] > self.len {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("buffer {i} offset outside the file"),
                ));
            }
        }
        Ok(Header {
            version,
            width,
            height,
            scale: unsafe { load_u32(map, OFF_SCALE) }.max(1),
            screen: (
                unsafe { load_u32(map, OFF_SCREEN_W) },
                unsafe { load_u32(map, OFF_SCREEN_H) },
            ),
            fps: unsafe { load_u32(map, OFF_FPS) },
            buf_off,
            buf_bytes,
        })
    }

    /// Has the file been replaced or resized since we mapped it?
    pub fn stale(&self) -> bool {
        use std::os::unix::fs::MetadataExt;
        match fs::metadata(&self.path) {
            Ok(md) => md.ino() != self.ino || md.dev() != self.dev || md.len() as usize != self.len,
            Err(_) => true,
        }
    }

    /// Re-map after the writer recreated the file (a head resized). Keeps the
    /// last frame on screen until a new one is published.
    pub fn remap(&mut self) -> io::Result<()> {
        let (map, len, dev, ino) = map_ro(&self.path)?;
        unsafe { libc::munmap(self.map as *mut libc::c_void, self.len) };
        self.map = map;
        self.len = len;
        self.dev = dev;
        self.ino = ino;
        self.seq = 0;
        self.filled = false;
        self.validate()
    }

    /// Try to take the newest frame. `true` = `self.frame` holds a new picture.
    pub fn poll(&mut self) -> bool {
        if self.stale() && self.remap().is_err() {
            return false;
        }
        let Ok(hdr) = self.header() else { return false };
        let seq = unsafe { load_u64(self.map, OFF_SEQ) };
        if seq == 0 || seq == self.seq {
            return false;
        }
        let published = unsafe { load_u32(self.map, OFF_PUBLISHED) } as usize;
        if published >= BUFFERS {
            self.rejected += 1;
            return false;
        }
        let want = 2 * seq;
        // The writer is inside this buffer (odd) or it does not hold the frame
        // the header claims: keep the previous picture.
        if unsafe { load_u64(self.map, off_buf_seq(published)) } != want {
            self.rejected += 1;
            return false;
        }
        let (w, h) = (hdr.width as usize, hdr.height as usize);
        // B61: a fresh image needs everything; after that only the rows the
        // writer says it changed (or all of them when it says so).
        // `validate` pre-sizes the image, so "have we ever filled every pixel"
        // is its own flag: a partially filled image must never show.
        let fresh = self.frame.px.len() != w * h || !self.filled;
        if self.frame.px.len() != w * h {
            self.frame.px = vec![0; w * h];
        }
        // B86: the damage table describes *this* publish against the one before
        // it. v4 says which one (`prev_seq`); v3 only has `seq`, so a gap in the
        // numbers means a frame went by unseen. Either way an image that is not
        // exactly one publish behind is copied whole.
        let behind = if hdr.version >= VERSION {
            (unsafe { load_u64(self.map, OFF_PREV_SEQ) }) != self.seq
        } else {
            self.seq != 0 && seq != self.seq + 1
        };
        let rows = if fresh || behind {
            Damage::Full
        } else if hdr.version >= VERSION {
            self.damage_rects(w, h)
        } else {
            self.damage_rows(published, h)
        };
        self.copied_px = 0;
        unsafe {
            let src = std::slice::from_raw_parts(self.map.add(hdr.buf_off[published]) as *const u32, w * h);
            match rows {
                Damage::Full => {
                    self.frame.px.copy_from_slice(src);
                    self.copied_rows = h;
                    self.copied_px = w * h;
                    self.filled = true;
                }
                Damage::Rows(spans) => {
                    let mut copied = 0usize;
                    for &span in spans.iter().flatten() {
                        let (y0, y1) = span;
                        if y1 <= y0 || y1 > h {
                            continue;
                        }
                        self.frame.px[y0 * w..y1 * w].copy_from_slice(&src[y0 * w..y1 * w]);
                        copied += y1 - y0;
                    }
                    self.copied_rows = copied;
                    self.copied_px = copied * w;
                }
                Damage::Rects(list) => {
                    let mut rows = 0usize;
                    for r in list.iter() {
                        let (x0, x1) = (r.x0 as usize, r.x1 as usize);
                        for y in r.y0 as usize..r.y1 as usize {
                            self.frame.px[y * w + x0..y * w + x1]
                                .copy_from_slice(&src[y * w + x0..y * w + x1]);
                        }
                        rows += (r.y1 - r.y0) as usize;
                        self.copied_px += (x1 - x0) * (r.y1 - r.y0) as usize;
                    }
                    self.copied_rows = rows;
                }
            }
        }
        if unsafe { load_u64(self.map, off_buf_seq(published)) } != want {
            // Torn: a writer got in mid-copy, so the image now holds pixels from
            // two frames. Mark it unfilled so the next poll copies the whole
            // head: a partial copy must never leave a torn picture behind.
            self.filled = false;
            self.rejected += 1;
            return false;
        }
        self.frame.width = hdr.width;
        self.frame.height = hdr.height;
        self.frame.scale = hdr.scale;
        self.frame.screen = hdr.screen;
        self.frame.fps = hdr.fps;
        self.frame.seq = seq;
        self.seq = seq;
        true
    }

    /// The rows the writer says it changed in buffer `i` (B61). An unusable
    /// table means a full copy: a torn or hostile header must never leave a
    /// pixel from the wrong frame on screen.
    fn damage_rows(&self, i: usize, h: usize) -> Damage {
        if i >= BUFFERS {
            return Damage::Full;
        }
        let count = unsafe { load_u32(self.map, off_damage_count(i)) };
        if count == DAMAGE_FULL || count as usize > MAX_DAMAGE_SPANS {
            return Damage::Full;
        }
        let mut spans: [Option<(usize, usize)>; MAX_DAMAGE_SPANS] = [None; MAX_DAMAGE_SPANS];
        for (k, slot) in spans.iter_mut().enumerate().take(count as usize) {
            let y0 = unsafe { load_u32(self.map, off_damage_row(i, k)) } as usize;
            let y1 = unsafe { load_u32(self.map, off_damage_row(i, k) + 4) } as usize;
            *slot = Some((y0.min(h), y1.min(h)));
        }
        Damage::Rows(spans)
    }

    /// B86: the rectangles the writer names for the current frame, clipped to the
    /// head; an unusable table means a full copy.
    fn damage_rects(&self, w: usize, h: usize) -> Damage {
        let count = unsafe { load_u32(self.map, OFF_RECTS) };
        if count == RECTS_FULL || count as usize > MAX_DAMAGE_RECTS {
            return Damage::Full;
        }
        let mut list = Vec::with_capacity(count as usize);
        for k in 0..count as usize {
            let o = off_rect(k);
            let x0 = (unsafe { load_u32(self.map, o) } as usize).min(w);
            let y0 = (unsafe { load_u32(self.map, o + 4) } as usize).min(h);
            let x1 = (unsafe { load_u32(self.map, o + 8) } as usize).min(w);
            let y1 = (unsafe { load_u32(self.map, o + 12) } as usize).min(h);
            if x1 > x0 && y1 > y0 {
                list.push(Rect::new(x0 as i32, y0 as i32, x1 as i32, y1 as i32));
            }
        }
        Damage::Rects(list)
    }

    /// Pixels the last `poll` copied (B86: rectangles, so this is far below
    /// `copied_rows() * width` in rain).
    pub fn copied_px(&self) -> usize {
        self.copied_px
    }

    /// How many rows the last `poll` copied (B61: the reader's cost is damage,
    /// not frame size). Tests and rigs read it.
    pub fn copied_rows(&self) -> usize {
        self.copied_rows
    }

    /// Poll until a frame newer than `self.seq` arrives or `tries` runs out.
    /// Only for tests and rigs.
    #[cfg(test)]
    fn wait(&mut self, tries: usize) -> bool {
        for _ in 0..tries {
            if self.poll() {
                return true;
            }
        }
        false
    }
}

impl Drop for FrameReader {
    fn drop(&mut self) {
        unsafe {
            libc::munmap(self.map as *mut libc::c_void, self.len);
        }
        self.map = std::ptr::null_mut();
    }
}

/// Which rows of a buffer the reader has to copy (B61).
#[derive(Clone)]
enum Damage {
    Full,
    Rows([Option<(usize, usize)>; MAX_DAMAGE_SPANS]),
    Rects(Vec<Rect>),
}

struct Header {
    version: u32,
    width: u32,
    height: u32,
    scale: u32,
    screen: (u32, u32),
    fps: u32,
    buf_off: [usize; BUFFERS],
    buf_bytes: [u32; BUFFERS],
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::slice::HeadCrop;

    fn tmp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("citylive-shm-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn crop(x: usize, y: usize, w: usize, h: usize) -> HeadCrop {
        HeadCrop { x, y, w, h }
    }

    #[test]
    fn offsets_fit_the_header() {
        assert!(OFF_MAGIC + 8 <= HEADER_BYTES);
        assert_eq!(off_buf_offset(0), 64);
        assert_eq!(off_buf_seq(0), 72);
        assert_eq!(off_buf_bytes(0), 80);
        assert_eq!(off_buf_offset(1), 96);
        assert_eq!(off_buf_seq(1), 104);
        assert_eq!(off_buf_bytes(1), 112);
        assert_eq!(BUF_ENTRY, 32);
        assert!(off_buf_bytes(1) + 4 <= HEADER_BYTES);
        // One buffer (B61): 14.7 MB, not 29.5 MB, on a 2560x1440 head.
        assert_eq!(file_size(2560, 1440), HEADER_BYTES + 14_745_600);
        // B86: the rectangle table sits after the v3 tables and inside the header.
        assert!(OFF_RECTS >= OFF_DAMAGE + 2 * DAMAGE_ENTRY);
        assert!(off_rect(MAX_DAMAGE_RECTS) <= HEADER_BYTES);
        assert_eq!(OFF_PREV_SEQ, off_buf_offset(0) + 24);
        assert_eq!(RECT_ENTRY, 16);
    }

    /// The whole point: what the writer publishes is exactly what the reader
    /// gets, and the reader agrees on the header fields.
    #[test]
    fn round_trip() {
        let dir = tmp_dir("roundtrip");
        let path = dir.join("DP-3.frame");
        let (w, h) = (64usize, 40usize);
        let fb_w = 100usize;
        let fb_h = 40usize;
        let mut fb: Vec<u32> = (0..fb_w * fb_h).map(|i| 0x0010_0000 + i as u32).collect();
        let c = crop(10, 0, w, h);
        let mut writer = ShmWriter::open(&path, w as u32, h as u32, 2, (128, 80), 30).unwrap();
        let mut reader = FrameReader::open(&path).unwrap();
        assert!(!reader.poll(), "nothing published yet");

        let mut seq = 1u64;
        for round in 0..4 {
            // Change two rows and publish.
            for x in 0..fb_w {
                fb[((5 + round * 4) * fb_w) + x] = 0x00ff_0000 + round as u32;
            }
            let y = 5 + round * 4;
            let rects = vec![Rect::new(0, y as i32, w as i32, y as i32 + 1)];
            writer.publish(seq, &fb, fb_w, &c, &rects, round == 0);
            seq += 1;
            assert!(reader.poll(), "round {round}: no frame");
            assert_eq!(reader.frame.seq, seq - 1);
            let (fw, fh) = (reader.frame.width as usize, reader.frame.height as usize);
            assert_eq!((fw, fh), (w, h));
            assert_eq!(reader.frame.scale, 2);
            assert_eq!(reader.frame.screen, (128, 80));
            assert_eq!(reader.frame.fps, 30);
            for y in 0..h {
                for x in 0..w {
                    assert_eq!(
                        reader.frame.px[y * w + x],
                        fb[(c.y + y) * fb_w + c.x + x],
                        "round {round} pixel ({x},{y})"
                    );
                }
            }
        }
        let _ = fs::remove_dir_all(&dir);
    }

    /// A reader that arrives while the writer is inside a buffer keeps the
    /// previous frame instead of showing half a new one.
    #[test]
    fn a_torn_read_is_rejected() {
        let dir = tmp_dir("torn");
        let path = dir.join("DP-4.frame");
        let (w, h) = (32usize, 16usize);
        let fb: Vec<u32> = (0..w * h).map(|i| i as u32 | 0x0100_0000).collect();
        let c = crop(0, 0, w, h);
        let mut writer = ShmWriter::open(&path, w as u32, h as u32, 1, (32, 16), 30).unwrap();
        let mut reader = FrameReader::open(&path).unwrap();
        writer.publish(1, &fb, w, &c, &[], true);
        assert!(reader.poll());
        let good = reader.frame.clone();

        // (a) the writer rewrote the buffer we already showed, header still old
        let idx = writer.published() as usize;
        writer.poke(2, idx, 2 * 2 + 1); // odd: being written
        assert!(!reader.poll(), "an odd buffer seq must be rejected");
        assert_eq!(reader.frame, good, "the previous frame must stay");
        assert!(reader.rejected >= 1);

        // (b) the header claims a frame no buffer holds
        writer.poke(3, idx, 2 * 2);
        assert!(!reader.poll(), "a buffer that does not match the header must be rejected");
        assert_eq!(reader.frame, good);

        // (c) and when the writer really publishes, the reader takes it again
        writer.publish(4, &fb, w, &c, &[], true);
        assert!(reader.poll());
        assert_eq!(reader.frame.seq, 4);
        let _ = fs::remove_dir_all(&dir);
    }

    /// A resize recreates the file under a new inode; the reader re-maps and
    /// then sees the new size, without ever reading a half-built one.
    #[test]
    fn a_resize_is_remapped() {
        let dir = tmp_dir("resize");
        let path = dir.join("HDMI.frame");
        let (w, h) = (40usize, 20usize);
        let fb: Vec<u32> = vec![0x0000_1234; w * h];
        let c = crop(0, 0, w, h);
        let mut writer = ShmWriter::open(&path, w as u32, h as u32, 1, (40, 20), 30).unwrap();
        writer.publish(1, &fb, w, &c, &[], true);
        let mut reader = FrameReader::open(&path).unwrap();
        assert!(reader.poll());
        assert_eq!(reader.frame.width, 40);

        drop(writer);
        let (w2, h2) = (20usize, 10usize);
        let fb2: Vec<u32> = vec![0x0000_5678; w2 * h2];
        let c2 = crop(0, 0, w2, h2);
        let mut writer2 = ShmWriter::open(&path, w2 as u32, h2 as u32, 2, (40, 20), 30).unwrap();
        assert!(reader.stale(), "the reader must notice the new inode");
        writer2.publish(1, &fb2, w2, &c2, &[], true);
        assert!(reader.poll());
        assert_eq!((reader.frame.width, reader.frame.height), (20, 10));
        assert_eq!(reader.frame.scale, 2);
        assert_eq!(reader.frame.px.len(), 200);
        assert!(reader.frame.px.iter().all(|&p| p == 0x0000_5678));
        let _ = fs::remove_dir_all(&dir);
    }

    /// Nothing published = nothing for the reader, and (the real point) the
    /// writer copies no bytes at all when a frame is unchanged.
    #[test]
    fn idle_publishes_nothing() {
        let dir = tmp_dir("idle");
        let path = dir.join("DP-5.frame");
        let (w, h) = (48usize, 24usize);
        let fb: Vec<u32> = vec![0x0000_0f0f; w * h];
        let c = crop(0, 0, w, h);
        let mut writer = ShmWriter::open(&path, w as u32, h as u32, 1, (48, 24), 30).unwrap();
        writer.publish(1, &fb, w, &c, &[], true);
        writer.publish(2, &fb, w, &c, &[], true); // both buffers now hold it
        // ...and the third is a full copy because the frame before it was one,
        // which is exactly the state where the next publish can be free.
        writer.publish(3, &fb, w, &c, &[], false);
        let mut reader = FrameReader::open(&path).unwrap();
        assert!(reader.poll());
        // Three "idle" ticks: no new seq, so no publish and no reader work.
        for _ in 0..3 {
            assert!(!reader.poll());
        }
        assert_eq!(writer.seq(), 3);
        // An explicit publish with no damaged rows (and no damage remembered
        // from the frame before) copies nothing either.
        assert_eq!(writer.publish(4, &fb, w, &c, &[], false), 0);
        assert!(reader.poll());
        assert_eq!(reader.frame.seq, 4);
        let _ = fs::remove_dir_all(&dir);
    }

    /// Only the damaged rows are copied: with one buffer (B61) the buffer holds
    /// frame `n-1`, so a frame that changes 10 rows of 1440 copies exactly those.
    #[test]
    fn only_damaged_rows_are_copied() {
        let dir = tmp_dir("damage");
        let path = dir.join("DP-6.frame");
        let (w, h) = (100usize, 1440usize);
        let mut fb: Vec<u32> = vec![0x0000_0001; w * h];
        let c = crop(0, 0, w, h);
        let mut writer = ShmWriter::open(&path, w as u32, h as u32, 1, (100, 1440), 30).unwrap();
        let full = writer.publish(1, &fb, w, &c, &[], true);
        assert_eq!(full, w * h * 4, "the first frame is a full copy");
        // Frame 2 damages rows 200..210: 10 rows, not the head.
        for y in 200..210 {
            for x in 0..w {
                fb[y * w + x] = 0x0000_0002;
            }
        }
        let bytes = writer.publish(2, &fb, w, &c, &[Rect::new(0, 200, w as i32, 210)], false);
        assert_eq!(bytes, 10 * w * 4);
        // Frame 3 damages rows 900..901: 1 row.
        for x in 0..w {
            fb[900 * w + x] = 0x0000_0003;
        }
        let bytes = writer.publish(3, &fb, w, &c, &[Rect::new(0, 900, w as i32, 901)], false);
        assert_eq!(bytes, w * 4);
        // Frame 4 damages rows 1000..1001: 1 row again (nothing of frame 2 is
        // missing from the buffer any more).
        for x in 0..w {
            fb[1000 * w + x] = 0x0000_0004;
        }
        let bytes = writer.publish(4, &fb, w, &c, &[Rect::new(0, 1000, w as i32, 1001)], false);
        assert_eq!(bytes, w * 4);
        // And the picture is still right after all that.
        let mut reader = FrameReader::open(&path).unwrap();
        // Reader takes the newest published frame (4).
        assert!(reader.wait(4));
        assert_eq!(reader.frame.seq, 4);
        for (i, &p) in reader.frame.px.iter().enumerate() {
            assert_eq!(p, fb[i], "pixel {i}");
        }
        let _ = fs::remove_dir_all(&dir);
    }

    /// B61: the reader keeps its image and copies only the rows the writer's
    /// damage table names — while still ending up with exactly the published
    /// picture, including the rows the *other* buffer was holding.
    #[test]
    fn the_reader_copies_only_the_damaged_rows() {
        let dir = tmp_dir("partialcopy");
        let path = dir.join("DP-8.frame");
        let (w, h) = (64usize, 300usize);
        let mut fb: Vec<u32> = vec![0x0000_0001; w * h];
        let c = crop(0, 0, w, h);
        let mut writer = ShmWriter::open(&path, w as u32, h as u32, 1, (64, 300), 30).unwrap();
        let mut reader = FrameReader::open(&path).unwrap();

        writer.publish(1, &fb, w, &c, &[], true);
        assert!(reader.poll());
        assert_eq!(reader.copied_rows(), h, "the first frame is a full copy");

        // Frame 2 damages rows 10..14, into buffer 1 (which held nothing).
        for y in 10..14 {
            for x in 0..w {
                fb[y * w + x] = 0x0000_0002;
            }
        }
        writer.publish(2, &fb, w, &c, &[Rect::new(0, 10, w as i32, 14)], false);
        assert!(reader.poll());
        assert!(reader.copied_rows() > 0);
        assert!(reader.copied_rows() <= h);
        for y in 0..h {
            for x in 0..w {
                assert_eq!(reader.frame.px[y * w + x], fb[y * w + x], "frame 2 ({x},{y})");
            }
        }

        // Frame 3 damages rows 200..201: with one buffer (B61) that is the only
        // row the reader has to copy.
        for x in 0..w {
            fb[200 * w + x] = 0x0000_0003;
        }
        writer.publish(3, &fb, w, &c, &[Rect::new(0, 200, w as i32, 201)], false);
        assert!(reader.poll());
        let rows = reader.copied_rows();
        // With the damage history the reader may copy more than this frame's row
        // (the recent frames' rows too), never less; the image must be exact.
        assert!(rows >= 1, "copied {rows} of {h} rows for one damaged row");
        for y in 0..h {
            for x in 0..w {
                assert_eq!(reader.frame.px[y * w + x], fb[y * w + x], "frame 3 ({x},{y})");
            }
        }

        // A frame that changed nothing writes nothing (B61: the buffer already
        // holds every row of the picture); the reader may still re-copy the
        // recent frames' rows (the damage history), and stays exact.
        assert_eq!(writer.publish(4, &fb, w, &c, &[], false), 0);
        assert!(reader.poll(), "frame 4 is still a new frame number");
        assert!(reader.frame.px == fb, "the image is exact");
        let _ = fs::remove_dir_all(&dir);
    }

    /// B61b: an overlay painted into a buffer must be *taken back out* the next
    /// time that buffer is written, so a moving drop leaves no trail and a frame
    /// with no overlay is exactly the scene's own pixels.
    #[test]
    fn an_overlay_is_restored_by_the_next_publish() {
        let dir = tmp_dir("overlay");
        let path = dir.join("DP-10.frame");
        let (w, h) = (64usize, 48usize);
        let fb: Vec<u32> = (0..w * h).map(|i| 0x0000_0000 | (i as u32 & 0xFF)).collect();
        let c = crop(0, 0, w, h);
        let mut writer = ShmWriter::open(&path, w as u32, h as u32, 1, (64, 48), 30).unwrap();
        let mut reader = FrameReader::open(&path).unwrap();

        // Frame 1: the scene only.
        writer.publish(1, &fb, w, &c, &[], true);
        assert!(reader.poll());
        let clean = reader.frame.px.clone();
        assert_eq!(clean, fb);

        // Frame 2: a "drop" (two blended pixels) painted at (10,10) and (11,11).
        let drop_a = 0xFF00_00FF;
        let drop_rects = [
            Rect::new(10, 10, 11, 11),
            Rect::new(11, 11, 12, 12),
        ];
        writer.publish_with(2, &fb, w, &c, &[], false, &drop_rects, |s| {
            s.put_blended(10, 10, drop_a, 0.5);
            s.put_blended(11, 11, drop_a, 0.5);
        });
        assert!(reader.poll());
        let with_drop = reader.frame.px.clone();
        assert_ne!(with_drop[10 * w + 10], clean[10 * w + 10]);

        // Frame 3: no overlay at all — the drops must be gone, not smeared.
        writer.publish(3, &fb, w, &c, &[], false);
        assert!(reader.poll());
        assert_eq!(reader.frame.px, clean, "the overlay was not taken back out");

        // Frame 4: the drop moves; the old position is clean and the new one is
        // painted (publishing the same frame twice must not accumulate).
        let moved = [Rect::new(30, 20, 31, 21)];
        writer.publish_with(4, &fb, w, &c, &[], false, &moved, |s| {
            s.put_blended(30, 20, drop_a, 0.5);
        });
        assert!(reader.poll());
        assert_eq!(reader.frame.px[10 * w + 10], clean[10 * w + 10]);
        assert_ne!(reader.frame.px[20 * w + 30], clean[20 * w + 30]);
        let painted = reader.frame.px[20 * w + 30];
        writer.publish_with(5, &fb, w, &c, &[], false, &moved, |s| {
            s.put_blended(30, 20, drop_a, 0.5);
        });
        assert!(reader.poll());
        assert_eq!(reader.frame.px[20 * w + 30], painted, "painting twice must not darken");
        let _ = fs::remove_dir_all(&dir);
    }

    /// B93: the writer says when its buffer still carries overlay pixels, so
    /// the caller publishes once more when the weather stops (a quiet head has
    /// no other damage, and the last drops would stay frozen on it).
    #[test]
    fn the_writer_reports_overlay_pixels_still_in_the_buffer() {
        let dir = tmp_dir("overlaypending");
        let path = dir.join("DP-13.frame");
        let (w, h) = (32usize, 32usize);
        let fb: Vec<u32> = vec![0x0010_2030; w * h];
        let c = crop(0, 0, w, h);
        let mut writer = ShmWriter::open(&path, w as u32, h as u32, 1, (32, 32), 30).unwrap();
        let mut reader = FrameReader::open(&path).unwrap();
        assert!(!writer.overlay_pending());
        writer.publish(1, &fb, w, &c, &[], true);
        assert!(!writer.overlay_pending());
        let r = [Rect::new(5, 5, 6, 6)];
        writer.publish_with(2, &fb, w, &c, &[], false, &r, |s| s.put(5, 5, 0x00FF_FFFF));
        assert!(writer.overlay_pending(), "the drop is still in the buffer");
        assert!(reader.poll());
        assert_eq!(reader.frame.px[5 * w + 5], 0x00FF_FFFF);
        // The publish that follows the weather stopping takes it back out.
        writer.publish(3, &fb, w, &c, &[], false);
        assert!(!writer.overlay_pending());
        assert!(reader.poll());
        assert_eq!(reader.frame.px, fb);
        let _ = fs::remove_dir_all(&dir);
    }

    /// 2026-11-12: a reader that misses frames must still end up with every
    /// changed row (missed full repaints left stale bands on the desktop).
    #[test]
    fn a_reader_that_skips_frames_still_gets_every_row() {
        let dir = tmp_dir("skipframes");
        let path = dir.join("DP-12.frame");
        let (w, h) = (16usize, 64usize);
        let mut fb: Vec<u32> = vec![1; w * h];
        let c = crop(0, 0, w, h);
        let mut writer = ShmWriter::open(&path, w as u32, h as u32, 1, (16, 64), 30).unwrap();
        let mut reader = FrameReader::open(&path).unwrap();
        writer.publish(1, &fb, w, &c, &[], true);
        assert!(reader.poll());
        // Flush the history of the first full frame.
        for seq in 2..12u64 {
            writer.publish(seq, &fb, w, &c, &[], false);
            reader.poll();
        }
        // Three frames change different rows; the reader sees only the last.
        for (k, y) in [(12u64, 3usize), (13, 30), (14, 60)] {
            for x in 0..w {
                fb[y * w + x] = 100 + k as u32;
            }
            writer.publish(k, &fb, w, &c, &[Rect::new(0, y as i32, w as i32, y as i32 + 1)], false);
        }
        assert!(reader.poll());
        for y in 0..h {
            for x in 0..w {
                assert_eq!(reader.frame.px[y * w + x], fb[y * w + x], "({x},{y})");
            }
        }
        // A full repaint the reader misses is still delivered whole.
        fb.iter_mut().for_each(|p| *p = 7);
        writer.publish(15, &fb, w, &c, &[], true);
        writer.publish(16, &fb, w, &c, &[], false);
        assert!(reader.poll());
        assert!(reader.frame.px.iter().all(|&p| p == 7), "a missed full repaint left stale rows");
        let _ = fs::remove_dir_all(&dir);
    }

    /// The overlay's rows reach the reader: a head whose scene did not change
    /// still gets the drops (the reader copies whole rows).
    #[test]
    fn an_overlay_only_frame_still_names_its_rows() {
        let dir = tmp_dir("overlayrows");
        let path = dir.join("DP-11.frame");
        let (w, h) = (32usize, 40usize);
        let fb: Vec<u32> = vec![0x0000_0011; w * h];
        let c = crop(0, 0, w, h);
        let mut writer = ShmWriter::open(&path, w as u32, h as u32, 1, (32, 40), 30).unwrap();
        let mut reader = FrameReader::open(&path).unwrap();
        writer.publish(1, &fb, w, &c, &[], true);
        assert!(reader.poll());
        let one = [Rect::new(5, 7, 6, 8)];
        writer.publish_with(2, &fb, w, &c, &[], false, &one, |s| {
            s.put(5, 7, 0xFF12_3456);
        });
        assert!(reader.poll());
        assert!(reader.copied_rows() >= 1, "the drop's row must be copied");
        assert_eq!(reader.frame.px[7 * w + 5], 0xFF12_3456);
        let _ = fs::remove_dir_all(&dir);
    }

    /// A header that claims a version this reader does not know is refused
    /// outright: the layout below it has changed (B61 bumped it to 2).
    #[test]
    fn an_unknown_version_is_refused() {
        let dir = tmp_dir("version");
        let path = dir.join("DP-9.frame");
        let mut writer = ShmWriter::open(&path, 32, 16, 1, (32, 16), 30).unwrap();
        writer.publish(1, &vec![0u32; 32 * 16], 32, &crop(0, 0, 32, 16), &[], true);
        assert!(FrameReader::open(&path).is_ok());
        // Bump the version behind the writer's back.
        unsafe {
            store_u32(writer.map, OFF_VERSION, VERSION + 1);
        }
        assert!(FrameReader::open(&path).is_err(), "a future layout must not be read");
        let _ = fs::remove_dir_all(&dir);
    }

    /// A full recomposite is expensive for exactly one more frame; it must not
    /// leave the writer copy-copy-copying the whole head from then on.
    #[test]
    fn a_full_frame_does_not_poison_the_next_publishes() {
        let dir = tmp_dir("fullframe");
        let path = dir.join("DP-7.frame");
        let (w, h) = (64usize, 200usize);
        let mut fb: Vec<u32> = vec![0x0000_0009; w * h];
        let c = crop(0, 0, w, h);
        let mut writer = ShmWriter::open(&path, w as u32, h as u32, 1, (64, 200), 30).unwrap();
        writer.publish(1, &fb, w, &c, &[], true);
        writer.publish(2, &fb, w, &c, &[], true);
        // One full frame (a sky change), then damage-only frames.
        writer.publish(3, &fb, w, &c, &[Rect::new(0, 0, w as i32, h as i32)], true);
        let mut seq = 4u64;
        for row in 10..16 {
            for x in 0..w {
                fb[row * w + x] = row as u32;
            }
            let bytes = writer.publish(
                seq,
                &fb,
                w,
                &c,
                &[Rect::new(0, row as i32, w as i32, row as i32 + 1)],
                false,
            );
            // The buffer holds frame n-2, so a publish copies this frame's row
            // and the previous one: two rows, never the whole head again.
            if seq >= 6 {
                assert!(
                    bytes <= 2 * w * 4,
                    "frame {seq} copied {bytes} bytes (the whole head is {})",
                    w * h * 4
                );
            }
            seq += 1;
        }
        let _ = fs::remove_dir_all(&dir);
    }


    /// Throughput: how fast a full 2560×1440 head can be published, which is the
    /// worst case (every row copied). Printed with `--nocapture`.
    #[test]
    fn throughput_of_a_full_head() {
        let dir = tmp_dir("throughput");
        let path = dir.join("DP-3.frame");
        let (w, h) = (2560usize, 1440usize);
        let fb: Vec<u32> = vec![0x0000_2a2a; w * h];
        let c = crop(0, 0, w, h);
        let mut writer = ShmWriter::open(&path, w as u32, h as u32, 1, (2560, 1440), 30).unwrap();
        let frames = 30u64;
        let t0 = std::time::Instant::now();
        let mut bytes = 0usize;
        for n in 1..=frames {
            bytes += writer.publish(n, &fb, w, &c, &[], true);
        }
        let s = t0.elapsed().as_secs_f64();
        let mbs = bytes as f64 / 1e6 / s;
        eprintln!(
            "full 2560x1440 head: {} frames in {:.3} s = {:.2} ms/frame, {:.0} MB/s copied",
            frames,
            s,
            s * 1000.0 / frames as f64,
            mbs
        );
        // A memcpy of 14.7 MB cannot be slower than this on any machine we run
        // on; the assertion is a floor, the number above is the result.
        assert!(mbs > 300.0, "only {mbs:.0} MB/s");
        let _ = fs::remove_dir_all(&dir);
    }

    /// B85: the rain overlay, run through the real publish + reader loop for 600
    /// frames, must never leave a drop behind. The reader mirrors
    /// `plasma/frames/cityframe.cpp` (keeps its image, copies only the damaged
    /// rows); comparing it to a fresh composite catches any trail the save/
    /// restore leaves — an old drop position that was not taken back out.
    /// Round 2: a second reader that *skips* frames (polls every 4th) must still
    /// end up exact, because the writer advertises the last `DAMAGE_HISTORY`
    /// frames' rows.
    #[test]
    fn rain_overlay_leaves_no_trails() {
        let (w, h) = (200usize, 140usize);
        let dir = tmp_dir("raintrail");
        let path = dir.join("head.frame");
        // A static "scene": the drop pixels blend over whatever this holds.
        let src: Vec<u32> = (0..w * h).map(|i| 0xFF00_0000 | (i as u32 & 0x7f)).collect();

        // Heavy rain, wind from the west (a leftward slant).
        let mut wx = world::Weather::default();
        wx.code = 65;
        wx.precip = 3.0;
        wx.cloud = 100.0;
        wx.wind = 30.0;
        wx.wind_dir = Some(250.0);
        wx.visibility = Some(4000.0);

        let mut fx = weather_fx::Fx::new(0xC17E_15E7, w, h);
        for _ in 0..60 {
            fx.update(1.0 / 30.0, &wx);
        }

        let mut writer = ShmWriter::open(&path, w as u32, h as u32, 1, (w as u32, h as u32), 30)
            .unwrap();
        let mut reader = FrameReader::open(&path).unwrap();
        let mut skipper = FrameReader::open(&path).unwrap();
        let c = crop(0, 0, w, h);

        let mut overlay_rects: Vec<Rect> = Vec::new();
        let mut truth = render::Framebuffer::new(w, h);
        let mut seq = 1u64;
        for frame in 0..600 {
            fx.update(1.0 / 30.0, &wx);
            fx.overlay_rects_fine(&mut overlay_rects, STREAK_SLICE_ROWS);
            writer.publish_with(seq, &src, w, &c, &[], false, &overlay_rects, |s| {
                fx.draw_overlay(s, 0, 0);
            });
            seq += 1;
            if frame % 30 == 29 && reader.poll() {
                truth.px_mut().copy_from_slice(&src);
                fx.draw_overlay(&mut truth, 0, 0);
                assert_eq!(
                    reader.frame.px,
                    truth.px(),
                    "frame {frame}: the reader and a fresh composite differ (a trail)"
                );
            }
            // A reader that skips three frames in four, checked every 40 frames.
            if frame % 4 == 3 {
                skipper.poll();
            }
            if frame % 40 == 39 {
                truth.px_mut().copy_from_slice(&src);
                fx.draw_overlay(&mut truth, 0, 0);
                assert_eq!(
                    skipper.frame.px,
                    truth.px(),
                    "frame {frame}: the frame-skipping reader is stale (a trail)"
                );
            }
        }
        let _ = fs::remove_dir_all(&dir);
    }

    /// B86: a few rectangles in, exactly those pixels out. The reader copies the
    /// rectangles' area, not their rows.
    #[test]
    fn rect_table_round_trip() {
        let dir = tmp_dir("rects");
        let path = dir.join("R-1.frame");
        let (w, h) = (128usize, 96usize);
        let mut fb: Vec<u32> = vec![5; w * h];
        let c = crop(0, 0, w, h);
        let mut writer = ShmWriter::open(&path, w as u32, h as u32, 1, (128, 96), 30).unwrap();
        let mut reader = FrameReader::open(&path).unwrap();
        writer.publish(1, &fb, w, &c, &[], true);
        assert!(reader.poll());
        assert_eq!(reader.copied_px(), w * h);
        let rs = [Rect::new(3, 4, 10, 9), Rect::new(100, 70, 120, 90)];
        for r in rs {
            for y in r.y0..r.y1 {
                for x in r.x0..r.x1 {
                    fb[y as usize * w + x as usize] = 77;
                }
            }
        }
        let bytes = writer.publish(2, &fb, w, &c, &rs, false);
        assert_eq!(bytes, (7 * 5 + 20 * 20) * 4);
        assert!(reader.poll());
        assert_eq!(reader.copied_px(), 7 * 5 + 20 * 20, "only the rectangles are copied");
        assert_eq!(reader.frame.px, fb);
        // The table as written.
        unsafe {
            assert_eq!(load_u32(writer.map, OFF_RECTS), 2);
            assert_eq!(load_u64(writer.map, OFF_PREV_SEQ), 1);
        }
        let _ = fs::remove_dir_all(&dir);
    }

    /// B86: `prev_seq`, not the sequence number, decides whether a reader is
    /// caught up. A head the wallpaper did not publish to for a while (its seq
    /// jumps) is still read incrementally when the reader saw the publish before.
    #[test]
    fn a_seq_jump_between_publishes_is_not_a_missed_frame() {
        let dir = tmp_dir("seqjump");
        let path = dir.join("R-2.frame");
        let (w, h) = (64usize, 64usize);
        let mut fb: Vec<u32> = vec![1; w * h];
        let c = crop(0, 0, w, h);
        let mut writer = ShmWriter::open(&path, w as u32, h as u32, 1, (64, 64), 30).unwrap();
        let mut reader = FrameReader::open(&path).unwrap();
        writer.publish(1, &fb, w, &c, &[], true);
        assert!(reader.poll());
        fb[10 * w + 10] = 9;
        writer.publish(40, &fb, w, &c, &[Rect::new(10, 10, 11, 11)], false);
        assert!(reader.poll());
        assert_eq!(reader.copied_px(), 1, "seq 1 -> 40 with nothing between is not a gap");
        assert_eq!(reader.frame.px, fb);
        let _ = fs::remove_dir_all(&dir);
    }

    /// B86: a reader that misses publishes gets a full copy, so it needs no
    /// history in the table, and it ends up exact even when the missed frames
    /// changed different places and an overlay moved in between.
    #[test]
    fn a_skipping_reader_converges_with_rects() {
        let dir = tmp_dir("rectskip");
        let path = dir.join("R-3.frame");
        let (w, h) = (96usize, 80usize);
        let mut fb: Vec<u32> = vec![1; w * h];
        let c = crop(0, 0, w, h);
        let mut writer = ShmWriter::open(&path, w as u32, h as u32, 1, (96, 80), 30).unwrap();
        let mut reader = FrameReader::open(&path).unwrap();
        writer.publish(1, &fb, w, &c, &[], true);
        assert!(reader.poll());
        let mut truth = fb.clone();
        let mut rng = 0x1234_5678_u64;
        let mut next = move || {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            rng
        };
        for seq in 2..200u64 {
            let (x, y) = ((next() % 90) as i32, (next() % 70) as i32);
            let r = Rect::new(x, y, x + 1 + (next() % 5) as i32, y + 1 + (next() % 9) as i32);
            for yy in r.y0..r.y1.min(h as i32) {
                for xx in r.x0..r.x1.min(w as i32) {
                    fb[yy as usize * w + xx as usize] = seq as u32;
                }
            }
            let (ox, oy) = ((next() % 90) as i32, (next() % 70) as i32);
            let orect = Rect::new(ox, oy, ox + 3, oy + 3);
            writer.publish_with(seq, &fb, w, &c, &[r], false, &[orect], |s| {
                for dy in 0..3 {
                    for dx in 0..3 {
                        s.put(ox + dx, oy + dy, 0xFFAA_0000 + seq as u32);
                    }
                }
            });
            if seq % 5 == 0 {
                // Poll only every fifth publish.
                assert!(reader.poll());
                truth.copy_from_slice(&fb);
                for dy in 0..3 {
                    for dx in 0..3 {
                        truth[(oy + dy) as usize * w + (ox + dx) as usize] = 0xFFAA_0000 + seq as u32;
                    }
                }
                assert_eq!(reader.frame.px, truth, "publish {seq}");
                assert_eq!(reader.copied_px(), w * h, "a skip must be a full copy");
            }
        }
        let _ = fs::remove_dir_all(&dir);
    }

    /// B86: a big change (more than half the head) is announced as FULL.
    #[test]
    fn a_wide_change_falls_back_to_full() {
        let dir = tmp_dir("rectfull");
        let path = dir.join("R-4.frame");
        let (w, h) = (64usize, 64usize);
        let mut fb: Vec<u32> = vec![1; w * h];
        let c = crop(0, 0, w, h);
        let mut writer = ShmWriter::open(&path, w as u32, h as u32, 1, (64, 64), 30).unwrap();
        let mut reader = FrameReader::open(&path).unwrap();
        writer.publish(1, &fb, w, &c, &[], true);
        assert!(reader.poll());
        fb.iter_mut().take(w * 40).for_each(|p| *p = 3);
        writer.publish(2, &fb, w, &c, &[Rect::new(0, 0, 64, 40)], false);
        unsafe {
            assert_eq!(load_u32(writer.map, OFF_RECTS), RECTS_FULL);
            assert_eq!(load_u32(writer.map, off_damage_count(0)), DAMAGE_FULL);
        }
        assert!(reader.poll());
        assert_eq!(reader.copied_px(), w * h);
        assert_eq!(reader.frame.px, fb);
        let _ = fs::remove_dir_all(&dir);
    }

    /// B86: `coalesce` covers every input rectangle, stays inside the cap, and
    /// says FULL when it cannot do both cheaply.
    #[test]
    fn coalesce_covers_everything_within_the_cap() {
        let (w, h) = (2560i32, 1440i32);
        let mut rng = 0xDEAD_BEEF_u64;
        let mut next = move || {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            rng
        };
        for n in [0usize, 1, 50, 900, 1500, 4000, 20_000] {
            let input: Vec<Rect> = (0..n)
                .map(|_| {
                    let x = (next() % 2540) as i32;
                    let y = (next() % 1400) as i32;
                    Rect::new(x, y, x + 1 + (next() % 8) as i32, y + 1 + (next() % 30) as i32)
                })
                .collect();
            match coalesce(input.clone(), w, h) {
                None => eprintln!("coalesce {n:6} -> FULL"),
                Some(out) => {
                    assert!(out.len() <= MAX_DAMAGE_RECTS, "{} rects", out.len());
                    for r in &input {
                        assert!(
                            out.iter().any(|o| o.x0 <= r.x0 && o.y0 <= r.y0 && o.x1 >= r.x1 && o.y1 >= r.y1),
                            "{r:?} is not covered ({n} in, {} out)",
                            out.len()
                        );
                    }
                    let covered: i64 = out.iter().map(|r| ((r.x1 - r.x0) * (r.y1 - r.y0)) as i64).sum();
                    eprintln!("coalesce {n:6} -> {:4} rects, {:5} KB", out.len(), covered * 4 / 1024);
                }
            }
        }
    }

    /// B86 compatibility: a reader looking at a v3 file (an older wallpaper)
    /// uses the row table and the `seq != seen + 1` rule, and stays exact. The
    /// writer here is the v4 one with its version field turned back to 3, which
    /// is what an old writer's file looks like to a new reader.
    #[test]
    fn a_reader_still_reads_a_v3_file() {
        let dir = tmp_dir("v3compat");
        let path = dir.join("R-5.frame");
        let (w, h) = (48usize, 60usize);
        let mut fb: Vec<u32> = vec![2; w * h];
        let c = crop(0, 0, w, h);
        let mut writer = ShmWriter::open(&path, w as u32, h as u32, 1, (48, 60), 30).unwrap();
        unsafe { store_u32(writer.map, OFF_VERSION, VERSION_V3) };
        let mut reader = FrameReader::open(&path).expect("v3 files are still accepted");
        writer.publish(1, &fb, w, &c, &[], true);
        assert!(reader.poll());
        fb[7 * w + 3] = 99;
        writer.publish(2, &fb, w, &c, &[Rect::new(3, 7, 4, 8)], false);
        assert!(reader.poll());
        assert_eq!(reader.copied_rows(), 1, "v3 copies the named row");
        assert_eq!(reader.frame.px, fb);
        // A skipped frame: full copy under the v3 rule.
        fb[40 * w + 1] = 5;
        writer.publish(3, &fb, w, &c, &[Rect::new(1, 40, 2, 41)], false);
        fb[20 * w + 2] = 6;
        writer.publish(4, &fb, w, &c, &[Rect::new(2, 20, 3, 21)], false);
        assert!(reader.poll());
        assert_eq!(reader.copied_rows(), h);
        assert_eq!(reader.frame.px, fb);
        let _ = fs::remove_dir_all(&dir);
    }

    /// B86: what a downpour costs the reader on a 2560x1440 head of the 7040-wide
    /// union (the first head, where the fx field is 7040 wide), reader polling
    /// every frame. Printed with `--nocapture`; rain must stay under 30 MB/s
    /// (it was ~425 MB/s with whole rows).
    #[test]
    fn a_downpour_is_cheap_to_copy() {
        let (uw, uh) = (7040usize, 1440usize);
        let (w, h) = (2560usize, 1440usize);
        let src: Vec<u32> = (0..uw * uh).map(|i| 0xFF00_0000 | (i as u32 & 0xff)).collect();
        for (name, limit) in [("drizzle", 10.0), ("rain", 25.0), ("storm", 60.0)] {
            let dir = tmp_dir(&format!("budget-{name}"));
            let path = dir.join("head.frame");
            let wx = weather_fx::look::preset(name).unwrap();
            let mut fx = weather_fx::Fx::new(0xC17E_15E7, uw, uh);
            for _ in 0..90 {
                fx.update(1.0 / 30.0, &wx);
            }
            let mut writer =
                ShmWriter::open(&path, w as u32, h as u32, 1, (w as u32, h as u32), 30).unwrap();
            let mut reader = FrameReader::open(&path).unwrap();
            let c = crop(0, 0, w, h);
            let mut union_rects: Vec<Rect> = Vec::new();
            let (mut px, mut frames, mut fulls, mut rect_n) = (0usize, 0usize, 0usize, 0usize);
            for seq in 1..=150u64 {
                fx.update(1.0 / 30.0, &wx);
                fx.overlay_rects_fine(&mut union_rects, STREAK_SLICE_ROWS);
                let head: Vec<Rect> = union_rects
                    .iter()
                    .map(|r| r.intersect(&Rect::new(0, 0, w as i32, h as i32)))
                    .filter(|r| !r.is_empty())
                    .collect();
                writer.publish_with(seq, &src, uw, &c, &[], seq == 1, &head, |s| {
                    fx.draw_overlay(s, 0, 0);
                });
                assert!(reader.poll());
                if seq > 5 {
                    frames += 1;
                    px += reader.copied_px();
                    if reader.copied_px() == w * h {
                        fulls += 1;
                    }
                    rect_n += unsafe { load_u32(reader.map, OFF_RECTS) }.min(99_999) as usize;
                }
            }
            let mbs = px as f64 * 4.0 / frames as f64 * 30.0 / 1e6;
            eprintln!(
                "{name:8}: {mbs:6.1} MB/s per head, {:5.0} rects/frame, {fulls} full of {frames}",
                rect_n as f64 / frames as f64
            );
            assert!(mbs < limit, "{name}: {mbs:.1} MB/s");
            let _ = fs::remove_dir_all(&dir);
        }
    }

    /// B86: the tight rectangles the export asks the weather for must cover every
    /// pixel the overlay really paints, in every weather, or a drop would leave a
    /// trail. Records each painted pixel and checks it lies in a rectangle.
    #[test]
    fn overlay_rects_cover_every_painted_pixel() {
        struct Rec {
            px: Vec<(i32, i32)>,
            w: i32,
            h: i32,
        }
        impl weather_fx::Surface for Rec {
            fn world_bounds(&self) -> Rect {
                Rect::new(0, 0, self.w, self.h)
            }
            fn put(&mut self, x: i32, y: i32, _c: render::Color) {
                self.px.push((x, y));
            }
            fn put_blended(&mut self, x: i32, y: i32, _c: render::Color, _cov: f32) {
                self.px.push((x, y));
            }
        }
        let (w, h) = (900i32, 500i32);
        for name in ["drizzle", "rain", "storm", "snow", "sandstorm"] {
            let wx = weather_fx::look::preset(name).unwrap();
            let mut fx = weather_fx::Fx::new(0xC17E_15E7, w as usize, h as usize);
            let mut rects = Vec::new();
            for frame in 0..200 {
                fx.update(1.0 / 30.0, &wx);
                if frame % 7 != 0 {
                    continue;
                }
                fx.overlay_rects_fine(&mut rects, STREAK_SLICE_ROWS);
                let mut rec = Rec { px: Vec::new(), w, h };
                fx.draw_overlay(&mut rec, 0, 0);
                for &(x, y) in &rec.px {
                    if x < 0 || y < 0 || x >= w || y >= h {
                        continue;
                    }
                    assert!(
                        rects.iter().any(|r| x >= r.x0 && x < r.x1 && y >= r.y0 && y < r.y1),
                        "{name} frame {frame}: pixel ({x},{y}) is painted outside every rectangle"
                    );
                }
            }
        }
    }
}
