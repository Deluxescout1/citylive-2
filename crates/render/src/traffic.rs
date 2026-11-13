//! Traffic sprites (B29): cars, vans, buses, trucks, taxis and emergency
//! vehicles as placeholder pixel blocks with the right iso direction, plus
//! headlights and flashing lights.
//!
//! The sim owns where everything is; this module only knows how to draw a
//! vehicle of a given kind at a given pose. Sprites are built once per kind and
//! direction (four axes) at start-up, so a frame is blits and a few glow pixels.

use crate::framebuffer::{Framebuffer, Sprite};
use crate::iso::{Camera, HEIGHT_UNIT};
use crate::palette::{self, pal, Color};
use map::TileMap;

/// What is drawn, mirrored from `sim::traffic::VehicleKind` without the
/// dependency (the renderer never depends on the sim).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// A horse and cart (B49): the village's traffic before the motor car.
    Cart,
    Car,
    Van,
    Taxi,
    Bus,
    Truck,
    /// An electric car (B49/D72): quiet, no exhaust, a green plate.
    Ev,
    Emergency,
    /// B75: a cycle, scooter or motorbike — narrow and quick.
    Bike,
}

impl Kind {
    /// Body length along the road, in screen pixels (B32: 2x the 32-px art,
    /// which keeps the same world size on a 64-px tile).
    pub fn length_px(self) -> i32 {
        match self {
            Kind::Cart => 16,
            Kind::Car | Kind::Taxi | Kind::Ev => 14,
            Kind::Van => 18,
            Kind::Truck => 22,
            Kind::Bus => 28,
            Kind::Emergency => 20,
            Kind::Bike => 6,
        }
    }

    /// Body width across the road, in pixels.
    pub fn width_px(self) -> i32 {
        match self {
            Kind::Cart => 6,
            Kind::Car | Kind::Taxi | Kind::Ev => 7,
            Kind::Van => 8,
            Kind::Truck => 9,
            Kind::Bus => 9,
            Kind::Emergency => 8,
            Kind::Bike => 3,
        }
    }

    /// Height above the road, in pixels.
    pub fn height_px(self) -> i32 {
        match self {
            // The cart reads as a low flat bed with a load, not a car.
            Kind::Cart => 4,
            Kind::Car | Kind::Taxi => 6,
            Kind::Ev => 5,
            Kind::Van => 9,
            Kind::Truck => 11,
            Kind::Bus => 11,
            Kind::Emergency => 8,
            Kind::Bike => 3,
        }
    }

    /// How many colours this kind is drawn in (a car's paint, an EV's).
    pub fn variants(self) -> u8 {
        match self {
            Kind::Car => pal::CAR_BODY.len() as u8,
            Kind::Ev => pal::EV_BODY.len() as u8,
            Kind::Bike => 4,
            _ => 1,
        }
    }

    fn body(self, variant: u8) -> Color {
        match self {
            Kind::Cart => pal::CART,
            Kind::Car => palette::jitter(pal::CAR_BODY[variant as usize % pal::CAR_BODY.len()], 0.5, 0.08),
            Kind::Taxi => pal::TAXI,
            Kind::Ev => palette::jitter(pal::EV_BODY[variant as usize % pal::EV_BODY.len()], 0.5, 0.06),
            Kind::Van => pal::VAN,
            Kind::Bus => pal::BUS,
            Kind::Truck => pal::TRUCK,
            Kind::Emergency => pal::EMERGENCY,
            Kind::Bike => palette::rgb(0x3a, 0x3e, 0x48),
        }
    }
}

/// B75/D83: the *body* of a vehicle, on top of its class. The class (`Kind`)
/// says how it drives — a car, a van, a lorry, a bus; the style says what it
/// looks like, which is what a street full of traffic is actually made of.
/// Invented designs throughout: no real makes, marks or logos.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Style {
    #[default]
    Hatchback,
    Saloon,
    Estate,
    Suv,
    Pickup,
    Minivan,
    CityCar,
    Sports,
    Convertible,
    Classic,
    Camper,
    Motorbike,
    Scooter,
    Bicycle,
    DeliveryVan,
    BoxTruck,
    Lorry,
    BinLorry,
    Sweeper,
    IceCream,
    Coach,
    SchoolBus,
    TowTruck,
    Mixer,
    CraneTruck,
    Tractor,
    /// B75: the three classes that keep their own art — a taxi's roof sign, an
    /// emergency vehicle's white body, the village cart.
    Taxi,
    Emergency,
    Cart,
    /// B76: the winter service — a snowplough with its blade down and a gritter
    /// with its hopper. They come out when snow is lying.
    Plough,
    Gritter,
}

impl Style {
    /// Every style, in index order (the sim's `style` byte indexes this).
    pub const ALL: [Style; 31] = [
        Style::Hatchback,
        Style::Saloon,
        Style::Estate,
        Style::Suv,
        Style::Pickup,
        Style::Minivan,
        Style::CityCar,
        Style::Sports,
        Style::Convertible,
        Style::Classic,
        Style::Camper,
        Style::Motorbike,
        Style::Scooter,
        Style::Bicycle,
        Style::DeliveryVan,
        Style::BoxTruck,
        Style::Lorry,
        Style::BinLorry,
        Style::Sweeper,
        Style::IceCream,
        Style::Coach,
        Style::SchoolBus,
        Style::TowTruck,
        Style::Mixer,
        Style::CraneTruck,
        Style::Tractor,
        Style::Taxi,
        Style::Emergency,
        Style::Cart,
        Style::Plough,
        Style::Gritter,
    ];

    pub fn from_u8(v: u8) -> Style {
        *Self::ALL.get(v as usize).unwrap_or(&Style::Hatchback)
    }

    pub fn to_u8(self) -> u8 {
        Self::ALL.iter().position(|s| *s == self).unwrap_or(0) as u8
    }

