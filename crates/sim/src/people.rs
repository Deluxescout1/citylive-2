//! Residents and their daily life (D65, B43).
//!
//! One `Person` per resident the growth sim already counts: a name, an age, a
//! home, a job (or a school, or nothing), and family links. People are made
//! **deterministically** from `(seed, index)`: growing a city to the same day
//! twice, on two machines, leaves the same residents with the same names.
//!
//! The day is a **schedule**, not a state machine: every figure's position is a
//! pure function of the sim clock, the person and the city. Nothing walks from
//! one planning step to the next, so a month of catch-up costs exactly as much
//! as a minute, and the renderer can ask for the same hour twice and get the
//! same picture.
//!
//! What the renderer gets back is a flat list of [`Figure`]s at true scale: 3
//! pixels wide, 9 tall (children smaller), each with a pose (walking,
//! hammering, carrying a plank, chatting, playing, waving, sitting) and an
//! optional prop (a dog, a ball, a plank, a wheelbarrow).

use map::{Kind, TileMap};

use crate::{Field, Lot, LotKind, Stage};

/// Hard cap on the tracked population. A metropolis holds more people than the
/// wallpaper can ever show at once; the extra heads are turned away so a save
/// and the per-hour sync stay small (D65: "many more", not "all of them").
pub const MAX_PEOPLE: usize = 6000;

/// Walking speed along a commute corridor, in tiles per sim second. A tile is
/// roughly eight metres, so this is a brisk walk at the wallpaper's scale: a
/// 20-tile commute takes 100 sim seconds and reads as continuous motion at
/// 6 fps.
pub const WALK_TILES_S: f32 = 0.2;

// --- names ------------------------------------------------------------

/// First names, with a flag for the ones given to girls (a wedding needs a
/// bride, a birth a mother). The first 48 are the original B43 table in its
/// original order, so a save written before B48 keeps every name it had; new
/// names only ever append.
pub(crate) const FIRST: [(&str, bool); 133] = [
    ("Abigail", true), ("Alan", false), ("Amos", false), ("Anna", true), ("Arthur", false),
    ("Beatrice", true), ("Ben", false), ("Caleb", false), ("Clara", true), ("Daniel", false),
    ("Edith", true), ("Edwin", false), ("Eli", false), ("Eliza", true), ("Ella", true),
    ("Emmett", false), ("Ethan", false), ("Eva", true), ("Ezra", false), ("Faye", true),
    ("Felix", false), ("Flora", true), ("Frank", false), ("Grace", true), ("Hannah", true),
    ("Harold", false), ("Henry", false), ("Hugh", false), ("Ida", true), ("Iris", true),
    ("Isaac", false), ("Jacob", false), ("Jane", true), ("Jonah", false), ("Josephine", true),
    ("Kate", true), ("Levi", false), ("Lily", true), ("Lucy", true), ("Mabel", true),
    ("Martha", true), ("Nell", true), ("Noah", false), ("Olive", true), ("Owen", false),
    ("Rose", true), ("Ruth", true), ("Samuel", false),
    // B48: a wider, plainer mix of the world's names.
    ("Maya", true), ("Amara", true), ("Priya", true), ("Mei", true), ("Yuki", true),
    ("Sofia", true), ("Lucia", true), ("Nadia", true), ("Leila", true), ("Zara", true),
    ("Ingrid", true), ("Freya", true), ("Astrid", true), ("Esme", true), ("Marisol", true),
    ("Aiko", true), ("Thandi", true), ("Kalinda", true), ("Rosa", true), ("Odette", true),
    ("Mina", true), ("Tamsin", true), ("Wren", true), ("Imani", true), ("Noor", true),
    ("Selma", true), ("Greta", true), ("Anjali", true), ("Carmen", true), ("Dalia", true),
    ("Elena", true), ("Fatima", true), ("Hana", true), ("Ines", true), ("Kira", true),
    ("Lena", true), ("Mira", true), ("Nia", true), ("Paloma", true), ("Vera", true),
    ("Theo", false), ("Kofi", false), ("Ravi", false), ("Hiro", false), ("Mateo", false),
    ("Omar", false), ("Tomas", false), ("Anders", false), ("Jasper", false), ("Silas", false),
    ("Dmitri", false), ("Idris", false), ("Kenji", false), ("Luca", false), ("Nico", false),
    ("Pavel", false), ("Quentin", false), ("Rafael", false), ("Soren", false), ("Tariq", false),
    ("Viktor", false), ("Wen", false), ("Yusuf", false), ("Zane", false), ("Andre", false),
    ("Bram", false), ("Cyrus", false), ("Dario", false), ("Emeka", false), ("Finn", false),
    ("Gideon", false), ("Hamid", false), ("Ivan", false), ("Joaquin", false), ("Kaito", false),
    ("Leo", false), ("Malik", false), ("Niall", false), ("Otis", false), ("Piotr", false),
    ("Rhys", false), ("Sami", false), ("Tobias", false), ("Ulrich", false), ("Yosef", false),
];

/// Surnames. The first 40 are the B43 table, unchanged and in order.
pub(crate) const LAST: [&str; 101] = [
    "Abbott", "Baker", "Bishop", "Brewer", "Carter", "Cooper", "Dalton", "Doyle", "Ellis", "Foster",
    "Gardner", "Grant", "Hale", "Harding", "Hayes", "Holt", "Hughes", "Keller", "Lane", "Marsh",
    "Mercer", "Mills", "Moore", "Nash", "Osborne", "Page", "Palmer", "Pierce", "Porter", "Reed",
    "Rowe", "Sawyer", "Shaw", "Sherman", "Stone", "Taylor", "Vance", "Wade", "Whitaker", "Young",
    "Okafor", "Nakamura", "Petrov", "Santos", "Haddad", "Lindqvist", "Moreau", "Banerjee",
    "Castillo", "Nguyen", "Kowalski", "Adeyemi", "Fischer", "Rossi", "Tanaka", "Alvarez", "Kim",
    "Osei", "Hartmann", "Larsen", "Dubois", "Mensah", "Iyer", "Brandt", "Novak", "Ferreira",
    "Yilmaz", "Calloway", "Thorne", "Vasquez", "Okonkwo", "Sato", "Lund", "Medina", "Rahman",
    "Costa", "Weber", "Ito", "Duarte", "Byrne", "Chen", "Diaz", "Eriksson", "Fontaine", "Gupta",
    "Haas", "Ibrahim", "Jovanovic", "Khan", "Lopez", "Mbeki", "Nilsson", "Ortiz", "Park", "Quinn",
    "Rivera", "Singh", "Torres", "Ueda", "Varga", "Zielinski",
];

/// The jacket/coat colours people wear, in the palette's own order. Kept as an
/// index so a person's clothes never change when the palette is retinted.
pub const TONES: usize = 8;

/// Two coats the life events add on top of the everyday eight (B48): the
/// bride's white and the dark of a funeral or a groom's suit. Nobody is *born*
/// into them; only a scene dresses a figure in one.
pub const TONE_WHITE: u8 = 10;
pub const TONE_DARK: u8 = 11;

/// How many sim days make one year of a resident's life (B48, retuned by B50).
/// A world lasts about a month (30 days +-30 %, `ending::State::plan`), and a
/// world should span about three generations: 0.4 days to the year is 75 life
/// years in 30 days, so the first settlers' grandchildren are grown when the
/// end comes. (B48 had 3 days a year, a decade per world.)
pub const LIFE_YEAR_DAYS: f64 = 0.4;

/// Is the person still in the city (B48)? The list keeps everyone who ever
/// lived here, so `spouse`/`parent` indices never shift; the dead and the
/// departed stay as quiet entries that nothing draws or counts.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Life {
    #[default]
    Alive,
    Dead,
    /// Moved away (hard times, or a child taken in elsewhere).
    Away,
}

impl Life {
    pub fn to_u8(self) -> u8 {
        match self {
            Life::Alive => 0,
            Life::Dead => 1,
            Life::Away => 2,
        }
    }

    pub fn from_u8(v: u8) -> Life {
        match v {
            1 => Life::Dead,
            2 => Life::Away,
            _ => Life::Alive,
        }
    }
}

// --- people -----------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AgeGroup {
    /// 18-64: works if there is work.
    Adult,
    /// Under 18: school, play, home.
    Child,
    /// 65 and over: the park, the porch, the shop.
    Elder,
}

/// Where a person works. `Shop` keeps its keeper at the door; `Office` and
/// `Factory` swallow their people and spill them at shift change.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum JobKind {
    #[default]
    None,
    Office,
    Factory,
    Shop,
    School,
    Farm,
}

impl JobKind {
    pub fn to_u8(self) -> u8 {
        match self {
            JobKind::None => 0,
            JobKind::Office => 1,
            JobKind::Factory => 2,
            JobKind::Shop => 3,
            JobKind::School => 4,
            JobKind::Farm => 5,
        }
    }

    pub fn from_u8(v: u8) -> JobKind {
        match v {
            1 => JobKind::Office,
            2 => JobKind::Factory,
            3 => JobKind::Shop,
            4 => JobKind::School,
            5 => JobKind::Farm,
            _ => JobKind::None,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Person {
    /// Indices into [`FIRST`] / [`LAST`].
    pub first: u8,
    pub last: u8,
    pub age: u16,
    pub kind: AgeGroup,
    /// Home door tile (just outside the lot), None for someone who lost a home.
    pub home: Option<(i32, i32)>,
    /// Work door tile, or the top-left tile of a field for a farm hand.
    pub job: Option<(i32, i32)>,
    pub job_kind: JobKind,
    /// Partner and parent, by `id` (the index in [`People::list`]).
    pub spouse: Option<u32>,
    pub parent: Option<u32>,
    /// Which coat: 0..TONES.
    pub tone: u8,
    /// Small per-person offset so two neighbours do not stand on one another.
    pub jitter: u8,
    /// The sim day this person was born (negative for the town's first
    /// generation), so age follows the clock (B48).
    pub born: i32,
    pub life: Life,
}

impl Person {
    pub fn name(&self) -> String {
        format!("{} {}", self.given(), self.surname())
    }

    /// The given name alone.
    pub fn given(&self) -> &'static str {
        FIRST[self.first as usize % FIRST.len()].0
    }

    /// Was this person given a girl's name? (A wedding's bride, a baby's
    /// mother.)
    pub fn female(&self) -> bool {
        FIRST[self.first as usize % FIRST.len()].1
    }

    /// Is the first name at this index a girl's? (A baby's name is drawn
    /// for the sex first.)
    pub fn female_index(i: u8) -> bool {
        FIRST[i as usize % FIRST.len()].1
    }

    pub fn alive(&self) -> bool {
        self.life == Life::Alive
    }

    /// The family name alone ("the Miller family").
    pub fn surname(&self) -> String {
        LAST[self.last as usize % LAST.len()].to_string()
    }
}

/// `true` between 08:30 and 17:00 on a working day.
fn work_hours(hour: f64) -> bool {
    (8.5..17.25).contains(&hour)
}

/// What a resident is doing: the schedule the renderer turns into a pose.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Act {
    /// Indoors, asleep. Nobody is on the street.
    Asleep,
    /// At home: porch, yard, door.
    Home,
    /// Walking to work (or school) — `to_work` false means homeward.
    Commute { to_work: bool },
    /// Inside the building (drawn only at breaks and shift change).
    Work,
    /// Out for lunch, at the door.
    Lunch,
    /// The evening: the park, the shops, a walk.
    Leisure,
    /// In class (drawn in the yard at break).
    School,
}

/// The day's shape, shared by every adult with a job. One place to tune the
/// whole city's rhythm (D65: rush hours busy, three in the morning empty).
/// The day's shape for one resident. `phase` (0..1, stable per person) staggers
/// the hours a trade keeps, so a high street is not one shop opening and closing
/// on the same minute.
pub fn schedule_at(
    kind: AgeGroup,
    job: JobKind,
    hour: f64,
    weekend: bool,
    phase: f32,
) -> Act {
    // The small hours: all indoors.
    if !(5.9..22.6).contains(&hour) {
        return Act::Asleep;
    }
    match kind {
        AgeGroup::Child => {
            if weekend {
                if (8.0..18.5).contains(&hour) {
                    Act::Leisure
                } else if hour >= 19.5 {
                    Act::Home
                } else {
                    Act::Home
                }
            } else if job == JobKind::School {
                if (7.1..8.45).contains(&hour) {
                    Act::Commute { to_work: true }
                } else if (8.45..12.0).contains(&hour) || (12.8..15.2).contains(&hour) {
                    Act::School
                } else if (12.0..12.8).contains(&hour) {
                    Act::Lunch
                } else if (15.2..15.9).contains(&hour) {
                    Act::Commute { to_work: false }
                } else if (15.9..18.6).contains(&hour) {
                    Act::Leisure
                } else {
                    Act::Home
                }
            } else if (9.0..18.0).contains(&hour) {
                // No school in this town yet: the yard and the park.
                Act::Leisure
            } else {
                Act::Home
            }
        }
        AgeGroup::Elder => {
            if weekend && (9.0..20.0).contains(&hour) {
                Act::Leisure
            } else if (8.5..11.5).contains(&hour) || (13.5..17.5).contains(&hour) {
                Act::Leisure
            } else {
                Act::Home
            }
        }
        AgeGroup::Adult => {
            let off = weekend || job == JobKind::None;
            if off {
                if (9.0..20.5).contains(&hour) {
                    Act::Leisure
                } else {
                    Act::Home
                }
            } else if job == JobKind::Shop {
                // The keeper walks to the shop in the morning and walks home
                // when the shutter comes down — and the shutter comes down at
                // *their* hour, anywhere from about five to half past six.
                // B46c: every shopkeeper used to stand inside until 18:30 and
                // then appear at home, so a town where a third of the adults
                // keep shops had nobody on the pavement at 17:30 (seed 3, the
                // tundra world, failed the rush-hour test for exactly that).
                let (open, close) = shop_hours(phase);
                if hour < open {
                    Act::Home
                } else if hour < open + SHOP_WALK_IN {
                    Act::Commute { to_work: true }
                } else if hour < close {
                    Act::Work
                } else if hour < close + SHOP_WALK_OUT {
                    Act::Commute { to_work: false }
                } else {
                    Act::Leisure
                }
            } else if job == JobKind::Farm {
                if (6.4..11.9).contains(&hour) || (12.8..18.0).contains(&hour) {
                    Act::Work
                } else if (11.9..12.8).contains(&hour) {
                    Act::Lunch
                } else {
                    Act::Home
                }
            } else if (6.9..8.7).contains(&hour) {
                Act::Commute { to_work: true }
            } else if work_hours(hour) {
                if (12.0..12.8).contains(&hour) {
                    Act::Lunch
                } else {
                    Act::Work
                }
            } else if (17.0..18.6).contains(&hour) {
                Act::Commute { to_work: false }
            } else if (18.6..21.8).contains(&hour) {
                Act::Leisure
            } else {
                Act::Home
            }
        }
    }
}

/// A shop's opening and closing hour: 7:00-16:54 at the earliest, 7:48-18:24 at
/// the latest, spread over the trade by the keeper's own phase.
const SHOP_WALK_IN: f64 = 0.5;
const SHOP_WALK_OUT: f64 = 0.6;

fn shop_hours(phase: f32) -> (f64, f64) {
    let p = (phase as f64).clamp(0.0, 1.0);
    (7.0 + 0.8 * p, 16.9 + 1.6 * p)
}

/// The plain schedule, for the tests and any caller with no resident in hand.
pub fn schedule(kind: AgeGroup, job: JobKind, hour: f64, weekend: bool) -> Act {
    schedule_at(kind, job, hour, weekend, 0.5)
}

// --- what the renderer draws -----------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pose {
    Walk,
    Stand,
    /// Hammering, hoeing, sweeping: one arm works.
    Work,
    /// A plank or a sack across the chest.
    Carry,
    /// Two people turned towards each other.
    Chat,
    /// Arms out, running about (children).
    Play,
    /// One arm up, from a door or a kerb.
    Wave,
    /// On a bench.
    Sit,
    /// B77: not a person — a piece of the week's furniture (a bin at the kerb,
    /// a market stall). The body is blank and only the prop is drawn, so it
    /// rides the people layer's depth test and damage rects.
    Object,
}

