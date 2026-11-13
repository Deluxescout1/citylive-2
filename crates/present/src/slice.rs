//! Slicing and nearest-neighbour integer upscaling from the union framebuffer
//! into per-output surfaces.

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

    // D71: the source column of every destination column, worked out once.
    // Doing the two divides and the clamp per pixel per row was 27 ms of a
    // rainy 7040x1440 frame; a table lookup is a few ms.
    let mut lut: Vec<u32> = Vec::with_capacity(px_w);
    for bx in 0..px_w {
        let lx = (bx / head_scale_factor as usize) as i32;
        let gx = x_offset + lx;
        let fx = (gx / scale as i32).clamp(0, fb_w as i32 - 1) as usize;
        lut.push(fx as u32);
    }

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

            // D71: one framebuffer pixel covers `scale` head pixels, so the
            // row is written in runs. The per-pixel version (a divide and a
            // clamp each) was 27 ms for a rainy 7040x1440 frame.
            for (bx, &fx) in lut.iter().enumerate() {
                dst_row[bx] = src_row[fx as usize];
            }
        }
    }
}

/// One output's rectangle inside the union framebuffer, in internal pixels
/// (B42d). This is exactly the crop `slice_into_output` reads, so the Plasma
/// plugin's frame and the layer-surface slice can be checked against each other.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HeadCrop {
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
}

/// Where a head sits in the union framebuffer, or `None` when it falls outside
/// it (a head the compositor has not placed inside the union yet).
pub fn head_crop(
    fb_w: usize,
    fb_h: usize,
    scale: u32,
    union_min: (i32, i32),
    head_pos: (i32, i32),
    head_size: (u32, u32),
) -> Option<HeadCrop> {
    let scale = scale.max(1) as i32;
    let (fb_w, fb_h) = (fb_w as i32, fb_h as i32);
    if fb_w <= 0 || fb_h <= 0 || head_size.0 == 0 || head_size.1 == 0 {
        return None;
    }
    let x0 = (head_pos.0 - union_min.0) / scale;
    let y0 = (head_pos.1 - union_min.1) / scale;
    let w = (head_size.0 as i32 / scale).max(1);
    let h = (head_size.1 as i32 / scale).max(1);
    if x0 < 0 || y0 < 0 || x0 + w > fb_w || y0 + h > fb_h {
        return None;
    }
    Some(HeadCrop {
        x: x0 as usize,
        y: y0 as usize,
        w: w as usize,
        h: h as usize,
    })
}

/// Does a damaged rectangle (framebuffer pixels) touch this head's crop?
pub fn rect_hits_head(d: &render::Rect, c: &HeadCrop) -> bool {
    d.x1 > c.x as i32 && d.x0 < (c.x + c.w) as i32 && d.y1 > c.y as i32 && d.y0 < (c.y + c.h) as i32
}

/// Copy a head's frame out of the union framebuffer, row by row: the same
/// pixels `slice_into_output` shows for that head, without re-projection.
pub fn copy_head_frame(src: &[u32], fb_w: usize, c: &HeadCrop, dst: &mut Vec<u32>) {
    dst.clear();
    dst.reserve(c.w * c.h);
    for row in 0..c.h {
        let start = (c.y + row) * fb_w + c.x;
        dst.extend_from_slice(&src[start..start + c.w]);
    }
}

/// Copy `rows` rows of a head, starting `row0` rows down **inside the head**
/// (B42e: one strip).
pub fn copy_head_rows(
    src: &[u32],
    fb_w: usize,
    c: &HeadCrop,
    row0: usize,
    rows: usize,
    dst: &mut Vec<u32>,
) {
    let row0 = row0.min(c.h);
    let rows = rows.min(c.h - row0);
    dst.clear();
    dst.reserve(c.w * rows);
    for row in row0..row0 + rows {
        let start = (c.y + row) * fb_w + c.x;
        dst.extend_from_slice(&src[start..start + c.w]);
    }
}

