//! Hand-authored pixel sprites, B22b. 2× detail for the 64-px engine (D61):
//! bodies with heads and legs, stags with antlers, waterfowl with a wake,
//! 3-frame wing beats — readable silhouettes at 1×, in the project palette.
//! Built once in `Wildlife::new` (nothing allocates per frame).
//!
//! Rows use single-letter legends; `.` is transparent. Facing is right unless
//! noted. The animal stands on the sprite's bottom row, and `draw` blits the
//! sprite centred on that point — keep the lowest opaque row as the ground
//! for land species and the water line for paddlers.
//!
//! When this crate is wired into `render`, the colour constants below
//! can join `render::palette` for night/season swaps (planner's call).

use render::framebuffer::Sprite;
use render::palette::{Color, rgb, TRANSPARENT};

// --- fur / feather / fish -----------------------------------------------
pub const DEER: Color = rgb(0xa2, 0x6c, 0x4a);
pub const DEER_HEAD: Color = rgb(0x8a, 0x5a, 0x3c);
pub const DEER_DARK: Color = rgb(0x7c, 0x51, 0x36);
pub const DEER_LEG: Color = rgb(0x64, 0x44, 0x30);
pub const DEER_ANTLER: Color = rgb(0xc9, 0xa2, 0x6b);
pub const RABBIT: Color = rgb(0xb8, 0xaa, 0x96);
pub const RABBIT_DARK: Color = rgb(0x8a, 0x7c, 0x6c);
pub const COW: Color = rgb(0xe8, 0xe4, 0xdc);
pub const COW_BROWN: Color = rgb(0x8a, 0x5a, 0x3a);
pub const COW_DARK: Color = rgb(0x4a, 0x3a, 0x30);
pub const COW_HORN: Color = rgb(0xd8, 0xd0, 0xc0);
pub const WOOL: Color = rgb(0xed, 0xe8, 0xdf);
pub const SHEEP_DARK: Color = rgb(0x7a, 0x6c, 0x5c);
pub const HORSE: Color = rgb(0x6a, 0x4a, 0x36);
pub const HORSE_MANE: Color = rgb(0x33, 0x24, 0x1a);
pub const HORSE_DARK: Color = rgb(0x4e, 0x36, 0x26);
pub const DUCK: Color = rgb(0x8a, 0x7a, 0x58);
pub const DUCK_BELLY: Color = rgb(0xcf, 0xc8, 0xb4);
pub const DUCK_HEAD: Color = rgb(0x3e, 0x6e, 0x3a);
pub const BEAK: Color = rgb(0xd0, 0x9a, 0x3e);
pub const SWAN: Color = rgb(0xf4, 0xf1, 0xea);
pub const BIRD: Color = rgb(0x44, 0x4a, 0x56);
pub const BIRD_WING: Color = rgb(0x9c, 0xa4, 0xac);
pub const GULL: Color = rgb(0xef, 0xef, 0xec);
pub const GULL_WING: Color = rgb(0x9c, 0xa4, 0xac);
pub const OWL: Color = rgb(0x8a, 0x6a, 0x4a);
pub const OWL_BELLY: Color = rgb(0xd0, 0xc0, 0xa0);
pub const OWL_EYE: Color = rgb(0xff, 0xd0, 0x70);
pub const BRANCH: Color = rgb(0x54, 0x3d, 0x2a);
pub const BAT: Color = rgb(0x3a, 0x3d, 0x46);
pub const BAT_WING: Color = rgb(0x55, 0x5a, 0x66);
pub const FISH: Color = rgb(0xa8, 0xb8, 0xc8);
pub const SPLASH: Color = rgb(0xe8, 0xf7, 0xff);
pub const WAKE: Color = rgb(0xc4, 0xe2, 0xf2);

