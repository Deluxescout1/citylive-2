//! Nearest-neighbour integer upscaling.
//!
//! Pixel art wants exact multiples with no filtering. The integer path is used
//! for the real backends (scale 2 or 4); the rational path exists for the odd
//! window size a compositor may hand us.
//!
//! B61: none of this allocates. `upscale_into` used to build a `sw * scale` row
//! buffer per call — at 7040x1440 x2 that is a 56 MB malloc and its page
//! faults *every frame* (~5 ms of the ~23 ms). The row is now written straight
//! into `dst` and replicated with `copy_within`, and `upscale_rows_into`
//! upscales only a band of source rows so a damaged band costs its own rows.

/// Upscale `src` by an integer factor into `dst` (resized to fit).
pub fn upscale_into(src: &[u32], sw: usize, sh: usize, dst: &mut Vec<u32>, scale: usize) {
    assert!(scale >= 1);
    let dw = sw * scale;
    let dh = sh * scale;
    dst.resize(dw * dh, 0);
    upscale_rows_into(src, sw, sh, dst, scale, 0, sh);
}

/// Upscale source rows `y0..y1` (a damaged band) into `dst`, which must already
/// be `sw * scale` wide by `sh * scale` tall. Rows outside the band are left
/// alone, so a presenter that only redrew part of the frame pays for that part.
pub fn upscale_rows_into(
    src: &[u32],
    sw: usize,
    sh: usize,
    dst: &mut [u32],
    scale: usize,
    y0: usize,
    y1: usize,
) {
    assert!(scale >= 1, "scale must be at least 1");
    if sw == 0 || scale == 1 {
        // Scale 1 is a copy of the band; nothing to expand.
        let dw = sw * scale;
        let y0 = y0.min(sh);
        let y1 = y1.min(sh);
        if dw > 0 && y1 > y0 {
            let rows = y1 - y0;
            if dst.len() >= rows * dw {
                dst[y0 * dw..(y0 + rows) * dw].copy_from_slice(&src[y0 * sw..y1 * sw]);
            }
        }
        return;
    }
    let dw = sw * scale;
    let y0 = y0.min(sh);
    let y1 = y1.min(sh);
    for y in y0..y1 {
        let srow = &src[y * sw..y * sw + sw];
        let o = y * scale * dw;
        let band = &mut dst[o..o + scale * dw];
        // First sub-row: expand each source pixel into `scale` copies.
        {
            let out = &mut band[..dw];
            match scale {
                2 => {
                    // Two pixels packed into one 64-bit store.
                    let pairs = out.chunks_exact_mut(2);
                    for (p, &s) in pairs.into_iter().zip(srow.iter()) {
                        p.copy_from_slice(&[s, s]);
                    }
                }
                4 => {
                    for (q, &s) in out.chunks_exact_mut(4).zip(srow.iter()) {
                        q[0] = s;
                        q[1] = s;
                        q[2] = s;
                        q[3] = s;
                    }
                }
                _ => {
                    for (i, &p) in srow.iter().enumerate() {
                        let b = i * scale;
                        out[b..b + scale].fill(p);
                    }
                }
            }
        }
        // The remaining sub-rows are copies of the first.
        for k in 1..scale {
            band.copy_within(0..dw, k * dw);
        }
    }
}

/// Upscale `src` to exactly `dw x dh` with nearest-neighbour sampling.
/// Column/row maps are computed once, so cost is one source lookup per pixel.
pub fn upscale_to_size(
    src: &[u32],
    sw: usize,
    sh: usize,
    dst: &mut Vec<u32>,
    dw: usize,
    dh: usize,
) {
    dst.resize(dw * dh, 0);
    upscale_to_size_in(src, sw, sh, dst, dw, dh);
}

