//! The timelapse (B51/D67): a tiny picture of the city for every sim day.
//!
//! Once per sim day the wallpaper hands over the city's static layer (terrain
//! and buildings, ARGB pixels); it is boxed down by [`FACTOR`], snapped to a
//! fixed 256-colour palette and appended to a file beside the save as a delta
//! against the day before (most of a city does not change in a day, so a delta
//! is a few hundred bytes). When the world ends, [`assemble_gif`] turns the
//! file into an animated GIF.
//!
//! No dependencies, no per-frame cost: [`Recorder::record`] is a few ms, once
//! a day. The file is capped at [`BUDGET`] bytes per world.

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

/// The downscale of the static layer.
pub const FACTOR: usize = 8;

/// The most a world's timelapse file may take.
pub const BUDGET: u64 = 5 * 1024 * 1024;

/// The sky colour shown where the static layer is transparent.
const SKY: u32 = 0xFF9CC4E4;

const MAGIC: &[u8; 4] = b"CLTL";

/// The fixed palette: a 6x6x6 colour cube (0..216) and a 40-step grey ramp.
pub fn palette() -> [[u8; 3]; 256] {
    let mut p = [[0u8; 3]; 256];
    for i in 0..216usize {
        let (r, g, b) = (i / 36, (i / 6) % 6, i % 6);
        p[i] = [(r * 51) as u8, (g * 51) as u8, (b * 51) as u8];
    }
    for i in 0..40usize {
        let v = (i * 255 / 39) as u8;
        p[216 + i] = [v, v, v];
    }
    p
}

fn quantize(r: u8, g: u8, b: u8) -> u8 {
    let mx = r.max(g).max(b) as i32;
    let mn = r.min(g).min(b) as i32;
    if mx - mn < 10 {
        let v = (r as u32 + g as u32 + b as u32) / 3;
        return 216 + ((v * 39 + 127) / 255) as u8;
    }
    let q = |v: u8| ((v as u32 * 5 + 127) / 255) as usize;
    (q(r) * 36 + q(g) * 6 + q(b)) as u8
}

/// Box a w x h ARGB image down by [`FACTOR`] to palette indices. Transparent
/// pixels show the sky. Returns (width, height, indices).
pub fn downscale(px: &[u32], w: usize, h: usize) -> (usize, usize, Vec<u8>) {
    downscale_from(px, w, h, 0)
}

/// B84: the same, but rows above `top` are never read. The static layer's top
/// rows are the sky band and hold nothing (transparent), and a transparent
/// pixel is already drawn as [`SKY`] here — so skipping them changes no byte
/// and keeps those pages out of the process's resident set. Without it, one
/// timelapse frame a day touched all ten million pixels of the layer.
pub fn downscale_from(px: &[u32], w: usize, h: usize, top: usize) -> (usize, usize, Vec<u8>) {
    let (ow, oh) = (w / FACTOR, h / FACTOR);
    let mut out = vec![0u8; ow * oh];
    for oy in 0..oh {
        for ox in 0..ow {
            let (mut r, mut g, mut b) = (0u32, 0u32, 0u32);
            for dy in 0..FACTOR {
                let y = oy * FACTOR + dy;
                if y < top {
                    // Sky band: nothing was ever drawn here.
                    r += (SKY >> 16) & 255;
                    g += (SKY >> 8) & 255;
                    b += SKY & 255;
                    continue;
                }
                let row = y * w + ox * FACTOR;
                for dx in 0..FACTOR {
                    let c = px[row + dx];
                    let c = if c >> 24 == 0 { SKY } else { c };
                    r += (c >> 16) & 255;
                    g += (c >> 8) & 255;
                    b += c & 255;
                }
            }
            let n = (FACTOR * FACTOR) as u32;
            out[oy * ow + ox] = quantize((r / n) as u8, (g / n) as u8, (b / n) as u8);
        }
    }
    (ow, oh, out)
}