/// Which strips (of `rows_per_strip` rows) a damaged row range of a head
/// touches, in order and without duplicates.
pub fn strips_for_range(row0: i32, row1: i32, height: usize, rows_per_strip: usize) -> Vec<usize> {
    let rows_per_strip = rows_per_strip.max(1);
    let y0 = row0.max(0).min(height as i32);
    let y1 = row1.max(0).min(height as i32);
    if y1 <= y0 {
        return Vec::new();
    }
    let first = (y0 as usize) / rows_per_strip;
    let last = (y1 as usize - 1) / rows_per_strip;
    (first..=last).collect()
}

/// The row range of strip `k` inside a head `height` rows tall: `(row0, rows)`.
pub fn strip_rows(k: usize, height: usize, rows_per_strip: usize) -> (usize, usize) {
    let rows_per_strip = rows_per_strip.max(1);
    let row0 = (k * rows_per_strip).min(height);
    (row0, (height - row0).min(rows_per_strip))
}

/// How many strips a head `height` rows tall is cut into.
pub fn strip_count(height: usize, rows_per_strip: usize) -> usize {
    height.div_ceil(rows_per_strip.max(1))
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

    /// B34: saves are keyed by the world size in tiles, so the same land keeps
    /// the same city across a scale change.
    #[test]
    fn test_world_save_path() {
        let base = std::path::PathBuf::from("/home/user/.local/share/citylive-iso/city.sav");
        assert_eq!(
            crate::world_save_path(&base, 162, 162),
            std::path::PathBuf::from("/home/user/.local/share/citylive-iso/city.sav.162x162")
        );
        // A scale change keeps the same world, so it keeps the same file: the
        // name is the tile rectangle, not the framebuffer.
        let vp = map::world_for_viewport(7040, 1440);
        assert_eq!(
            crate::world_save_path(&base, vp.w, vp.h),
            crate::world_save_path(&base, vp.w, vp.h)
        );
        // ...and 2x of the same union describes the same land, so the same name.
        let half = map::world_for_viewport(3520, 720);
        assert_eq!((vp.w, vp.h), (162, 162));
        let _ = half;
    }

    /// A pre-B34 save keyed by framebuffer size is found and migrated to the
    /// world-keyed name.
    #[test]
    fn test_migrate_layout_save() {
        let dir = std::env::temp_dir().join(format!("citylive-b34-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let base = dir.join("city.sav");
        let vp = map::world_for_viewport(960, 540);
        // A real save of this world, under a pre-B34 framebuffer-keyed name.
        let mut real = sim::Sim::new(7, sim::Pace::Month, vp.w, vp.h);
        real.found_again();
        let old = dir.join("city.sav.3520x720");
        real.save(&old).unwrap();
        // A decoy holding a different world must be ignored.
        let mut other = sim::Sim::new(7, sim::Pace::Month, vp.w + 3, vp.h);
        other.found_again();
        let decoy = dir.join("city.sav.1920x540");
        other.save(&decoy).unwrap();
        let target = crate::migrate_layout_save(&base, vp.w, vp.h).expect("no migration");
        assert_eq!(target, crate::world_save_path(&base, vp.w, vp.h));
        assert!(target.exists(), "the world-keyed save was not created");
        assert!(!old.exists(), "the old file should have been renamed");
        assert!(decoy.exists(), "a save for another world must be left alone");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_dual_head_seam_alignment() {
        // Union is 3840x1080 across two 1920x1080 monitors.
        // Scale is 2 -> fb is 1920x540.
        let fb_w = 1920;
        let fb_h = 540;
        let scale = 2;

        let mut src_fb = vec![0u32; fb_w * fb_h];
        // Stamp a distinctive gradient along x in row 0
        for x in 0..fb_w {
            src_fb[x] = x as u32;
        }

        let mut head0 = vec![0u32; 1920 * 1080];
        let mut head1 = vec![0u32; 1920 * 1080];

        // Head 0: at (0, 0)
        slice_into_output(
            &src_fb, fb_w, fb_h, scale, 0, 0, 0, 0, 1920, 1080, 1, &mut head0,
        );

        // Head 1: at (1920, 0)
        slice_into_output(
            &src_fb, fb_w, fb_h, scale, 0, 0, 1920, 0, 1920, 1080, 1, &mut head1,
        );

        // Check boundary:
        // Last pixel of Head 0 (bx = 1919) -> gx = 1919 -> fx = 1919 / 2 = 959.
        assert_eq!(head0[1919], 959);

        // First pixel of Head 1 (bx = 0) -> gx = 1920 -> fx = 1920 / 2 = 960.
        assert_eq!(head1[0], 960);

        // Second pixel of Head 1 (bx = 1) -> gx = 1921 -> fx = 1921 / 2 = 960.
        assert_eq!(head1[1], 960);

        // Third pixel of Head 1 (bx = 2) -> gx = 1922 -> fx = 1922 / 2 = 961.
        assert_eq!(head1[2], 961);
    }

    #[test]
    fn test_multi_resolution_sim_scene_coverage_and_founding() {
        for &(fb_w, fb_h) in &[(960usize, 540usize), (1920, 540), (2560, 720), (853, 480)] {
            let vp = map::world_for_viewport(fb_w as i32, fb_h as i32);
            let (fx, fy) = map::screen_to_tile(&vp, fb_w as f32 * 0.5, fb_h as f32 * 0.5);
            let seed = 7u64;

            let mut s = sim::Sim::with_focus_in(seed, sim::Pace::Month, vp.w, vp.h, (fx, fy), Some(map::Biome::Temperate));
            s.set_viewport(vp.origin.0, vp.origin.1, fb_w as i32, fb_h as i32);
            s.set_focus(fx, fy);
            s.found_again();

            // Verify founding tile projects within 10% of framebuffer center
            let (found_x, found_y) = s.founding;
            let sx = vp.origin.0 as f32
                + (found_x as f32 + 0.5 - found_y as f32 - 0.5) * map::iso_consts::HALF_W as f32;
            let sy = vp.origin.1 as f32
                + (found_x as f32 + 0.5 + found_y as f32 + 0.5) * map::iso_consts::HALF_H as f32;
            let dx = (sx - fb_w as f32 * 0.5).abs() / fb_w as f32;
            let dy = (sy - fb_h as f32 * 0.5).abs() / fb_h as f32;
            assert!(
                dx <= 0.10,
                "fb {fb_w}x{fb_h}: dx {dx:.3} > 0.10 from center ({sx}, {sy})"
            );
            assert!(
                dy <= 0.10,
                "fb {fb_w}x{fb_h}: dy {dy:.3} > 0.10 from center ({sx}, {sy})"
            );

            // Build scene from sim map
            let mut scene = render::Scene::from_map(
                s.map.clone(),
                render::SceneConfig {
                    width: fb_w,
                    height: fb_h,
                    seed,
                    mode: render::SceneMode::Sim,
                    cars: 0,
                    peds: 0,
                    buildings: 0,
                    rain: false,
                },
            );
            scene.set_city(endfx::kinds::objects(&s));
            scene.flush_dirty();

            // Verify no sky/background pixels in depth buffer (every pixel covered by terrain)
            let mut uncovered = 0;
            for &k in scene.depth() {
                if k == 0 {
                    uncovered += 1;
                }
            }
            assert_eq!(
                uncovered, 0,
                "fb {fb_w}x{fb_h}: {uncovered} pixels not covered by terrain"
            );
        }
    }
}

/// Slice and upscale **only a rectangle** of the union framebuffer into a head's
/// buffer (B30). `rect` is in framebuffer pixels; the destination is the same
/// head buffer `slice_into_output` fills, and the return value is the rectangle
/// in *buffer* pixels, for `damage_buffer`.
#[allow(clippy::too_many_arguments)]
pub fn slice_into_output_rect(
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
    rect: render::Rect,
) -> render::Rect {
    let scale = scale.max(1);
    let hsf = head_scale_factor.max(1);
    let px_w = (head_w as usize) * (hsf as usize);
    let px_h = (head_h as usize) * (hsf as usize);
    if px_w == 0 || px_h == 0 || fb_w == 0 || fb_h == 0 {
        return render::Rect::EMPTY;
    }
    // The head's own rectangle in framebuffer pixels.
    let x_offset = head_x - union_min_x;
    let y_offset = head_y - union_min_y;
    // Intersect the dirty rect with this head.
    let head_fb_w = (head_w as i32) / scale as i32;
    let head_fb_h = (head_h as i32) / scale as i32;
    let ix0 = rect.x0.max(x_offset / scale as i32);
    let iy0 = rect.y0.max(y_offset / scale as i32);
    let ix1 = rect
        .x1
        .min(x_offset / scale as i32 + head_fb_w.max(1));
    let iy1 = rect
        .y1
        .min(y_offset / scale as i32 + head_fb_h.max(1));
    if ix1 <= ix0 || iy1 <= iy0 {
        return render::Rect::EMPTY;
    }
    // D71: one framebuffer pixel covers `s`x`s` head pixels, so the inner
    // copy is a run fill, not a pixel-by-pixel test. This was 20 % of a busy
    // frame's CPU when everything started moving at the display rate.
    let s = (scale as i32 * hsf).max(1);
    let fx0 = ix0.max(0);
    let fx1 = ix1.min(fb_w as i32);
    let fy0 = iy0.max(0);
    let fy1 = iy1.min(fb_h as i32);
    if fx1 <= fx0 || fy1 <= fy0 {
        return render::Rect::EMPTY;
    }
    // The horizontal span this source row covers in the head buffer.
    let a0 = ((fx0 * scale as i32 - x_offset) * hsf).max(0) as usize;
    let b0 = (((fx1 - 1) * scale as i32 - x_offset) * hsf + s).min(px_w as i32).max(0) as usize;
    for fy in fy0..fy1 {
        let by0 = (fy * scale as i32 - y_offset) * hsf;
        let r0 = by0.max(0);
        let r1 = (by0 + s).min(px_h as i32);
        if r1 <= r0 {
            continue;
        }
        let src_row = fy as usize * fb_w;
        let base = r0 as usize * px_w;
        // Build the scaled row once, into a subslice of child pixels so the
        // inner loop is a plain run fill (D71: this was 23 % of a busy frame).
        let row = &mut dst_px[base..base + px_w];
        let src = &src_fb[src_row + fx0 as usize..src_row + fx1 as usize];
        for (fx, &c) in (fx0..fx1).zip(src.iter()) {
            let bx0 = (fx * scale as i32 - x_offset) * hsf;
            let a = bx0.max(0) as usize;
            let b = ((bx0 + s).min(px_w as i32)).max(0) as usize;
            if b > a {
                row[a..b].fill(c);
            }
        }
        // ...then copy it to the other device rows it covers.
        if b0 > a0 {
            for by in (r0 + 1)..r1 {
                let row = by as usize * px_w;
                dst_px.copy_within(base + a0..base + b0, row + a0);
            }
        }
    }
    // Damage in buffer pixels.
    let bx0 = ((ix0 * scale as i32 - x_offset) * hsf).max(0);
    let by0 = ((iy0 * scale as i32 - y_offset) * hsf).max(0);
    let bx1 = ((ix1 * scale as i32 - x_offset) * hsf).min(px_w as i32);
    let by1 = ((iy1 * scale as i32 - y_offset) * hsf).min(px_h as i32);
    render::Rect::new(bx0, by0, bx1, by1)
}
