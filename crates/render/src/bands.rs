//! B61e: items bucketed by screen row band.
//!
//! A damage-limited composite draws every layer once per damaged rectangle (a
//! rush-hour frame is ~90 rectangles), and each layer used to walk *its whole
//! list* every time: 900 residents, 300 vehicles, the water tiles, the lamps.
//! A [`Bands`] index is built once when the positions are set, so a rectangle
//! only visits the items on the rows it covers.
//!
//! Order: within one band the items stay in list order, and bands are visited
//! top to bottom. Across bands the order therefore changes when a list is not
//! sorted by `y` — the brief allows exactly that, and it is proved pixel for
//! pixel by `render/tests/bands.rs` (a full frame, drawn with and without the
//! index, must be identical).

use crate::framebuffer::Rect;

/// How tall one band is, in pixels. Roughly a damage cell: fine enough that a
/// band holds tens of items, coarse enough that the band list stays short.
pub const BAND_H: i32 = 64;

/// Items bucketed by screen row.
#[derive(Default, Debug, Clone)]
pub struct Bands {
    bands: Vec<Vec<u32>>,
    /// Bumped whenever the contents change, so a caller can tell "nothing to
    /// draw" from "not built yet".
    count: usize,
}

impl Bands {
    /// B84: the bytes this index holds (band lists + their spare capacity).
    pub fn bytes(&self) -> usize {
        self.bands.capacity() * std::mem::size_of::<Vec<u32>>()
            + self.bands.iter().map(|b| b.capacity() * 4).sum::<usize>()
    }

    /// Rebuild from `(screen y, index)` pairs in list order. `height` is the
    /// framebuffer height (bands cover `0..height`).
    pub fn rebuild<I: IntoIterator<Item = (i32, u32)>>(&mut self, height: i32, items: I) {
        let n = ((height.max(1) as i32 + BAND_H - 1) / BAND_H).max(1) as usize;
        if self.bands.len() != n {
            self.bands.clear();
            self.bands.resize_with(n, Vec::new);
        }
        for b in &mut self.bands {
            b.clear();
        }
        self.count = 0;
        for (y, i) in items {
            let band = (y.max(0) / BAND_H) as usize;
            let band = band.min(n - 1);
            self.bands[band].push(i);
            self.count += 1;
        }
    }

    /// Forget everything (the list it indexed is empty).
    pub fn clear(&mut self) {
        for b in &mut self.bands {
            b.clear();
        }
        self.count = 0;
    }

    /// How many items are indexed.
    pub fn len(&self) -> usize {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// B87: like [`Self::for_each`], for a list whose bands were built in
    /// ascending x order (`rebuild` was fed the items sorted by `(band, x)`):
    /// only the items with `x` in `x0..x1` are visited, found by bisection.
    /// `x_of` gives an item's screen x.
    pub fn for_each_x(
        &self,
        clip: Rect,
        pad: i32,
        pad_x: i32,
        x_of: impl Fn(u32) -> i32,
        mut f: impl FnMut(u32),
    ) {
        if self.bands.is_empty() || clip.is_empty() {
            return;
        }
        let y0 = clip.y0 - pad;
        let y1 = clip.y1 + pad;
        let b0 = (y0.max(0) / BAND_H) as usize;
        let b1 = (y1.max(0) / BAND_H) as usize;
        let last = self.bands.len() - 1;
        let (lo, hi) = (clip.x0 - pad_x, clip.x1 + pad_x);
        for band in &self.bands[b0.min(last)..=b1.min(last)] {
            let start = band.partition_point(|&i| x_of(i) < lo);
            for &i in &band[start..] {
                if x_of(i) >= hi {
                    break;
                }
                f(i);
            }
        }
    }

    /// Visit the items whose band can touch `clip`, grown by `pad` pixels (a
    /// sprite's own height above and below its feet). `f` gets the list index.
    pub fn for_each(&self, clip: Rect, pad: i32, mut f: impl FnMut(u32)) {
        if self.bands.is_empty() || clip.is_empty() {
            return;
        }
        let y0 = clip.y0 - pad;
        let y1 = clip.y1 + pad;
        let b0 = (y0.max(0) / BAND_H) as usize;
        let b1 = (y1.max(0) / BAND_H) as usize;
        let last = self.bands.len() - 1;
        for band in &self.bands[b0.min(last)..=b1.min(last)] {
            for &i in band {
                f(i);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_band_holds_what_falls_in_it_and_the_order_is_the_lists() {
        let mut b = Bands::default();
        // (y, index) in list order, deliberately unsorted in y.
        let items = [(10, 0u32), (200, 1), (70, 2), (15, 3), (1000, 4)];
        b.rebuild(1080, items);
        assert_eq!(b.len(), 5);
        let mut seen = Vec::new();
        b.for_each(Rect::new(0, 0, 10, 1080), 0, |i| seen.push(i));
        // Nothing dropped, bands top to bottom.
        assert_eq!(seen, vec![0, 3, 2, 1, 4]);
    }

    #[test]
    fn a_clip_only_visits_the_bands_it_covers() {
        let mut b = Bands::default();
        b.rebuild(1080, [(10, 0u32), (200, 1), (1000, 2)]);
        let mut seen = Vec::new();
        b.for_each(Rect::new(0, 190, 10, 210), 0, |i| seen.push(i));
        assert_eq!(seen, vec![1]);
    }

    #[test]
    fn the_pad_reaches_into_the_band_below() {
        let mut b = Bands::default();
        // y = 70 is band 1; a sprite 14 px tall there reaches up to row 56.
        b.rebuild(1080, [(70, 7u32)]);
        let mut seen = Vec::new();
        b.for_each(Rect::new(0, 58, 10, 60), 14, |i| seen.push(i));
        assert_eq!(seen, vec![7], "the pad reaches the band the sprite's top is in");
        seen.clear();
        b.for_each(Rect::new(0, 58, 10, 60), 0, |i| seen.push(i));
        assert!(seen.is_empty(), "no pad, no band 1: the item cannot show here");
        // A band is coarse on purpose: a clip ending exactly on the band edge
        // also visits the next band, and the item's own bbox test is what
        // decides inside the draw. That is the trade (a band is 64 rows).
        b.for_each(Rect::new(0, 58, 10, 64), 0, |i| seen.push(i));
        assert_eq!(seen, vec![7]);
    }

    #[test]
    fn a_clip_outside_the_frame_visits_nothing_and_an_empty_index_is_cheap() {
        let mut b = Bands::default();
        b.rebuild(1080, [(10, 0u32)]);
        let mut seen = Vec::new();
        b.for_each(Rect::new(0, 1080, 10, 1200), 0, |i| seen.push(i));
        assert!(seen.is_empty());
        b.clear();
        assert!(b.is_empty());
        b.for_each(Rect::new(0, 0, 10, 1080), 0, |i| seen.push(i));
        assert!(seen.is_empty());
    }
}