/// A set of all sprite tables, one entry per species group in `Species`
/// order, frames in animation order.
pub struct Sheets {
    /// Stag standing, stag grazing, doe standing, doe grazing.
    pub deer: [Sprite; 4],
    /// Idle, hop, idle.
    pub rabbit: [Sprite; 3],
    pub cow: [Sprite; 2],
    pub sheep: [Sprite; 2],
    pub horse: [Sprite; 2],
    pub duck: [Sprite; 2],
    pub swan: [Sprite; 2],
    /// Wings up, wings mid, wings down.
    pub bird: [Sprite; 3],
    /// Perched, gliding, wings up.
    pub gull: [Sprite; 3],
    /// Eyes open, blink.
    pub owl: [Sprite; 2],
    /// Wings up, wings mid, wings down.
    pub bat: [Sprite; 3],
    pub fish: [Sprite; 3],
    /// B80: hen pecking, hen walking.
    pub chicken: [Sprite; 2],
    /// B80: a tractor with a plough, facing right then left, two wheel frames each.
    pub tractor: [Sprite; 4],
    /// B80: a combine harvester, facing right then left.
    pub combine: [Sprite; 4],
}

fn mk(rows: &[&str], color: impl Fn(char) -> Option<Color>) -> Sprite {
    let h = rows.len();
    // Right-side transparent padding is trimmed: rows may end however many
    // dots the author counted, the width is the rightmost painted column.
    let w = rows
        .iter()
        .map(|r| r.trim_end_matches('.').chars().count())
        .max()
        .unwrap_or(0);
    let mut s = Sprite::new(w, h);
    for (y, row) in rows.iter().enumerate() {
        for (x, c) in row.chars().enumerate() {
            if c == '.' {
                continue;
            }
            if let Some(col) = color(c) {
                s.set(x, y, col);
            }
        }
    }
    s
}

impl Sheets {
    /// B46e: the sheets for a land. A savanna's deer are tan antelope with dark
    /// horns, its horses are zebras, its cattle dark wildebeest: the same
    /// bodies, recoloured, so the herds on the plain are the plain's own.
    pub fn build_for(biome: map::Biome) -> Self {
        let mut s = Self::build();
        if matches!(biome, map::Biome::Savanna | map::Biome::Steppe) {
            let recolour = |spr: &mut Sprite, f: &dyn Fn(usize, usize, Color) -> Color| {
                for y in 0..spr.h {
                    for x in 0..spr.w {
                        let c = spr.px[y * spr.w + x];
                        if c >> 24 != 0 {
                            spr.px[y * spr.w + x] = f(x, y, c);
                        }
                    }
                }
            };
            for d in s.deer.iter_mut() {
                recolour(d, &|_, _, c| {
                    if c == DEER { rgb(0xc8, 0x92, 0x58) }
                    else if c == DEER_HEAD { rgb(0xb0, 0x7a, 0x48) }
                    else if c == DEER_DARK { rgb(0x9a, 0x66, 0x3c) }
                    else if c == DEER_LEG { rgb(0x72, 0x4c, 0x30) }
                    else if c == DEER_ANTLER { rgb(0x3a, 0x2c, 0x24) }
                    else { c }
                });
            }
            for h in s.horse.iter_mut() {
                recolour(h, &|x, y, c| {
                    if c == HORSE_MANE { rgb(0x20, 0x20, 0x24) }
                    else if (x + 2 * y) % 3 == 0 { rgb(0x26, 0x26, 0x2a) }
                    else { rgb(0xec, 0xe8, 0xe0) }
                });
            }
            for c in s.cow.iter_mut() {
                recolour(c, &|_, _, col| {
                    if col == COW { rgb(0x6a, 0x66, 0x62) }
                    else if col == COW_BROWN { rgb(0x52, 0x4c, 0x48) }
                    else if col == COW_DARK { rgb(0x34, 0x30, 0x2e) }
                    else { col }
                });
            }
        }
        // B80: the islands keep goats, not sheep: the same bodies in tan and brown.
        if matches!(biome, map::Biome::Islands | map::Biome::Lagoon) {
            for sh in s.sheep.iter_mut() {
                for px in sh.px.iter_mut() {
                    if *px >> 24 != 0 {
                        *px = if *px == WOOL { rgb(0xcc, 0xae, 0x80) } else if *px == SHEEP_DARK { rgb(0x6a, 0x4a, 0x30) } else { *px };
                    }
                }
            }
        }
        if biome == map::Biome::Highlands {
            for c in s.cow.iter_mut() {
                for px in c.px.iter_mut() {
                    if *px >> 24 != 0 {
                        *px = if *px == COW { rgb(0xb8, 0x62, 0x34) }
                            else if *px == COW_BROWN { rgb(0x8a, 0x44, 0x24) }
                            else { *px };
                    }
                }
            }
        }
        s
    }

