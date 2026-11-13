//! Rain overlay: ~1500 blended streaks, the cheapest thing that reads as rain.

use crate::framebuffer::Framebuffer;
use crate::palette::{Color, pal};
use map::rng::Rng;

pub const DEFAULT_DROPS: usize = 1500;

#[derive(Clone, Copy, Debug)]
struct Drop {
    x: f32,
    y: f32,
    len: u8,
    speed: f32,
}

pub struct Rain {
    drops: Vec<Drop>,
    w: f32,
    h: f32,
    color: Color,
    coverage: f32,
}

impl Rain {
    pub fn new(w: usize, h: usize, seed: u64, count: usize) -> Self {
        let mut rng = Rng::new(seed ^ 0x2A1A_0000);
        let wf = w as f32;
        let hf = h as f32;
        let mut drops = Vec::with_capacity(count);
        for _ in 0..count {
            drops.push(Drop {
                x: rng.range_f32(-40.0, wf + 40.0),
                y: rng.range_f32(0.0, hf),
                len: rng.range(3, 7) as u8,
                speed: rng.range_f32(420.0, 760.0),
            });
        }
        Self {
            drops,
            w: wf,
            h: hf,
            color: pal::RAIN,
            coverage: 0.30,
        }
    }

    pub fn update(&mut self, dt: f32) {
        // A stall (suspend, a slow frame) must not throw drops far outside the
        // frame, where the single wrap below would leave them for many frames.
        let dt = if dt.is_finite() { dt.clamp(0.0, 0.1) } else { 0.0 };
        let wind = 190.0;
        for d in &mut self.drops {
            d.y += d.speed * dt;
            d.x -= wind * dt;
            if d.y > self.h + 8.0 {
                d.y -= self.h + 12.0;
                d.x += self.w * 0.3;
            }
            if d.x < -48.0 {
                d.x += self.w + 80.0;
            }
        }
    }

    pub fn draw(&self, fb: &mut Framebuffer) {
        for d in &self.drops {
            let x0 = d.x as i32;
            let y0 = d.y as i32;
            // The streak leans with the wind: one pixel left every two down.
            for i in 0..d.len as i32 {
                fb.put_blended(x0 - i / 2, y0 + i, self.color, self.coverage);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_long_stall_keeps_drops_near_the_frame() {
        let mut r = Rain::new(400, 300, 3, 200);
        r.update(5000.0);
        r.update(f32::NAN);
        for d in &r.drops {
            assert!(d.x.is_finite() && d.y.is_finite());
            assert!(d.y <= r.h + 8.0 + 80.0 && d.x >= -48.0 - 80.0, "{d:?}");
        }
    }

    use super::*;

    #[test]
    fn rain_stays_on_screen_after_many_steps() {
        let mut r = Rain::new(960, 540, 1, 1500);
        for _ in 0..600 {
            r.update(1.0 / 30.0);
        }
        assert!(r.drops.iter().all(|d| d.y < 560.0 && d.x > -60.0));
    }
}
