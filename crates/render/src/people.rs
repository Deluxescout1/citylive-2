//! People at true scale (B43/D65): 3 px wide, ~10 tall, with poses.
//!
//! The sim owns who is where and what they are doing; this module only knows
//! how to draw a figure. Every combination of pose, coat colour, walk frame,
//! child/adult and facing is rasterised **once** into a tiny sprite at start-up
//! (about 0.3 MB for the whole set), so a frame of a thousand people is blits,
//! not drawing code. Ambient light is baked in on a palette change, exactly like
//! the pedestrians and the cars (`Life::retint`).
//!
//! Occlusion is the same per-pixel painter key the actors and traffic use: a
//! figure behind a house is hidden by the house, a figure on the pavement in
//! front of it is drawn over it.

use map::TileMap;

use crate::actors::{DepthMode, actor_key};
use crate::framebuffer::{Framebuffer, Sprite};
use crate::iso::{Camera, HEIGHT_UNIT};
use crate::palette::{self, Color, pal};

/// Pose codes (the sim's `people::Pose`, in the same order).
pub const POSE_WALK: u8 = 0;
pub const POSE_STAND: u8 = 1;
pub const POSE_WORK: u8 = 2;
pub const POSE_CARRY: u8 = 3;
pub const POSE_CHAT: u8 = 4;
pub const POSE_PLAY: u8 = 5;
pub const POSE_WAVE: u8 = 6;
pub const POSE_SIT: u8 = 7;
/// B77: not a person — the figure's sprite is blank and only its prop is
/// drawn. It is the weekly rhythm's furniture (a wheelie bin at the kerb, a
/// market stall), carried through the people layer so it is depth-tested and
/// damaged exactly like a resident (D71: no new per-frame system).
pub const POSE_OBJECT: u8 = 8;
pub const POSES: usize = 9;

/// Prop codes (the sim's `people::Prop`).
pub const PROP_NONE: u8 = 0;
pub const PROP_LUMBER: u8 = 1;
pub const PROP_BARROW: u8 = 2;
pub const PROP_DOG: u8 = 3;
pub const PROP_BALL: u8 = 4;
/// B48: a pram, a removal van, and confetti over a waving wedding guest.
pub const PROP_STROLLER: u8 = 5;
pub const PROP_TRUCK: u8 = 6;
pub const PROP_CONFETTI: u8 = 7;
/// B48 follow-up: a flower arch over a wedding couple and a hearse at the
/// cemetery gate.
pub const PROP_ARCH: u8 = 8;
pub const PROP_HEARSE: u8 = 9;
/// B75: a shopping bag and an umbrella (the rain).
pub const PROP_BAG: u8 = 10;
pub const PROP_UMBRELLA: u8 = 11;
/// B77/D84: the weekly rhythm's props — a wheelie bin at the kerb, a market
/// stall, and a cardboard box being carried.
pub const PROP_BIN: u8 = 12;
pub const PROP_STALL: u8 = 13;
pub const PROP_BOX: u8 = 14;
const PROPS: usize = 15;

/// 0-7 civilian coats; 8 and 9 are hi-vis vests with a hard hat (B43c); 10 is the
/// bride's white and 11 a funeral's dark (B48).
/// B62: 12 and 13 are the two gameday jerseys (blue, red) and 14 a marching
/// band's uniform (maroon, with a tall hat and a brass instrument).
/// B75: 15-17 are the elders — three muted coats and grey hair, so age reads on
/// the pavement at 1x (D83: attribute a figure's identity at a glance).
pub const TONES: usize = 18;

/// One figure to draw this frame.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Figure {
    pub x: f32,
    pub y: f32,
    pub pose: u8,
    pub tone: u8,
    pub child: bool,
    pub prop: u8,
    /// 0 = +x, 1 = +y, 2 = -x, 3 = -y.
    pub dir: u8,
    /// On an open lot (park, construction site): paint over the plot's own
    /// painter key, or the lawn would hide everyone standing on it.
    pub open: bool,
    /// Feet this many pixels above the ground (a hand on the scaffold).
    pub lift: u8,
    /// Which sprite frame the walk/work cycle is on (filled by the scene when
    /// the list is set, so `draw` and the damage check use the same answer).
    pub frame: u8,
}

impl Figure {
    /// Would this figure paint the same pixels as `self` did on the previous
    /// frame? Position to the pixel, pose, coat, prop and animation frame.
    /// D71: an unchanged figure costs no composite work at all.
    pub fn same_pixel(&self, f: &Figure, frame: u8) -> bool {
        self.frame == frame
            && self.pose == f.pose
            && self.tone == f.tone
            && self.child == f.child
            && self.prop == f.prop
            && self.dir == f.dir
            && self.open == f.open
            && self.lift == f.lift
            && self.x.to_bits() == f.x.to_bits()
            && self.y.to_bits() == f.y.to_bits()
    }
}

const SPR_W: usize = 9;
const SPR_H: usize = 15;
/// Frames per pose: walking and working animate; the rest are static.
const FRAMES: usize = 2;

fn frames_of(pose: u8) -> usize {
    match pose {
        POSE_WALK | POSE_WORK | POSE_WAVE | POSE_PLAY => FRAMES,
        _ => 1,
    }
}

/// The animation clock as the scene wants it: sim seconds, wrapped to a week.
///
/// An `f32` holds 24 bits: at day 90 of a world (7.8 million sim seconds) the
/// clock moves in half-second steps, so the walk cycle (2.6 frames a second) lurched
/// by a frame or two at a time (B62 noted it, B67 measured it). A week is
/// `604 800 * 2.6 = 1 572 480` frames, divisible by every cycle length there is,
/// so wrapping at a week is invisible, the hour and the weekday the rigs read
/// from it are unchanged, and the precision stays at a sixteenth of a second.
pub fn clock_f32(t: f64) -> f32 {
    t.rem_euclid(604_800.0) as f32
}

/// The walk/work cycle frame a figure shows at animation clock `t`. Shared by
/// the renderer and by the scene's "did it change a pixel" test, so the two can
/// never disagree.
pub fn walk_frame(f: &Figure, t: f32) -> u8 {
    let n = frames_of(f.pose);
    if n > 1 {
        (((t * 2.6) as i32 + (f.y.floor() as i32 + f.x.floor() as i32) * 3).rem_euclid(n as i32))
            as u8
    } else {
        0
    }
}

/// Coat colours: the palette's shirt set plus a work jacket and a raincoat, so
/// a crowd is not six identical people.
fn coat(tone: u8) -> Color {
    match tone % TONES as u8 {
        0..=5 => pal::PEOPLE_SHIRT[(tone as usize) % pal::PEOPLE_SHIRT.len()],
        6 => palette::rgb(0x6a, 0x5a, 0x4a),
        7 => palette::rgb(0xb8, 0x6a, 0x2e),
        8 => pal::HIVIS,
        9 => pal::HIVIS_O,
        10 => palette::rgb(0xf4, 0xf0, 0xe8),
        12 => palette::rgb(0x1f, 0x4c, 0xc0),
        13 => palette::rgb(0xd0, 0x22, 0x30),
        14 => palette::rgb(0x8a, 0x20, 0x30),
        // B75: the elders' three coats — a grey cardigan, a fawn mac and a
        // plum one — muted enough to read as older, clear enough to see.
        15 => palette::rgb(0x9a, 0x9a, 0xa2),
        16 => palette::rgb(0xc2, 0xa8, 0x84),
        17 => palette::rgb(0x8a, 0x72, 0x92),
        _ => palette::rgb(0x2a, 0x2a, 0x32),
    }
}

