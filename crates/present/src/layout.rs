//! Display layout: which outputs form the wallpaper, how they union into one
//! framebuffer, and where the village is founded (B17 items 2–4).
//!
//! Pure functions over output descriptions, so the awkward cases (a streaming
//! output parked below the monitors, mixed heights, an ultrawide union) can be
//! tested without a compositor.

/// One output as the compositor reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputSpec {
    pub name: String,
    pub pos: (i32, i32),
    pub size: (u32, u32),
}

impl OutputSpec {
    pub fn new(name: &str, x: i32, y: i32, w: u32, h: u32) -> Self {
        Self {
            name: name.to_string(),
            pos: (x, y),
            size: (w, h),
        }
    }
}

/// A virtual / headless output (Sunshine, xvfb, VNC…) is not a real monitor.
pub fn is_virtual(name: &str) -> bool {
    name.starts_with("Virtual") || name.starts_with("HEADLESS") || name.starts_with("headless")
}

/// Should this output be part of the wallpaper? An explicit `--include-output`
/// always wins; `--ignore-output` and the virtual-name rule drop the rest.
pub fn included(name: &str, include: &[String], ignore: &[String]) -> bool {
    if include.iter().any(|n| n == name) {
        return true;
    }
    if ignore.iter().any(|n| n == name) {
        return false;
    }
    !is_virtual(name)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    pub min_x: i32,
    pub min_y: i32,
    pub union_w: u32,
    pub union_h: u32,
    pub scale: u32,
    pub fb_w: usize,
    pub fb_h: usize,
    /// Union-logical centre of the founding monitor (see [`founding_point`]).
    pub focus: (i32, i32),
    /// Index (into the included list) of the monitor the village starts on.
    pub focus_index: usize,
}