    /// Build every table. One-off: allowed to allocate (crate rule is
    /// "no allocation per **frame**").
    pub fn build() -> Self {
        // --- Deer: a ~10-px body on 3-px legs; stags carry a rack, does
        // don't. The rack is a 5×3 mass; grazing sweeps it back over the
        // neck. ----------------------------------------------------------------
        let deer = [
            // Stag, standing (15×12).
            mk(
                &[
                    ".........A.A...",
                    "........AAAAA..",
                    ".........A.A...",
                    "..........hh...",
                    ".........nhhhh.",
                    ".BB......nnn...",
                    ".BBBBBBBBBB....",
                    "tBBBBBBBBBB....",
                    "..DDDDDDDD.....",
                    "..dd...dd......",
                    "..dd...dd......",
                    "..dd...dd......",
                ],
                deer_col,
            ),
            // Stag, grazing (15×10): muzzle at belly height, rack swept back.
            mk(
                &[
                    ".......A.AA....",
                    "........A.A....",
                    ".BB.......AA...",
                    ".BB......nnA...",
                    ".BBBBBBBBBBnh..",
                    "tBBBBBBBBBBnhh.",
                    "..DDDDDDDD.....",
                    "..dd...dd......",
                    "..dd...dd......",
                    "..dd...dd......",
                ],
                deer_col,
            ),
            // Doe, standing (13×9).
            mk(
                &[
                    ".......hha....",
                    "......nhhhh...",
                    ".BB...........",
                    ".BBBBBBBBB....",
                    "tBBBBBBBBB....",
                    "..DDDDDDDD....",
                    "..dd..dd.....",
                    "..dd..dd.....",
                    "..dd..dd.....",
                ],
                deer_col,
            ),
            // Doe, grazing (13×7): muzzle at the grass.
            mk(
                &[
                    ".BB...........",
                    ".BBBBBBBBh....",
                    "tBBBBBBBBhh...",
                    "..DDDDDDD.....",
                    "..dd..dd.....",
                    "..dd..dd.....",
                    "..dd..dd.....",
                ],
                deer_col,
            ),
        ];

        // --- Rabbit (6×5): two ears, a haunch, a cotton tail. --------------
        let rabbit = [
            // Idle, crouched.
            mk(
                &[
                    "..rr..",
                    "..rr..",
                    ".rrrr.",
                    "rrrrrd",
                    ".d..d.",
                ],
                rabbit_col,
            ),
            // Hop: body tucked, ears laid back.
            mk(
                &[
                    "......",
                    "..rr..",
                    ".rrrrd",
                    ".rrrrd",
                    "..d.d.",
                ],
                rabbit_col,
            ),
            mk(
                &[
                    "..rr..",
                    "..rr..",
                    ".rrrr.",
                    "rrrrrd",
                    ".d..d.",
                ],
                rabbit_col,
            ),
        ];

        // --- Cow (13×9): a box body, four legs, horns, a dark face and one
        // brown patch. ---------------------------------------------------------
        let cow = [
            // Head up.
            mk(
                &[
                    "..........H.H",
                    "..........DwD",
                    ".........wDDw",
                    ".DwwCCCwwww...",
                    ".DwwCCCwwww...",
                    ".Dwwwwwwww....",
                    "...DD..DD.....",
                    "...DD..DD.....",
                    "...DD..DD.....",
                ],
                cow_col,
            ),
            // Head down: hump up, muzzle at the belly.
            mk(
                &[
                    "..wCCCwwww....",
                    "..CCCwwww.....",
                    "..wwwwwwwwwD..",
                    "..wwwwwwwwwDD",
                    "..wwwwwwww..D",
                    "...DD..DD.....",
                    "...DD..DD.....",
                    "...DD..DD.....",
                ],
                cow_col,
            ),
        ];

        // --- Sheep (11×8): a wool bump on four dark legs, dark face. --------
        let sheep = [
            // Head up.
            mk(
                &[
                    "..wwwwwww.....",
                    ".wwwwwwwwD....",
                    ".wwwwwwwwDD...",
                    ".wwwwwwwwww...",
                    ".wwwwwwwwww...",
                    "..DD...DD.....",
                    "..DD...DD.....",
                    "..DD...DD.....",
                ],
                sheep_col,
            ),
            // Grazing: the face slides down to the legs.
            mk(
                &[
                    "..wwwwwww.....",
                    ".wwwwwwwwww...",
                    ".wwwwwwwwDD...",
                    ".wwwwwwwwDD...",
                    ".wwwwwwwwww...",
                    "..DD...DD.....",
                    "..DD...DD.....",
                    "..DD...DD.....",
                ],
                sheep_col,
            ),
        ];

        // --- Horse (13×10): long legs, a mane down the neck, a tail. -------
        let horse = [
            // Head up.
            mk(
                &[
                    "........m.mh.",
                    ".......mhhhh.",
                    "......mhh....",
                    ".BBBBBBBm....",
                    "mBBBBBBBBB...",
                    "m.BBBBBB.....",
                    "m..dd..dd....",
                    "...dd..dd....",
                    "...dd..dd....",
                    "...dd..dd....",
                ],
                horse_col,
            ),
            // Head down: mane over the lowered neck, muzzle at the grass.
            mk(
                &[
                    ".BBB..........",
                    ".BBBBBBBBB....",
                    ".BBBBBBBBBm...",
                    ".BBBBBBBBBmh..",
                    "..BBBBBBBBhh..",
                    "..BBBBBBBB.h.",
                    "..BBBBBBB.....",
                    "...dd..dd.....",
                    "...dd..dd.....",
                    "...dd..dd.....",
                ],
                horse_col,
            ),
        ];

        // --- Duck (mallard, 10×5): green head, orange bill, wake behind. --
        let duck = [
            // Paddling.
            mk(
                &[
                    ".....hh.....",
                    ".....hhB....",
                    "...wwwh.....",
                    ".wwwwwwww...",
                    ".WWwwwwww...",
                ],
                duck_col,
            ),
            // Dabbling: head to the water line, wake on both sides.
            mk(
                &[
                    "..wwwwww....",
                    ".wwwwwwww...",
                    ".wwwwwwwh.",
                    ".wwwwwwwBB",
                    ".WwwwwwwwW",
                ],
                duck_col,
            ),
        ];

        // --- Swan (11×7): an S-neck, a bill, a wake. -----------------------
        let swan = [
            // Cruising.
            mk(
                &[
                    "........ss....",
                    ".......ssB....",
                    ".......ss.....",
                    "......sss.....",
                    "......ss......",
                    "......sssss.",
                    ".WWsssssss.",
                ],
                swan_col,
            ),
            // Feeding: the neck curves to the water.
            mk(
                &[
                    "........s.....",
                    ".......ssB....",
                    ".......sss....",
                    "......ssss....",
                    "......sssss.",
                    ".WWsssssss.",
                ],
                swan_col,
            ),
        ];

        // --- Bird (7×4): a crossing silhouette, a three-beat wing. ---------
        let bird = [
            // Wings up.
            mk(
                &[
                    "....w...",
                    "...www..",
                    "..dddddd.",
                    "....dd..",
                ],
                bird_col,
            ),
            // Mid: wings folded to the body.
            mk(
                &[
                    ".......",
                    ".......",
                    "..dddddd.",
                    "....dd..",
                ],
                bird_col,
            ),
            // Wings down.
            mk(
                &[
                    "..dddddd.",
                    "...www..",
                    "....w...",
                    ".......",
                ],
                bird_col,
            ),
        ];

        // --- Gull: a perched frame, gliding and a wings-up beat. -----------
        let gull = [
            // Perched: upright, two-px bill forward.
            mk(
                &[
                    ".ggBB",
                    ".ggg.",
                    ".ggg.",
                    "ggggg",
                    ".gg..",
                    ".g.g.",
                ],
                gull_col,
            ),
            // Gliding: wings out level.
            mk(&["wwggBwww.", ".gggg...."], gull_col),
            // Wings up.
            mk(
                &[
                    "w.......w",
                    ".w.....w.",
                    "..wggBw..",
                    "...gg....",
                ],
                gull_col,
            ),
        ];

        // --- Owl (8×9): on a branch, big amber eyes, a blink. --------------
        let owl = [
            mk(
                &[
                    ".o....o.",
                    "ooooooo.",
                    "oeeooeo.",
                    "ooobooo.",
                    "owwoooo.",
                    "owwwooo.",
                    "owwoooo.",
                    "..d..d..",
                    "TTTTTTTT",
                ],
                owl_col,
            ),
            // Blink.
            mk(
                &[
                    ".o....o.",
                    "ooooooo.",
                    "ooooooo.",
                    "ooobooo.",
                    "owwoooo.",
                    "owwwooo.",
                    "owwoooo.",
                    "..d..d..",
                    "TTTTTTTT",
                ],
                owl_col,
            ),
        ];

        // --- Bat (9×4): a three-beat flap. ---------------------------------
        let bat = [
            // Wings up.
            mk(
                &[
                    "w.......w",
                    ".w.b.b.w.",
                    "..wbbbw..",
                    "...bbb...",
                ],
                bat_col,
            ),
            // Mid: wings level.
            mk(
                &[
                    "...b.b...",
                    "w..bbb..w",
                    ".w.bbb.w.",
                    "...bbb...",
                ],
                bat_col,
            ),
            // Wings down.
            mk(
                &[
                    "...b.b...",
                    "...bbb...",
                    "..w...w..",
                    "w.......w",
                ],
                bat_col,
            ),
        ];

        // --- Fish (9×5): a three-frame jump and splash. ---------------------
        let fish = [
            // Droplets arcing out of the water line.
            mk(
                &[
                    "....s....",
                    "...s.s...",
                    "..s...s..",
                    ".s.....s.",
                    "sssssssss",
                ],
                |c| (c == 's').then_some(SPLASH),
            ),
            // The fish itself, mid-arc, over the splash.
            mk(
                &[
                    "....s....",
                    "....fffff",
                    ".ff..fff..",
                    "....ss.s.",
                    "sssssssss",
                ],
                |c| match c {
                    'f' => Some(FISH),
                    's' => Some(SPLASH),
                    _ => None,
                },
            ),
            // Splash burst and the ripple ring on the surface.
            mk(
                &[
                    "..s.s.s..",
                    ".s.sss.s.",
                    "sss.ss.s.",
                    "sssssssss",
                    "..sss.s..",
                ],
                |c| (c == 's').then_some(SPLASH),
            ),
        ];

        // --- B80: the farmyard. A hen is 6 px; the machines are the size of a
        // car, with a plough or a header trailing so they read as working. -----
        let chicken = [
            mk(
                &[
                    "....r.",
                    "...rrr",
                    ".wwwwr",
                    "wwwwww",
                    ".wwww.",
                    "..y.y.",
                ],
                chicken_col,
            ),
            mk(
                &[
                    ".....r",
                    "....rrr",
                    ".wwwwb.",
                    "wwwwww.",
                    ".wwww..",
                    "..y.y..",
                ],
                chicken_col,
            ),
        ];
        let tractor = [
            tractor_sprite(false, false),
            tractor_sprite(false, true),
            flip(&tractor_sprite(false, false)),
            flip(&tractor_sprite(false, true)),
        ];
        let combine = [
            combine_sprite(false),
            combine_sprite(true),
            flip(&combine_sprite(false)),
            flip(&combine_sprite(true)),
        ];

        Sheets {
            chicken,
            tractor,
            combine,
            deer,
            rabbit,
            cow,
            sheep,
            horse,
            duck,
            swan,
            bird,
            gull,
            owl,
            bat,
            fish,
        }
    }