/// B75: an elder's hair is silver. Everyone else keeps the four hair colours.
fn is_elder(tone: u8) -> bool {
    (15..18).contains(&(tone % TONES as u8))
}

fn skin(tone: u8) -> Color {
    match tone % 3 {
        0 => pal::PEOPLE_HEAD,
        1 => palette::rgb(0xd8, 0xb0, 0x8a),
        _ => palette::rgb(0xb0, 0x82, 0x60),
    }
}

fn hair(tone: u8) -> Color {
    if is_elder(tone) {
        return palette::rgb(0xd6, 0xd2, 0xca);
    }
    match tone % 4 {
        0 => palette::rgb(0x3a, 0x2c, 0x22),
        1 => palette::rgb(0x5a, 0x42, 0x2c),
        2 => palette::rgb(0x8a, 0x74, 0x50),
        _ => palette::rgb(0x22, 0x1e, 0x1c),
    }
}

fn trousers() -> Color {
    pal::PEOPLE_LEGS
}

/// Rasterise one figure into a sprite. `frame` is the walk/arm cycle, `mirror`
/// flips it for a face the other way.
fn sprite(pose: u8, tone: u8, frame: usize, child: bool, mirror: bool) -> Sprite {
    // B77: an object figure has no body — its prop is the whole picture.
    if pose == POSE_OBJECT {
        let mut s = Sprite::new(1, 1);
        s.set_anchor(0, 0);
        return s;
    }
    let mut s = Sprite::new(SPR_W, SPR_H);
    s.set_anchor((SPR_W / 2) as i32, (SPR_H - 1) as i32);
    let c = coat(tone);
    let c_dark = palette::shade(c, 0.78);
    let c_lit = palette::shade(c, 1.12);
    let sk = skin(tone);
    let hr = hair(tone);
    // The bride's dress falls to the ground in her own white, with a veil;
    // everyone else wears trousers.
    let bride = tone as usize % TONES == 10;
    let lg = if bride { palette::shade(c, 0.9) } else { trousers() };
    let hr = if bride { palette::rgb(0xe8, 0xe6, 0xe2) } else { hr };
    // Ground rows: an adult is 12 px tall from the last row up; a child 8
    // (B75: one pixel taller than B43 so a lone figure reads on a 64-px tile).
    let feet = SPR_H as i32 - 1; // 14
    let h = if child { 8 } else { 12 };
    let top = feet - h + 1; // first body row
    let cx = 4i32;
    let mut put = |x: i32, y: i32, col: Color| {
        let x = if mirror { SPR_W as i32 - 1 - x } else { x };
        if x >= 0 && y >= 0 {
            s.set(x as usize, y as usize, col);
        }
    };
    match pose {
        POSE_SIT => {
            // A bench under the sitter, legs forward.
            let by = top + 4;
            for dx in -3..=3 {
                put(cx + dx, by, palette::rgb(0x7a, 0x58, 0x38));
                put(cx + dx, by + 1, palette::rgb(0x54, 0x3c, 0x26));
            }
            put(cx - 2, by + 2, palette::rgb(0x54, 0x3c, 0x26));
            put(cx + 2, by + 2, palette::rgb(0x54, 0x3c, 0x26));
            // Seated torso (shorter) and head.
            put(cx, top, hr);
            put(cx - 1, top + 1, sk);
            put(cx, top + 1, sk);
            put(cx + 1, top + 1, sk);
            put(cx - 1, top + 2, c_dark);
            put(cx, top + 2, c);
            put(cx + 1, top + 2, c);
            put(cx, top + 3, c);
            put(cx, top + 4, lg);
            put(cx + 1, top + 4, lg);
            put(cx + 2, top + 4, lg);
            return s;
        }
        _ => {}
    }
    // Head.
    put(cx, top, hr);
    put(cx - 1, top, hr);
    put(cx, top + 1, sk);
    put(cx - 1, top + 1, sk);
    put(cx + 1, top + 1, sk);
    if pose == POSE_CHAT {
        // Turned: the eye is on the far side of the face.
        put(cx + 1, top + 2, sk);
        put(cx, top + 2, palette::rgb(0x3a, 0x2c, 0x22));
    } else {
        put(cx, top + 2, sk);
        put(cx + 1, top + 2, sk);
    }
    // B43c: a hard hat (yellow, or white for the foreman) over the hair.
    if matches!(tone % TONES as u8, 8 | 9) && !child {
        let hat = if tone % 2 == 0 { pal::HARDHAT } else { pal::HARDHAT_W };
        put(cx - 1, top, hat);
        put(cx, top, hat);
        put(cx + 1, top, palette::shade(hat, 0.82));
        put(cx - 2, top + 1, palette::shade(hat, 0.82));
    }
    // B62: a band member's tall hat with a plume, and a brass horn out front.
    if tone % TONES as u8 == 14 && !child {
        for dx in -1..=1 {
            put(cx + dx, top - 1, palette::rgb(0xe8, 0xc8, 0x50));
        }
        put(cx, top - 2, palette::rgb(0xf4, 0xf4, 0xf4));
    }
    // Torso.
    let torso = top + 3;
    put(cx - 1, torso, c_dark);
    put(cx, torso, c);
    put(cx + 1, torso, c_lit);
    put(cx - 1, torso + 1, c);
    put(cx, torso + 1, c);
    put(cx + 1, torso + 1, c_dark);
    put(cx, torso + 2, c);
    // Arms and legs depend on the pose.
    let leg_top = torso + 3;
    match pose {
        POSE_WALK => {
            // Arms swing with the legs.
            if frame == 0 {
                put(cx - 2, torso, c_dark);
                put(cx - 2, torso + 1, c_dark);
                put(cx - 2, leg_top + 1, lg);
                put(cx, leg_top + 1, lg);
                put(cx + 1, leg_top, lg);
            } else {
                put(cx + 2, torso, c_lit);
                put(cx + 1, leg_top + 1, lg);
                put(cx, leg_top, lg);
                put(cx - 1, leg_top + 1, lg);
            }
        }
        POSE_WORK => {
            // One arm works: up on frame 0, down on frame 1, with a tool.
            let swing = frame == 0;
            put(cx - 2, torso, c_dark);
            put(cx - 2, torso + 1, c_dark);
            if swing {
                put(cx + 2, torso - 1, c_lit);
                put(cx + 3, torso - 2, sk);
                put(cx + 3, torso - 3, palette::rgb(0x8a, 0x8a, 0x92));
            } else {
                put(cx + 2, torso + 1, c_lit);
                put(cx + 3, torso + 2, sk);
                put(cx + 3, torso + 3, palette::rgb(0xc8, 0xcc, 0xd4));
            }
            put(cx - 1, leg_top, lg);
            put(cx + 1, leg_top, lg);
        }
        POSE_CARRY => {
            // Both arms up holding a plank across the chest.
            put(cx - 2, torso, c_dark);
            put(cx + 2, torso, c_lit);
            for dx in -3..=3 {
                put(cx + dx, torso - 1, palette::rgb(0xb0, 0x84, 0x4e));
                put(cx + dx, torso, palette::rgb(0x8a, 0x62, 0x36));
            }
            put(cx - 1, leg_top, lg);
            put(cx + 1, leg_top, lg);
        }
        POSE_CHAT => {
            put(cx - 2, torso, c_dark);
            put(cx + 2, torso + 1, c_lit);
            put(cx - 1, leg_top, lg);
            put(cx + 1, leg_top, lg);
        }
        POSE_PLAY => {
            // Arms out, legs apart, running about.
            put(cx - 2, torso - 1, c_dark);
            put(cx + 2, torso - 1, c_lit);
            put(cx - 2, leg_top + (frame as i32), lg);
            put(cx + 2, leg_top + (1 - frame as i32), lg);
        }
        POSE_WAVE => {
            put(cx - 2, torso + 1, c_dark);
            // The waving arm: up, and the hand moves with the frame.
            put(cx + 2, torso, c_lit);
            put(cx + 3, torso - 1, c_lit);
            put(cx + 3 + frame as i32, torso - 2, sk);
            put(cx - 1, leg_top, lg);
            put(cx + 1, leg_top, lg);
        }
        _ => {
            // Standing: arms down, feet together.
            put(cx - 2, torso + 1, c_dark);
            put(cx + 2, torso + 1, c_lit);
            put(cx - 1, leg_top, lg);
            put(cx + 1, leg_top, lg);
        }
    }
    if tone % TONES as u8 == 14 && !child {
        let brass = palette::rgb(0xf0, 0xc8, 0x40);
        put(cx + 2, torso, brass);
        put(cx + 3, torso, brass);
        put(cx + 3, torso - 1, palette::shade(brass, 1.2));
    }
    // A skirt flares at the hem.
    if bride && pose != POSE_SIT {
        put(cx - 2, feet - 1, lg);
        put(cx + 2, feet - 1, lg);
        put(cx, feet - 1, lg);
        put(cx, feet - 2, lg);
    }
    // Feet.
    put(cx - 1, feet, palette::rgb(0x2e, 0x2a, 0x2a));
    put(cx + 1, feet, palette::rgb(0x2e, 0x2a, 0x2a));
    // Legs: two rows, so a figure has visible legs at 1×.
    if pose != POSE_PLAY {
        put(cx - 1, leg_top + 1, lg);
        put(cx + 1, leg_top + 1, lg);
    }
    s
}