    pub fn name(self) -> &'static str {
        match self {
            Style::Hatchback => "hatchback",
            Style::Saloon => "saloon",
            Style::Estate => "estate",
            Style::Suv => "suv",
            Style::Pickup => "pickup",
            Style::Minivan => "minivan",
            Style::CityCar => "city car",
            Style::Sports => "sports car",
            Style::Convertible => "convertible",
            Style::Classic => "classic",
            Style::Camper => "campervan",
            Style::Motorbike => "motorbike",
            Style::Scooter => "scooter",
            Style::Bicycle => "bicycle",
            Style::DeliveryVan => "delivery van",
            Style::BoxTruck => "box truck",
            Style::Lorry => "articulated lorry",
            Style::BinLorry => "bin lorry",
            Style::Sweeper => "street sweeper",
            Style::IceCream => "ice-cream van",
            Style::Coach => "tour coach",
            Style::SchoolBus => "school bus",
            Style::TowTruck => "tow truck",
            Style::Mixer => "cement mixer",
            Style::CraneTruck => "crane truck",
            Style::Tractor => "tractor",
            Style::Taxi => "taxi",
            Style::Emergency => "emergency vehicle",
            Style::Cart => "cart",
            Style::Plough => "snowplough",
            Style::Gritter => "gritter",
        }
    }

    /// Length, width and height in pixels (a saloon is 15x7x6, the old car).
    pub fn dims(self) -> (f32, f32, f32) {
        match self {
            Style::Hatchback => (13.0, 7.0, 6.0),
            Style::Saloon => (15.0, 7.0, 6.0),
            Style::Estate => (17.0, 7.0, 6.5),
            Style::Suv => (15.0, 8.0, 8.0),
            Style::Pickup => (17.0, 8.0, 7.5),
            Style::Minivan => (16.0, 8.0, 8.5),
            Style::CityCar => (11.0, 6.0, 6.0),
            Style::Sports => (14.0, 7.0, 4.5),
            Style::Convertible => (14.0, 7.0, 5.0),
            Style::Classic => (15.0, 7.0, 7.5),
            Style::Camper => (19.0, 8.0, 10.0),
            Style::Motorbike => (7.0, 3.0, 5.0),
            Style::Scooter => (6.0, 3.0, 5.0),
            Style::Bicycle => (5.0, 2.0, 4.0),
            Style::DeliveryVan => (17.0, 8.0, 9.0),
            Style::BoxTruck => (20.0, 9.0, 11.0),
            Style::Lorry => (29.0, 9.0, 12.0),
            Style::BinLorry => (20.0, 9.0, 10.0),
            Style::Sweeper => (18.0, 9.0, 8.0),
            Style::IceCream => (17.0, 8.0, 9.0),
            Style::Coach => (30.0, 9.0, 12.0),
            Style::SchoolBus => (26.0, 9.0, 11.0),
            Style::TowTruck => (20.0, 9.0, 8.0),
            Style::Mixer => (22.0, 9.0, 10.0),
            Style::CraneTruck => (24.0, 9.0, 8.0),
            Style::Tractor => (16.0, 8.0, 9.0),
            Style::Taxi => (15.0, 7.0, 6.0),
            Style::Emergency => (20.0, 8.0, 8.0),
            Style::Cart => (10.0, 6.0, 3.0),
            Style::Plough => (20.0, 9.0, 9.0),
            Style::Gritter => (20.0, 9.0, 11.0),
        }
    }

    /// How many paints this style is offered in.
    pub fn variants(self) -> u8 {
        match self {
            Style::Motorbike | Style::Scooter | Style::Bicycle => 4,
            Style::IceCream | Style::Camper | Style::Classic => 4,
            _ => 8,
        }
    }

    /// Is this a work vehicle (fleet liveries) rather than a private car?
    fn fleet(self) -> bool {
        matches!(
            self,
            Style::DeliveryVan
                | Style::BoxTruck
                | Style::Lorry
                | Style::BinLorry
                | Style::Sweeper
                | Style::Coach
                | Style::SchoolBus
                | Style::TowTruck
                | Style::Mixer
                | Style::CraneTruck
                | Style::Tractor
                | Style::Plough
                | Style::Gritter
        )
    }

    /// The paint for a variant, from the right table for the style.
    fn body(self, variant: u8) -> Color {
        match self {
            Style::IceCream => match variant % 2 {
                0 => palette::rgb(0xf4, 0xf0, 0xe8),
                _ => palette::rgb(0xf0, 0xc8, 0xd8),
            },
            Style::Camper => match variant % 3 {
                0 => palette::rgb(0xe8, 0xdc, 0xc0),
                1 => palette::rgb(0x88, 0xb8, 0xb0),
                _ => palette::rgb(0xe0, 0xe0, 0xd8),
            },
            Style::SchoolBus => pal::SCHOOL_BUS,
            Style::Classic => match variant % 4 {
                0 => palette::rgb(0x2e, 0x3a, 0x30),
                1 => palette::rgb(0x50, 0x1e, 0x26),
                2 => palette::rgb(0x20, 0x24, 0x30),
                _ => palette::rgb(0x6a, 0x5a, 0x3a),
            },
            Style::Motorbike | Style::Scooter => match variant % 4 {
                0 => palette::rgb(0xb0, 0x34, 0x30),
                1 => palette::rgb(0x30, 0x38, 0x48),
                2 => palette::rgb(0xe0, 0xdc, 0xd0),
                _ => palette::rgb(0x40, 0x6a, 0x58),
            },
            Style::Bicycle => match variant % 4 {
                0 => palette::rgb(0x3a, 0x3e, 0x48),
                1 => palette::rgb(0x6a, 0x30, 0x2a),
                2 => palette::rgb(0x2e, 0x50, 0x6a),
                _ => palette::rgb(0x50, 0x56, 0x40),
            },
            Style::Taxi => pal::TAXI,
            Style::Emergency => pal::EMERGENCY,
            Style::Cart => pal::CART,
            _ if self.fleet() => pal::FLEET_PAINT[variant as usize % pal::FLEET_PAINT.len()],
            _ => pal::CAR_PAINT[variant as usize % pal::CAR_PAINT.len()],
        }
    }
}