    /// Per-frame sprite statistics for the handback report (tools only):
    /// (species id, frame, width, height, opaque px) in `Species::id()`
    /// order.
    pub fn frame_stats(&self) -> Vec<(usize, usize, usize, usize, usize)> {
        let all: &[&[Sprite]] = &[
            &self.bird,
            &self.gull,
            &self.duck,
            &self.swan,
            &self.fish,
            &self.deer,
            &self.rabbit,
            &self.cow,
            &self.sheep,
            &self.horse,
            &self.owl,
            &self.bat,
            &self.chicken,
            &self.tractor,
            &self.combine,
        ];
        let mut out = Vec::new();
        for (i, frames) in all.iter().enumerate() {
            for (f, s) in frames.iter().enumerate() {
                let px = s.px.iter().filter(|&&c| c != TRANSPARENT).count();
                out.push((i, f, s.w, s.h, px));
            }
        }
        out
    }
}

// --- per-species legends -------------------------------------------------------

fn rect(s: &mut Sprite, x: i32, y: i32, w: i32, h: i32, c: Color) {
    for yy in y..y + h {
        for xx in x..x + w {
            if xx >= 0 && yy >= 0 && (xx as usize) < s.w && (yy as usize) < s.h {
                s.set(xx as usize, yy as usize, c);
            }
        }
    }
}