/// A prop drawn beside the figure: a dog, a ball, a wheelbarrow.
fn prop_sprite(prop: u8, tone: u8, frame: usize, mirror: bool) -> Option<Sprite> {
    if prop == PROP_NONE || prop == PROP_LUMBER {
        return None;
    }
    if prop == PROP_TRUCK {
        return Some(truck_sprite(mirror));
    }
    if prop == PROP_HEARSE {
        return Some(hearse_sprite(mirror));
    }
    if prop == PROP_ARCH {
        return Some(arch_sprite(mirror));
    }
    if prop == PROP_UMBRELLA {
        return Some(umbrella_sprite(mirror));
    }
    if prop == PROP_STALL {
        return Some(stall_sprite(tone, mirror));
    }
    let mut s = Sprite::new(SPR_W, SPR_H);
    s.set_anchor((SPR_W / 2) as i32, (SPR_H - 1) as i32);
    let feet = (SPR_H - 1) as i32;
    let mut put = |x: i32, y: i32, col: Color| {
        let x = if mirror { SPR_W as i32 - 1 - x } else { x };
        if x >= 0 && y >= 0 && (x as usize) < SPR_W && (y as usize) < SPR_H {
            s.set(x as usize, y as usize, col);
        }
    };
    match prop {
        PROP_DOG => {
            let bx = 4 + frame as i32 % 2;
            let fur = palette::rgb(0x6a, 0x4e, 0x34);
            put(bx, feet - 2, fur);
            put(bx + 1, feet - 2, fur);
            put(bx + 2, feet - 2, fur);
            put(bx + 2, feet - 3, fur); // head
            put(bx + 1, feet - 1, fur);
            put(bx + 2, feet - 1, fur);
            put(bx + 3, feet - 3, palette::rgb(0x8a, 0x6a, 0x48)); // tail
            let _ = tone;
        }
        PROP_BALL => {
            let c = palette::rgb(0xd8, 0x3a, 0x2e);
            put(4, feet - 1, c);
            put(5, feet - 1, c);
            put(4, feet - 2, palette::rgb(0xf0, 0xe8, 0xd8));
        }
        PROP_STROLLER => {
            // A pram pushed in front: a hooded basket on two wheels, the
            // handle up to the parent's hands.
            let hood = palette::rgb(0x7a, 0x9a, 0xc4);
            let body = palette::rgb(0xb4, 0xca, 0xe6);
            let dark = palette::rgb(0x34, 0x34, 0x3c);
            put(5, feet - 5, dark);
            put(6, feet - 4, dark);
            put(7, feet - 4, hood);
            put(8, feet - 4, hood);
            put(6, feet - 3, body);
            put(7, feet - 3, body);
            put(8, feet - 3, hood);
            put(6, feet - 2, body);
            put(7, feet - 2, body);
            put(8, feet - 2, palette::shade(body, 0.85));
            put(6, feet - 1, dark);
            put(8, feet - 1, dark);
            let _ = tone;
        }
        PROP_CONFETTI => {
            // Bits of colour in the air round a raised hand; the second frame
            // has fallen a pixel.
            let cols = [
                palette::rgb(0xe8, 0x4a, 0x5a),
                palette::rgb(0xf2, 0xd2, 0x4a),
                palette::rgb(0x5a, 0xa8, 0xe8),
                palette::rgb(0xf2, 0x8a, 0xc0),
                palette::rgb(0x6a, 0xd0, 0x7a),
            ];
            let at = [(2, 0), (7, 1), (1, 3), (8, 4), (4, 1), (6, 5), (0, 1), (3, 5)];
            for (k, (x, y)) in at.iter().enumerate() {
                let (x, y) = (*x, *y + frame as i32 * (1 + (k as i32 % 2)));
                put(x, y, cols[k % cols.len()]);
            }
        }
        PROP_BARROW => {
            let wood = palette::rgb(0x8a, 0x62, 0x36);
            for dx in -2..=2 {
                put(4 + dx, feet - 4, wood);
                put(4 + dx, feet - 3, palette::rgb(0x6a, 0x4a, 0x28));
            }
            put(6, feet - 2, palette::rgb(0x3a, 0x3a, 0x3a)); // wheel
            put(1, feet - 3, palette::rgb(0x9a, 0x9a, 0x9a)); // shovel load
        }
        PROP_BAG => {
            // A paper carrier bag hanging from the hand: two pale rows and a
            // dark handle, on the near side of the figure (B75: a shopper).
            let bag = palette::rgb(0xd8, 0xc8, 0xa0);
            let bag_d = palette::rgb(0xb0, 0x9e, 0x78);
            put(6, feet - 4, palette::rgb(0x5a, 0x4a, 0x3a));
            put(6, feet - 3, bag);
            put(7, feet - 3, bag_d);
            put(6, feet - 2, bag);
            put(7, feet - 2, bag_d);
            put(6, feet - 1, bag_d);
        }
        PROP_BIN => {
            // B77: a wheelie bin standing at the kerb. Three households, three
            // bins (green, black, blue — the colours a UK street actually has),
            // a light lid so the bin reads against dark tarmac at 1x.
            let (body, body_d, body_l) = match tone as usize % 3 {
                0 => (
                    palette::rgb(0x4e, 0x8a, 0x46),
                    palette::rgb(0x3a, 0x68, 0x36),
                    palette::rgb(0x6f, 0xaa, 0x5e),
                ),
                1 => (
                    palette::rgb(0x3c, 0x42, 0x46),
                    palette::rgb(0x2a, 0x2e, 0x32),
                    palette::rgb(0x5c, 0x64, 0x68),
                ),
                _ => (
                    palette::rgb(0x3c, 0x5c, 0x8a),
                    palette::rgb(0x2c, 0x44, 0x68),
                    palette::rgb(0x5e, 0x82, 0xb4),
                ),
            };
            let lid = palette::rgb(0xa8, 0xae, 0xa4);
            let x0 = 0;
            for y in feet - 10..=feet - 3 {
                put(x0, y, body_d);
                put(x0 + 1, y, body);
                put(x0 + 2, y, body);
                put(x0 + 3, y, body_l);
                put(x0 + 4, y, body_l);
                put(x0 + 5, y, body_d);
            }
            for x in x0..=x0 + 5 {
                put(x, feet - 11, lid);
            }
            for x in x0 + 1..=x0 + 4 {
                put(x, feet - 12, palette::shade(lid, 0.82));
            }
            put(x0 + 5, feet - 13, lid); // the handle, up
            put(x0, feet - 2, palette::rgb(0x1a, 0x1a, 0x1e));
            put(x0 + 5, feet - 2, palette::rgb(0x1a, 0x1a, 0x1e));
        }
        PROP_BOX => {
            // B77: a cardboard box carried in both arms (a moving day).
            let box_c = palette::rgb(0xb8, 0x8e, 0x58);
            let box_l = palette::rgb(0xcd, 0xa2, 0x66);
            let box_d = palette::rgb(0x8e, 0x6a, 0x3e);
            for y in feet - 8..=feet - 3 {
                for x in 2..=7 {
                    let c = if y == feet - 8 {
                        box_l
                    } else if x == 7 {
                        box_d
                    } else {
                        box_c
                    };
                    put(x, y, c);
                }
            }
            // The tape down the middle and a hand under it.
            for y in feet - 8..=feet - 3 {
                put(4, y, palette::rgb(0xd8, 0xc8, 0x9a));
            }
            put(3, feet - 2, palette::rgb(0xe8, 0xc4, 0xa0));
        }
        _ => {}
    }
    Some(s)
}

