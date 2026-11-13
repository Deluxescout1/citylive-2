//! Tile kinds and lot kinds in a save (diagnostic).
fn main() {
    let p = std::env::args().nth(1).expect("save path");
    let s = sim::Sim::load(std::path::Path::new(&p)).expect("load");
    let mut k = std::collections::BTreeMap::<String, u32>::new();
    for y in 0..s.map.h {
        for x in 0..s.map.w {
            *k.entry(format!("{:?}", s.map.at(x, y).kind)).or_default() += 1;
        }
    }
    println!("time {:.2} days, map {}x{}", s.time / 86400.0, s.map.w, s.map.h);
    println!("tiles {k:?}");
    let mut l = std::collections::BTreeMap::<String, u32>::new();
    for lot in &s.lots {
        *l.entry(format!("{:?}", lot.kind)).or_default() += 1;
    }
    println!("lots {l:?}");
    println!("lanes {}", s.lane_tiles().len());
}