/// One vehicle to draw this frame.
#[derive(Clone, Copy, Debug)]
pub struct Body {
    /// Continuous tile position of its centre.
    pub x: f32,
    pub y: f32,
    /// Facing, axis aligned: (1,0) down-right on screen, (0,1) down-left.
    pub dir: (f32, f32),
    pub kind: Kind,
    /// B75: the body. The kind still decides how it drives and which lamps it
    /// carries; the style decides what it looks like (26 invented designs).
    pub style: Style,
    /// Colour pick into the style's own paint table.
    pub variant: u8,
    /// Headlights on (dusk to dawn, and for emergency vehicles always).
    pub lights: bool,
    /// Emergency light phase in seconds (blinks red/blue).
    pub blink: f32,
    /// Height above the ground in pixels: a car on the upper deck of a
    /// multi-storey car park stands on it, not through it (B59).
    pub lift: f32,
    /// Brake lights on: a car holding in a queue, or stopped in a jam (B59).
    pub brake: bool,
    /// B75: the indicator (0 none, 1 one side, 2 the other), blinked in `draw`.
    pub signal: i8,
}

/// Vehicles, with the sprites they are drawn from.
pub struct TrafficRenderer {
    /// Per kind, per colour variant, per facing.
    sprites: Vec<Vec<[Sprite; 4]>>,
    /// The same, in the moonlight: a car's paint is not noon-bright at night
    /// (B59b: a lit car park of pastel boxes on a dark lot read as glare).
    night: Vec<Vec<[Sprite; 4]>>,
    kinds: [Kind; 8],
}

impl TrafficRenderer {
    /// B61f: the bytes the sprite table holds (for the memory report).
    pub fn bytes(&self) -> usize {
        let set = |v: &Vec<Vec<[Sprite; 4]>>| {
            v.iter()
                .flat_map(|k| k.iter())
                .flat_map(|d| d.iter())
                .map(|s| s.bytes())
                .sum::<usize>()
        };
        set(&self.sprites) + set(&self.night)
    }

    pub fn new() -> Self {
        let kinds = [
            Kind::Cart,
            Kind::Car,
            Kind::Van,
            Kind::Taxi,
            Kind::Bus,
            Kind::Truck,
            Kind::Ev,
            Kind::Emergency,
        ];
        let sprites: Vec<Vec<[Sprite; 4]>> = Style::ALL
            .iter()
            .map(|st| {
                (0..st.variants())
                    .map(|v| {
                        [
                            sprite_for_style(*st, 0, v),
                            sprite_for_style(*st, 1, v),
                            sprite_for_style(*st, 2, v),
                            sprite_for_style(*st, 3, v),
                        ]
                    })
                    .collect()
            })
            .collect();
        let night = sprites
            .iter()
            .map(|set: &Vec<[Sprite; 4]>| {
                set.iter()
                    .map(|four| [dim(&four[0]), dim(&four[1]), dim(&four[2]), dim(&four[3])])
                    .collect()
            })
            .collect();
        Self { sprites, night, kinds }
    }

    fn index_of(&self, kind: Kind) -> usize {
        self.kinds.iter().position(|k| *k == kind).unwrap_or(0)
    }