/// B75: an umbrella over a figure in the rain. Its own, taller sprite: a domed
/// canopy in two blues above the head and a stem down to the hand.
fn umbrella_sprite(mirror: bool) -> Sprite {
    const W: usize = 15;
    const H: usize = 26;
    let mut s = Sprite::new(W, H);
    s.set_anchor((W / 2) as i32, (H - 1) as i32);
    let feet = (H - 1) as i32;
    let mut put = |x: i32, y: i32, col: Color| {
        let x = if mirror { W as i32 - 1 - x } else { x };
        if x >= 0 && y >= 0 && (x as usize) < W && (y as usize) < H {
            s.set(x as usize, y as usize, col);
        }
    };
    let canopy = palette::rgb(0x36, 0x52, 0x84);
    let canopy_l = palette::rgb(0x54, 0x76, 0xac);
    let canopy_d = palette::rgb(0x26, 0x3c, 0x62);
    // The dome: rows above the head, widest at the rim.
    for (dy, half) in [(13i32, 1i32), (14, 2), (15, 3), (16, 4), (17, 5), (18, 6)] {
        for dx in -half..=half {
            let c = if dx.abs() == half { canopy_d } else if dx % 3 == 0 { canopy_l } else { canopy };
            put(7 + dx, feet - dy, c);
        }
    }
    // The stem, and a hand on it.
    for dy in 4..=12 {
        put(7, feet - dy, palette::rgb(0x4a, 0x42, 0x38));
    }
    s
}

/// A removal van in isometric (B48), long axis along +x (down and right on
/// screen); the mirrored sprite runs along +y. Built from three boxes projected
/// with the engine's own 2:1 diamond, so it sits on the street like a car does.
/// B77/D84: a market stall — a striped awning over a trestle table with boxes
/// of produce on it. Two awning colourways by tone (a red-and-cream and a
/// blue-and-cream canvas), so a row of stalls along a square is not one stamp
/// repeated. `mirror` turns it the other way along the square.
fn stall_sprite(tone: u8, mirror: bool) -> Sprite {
    const W: usize = 46;
    const H: usize = 46;
    const OX: i32 = 23;
    const OY: i32 = 36;
    let mut s = Sprite::new(W, H);
    let mut put = |x: i32, y: i32, z: i32, col: Color| {
        let sx = x - y + OX;
        let sy = (x + y) / 2 - z + OY;
        let sx = if mirror { W as i32 - 1 - sx } else { sx };
        if sx >= 0 && sy >= 0 && (sx as usize) < W && (sy as usize) < H {
            s.set(sx as usize, sy as usize, col);
        }
    };
    // The awning's stripes: two markets, two canvases.
    let (a, b) = if tone % 2 == 0 {
        (palette::rgb(0xc2, 0x3e, 0x36), palette::rgb(0xef, 0xe4, 0xc8))
    } else {
        (palette::rgb(0x33, 0x5c, 0x96), palette::rgb(0xe8, 0xe6, 0xdc))
    };
    let wood = palette::rgb(0x8a, 0x66, 0x3c);
    let wood_d = palette::rgb(0x6a, 0x4c, 0x2c);
    // The counter: a solid front panel from the ground to z 8.
    for x in 0..18 {
        for z in 0..8 {
            put(x, 10, z, if x % 5 == 0 { wood_d } else { wood });
        }
    }
    // Its top, in two rows of depth.
    for x in 0..18 {
        for y in 9..12 {
            put(x, y, 8, palette::shade(wood, 1.15));
        }
    }
    // Goods: crates and produce in the stall's own colours.
    let goods = [
        palette::rgb(0xc8, 0x4c, 0x2c),  // tomatoes
        palette::rgb(0xe0, 0xb4, 0x38),  // grain
        palette::rgb(0x66, 0x8c, 0x36),  // greens
        palette::rgb(0x8a, 0x42, 0x74),  // plums
    ];
    for k in 0..4u32 {
        let x0 = 2 + k as i32 * 4;
        let c = goods[(k as usize + tone as usize) % goods.len()];
        for x in x0..x0 + 3 {
            for y in 9..11 {
                put(x, y, 9, palette::rgb(0x9a, 0x76, 0x48));
            }
        }
        for x in x0..x0 + 3 {
            put(x, 9, 10, c);
        }
    }
    // Four posts up to the awning.
    for (px, py) in [(0, 11), (17, 11), (0, 1), (17, 1)] {
        for z in 8..22 {
            put(px, py, z, palette::rgb(0x5a, 0x5a, 0x5e));
        }
    }
    // The awning: a striped canvas, edge-on, tipped back over the counter.
    for x in 0..19 {
        for y in 1..12 {
            let stripe = ((x + if mirror { 0 } else { 0 }) / 3) % 2 == 0;
            put(x, y, 22, if stripe { a } else { b });
        }
    }
    // Its falling front edge, two rows deep so the awning reads as a roof.
    for x in 0..19 {
        let stripe = (x / 3) % 2 == 0;
        for z in 20..22 {
            put(x, 12, z, if stripe { a } else { b });
        }
    }
    let ax = if mirror { W as i32 - 1 - OX } else { OX };
    s.set_anchor(ax, OY);
    s
}