impl Pose {
    /// The renderer's pose code (same order as `render::people`).
    pub fn to_u8(self) -> u8 {
        match self {
            Pose::Walk => 0,
            Pose::Stand => 1,
            Pose::Work => 2,
            Pose::Carry => 3,
            Pose::Chat => 4,
            Pose::Play => 5,
            Pose::Wave => 6,
            Pose::Sit => 7,
            Pose::Object => 8,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Prop {
    #[default]
    None,
    /// A plank across the chest (a builder carrying timber).
    Lumber,
    /// A wheelbarrow pushed in front.
    Barrow,
    /// A dog on a lead.
    Dog,
    /// A ball at the feet.
    Ball,
    /// A pram pushed in front (B48: a baby in the household).
    Stroller,
    /// A removal van (B48: a family moving in or out).
    Truck,
    /// Confetti in the air over a waving guest (B48: a wedding).
    Confetti,
    /// A flower arch over the couple (B48: a wedding).
    Arch,
    /// A hearse at the cemetery gate (B48: a funeral).
    Hearse,
    /// B75: a shopping bag swinging from the hand (a shopper, a window-shopper).
    Bag,
    /// B75: an umbrella over a figure in the rain.
    Umbrella,
    /// B77: a wheelie bin at the kerb on bin day.
    Bin,
    /// B77: a market stall — awning, counter, goods.
    Stall,
    /// B77: a cardboard box, carried on a moving day or stacked by a stall.
    Box,
}

impl Prop {
    pub fn to_u8(self) -> u8 {
        match self {
            Prop::None => 0,
            Prop::Lumber => 1,
            Prop::Barrow => 2,
            Prop::Dog => 3,
            Prop::Ball => 4,
            Prop::Stroller => 5,
            Prop::Truck => 6,
            Prop::Confetti => 7,
            Prop::Arch => 8,
            Prop::Hearse => 9,
            Prop::Bag => 10,
            Prop::Umbrella => 11,
            Prop::Bin => 12,
            Prop::Stall => 13,
            Prop::Box => 14,
        }
    }
}

/// One figure to draw, at a continuous tile position with its feet on the
/// ground.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Figure {
    pub x: f32,
    pub y: f32,
    pub pose: Pose,
    /// 0..TONES coat colour.
    pub tone: u8,
    pub child: bool,
    pub prop: Prop,
    /// Facing: 0 = +x, 1 = +y, 2 = -x, 3 = -y.
    pub dir: u8,
    /// Standing on an open lot (a park or a construction site) rather than a
    /// street: the renderer must let the figure paint over the plot's own
    /// painter key, or the park's lawn would hide everybody on it.
    pub open: bool,
    /// How far above the ground the feet are, in pixels (a worker on a
    /// scaffold), so the figure stands on the frame rather than beside it.
    pub lift: u8,
}

// --- state ------------------------------------------------------------

/// A finished home, as the sim sees it.
#[derive(Clone, Copy, Debug)]
pub struct HomeSlot {
    pub x: i32,
    pub y: i32,
    pub door: (i32, i32),
    pub capacity: u32,
}

/// A finished workplace.
#[derive(Clone, Copy, Debug)]
pub struct JobSlot {
    pub x: i32,
    pub y: i32,
    pub door: (i32, i32),
    pub kind: JobKind,
}

/// Somewhere people gather (B43b): a park or plaza (open ground), a shop front,
/// the church steps, the hall — or, in a village with none of those yet, the
/// middle of the oldest street. Without this a brand-new village had nowhere to
/// socialise and everyone just stood on their own porch.
#[derive(Clone, Copy, Debug)]
pub struct Venue {
    pub x: f32,
    pub y: f32,
    /// How wide the gathering is (a park is wide, a shop front is a doorway).
    pub span: f32,
    /// Open ground: figures may stand on the plot itself.
    pub open: bool,
    /// A park: benches and children at play.
    pub park: bool,
}

/// A household that moved in (B48): the door, the family name and how many.
#[derive(Clone, Copy, Debug)]
pub struct Arrival {
    pub door: (i32, i32),
    pub surname: u8,
    pub size: u8,
}

/// Children start school at this age.
pub const SCHOOL_AGE: u16 = 5;

/// The city a person lives in, borrowed for one sync or one view.
pub struct World<'a> {
    pub map: &'a TileMap,
    pub lots: &'a [Lot],
    pub fields: &'a [Field],
    pub seed: u64,
}

impl<'a> World<'a> {
    fn parks(&self) -> Vec<&'a Lot> {
        self.lots
            .iter()
            .filter(|l| l.kind == LotKind::Park || l.kind == LotKind::Landmark(crate::landmarks::LandmarkKind::Plaza))
            .collect()
    }

    /// The places people socialise, in a stable order: parks and plazas first,
    /// then the shop/store/church/hall doors, then the village centre.
    pub fn venues(&self) -> Vec<Venue> {
        let mut v: Vec<Venue> = Vec::new();
        for l in self.lots.iter() {
            if l.kind == LotKind::Park
                || l.kind == LotKind::Landmark(crate::landmarks::LandmarkKind::Plaza)
            {
                v.push(Venue {
                    x: l.x as f32 + l.w as f32 * 0.5,
                    y: l.y as f32 + l.h as f32 * 0.5,
                    span: (l.w.min(l.h) as f32 * 0.42).max(1.2),
                    open: true,
                    park: true,
                });
            }
        }
        for l in self.lots.iter() {
            if !l.finished() {
                continue;
            }
            if matches!(
                l.kind,
                LotKind::Shop
                    | LotKind::Store
                    | LotKind::Church
                    | LotKind::Hall
                    | LotKind::LogCabin
            ) {
                let (dx, dy) = door_tile(l);
                v.push(Venue {
                    x: dx as f32 + 0.5,
                    y: dy as f32 + 0.5,
                    span: 1.3,
                    open: false,
                    park: false,
                });
            }
        }
        if v.is_empty() {
            // A village with no civic building yet: the street outside the
            // oldest lot is where everybody meets, which is true of villages.
            if let Some(l) = self.lots.first() {
                let (dx, dy) = door_tile(l);
                v.push(Venue {
                    x: dx as f32 + 0.5,
                    y: dy as f32 + 0.5,
                    span: 1.8,
                    open: false,
                    park: false,
                });
            }
        }
        v
    }