    /// Draw every body, with depth keys so buildings in front hide them.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &self,
        fb: &mut Framebuffer,
        cam: &Camera,
        map: &TileMap,
        bodies: &[Body],
        bands: &crate::bands::Bands,
        depth: &[u16],
        glow: f32,
        mode: crate::actors::DepthMode,
        city_key: &[u16],
        snow: f32,
    ) {
        // B61e: one vehicle, by list index; the band index below decides which
        // ones a damaged rectangle visits (there are ~300 of them and a rush
        // hour is ~90 rectangles).
        let clip = fb.clip();
        let mut one = |bi: usize| {
            let b = &bodies[bi];
            let z = map.ground_px(b.x, b.y) + b.lift as i32;
            let (sx, sy) = cam.project(b.x, b.y, z);
            let dir_i = dir_index(b.dir);
            let dark = glow > 0.35;
            let si = b.style.to_u8() as usize;
            let set = if dark { &self.night[si] } else { &self.sprites[si] };
            let s = &set[b.variant as usize % set.len()][dir_i];
            let mut key = crate::actors::actor_key(b.x.floor() as i32, b.y.floor() as i32);
            // B59: a car standing in a bay is on a plot, and a plot paints its
            // whole ground with its own (far-corner) key — a car there must
            // beat it or it is drawn behind the asphalt it is parked on.
            let ci = b.y.floor() as i32 * map.w + b.x.floor() as i32;
            if ci >= 0 {
                if let Some(&ck) = city_key.get(ci as usize) {
                    key = key.max(ck);
                }
            }
            let px = sx as i32 - s.ax;
            let py = sy as i32 - s.ay;
            match mode {
                crate::actors::DepthMode::PerPixel => fb.blit_sprite_depth(s, px, py, key, depth),
                crate::actors::DepthMode::Anchor => {
                    fb.blit_sprite_anchor(s, px, py, key, depth);
                }
                crate::actors::DepthMode::Off => {
                    fb.blit_sprite(s, px, py);
                }
            }
            // B76: a cap of snow on a parked or moving car — the top row of the
            // body's own box, two pixels deep, so a snowy street's cars read as
            // covered rather than clean.
            if snow > 0.08 {
                let cap = palette::rgb(0xf2, 0xf7, 0xfc);
                let w = s.w as i32;
                let top = py + 1;
                let inset = (w / 6).max(0);
                for xx in (px + inset)..(px + w - inset) {
                    if xx < 0 || top < 0 || xx >= fb.w as i32 || top >= fb.h as i32 {
                        continue;
                    }
                    let i = top as usize * fb.w + xx as usize;
                    let visible = match mode {
                        crate::actors::DepthMode::Off => true,
                        _ => depth.get(i).map(|d| *d <= key).unwrap_or(false),
                    };
                    if visible {
                        fb.put(xx, top, cap);
                    }
                }
            }
            // Headlights and the flashing bar are drawn *through* the same depth
            // plane as the body: a car behind a building must not show its
            // lights on the roof (B29).
            let lit = |fb: &Framebuffer, x: i32, y: i32| -> bool {
                if x < 0 || y < 0 {
                    return false;
                }
                let i = y as usize * fb.w + x as usize;
                match mode {
                    crate::actors::DepthMode::Off => true,
                    _ => depth.get(i).map(|d| *d <= key).unwrap_or(false),
                }
            };
            if b.lights {
                let (fx, fy) = (b.dir.0, b.dir.1);
                let front = cam.project(b.x + fx * 0.45, b.y + fy * 0.45, z);
                let head = if b.kind == Kind::Emergency { pal::EMERGENCY_RED } else { pal::HEADLIGHT };
                // A 2 px cone plus a faint halo: at 1x one pixel is too easy to
                // miss on a dark street (B29 item 2).
                let (hx, hy) = (front.0 as i32, front.1 as i32);
                if lit(fb, hx, hy) {
                    fb.put_blended(hx, hy, head, 0.95);
                }
                if lit(fb, hx + 1, hy) {
                    fb.put_blended(hx + 1, hy, head, 0.75);
                }
                if lit(fb, hx, hy - 1) {
                    fb.put_blended(hx, hy - 1, head, 0.45);
                }
                if lit(fb, hx - 1, hy) {
                    fb.put_blended(hx - 1, hy, head, 0.40);
                }
            }
            if b.brake {
                // A brake light is brighter than a tail light and sits on the
                // back face, which is what makes a rush-hour queue read (D72).
                let (fx, fy) = (b.dir.0, b.dir.1);
                let tail = cam.project(b.x - fx * 0.42, b.y - fy * 0.42, z);
                let (tx, ty) = (tail.0 as i32, tail.1 as i32);
                let strength = if glow > 0.35 { 1.0 } else { 0.75 };
                if lit(fb, tx, ty) {
                    fb.put_blended(tx, ty, pal::TAILLIGHT, strength);
                }
                if lit(fb, tx, ty + 1) {
                    fb.put_blended(tx, ty + 1, pal::TAILLIGHT, strength * 0.6);
                }
            } else if b.lights {
                let (fx, fy) = (b.dir.0, b.dir.1);
                let tail = cam.project(b.x - fx * 0.45, b.y - fy * 0.45, z);
                if lit(fb, tail.0 as i32, tail.1 as i32) {
                    fb.put_blended(tail.0 as i32, tail.1 as i32, pal::TAILLIGHT, 0.55);
                }
            }
            // B75: the indicator — an amber pixel on the turning side of the
            // nose, on for the long half of a one-second blink.
            if b.signal != 0 {
                let (fx, fy) = (b.dir.0, b.dir.1);
                // The two corners across the nose, on the 2:1 diamond.
                let (cx, cy) = if fx.abs() >= fy.abs() {
                    (fx * 0.42, if b.signal == 1 { 0.5 } else { -0.5 })
                } else {
                    (if b.signal == 1 { 0.5 } else { -0.5 }, fy * 0.42)
                };
                let p = cam.project(b.x + cx, b.y + cy, z);
                let on = (b.blink * 3.0) as i32 % 2 == 0;
                if on && lit(fb, p.0 as i32, p.1 as i32) {
                    fb.put_blended(p.0 as i32, p.1 as i32, palette::rgb(0xf2, 0xa0, 0x28), 0.95);
                }
            }
            if b.kind == Kind::Emergency {
                // Flashing bar: red and blue alternate about twice a second.
                let phase = (b.blink * 2.2) as i32;
                let c = if phase % 2 == 0 {
                    pal::EMERGENCY_RED
                } else {
                    pal::EMERGENCY_BLUE
                };
                if lit(fb, px + s.w as i32 / 2, py - 1) {
                    fb.put_blended(px + s.w as i32 / 2, py - 1, c, 1.0);
                }
                let side = if phase % 2 == 0 { -1 } else { 1 };
                if lit(fb, px + s.w as i32 / 2 + side, py - 1) {
                    fb.put_blended(px + s.w as i32 / 2 + side, py - 1, c, 0.5);
                }
            }
        };
        if bands.is_empty() {
            for i in 0..bodies.len() {
                one(i);
            }
        } else {
            // A vehicle's sprite stands ~40 px above its wheels (a bus more).
            bands.for_each(clip, 48, |i| one(i as usize));
        }
    }
}

impl Default for TrafficRenderer {
    fn default() -> Self {
        Self::new()
    }
}

/// A car sprite for the decorative life layer (actors.rs), which has no
/// direction of its own: always +x, colour by `variant`.
pub fn sprite_for_actor(variant: u8) -> Sprite {
    sprite_for(Kind::Car, 0, variant)
}

/// Which of the four sprites a direction needs: 0 = +x, 1 = +y, 2 = -x, 3 = -y.
pub fn dir_index(dir: (f32, f32)) -> usize {
    if dir.0.abs() >= dir.1.abs() {
        if dir.0 >= 0.0 {
            0
        } else {
            2
        }
    } else if dir.1 >= 0.0 {
        1
    } else {
        3
    }
}

/// A sprite as the moon sees it: darker and bluer, windows and lamps aside.
fn dim(s: &Sprite) -> Sprite {
    let mut o = Sprite::new(s.w, s.h);
    o.set_anchor(s.ax, s.ay);
    for (i, &c) in s.px.iter().enumerate() {
        if c == 0 {
            continue;
        }
        let (r, g, b) = (palette::r(c) as f32, palette::g(c) as f32, palette::b(c) as f32);
        o.px[i] = palette::with_alpha(
            palette::rgb((r * 0.50) as u8, (g * 0.56) as u8, (b * 0.72) as u8),
            palette::alpha(c),
        );
    }
    o
}

/// One vehicle as an isometric box: a top face, the two sides that face the
/// camera, a windscreen, wheels and a shadow (B32 — 2× the old 7-px bar, so a
/// car reads as a car at 1×).
fn sprite_for(kind: Kind, dir: usize, variant: u8) -> Sprite {
    // B75: the class's own default body, for the callers that only hold a kind
    // (the decorative actors, the sprite tests).
    let style = match kind {
        Kind::Cart => return cart_sprite(dir, variant),
        Kind::Car => Style::Saloon,
        Kind::Van => Style::DeliveryVan,
        Kind::Taxi => Style::Taxi,
        Kind::Bus => Style::Coach,
        Kind::Truck => Style::BoxTruck,
        Kind::Ev => Style::Hatchback,
        Kind::Emergency => Style::Emergency,
        Kind::Bike => Style::Bicycle,
    };
    sprite_for_style(style, dir, variant)
}