fn put_varint(out: &mut Vec<u8>, mut v: usize) {
    while v >= 0x80 {
        out.push((v & 0x7f) as u8 | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn get_varint(b: &[u8], at: &mut usize) -> Option<usize> {
    let mut v = 0usize;
    let mut shift = 0;
    loop {
        let byte = *b.get(*at)?;
        *at += 1;
        v |= ((byte & 0x7f) as usize) << shift;
        if byte & 0x80 == 0 {
            return Some(v);
        }
        shift += 7;
        if shift > 35 {
            return None;
        }
    }
}

/// Encode `cur` as a delta against `prev`: (zero-run, literal-count, literals)*.
fn encode_delta(prev: &[u8], cur: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let n = cur.len();
    let mut i = 0;
    while i < n {
        let z = i;
        while i < n && prev[i] == cur[i] {
            i += 1;
        }
        let zero = i - z;
        if i == n {
            break;
        }
        let l = i;
        // A literal run ends at a gap of 4+ unchanged pixels.
        let mut same = 0;
        while i < n {
            if prev[i] == cur[i] {
                same += 1;
            } else {
                same = 0;
            }
            i += 1;
            if same >= 4 {
                break;
            }
        }
        i -= same;
        put_varint(&mut out, zero);
        put_varint(&mut out, i - l);
        for k in l..i {
            out.push(prev[k] ^ cur[k]);
        }
    }
    out
}

fn apply_delta(prev: &mut [u8], d: &[u8]) -> Option<()> {
    let mut at = 0usize;
    let mut pos = 0usize;
    while at < d.len() {
        let zero = get_varint(d, &mut at)?;
        let lit = get_varint(d, &mut at)?;
        pos = pos.checked_add(zero)?;
        if pos + lit > prev.len() || at + lit > d.len() {
            return None;
        }
        for k in 0..lit {
            prev[pos + k] ^= d[at + k];
        }
        at += lit;
        pos += lit;
    }
    Some(())
}

/// The frames of a timelapse file, decoded.
pub struct Frames {
    pub w: usize,
    pub h: usize,
    /// (sim day, palette indices).
    pub frames: Vec<(u32, Vec<u8>)>,
}

/// Read a timelapse file. A torn last frame (a crash mid-write) is dropped.
pub fn read_file(path: &Path) -> io::Result<Frames> {
    let mut f = std::fs::File::open(path)?;
    let mut b = Vec::new();
    f.read_to_end(&mut b)?;
    if b.len() < 9 || &b[..4] != MAGIC {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "not a timelapse"));
    }
    let w = u16::from_le_bytes([b[4], b[5]]) as usize;
    let h = u16::from_le_bytes([b[6], b[7]]) as usize;
    let mut cur = vec![0u8; w * h];
    let mut frames = Vec::new();
    let mut at = 9;
    while at + 8 <= b.len() {
        let day = u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]);
        let len = u32::from_le_bytes([b[at + 4], b[at + 5], b[at + 6], b[at + 7]]) as usize;
        at += 8;
        if at + len > b.len() {
            break;
        }
        if apply_delta(&mut cur, &b[at..at + len]).is_none() {
            break;
        }
        at += len;
        frames.push((day, cur.clone()));
    }
    Ok(Frames { w, h, frames })
}

/// Appends one frame a day to a world's timelapse file.
pub struct Recorder {
    path: PathBuf,
    w: usize,
    h: usize,
    prev: Vec<u8>,
    last_day: Option<u32>,
    size: u64,
}

/// What `record` did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Recorded {
    /// A frame was appended (its delta size in bytes).
    Added(usize),
    /// This day is already recorded.
    Duplicate,
    /// The budget is spent.
    Full,
}

impl Recorder {
    /// Open (or begin) the file at `path`.
    pub fn open(path: &Path) -> Recorder {
        let mut r = Recorder {
            path: path.to_path_buf(),
            w: 0,
            h: 0,
            prev: Vec::new(),
            last_day: None,
            size: 0,
        };
        if let Ok(fr) = read_file(path) {
            r.w = fr.w;
            r.h = fr.h;
            if let Some((d, px)) = fr.frames.last() {
                r.last_day = Some(*d);
                r.prev = px.clone();
            } else {
                r.prev = vec![0; fr.w * fr.h];
            }
            r.size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        }
        r
    }

    pub fn last_day(&self) -> Option<u32> {
        self.last_day
    }

    pub fn size(&self) -> u64 {
        self.size
    }

    /// Record the city's static layer (`w` x `h` ARGB) for sim day `day`.
    pub fn record(&mut self, day: u32, px: &[u32], w: usize, h: usize) -> io::Result<Recorded> {
        self.record_force(day, px, w, h, false)
    }

    /// B84: as [`Self::record`], but the layer's rows above `top` are the sky
    /// band and are not read (see [`downscale_from`]).
    pub fn record_from(
        &mut self,
        day: u32,
        px: &[u32],
        w: usize,
        h: usize,
        top: usize,
    ) -> io::Result<Recorded> {
        self.record_force_from(day, px, w, h, false, top)
    }