fn disc(s: &mut Sprite, cx: i32, cy: i32, r: i32, c: Color) {
    for yy in -r..=r {
        for xx in -r..=r {
            if xx * xx + yy * yy <= r * r + r / 2 {
                rect(s, cx + xx, cy + yy, 1, 1, c);
            }
        }
    }
}

fn wheel(s: &mut Sprite, cx: i32, cy: i32, r: i32, turn: bool) {
    let tyre = rgb(0x26, 0x26, 0x2a);
    disc(s, cx, cy, r, tyre);
    disc(s, cx, cy, (r / 2).max(1), rgb(0xe4, 0xc0, 0x34));
    // The tread: a lug on the rim that moves with the wheel.
    let (lx, ly) = if turn { (cx + r - 1, cy - 1) } else { (cx - r + 1, cy + 1) };
    rect(s, lx, ly, 1, 1, rgb(0x4a, 0x4a, 0x50));
}

/// A tractor with its plough trailing behind it (facing right, 30x18).
fn tractor_sprite(_unused: bool, turn: bool) -> Sprite {
    let mut s = Sprite::new(30, 18);
    let green = rgb(0x3c, 0x94, 0x3e);
    let dgreen = rgb(0x28, 0x62, 0x2c);
    let lgreen = rgb(0x5c, 0xb4, 0x56);
    // The plough: a beam trailing to the left with three share blades.
    let steel = rgb(0x9a, 0x9e, 0xa4);
    let dsteel = rgb(0x5e, 0x62, 0x68);
    rect(&mut s, 0, 12, 10, 2, dsteel);
    for k in 0..3 {
        rect(&mut s, 1 + k * 3, 14, 2, 3, steel);
        rect(&mut s, 1 + k * 3, 16, 2, 1, dsteel);
    }
    // Exhaust.
    rect(&mut s, 21, 0, 1, 6, rgb(0x70, 0x70, 0x74));
    // The hood, in front (right).
    rect(&mut s, 18, 6, 11, 6, green);
    rect(&mut s, 18, 6, 11, 1, lgreen);
    rect(&mut s, 18, 11, 11, 1, dgreen);
    rect(&mut s, 28, 7, 1, 4, dgreen);
    // The cab: glass on a green frame with a roof.
    rect(&mut s, 11, 1, 9, 2, dgreen);
    rect(&mut s, 11, 3, 8, 6, rgb(0xa8, 0xcc, 0xe0));
    rect(&mut s, 11, 3, 1, 6, dgreen);
    rect(&mut s, 18, 3, 1, 6, dgreen);
    rect(&mut s, 11, 9, 9, 3, green);
    rect(&mut s, 12, 5, 3, 2, rgb(0xd8, 0xec, 0xf6));
    // The wheels: big at the back, small at the front.
    wheel(&mut s, 12, 13, 4, turn);
    wheel(&mut s, 25, 15, 2, !turn);
    s
}