/// The taxi, in its own yellow with a sign on the roof.
fn taxi_sprite(dir: usize) -> Sprite {
    let body = pal::TAXI;
    let spec = BoxSpec {
        len: 15.0,
        wid: 7.0,
        hgt: 6.0,
        body,
        roof: palette::shade(body, 1.08),
        glass: pal::CAR_GLASS,
        windows: false,
        wheels: true,
        iso: true,
        cabin: true,
    };
    let mut s = iso_box_sprite(dir, &spec);
    // The roof sign, a pale pixel row above the cabin.
    let (ax, ay) = (s.ax, s.ay);
    for dx in -1..=1 {
        s.set((ax + dx).max(0) as usize, (ay - 7).max(0) as usize, palette::rgb(0xf4, 0xf0, 0xd8));
    }
    s
}

/// An emergency vehicle: a white box with a red side stripe and a clear deck
/// for the flashing bar the traffic layer draws over it.
fn emergency_sprite(dir: usize) -> Sprite {
    let body = pal::EMERGENCY;
    let spec = BoxSpec {
        len: 20.0,
        wid: 8.0,
        hgt: 8.0,
        body,
        roof: palette::shade(body, 1.04),
        glass: pal::CAR_GLASS,
        windows: false,
        wheels: true,
        iso: true,
        cabin: false,
    };
    let mut s = iso_box_sprite(dir, &spec);
    // The stripe: a red band down the middle of the sprite's visible rows.
    let (ax, ay) = (s.ax, s.ay);
    for dx in -8..=8 {
        let x = ax + dx;
        let y = ay - 3;
        if x >= 0 && y >= 0 && (x as usize) < s.w && (y as usize) < s.h {
            s.set(x as usize, y as usize, palette::rgb(0xc8, 0x2e, 0x28));
        }
    }
    s
}

/// B75: the sprite for a style, for the review sheet and the tests.
pub fn style_sprite(style: Style, dir: usize, variant: u8) -> Sprite {
    sprite_for_style(style, dir, variant)
}

/// B75: a whole style as an isometric body plus whatever makes it that style —
/// an articulated trailer, a drum, a jib, a cone. Every part is the same box
/// the traffic has always used, so all of them sit on the road the same way.
fn sprite_for_style(style: Style, dir: usize, variant: u8) -> Sprite {
    match style {
        Style::Cart => return cart_sprite(dir, variant),
        Style::Taxi => return taxi_sprite(dir),
        Style::Emergency => return emergency_sprite(dir),
        _ => {}
    }
    let (len, wid, hgt) = style.dims();
    let body = style.body(variant);
    let cabin = matches!(
        style,
        Style::Hatchback
            | Style::Saloon
            | Style::Estate
            | Style::Suv
            | Style::Pickup
            | Style::Minivan
            | Style::CityCar
            | Style::Sports
            | Style::Convertible
            | Style::Classic
            | Style::Camper
    );
    let windows = matches!(style, Style::Coach | Style::SchoolBus | Style::Minivan);
    let spec = BoxSpec {
        len,
        wid,
        hgt,
        body,
        roof: palette::shade(body, 1.08),
        glass: pal::CAR_GLASS,
        windows,
        wheels: style != Style::Bicycle,
        iso: true,
        cabin,
    };
    let main = iso_box_sprite(dir, &spec);
    let axis = travel_axis(dir);
    // A part: another box, placed `off` tiles along the travel axis (positive
    // is towards the front).
    let part = |l: f32, w: f32, h: f32, b: Color, off: f32| -> (Sprite, f32) {
        let sp = iso_box_sprite(
            dir,
            &BoxSpec {
                len: l,
                wid: w,
                hgt: h,
                body: b,
                roof: palette::shade(b, 1.12),
                glass: pal::CAR_GLASS,
                windows: false,
                wheels: false,
                iso: true,
                cabin: false,
            },
        );
        (sp, off)
    };
    let extra: Option<(Sprite, f32)> = match style {
        // The lorry is drawn as a cab and a trailer, not one long box.
        Style::Lorry => None,
        Style::Mixer => Some(part(12.0, 7.0, 10.0, palette::rgb(0xd8, 0xd2, 0xc2), -2.5)),
        Style::CraneTruck => Some(part(15.0, 2.5, 2.5, pal::HIVIS_O, 5.0)),
        Style::BinLorry => Some(part(9.0, 8.5, 8.0, palette::shade(body, 0.92), -3.0)),
        Style::IceCream => Some(part(3.0, 3.0, 6.0, palette::rgb(0xe8, 0xc0, 0x60), 2.0)),
        Style::Camper => Some(part(8.0, 7.0, 3.0, palette::rgb(0xf4, 0xf0, 0xe8), 3.0)),
        Style::TowTruck => Some(part(11.0, 2.0, 2.0, palette::rgb(0xd0, 0x3c, 0x30), -4.0)),
        Style::Tractor => Some(part(6.0, 9.5, 6.0, palette::rgb(0x28, 0x28, 0x2c), -5.0)),
        // B76: the plough's blade sits across the nose, low and wide; a
        // gritter's hopper rides behind the cab.
        Style::Plough => Some(part(2.0, 12.0, 3.0, palette::rgb(0xd8, 0xd2, 0xc8), 9.0)),
        Style::Gritter => Some(part(9.0, 8.0, 5.0, palette::rgb(0xc8, 0x9a, 0x2e), -3.0)),
        // B75: a rider, so two wheels read as a person on a bike rather than a
        // speck of paint. A dark jacket, a pale head.
        Style::Motorbike | Style::Scooter | Style::Bicycle => {
            Some(part(2.6, 2.6, 6.5, palette::rgb(0x38, 0x3a, 0x44), -0.4))
        }
        _ => None,
    };
    match style {
        Style::Lorry => {
            // A short cab in front, the long trailer behind it.
            let cab = iso_box_sprite(
                dir,
                &BoxSpec { len: 8.0, wid: 9.0, hgt: 12.0, body, roof: palette::shade(body, 1.1), glass: pal::CAR_GLASS, windows: false, wheels: true, iso: true, cabin: false },
            );
            let trailer = iso_box_sprite(
                dir,
                &BoxSpec { len: 19.0, wid: 9.0, hgt: 11.0, body: palette::shade(body, 0.94), roof: palette::shade(body, 1.04), glass: pal::CAR_GLASS, windows: false, wheels: true, iso: true, cabin: false },
            );
            let mut both = Sprite::new(
                (cab.w.max(trailer.w) + 20).max(1),
                (cab.h.max(trailer.h) + 10).max(1),
            );
            both.set_anchor((both.w / 2) as i32, (both.h - 4) as i32);
            let (ax, ay) = (both.ax, both.ay);
            both.blit(
                &trailer,
                ax - (axis.0 * 5.0) as i32 - trailer.ax,
                ay - (axis.1 * 5.0) as i32 - trailer.ay,
            );
            both.blit(
                &cab,
                ax + (axis.0 * 8.0) as i32 - cab.ax,
                ay + (axis.1 * 8.0) as i32 - cab.ay,
            );
            both
        }
        _ => match extra {
            None => main,
            Some((p, off)) => {
                let w = main.w.max(p.w) + 24;
                let h = main.h.max(p.h) + 8;
                let mut combined = Sprite::new(w, h);
                combined.set_anchor((w / 2) as i32, (h - 4) as i32);
                let (ax, ay) = (combined.ax, combined.ay);
                combined.blit(&main, ax - main.ax, ay - main.ay);
                combined.blit(
                    &p,
                    ax + (axis.0 * off) as i32 - p.ax,
                    ay + (axis.1 * off) as i32 - p.ay,
                );
                combined
            }
        },
    }
}