    /// B84: [`Self::record_final`] for a layer whose first `top` rows are sky.
    pub fn record_final_from(
        &mut self,
        day: u32,
        px: &[u32],
        w: usize,
        h: usize,
        top: usize,
    ) -> io::Result<Recorded> {
        self.record_force_from(day, px, w, h, true, top)
    }

    /// Record a frame even if the day is already in the file.
    ///
    /// B65: the world's last frame is taken at the end of the ending's
    /// aftermath, which often falls inside a day already recorded; and the
    /// timelapse stops there, before the land starts turning into the next
    /// biome, so the story ends with the ruins and not with the next world.
    pub fn record_final(&mut self, day: u32, px: &[u32], w: usize, h: usize) -> io::Result<Recorded> {
        self.record_force(day, px, w, h, true)
    }

    fn record_force(
        &mut self,
        day: u32,
        px: &[u32],
        w: usize,
        h: usize,
        force: bool,
    ) -> io::Result<Recorded> {
        self.record_force_from(day, px, w, h, force, 0)
    }

    fn record_force_from(
        &mut self,
        day: u32,
        px: &[u32],
        w: usize,
        h: usize,
        force: bool,
        top: usize,
    ) -> io::Result<Recorded> {
        if !force && self.last_day.is_some_and(|d| d >= day) {
            return Ok(Recorded::Duplicate);
        }
        let (ow, oh, idx) = downscale_from(px, w, h, top);
        if ow == 0 || oh == 0 {
            return Ok(Recorded::Duplicate);
        }
        let fresh = self.size == 0 || self.w != ow || self.h != oh;
        let delta;
        if fresh {
            delta = encode_delta(&vec![0u8; ow * oh], &idx);
        } else {
            delta = encode_delta(&self.prev, &idx);
        }
        let add = delta.len() as u64 + 8 + if fresh { 9 } else { 0 };
        if !fresh && self.size + add > BUDGET {
            return Ok(Recorded::Full);
        }
        let mut file = if fresh {
            // A new file (or the screen layout changed: start the story over).
            let mut f = std::fs::File::create(&self.path)?;
            f.write_all(MAGIC)?;
            f.write_all(&(ow as u16).to_le_bytes())?;
            f.write_all(&(oh as u16).to_le_bytes())?;
            f.write_all(&[FACTOR as u8])?;
            self.size = 9;
            f
        } else {
            std::fs::OpenOptions::new().append(true).open(&self.path)?
        };
        let mut rec = Vec::with_capacity(delta.len() + 8);
        rec.extend_from_slice(&day.to_le_bytes());
        rec.extend_from_slice(&(delta.len() as u32).to_le_bytes());
        rec.extend_from_slice(&delta);
        file.write_all(&rec)?;
        self.size += rec.len() as u64;
        self.w = ow;
        self.h = oh;
        self.prev = idx;
        self.last_day = Some(day);
        Ok(Recorded::Added(delta.len()))
    }
}

// ---------------------------------------------------------------------------
// GIF
// ---------------------------------------------------------------------------

struct BitWriter {
    out: Vec<u8>,
    acc: u32,
    nbits: u32,
}

impl BitWriter {
    fn put(&mut self, code: u32, size: u32) {
        self.acc |= code << self.nbits;
        self.nbits += size;
        while self.nbits >= 8 {
            self.out.push((self.acc & 255) as u8);
            self.acc >>= 8;
            self.nbits -= 8;
        }
    }
    fn finish(&mut self) {
        if self.nbits > 0 {
            self.out.push((self.acc & 255) as u8);
            self.acc = 0;
            self.nbits = 0;
        }
    }
}

/// GIF-flavoured LZW of 8-bit indices (minimum code size 8).
fn lzw(data: &[u8]) -> Vec<u8> {
    const CLEAR: u32 = 256;
    const EOI: u32 = 257;
    let mut bw = BitWriter { out: Vec::new(), acc: 0, nbits: 0 };
    let mut table: std::collections::HashMap<(u32, u8), u32> = std::collections::HashMap::new();
    let mut next = 258u32;
    let mut size = 9u32;
    bw.put(CLEAR, size);
    let mut cur: Option<u32> = None;
    for &b in data {
        match cur {
            None => cur = Some(b as u32),
            Some(c) => {
                if let Some(&code) = table.get(&(c, b)) {
                    cur = Some(code);
                } else {
                    bw.put(c, size);
                    table.insert((c, b), next);
                    next += 1;
                    if next > (1 << size) && size < 12 {
                        size += 1;
                    }
                    if next >= 4096 {
                        bw.put(CLEAR, size);
                        table.clear();
                        next = 258;
                        size = 9;
                    }
                    cur = Some(b as u32);
                }
            }
        }
    }
    if let Some(c) = cur {
        bw.put(c, size);
    }
    bw.put(EOI, size);
    bw.finish();
    bw.out
}