fn truck_sprite(mirror: bool) -> Sprite {
    const W: usize = 44;
    const H: usize = 40;
    const OX: i32 = 10;
    const OY: i32 = 17;
    let mut s = Sprite::new(W, H);
    let (ax, ay) = (21i32, 27i32);
    let mut put = |x: i32, y: i32, z: i32, col: Color| {
        let sx = x - y + OX;
        let sy = (x + y) / 2 - z + OY;
        let sx = if mirror { W as i32 - 1 - sx } else { sx };
        if sx >= 0 && sy >= 0 && (sx as usize) < W && (sy as usize) < H {
            s.set(sx as usize, sy as usize, col);
        }
    };
    // (x0, x1, y0, y1, z0, z1, top, +y face, +x face)
    let cargo = (0, 22, 0, 9, 2, 16);
    let cab = (22, 31, 0, 9, 2, 11);
    let cream = palette::rgb(0xe6, 0xdf, 0xcc);
    let blue = palette::rgb(0x3a, 0x62, 0x98);
    for (b, top, l, r) in [
        (cargo, palette::shade(cream, 1.06), palette::shade(cream, 0.86), palette::shade(cream, 0.72)),
        (cab, palette::shade(blue, 1.15), palette::shade(blue, 0.95), palette::shade(blue, 0.78)),
    ] {
        let (x0, x1, y0, y1, z0, z1) = b;
        for x in x0..x1 {
            for z in z0..z1 {
                put(x, y1, z, l);
            }
        }
        for y in y0..y1 {
            for z in z0..z1 {
                put(x1, y, z, r);
            }
        }
        for x in x0..=x1 {
            for y in y0..=y1 {
                put(x, y, z1, top);
            }
        }
    }
    // A red stripe along the cargo box, a windscreen and a side window.
    for x in 0..22 {
        put(x, 9, 8, palette::rgb(0xc8, 0x3a, 0x30));
        put(x, 9, 9, palette::rgb(0xc8, 0x3a, 0x30));
    }
    for y in 1..8 {
        for z in 6..9 {
            put(31, y, z, palette::rgb(0xa8, 0xc8, 0xe0));
        }
    }
    for x in 24..30 {
        for z in 6..9 {
            put(x, 9, z, palette::rgb(0xa8, 0xc8, 0xe0));
        }
    }
    // Wheels on the near side.
    for cx in [4, 16, 26] {
        for dx in 0..3 {
            for z in 0..3 {
                put(cx + dx, 9, z, palette::rgb(0x24, 0x24, 0x28));
            }
        }
    }
    let ax = if mirror { W as i32 - 1 - ax } else { ax };
    s.set_anchor(ax, ay);
    s
}

/// A hearse in isometric, long axis along +x like the van: black body, a long
/// glass band, a chrome stripe and a few white and pink flowers on the roof.
fn hearse_sprite(mirror: bool) -> Sprite {
    const W: usize = 38;
    const H: usize = 32;
    const OX: i32 = 8;
    const OY: i32 = 11;
    let mut s = Sprite::new(W, H);
    let (ax, ay) = (17i32, 20i32);
    let mut put = |x: i32, y: i32, z: i32, col: Color| {
        let sx = x - y + OX;
        let sy = (x + y) / 2 - z + OY;
        let sx = if mirror { W as i32 - 1 - sx } else { sx };
        if sx >= 0 && sy >= 0 && (sx as usize) < W && (sy as usize) < H {
            s.set(sx as usize, sy as usize, col);
        }
    };
    let black = palette::rgb(0x2a, 0x2a, 0x32);
    // Cabin and long tail, then the lower bonnet.
    for (b, k) in [((0, 20, 0, 8, 2, 11), 1.0f32), ((20, 27, 0, 8, 2, 7), 1.0)] {
        let (x0, x1, y0, y1, z0, z1) = b;
        for x in x0..x1 {
            for z in z0..z1 {
                put(x, y1, z, palette::shade(black, 0.95 * k));
            }
        }
        for y in y0..y1 {
            for z in z0..z1 {
                put(x1, y, z, palette::shade(black, 0.75 * k));
            }
        }
        for x in x0..=x1 {
            for y in y0..=y1 {
                put(x, y, z1, palette::shade(black, 1.35 * k));
            }
        }
    }
    let glass = palette::rgb(0x8a, 0x9a, 0xaa);
    for x in 3..18 {
        for z in 6..9 {
            put(x, 8, z, glass);
        }
    }
    for x in 0..27 {
        put(x, 8, 4, palette::rgb(0xc0, 0xc4, 0xca));
    }
    // Flowers on the roof.
    for (x, y, c) in [
        (6, 3, (0xf4, 0xf0, 0xe8)),
        (8, 4, (0xf2, 0x8a, 0xc0)),
        (10, 3, (0xf4, 0xf0, 0xe8)),
        (12, 4, (0xf2, 0x8a, 0xc0)),
        (14, 3, (0xf4, 0xf0, 0xe8)),
    ] {
        put(x, y, 11, palette::rgb(c.0, c.1, c.2));
        put(x, y, 12, palette::rgb(c.0, c.1, c.2));
    }
    for cx in [3, 20] {
        for dx in 0..3 {
            for z in 0..3 {
                put(cx + dx, 8, z, palette::rgb(0x18, 0x18, 0x1c));
            }
        }
    }
    let ax = if mirror { W as i32 - 1 - ax } else { ax };
    s.set_anchor(ax, ay);
    s
}