/// The screen-space direction a vehicle travels, as a unit vector on the 2:1
/// diamond (used to place the parts of a multi-box design).
fn travel_axis(dir: usize) -> (f32, f32) {
    match dir {
        0 => (0.894, 0.447),
        1 => (-0.894, 0.447),
        2 => (-0.894, -0.447),
        _ => (0.894, -0.447),
    }
}

/// A horse and cart (B49): a flat timber bed with a load, and the horse in
/// front of it. Two boxes would read as a lorry; the horse is a small dark shape
/// with a neck and legs, which is enough at this scale.
fn cart_sprite(dir: usize, variant: u8) -> Sprite {
    let load = pal::CART_LOAD[variant as usize % pal::CART_LOAD.len()];
    let bed = iso_box_sprite(
        dir,
        &BoxSpec {
            len: 7.0,
            wid: 6.0,
            hgt: 3.0,
            body: pal::CART,
            roof: load,
            glass: pal::CART,
            windows: false,
            wheels: true,
            iso: true,
            cabin: false,
        },
    );
    let horse = iso_box_sprite(
        dir,
        &BoxSpec {
            len: 5.0,
            wid: 3.0,
            hgt: 7.0,
            body: pal::HORSE,
            roof: pal::HORSE_DARK,
            glass: pal::HORSE_DARK,
            windows: false,
            wheels: false,
            iso: true,
            cabin: false,
        },
    );
    // The travel axis on screen, so the horse goes in front of the bed.
    let axis: (f32, f32) = match dir {
        0 => (2.0, 1.0),
        1 => (-2.0, 1.0),
        2 => (-2.0, -1.0),
        _ => (2.0, -1.0),
    };
    let al = (axis.0 * axis.0 + axis.1 * axis.1).sqrt();
    let a = (axis.0 / al, axis.1 / al);
    let w = bed.w.max(horse.w) + 16;
    let h = bed.h.max(horse.h) + 6;
    let mut s = Sprite::new(w, h);
    let ax = (w / 2) as i32;
    let ay = (h - 3) as i32;
    s.set_anchor(ax, ay);
    // The bed sits back, the horse ahead of it.
    s.blit(
        &bed,
        ax - (a.0 * 4.0) as i32 - bed.ax,
        ay - (a.1 * 4.0) as i32 - bed.ay,
    );
    s.blit(
        &horse,
        ax + (a.0 * 5.0) as i32 - horse.ax,
        ay + (a.1 * 5.0) as i32 - horse.ay,
    );
    s
}

/// What an isometric vehicle box is made of.
pub struct BoxSpec {
    /// Length along the road, in screen pixels.
    pub len: f32,
    /// Width across the road, in pixels.
    pub wid: f32,
    /// Height above the ground, in pixels.
    pub hgt: f32,
    pub body: Color,
    pub roof: Color,
    pub glass: Color,
    /// A lit window strip along the sides (buses, trams, train cars).
    pub windows: bool,
    /// Two dark wheels at the bottom of the visible side.
    pub wheels: bool,
    /// B59b: the width runs along the *other isometric axis* (a true box on
    /// the tile grid), not the screen-perpendicular. Road vehicles use it so a
    /// car stands square in a bay and in its lane; the rail art keeps its look.
    pub iso: bool,
    /// A glazed cabin over a lower body (cars); otherwise a plain box.
    pub cabin: bool,
}