fn sub_blocks(out: &mut Vec<u8>, data: &[u8]) {
    for chunk in data.chunks(255) {
        out.push(chunk.len() as u8);
        out.extend_from_slice(chunk);
    }
    out.push(0);
}

/// Encode frames (palette indices, `w` x `h`) as a looping GIF, each upscaled
/// `scale` times, `delay_cs` hundredths of a second apiece and the last held
/// for `hold_cs`.
pub fn encode_gif(frames: &Frames, scale: usize, delay_cs: u16, hold_cs: u16) -> Vec<u8> {
    let scale = scale.max(1);
    let (w, h) = (frames.w * scale, frames.h * scale);
    let mut out = Vec::new();
    out.extend_from_slice(b"GIF89a");
    out.extend_from_slice(&(w as u16).to_le_bytes());
    out.extend_from_slice(&(h as u16).to_le_bytes());
    out.extend_from_slice(&[0xF7, 0, 0]);
    for c in palette() {
        out.extend_from_slice(&c);
    }
    out.extend_from_slice(&[0x21, 0xFF, 0x0B]);
    out.extend_from_slice(b"NETSCAPE2.0");
    out.extend_from_slice(&[3, 1, 0, 0, 0]);
    let n = frames.frames.len();
    for (i, (_, idx)) in frames.frames.iter().enumerate() {
        let delay = if i + 1 == n { hold_cs } else { delay_cs };
        out.extend_from_slice(&[0x21, 0xF9, 4, 0x00]);
        out.extend_from_slice(&delay.to_le_bytes());
        out.extend_from_slice(&[0, 0]);
        out.push(0x2C);
        out.extend_from_slice(&[0, 0, 0, 0]);
        out.extend_from_slice(&(w as u16).to_le_bytes());
        out.extend_from_slice(&(h as u16).to_le_bytes());
        out.push(0);
        out.push(8);
        let mut big = Vec::with_capacity(w * h);
        for y in 0..h {
            let row = &idx[(y / scale) * frames.w..(y / scale + 1) * frames.w];
            for &p in row {
                for _ in 0..scale {
                    big.push(p);
                }
            }
        }
        sub_blocks(&mut out, &lzw(&big));
    }
    out.push(0x3B);
    out
}

/// Turn a timelapse file into a GIF at `gif_path`. Returns the number of
/// frames written.
pub fn assemble_gif(tl_path: &Path, gif_path: &Path) -> io::Result<usize> {
    let frames = read_file(tl_path)?;
    if frames.frames.is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "no frames"));
    }
    let bytes = encode_gif(&frames, 2, 12, 300);
    std::fs::write(gif_path, bytes)?;
    Ok(frames.frames.len())
}