/// A flower arch over a wedding couple: two posts and a curved top in green,
/// studded with white and pink blooms. Drawn for an arch running along the
/// screen's "\" diagonal (the frontage along x); mirrored for the other.
fn arch_sprite(mirror: bool) -> Sprite {
    const W: usize = 16;
    const H: usize = 22;
    let mut s = Sprite::new(W, H);
    let mut put = |x: i32, y: i32, col: Color| {
        let x = if mirror { W as i32 - 1 - x } else { x };
        if x >= 0 && y >= 0 && (x as usize) < W && (y as usize) < H {
            s.set(x as usize, y as usize, col);
        }
    };
    let leaf = palette::rgb(0x4a, 0x84, 0x48);
    let leaf_d = palette::rgb(0x34, 0x62, 0x34);
    let petals = [
        palette::rgb(0xf4, 0xf0, 0xe8),
        palette::rgb(0xf2, 0x8a, 0xc0),
        palette::rgb(0xf4, 0xf0, 0xe8),
        palette::rgb(0xf2, 0xd2, 0x4a),
    ];
    // Posts stand on the "\" diagonal: the left foot is higher on screen
    // than the right, by half a pixel per pixel across.
    let (lx, rx) = (2i32, 13i32);
    let foot = |x: i32| 14 + (x - lx) / 2; // ground row at post x
    for (px, _) in [(lx, 0), (rx, 0)] {
        let g = foot(px);
        for k in 0..11 {
            put(px, g - k, if k % 2 == 0 { leaf } else { leaf_d });
        }
    }
    // The top: a shallow curve between the two post tops.
    for x in lx..=rx {
        let t = (x - lx) as f32 / (rx - lx) as f32;
        let top_l = foot(lx) - 10;
        let top_r = foot(rx) - 10;
        let y = top_l as f32 + (top_r - top_l) as f32 * t - (1.0 - (2.0 * t - 1.0).powi(2)) * 3.0;
        put(x, y.round() as i32, leaf);
        put(x, y.round() as i32 + 1, leaf_d);
    }
    // Blooms along the top and up the posts.
    for (k, x) in (lx..=rx).step_by(2).enumerate() {
        let t = (x - lx) as f32 / (rx - lx) as f32;
        let y = foot(lx) as f32 - 10.0 + ((foot(rx) - foot(lx)) as f32) * t
            - (1.0 - (2.0 * t - 1.0).powi(2)) * 3.0;
        put(x, y.round() as i32 - 1, petals[k % 4]);
    }
    for (k, g) in [foot(lx) - 7, foot(lx) - 4, foot(rx) - 6, foot(rx) - 3].iter().enumerate() {
        put(if k < 2 { lx } else { rx }, *g, petals[(k + 1) % 4]);
    }
    s.set_anchor((lx + rx) / 2, 16);
    s
}

/// Materials at a building site (B43): a plank stack, a brick pallet, a heap of
/// sand. Drawn under the crew, so a site reads as work from a glance even when
/// the frame itself is still bare.
fn site_sprite(kind: u8) -> Sprite {
    let mut s = Sprite::new(16, 12);
    s.set_anchor(8, 11);
    let ink = palette::rgb(0x3a, 0x36, 0x30);
    match kind % 3 {
        0 => {
            // Sawn planks, two courses, pale ends towards the viewer.
            for row in 0..2 {
                let y = 9 - row as i32 * 3;
                for dx in -6i32..=6 {
                    let c = if (dx + row as i32) % 3 == 0 {
                        palette::rgb(0xe0, 0xc0, 0x8c)
                    } else {
                        palette::rgb(0xb0, 0x84, 0x4e)
                    };
                    s.set((8 + dx) as usize, y as usize, c);
                    s.set((8 + dx) as usize, (y - 1) as usize, palette::shade(c, 0.8));
                }
                s.set((8 - 6) as usize, (y - 1) as usize, ink);
                s.set((8 + 6) as usize, (y - 1) as usize, ink);
            }
        }
        1 => {
            // A pallet of bricks.
            for row in 0..3i32 {
                let y = 9 - row * 2;
                for dx in -4i32..=4 {
                    let c = if (dx + row) % 2 == 0 {
                        palette::rgb(0xa8, 0x4a, 0x3a)
                    } else {
                        palette::rgb(0x8a, 0x3c, 0x30)
                    };
                    s.set((8 + dx) as usize, y as usize, c);
                    s.set((8 + dx) as usize, (y - 1) as usize, c);
                }
            }
            for dx in -5i32..=5 {
                s.set((8 + dx) as usize, 10, palette::rgb(0x8a, 0x62, 0x36));
            }
        }
        _ => {
            // A sand/cement heap with a shovel standing in it.
            for (dx, dy, w) in [(-3i32, 0i32, 7i32), (-2, 1, 5), (-1, 2, 3)] {
                for k in 0..w {
                    s.set((8 + dx + k) as usize, (10 - dy) as usize, palette::rgb(0xc8, 0xb0, 0x80));
                }
            }
            s.set(11, 6, palette::rgb(0x6a, 0x5a, 0x48));
            s.set(11, 7, palette::rgb(0x9a, 0x9a, 0x92));
        }
    }
    s
}

pub struct PeopleRenderer {
    /// Sprites indexed [pose][tone][frame][child][mirror].
    sprites: Vec<Sprite>,
    props: Vec<Option<Sprite>>,
    sites: Vec<Sprite>,
    base: Vec<Sprite>,
    base_props: Vec<Option<Sprite>>,
    base_sites: Vec<Sprite>,
    tint: [f32; 3],
    /// B76: winter — everyone is in a heavy coat, not a summer shirt.
    winter: bool,
}

fn index(pose: usize, tone: usize, frame: usize, child: usize, mirror: usize) -> usize {
    ((pose * TONES + tone) * FRAMES + frame) * 4 + child * 2 + mirror
}

/// Where a prop's sprite lives in the table: props are grouped by code, then
/// tone, frame and mirror (B48: this used to subtract one from the code, so a
/// dog was drawn with the barrow's sprite, a ball with the dog's).
fn prop_index(prop: u8, tone: usize, frame: usize, mirror: usize) -> usize {
    (prop as usize).min(PROPS - 1) * TONES * FRAMES * 2 + tone * FRAMES * 2 + frame * 2 + mirror
}

const SLOTS: usize = POSES * TONES * FRAMES * 2 * 2;

impl PeopleRenderer {
    /// B61f: the bytes the pose table holds (for the memory report).
    pub fn bytes(&self) -> usize {
        let one = |v: &Vec<Sprite>| v.iter().map(|s| s.bytes()).sum::<usize>();
        let prop = |v: &Vec<Option<Sprite>>| {
            v.iter().flatten().map(|s| s.bytes()).sum::<usize>()
        };
        one(&self.sprites) + prop(&self.props) + one(&self.sites) + one(&self.base)
            + prop(&self.base_props) + one(&self.base_sites)
    }

    pub fn new() -> Self {
        let mut sprites = Vec::with_capacity(SLOTS);
        for _ in 0..SLOTS {
            sprites.push(Sprite::new(1, 1));
        }
        for pose in 0..POSES {
            for tone in 0..TONES {
                for frame in 0..FRAMES {
                    for child in 0..2 {
                        for mirror in 0..2 {
                            let i = index(pose, tone, frame, child, mirror);
                            sprites[i] = sprite(
                                pose as u8,
                                tone as u8,
                                frame,
                                child == 1,
                                mirror == 1,
                            );
                        }
                    }
                }
            }
        }
        let mut props = Vec::new();
        for prop in 0..PROPS as u8 {
            for _tone in 0..TONES {
                for frame in 0..FRAMES {
                    for mirror in 0..2 {
                        props.push(prop_sprite(prop, _tone as u8, frame, mirror == 1));
                    }
                }
            }
        }
        let sites: Vec<Sprite> = (0..3).map(site_sprite).collect();
        let base = sprites.clone();
        let base_props = props.clone();
        let base_sites = sites.clone();
        Self {
            sprites,
            props,
            sites,
            base,
            base_props,
            base_sites,
            tint: [1.0; 3],
            winter: false,
        }
    }