/// Draw an axis-aligned isometric box as a sprite, anchored on its ground point.
/// Used for every vehicle in the game (traffic, trains, trams, boats, ships), so
/// they all sit on the ground plane the same way — which is what puts a train
/// car on its rails instead of beside them (H31b).
pub fn iso_box_sprite(dir: usize, spec: &BoxSpec) -> Sprite {
    if spec.iso {
        return iso_vehicle_sprite(dir, spec);
    }
    let (len, wid, hgt) = (spec.len, spec.wid, spec.hgt);
    // Screen axis of travel, and the axis across the road (2:1 iso).
    let axis: (f32, f32) = match dir {
        0 => (2.0, 1.0),
        1 => (-2.0, 1.0),
        2 => (-2.0, -1.0),
        _ => (2.0, -1.0),
    };
    let al = (axis.0 * axis.0 + axis.1 * axis.1).sqrt();
    let a = (axis.0 / al, axis.1 / al);
    let p = if spec.iso { (-a.0, a.1) } else { (-a.1, a.0) };
    let (hl, hw2) = (len * 0.5, wid * 0.5);
    let he = a.0.abs() * hl + p.0.abs() * hw2;
    let ve = a.1.abs() * hl + p.1.abs() * hw2;
    let w = (2.0 * he + 6.0).ceil() as usize;
    let h = (2.0 * ve + hgt + 4.0).ceil() as usize;
    let mut fb = Framebuffer::new(w, h);
    let (ox, oy) = (w as f32 * 0.5, h as f32 - ve - 2.0);
    let proj = |u: f32, v: f32, z: f32| -> (f32, f32) {
        (ox + u * a.0 + v * p.0, oy + u * a.1 + v * p.1 - z)
    };
    let body = spec.body;
    let top = spec.roof;
    let lit = palette::shade(body, 0.98);
    let shade = palette::shade(body, 0.68);
    // B60/D74: no baked shadow in the sprite any more — the vehicle's shadow is
    // projected each frame from the same light everything else reads, so it
    // turns with the sun instead of always pointing the same way.
    // The four side faces: draw the ones that face the camera.
    let sides = [
        ((0.0, -hw2), (len, 0.0)),
        ((0.0, hw2), (len, 0.0)),
        ((-hl, 0.0), (0.0, wid)),
        ((hl, 0.0), (0.0, wid)),
    ];
    for (i, (o, d)) in sides.into_iter().enumerate() {
        // Outward normal of this side, in (u, v) space, then on screen.
        let outward = if i == 0 || i == 2 { -1.0 } else { 1.0 };
        let n = if i < 2 {
            (0.0, outward)
        } else {
            (outward, 0.0)
        };
        let nx = n.0 * a.0 + n.1 * p.0;
        let ny = n.0 * a.1 + n.1 * p.1;
        if (spec.iso && ny <= 0.0) || (ny <= 0.0 && nx <= 0.0) {
            continue;
        }
        let c0 = (o.0, o.1);
        let c1 = (o.0 + d.0, o.1 + d.1);
        let col = if ny > nx * 0.4 { shade } else { lit };
        fb.fill_poly(
            &[
                proj(c0.0, c0.1, hgt),
                proj(c1.0, c1.1, hgt),
                proj(c1.0, c1.1, 0.0),
                proj(c0.0, c0.1, 0.0),
            ],
            col,
        );
        if spec.windows && hgt > 4.0 {
            // A strip of lit windows along the upper half of the side.
            let steps = (len / 4.0).max(2.0) as i32;
            for k in 0..steps {
                let t0 = k as f32 / steps as f32;
                let t1 = (k as f32 + 0.6) / steps as f32;
                let pa = (c0.0 + d.0 * t0, c0.1 + d.1 * t0);
                let pb = (c0.0 + d.0 * t1, c0.1 + d.1 * t1);
                fb.fill_poly(
                    &[
                        proj(pa.0, pa.1, hgt - 1.5),
                        proj(pb.0, pb.1, hgt - 1.5),
                        proj(pb.0, pb.1, hgt * 0.5),
                        proj(pa.0, pa.1, hgt * 0.5),
                    ],
                    pal::BUS_WINDOW,
                );
            }
        }
        if i == 3 {
            // Windscreen on the front face.
            fb.fill_poly(
                &[
                    proj(c0.0, c0.1, hgt - 1.0),
                    proj(c1.0, c1.1, hgt - 1.0),
                    proj(c1.0, c1.1, hgt * 0.45),
                    proj(c0.0, c0.1, hgt * 0.45),
                ],
                spec.glass,
            );
        }
    }
    // The top face, with a highlight down the sunlit side.
    fb.fill_poly(
        &[
            proj(-hl, -hw2, hgt),
            proj(hl, -hw2, hgt),
            proj(hl, hw2, hgt),
            proj(-hl, hw2, hgt),
        ],
        top,
    );
    let (r0, r1) = (
        proj(-hl * 0.6, -hw2 * 0.55, hgt),
        proj(hl * 0.6, -hw2 * 0.55, hgt),
    );
    fb.line(r0, r1, palette::shade(top, 1.12));
    if spec.wheels {
        for u in [-hl * 0.6, hl * 0.6] {
            let c = proj(u, hw2, 1.0);
            fb.fill_rect(c.0 as i32 - 1, c.1 as i32 - 1, 2, 2, pal::CAR_WHEEL);
        }
    }
    // Copy into a sprite, dropping the transparent background.
    let mut s = Sprite::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let c = fb.px()[y * w + x];
            if c != 0 {
                s.set(x, y, c);
            }
        }
    }
    s.set_anchor(ox as i32, oy as i32);
    s
}

