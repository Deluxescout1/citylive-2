//! B72: kit sprites export the window panes the sim can switch.
use buildkit::{BuildingType, Light, Sprite, draw};

#[test]
fn sprites_export_window_panes() {
    let light = Light::night();
    let mut n = 0;
    let mut per = Vec::new();
    for &bt in BuildingType::ALL {
        let spec = bt.default_spec(42, 0);
        let mut s = Sprite::new(1, 1);
        draw(&spec, &light, &mut s);
        n += s.win.len();
        per.push((bt, s.win.len()));
    }
    eprintln!("total panes {n}");
    for (bt, k) in per.iter().filter(|(_, k)| *k > 0).take(12) {
        eprintln!("  {:?}: {k}", bt);
    }
    assert!(n > 0, "no window panes exported");
}
