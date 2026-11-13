//! Slicing and nearest-neighbour integer upscaling from the union framebuffer
//! into per-monitor output surfaces for GNOME Wayland.

/// Slice and nearest-neighbour upscale the union framebuffer into an output's buffer.
///
/// * `src_fb`: entire union framebuffer in `0xAARRGGBB` (or `Xrgb8888`), size `fb_w * fb_h`
/// * `fb_w`, `fb_h`: dimensions of the union framebuffer
/// * `scale`: integer upscale factor chosen from `map::pick_scale` on the tallest output
/// * `union_min_x`, `union_min_y`: top-left logical coordinates of the union bounding box
/// * `head_x`, `head_y`: logical position of this output
/// * `head_w`, `head_h`: logical size of this output
/// * `head_scale_factor`: Wayland buffer scale factor of this output (usually 1, or 2 for HiDPI)
/// * `dst_px`: output pixel buffer to write into, size `(head_w * head_scale_factor) * (head_h * head_scale_factor)`
#[allow(clippy::too_many_arguments)]
pub fn slice_into_output(
    src_fb: &[u32],
    fb_w: usize,
    fb_h: usize,
    scale: u32,
    union_min_x: i32,
    union_min_y: i32,
    head_x: i32,
    head_y: i32,
    head_w: u32,
    head_h: u32,
    head_scale_factor: i32,
    dst_px: &mut [u32],
) {
    let scale = scale.max(1);
    let head_scale_factor = head_scale_factor.max(1);
    let px_w = (head_w as usize) * (head_scale_factor as usize);
    let px_h = (head_h as usize) * (head_scale_factor as usize);

    if px_w == 0 || px_h == 0 || fb_w == 0 || fb_h == 0 {
        return;
    }
    assert!(
        dst_px.len() >= px_w * px_h,
        "dst_px buffer too small: expected {}, got {}",
        px_w * px_h,
        dst_px.len()
    );
    assert!(
        src_fb.len() >= fb_w * fb_h,
        "src_fb buffer too small: expected {}, got {}",
        fb_w * fb_h,
        src_fb.len()
    );

    let x_offset = head_x - union_min_x;
    let y_offset = head_y - union_min_y;

    let mut last_fy: Option<usize> = None;

    for by in 0..px_h {
        let ly = (by as i32) / head_scale_factor;
        let gy = y_offset + ly;
        let fy = (gy / scale as i32).clamp(0, fb_h as i32 - 1) as usize;

        let dst_row_start = by * px_w;
        let dst_row_end = dst_row_start + px_w;

        if Some(fy) == last_fy {
            // Rows with the same source row are identical; fast memory copy.
            let prev_start = (by - 1) * px_w;
            dst_px.copy_within(prev_start..prev_start + px_w, dst_row_start);
        } else {
            last_fy = Some(fy);
            let src_row = &src_fb[fy * fb_w..(fy + 1) * fb_w];
            let dst_row = &mut dst_px[dst_row_start..dst_row_end];

            for bx in 0..px_w {
                let lx = (bx as i32) / head_scale_factor;
                let gx = x_offset + lx;
                let fx = (gx / scale as i32).clamp(0, fb_w as i32 - 1) as usize;
                dst_row[bx] = src_row[fx];
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_head_2x() {
        // 4x2 FB upscaled 2x into 8x4 buffer
        let fb_w = 4;
        let fb_h = 2;
        let src_fb: Vec<u32> = (0..(fb_w * fb_h) as u32).collect();
        let mut dst = vec![0u32; 8 * 4];

        slice_into_output(
            &src_fb, fb_w, fb_h, 2, 0, 0, 0, 0, 8, 4, 1, &mut dst,
        );

        // Check top-left 2x2 block
        assert_eq!(dst[0], 0);
        assert_eq!(dst[1], 0);
        assert_eq!(dst[8], 0);
        assert_eq!(dst[9], 0);

        // Check next 2x2 block
        assert_eq!(dst[2], 1);
        assert_eq!(dst[3], 1);
        assert_eq!(dst[10], 1);
        assert_eq!(dst[11], 1);
    }
}