/// A combine harvester: cab, grain tank, auger, and a wide header with a reel
/// at the front (facing right, 56x26).
fn combine_sprite(turn: bool) -> Sprite {
    let mut s = Sprite::new(56, 27);
    let red = rgb(0xc4, 0x34, 0x28);
    let lred = rgb(0xe0, 0x58, 0x44);
    let dred = rgb(0x8a, 0x24, 0x1e);
    let yellow = rgb(0xe8, 0xc4, 0x38);
    let dyellow = rgb(0xb4, 0x8c, 0x20);
    let grey = rgb(0x70, 0x70, 0x74);
    // The unloading auger folded over the grain tank.
    rect(&mut s, 8, 1, 1, 7, grey);
    rect(&mut s, 8, 1, 14, 1, grey);
    rect(&mut s, 21, 1, 1, 3, grey);
    // The grain tank on the back of the body, and the body.
    rect(&mut s, 5, 7, 14, 6, lred);
    rect(&mut s, 5, 7, 14, 1, rgb(0xf0, 0x7a, 0x60));
    rect(&mut s, 5, 12, 40, 8, red);
    rect(&mut s, 5, 12, 40, 1, lred);
    rect(&mut s, 5, 18, 40, 3, dred);
    rect(&mut s, 12, 12, 1, 6, dred);
    // The cab at the front of the body: a tall glazed box with a roof.
    rect(&mut s, 31, 2, 14, 2, dred);
    rect(&mut s, 31, 4, 14, 9, rgb(0xa8, 0xcc, 0xe0));
    rect(&mut s, 31, 4, 1, 9, dred);
    rect(&mut s, 44, 4, 1, 9, dred);
    rect(&mut s, 33, 6, 5, 3, rgb(0xd8, 0xec, 0xf6));
    rect(&mut s, 38, 4, 1, 9, dred);
    // The header: a wide cutter bar with a slatted reel over it, jutting ahead.
    rect(&mut s, 45, 16, 11, 6, yellow);
    rect(&mut s, 45, 21, 11, 2, dyellow);
    rect(&mut s, 46, 9, 10, 7, dyellow);
    for k in 0..5 {
        rect(&mut s, 47 + k * 2, 9, 1, 7, yellow);
    }
    rect(&mut s, 46, 9, 10, 1, yellow);
    // The feeder housing joining the header to the body.
    rect(&mut s, 43, 14, 4, 6, dred);
    // Wheels: the big drive wheel under the front of the body, a small steering one behind.
    wheel(&mut s, 36, 21, 5, turn);
    wheel(&mut s, 10, 23, 3, !turn);
    s
}