/// The name of the timelapse file for a save path (`city.sav.WxH` ->
/// `city.sav.WxH.timelapse`).
pub fn path_for(save_path: &Path) -> PathBuf {
    let mut s = save_path.as_os_str().to_os_string();
    s.push(".timelapse");
    PathBuf::from(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(w: usize, h: usize, seed: u32, spots: u32) -> Vec<u32> {
        let mut v = vec![0u32; w * h];
        for y in 0..h {
            for x in 0..w {
                // land with a bit of texture, a sky strip on top
                v[y * w + x] = if y < 16 {
                    0
                } else {
                    0xFF000000 | (60 + ((x / 16) as u32 * 7) % 80) << 16 | (120 + ((y / 16) as u32 * 5) % 60) << 8 | 70
                };
            }
        }
        for s in 0..spots {
            let x = ((seed + s) * 37) as usize % (w - 24);
            let y = 24 + ((seed + s) * 53) as usize % (h - 40);
            for dy in 0..16 {
                for dx in 0..16 {
                    v[(y + dy) * w + x + dx] = 0xFFC8B090;
                }
            }
        }
        v
    }

    fn tmp(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("citylive-b51-{tag}.timelapse"))
    }

    #[test]
    fn delta_round_trips() {
        let a: Vec<u8> = (0..500).map(|i| (i * 7 % 251) as u8).collect();
        let mut b = a.clone();
        for i in [3usize, 4, 5, 90, 91, 400, 499] {
            b[i] ^= 0x55;
        }
        let d = encode_delta(&a, &b);
        let mut c = a.clone();
        apply_delta(&mut c, &d).unwrap();
        assert_eq!(c, b);
        assert!(d.len() < 40, "{}", d.len());
        // identical frames are empty
        assert!(encode_delta(&b, &b).is_empty());
    }

    #[test]
    fn frames_record_resume_and_decode() {
        let p = tmp("rec");
        let _ = std::fs::remove_file(&p);
        let (w, h) = (640, 320);
        let mut r = Recorder::open(&p);
        for day in 0..10u32 {
            let px = img(w, h, 1, day);
            assert!(matches!(r.record(day, &px, w, h).unwrap(), Recorded::Added(_)));
            assert_eq!(r.record(day, &px, w, h).unwrap(), Recorded::Duplicate);
        }
        // A restart picks up where it left off.
        let mut r2 = Recorder::open(&p);
        assert_eq!(r2.last_day(), Some(9));
        r2.record(10, &img(w, h, 1, 10), w, h).unwrap();
        let fr = read_file(&p).unwrap();
        assert_eq!(fr.frames.len(), 11);
        assert_eq!((fr.w, fr.h), (80, 40));
        let (_, want) = (0, downscale(&img(w, h, 1, 4), w, h).2);
        assert_eq!(fr.frames[4].1, want, "frame 4 decodes exactly");
        // deterministic: the same inputs give the same bytes
        let q = tmp("rec2");
        let _ = std::fs::remove_file(&q);
        let mut r3 = Recorder::open(&q);
        for day in 0..11u32 {
            r3.record(day, &img(w, h, 1, day), w, h).unwrap();
        }
        assert_eq!(std::fs::read(&p).unwrap(), std::fs::read(&q).unwrap());
    }

    #[test]
    fn size_budget_holds() {
        let p = tmp("budget");
        let _ = std::fs::remove_file(&p);
        // A real-size layer, a hundred days: far inside the budget; and the
        // cap itself refuses to be passed.
        let (w, h) = (2400, 1000);
        let mut r = Recorder::open(&p);
        for day in 0..100u32 {
            r.record(day, &img(w, h, 3, day % 40), w, h).unwrap();
        }
        assert!(r.size() < BUDGET, "{} bytes for 100 days", r.size());
        r.size = BUDGET - 3;
        assert_eq!(r.record(500, &img(w, h, 9, 5), w, h).unwrap(), Recorded::Full);
    }

    #[test]
    fn a_final_frame_lands_even_on_a_day_already_recorded() {
        let p = tmp("final");
        let _ = std::fs::remove_file(&p);
        let (w, h) = (320, 160);
        let mut r = Recorder::open(&p);
        r.record(7, &img(w, h, 1, 1), w, h).unwrap();
        // The end of an ending often falls inside a day already in the file.
        assert_eq!(r.record(7, &img(w, h, 1, 5), w, h).unwrap(), Recorded::Duplicate);
        assert!(matches!(r.record_final(7, &img(w, h, 1, 5), w, h).unwrap(), Recorded::Added(_)));
        let fr = read_file(&p).unwrap();
        assert_eq!(fr.frames.len(), 2);
        assert_eq!(fr.frames[1].0, 7);
        assert_eq!(fr.frames[1].1, downscale(&img(w, h, 1, 5), w, h).2, "the last frame is the one asked for");
    }

    #[test]
    fn gif_is_well_formed() {
        let p = tmp("gif");
        let _ = std::fs::remove_file(&p);
        let (w, h) = (320, 160);
        let mut r = Recorder::open(&p);
        for day in 0..4u32 {
            r.record(day, &img(w, h, 2, day), w, h).unwrap();
        }
        let g = std::env::temp_dir().join("citylive-b51.gif");
        assert_eq!(assemble_gif(&p, &g).unwrap(), 4);
        let b = std::fs::read(&g).unwrap();
        assert_eq!(&b[..6], b"GIF89a");
        assert_eq!(*b.last().unwrap(), 0x3B);
        assert_eq!(u16::from_le_bytes([b[6], b[7]]), 80);
    }
}
