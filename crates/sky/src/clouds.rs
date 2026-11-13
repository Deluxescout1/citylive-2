//! B57: the bridge between the sky's palette and the renderer's cloud field.
//!
//! The clouds are *drawn* by `render::clouds` (they move every frame, and the
//! scene owns the frame), but what colour they are comes from the sky: the
//! palette the hour gives, and the sun/moon the light model knows. `render` must
//! not depend on `sky`, so this is the one place the two meet.

use crate::{Palette, SkyState};
use render::clouds::{Ink, SkyTint};

/// The palette, flattened to what cloud ink needs.
pub fn tint(pal: &Palette) -> SkyTint {
    tint_with_sun(pal, 0.5)
}

/// As [`tint`], with where the sun (or, at night, the moon) sits across the band:
/// 0 = the left edge, 1 = the right. That is the side a cloud is lit from.
pub fn tint_with_sun(pal: &Palette, sun_fraction: f32) -> SkyTint {
    SkyTint {
        horizon: pal.horizon,
        zenith: pal.zenith,
        ambient: pal.ambient_tint,
        glow: pal.window_glow,
        star_alpha: pal.star_alpha,
        sun_x: (sun_fraction * 2.0 - 1.0).clamp(-1.0, 1.0),
    }
}

/// Hand a scene this hour's clouds: cover and storm from the weather, the drift
/// from the wind, and the ink from the light. Returns true when the field or its
/// colour changed (the caller then wants a frame).
pub fn update(
    scene: &mut render::Scene,
    st: &SkyState,
    pal: &Palette,
    storm: f32,
    wind_dir: f32,
    wind_kmh: f32,
) -> bool {
    let mut changed = scene.set_clouds(st.cloud, storm);
    scene.set_cloud_wind(wind_dir, wind_kmh);
    let sun_fraction = crate::backdrop::sun_fraction(st);
    changed |= scene.set_cloud_ink(Ink::new(
        tint_with_sun(pal, sun_fraction),
        &scene.light_state(),
        st.cloud,
        storm,
    ));
    changed
}
