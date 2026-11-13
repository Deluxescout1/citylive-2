//! citylive-artspike: B25 art-detail spike.
//!
//! Modes:
//!   render --tile 32|64 --mode day|night --out FILE   one on-screen 1920×1080 PNG
//!   renders                                              all four finals + crops + compare
//!   bench                                                union-size measurements
//!   crops                                                building crops only

mod bench;
mod buildings;
mod col;
mod compare;
mod font;
mod geom;
mod parts;
mod pngio;
mod procstat;
mod scene;

use std::error::Error;
use std::path::PathBuf;

use render::Framebuffer;

use crate::col::Mode;
use crate::scene::Scene;

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args().skip(1);
    let cmd = args.next().unwrap_or_else(|| "all".to_string());
    let outdir = PathBuf::from(
        std::env::var("ARTSPIKE_OUT").unwrap_or_else(|_| "handbacks/shots".to_string()),
    );
    std::fs::create_dir_all(&outdir)?;

    match cmd.as_str() {
        "render" => {
            let mut tile = 64;
            let mut mode = Mode::Day;
            let mut out = outdir.join("render.png");
            let mut w: Option<usize> = None;
            let mut h: Option<usize> = None;
            let mut seed = compare::SEED;
            while let Some(a) = args.next() {
                match a.as_str() {
                    "--tile" => tile = args.next().unwrap().parse()?,
                    "--mode" => {
                        mode = match args.next().unwrap().as_str() {
                            "night" => Mode::Night,
                            _ => Mode::Day,
                        }
                    }
                    "--out" => out = PathBuf::from(args.next().unwrap()),
                    "--w" => w = Some(args.next().unwrap().parse()?),
                    "--h" => h = Some(args.next().unwrap().parse()?),
                    "--seed" => seed = args.next().unwrap().parse()?,
                    other => return Err(format!("unknown arg {other}").into()),
                }
            }
            let (iw, ih) = match (w, h) {
                (Some(w), Some(h)) => (w, h),
                _ => {
                    if tile == 32 {
                        (960, 540)
                    } else {
                        (1920, 1080)
                    }
                }
            };
            let scene = Scene::new(tile, iw, ih, mode, seed);
            let mut fb = Framebuffer::new(iw, ih);
            let t = std::time::Instant::now();
            let crops = scene.render(&mut fb);
            eprintln!(
                "render: {}×{} tile={} mode={:?}: {:.1} ms ({} crops)",
                iw,
                ih,
                tile,
                mode,
                t.elapsed().as_secs_f64() * 1000.0,
                crops.len()
            );
            if tile == 32 {
                let mut up = Vec::new();
                render::scale::upscale_into(fb.px(), fb.w, fb.h, &mut up, 2);
                let mut big = Framebuffer::new(fb.w * 2, fb.h * 2);
                big.px_mut().copy_from_slice(&up);
                pngio::write_png(&out, &big)?;
            } else {
                pngio::write_png(&out, &fb)?;
            }
            eprintln!("wrote {}", out.display());
        }
        "renders" => {
            let r = compare::render_all();
            compare::write_finals(&r, &outdir)?;
            compare::write_crops(&r, &outdir)?;
            let refimg = PathBuf::from("reference/images/6-dense-downtown-plaza.png");
            compare::write_compare(&r, &outdir, &refimg)?;
            eprintln!("wrote finals + crops + compare to {}", outdir.display());
        }
        "compare" => {
            let r = compare::render_all();
            let refimg = PathBuf::from("reference/images/6-dense-downtown-plaza.png");
            compare::write_compare(&r, &outdir, &refimg)?;
        }
        "bench" => {
            bench::run_1080();
            bench::run();
        }
        "crops" => {
            let r = compare::render_all();
            compare::write_crops(&r, &outdir)?;
        }
        "all" => {
            let r = compare::render_all();
            compare::write_finals(&r, &outdir)?;
            compare::write_crops(&r, &outdir)?;
            let refimg = PathBuf::from("reference/images/6-dense-downtown-plaza.png");
            compare::write_compare(&r, &outdir, &refimg)?;
            bench::run_1080();
            bench::run();
        }
        other => return Err(format!("unknown command {other}").into()),
    }
    Ok(())
}