    /// Bake a new ambient light in. Runs only when the palette changes, and the
    /// whole set is ~0.3 MB of pixels.
    /// B76: winter coats. The palette's casual shirts become the muted coats
    /// and jackets (the elders' grey and fawn, a dark mac, an orange work coat),
    /// so a winter crowd reads as wrapped up; hi-vis, brides, jerseys and the
    /// band keep their own clothes.
    fn tone_of(&self, tone: u8) -> u8 {
        if !self.winter {
            return tone;
        }
        match tone % TONES as u8 {
            0 => 6,
            1 => 15,
            2 => 16,
            3 => 7,
            4 => 17,
            5 => 6,
            _ => tone,
        }
    }

    /// Turn the winter wardrobe on or off (a season change, not a frame change).
    pub fn set_winter(&mut self, winter: bool) {
        self.winter = winter;
    }

    pub fn retint(&mut self, tint: [f32; 3]) {
        if (0..3).all(|i| (tint[i] - self.tint[i]).abs() < 0.004) {
            return;
        }
        self.tint = tint;
        let lut = crate::light::lut(tint);
        for (dst, src) in self.sprites.iter_mut().zip(self.base.iter()) {
            if dst.px.len() != src.px.len() {
                *dst = src.clone();
            } else {
                dst.px.copy_from_slice(&src.px);
            }
            crate::light::tint_pixels(&mut dst.px, &lut);
        }
        for (dst, src) in self.sites.iter_mut().zip(self.base_sites.iter()) {
            dst.px.copy_from_slice(&src.px);
            crate::light::tint_pixels(&mut dst.px, &lut);
        }
        for (dst, src) in self.props.iter_mut().zip(self.base_props.iter()) {
            match src {
                Some(s) => match dst {
                    Some(d) if d.px.len() == s.px.len() => {
                        d.px.copy_from_slice(&s.px);
                        crate::light::tint_pixels(&mut d.px, &lut);
                    }
                    _ => *dst = Some(s.clone()),
                },
                None => *dst = None,
            }
        }
    }

    /// Draw every figure. `t` is the scene's animation clock in seconds (the
    /// walk and hammer cycles), so a caller can pin a frame in a test.
    /// `px` is the screen position of every figure, computed once per frame by
    /// the scene (B61c: a damage-limited composite calls this per damaged
    /// rectangle, and projecting nine hundred figures per rectangle was most of
    /// a rush-hour frame).
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &self,
        fb: &mut Framebuffer,
        _cam: &Camera,
        map: &TileMap,
        figures: &[Figure],
        px_at: &[(i32, i32)],
        bands: &crate::bands::Bands,
        depth: &[u16],
        mode: DepthMode,
        t: f32,
        city_key: &[u16],
        exempt_from: usize,
    ) {
        // B62: the figures from `exempt_from` on (a festival's crowd, a band, a
        // gameday lot) are the spectacle: always drawn as people, never as dots.
        // LOD: on a wide union the outer thirds are across the room; there a
        // 2 px dot is all the eye can resolve, and it is a tenth of the cost.
        // LOD only ever changes detail (D71): a figure is never dropped, so a
        // crowd past the readability limit is drawn as dots instead.
        //
        // B75/D83: and that is what emptied the streets. A grown city passes
        // 1 400 figures by mid-morning, so *everyone* was drawn as a two-pixel
        // dot of coat colour — 2 500 people in the frame and not one readable
        // person. A figure is 9x15 px at every distance on a 1x frame, so the
        // dot is kept for a crowd no eye can take apart (and for the far edge of
        // a spanned union), not for an ordinary busy street.
        let wide = fb.w >= 4000;
        let mid = fb.w as i32 / 2;
        let radius = (fb.w as i32 * 9) / 10;
        let crowded = figures.len().min(exempt_from) > 5200;
        // D71: a partial composite hands the framebuffer a damage rectangle.
        // Skip the figures outside it before doing any per-figure work.
        let clip = fb.clip();
        // B61e: one figure, by list index. Called through the band index below,
        // so a damaged rectangle visits only the rows it covers.
        let mut one = |i: usize| {
            let f = &figures[i];
            let (px, py) = match px_at.get(i) {
                Some(p) => *p,
                None => return,
            };
            if px + SPR_W as i32 + 4 < clip.x0
                || px - SPR_W as i32 - 4 >= clip.x1
                || py + SPR_H as i32 + 4 < clip.y0
                || py - SPR_H as i32 - 4 >= clip.y1
            {
                return;
            }
            let mut key = actor_key(f.x.floor() as i32, f.y.floor() as i32);
            if f.open {
                // A plot paints its whole sprite with its own (front-corner)
                // key; a figure on the lawn must beat it (B43).
                let i = (f.y.floor() as i32 * map.w + f.x.floor() as i32).max(0) as usize;
                if let Some(&ck) = city_key.get(i) {
                    key = key.max(ck);
                }
            }
            let far = wide && (px - mid).abs() > radius;
            if (far || crowded) && i < exempt_from && f.pose != POSE_OBJECT {
                // A dot: the coat colour at the feet, two pixels tall so a
                // distant crowd still reads as people rather than ants.
                let c = coat(self.tone_of(f.tone));
                if px >= 0
                    && py >= 0
                    && (px as usize) < fb.w
                    && (py as usize) < fb.h
                    && (mode == DepthMode::Off || depth[py as usize * fb.w + px as usize] <= key)
                {
                    fb.put(px, py, c);
                    fb.put(px, py - 1, c);
                    if !f.child {
                        fb.put(px, py - 2, c);
                    }
                }
                return;
            }
            let pose = (f.pose as usize).min(POSES - 1);
            let tone = self.tone_of(f.tone) as usize % TONES;
            let frame = if f.pose == POSE_OBJECT {
                0
            } else {
                walk_frame(f, t) as usize
            };
            let mirror = (f.dir == 2 || f.dir == 3) as usize;
            let child = f.child as usize;
            let s = &self.sprites[index(pose, tone, frame, child, mirror)];
            let (ox, oy) = (px - s.ax, py - s.ay);
            if f.pose != POSE_OBJECT {
                match mode {
                    DepthMode::PerPixel => fb.blit_sprite_depth(s, ox, oy, key, depth),
                    DepthMode::Anchor => {
                        fb.blit_sprite_anchor(s, ox, oy, key, depth);
                    }
                    DepthMode::Off => fb.blit_sprite(s, ox, oy),
                }
            }
            // A carried prop or a pet is drawn through the same depth test, so
            // a wheelbarrow behind a wall does not show on the wall.
            if f.prop != PROP_NONE && f.prop != PROP_LUMBER {
                let pi = prop_index(f.prop, tone, frame, mirror);
                if let Some(Some(p)) = self.props.get(pi) {
                    match mode {
                        DepthMode::PerPixel => fb.blit_sprite_depth(p, px - p.ax, py - p.ay, key, depth),
                        DepthMode::Anchor => {
                            fb.blit_sprite_anchor(p, px - p.ax, py - p.ay, key, depth);
                        }
                        DepthMode::Off => fb.blit_sprite(p, px - p.ax, py - p.ay),
                    }
                }
            }
        };
        if bands.is_empty() {
            for i in 0..figures.len() {
                one(i);
            }
        } else {
            // A figure is SPR_H tall above its feet: the pad is its own height.
            let pad = SPR_H as i32 + 4;
            bands.for_each(clip, pad, |i| one(i as usize));
        }
    }

    /// The materials at each site: drawn before the crews, through the same
    /// depth plane so a pallet behind a wall stays behind it.
    pub fn draw_sites(
        &self,
        fb: &mut Framebuffer,
        cam: &Camera,
        map: &TileMap,
        sites: &[(f32, f32, u8)],
        depth: &[u16],
        mode: DepthMode,
    ) {
        for &(x, y, kind) in sites {
            let z = map.ground_px(x, y);
            let (sx, sy) = cam.project(x, y, z);
            let sp = &self.sites[(kind as usize) % self.sites.len()];
            let key = actor_key(x.floor() as i32, y.floor() as i32);
            match mode {
                DepthMode::PerPixel => {
                    fb.blit_sprite_depth(sp, sx as i32 - sp.ax, sy as i32 - sp.ay, key, depth)
                }
                DepthMode::Anchor => {
                    fb.blit_sprite_anchor(sp, sx as i32 - sp.ax, sy as i32 - sp.ay, key, depth);
                }
                DepthMode::Off => fb.blit_sprite(sp, sx as i32 - sp.ax, sy as i32 - sp.ay),
            }
        }
    }

    /// Sprite count, for the handback.
    pub fn sprite_count(&self) -> usize {
        self.sprites.len() + self.props.iter().filter(|p| p.is_some()).count()
    }

    /// Lay the whole sprite set out in a grid (a review sheet). Returns the size
    /// and the pixels, so the caller can encode a PNG without the renderer
    /// growing an image dependency.
    pub fn sheet(&self, cols: usize) -> (usize, usize, Vec<Color>) {
        let cell_w = SPR_W + 3;
        let cell_h = SPR_H + 3;
        let rows = SLOTS.div_ceil(cols);
        let (w, h) = (cols * cell_w, rows * cell_h);
        let mut px = vec![palette::rgb(0x3a, 0x50, 0x30); w * h];
        for i in 0..SLOTS {
            let s = &self.sprites[i];
            let (cx, cy) = ((i % cols) * cell_w + 1, (i / cols) * cell_h + 1);
            for y in 0..s.h {
                for x in 0..s.w {
                    let c = s.px[y * s.w + x];
                    if c != crate::palette::TRANSPARENT {
                        px[(cy + y) * w + cx + x] = c;
                    }
                }
            }
        }
        (w, h, px)
    }
}

