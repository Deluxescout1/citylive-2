//! Slicing and nearest-neighbour integer upscaling from the union framebuffer
//! into the X11 union window buffer.

/// Slice and nearest-neighbour upscale a monitor's portion of the union framebuffer
/// into the corresponding rectangle in the full-window pixel buffer.
///
/// * `src_fb`: entire union framebuffer in `0xAARRGGBB` (or 24/32-bit XRGB), size `fb_w * fb_h`
/// * `fb_w`, `fb_h`: dimensions of the union framebuffer
/// * `scale`: integer upscale factor chosen from `map::pick_scale` on the tallest monitor
/// * `mon_x`, `mon_y`: top-left position of this monitor relative to the union bounding box
/// * `mon_w`, `mon_h`: dimensions of this monitor in pixels
/// * `win_w`: full width of the union window buffer
/// * `win_px`: pixel buffer of the full union window (size at least `win_w * win_h`)
pub fn slice_monitor_into_window(
    src_fb: &[u32],
    fb_w: usize,
    fb_h: usize,
    scale: u32,
    mon_x: usize,
    mon_y: usize,
    mon_w: usize,
    mon_h: usize,
    win_w: usize,
    win_px: &mut [u32],
) {
    let scale = scale.max(1) as usize;
    if mon_w == 0 || mon_h == 0 || fb_w == 0 || fb_h == 0 || win_w == 0 {
        return;
    }

    let mut last_fy: Option<usize> = None;

    for dy in 0..mon_h {
        let wy = mon_y + dy;
        let fy = (wy / scale).clamp(0, fb_h - 1);
        let dst_row_start = wy * win_w + mon_x;

        if Some(fy) == last_fy {
            // Rows with the same source row are identical; fast memory copy.
            let prev_start = (wy - 1) * win_w + mon_x;
            win_px.copy_within(prev_start..prev_start + mon_w, dst_row_start);
        } else {
            last_fy = Some(fy);
            let src_row = &src_fb[fy * fb_w..(fy + 1) * fb_w];
            let dst_row = &mut win_px[dst_row_start..dst_row_start + mon_w];

            for dx in 0..mon_w {
                let wx = mon_x + dx;
                let fx = (wx / scale).clamp(0, fb_w - 1);
                dst_row[dx] = src_row[fx];
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_monitor_2x() {
        // 4x2 FB upscaled 2x into 8x4 window
        let fb_w = 4;
        let fb_h = 2;
        let src_fb: Vec<u32> = (0..(fb_w * fb_h) as u32).collect();
        let mut win_px = vec![0u32; 8 * 4];

        slice_monitor_into_window(&src_fb, fb_w, fb_h, 2, 0, 0, 8, 4, 8, &mut win_px);

        assert_eq!(win_px[0], 0);
        assert_eq!(win_px[1], 0);
        assert_eq!(win_px[8], 0);
        assert_eq!(win_px[9], 0);

        assert_eq!(win_px[2], 1);
        assert_eq!(win_px[3], 1);
        assert_eq!(win_px[10], 1);
        assert_eq!(win_px[11], 1);
    }

    #[test]
    fn test_dual_monitor_seam() {
        // 3840x1080 union across two 1920x1080 monitors
        let fb_w = 1920;
        let fb_h = 540;
        let scale = 2;

        let mut src_fb = vec![0u32; fb_w * fb_h];
        for x in 0..fb_w {
            src_fb[x] = x as u32;
        }

        let mut win_px = vec![0u32; 3840 * 1080];

        // Monitor 0: at (0, 0)
        slice_monitor_into_window(&src_fb, fb_w, fb_h, scale, 0, 0, 1920, 1080, 3840, &mut win_px);

        // Monitor 1: at (1920, 0)
        slice_monitor_into_window(&src_fb, fb_w, fb_h, scale, 1920, 0, 1920, 1080, 3840, &mut win_px);

        // Seam boundary:
        // x = 1919 (Monitor 0 rightmost pixel)
        assert_eq!(win_px[1919], 959);
        // x = 1920 (Monitor 1 leftmost pixel)
        assert_eq!(win_px[1920], 960);
        // x = 1921 (Monitor 1 second pixel)
        assert_eq!(win_px[1921], 960);
        // x = 1922 (Monitor 1 third pixel)
        assert_eq!(win_px[1922], 961);
    }
}