/// Mirror a sprite left-to-right (the machines drive both ways).
fn flip(s: &Sprite) -> Sprite {
    let mut o = Sprite::new(s.w, s.h);
    for y in 0..s.h {
        for x in 0..s.w {
            let c = s.px[y * s.w + x];
            if c != TRANSPARENT {
                o.set(s.w - 1 - x, y, c);
            }
        }
    }
    o
}

fn chicken_col(c: char) -> Option<Color> {
    match c {
        'w' => Some(rgb(0xf0, 0xea, 0xdc)),
        'r' => Some(rgb(0xc4, 0x3a, 0x30)),
        'b' => Some(rgb(0xd8, 0xa0, 0x38)),
        'y' => Some(rgb(0xd8, 0xa0, 0x38)),
        _ => None,
    }
}

fn tractor_col(c: char) -> Option<Color> {
    match c {
        'g' => Some(rgb(0x3c, 0x94, 0x3e)),
        'G' => Some(rgb(0x28, 0x62, 0x2c)),
        'w' => Some(rgb(0xa8, 0xcc, 0xe0)),
        'k' => Some(rgb(0x26, 0x26, 0x2a)),
        'y' => Some(rgb(0xe4, 0xc0, 0x34)),
        'x' => Some(rgb(0x70, 0x70, 0x74)),
        'p' => Some(rgb(0x9a, 0x9e, 0xa4)),
        'P' => Some(rgb(0x5e, 0x62, 0x68)),
        _ => None,
    }
}

