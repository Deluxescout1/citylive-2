//! PNG output (RGBA8). Same pattern as the M0 spike: frames are 0xAARRGGBB,
//! PNG wants byte order R,G,B,A.

use std::error::Error;
use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

use render::Framebuffer;
use render::palette;

pub fn write_png(path: &Path, fb: &Framebuffer) -> Result<(), Box<dyn Error>> {
    let file = File::create(path)?;
    let w = BufWriter::new(file);

    let mut enc = png::Encoder::new(w, fb.w as u32, fb.h as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc.write_header()?;

    let mut data = Vec::with_capacity(fb.px().len() * 4);
    for &p in fb.px() {
        data.push(palette::r(p));
        data.push(palette::g(p));
        data.push(palette::b(p));
        data.push(255);
    }
    writer.write_image_data(&data)?;
    writer.finish()?;
    Ok(())
}

/// Decode an 8-bit RGB/RGBA PNG into 0xAARRGGBB pixels (for the reference
/// crops in the compare sheet).
pub fn read_png(path: &Path) -> Result<(u32, u32, Vec<u32>), Box<dyn Error>> {
    let file = File::open(path)?;
    let dec = png::Decoder::new(BufReader::new(file));
    let mut reader = dec.read_info()?;
    let mut buf = vec![0; reader.output_buffer_size().unwrap_or(0)];
    let info = reader.next_frame(&mut buf)?;
    let (w, h) = (info.width, info.height);
    let px = match info.color_type {
        png::ColorType::Rgba => buf[..(w * h * 4) as usize]
            .chunks_exact(4)
            .map(|c| palette::rgb(c[0], c[1], c[2]))
            .collect(),
        png::ColorType::Rgb => buf[..(w * h * 3) as usize]
            .chunks_exact(3)
            .map(|c| palette::rgb(c[0], c[1], c[2]))
            .collect(),
        other => return Err(format!("unsupported PNG color type {other:?}").into()),
    };
    Ok((w, h, px))
}

use std::io::BufReader;