/// A road vehicle as a true isometric box (B59b): the body on the tile grid, so
/// it stands square in a lane or a bay, with a glazed cabin on top of the
/// cars and the vans' windscreens. Same pixel sizes as before: a car is 14 px
/// long, 7 wide and 6 high.
fn iso_vehicle_sprite(dir: usize, spec: &BoxSpec) -> Sprite {
    let (len, wid, hgt) = (spec.len, spec.wid, spec.hgt);
    let axis: (f32, f32) = match dir {
        0 => (2.0, 1.0),
        1 => (-2.0, 1.0),
        2 => (-2.0, -1.0),
        _ => (2.0, -1.0),
    };
    let al = (axis.0 * axis.0 + axis.1 * axis.1).sqrt();
    let a = (axis.0 / al, axis.1 / al);
    let p = (-a.0, a.1);
    let (hl, hw2) = (len * 0.5, wid * 0.5);
    let he = a.0.abs() * hl + p.0.abs() * hw2;
    let ve = a.1.abs() * hl + p.1.abs() * hw2;
    let w = (2.0 * he + 6.0).ceil() as usize;
    let h = (2.0 * ve + hgt + 4.0).ceil() as usize;
    let mut fb = Framebuffer::new(w, h);
    let (ox, oy) = (w as f32 * 0.5, h as f32 - ve - 2.0);
    let proj = |u: f32, v: f32, z: f32| -> (f32, f32) {
        (ox + u * a.0 + v * p.0, oy + u * a.1 + v * p.1 - z)
    };
    // The two faces that look at the camera: the one whose outward vector
    // points down-left is the lit one, the down-right one is in shade.
    let u_out = if a.1 > 0.0 { 1.0f32 } else { -1.0 };
    let v_out = if p.1 > 0.0 { 1.0f32 } else { -1.0 };
    let left_is_u = (a.0 * u_out) < 0.0;
    let paint = |fb: &mut Framebuffer,
                 (u0, u1): (f32, f32),
                 (v0, v1): (f32, f32),
                 (z0, z1): (f32, f32),
                 side: Color,
                 top: Color,
                 uface: Option<Color>| {
        // The face at the far end along u (or near end, by what the camera
        // sees), then the face along v, then the top.
        let uu = if u_out > 0.0 { u1 } else { u0 };
        let vv = if v_out > 0.0 { v1 } else { v0 };
        let lit = palette::shade(side, 0.98);
        let dark = palette::shade(side, 0.66);
        let (uc, vc) = if left_is_u { (lit, dark) } else { (dark, lit) };
        let uc = uface.unwrap_or(uc);
        fb.fill_poly(
            &[proj(uu, v0, z1), proj(uu, v1, z1), proj(uu, v1, z0), proj(uu, v0, z0)],
            uc,
        );
        fb.fill_poly(
            &[proj(u0, vv, z1), proj(u1, vv, z1), proj(u1, vv, z0), proj(u0, vv, z0)],
            vc,
        );
        fb.fill_poly(
            &[proj(u0, v0, z1), proj(u1, v0, z1), proj(u1, v1, z1), proj(u0, v1, z1)],
            top,
        );
    };
    let body = spec.body;
    let cab = matches!(spec.cabin, true);
    if cab {
        // Sedan: a body with a hood and a boot, a glazed cabin over the middle.
        let h1 = (hgt * 0.5).round().max(2.0);
        paint(&mut fb, (-hl, hl), (-hw2, hw2), (0.0, h1), body, palette::shade(body, 1.08), None);
        let cu = (-hl * 0.42, hl * 0.30);
        let cv = (-hw2 * 0.80, hw2 * 0.80);
        // Glass on the cabin's visible faces, the roof in the body colour.
        paint(
            &mut fb,
            cu,
            cv,
            (h1, hgt),
            spec.glass,
            spec.roof,
            None,
        );
        // A pillar of body colour at the far corner so the glass reads as
        // windows, not a black block.
        let uu = if u_out > 0.0 { cu.1 } else { cu.0 };
        let vv = if v_out > 0.0 { cv.1 } else { cv.0 };
        let (px0, py0) = proj(uu, vv, h1);
        let (px1, py1) = proj(uu, vv, hgt);
        fb.line((px0, py0), (px1, py1), palette::shade(body, 0.85));
    } else {
        paint(&mut fb, (-hl, hl), (-hw2, hw2), (0.0, hgt), body, spec.roof, None);
        // A windscreen band on the leading face.
        let uu = hl;
        let (v0, v1) = (-hw2 * 0.86, hw2 * 0.86);
        if (u_out > 0.0) && hgt > 4.0 {
            fb.fill_poly(
                &[
                    proj(uu, v0, hgt - 1.0),
                    proj(uu, v1, hgt - 1.0),
                    proj(uu, v1, hgt * 0.45),
                    proj(uu, v0, hgt * 0.45),
                ],
                spec.glass,
            );
        }
        if spec.windows && hgt > 4.0 {
            // A strip of lit windows along the upper half of the near side.
            let vv = if v_out > 0.0 { hw2 } else { -hw2 };
            let steps = (len / 4.0).max(2.0) as i32;
            for k in 0..steps {
                let t0 = -hl + len * k as f32 / steps as f32 + 0.5;
                let t1 = -hl + len * (k as f32 + 0.7) / steps as f32;
                fb.fill_poly(
                    &[
                        proj(t0, vv, hgt - 1.5),
                        proj(t1, vv, hgt - 1.5),
                        proj(t1, vv, hgt * 0.5),
                        proj(t0, vv, hgt * 0.5),
                    ],
                    pal::BUS_WINDOW,
                );
            }
        }
    }
    if spec.wheels {
        // Wheels on the near side: two dark pixels pairs at the bottom.
        let vv = if v_out > 0.0 { hw2 } else { -hw2 };
        for u in [-hl * 0.6, hl * 0.6] {
            let c = proj(u, vv, 1.0);
            fb.fill_rect(c.0 as i32 - 1, c.1 as i32 - 1, 2, 2, pal::CAR_WHEEL);
        }
    }
    let mut s = Sprite::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let c = fb.px()[y * w + x];
            if c != 0 {
                s.set(x, y, c);
            }
        }
    }
    s.set_anchor(ox as i32, oy as i32);
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direction_maps_to_a_sprite() {
        assert_eq!(dir_index((1.0, 0.0)), 0);
        assert_eq!(dir_index((0.0, 1.0)), 1);
        assert_eq!(dir_index((-1.0, 0.0)), 2);
        assert_eq!(dir_index((0.0, -1.0)), 3);
    }

    #[test]
    fn a_sprite_actually_has_pixels() {
        for d in 0..4 {
            let s = sprite_for(Kind::Car, d, 0);
            let set = s.px.iter().filter(|c| **c != 0).count();
            assert!(set >= 6, "dir {d}: only {set} pixels set");
        }
    }

    #[test]
    fn every_kind_has_a_sprite_of_its_own_size() {
        for k in [Kind::Car, Kind::Bus, Kind::Truck, Kind::Emergency] {
            let s = sprite_for(k, 0, 0);
            assert!(s.w > 2 && s.h > 2, "{:?} sprite is {}x{}", k, s.w, s.h);
        }
    }
}

#[cfg(test)]
mod sprite_dump {
    use super::*;
    #[test]
    fn dump() {
        for d in 0..4 {
            let s = sprite_for(Kind::Car, d, 0);
            eprintln!("dir {d}: {}x{}", s.w, s.h);
            for y in 0..s.h {
                let mut row = String::new();
                for x in 0..s.w {
                    let c = s.get(x, y);
                    row.push(if c == 0 { '.' } else if c == pal::CAR_WHEEL { 'W' } else if c == pal::CAR_GLASS { 'G' } else { '#' });
                }
                eprintln!("{row}");
            }
        }
    }
}
