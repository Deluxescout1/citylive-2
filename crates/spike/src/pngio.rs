//! PNG output (RGBA8). The frame is `0xAARRGGBB`; PNG wants byte order R,G,B,A.

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

/// Write `path` plus a `<stem>@<scale>x.png` nearest-upscaled copy.
pub fn write_png_scaled(path: &Path, fb: &Framebuffer, scale: usize) -> Result<(), Box<dyn Error>> {
    write_png(path, fb)?;

    let mut up = Vec::new();
    render::scale::upscale_into(fb.px(), fb.w, fb.h, &mut up, scale);
    let mut big = Framebuffer::new(fb.w * scale, fb.h * scale);
    big.px_mut().copy_from_slice(&up);

    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "frame".to_string());
    let scaled = path.with_file_name(format!("{stem}@{scale}x.png"));
    write_png(&scaled, &big)?;
    Ok(())
}