impl Default for PeopleRenderer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod prop_tests {
    use super::*;

    /// B67: the leg cycle steps evenly on day 1 and on day 90 of a world.
    #[test]
    fn the_walk_cycle_keeps_its_cadence_late_in_a_world() {
        let f = Figure { x: 3.2, y: 4.7, pose: 0, tone: 0, child: false, prop: 0, dir: 0, open: false, lift: 0, frame: 0 };
        let steps = |t0: f64| -> Vec<i32> {
            let mut prev = walk_frame(&f, clock_f32(t0)) as i32;
            let n = frames_of(f.pose) as i32;
            (1..=120)
                .map(|k| {
                    let now = walk_frame(&f, clock_f32(t0 + k as f64 / 30.0)) as i32;
                    let d = (now - prev).rem_euclid(n);
                    prev = now;
                    d
                })
                .collect()
        };
        let early = steps(3600.0 * 8.0);
        let late = steps(90.0 * 86_400.0 + 3600.0 * 8.0);
        // Same number of frame advances in four seconds (10.4 of them), and no
        // step of more than one frame, early or late.
        let count = |v: &Vec<i32>| v.iter().filter(|d| **d != 0).count();
        assert!(count(&early).abs_diff(count(&late)) <= 1, "{early:?} vs {late:?}");
        assert!(late.iter().all(|d| *d <= 1), "a late-world leg cycle lurched: {late:?}");
        // The plain `as f32` was the bug: half-second steps.
        let raw = (90.0 * 86_400.0 + 3600.0 * 8.0 + 1.0 / 30.0) as f32 - (90.0 * 86_400.0 + 3600.0 * 8.0) as f32;
        assert!(raw == 0.0 || raw >= 0.49, "f32 at day 90 steps by {raw}");
    }

    fn has(s: &Sprite, c: Color) -> bool {
        s.px.contains(&c)
    }

    /// Every prop code draws its own sprite (B48 fixed an off-by-one that
    /// drew the dog as a wheelbarrow), and the new ones are the size they
    /// should be.
    #[test]
    fn each_prop_draws_its_own_sprite() {
        let r = PeopleRenderer::new();
        let at = |prop: u8, frame: usize| r.props[prop_index(prop, 0, frame, 0)].as_ref();
        assert!(at(PROP_NONE, 0).is_none() && at(PROP_LUMBER, 0).is_none());
        let dog = at(PROP_DOG, 0).expect("a dog sprite");
        assert!(has(dog, palette::rgb(0x6a, 0x4e, 0x34)), "the dog is not fur-coloured");
        let barrow = at(PROP_BARROW, 0).expect("a barrow sprite");
        assert!(has(barrow, palette::rgb(0x8a, 0x62, 0x36)));
        assert!(!has(dog, palette::rgb(0x8a, 0x62, 0x36)));
        let ball = at(PROP_BALL, 0).expect("a ball sprite");
        assert!(has(ball, palette::rgb(0xd8, 0x3a, 0x2e)));
        let pram = at(PROP_STROLLER, 0).expect("a pram sprite");
        assert!(has(pram, palette::rgb(0x7a, 0x9a, 0xc4)));
        let van = at(PROP_TRUCK, 0).expect("a van sprite");
        assert!(van.w >= 40 && van.h >= 30, "the van is {}x{}", van.w, van.h);
        let mirrored = r.props[prop_index(PROP_TRUCK, 0, 0, 1)].as_ref().unwrap();
        assert_eq!((van.w, van.h), (mirrored.w, mirrored.h));
        assert_ne!(van.px, mirrored.px, "the mirrored van must run the other way");
        let hearse = at(PROP_HEARSE, 0).expect("a hearse sprite");
        assert!(hearse.w >= 30 && has(hearse, palette::rgb(0xf2, 0x8a, 0xc0)), "no flowers on the hearse");
        let arch = at(PROP_ARCH, 0).expect("an arch sprite");
        assert!(has(arch, palette::rgb(0xf2, 0x8a, 0xc0)), "the arch has no blooms");
        // Confetti falls between its two frames.
        assert_ne!(at(PROP_CONFETTI, 0).unwrap().px, at(PROP_CONFETTI, 1).unwrap().px);
    }

    #[test]
    fn the_bride_is_white_and_the_mourner_is_dark() {
        let r = PeopleRenderer::new();
        let white = palette::rgb(0xf4, 0xf0, 0xe8);
        let s = &r.sprites[index(POSE_STAND as usize, 10, 0, 0, 0)];
        assert!(has(s, white), "no white on the bride");
        let d = &r.sprites[index(POSE_STAND as usize, 11, 0, 0, 0)];
        assert!(has(d, palette::rgb(0x2a, 0x2a, 0x32)));
        assert!(!has(d, white));
    }
}