    pub fn school(&self) -> Option<&'a Lot> {
        self.lots.iter().find(|l| {
            l.finished()
                && matches!(l.kind, LotKind::Landmark(crate::landmarks::LandmarkKind::School))
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct People {
    pub list: Vec<Person>,
    /// Commute corridors (home door → work door), rebuilt when the street
    /// network or the buildings change. Deterministic, not saved.
    corridors: Vec<Vec<(i32, i32)>>,
    /// B75: stroll routes (a home door → a gathering place). The ambient half of
    /// the street: people who are not commuting but are still out using the
    /// world, walking a pavement rather than standing on a single tile.
    strolls: Vec<Vec<(i32, i32)>>,
    /// B75: the sky is wet (rain or snow, set by the wallpaper's weather note):
    /// people outdoors carry an umbrella. Not saved — the weather is not ours.
    pub wet: bool,
    /// What the corridors were planned from, so a rebuild is skipped when
    /// nothing that matters has changed.
    signature: (usize, usize, usize),
    /// B48: how many of `list` are still in the city (cached; recount()).
    living: usize,
    /// B48: the scenes the life events stage (weddings, funerals, moving
    /// trucks), told once and drawn while they last. Saved.
    pub scenes: Vec<crate::life::LifeScene>,
    /// B48: the last sim day the life pass ran. Saved.
    pub last_day: i64,
    /// B48: adults pushing a pram, derived from the list (not saved).
    pub(crate) walkers: Vec<u32>,
    /// B48: the town is in a slump (derived from the day; nobody is hired).
    pub(crate) slump: bool,
    /// B52: the economy says the town is in a slump (set by the sim before the
    /// life pass; the layoffs and the leaving rate follow it). Not saved.
    pub(crate) econ_slump: bool,
}

impl People {
    /// B84: the bytes this layer holds (the residents, the cached routes and
    /// the staged life scenes), for the wallpaper's memory report.
    pub fn bytes(&self) -> usize {
        fn v<T>(x: &Vec<T>) -> usize {
            x.capacity() * std::mem::size_of::<T>()
        }
        fn nested<T>(x: &Vec<Vec<T>>) -> usize {
            x.capacity() * std::mem::size_of::<Vec<T>>() + x.iter().map(v).sum::<usize>()
        }
        v(&self.list)
            + nested(&self.corridors)
            + nested(&self.strolls)
            + v(&self.scenes)
            + v(&self.walkers)
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    /// Commute corridors currently planned (diagnostics / handback).
    pub fn corridor_count(&self) -> usize {
        self.corridors.len()
    }

    /// Total tiles across the corridors (diagnostics).
    pub fn corridor_tiles(&self) -> usize {
        self.corridors.iter().map(|c| c.len()).sum()
    }

    /// How much of the commute is on the street: lanes, pavements and worn
    /// footpaths, as a fraction of every corridor tile (B43b item 2).
    pub fn corridor_street_share(&self, map: &TileMap) -> f32 {
        let mut n = 0usize;
        let mut on = 0usize;
        for c in &self.corridors {
            for &(x, y) in c {
                n += 1;
                let t = map.at(x, y);
                if t.kind.is_lane()
                    || t.kind == Kind::Sidewalk
                    || map::WearStage::of(map.wear(x, y)) >= map::WearStage::Footpath
                {
                    on += 1;
                }
            }
        }
        if n == 0 {
            1.0
        } else {
            on as f32 / n as f32
        }
    }

    pub fn get(&self, id: u32) -> Option<&Person> {
        self.list.get(id as usize)
    }

    pub fn living(&self) -> usize {
        self.living
    }

    /// Re-derive the cached counts after the list changed outside the life
    /// pass (a load, a test that edits the roster).
    pub fn recount(&mut self) {
        self.living = self.list.iter().filter(|p| p.alive()).count();
        self.refresh_walkers();
    }

    /// The adults who push a pram: the mother (the baby's `parent`) while the
    /// baby is under two years old. Derived, so a load recomputes it.
    pub fn refresh_walkers(&mut self) {
        let mut w: Vec<u32> = Vec::new();
        for p in self.list.iter() {
            if p.alive() && p.age < 2 {
                if let Some(m) = p.parent {
                    if self.list.get(m as usize).map(|m| m.alive()).unwrap_or(false) {
                        w.push(m);
                    }
                }
            }
        }
        w.sort_unstable();
        w.dedup();
        self.walkers = w;
    }

    /// Append households until the city holds `target`, keep the roster
    /// attached to the buildings that exist, and re-plan the commute corridors
    /// when its streets or buildings changed. Called once per growth decision
    /// (an hour of sim time), so it costs nothing per frame. `day` is the sim
    /// day, so a newcomer's birthday follows the clock (B48). Returns the
    /// households that moved in, for the moving truck and the feed.
    pub fn sync(
        &mut self,
        w: &World,
        target: u32,
        homes: &[HomeSlot],
        jobs: &[JobSlot],
        day: i64,
    ) -> Vec<Arrival> {
        self.repair(w, homes, jobs);
        let arrivals = self.grow(w, target, homes, jobs, day);
        self.plan_corridors(w);
        arrivals
    }

    /// Drop jobs that no longer exist (a farm ploughed under by the suburbs, a
    /// workshop demolished), so nobody commutes to a hole in the ground.
    fn repair(&mut self, w: &World, homes: &[HomeSlot], jobs: &[JobSlot]) {
        if homes.is_empty() {
            return;
        }
        let doors: Vec<(i32, i32)> = homes.iter().map(|h| h.door).collect();
        let door_set: std::collections::BTreeSet<(i32, i32)> = doors.iter().copied().collect();
        // A door is "still there" if a finished home has a door nearby: a house
        // rebuilt bigger moves its door a tile or two, it does not vanish. The
        // household follows the door to its new tile (B48: the roster counts
        // beds by door, so a stale tile would leave a house looking empty).
        let snap = |h: (i32, i32)| -> Option<(i32, i32)> {
            if door_set.contains(&h) {
                return Some(h);
            }
            for r in 1..=4i32 {
                for dy in -r..=r {
                    for dx in -r..=r {
                        if dx.abs() != r && dy.abs() != r {
                            continue;
                        }
                        let c = (h.0 + dx, h.1 + dy);
                        if door_set.contains(&c) {
                            return Some(c);
                        }
                    }
                }
            }
            None
        };
        let n = self.list.len();
        let alive: Vec<bool> = self.list.iter().map(|p| p.alive()).collect();
        let mut home: Vec<Option<(i32, i32)>> = Vec::with_capacity(n);
        for (i, p) in self.list.iter().enumerate() {
            home.push(if alive[i] { p.home.and_then(snap) } else { None });
        }
        // A partner keeps the couple together, then anyone still out in the
        // cold takes a door picked by their old address, so a whole household
        // whose house went finds the same new one (deterministic).
        for id in 0..n {
            if alive[id] && home[id].is_none() {
                if let Some(sp) = self.list[id].spouse {
                    if let Some(h) = home.get(sp as usize).copied().flatten() {
                        home[id] = Some(h);
                    }
                }
            }
        }
        for id in 0..n {
            if alive[id] && home[id].is_none() {
                let key = match self.list[id].home {
                    Some((x, y)) => {
                        (x.wrapping_mul(73_856_093) ^ y.wrapping_mul(19_349_663)) as u32 as usize
                    }
                    None => id,
                };
                home[id] = Some(doors[key % doors.len()]);
            }
        }
        for id in 0..n {
            if !alive[id] {
                continue;
            }
            if let Some(sp) = self.list[id].spouse {
                if (sp as usize) < n && id < sp as usize && alive[sp as usize] {
                    let h = home[id];
                    home[sp as usize] = h;
                }
            }
        }
        for (id, p) in self.list.iter_mut().enumerate() {
            if alive[id] {
                p.home = home[id];
            }
        }
        let job_doors: std::collections::BTreeSet<(i32, i32)> =
            jobs.iter().map(|j| j.door).collect();
        for p in self.list.iter_mut() {
            if !p.alive() {
                continue;
            }
            if let Some(j) = p.job {
                let there = job_doors.contains(&j)
                    || (p.job_kind == JobKind::Farm
                        && w.fields
                            .iter()
                            .any(|f| j.0 >= f.x && j.0 < f.x + f.w && j.1 >= f.y && j.1 < f.y + f.h));
                if !there {
                    p.job = None;
                    p.job_kind = JobKind::None;
                }
            }
        }
        // A rebuilt building moves its door, which would leave its whole staff
        // unemployed for good. Give the working-age and the schoolchildren a
        // new place, deterministically (four in five adults work).
        if jobs.is_empty() {
            return;
        }
        let school = jobs.iter().find(|j| j.kind == JobKind::School).copied();
        let walkers = &self.walkers;
        let slump = self.slump;
        for (id, p) in self.list.iter_mut().enumerate() {
            if p.job.is_some() || !p.alive() {
                continue;
            }
            match p.kind {
                AgeGroup::Adult
                    if ((id as u64).wrapping_mul(2_654_435_761) >> 8) % 10 < 8
                        && !slump
                        && !walkers.contains(&(id as u32)) =>
                {
                    let j = jobs[(id * 7) % jobs.len()];
                    p.job = Some(j.door);
                    p.job_kind = j.kind;
                }
                AgeGroup::Child if p.age >= SCHOOL_AGE => {
                    if let Some(j) = school {
                        p.job = Some(j.door);
                        p.job_kind = JobKind::School;
                    }
                }
                _ => {}
            }
        }
    }

    /// Move households into the empty beds until the city holds `target`
    /// people (B48). A house takes one household at a time: a couple, a
    /// family, roommates, an old pair or one person; a block of flats takes
    /// one after another. A home that has never housed anyone fills at once
    /// (the street grew, so people came); a home someone left is taken again
    /// only now and then, so a vacancy shows for a few days.
    fn grow(
        &mut self,
        w: &World,
        target: u32,
        homes: &[HomeSlot],
        jobs: &[JobSlot],
        day: i64,
    ) -> Vec<Arrival> {
        let mut out = Vec::new();
        let want = (target as usize).min(MAX_PEOPLE);
        // The roster keeps the dead and the departed so ids never shift; cap
        // that history as well, or a thousand-year city would grow it forever.
        if want <= self.living || homes.is_empty() || self.list.len() >= MAX_PEOPLE * 2 {
            return out;
        }
        let mut occ: std::collections::BTreeMap<(i32, i32), u32> =
            std::collections::BTreeMap::new();
        let mut ever: std::collections::BTreeSet<(i32, i32)> = std::collections::BTreeSet::new();
        for p in self.list.iter() {
            if let Some(h) = p.home {
                ever.insert(h);
                if p.alive() && p.age >= 2 {
                    *occ.entry(h).or_insert(0) += 1;
                }
            }
        }
        let has_school = jobs.iter().any(|j| j.kind == JobKind::School);
        for h in homes {
            if self.living >= want || self.list.len() >= MAX_PEOPLE * 2 {
                break;
            }
            let taken = occ.get(&h.door).copied().unwrap_or(0);
            let mut free = h.capacity.saturating_sub(taken);
            if free == 0 {
                continue;
            }
            // A house is one household's; a flat block holds several.
            if h.capacity <= 4 && taken > 0 {
                continue;
            }
            if ever.contains(&h.door) && taken == 0 {
                // A vacancy: somebody answers the advert about one day in five.
                let k = map::rng::hash2(
                    h.door.0,
                    h.door.1,
                    w.seed ^ (day as u64).wrapping_mul(0x9E37),
                );
                if k > 0.2 {
                    continue;
                }
            }
            while free > 0 && self.living < want {
                let id0 = self.list.len() as u64;
                let mut rng = map::rng::Rng::new(
                    w.seed ^ id0.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xB43_9E0,
                );
                // How many beds this household wants: usually the whole house.
                let goal = if h.capacity <= 4 {
                    let r = rng.f32();
                    if r < 0.60 {
                        4
                    } else if r < 0.75 {
                        3
                    } else if r < 0.90 {
                        2
                    } else {
                        1
                    }
                } else {
                    1 + rng.usize(4) as u32
                };
                let goal = goal.min(free).min((want - self.living) as u32).max(1) as usize;
                let (made, surname) =
                    self.new_household(&mut rng, h.door, goal, day, jobs, has_school);
                if made == 0 {
                    break;
                }
                self.living += made;
                free = free.saturating_sub(made as u32);
                out.push(Arrival {
                    door: h.door,
                    surname,
                    size: made as u8,
                });
                if h.capacity <= 4 {
                    break;
                }
            }
        }
        out
    }

    /// One household of exactly `n` people at `door`: who they are follows
    /// the number of beds they fill. Returns how many were made and the family
    /// name.
    fn new_household(
        &mut self,
        rng: &mut map::rng::Rng,
        door: (i32, i32),
        n: usize,
        day: i64,
        jobs: &[JobSlot],
        has_school: bool,
    ) -> (usize, u8) {
        #[derive(Clone, Copy)]
        enum Mix {
            Single,
            ElderOne,
            Couple,
            Roommates(usize),
            ElderPair,
            Parent(usize),
            Family(usize),
        }
        let r = rng.f32();
        let mix = match n {
            1 => {
                if r < 0.8 {
                    Mix::Single
                } else {
                    Mix::ElderOne
                }
            }
            2 => {
                if r < 0.40 {
                    Mix::Couple
                } else if r < 0.60 {
                    Mix::Roommates(2)
                } else if r < 0.80 {
                    Mix::ElderPair
                } else {
                    Mix::Parent(1)
                }
            }
            3 => {
                if r < 0.50 {
                    Mix::Family(1)
                } else if r < 0.75 {
                    Mix::Parent(2)
                } else {
                    Mix::Roommates(3)
                }
            }
            _ => {
                if r < 0.80 {
                    Mix::Family(n - 2)
                } else if r < 0.90 {
                    Mix::Parent(n - 1)
                } else {
                    Mix::Roommates(n)
                }
            }
        };
        let surname = rng.usize(LAST.len()) as u8;
        let id0 = self.list.len() as u32;
        let born_of = |age: u16, rng: &mut map::rng::Rng| -> i32 {
            day as i32 - (age as f64 * LIFE_YEAR_DAYS) as i32 - rng.usize(3) as i32
        };
        let pick_first = |rng: &mut map::rng::Rng, female: bool| -> u8 {
            for _ in 0..64 {
                let i = rng.usize(FIRST.len());
                if FIRST[i].1 == female {
                    return i as u8;
                }
            }
            0
        };
        let kind_of = |age: u16| match age {
            0..=17 => AgeGroup::Child,
            18..=64 => AgeGroup::Adult,
            _ => AgeGroup::Elder,
        };
        // The adults first, then the children; every child points at the
        // mother (or the only parent), so a pram has somebody to push it.
        let mut adults: Vec<(u8, u8, u16)> = Vec::new(); // first, last, age
        let mut kids = 0usize;
        let mut paired = false;
        match mix {
            Mix::Single | Mix::ElderOne => {
                let age = if matches!(mix, Mix::Single) {
                    rng.range_f32(19.0, 64.0) as u16
                } else {
                    rng.range_f32(65.0, 88.0) as u16
                };
                let female = rng.chance(0.5);
                adults.push((pick_first(rng, female), surname, age));
            }
            Mix::Couple | Mix::Family(_) | Mix::ElderPair => {
                let elder = matches!(mix, Mix::ElderPair);
                let a1 = if elder {
                    rng.range_f32(65.0, 86.0) as u16
                } else if matches!(mix, Mix::Family(_)) {
                    rng.range_f32(25.0, 56.0) as u16
                } else {
                    rng.range_f32(20.0, 60.0) as u16
                };
                let gap = rng.range_f32(-5.0, 5.0) as i32;
                let (lo, hi) = if elder { (65, 90) } else { (20, 64) };
                let a2 = (a1 as i32 + gap).clamp(lo, hi) as u16;
                let f1 = rng.chance(0.5);
                adults.push((pick_first(rng, f1), surname, a1));
                adults.push((pick_first(rng, !f1), surname, a2));
                paired = true;
                if let Mix::Family(k) = mix {
                    kids = k;
                }
            }
            Mix::Parent(k) => {
                let age = rng.range_f32(26.0, 56.0) as u16;
                let female = rng.chance(0.65);
                adults.push((pick_first(rng, female), surname, age));
                kids = k;
            }
            Mix::Roommates(k) => {
                for i in 0..k {
                    let age = rng.range_f32(19.0, 46.0) as u16;
                    let last = if i == 0 { surname } else { rng.usize(LAST.len()) as u8 };
                    let female = rng.chance(0.5);
                    adults.push((pick_first(rng, female), last, age));
                }
            }
        }
        // Who is the household's mother: the first woman among the adults.
        let mother = adults
            .iter()
            .position(|a| FIRST[a.0 as usize].1)
            .unwrap_or(0);
        let youngest = adults.iter().map(|a| a.2).min().unwrap_or(30);
        let max_kid = (youngest.saturating_sub(19)).min(17) as f32;
        let baby_in_house = kids > 0 && rng.chance(0.25);
        let mut kid_ages: Vec<u16> = Vec::new();
        for i in 0..kids {
            let age = if i == 0 && baby_in_house {
                rng.range_f32(0.0, 1.9) as u16
            } else {
                rng.range_f32(0.0, max_kid + 0.99) as u16
            };
            kid_ages.push(age.min(17));
        }
        let has_baby = kid_ages.iter().any(|a| *a < 2);
        let mut made = 0usize;
        for (i, (first, last, age)) in adults.iter().enumerate() {
            let kind = kind_of(*age);
            let at_home = has_baby && i == mother;
            let (job, job_kind) = match kind {
                AgeGroup::Adult if !jobs.is_empty() && !at_home && rng.f32() < 0.86 => {
                    let j = &jobs[rng.usize(jobs.len())];
                    (Some(j.door), j.kind)
                }
                _ => (None, JobKind::None),
            };
            let spouse = if paired { Some(id0 + (1 - i) as u32) } else { None };
            let born = born_of(*age, rng);
            self.list.push(Person {
                first: *first,
                last: *last,
                age: *age,
                kind,
                home: Some(door),
                job,
                job_kind,
                spouse,
                parent: None,
                tone: rng.usize(TONES) as u8,
                jitter: rng.usize(12) as u8,
                born,
                life: Life::Alive,
            });
            made += 1;
        }
        for age in kid_ages {
            let kind = kind_of(age);
            let (job, job_kind) = match jobs.iter().find(|j| j.kind == JobKind::School) {
                Some(j) if has_school && age >= SCHOOL_AGE => (Some(j.door), JobKind::School),
                _ => (None, JobKind::None),
            };
            let born = born_of(age, rng);
            let female = rng.chance(0.5);
            let last = adults[mother.min(adults.len() - 1)].1;
            self.list.push(Person {
                first: pick_first(rng, female),
                last,
                age,
                kind,
                home: Some(door),
                job,
                job_kind,
                spouse: None,
                parent: Some(id0 + mother as u32),
                tone: rng.usize(TONES) as u8,
                jitter: rng.usize(12) as u8,
                born,
                life: Life::Alive,
            });
            made += 1;
        }
        if has_baby {
            self.refresh_walkers();
        }
        (made, surname)
    }

    /// Test/repair hook: give every person a job that matches their home town.
    /// (Only used by tests to build a dense roster quickly.)
    #[doc(hidden)]
    pub fn assign(&mut self, id: u32, home: (i32, i32), job: Option<(i32, i32)>, kind: JobKind) {
        if let Some(p) = self.list.get_mut(id as usize) {
            p.home = Some(home);
            p.job = job;
            p.job_kind = kind;
        }
    }

    /// Plan the commute corridors: a few routes from a home to a workplace that
    /// the commuters share. People are assigned to one by their id, so the
    /// morning rush reads as streams along the real streets instead of one A*
    /// per resident (thousands of searches would cost more than the city).
    fn plan_corridors(&mut self, w: &World) {
        let lanes = w.map.tiles.iter().filter(|t| t.kind.is_lane()).count();
        let finished = w.lots.iter().filter(|l| l.finished()).count();
        let sig = (lanes, finished, w.fields.len());
        if sig == self.signature && !self.corridors.is_empty() {
            return;
        }
        self.signature = sig;
        let homes: Vec<(i32, i32)> = self.list.iter().filter_map(|p| p.home).collect();
        let jobs: Vec<(i32, i32)> = self
            .list
            .iter()
            .filter(|p| matches!(p.job_kind, JobKind::Office | JobKind::Factory | JobKind::Shop | JobKind::School))
            .filter_map(|p| p.job)
            .collect();
        if homes.len() < 2 || jobs.is_empty() {
            self.corridors.clear();
            self.strolls.clear();
            return;
        }
        // Buildings are walls for a pedestrian (D45); a door is on the pavement.
        let mut blocked = std::collections::BTreeSet::new();
        for l in w.lots.iter() {
            for ty in l.y..l.y + l.h {
                for tx in l.x..l.x + l.w {
                    blocked.insert((tx, ty));
                }
            }
        }
        // The lane network, for snapping a commute onto the street (B43b).
        let lane_tiles: Vec<(i32, i32)> = if lanes >= 4 {
            (0..w.map.h)
                .flat_map(|y| (0..w.map.w).map(move |x| (x, y)))
                .filter(|&(x, y)| w.map.at(x, y).kind.is_lane())
                .collect()
        } else {
            Vec::new()
        };
        let mut out = Vec::with_capacity(32);
        for i in 0..32usize {
            let from = homes[(i * 17) % homes.len()];
            let to = jobs[(i * 11 + 3) % jobs.len()];
            if from == to {
                continue;
            }
            let path = commute_path(w, &blocked, &lane_tiles, from, to, w.seed ^ (i as u64));
            if let Some(path) = path {
                if path.len() >= 2 {
                    out.push(path);
                }
            }
        }
        self.corridors = out;
        // B75: the stroll routes — a door to a place people go without a
        // timetable (the park, the shop front, the church steps, the plaza).
        // They are what the ambient crowd walks, so a pavement between two
        // venues is never empty the way it was when everyone stood on the six
        // spots inside the park.
        let venues = w.venues();
        let mut strolls: Vec<Vec<(i32, i32)>> = Vec::with_capacity(48);
        if !venues.is_empty() && !homes.is_empty() {
            for i in 0..48usize {
                let from = homes[(i * 29 + 7) % homes.len()];
                let v = venues[(i * 5) % venues.len()];
                let to = (v.x as i32, v.y as i32);
                let path = commute_path(w, &blocked, &lane_tiles, from, to, w.seed ^ 0x5701 ^ (i as u64));
                if let Some(path) = path {
                    if path.len() >= 3 {
                        strolls.push(path);
                    }
                }
            }
        }
        self.strolls = strolls;
    }

    /// B75: the routes the ambient crowd may walk (strolls first, then the
    /// commute corridors), for a walker index.
    fn stroll_route(&self, i: usize) -> Option<&Vec<(i32, i32)>> {
        if !self.strolls.is_empty() {
            let pick = i % (self.strolls.len() * 2);
            if pick < self.strolls.len() {
                return self.strolls.get(pick);
            }
        }
        if !self.corridors.is_empty() {
            return self.corridors.get(i % self.corridors.len());
        }
        self.strolls.get(i % self.strolls.len().max(1))
    }
}

/// B75: how much of the population is *on the street* at this hour, as a
/// fraction of the living. This is the ambient half of the day — people not
/// commuting and not at a venue, but walking somewhere: the pavement is never
/// bare from breakfast to bedtime, and it doubles at the rush hours, thickens
/// again in the early evening and thins to a night crowd after eleven.
///
/// B77/D84: a Saturday and a Sunday have their own shapes. Saturday keeps its
/// shopkeepers and its market crowd all day and has no rush hour to speak of;
/// Sunday morning is the quietest street of the week and Sunday afternoon
/// belongs to the park.
pub fn street_rate(hour: f64, weekday: f64) -> f32 {
    if !(6.0..23.4).contains(&hour) {
        return 0.0; // the small hours belong to nobody (the brief's 3 am)
    }
    let dow = weekday as i32;
    if dow == 6 {
        // Saturday: a late start, then the shopping street all day.
        return if (6.0..8.0).contains(&hour) {
            0.10
        } else if (8.0..10.0).contains(&hour) {
            0.30
        } else if (10.0..17.0).contains(&hour) {
            0.44
        } else if (17.0..19.0).contains(&hour) {
            0.40
        } else if (19.0..22.0).contains(&hour) {
            0.34
        } else {
            0.12
        };
    }
    if dow == 0 {
        // Sunday: nobody is going anywhere early, the church and the park have
        // the day, the evening is quiet.
        return if (6.0..9.0).contains(&hour) {
            0.09
        } else if (9.0..11.5).contains(&hour) {
            0.26
        } else if (11.5..13.0).contains(&hour) {
            0.20
        } else if (13.0..17.5).contains(&hour) {
            0.36
        } else if (17.5..21.0).contains(&hour) {
            0.26
        } else {
            0.09
        };
    }
    let base: f32 = if (7.0..9.0).contains(&hour) || (16.7..18.6).contains(&hour) {
        0.50 // the two rush hours
    } else if (9.0..12.0).contains(&hour) || (13.0..16.7).contains(&hour) {
        0.24 // the working day: shoppers, parents, the retired
    } else if (12.0..13.0).contains(&hour) {
        0.30 // lunch: out to the shop and back
    } else if (18.6..21.6).contains(&hour) {
        0.34 // the evening stroll
    } else if (21.6..23.4).contains(&hour) {
        0.11 // the night crowd, on the way home
    } else {
        0.08 // the early risers
    };
    base
}

/// B75: the tone a resident is drawn in. An elder wears one of the three muted
/// coats and grey hair (the renderer's tones 15..17), so age reads at 1× and a
/// figure can be followed down a street.
pub fn tone_of(p: &Person) -> u8 {
    if p.kind == AgeGroup::Elder {
        15 + (p.tone % 3)
    } else {
        p.tone
    }
}

impl People {
    /// Everyone visible at `hour` (0..24) on this city: residents following
    /// their schedule, construction crews, farmers, shopkeepers, and the
    /// social life of the parks.
    pub fn view(&self, w: &World, hour: f64, time: f64, weekday: f64) -> Vec<Figure> {
        let mut out = Vec::with_capacity(512);
        self.view_into(w, hour, time, weekday, &mut out);
        out
    }

    /// The same, into a caller-owned vector: the wallpaper calls this every
    /// display frame (D71), so it must not allocate once the buffer is warm.
    /// `out` is cleared first; nothing else is freed.
    pub fn view_into(
        &self,
        w: &World,
        hour: f64,
        time: f64,
        weekday: f64,
        out: &mut Vec<Figure>,
    ) {
        out.clear();
        let weekend = weekday == 0.0 || weekday == 6.0;
        // Night: the streets are empty (the brief's 3 am).
        let night = !(5.9..22.6).contains(&hour);

        // 1. Construction sites: a crew at every lot being built (D33/D65),
        //    on site hours: 07:00-18:00 on weekdays, a half day on Saturday.
        if site_hours(hour, weekday) {
            for l in w.lots.iter() {
                if l.finished() || l.stage < Stage::Foundation {
                    continue;
                }
                // A site reads as busy from across the street: three or four
                // hands, one of them up on the scaffold (B43b item 5).
                let n = ((l.w * l.h) / 3).clamp(3, 5) as usize;
                for k in 0..n {
                    let (mut x, mut y, pose, prop, lift) = builder_pose(l, k, time);
                    // B75: a hand never stands in the carriageway. In a dense
                    // block the verge outside the wall is the road, so step
                    // inside the plot instead.
                    if w.map.at(x.floor() as i32, y.floor() as i32).kind.is_lane() {
                        if x > l.x as f32 + l.w as f32 {
                            x = l.x as f32 + l.w as f32 - 0.45;
                        } else {
                            y = l.y as f32 + l.h as f32 - 0.45;
                        }
                    }
                    out.push(Figure {
                        x,
                        y,
                        pose,
                        // B43c: hi-vis vest and hard hat (the renderer's tones 8, 9).
                        tone: 8 + ((l.seed as usize + k) % 2) as u8,
                        child: false,
                        prop,
                        dir: l.front,
                        open: true,
                        lift,
                    });
                }
            }
        }

        // 2. Farms: hands in the fields from first light to dusk, a break at
        //    noon (D38, D65).
        if !night {
            let farming = (6.2..18.2).contains(&hour);
            if farming || (18.2..19.5).contains(&hour) {
                for (fi, f) in w.fields.iter().enumerate() {
                    // B80: hands work the crops, not the pasture, and only every
                    // other field has someone in it (a countryside of many farms
                    // must not be a crowd).
                    if !map::crop_of(&w.map.at(f.x, f.y)).is_arable() || fi % 2 == 1 {
                        continue;
                    }
                    let n = ((f.w * f.h) / 24).clamp(1, 2) as usize;
                    for k in 0..n {
                        let (x, y) = field_spot(f, fi, k, w.seed);
                        let lunch = (11.9..12.8).contains(&hour);
                        out.push(Figure {
                            x,
                            y,
                            pose: if lunch { Pose::Sit } else { Pose::Work },
                            tone: ((fi * 5 + k * 3) % TONES) as u8,
                            child: false,
                            prop: Prop::None,
                            dir: 1,
                            open: false,
                            lift: 0,
                        });
                    }
                }
            }
        }

        // 3. Shopkeepers at their doors (open 08:00-18:30).
        if (8.0..18.5).contains(&hour) {
            for l in w.lots.iter() {
                if !l.finished() || !matches!(l.kind, LotKind::Shop | LotKind::Store) {
                    continue;
                }
                let (dx, dy) = door_tile(l);
                out.push(Figure {
                    x: dx as f32 + 0.5,
                    y: dy as f32 + 0.4,
                    pose: if (l.seed as u64).wrapping_mul(7) % 5 == 0 {
                        Pose::Wave
                    } else {
                        Pose::Stand
                    },
                    tone: (l.seed as usize % TONES) as u8,
                    child: false,
                    prop: Prop::None,
                    dir: l.front,
                    open: false,
                    lift: 0,
                });
            }
        }

        // 4. Shift change: people streaming in and out of offices and works.
        if !night {
            let shift = (8.3..8.9).contains(&hour) || (17.1..17.7).contains(&hour);
            if shift {
                for l in w.lots.iter() {
                    if !l.finished() {
                        continue;
                    }
                    if !matches!(l.kind, LotKind::Office | LotKind::Tower | LotKind::Industry) {
                        continue;
                    }
                    let (dx, dy) = door_tile(l);
                    let n = (l.storeys as usize / 3).clamp(1, 4);
                    for k in 0..n {
                        let s = ((l.seed as usize + k * 13) % 9) as f32 / 9.0 - 0.5;
                        out.push(Figure {
                            x: dx as f32 + 0.5 + s,
                            y: dy as f32 + 0.5 + s * 0.5,
                            pose: Pose::Walk,
                            tone: ((l.seed as usize + k) % TONES) as u8,
                            child: false,
                            prop: Prop::None,
                            dir: if hour < 12.0 { l.front.wrapping_add(2) % 4 } else { l.front },
                            open: false,
                            lift: 0,
                        });
                    }
                }
            }
        }

        // 4b. Breaks: a few of the office and works staff stand outside with a
        //     cup at mid-morning and mid-afternoon (D65: who is outside doing
        //     what should be legible at a glance).
        if !night {
            let break_t = (10.0..10.5).contains(&hour) || (14.7..15.2).contains(&hour);
            if break_t {
                for l in w.lots.iter() {
                    if !l.finished()
                        || !matches!(l.kind, LotKind::Office | LotKind::Tower | LotKind::Industry)
                    {
                        continue;
                    }
                    let (dx, dy) = door_tile(l);
                    let n = (l.storeys as usize / 4).clamp(1, 3);
                    for k in 0..n {
                        let s = ((l.seed as usize + k * 5) % 7) as f32 / 7.0 - 0.5;
                        out.push(Figure {
                            x: dx as f32 + 0.6 + s,
                            y: dy as f32 + 0.3 - s * 0.4,
                            pose: if k == 0 { Pose::Chat } else { Pose::Stand },
                            tone: ((l.seed as usize + k * 3) % TONES) as u8,
                            child: false,
                            prop: Prop::None,
                            dir: 1,
                            open: false,
                            lift: 0,
                        });
                    }
                }
            }
        }

        // 5. The gathering places: parks and plazas of course, but also the
        //    shop fronts and the church steps — a village with no park yet
        //    still meets somewhere (B43b: nobody socialised at 19:00).
        // A city has more shop fronts and parks than a frame can show: the
        // sixteen busiest places keep the crowds readable and the tick cheap
        // (the venue list is ordered parks first, then civic doors).
        let mut venues = w.venues();
        venues.truncate(16);
        // Safe standing spots for each venue, worked out once: `dry` and
        // `off_road` search rings, and doing that for every visitor at every
        // frame cost 2.6 ms a tick at 19:00 (B43b measurement).
        let spots: Vec<Vec<(f32, f32)>> = venues
            .iter()
            .enumerate()
            .map(|(vi, v)| {
                // B75: more standing room per venue. Fourteen spots put a
                // hundred visitors from `Leisure` on six tiles; the crowd read
                // as one pixel of coats.
                let n = if v.park { 26 } else { 8 };
                (0..n)
                    .map(|k| {
                        off_road(
                            w,
                            dry(w, venue_spot(v, vi, k, w.seed), (v.x, v.y)),
                        )
                    })
                    .collect()
            })
            .collect();
        if (9.0..21.0).contains(&hour) {
            for (vi, v) in venues.iter().enumerate() {
                // A park holds several groups; a doorway holds one.
                let groups = if v.park {
                    // B75: a park on a summer Sunday holds several parties.
                    ((v.span * v.span) / 3.0).clamp(4.0, 10.0) as usize
                } else {
                    3
                };
                for g in 0..groups {
                    for m in 0..3 {
                        let (px, py) = spots[vi][(g * 3 + m) % spots[vi].len()];
                        let (sx, sy) = kerb_shift(v, g * 3 + m);
                        out.push(Figure {
                            x: px + sx,
                            y: py + sy,
                            pose: if m == 0 && g == 0 && v.park {
                                Pose::Sit
                            } else {
                                Pose::Chat
                            },
                            tone: ((vi * 3 + g * 5 + m) % TONES) as u8,
                            child: false,
                            prop: Prop::None,
                            dir: (m % 4) as u8,
                            open: v.open,
                            lift: 0,
                        });
                    }
                    if g == 0 && v.park {
                        for m in 0..2 {
                            let (px, py) = spots[vi][(12 + m) % spots[vi].len()];
                            out.push(Figure {
                                x: px,
                                y: py,
                                pose: Pose::Play,
                                tone: ((vi + m * 3 + 1) % TONES) as u8,
                                child: true,
                                prop: Prop::Ball,
                                dir: (m % 4) as u8,
                                open: v.open,
                                lift: 0,
                            });
                        }
                    }
                }
                if v.park && (18.0..21.0).contains(&hour) {
                    let (gx, gy) = spots[vi][(18) % spots[vi].len()];
                    out.push(Figure {
                        x: gx,
                        y: gy,
                        pose: Pose::Walk,
                        tone: ((vi + 5) % TONES) as u8,
                        child: false,
                        prop: Prop::Dog,
                        dir: 0,
                        open: v.open,
                        lift: 0,
                    });
                }
            }
        }

        // 6. The residents themselves, following the schedule.
        for (id, p) in self.list.iter().enumerate() {
            // The dead and the departed draw nothing; a baby is indoors or in
            // the pram (B48).
            if !p.alive() || p.age < 3 {
                continue;
            }
            let Some(home) = p.home else { continue };
            let j = p.jitter as f32 / 12.0;
            // Each commuter leaves at their own minute, so a corridor holds a
            // spread of people walking at different points rather than one
            // crowd marching in lockstep — and each trade keeps its own hours
            // (B46c: a shopkeeper closes when their shop does).
            let phase = ((id as u64).wrapping_mul(2_654_435_761) >> 9) as f32 % 100.0 / 100.0;
            let act = schedule_at(p.kind, p.job_kind, hour, weekend, phase);
            match act {
                Act::Asleep | Act::Work | Act::School => {}
                Act::Commute { to_work } => {
                    let to = if to_work { p.job } else { Some(home) };
                    let Some(to) = to else { continue };
                    if to_work && p.job.is_none() {
                        continue;
                    }
                    let from = if to_work { home } else { p.job.unwrap_or(home) };
                    if self.corridors.is_empty() || id % 23 == 0 {
                        // No corridors (a young village): walk the open ground
                        // in a straight line, which at this scale reads fine.
                        let f = commute_fraction(hour, to_work, p.kind, p.job_kind, phase);
                        if f <= 0.004 || f >= 0.996 {
                            // Still inside (coffee) or already at the desk: not
                            // on the street, so not drawn.
                            continue;
                        }
                        let (x, y) = lerp_tile(from, to, f);
                        out.push(Figure {
                            x,
                            y,
                            pose: Pose::Walk,
                            tone: tone_of(p),
                            child: p.kind == AgeGroup::Child,
                            prop: Prop::None,
                            dir: dir_towards(from, to),
                            open: false,
                            lift: 0,
                        });
                    } else {
                        let c = &self.corridors[(id * 13 + j as usize) % self.corridors.len()];
                        let f = commute_fraction(hour, to_work, p.kind, p.job_kind, phase);
                        if f <= 0.004 || f >= 0.996 {
                            continue;
                        }
                        let (x, y) = along(c, if to_work { f } else { 1.0 - f }, j);
                        out.push(Figure {
                            x,
                            y,
                            pose: Pose::Walk,
                            tone: tone_of(p),
                            child: p.kind == AgeGroup::Child,
                            prop: if p.prop_dog(id as u32) { Prop::Dog } else { Prop::None },
                            dir: dir_along(c, if to_work { f } else { 1.0 - f }),
                            open: false,
                            lift: 0,
                        });
                    }
                }
                Act::Home => {
                    // Out of doors in the evening only; asleep and inside at
                    // night. One in twelve waves from the door.
                    let morning = (6.0..7.4).contains(&hour);
                    // Early risers, not the whole street at their doors.
                    // B75: the evening porch is for the few who like to sit
                    // out; a whole town standing on its steps was the other
                    // half of why the streets looked empty.
                    if !(17.5..22.0).contains(&hour) && !(morning && (id % 10) < 3) {
                        continue;
                    }
                    if !morning && id % 4 != 0 {
                        continue;
                    }
                    {
                        let (x, y) = home_spot(home, p, j);
                        let dog = p.prop_dog(id as u32) && (18.0..21.5).contains(&hour);
                        out.push(Figure {
                            x,
                            y,
                            pose: if dog {
                                Pose::Walk
                            } else if (id as u64).wrapping_mul(31) % 12 == 0 {
                                Pose::Wave
                            } else {
                                Pose::Stand
                            },
                            tone: tone_of(p),
                            child: p.kind == AgeGroup::Child,
                            prop: if dog { Prop::Dog } else { Prop::None },
                            dir: 1,
                            open: false,
                            lift: 0,
                        });
                    }
                }
                Act::Lunch => {
                    // B75: a third of the lunch crowd walks out to eat instead
                    // of sitting at the firm's door; the ambient layer below
                    // puts them on the pavement (the same third, so nobody is
                    // drawn twice).
                    if id % 3 == 0 {
                        continue;
                    }
                    if let Some(job) = p.job {
                        let (dx, dy) = doorish(w, job, p);
                        out.push(Figure {
                            x: dx,
                            y: dy,
                            pose: if (id as u64) % 3 == 0 { Pose::Chat } else { Pose::Sit },
                            tone: tone_of(p),
                            child: p.kind == AgeGroup::Child,
                            prop: Prop::None,
                            dir: 1,
                            open: false,
                            lift: 0,
                        });
                    }
                }
                Act::Leisure => {
                    // Where people go: the park, the shop front, the church
                    // steps, else the porch. Not everyone goes out — otherwise
                    // a park is one solid crowd — but somebody always does.
                    // B75: not everyone who is free goes to the same park. A
                    // third of the adults gather; the rest walk (the ambient
                    // layer picks them up) or stay in.
                    let goes_out = if p.kind == AgeGroup::Child {
                        id % 5 < 4
                    } else if weekend {
                        // B75: the weekend is what a park is for.
                        (id * 31) % 100 < 55
                    } else {
                        (id * 31) % 100 < 35
                    };
                    let (x, y, pose, prop, open) = if goes_out && !venues.is_empty() {
                        let vi = (id * 7) % venues.len();
                        let v = venues[vi];
                        // A handful of clusters per venue, not one person per
                        // spot: people gather in groups, they do not space
                        // themselves evenly like trees.
                        let sp = &spots[vi];
                        // Everyone picks their own spot from the whole set, so
                        // a park is dotted with people, not six blobs (B75).
                        let k = (id * 7 + p.jitter as usize) % sp.len().max(1);
                        let (px, py) = sp[k];
                        let (sx, sy) = kerb_shift(&v, id % 11);
                        (
                            px + sx,
                            py + sy,
                            if p.kind == AgeGroup::Child { Pose::Play } else { Pose::Chat },
                            if p.kind == AgeGroup::Child {
                                Prop::Ball
                            } else if p.prop_dog(id as u32) {
                                Prop::Dog
                            } else {
                                Prop::None
                            },
                            v.open,
                        )
                    } else if p.kind != AgeGroup::Child {
                        // Out for a walk: the ambient layer below picks them up
                        // (B75), so an afternoon street has movement in it.
                        continue;
                    } else {
                        let (x, y) = home_spot(home, p, j);
                        (
                            x,
                            y,
                            Pose::Play,
                            Prop::Ball,
                            false,
                        )
                    };
                    out.push(Figure {
                        x,
                        y,
                        pose,
                        tone: tone_of(p),
                        child: p.kind == AgeGroup::Child,
                        prop,
                        dir: (id % 4) as u8,
                        open,
                        lift: 0,
                    });
                }
            }
        }

        // 7. The street itself (B75/D83): the ambient crowd. Not everyone who
        //    is out is commuting or sitting in a park — a living street carries
        //    people walking somewhere all day, and that is what made the frames
        //    read as empty: outside the two rush windows the walk pose count was
        //    zero. Each walker has a real route (a pavement from a door to a
        //    place people go), a real identity (their own coat, an elder's grey)
        //    and a position that advances with the clock, so the same person can
        //    be followed along the pavement.
        let out_rate = street_rate(hour, weekday);
        if out_rate > 0.0 {
            let target = ((self.living as f32) * out_rate) as usize;
            // Count the people already walking (the commuters, the dog
            // walkers): the ambient layer tops the *moving* crowd up to the
            // day's target instead of piling onto a crowd standing still.
            let mut walking = out.iter().filter(|f| f.pose == Pose::Walk).count();
            if walking < target {
                let need = target - walking;
                let stride = (self.living / need.max(1)).max(1);
                for (id, p) in self.list.iter().enumerate().step_by(stride) {
                    if walking >= target {
                        break;
                    }
                    if p.home.is_none() || !p.alive() || p.age < 3 {
                        continue;
                    }
                    // Children are in bed by nine; nobody strolls at noon on a
                    // school day (B75: kids only by day).
                    if p.kind == AgeGroup::Child && !(7.0..19.5).contains(&hour) {
                        continue;
                    }
                    let phase = ((id as u64).wrapping_mul(2_654_435_761) >> 9) as f32 % 100.0 / 100.0;
                    let act = schedule_at(p.kind, p.job_kind, hour, weekend, phase);
                    let strolling = match act {
                        Act::Home | Act::Work | Act::School => true,
                        // The third of the lunch crowd that walks out to eat.
                        Act::Lunch => id % 3 == 0,
                        // The adults who are not gathering in a park.
                        Act::Leisure => {
                            p.kind != AgeGroup::Child && (id * 31) % 100 >= 35
                        }
                        _ => false,
                    };
                    if !strolling {
                        continue; // already on the street or at the venue
                    }
                    let Some(route) = self.stroll_route(id * 31 + 7) else { continue };
                    let n = route.len();
                    if n < 2 {
                        continue;
                    }
                    // A stroll is slower than a commute, and each person keeps
                    // their own pace and starting point.
                    let speed = 0.055 + (p.tone % 4) as f32 * 0.006;
                    let leg = time as f32 * speed * 2.0 / n as f32 + phase * 7.0;
                    let back = (leg.floor() as i64) % 2 != 0;
                    let raw = leg.fract();
                    let f = if back { 1.0 - raw } else { raw };
                    let j = p.jitter as f32 / 12.0;
                    let (x, y) = along(route, f, j);
                    let dir = dir_along(route, f);
                    out.push(Figure {
                        x,
                        y,
                        pose: Pose::Walk,
                        tone: tone_of(p),
                        child: p.kind == AgeGroup::Child,
                        prop: if p.prop_dog(id as u32) && (6.5..21.0).contains(&hour) {
                            Prop::Dog
                        } else if id % 37 == 0 {
                            Prop::Bag
                        } else {
                            Prop::None
                        },
                        dir,
                        open: false,
                        lift: 0,
                    });
                    walking += 1;
                }
            }
        }

        // 8. Prams (B48): a mother with a baby under two walks the pavement
        //    outside her door in the mid-morning and the afternoon.
        let outing = (9.6..11.8).contains(&hour) || (15.0..17.2).contains(&hour);
        if outing && !self.walkers.is_empty() {
            for &id in &self.walkers {
                let Some(p) = self.list.get(id as usize) else { continue };
                let Some(home) = p.home else { continue };
                let (x, y, dir) = stroll(w, home, time, id);
                out.push(Figure {
                    x,
                    y,
                    pose: Pose::Walk,
                    tone: tone_of(p),
                    child: false,
                    prop: Prop::Stroller,
                    dir,
                    open: false,
                    lift: 0,
                });
            }
        }

        // 9. The day's gatherings: a wedding, a funeral, a van at a door.
        let n0 = out.len();
        self.scene_figures(time, out);
        // B64: a crowd on a lawn square must not stand in its fountain basin.
        for f in out[n0..].iter_mut() {
            if w.in_park_water(f.x, f.y) {
                let (x, y) = dry(w, (f.x, f.y), (f.x, f.y));
                f.x = x;
                f.y = y;
            }
        }
        // 10. Pavements (B75/D83): a person who is standing, sitting or
        //     chatting belongs on the pavement, not in the carriageway. A door
        //     that opens straight onto a road used to put the whole evening on
        //     the tarmac; step them to the kerb.
        for f in out.iter_mut() {
            if f.pose == Pose::Walk {
                continue;
            }
            if w.map.at(f.x.floor() as i32, f.y.floor() as i32).kind.is_lane() {
                let (x, y) = off_road(w, (f.x, f.y));
                f.x = x;
                f.y = y;
            }
        }
        // 11. Rain (B75): everyone outdoors under an umbrella — except the
        //     hands, who need both, and the people already carrying something.
        if self.wet {
            for f in out.iter_mut() {
                if matches!(f.prop, Prop::None | Prop::Dog) && f.pose != Pose::Sit {
                    f.prop = Prop::Umbrella;
                }
            }
        }
    }
}

/// Where a pram is while it walks the pavement beside a door: back and forth
/// along the stretch of pavement (or lane) the door opens onto, slowly.
fn stroll(w: &World, home: (i32, i32), time: f64, id: u32) -> (f32, f32, u8) {
    let walk = |x: i32, y: i32| {
        w.map.in_bounds(x, y) && (w.map.at(x, y).kind == Kind::Sidewalk || w.map.at(x, y).kind.is_lane())
    };
    let mut best = (0i32, 0usize);
    let dirs = [(1, 0), (-1, 0), (0, 1), (0, -1)];
    for (i, (dx, dy)) in dirs.iter().enumerate() {
        let mut n = 0;
        while n < 6 && walk(home.0 + dx * (n + 1), home.1 + dy * (n + 1)) {
            n += 1;
        }
        if n > best.0 {
            best = (n, i);
        }
    }
    let (dx, dy) = dirs[best.1];
    let span = best.0 as f32;
    if span < 2.0 {
        return (home.0 as f32 + 0.5, home.1 as f32 + 0.5, 1);
    }
    let q = (time as f32 * 0.05 * WALK_TILES_S / span.max(1.0) * 5.0 + id as f32 * 0.37).rem_euclid(2.0);
    let (f, fwd) = if q < 1.0 { (q, true) } else { (2.0 - q, false) };
    let d = if fwd { best.1 } else { best.1 ^ 1 };
    (
        home.0 as f32 + 0.5 + dx as f32 * f * span,
        home.1 as f32 + 0.5 + dy as f32 * f * span,
        [0u8, 2, 1, 3][d],
    )
}

impl Person {
    /// Does this person walk a dog? A stable third of dog-owning adults.
    fn prop_dog(&self, id: u32) -> bool {
        self.kind == AgeGroup::Adult
            && self.job_kind != JobKind::Farm
            && ((id as u64).wrapping_mul(2_654_435_761) >> 8) % 9 == 0
    }
}

// --- geometry helpers -------------------------------------------------

/// A commute from a door to a door (B43b). With a street network, the walk is
/// routed between the *nearest lane tiles* and only the doorstep is grass; in a
/// settler village with no lanes yet, the direct walk is the honest picture.
fn commute_path(
    w: &World,
    blocked: &std::collections::BTreeSet<(i32, i32)>,
    lanes: &[(i32, i32)],
    from: (i32, i32),
    to: (i32, i32),
    seed: u64,
) -> Option<Vec<(i32, i32)>> {
    if lanes.is_empty() {
        return crate::grid::route_walk(w.map, from, to, seed, blocked);
    }
    let near = |p: (i32, i32), max: i32| -> Option<(i32, i32)> {
        lanes
            .iter()
            .copied()
            .filter(|l| (l.0 - p.0).abs() + (l.1 - p.1).abs() <= max)
            .min_by_key(|l| {
                let (dx, dy) = (l.0 - p.0, l.1 - p.1);
                dx * dx + dy * dy
            })
    };
    let (Some(a), Some(b)) = (near(from, 10), near(to, 10)) else {
        // The street does not reach here: an honest walk across the ground.
        return crate::grid::route_walk(w.map, from, to, seed, blocked);
    };
    let mid = crate::grid::route_walk_streets(w.map, a, b, seed, blocked)?;
    let mut path = Vec::with_capacity(mid.len() + 2);
    if from != a {
        path.push(from);
    }
    path.extend(mid);
    if to != b {
        path.push(to);
    }
    Some(path)
}

/// Where the door on a plot's street side is: the middle of the frontage, one
/// tile out from the wall.
pub fn door_tile(l: &Lot) -> (i32, i32) {
    match l.front % 4 {
        0 => (l.x + l.w, l.y + l.h / 2),
        1 => (l.x + l.w / 2, l.y + l.h),
        2 => (l.x - 1, l.y + l.h / 2),
        _ => (l.x + l.w / 2, l.y - 1),
    }
}

/// A builder on a construction site: two hammering on the frame, one carrying
/// a plank back and forth along the frontage (`time` drives the walk so a crew
/// visibly works instead of standing frozen).
/// Building-site working hours (B43c): 07:00-18:00 Monday to Friday, 07:00-13:00
/// on Saturday, nobody on Sunday. `weekday` is 0 = Sunday.
pub fn site_hours(hour: f64, weekday: f64) -> bool {
    match weekday as i32 {
        0 => false,
        6 => (7.0..13.0).contains(&hour),
        _ => (7.0..18.0).contains(&hour),
    }
}

fn builder_pose(l: &Lot, k: usize, time: f64) -> (f32, f32, Pose, Prop, u8) {    // B75/D83: the crew stands on the two faces the camera can see (the +x and
    // +y sides of the plot), never behind the wall. A site whose hands were
    // hidden on the far side read as an empty site.
    let fx = l.x as f32 + l.w as f32 + 0.45; // just outside the +x wall
    let fy = l.y as f32 + l.h as f32 + 0.45; // just outside the +y wall
    match k {
        // On the +x face, hammering at the frame.
        0 => (fx, l.y as f32 + l.h as f32 * 0.45, Pose::Work, Prop::None, 0),
        // The carrier: a triangle wave along the +y frontage, a plank across
        // the chest. `time` drives the walk so the crew visibly works.
        1 => {
            let span = l.w.max(2) as f32;
            let t = ((time as f32 * 0.12) + 0.5).rem_euclid(2.0);
            let f = if t < 1.0 { t } else { 2.0 - t };
            (
                l.x as f32 + 0.5 + f * (span - 1.0),
                fy,
                Pose::Carry,
                Prop::Lumber,
                0,
            )
        }
        // Wheeling bricks along the other visible face.
        2 => (fx, l.y as f32 + l.h as f32 * 0.85, Pose::Work, Prop::Barrow, 0),
        // Up on the scaffold: the figure stands on the frame, six pixels up
        // (B43b item 5), hammering at the top plate.
        3 => (
            fx,
            l.y as f32 + l.h as f32 * 0.15,
            Pose::Work,
            Prop::None,
            6,
        ),
        // A fourth hand on the +y face, walking the site.
        _ => {
            let span = l.h.max(2) as f32;
            let t = ((time as f32 * 0.10) + 1.2).rem_euclid(2.0);
            let f = if t < 1.0 { t } else { 2.0 - t };
            (
                fx,
                l.y as f32 + 0.5 + f * (span - 1.0),
                Pose::Walk,
                Prop::None,
                0,
            )
        }
    }
}

fn field_spot(f: &Field, fi: usize, k: usize, seed: u64) -> (f32, f32) {
    let n = ((f.w * f.h) / 24).clamp(1, 4);
    let _ = n;
    let h = map::rng::hash2(f.x + k as i32 * 3, f.y + fi as i32 * 5, seed ^ 0xF13);
    let h2 = map::rng::hash2(f.x - k as i32, f.y + 7, seed ^ 0x9A1);
    let x = f.x as f32 + 0.6 + h * (f.w as f32 - 1.2).max(0.2);
    let y = f.y as f32 + 0.6 + h2 * (f.h as f32 - 1.2).max(0.2);
    (x, y)
}

impl World<'_> {
    /// B64: is this tile-space point in a park's pond or a fountain basin (the
    /// lawn square's, or the civic square's)?
    pub fn in_park_water(&self, x: f32, y: f32) -> bool {
        let (tx, ty) = (x.floor() as i32, y.floor() as i32);
        self.lots.iter().any(|l| {
            let civic = l.kind == LotKind::Landmark(crate::landmarks::LandmarkKind::Plaza);
            (l.kind == LotKind::Park || civic)
                && tx >= l.x - 1
                && tx <= l.x + l.w
                && ty >= l.y - 1
                && ty <= l.y + l.h
                && map::in_park_water(
                    l.w,
                    l.h,
                    civic,
                    map::ParkKind::of_style(l.style),
                    x - l.x as f32,
                    y - l.y as f32,
                )
        })
    }
}

/// Nudge a spot out of the water (a park's pond, a river bend), towards the
/// centre of the open space it belongs to.
fn dry(w: &World, (x, y): (f32, f32), _centre: (f32, f32)) -> (f32, f32) {
    let wet = |x: f32, y: f32| {
        if w.map.at(x.floor() as i32, y.floor() as i32).is_water() {
            return true;
        }
        // A park's pond and fountain basins are drawn by the park sprite, not
        // water tiles (B43b, B64): the same `map::park_water` geometry the art
        // uses keeps people out.
        w.in_park_water(x, y)
    };
    if !wet(x, y) {
        return (x, y);
    }
    // The nearest dry ground, searching outwards in rings.
    for r in 1..=8i32 {
        for dy in -r..=r {
            for dx in -r..=r {
                if dx.abs() != r && dy.abs() != r {
                    continue;
                }
                // Keep the sub-tile offset: snapping to the tile centre made
                // every relocated spot land on the same pixel (B75).
                let (nx, ny) = (
                    x.floor() + dx as f32 + x.fract(),
                    y.floor() + dy as f32 + y.fract(),
                );
                if !wet(nx, ny) {
                    return (nx, ny);
                }
            }
        }
    }
    (x, y)
}

/// Off the carriageway (B43b cosmetic): a crowd stands on the pavement or the
/// verge, not in the middle of the road. Searches the nearest non-lane tile.
fn off_road(w: &World, (x, y): (f32, f32)) -> (f32, f32) {
    let on_road =
        |x: f32, y: f32| w.map.at(x.floor() as i32, y.floor() as i32).kind.is_lane();
    if !on_road(x, y) {
        return (x, y);
    }
    for r in 1..=3i32 {
        for dy in -r..=r {
            for dx in -r..=r {
                if dx.abs() != r && dy.abs() != r {
                    continue;
                }
                let (nx, ny) = (
                    x.floor() + dx as f32 + x.fract(),
                    y.floor() + dy as f32 + y.fract(),
                );
                if !on_road(nx, ny) {
                    return (nx, ny);
                }
            }
        }
    }
    (x, y)
}

/// B75: a sub-tile offset for a doorway crowd, so twelve people outside one
/// shop stand in a little group instead of on one pixel. Only for doorways: a
/// park's spots are already spread over its lawn (and one of them may be beside
/// the pond, where a nudge could put a figure in the water).
fn kerb_shift(v: &Venue, k: usize) -> (f32, f32) {
    if v.park {
        return (0.0, 0.0);
    }
    // The golden angle never repeats a direction, so a dozen visitors ring the
    // door instead of piling onto its step.
    let a = k as f32 * 2.3999632;
    let r = 0.55 + (k % 4) as f32 * 0.34;
    (a.cos() * r, a.sin() * r * 0.6)
}

/// A point in a gathering place, spread by venue index and group.
fn venue_spot(v: &Venue, vi: usize, g: usize, seed: u64) -> (f32, f32) {
    let a = (vi as f32 * 1.3) + (g as f32 * 0.7);
    let r = v.span
        * (0.30 + 0.6 * map::rng::hash2(vi as i32 + g as i32, g as i32, seed ^ 0x5EA));
    (v.x + a.cos() * r, v.y + a.sin() * r * 0.7)
}

fn plaza_spot(p: &Lot, pi: usize, g: usize, seed: u64) -> (f32, f32) {
    let cx = p.x as f32 + p.w as f32 * 0.5;
    let cy = p.y as f32 + p.h as f32 * 0.5;
    let a = (pi as f32 * 1.3) + (g as f32 * 0.7);
    let r = 0.22 + (map::rng::hash2(pi as i32 + g as i32, g as i32, seed ^ 0x5EA) * 0.22);
    let rr = r * p.w.min(p.h) as f32;
    (cx + a.cos() * rr, cy + a.sin() * rr * 0.7)
}

/// A resident's place about their own door: at the step, on the path, or in
/// the bit of yard beside it.
fn home_spot(home: (i32, i32), p: &Person, j: f32) -> (f32, f32) {
    // B75: a household of five used to stand on one doorstep pixel. The spot
    // is a small ring of positions in the yard, picked by the person's own
    // jitter, so a porch reads as a family and not as one figure.
    let a = (p.jitter as f32 / 12.0) * std::f32::consts::TAU;
    let r = 0.35 + (p.tone % 3) as f32 * 0.18;
    let dx = (j - 0.5) * 0.9 + a.cos() * r;
    let dy = ((p.jitter as f32 * 0.37).fract() - 0.5) * 0.5 + a.sin() * r * 0.5;
    (home.0 as f32 + 0.5 + dx, home.1 as f32 + 0.5 + dy)
}

/// At a workplace door, spread a little.
fn doorish(w: &World, job: (i32, i32), p: &Person) -> (f32, f32) {
    if p.job_kind == JobKind::Farm {
        let f = w.fields.first();
        if let Some(f) = f {
            let (x, y) = field_spot(f, 0, (p.tone % 3) as usize, w.seed);
            return (x, y);
        }
    }
    // B75: a lunch crowd is spread *along the frontage*, on the pavement, one
    // or two tiles either side of the door — not stacked on one tile.
    let base = (job.0 as f32 + 0.5, job.1 as f32 + 0.45);
    let j = (p.jitter as f32 - 6.0) * 0.34;
    let cand = if p.jitter % 2 == 0 {
        (base.0 + j, base.1 + 0.14)
    } else {
        (base.0 + 0.14, base.1 + j * 0.7)
    };
    let t = w.map.at(cand.0.floor() as i32, cand.1.floor() as i32);
    if t.kind.is_lane() || t.is_water() {
        base
    } else {
        cand
    }
}

fn lerp_tile(a: (i32, i32), b: (i32, i32), f: f32) -> (f32, f32) {
    let f = f.clamp(0.0, 1.0);
    (
        a.0 as f32 + (b.0 - a.0) as f32 * f + 0.5,
        a.1 as f32 + (b.1 - a.1) as f32 * f + 0.5,
    )
}

/// Where along the commute a person is at `hour`, 0..1. Departure is staggered
/// so the morning brings a rush rather than a single burst.
fn commute_fraction(
    hour: f64,
    to_work: bool,
    kind: AgeGroup,
    job: JobKind,
    phase: f32,
) -> f32 {
    // A walk takes about 35 sim minutes; `phase` (0..1, one per person) moves
    // the departure across the window, so at any moment the corridor holds
    // walkers from the doorstep to the office door.
    let dur = 0.58;
    // A shopkeeper walks on their own shop's hours (B46c), which is why the
    // window is staggered by the same phase the schedule uses.
    let (base, window) = if job == JobKind::Shop {
        let (open, close) = shop_hours(phase);
        if to_work {
            (open, 0.0)
        } else {
            (close, 0.0)
        }
    } else if to_work {
        if kind == AgeGroup::Child {
            (7.2, 0.45)
        } else {
            (6.9, 0.9)
        }
    } else if kind == AgeGroup::Child {
        (15.2, 0.4)
    } else {
        (17.0, 0.9)
    };
    let dep = base + window * phase as f64;
    ((hour - dep) / dur).clamp(0.0, 1.0) as f32
}

fn along(path: &[(i32, i32)], f: f32, j: f32) -> (f32, f32) {
    if path.len() < 2 {
        let p = path.first().copied().unwrap_or((0, 0));
        return (p.0 as f32 + 0.5, p.1 as f32 + 0.5);
    }
    let f = f.clamp(0.0, 1.0) * (path.len() - 1) as f32;
    let i = (f as usize).min(path.len() - 2);
    let frac = f - i as f32;
    let a = path[i];
    let b = path[i + 1];
    // A lane offset so a crowd walks abreast instead of in single file.
    let lane = (j - 0.5) * 0.24;
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let ox = -dy as f32 * lane;
    let oy = dx as f32 * lane * 0.5;
    (
        a.0 as f32 + (dx as f32) * frac + 0.5 + ox,
        a.1 as f32 + (dy as f32) * frac + 0.5 + oy,
    )
}

fn dir_along(path: &[(i32, i32)], f: f32) -> u8 {
    if path.len() < 2 {
        return 0;
    }
    let f = f.clamp(0.0, 1.0) * (path.len() - 1) as f32;
    let i = (f as usize).min(path.len() - 2);
    let (dx, dy) = (path[i + 1].0 - path[i].0, path[i + 1].1 - path[i].1);
    if dx.abs() >= dy.abs() {
        if dx > 0 { 0 } else { 2 }
    } else if dy > 0 {
        1
    } else {
        3
    }
}

fn dir_towards(a: (i32, i32), b: (i32, i32)) -> u8 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    if dx.abs() >= dy.abs() {
        if dx > 0 { 0 } else { 2 }
    } else if dy > 0 {
        1
    } else {
        3
    }
}

/// Is this tile somewhere a pedestrian may stand? (Used by tests.)
pub fn walkable(map: &TileMap, x: i32, y: i32) -> bool {
    let t = map.at(x, y);
    !t.is_water() && t.kind != Kind::Rock && t.relief() < 2
}

#[cfg(test)]
mod tests {
    use super::*;

    fn person(kind: AgeGroup, job: JobKind) -> Person {
        Person {
            first: 0,
            last: 0,
            age: 30,
            kind,
            home: Some((10, 10)),
            job: Some((20, 20)),
            job_kind: job,
            spouse: None,
            parent: None,
            tone: 1,
            jitter: 3,
            born: 0,
            life: Life::Alive,
        }
    }

    #[test]
    fn three_am_is_empty_and_rush_hour_is_busy() {
        let p = person(AgeGroup::Adult, JobKind::Office);
        assert_eq!(schedule(p.kind, p.job_kind, 3.0, false), Act::Asleep);
        assert_eq!(schedule(p.kind, p.job_kind, 23.5, false), Act::Asleep);
        assert!(matches!(
            schedule(p.kind, p.job_kind, 7.5, false),
            Act::Commute { to_work: true }
        ));
        assert!(matches!(
            schedule(p.kind, p.job_kind, 17.5, false),
            Act::Commute { to_work: false }
        ));
        assert_eq!(schedule(p.kind, p.job_kind, 19.0, false), Act::Leisure);
        assert_eq!(schedule(p.kind, p.job_kind, 12.4, false), Act::Lunch);
    }

    #[test]
    fn the_weekend_empties_the_offices_and_fills_the_park() {
        let p = person(AgeGroup::Adult, JobKind::Office);
        assert_eq!(schedule(p.kind, p.job_kind, 10.0, true), Act::Leisure);
        assert_eq!(schedule(p.kind, p.job_kind, 10.0, false), Act::Work);
    }

    #[test]
    fn shopkeepers_are_at_the_door_all_day() {
        let p = person(AgeGroup::Adult, JobKind::Shop);
        assert_eq!(schedule(p.kind, p.job_kind, 10.0, false), Act::Work);
        assert_eq!(schedule(p.kind, p.job_kind, 20.0, false), Act::Leisure);
    }

    #[test]
    fn a_child_with_a_school_goes_to_it() {
        let p = person(AgeGroup::Child, JobKind::School);
        assert_eq!(schedule(p.kind, p.job_kind, 10.0, false), Act::School);
        assert_eq!(schedule(p.kind, p.job_kind, 12.4, false), Act::Lunch);
        assert_eq!(schedule(p.kind, p.job_kind, 16.0, false), Act::Leisure);
    }

    #[test]
    fn doors_sit_outside_the_walls() {
        let l = Lot {
            x: 10,
            y: 10,
            w: 4,
            h: 3,
            level: 0,
            kind: LotKind::House,
            storeys: 2,
            style: 0,
            seed: 1,
            front: 1,
            stage: Stage::Finished,
            stage_end: 0.0,
            started: 0.0,
        };
        assert_eq!(door_tile(&l), (12, 13));
        let l2 = Lot { front: 0, ..l.clone() };
        assert_eq!(door_tile(&l2), (14, 11));
    }

    // --- the city's residents, through the sim ----------------------------

    fn grown(seed: u64, days: f64) -> crate::Sim {
        let mut s = crate::Sim::new(seed, crate::Pace::Month, 71, 71);
        s.run_to(days * 86400.0);
        s
    }

    /// B75/D83: nobody stands in the carriageway. Walking along a street with
    /// no pavement is allowed (a village has no kerbs), and a crossing is a
    /// crossing — but a figure with a *stationary* pose belongs on the pavement,
    /// the lawn or a doorstep, never in the middle of the road.
    #[test]
    fn nobody_stands_in_the_road() {
        for seed in [1u64, 3, 7, 11] {
            let mut s = grown(seed, 14.0);
            for hour in [8.5, 12.25, 17.5, 19.0] {
                s.set_clock_hour(hour);
                let v = s.people_view();
                let mut bad = 0;
                for f in &v {
                    if f.pose == Pose::Walk {
                        continue;
                    }
                    let t = s.map.at(f.x.floor() as i32, f.y.floor() as i32);
                    if t.kind.is_lane() {
                        bad += 1;
                    }
                }
                assert_eq!(
                    bad,
                    0,
                    "seed {seed} {hour}h: {bad} people standing in the road"
                );
            }
        }
    }

    /// B75: the street is alive outside the rush hours. The old model walked
    /// nobody at all between nine and five — this pins the ambient layer in.
    #[test]
    fn the_street_is_alive_all_day() {
        let mut s = grown(7, 16.0);
        for hour in [9.5, 12.25, 14.5, 20.0] {
            s.set_clock_hour(hour);
            let v = s.people_view();
            let walkers = v.iter().filter(|f| f.pose == Pose::Walk).count();
            let living = s.people.living().max(1);
            assert!(
                walkers * 100 >= living * 5,
                "{hour}h: only {walkers} walkers for {living} residents"
            );
        }
        // ...and the small hours are still empty of *people* (B77: the bins at
        // the kerb are furniture, not somebody out for a walk).
        s.set_clock_hour(3.0);
        let out = s.people_view();
        let people = out.iter().filter(|f| f.pose != Pose::Object).count();
        assert_eq!(people, 0, "3 am has {people} people out");
        let _ = out;
    }

    /// B75: the crowd is spread, not stacked. A park's visitors used to be six
    /// figures on six tiles; a crowd of a hundred must occupy many more tiles
    /// than that or the street reads empty beside it.
    #[test]
    fn a_crowd_is_spread_over_the_ground() {
        let mut s = grown(7, 16.0);
        s.set_clock_hour(19.0);
        let v = s.people_view();
        let mut per_tile: std::collections::BTreeMap<(i32, i32), usize> = Default::default();
        for f in &v {
            if matches!(f.pose, Pose::Chat | Pose::Stand | Pose::Sit | Pose::Play) {
                *per_tile
                    .entry((f.x.floor() as i32, f.y.floor() as i32))
                    .or_insert(0) += 1;
            }
        }
        let worst = per_tile.values().copied().max().unwrap_or(0);
        let total: usize = per_tile.values().sum();
        // A knot of people at one venue (a market stall, a festival crowd, B62)
        // is legitimate; what B75 removed is a *town* stacked on six tiles. The
        // bar is therefore "no tile holds a tenth of the standing crowd", with
        // an absolute ceiling for a pathological one.
        assert!(
            worst <= 30 && worst * 10 <= total.max(10),
            "{worst} of {total} standing on one tile: the crowd is stacked"
        );
        assert!(
            per_tile.len() * 5 >= v.len(),
            "{} people on {} tiles: the crowd is bunched",
            v.len(),
            per_tile.len()
        );
    }

    /// B75: crews work the site by day and go home at night (D83: "crews are on
    /// site during working hours only").
    #[test]
    fn the_crews_keep_working_hours() {
        let mut s = grown(3, 3.0);
        let mut day_with_site = 0;
        let mut night_checked = 0;
        for day in 3..=6 {
            for step in 0..(24 * 6) {
                let t = day as f64 * 86400.0 + step as f64 * 600.0;
                s.run_to(t);
                let hour = (s.time / 3600.0).rem_euclid(24.0);
                let sites = s
                    .lots
                    .iter()
                    .filter(|l| !l.finished() && l.stage >= Stage::Foundation)
                    .count();
                if sites == 0 {
                    continue;
                }
                let v = s.people_view();
                let hands = v.iter().filter(|f| f.tone == 8 || f.tone == 9).count();
                let d = ((s.time / 86400.0) + 3.0).floor().rem_euclid(7.0);
                let weekend = d == 0.0 || d == 6.0;
                if !weekend && (7.0..18.0).contains(&hour) {
                    assert!(
                        hands >= sites.min(3),
                        "day {day} {hour:.1}h: {sites} sites and {hands} hands"
                    );
                    day_with_site += 1;
                }
                if hour >= 20.0 || hour < 5.5 {
                    assert_eq!(hands, 0, "day {day} {hour:.1}h: {hands} on site at night");
                    night_checked += 1;
                }
            }
        }
        assert!(day_with_site > 0, "no site was ever staffed by day");
        assert!(night_checked > 0, "no site was ever visited at night");
    }

    #[test]
    fn every_resident_has_a_name_and_a_home() {
        let s = grown(3, 12.0);
        let n = s.residents() as usize;
        assert!(n > 20, "a day-12 town should hold people, got {n}");
        // B48: a house takes one household, so a few beds stay free (room for
        // the babies); the town holds most of what it has beds for.
        assert!(
            s.people.living() <= n.min(MAX_PEOPLE) && s.people.living() * 100 >= n * 60,
            "{} people for {n} beds",
            s.people.living()
        );
        let lot_tiles: Vec<(i32, i32)> = s.lots.iter().map(|l| (l.x, l.y)).collect();
        for (id, p) in s.people.list.iter().enumerate() {
            assert!(!p.name().is_empty());
            let home = p.home.expect("every resident has a home");
            // The door is one tile out from a real house, not a phantom.
            assert!(
                lot_tiles.iter().any(|&(x, y)| (x - home.0).abs() <= 6 && (y - home.1).abs() <= 6),
                "{} lives at {home:?}, which is in no lot",
                p.name()
            );
            if let Some(sp) = p.spouse {
                assert_eq!(s.people.list[sp as usize].spouse, Some(id as u32));
                assert_eq!(
                    s.people.list[sp as usize].home, p.home,
                    "partners live together"
                );
            }
        }
        assert!(
            s.people.list.iter().filter(|p| p.job.is_some()).count() > 5,
            "some residents should have jobs"
        );
    }

    #[test]
    fn residents_are_the_same_people_on_the_same_day() {
        let a = grown(5, 8.0);
        let b = grown(5, 8.0);
        assert_eq!(a.hash(), b.hash());
        let na: Vec<String> = a.people.list.iter().map(|p| p.name()).collect();
        let nb: Vec<String> = b.people.list.iter().map(|p| p.name()).collect();
        assert_eq!(na, nb);
        // ...and a different seed is a different town.
        let c = grown(6, 8.0);
        let nc: Vec<String> = c.people.list.iter().map(|p| p.name()).collect();
        assert_ne!(na, nc);
    }

    #[test]
    fn the_day_empties_the_streets_at_three_and_fills_them_at_eight() {
        let mut s = grown(4, 8.0);
        s.set_clock_hour(3.0);
        let night = s.people_view();
        // B77: the bins at the kerb are furniture, not people.
        let people = night.iter().filter(|f| f.pose != Pose::Object).count();
        assert_eq!(people, 0, "3 am: {people} people outdoors");
        s.set_clock_hour(8.0);
        let rush = s.people_view();
        assert!(rush.len() > 40, "08:00: only {} figures", rush.len());
        assert!(
            rush.iter().any(|f| f.pose == Pose::Walk),
            "the morning rush has people walking"
        );
        // D71: there is no figure cap any more, so the count is whatever the
        // schedules put outdoors — never a thinned sample.
        assert_eq!(
            rush.len(),
            s.people_view().len(),
            "the view must draw every figure it is given"
        );
        // ...and the same instant twice is the same picture.
        assert_eq!(rush, s.people_view());
    }

    #[test]
    fn the_view_keeps_the_activities_the_brief_asks_for() {
        // Every seed the planner named: a building or a farmer by day, and
        // social life in the evening — in a village with no park yet as well as
        // in a town (B43b item 1: the parks arrived too late for the test).
        for seed in [1u64, 3, 7, 11] {
            let mut s = grown(seed, 3.0);
            let mut worked = false;
            // A house is framed inside twenty minutes (D37), so the work check
            // walks the clock in five-minute steps rather than hourly — an
            // hourly sample can miss every crew on the map.
            'days: for day in 3..=8 {
                for step in 0..(24 * 12) {
                    s.run_to(day as f64 * 86400.0 + step as f64 * 300.0);
                    let hour = (s.time / 3600.0).rem_euclid(24.0);
                    if !(9.0..16.0).contains(&hour) {
                        continue;
                    }
                    let v = s.people_view();
                    if v.iter()
                        .any(|f| f.pose == Pose::Work || f.pose == Pose::Carry)
                    {
                        worked = true;
                        break 'days;
                    }
                }
            }
            assert!(worked, "seed {seed}: no builder or farmer in the village era");
            s.set_clock_hour(19.0);
            let v = s.people_view();
            assert!(
                v.iter().any(|f| f.pose == Pose::Chat
                    || f.pose == Pose::Play
                    || f.pose == Pose::Sit),
                "seed {seed}: evening with no social life"
            );
            assert!(
                v.iter().any(|f| f.prop != Prop::None),
                "seed {seed}: evening with no props"
            );
        }
    }

    /// B43b item 4: about two people in five are out at 08:00 and 17:30.
    #[test]
    fn rush_hour_fills_the_street() {
        for seed in [1u64, 3, 7, 11] {
            let mut s = grown(seed, 8.0);
            let mut kinds = std::collections::BTreeMap::new();
            for p in s.people.list.iter() {
                *kinds.entry(format!("{:?}", p.job_kind)).or_insert(0) += 1;
            }
            println!("seed {seed} job kinds: {kinds:?}");
            for p in s.people.list.iter().take(2000) {
                let a = schedule(p.kind, p.job_kind, 17.5, false);
                *kinds.entry(format!("act17.5 {:?}", a)).or_insert(0) += 1;
            }
            for hour in [8.0, 17.5] {
                s.set_clock_hour(hour);
                let v = s.people_view();
                let mut poses = std::collections::BTreeMap::new();
                for f in v.iter() {
                    *poses.entry(format!("{:?}", f.pose)).or_insert(0) += 1;
                }
                let mut acts = std::collections::BTreeMap::new();
                for p in s.people.list.iter() {
                    *acts.entry(format!("{:?}", schedule(p.kind, p.job_kind, hour, false))).or_insert(0) += 1;
                }
                println!("  {hour}h poses {poses:?} acts {acts:?}");
                let n = s.residents() as usize;
                assert!(
                    v.len() * 100 >= n * 30,
                    "seed {seed} {hour}h: {} figures for {n} residents",
                    v.len()
                );
                // 20 % was the bound when the evening rush was one crowd
                // walking home; with the venues and the porches the evening
                // spread is wider (seed 3 measured 17.4 % walking while 94 % of
                // the town is outdoors), so the bound is 15 %.
                assert!(
                    v.iter().filter(|f| f.pose == Pose::Walk).count() * 100 >= n * 15,
                    "seed {seed} {hour}h: too few walkers"
                );
            }
        }
    }

    /// B43b item 2: the commute is on the street when the street reaches.
    #[test]
    fn commuters_walk_the_street() {
        for seed in [1u64, 3, 7, 11] {
            let mut s = grown(seed, 8.0);
            s.set_clock_hour(8.0);
            let _ = s.people_view();
            let share = s.people.corridor_street_share(&s.map);
            assert!(
                share >= 0.70,
                "seed {seed}: only {:.0} % of the commute is on a street",
                share * 100.0
            );
        }
    }

    /// B43b item 3: nobody stands in a park's drawn pond.
    #[test]
    fn nobody_stands_in_a_park_pond() {
        for seed in [1u64, 3, 7, 11] {
            let mut s = grown(seed, 10.0);
            s.set_clock_hour(19.0);
            let v = s.people_view();
            let mut wet = 0;
            for f in &v {
                for l in s.lots.iter().filter(|l| {
                    l.kind == LotKind::Park
                        || l.kind == LotKind::Landmark(crate::landmarks::LandmarkKind::Plaza)
                }) {
                    let civic = l.kind != LotKind::Park;
                    if f.x >= l.x as f32
                        && f.x < (l.x + l.w) as f32
                        && f.y >= l.y as f32
                        && f.y < (l.y + l.h) as f32
                        && map::in_park_water(
                            l.w,
                            l.h,
                            civic,
                            map::ParkKind::of_style(l.style),
                            f.x - l.x as f32,
                            f.y - l.y as f32,
                        )
                    {
                        wet += 1;
                    }
                }
            }
            assert_eq!(wet, 0, "seed {seed}: {wet} figures standing in a pond or fountain basin");
        }
    }

    /// B43b item 5: three or four hands at every site.
    #[test]
    fn every_site_has_a_crew() {
        let mut s = grown(3, 4.0);
        let mut checked = 0;
        for step in 0..(24 * 12 * 2) {
            s.run_to(4.0 * 86400.0 + step as f64 * 300.0);
            let hour = (s.time / 3600.0).rem_euclid(24.0);
            if !(9.0..12.0).contains(&hour) || s.weekday() == 0.0 {
                continue;
            }
            let v = s.people_view();
            for l in s.lots.iter().filter(|l| !l.finished() && l.stage >= Stage::Foundation) {
                let crew = v
                    .iter()
                    .filter(|f| {
                        f.x >= l.x as f32 - 2.0
                            && f.x < (l.x + l.w) as f32 + 2.0
                            && f.y >= l.y as f32 - 2.0
                            && f.y < (l.y + l.h) as f32 + 2.0
                            && (f.pose == Pose::Work || f.pose == Pose::Carry)
                    })
                    .count();
                assert!(crew >= 3, "a {}x{} site with {crew} hands", l.w, l.h);
                checked += 1;
            }
            if checked > 6 {
                break;
            }
        }
        assert!(checked > 0, "no construction site in the first days");
    }

    #[test]
    fn a_save_keeps_the_residents_and_an_old_save_grows_them() {
        let mut s = grown(9, 10.0);
        let dir = std::env::temp_dir().join("citylive-b43-people");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("people.save");
        s.save(&path).unwrap();
        let back = crate::Sim::load(&path).unwrap();
        assert_eq!(back.people.list.len(), s.people.list.len());
        for (a, b) in s.people.list.iter().zip(back.people.list.iter()) {
            assert_eq!(a.name(), b.name());
            assert_eq!(a.home, b.home);
            assert_eq!(a.job, b.job);
            assert_eq!(a.spouse, b.spouse);
            assert_eq!(a.parent, b.parent);
        }
        // An old save (version 6) has no resident section: patch the version
        // byte and clear the list, and the loader has to grow them back from
        // the city it recorded.
        s.people.list.clear();
        s.save(&path).unwrap();
        let mut bytes = std::fs::read(&path).unwrap();
        bytes[4..8].copy_from_slice(&6u32.to_le_bytes());
        // B46: a v8 save also carries the world's biome (one byte just after
        // the founding point), which a v6 reader never asked for.
        bytes.remove(41);
        std::fs::write(&path, &bytes).unwrap();
        let old = crate::Sim::load(&path).unwrap();
        let n = old.residents() as usize;
        assert!(old.people.living() > n / 2 && old.people.living() <= n, "{} of {n}", old.people.living());
        assert!(old.people.list.iter().filter(|p| p.alive()).all(|p| p.home.is_some()));
        assert!(!old.people.list.is_empty());
    }
}

