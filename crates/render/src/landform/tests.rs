use super::*;

fn spec(form: Form, biome: Biome) -> Spec {
    Spec { form, biome, seed: 7, radius: 6.0, height: 120.0 }
}

#[test]
fn every_form_in_every_land_bakes_a_sprite() {
    for b in [Biome::Temperate, Biome::Desert, Biome::Tundra, Biome::Tropical] {
        for f in Form::for_biome(b) {
            let s = bake(&spec(f, b), &Sun::default_day()).sprite;
            let solid = s.px.iter().filter(|&&c| c != TRANSPARENT).count();
            assert!(solid > 4000, "{f:?} in {b:?} is nearly empty ({solid} px)");
            assert!(s.w < 1100 && s.h < 700, "{f:?} sprite is {}x{}", s.w, s.h);
        }
    }
}

#[test]
fn baking_is_deterministic() {
    let a = bake(&spec(Form::Mountain, Biome::Tundra), &Sun::default_day()).sprite;
    let b = bake(&spec(Form::Mountain, Biome::Tundra), &Sun::default_day()).sprite;
    assert_eq!(a.px, b.px);
}

#[test]
fn the_light_turns_the_faces() {
    let mut left = Sun::default_day();
    let mut right = left;
    left.dir = [-0.5, 0.5, 0.7];
    right.dir = [0.5, -0.5, 0.7];
    let a = bake(&spec(Form::Mesa, Biome::Desert), &left).sprite;
    let b = bake(&spec(Form::Mesa, Biome::Desert), &right).sprite;
    assert_ne!(a.px, b.px, "moving the sun changed nothing");
}

#[test]
fn a_facet_is_one_ramp_step_not_a_gradient() {
    // A landform draws from a handful of hand-picked ramps: a hard-edged sprite
    // has few distinct colours (a shaded one would have thousands).
    let s = bake(&spec(Form::Cone, Biome::Desert), &Sun::default_day()).sprite;
    let mut cols: Vec<u32> = s.px.iter().copied().filter(|&c| c != TRANSPARENT).collect();
    cols.sort_unstable();
    cols.dedup();
    assert!(cols.len() < 90, "{} distinct colours: that is shading, not facets", cols.len());
}

#[test]
fn every_pixel_belongs_to_a_painter_slice() {
    let site = Site {
        form: Form::Butte,
        biome: Biome::Desert,
        cx: 20.5,
        cy: 20.5,
        radius: 5.0,
        base: 4.0,
        peak: 9.0,
        seed: 3,
    };
    let lf = Landform::bake(site, &Sun::default_day());
    for i in 0..lf.sprite.px.len() {
        if lf.sprite.px[i] != TRANSPARENT {
            let d = lf.diag[i] as i32;
            assert!(d >= lf.d_min && d <= lf.d_max, "pixel {i} is on diagonal {d}");
        }
    }
}

#[test]
fn a_site_is_found_and_its_footprint_cannot_be_built_on() {
    let map = map::generate_with(5, 107, 107, Some(Biome::Desert)).unwrap();
    let sites = detect(&map);
    assert!(!sites.is_empty(), "a desert map should hold a landform");
    for s in &sites {
        let t = map.at(s.cx as i32, s.cy as i32);
        assert!(t.relief() >= 3, "the footprint centre must not read as flat ground");
        assert!(!map.flat_buildable(s.cx as i32, s.cy as i32));
    }
}