/// Union of the included outputs, the integer scale (D61), and the monitor the
/// village is founded on.
///
/// D61: the scale comes from [`map::pick_scale`] — `max(1, round(tallest /
/// 1440))`, the one rule the spike, the X11 backend, the GNOME backend and this
/// wallpaper all use — or the `--scale` override. Before B34 this file still
/// carried D42's "≈720 rows" rule, so on the owner's 1440p desk the wallpaper picked
/// 2× and doubled everything on screen. The founding monitor is the **largest**
/// one when the union is very wide (aspect > 3:1) — on a 3-monitor desk plus a
/// virtual output the union centre is nowhere near a real screen — otherwise the
/// union centre.
pub fn layout(
    outputs: &[OutputSpec],
    include: &[String],
    ignore: &[String],
    scale_override: Option<u32>,
) -> Option<Layout> {
    let live: Vec<&OutputSpec> = outputs
        .iter()
        .filter(|o| o.size.0 > 0 && o.size.1 > 0 && included(&o.name, include, ignore))
        .collect();
    if live.is_empty() {
        return None;
    }

    let mut min_x = i32::MAX;
    let mut min_y = i32::MAX;
    let mut max_x = i32::MIN;
    let mut max_y = i32::MIN;
    let mut tallest = 0u32;
    for o in &live {
        min_x = min_x.min(o.pos.0);
        min_y = min_y.min(o.pos.1);
        max_x = max_x.max(o.pos.0 + o.size.0 as i32);
        max_y = max_y.max(o.pos.1 + o.size.1 as i32);
        tallest = tallest.max(o.size.1);
    }
    let union_w = (max_x - min_x).max(1) as u32;
    let union_h = (max_y - min_y).max(1) as u32;

    // One rule for every backend (D61).
    let (scale, fb_w, fb_h) = match scale_override {
        Some(s) => {
            let s = s.max(1);
            (s, (union_w / s).max(1) as usize, (union_h / s).max(1) as usize)
        }
        None => {
            let (s, w, h) = map::pick_scale(union_w as i32, tallest as i32);
            (s, w as usize, h as usize)
        }
    };

    // The founding monitor.
    let aspect = union_w as f32 / union_h.max(1) as f32;
    let mut focus_index = 0usize;
    if aspect > 3.0 {
        // The largest monitor; on a tie the one whose centre is closest to the
        // union centre (B20: DP-4 on the owner's desk, not DP-3).
        let cx = min_x + union_w as i32 / 2;
        let cy = min_y + union_h as i32 / 2;
        let mut best: Option<(u64, i64, usize)> = None;
        for (i, o) in live.iter().enumerate() {
            let area = o.size.0 as u64 * o.size.1 as u64;
            let ox = o.pos.0 + o.size.0 as i32 / 2;
            let oy = o.pos.1 + o.size.1 as i32 / 2;
            let d = ((ox - cx) as i64).pow(2) + ((oy - cy) as i64).pow(2);
            // Larger area wins; equal area falls back to the smaller distance.
            let better = match best {
                None => true,
                Some((ba, bd, _)) => area > ba || (area == ba && d < bd),
            };
            if better {
                best = Some((area, d, i));
            }
        }
        focus_index = best.map(|(_, _, i)| i).unwrap_or(0);
    } else {
        // Union centre: pick the output that contains it.
        let cx = min_x + union_w as i32 / 2;
        let cy = min_y + union_h as i32 / 2;
        for (i, o) in live.iter().enumerate() {
            if cx >= o.pos.0
                && cx < o.pos.0 + o.size.0 as i32
                && cy >= o.pos.1
                && cy < o.pos.1 + o.size.1 as i32
            {
                focus_index = i;
                break;
            }
        }
    }
    let o = live[focus_index];
    let focus = (
        o.pos.0 + (o.size.0 as i32) / 2 - min_x,
        o.pos.1 + (o.size.1 as i32) / 2 - min_y,
    );

    Some(Layout {
        min_x,
        min_y,
        union_w,
        union_h,
        scale,
        fb_w,
        fb_h,
        focus,
        focus_index,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// the owner's desktop: three monitors in a row plus the Sunshine streaming
    /// output parked below the middle one.
    fn triple() -> Vec<OutputSpec> {
        vec![
            OutputSpec::new("DP-3", 0, 0, 2560, 1440),
            OutputSpec::new("DP-4", 2560, 0, 2560, 1440),
            OutputSpec::new("HDMI-A-3", 5120, 0, 1920, 1080),
            OutputSpec::new("Virtual-Sunshine-4th", 2863, 1440, 1920, 1280),
        ]
    }

    #[test]
    fn virtual_outputs_are_ignored_by_default() {
        let l = layout(&triple(), &[], &[], None).unwrap();
        // 2560+2560+1920 wide, 1440 tall — the Sunshine output is skipped, so
        // the union does not stretch down to 2720.
        assert_eq!((l.union_w, l.union_h), (7040, 1440));
        // D61: 1440 rows of 64-px art at 1x — the whole union, no upscale.
        assert_eq!(l.scale, 1);
        assert_eq!((l.fb_w, l.fb_h), (7040, 1440));
    }

    #[test]
    fn include_and_ignore_override_the_default() {
        let with_virtual = layout(&triple(), &["Virtual-Sunshine-4th".into()], &[], None).unwrap();
        assert_eq!(with_virtual.union_h, 2720);
        let dropped = layout(&triple(), &[], &["HDMI-A-3".into()], None).unwrap();
        assert_eq!(dropped.union_w, 5120);
        // --include beats --ignore for the same name.
        let both = layout(
            &triple(),
            &["Virtual-Sunshine-4th".into()],
            &["Virtual-Sunshine-4th".into()],
            None,
        )
        .unwrap();
        assert_eq!(both.union_h, 2720);
    }

    #[test]
    fn founding_monitor_is_the_most_central_of_the_largest() {
        let l = layout(&triple(), &[], &[], None).unwrap();
        // 7040/1440 = 4.9:1. DP-3 and DP-4 are both 2560×1440, so the *most
        // central* one wins — DP-4, whose centre (3840, 720) is nearest the
        // union centre (3520, 720) (B20 answer 5).
        assert_eq!(l.focus_index, 1);
        assert_eq!(l.focus, (3840, 720));
    }

    #[test]
    fn a_normal_desk_founds_at_the_union_centre() {
        let outs = vec![
            OutputSpec::new("DP-1", 0, 0, 2560, 1440),
            OutputSpec::new("DP-2", 2560, 0, 1920, 1080),
        ];
        let l = layout(&outs, &[], &[], None).unwrap();
        assert_eq!((l.union_w, l.union_h), (4480, 1440));
        // 3.1:1 — wide, so the largest monitor (DP-1) wins and the focus is its
        // centre, which is what a person expects to look at.
        assert_eq!(l.focus, (1280, 720));
    }

    #[test]
    fn scale_override_and_tallest_monitor() {
        let l = layout(&triple(), &[], &[], Some(3)).unwrap();
        assert_eq!(l.scale, 3);
        assert_eq!((l.fb_w, l.fb_h), (7040 / 3, 1440 / 3));
        // A 1080-only desk: max(1, round(1080/1440)) = 1 (D61 — it shows more
        // world than the old 2x framing, which is what the owner asked for).
        let hd = vec![OutputSpec::new("HDMI-1", 0, 0, 1920, 1080)];
        let l = layout(&hd, &[], &[], None).unwrap();
        assert_eq!(l.scale, 1);
        // A 2160p desk doubles.
        let uhd = vec![OutputSpec::new("DP-1", 0, 0, 3840, 2160)];
        let l = layout(&uhd, &[], &[], None).unwrap();
        assert_eq!((l.scale, l.fb_w, l.fb_h), (2, 1920, 1080));
    }

    /// B34 item 1: the owner's desk at 1x — the regression that shipped 2x.
    #[test]
    fn triple_layout_is_scale_one() {
        let l = layout(&triple(), &[], &[], None).unwrap();
        assert_eq!((l.scale, l.fb_w, l.fb_h), (1, 7040, 1440));
        // The same answer the other backends compute.
        assert_eq!(map::pick_scale(7040, 1440), (1, 7040, 1440));
    }
}