fn combine_col(c: char) -> Option<Color> {
    match c {
        'R' => Some(rgb(0xc4, 0x34, 0x28)),
        'D' => Some(rgb(0x8a, 0x24, 0x1e)),
        'w' => Some(rgb(0xa8, 0xcc, 0xe0)),
        'k' => Some(rgb(0x26, 0x26, 0x2a)),
        'y' => Some(rgb(0xe4, 0xc0, 0x34)),
        'Y' => Some(rgb(0xe8, 0xc4, 0x38)),
        'x' => Some(rgb(0x70, 0x70, 0x74)),
        _ => None,
    }
}

fn deer_col(c: char) -> Option<Color> {
    match c {
        'A' => Some(DEER_ANTLER),
        'h' => Some(DEER_HEAD),
        'a' => Some(DEER_DARK),
        'n' => Some(DEER),
        'B' => Some(DEER),
        't' => Some(DEER_DARK),
        'D' => Some(DEER_DARK),
        'd' => Some(DEER_LEG),
        _ => None,
    }
}

fn rabbit_col(c: char) -> Option<Color> {
    match c {
        'r' => Some(RABBIT),
        'd' => Some(RABBIT_DARK),
        _ => None,
    }
}

fn cow_col(c: char) -> Option<Color> {
    match c {
        'H' => Some(COW_HORN),
        'D' => Some(COW_DARK),
        'w' => Some(COW),
        'C' => Some(COW_BROWN),
        _ => None,
    }
}

fn sheep_col(c: char) -> Option<Color> {
    match c {
        'w' => Some(WOOL),
        'D' => Some(SHEEP_DARK),
        _ => None,
    }
}

fn horse_col(c: char) -> Option<Color> {
    match c {
        'h' => Some(HORSE),
        'm' => Some(HORSE_MANE),
        'B' => Some(HORSE),
        'd' => Some(HORSE_DARK),
        _ => None,
    }
}

fn duck_col(c: char) -> Option<Color> {
    match c {
        'h' => Some(DUCK_HEAD),
        'B' => Some(BEAK),
        'w' => Some(DUCK),
        'W' => Some(WAKE),
        _ => None,
    }
}

fn swan_col(c: char) -> Option<Color> {
    match c {
        's' => Some(SWAN),
        'B' => Some(BEAK),
        'W' => Some(WAKE),
        _ => None,
    }
}

fn bird_col(c: char) -> Option<Color> {
    match c {
        'w' => Some(BIRD_WING),
        'd' => Some(BIRD),
        _ => None,
    }
}

fn gull_col(c: char) -> Option<Color> {
    match c {
        'w' => Some(GULL_WING),
        'g' => Some(GULL),
        'B' => Some(BEAK),
        _ => None,
    }
}

fn owl_col(c: char) -> Option<Color> {
    match c {
        'o' => Some(OWL),
        'e' => Some(OWL_EYE),
        'b' => Some(BEAK),
        'w' => Some(OWL_BELLY),
        'd' => Some(OWL_BELLY),
        'T' => Some(BRANCH),
        _ => None,
    }
}

fn bat_col(c: char) -> Option<Color> {
    match c {
        'w' => Some(BAT_WING),
        'b' => Some(BAT),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_frame_is_populated_and_sized() {
        let s = Sheets::build();
        for (sp, f, w, h, px) in s.frame_stats() {
            assert!(w >= 5 && h >= 2, "species {sp} frame {f}: {w}x{h} too small");
            assert!(px >= 6, "species {sp} frame {f}: only {px} px");
            // The farm machines (species 13 and 14) are car-sized.
            let cap = if sp >= 13 { 60 } else { 16 };
            assert!(
                w <= cap && h <= cap,
                "species {sp} frame {f}: {w}x{h} out of the 64-px budget"
            );
        }
        // The deer sheet carries both variants; stags are taller (antlers).
        assert!(s.deer[0].h > s.deer[2].h, "stag must be taller than the doe");
        // Waterfowl carry a wake.
        assert!(s.duck[0].px.iter().any(|&c| c == WAKE));
        assert!(s.swan[0].px.iter().any(|&c| c == WAKE));
        // Owls perch on a branch.
        assert!(s.owl[0].px.iter().any(|&c| c == BRANCH));
    }
}