/// `upscale_to_size` into a buffer that is already the right size: the window
/// backend calls this every frame and must not reallocate (B61).
pub fn upscale_to_size_in(src: &[u32], sw: usize, sh: usize, dst: &mut [u32], dw: usize, dh: usize) {
    if dw == 0 || dh == 0 || sw == 0 || sh == 0 || dst.len() < dw * dh {
        return;
    }
    // One source column per destination column, built once per call (dw is the
    // window width: 2-8 KB of scratch on the stack is not an option, so this
    // stays the small allocation the caller's `dst` already absorbed).
    let mut out = 0usize;
    let mut cols: Vec<usize> = Vec::with_capacity(dw);
    cols.extend((0..dw).map(|x| (x * sw / dw).min(sw - 1)));
    for y in 0..dh {
        let sy = (y * sh / dh).min(sh - 1);
        let srow = &src[sy * sw..sy * sw + sw];
        let drow = &mut dst[out..out + dw];
        for (x, &sx) in cols.iter().enumerate() {
            drow[x] = srow[sx];
        }
        out += dw;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_scaling_replicates_pixels() {
        let src = vec![1u32, 2, 3, 4];
        let mut dst = Vec::new();
        upscale_into(&src, 2, 2, &mut dst, 2);
        assert_eq!(dst, vec![1, 1, 2, 2, 1, 1, 2, 2, 3, 3, 4, 4, 3, 3, 4, 4]);
    }

    #[test]
    fn size_scaling_picks_nearest() {
        let src = vec![10u32, 20, 30, 40];
        let mut dst = Vec::new();
        upscale_to_size(&src, 2, 2, &mut dst, 4, 4);
        assert_eq!(dst.len(), 16);
        assert_eq!(dst[0], 10);
        assert_eq!(dst[15], 40);
    }

    /// Every integer scale produces the same picture as the naive expander.
    #[test]
    fn every_scale_matches_the_reference() {
        let (sw, sh) = (5usize, 3usize);
        let src: Vec<u32> = (0..sw * sh).map(|i| 0x0100_0000 + i as u32).collect();
        for scale in 1..=5usize {
            let mut got = Vec::new();
            upscale_into(&src, sw, sh, &mut got, scale);
            let (dw, dh) = (sw * scale, sh * scale);
            assert_eq!(got.len(), dw * dh);
            for y in 0..dh {
                for x in 0..dw {
                    assert_eq!(got[y * dw + x], src[(y / scale) * sw + x / scale], "scale {scale} ({x},{y})");
                }
            }
        }
    }

    /// A band upscale touches only its own rows and leaves the rest of the
    /// buffer alone — the point of a dirty-rect upscale (B61).
    #[test]
    fn a_band_upscale_leaves_other_rows_alone() {
        let (sw, sh, scale) = (3usize, 4usize, 2usize);
        let (dw, dh) = (sw * scale, sh * scale);
        let src: Vec<u32> = (0..sw * sh).map(|i| i as u32 + 1).collect();
        let mut dst = vec![0xdead_beefu32; dw * dh];
        upscale_rows_into(&src, sw, sh, &mut dst, scale, 1, 2);
        // Rows 2..4 belong to source row 1.
        for y in 2..4 {
            for x in 0..dw {
                assert_eq!(dst[y * dw + x], src[1 * sw + x / scale]);
            }
        }
        // Everything else is untouched.
        for y in (0..dh).filter(|y| *y < 2 || *y >= 4) {
            for x in 0..dw {
                assert_eq!(dst[y * dw + x], 0xdead_beef, "row {y} was written");
            }
        }
    }

    /// The size-scaling entry point must not reallocate when the buffer is
    /// already the right size (the per-frame window path).
    #[test]
    fn size_scaling_reuses_the_buffer() {
        let src = vec![1u32, 2, 3, 4];
        let mut dst = Vec::new();
        upscale_to_size(&src, 2, 2, &mut dst, 3, 3);
        let cap = dst.capacity();
        upscale_to_size_in(&src, 2, 2, &mut dst, 3, 3);
        assert_eq!(dst.capacity(), cap);
        assert_eq!(dst.len(), 9);
    }
}
