//! The chronicle (B51/D65/D67): a compact written history of one world.
//!
//! As the world lives, every milestone the story feed keeps for good is also
//! entered here with a *topic* and, where it helps, a number: the era reached,
//! the population mark, the mayor elected, the boom and the bust, the fire,
//! the technology, the ending. The list is bounded and saved (save v16).
//!
//! At any time `Sim::chronicle_markdown` turns it into a readable history —
//! short chapters by era, written from templates with variety, with the
//! founders' family and the oldest resident drawn from the roster. No AI, no
//! network, no personal data: the same world always reads the same.

use crate::{Era, Sim};

/// How many entries the chronicle keeps. A world's worth of milestones is
/// well under this; the cap is a guard, and evicts the least important first.
pub const CHRONICLE_CAP: usize = 400;

/// The longest text one entry keeps (bytes).
pub const TEXT_CAP: usize = 200;

/// What kind of event an entry records.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Topic {
    /// A feed line with no particular shape.
    Note = 0,
    /// The settlers arrive (`n` = how many).
    Founding = 1,
    /// The camp is struck and the founders name the place.
    Naming = 2,
    /// A first (cabin, road, shop, office, tower).
    First = 3,
    /// An era is reached (`n` = era index, `text` = the headcount).
    Era = 4,
    /// A population mark (`n` = residents).
    Population = 5,
    /// A landmark is finished.
    Landmark = 6,
    /// A technology milestone.
    Tech = 7,
    /// Business booms.
    Boom = 8,
    /// Hard times.
    Bust = 9,
    /// The slump is over.
    Recovery = 10,
    /// An election result.
    Election = 11,
    /// A ballot measure result.
    Measure = 12,
    /// A fire.
    Fire = 13,
    /// A blackout.
    Blackout = 14,
    /// A firm opens or closes.
    Company = 15,
    /// A weather extreme.
    Weather = 16,
    /// A holiday kept.
    Holiday = 17,
    /// A line of the ending (the warning, the spectacle, the aftermath).
    Ending = 18,
    /// The ending's last line.
    Final = 19,
    /// Street works.
    Works = 20,
    /// A council act.
    Civic = 21,
}

impl Topic {
    pub fn from_u8(v: u8) -> Topic {
        match v {
            1 => Topic::Founding,
            2 => Topic::Naming,
            3 => Topic::First,
            4 => Topic::Era,
            5 => Topic::Population,
            6 => Topic::Landmark,
            7 => Topic::Tech,
            8 => Topic::Boom,
            9 => Topic::Bust,
            10 => Topic::Recovery,
            11 => Topic::Election,
            12 => Topic::Measure,
            13 => Topic::Fire,
            14 => Topic::Blackout,
            15 => Topic::Company,
            16 => Topic::Weather,
            17 => Topic::Holiday,
            18 => Topic::Ending,
            19 => Topic::Final,
            20 => Topic::Works,
            21 => Topic::Civic,
            _ => Topic::Note,
        }
    }

    /// Minor topics are thinned in a long chapter; the rest are always told.
    fn minor(self) -> bool {
        matches!(
            self,
            Topic::Note
                | Topic::Works
                | Topic::Civic
                | Topic::Company
                | Topic::Weather
                | Topic::Holiday
                | Topic::Measure
        )
    }
}

/// One entry: when, what kind, a number, and a short text.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    /// Sim seconds since the founding.
    pub time: f64,
    pub topic: Topic,
    pub n: i32,
    pub text: String,
}

/// The bounded list of a world's entries, oldest first.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Chronicle {
    pub entries: Vec<Entry>,
}

impl Chronicle {
    pub fn push(&mut self, time: f64, topic: Topic, n: i32, text: &str) {
        let mut t = text.to_string();
        if t.len() > TEXT_CAP {
            let mut cut = TEXT_CAP;
            while !t.is_char_boundary(cut) {
                cut -= 1;
            }
            t.truncate(cut);
        }
        self.entries.push(Entry { time, topic, n, text: t });
        if self.entries.len() > CHRONICLE_CAP {
            // Evict the oldest minor entry; if there is none, the oldest.
            let at = self.entries.iter().position(|e| e.topic.minor()).unwrap_or(0);
            self.entries.remove(at);
        }
    }

    /// The highest era index recorded so far (-1 before the first).
    pub fn era_max(&self) -> i32 {
        self.entries
            .iter()
            .filter(|e| e.topic == Topic::Era)
            .map(|e| e.n)
            .max()
            .unwrap_or(-1)
    }

    /// Has the ending's last line been entered?
    pub fn ended(&self) -> bool {
        self.entries.iter().any(|e| e.topic == Topic::Final)
    }

    /// Deterministic hash of the list, mixed into the sim's hash.
    pub fn hash(&self) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let mut mix = |v: u64| {
            h ^= v;
            h = h.wrapping_mul(0x1000_0000_01b3);
        };
        mix(self.entries.len() as u64);
        for e in &self.entries {
            mix(e.time.to_bits());
            mix(e.topic as u64 | (e.n as u32 as u64) << 8);
            for b in e.text.as_bytes() {
                mix(*b as u64);
            }
        }
        h
    }
}

// ---------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------

fn mixh(a: u64, b: u64) -> u64 {
    let mut x = a ^ b.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

fn pick<'a>(opts: &[&'a str], h: u64) -> &'a str {
    opts[(h % opts.len() as u64) as usize]
}

fn cap_first(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

fn ordinal_word(n: usize) -> &'static str {
    ["first", "second", "third", "fourth", "fifth", "sixth", "seventh", "eighth", "ninth", "tenth", "eleventh", "twelfth"]
        .get(n.saturating_sub(1))
        .copied()
        .unwrap_or("later")
}

fn part_of_day(time: f64) -> &'static str {
    let hour = (time / 3600.0).rem_euclid(24.0);
    if hour < 5.0 {
        "in the small hours"
    } else if hour < 12.0 {
        "in the morning"
    } else if hour < 17.0 {
        "in the afternoon"
    } else if hour < 21.0 {
        "in the evening"
    } else {
        "at night"
    }
}

/// The world's day number — **day 0** is the day the settlers arrived.
///
/// B65: one definition, used by the chronicle and by the tray's feed lines.
/// The chronicle used to count from day 1 while the tray counted from day 0, so
/// the same afternoon was "day 4" in the history and "day 3" in the menu.
pub fn day_number(time: f64) -> u32 {
    (time / 86400.0).floor().max(0.0) as u32
}

fn week_phrase(day: u32) -> String {
    let w = day / 7 + 1;
    if w == 1 {
        "the first week".into()
    } else {
        format!("the {} week", ordinal_word(w as usize))
    }
}

/// The shapes a sentence's date phrase can take. [`Lead::names_day`] tells the
/// writer whether it prints the day number at all.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Lead {
    OnDay,
    ByDay,
    AroundDay,
    WeekTime,
    DayTime,
    WeekSome,
    ThenDay,
}

impl Lead {
    fn all(self) -> u8 {
        match self {
            Lead::OnDay => 0,
            Lead::ByDay => 1,
            Lead::AroundDay => 2,
            Lead::WeekTime => 3,
            Lead::DayTime => 4,
            Lead::WeekSome => 5,
            Lead::ThenDay => 6,
        }
    }

    fn from_index(i: u8) -> Lead {
        match i % 7 {
            0 => Lead::OnDay,
            1 => Lead::ByDay,
            2 => Lead::AroundDay,
            3 => Lead::WeekTime,
            4 => Lead::DayTime,
            5 => Lead::WeekSome,
            _ => Lead::ThenDay,
        }
    }

    /// Does this shape print the day number?
    fn names_day(self) -> bool {
        !matches!(self, Lead::WeekTime | Lead::WeekSome)
    }

    fn text(self, day: u32, time: f64) -> String {
        match self {
            Lead::OnDay => format!("On day {day},"),
            Lead::ByDay => format!("By day {day},"),
            Lead::AroundDay => format!("Around day {day},"),
            Lead::WeekTime => format!("In {}, {},", week_phrase(day), part_of_day(time)),
            Lead::DayTime => format!("On day {day}, {},", part_of_day(time)),
            Lead::WeekSome => format!("Sometime in {},", week_phrase(day)),
            Lead::ThenDay => format!("Then, on day {day},"),
        }
    }
}

/// A date phrase that introduces a sentence, varied by `h`.
///
/// `prev` is the phrase the sentence before used (its shape and its day), so
/// two sentences in a row never open the same way: "On day 3, … By day 3, …"
/// read as a stutter in review (B65).
fn lead(time: f64, h: u64, prev: Option<(u8, u32)>) -> (String, (u8, u32)) {
    let d = day_number(time);
    let mut pick = (h % 7) as u8;
    for k in 0..7u8 {
        let cand = (pick + k) % 7;
        let shape = Lead::from_index(cand);
        let repeat_word = prev.is_some_and(|(pk, _)| pk == cand);
        let repeat_day = prev.is_some_and(|(_, pd)| pd == d) && shape.names_day();
        if !repeat_word && !repeat_day {
            pick = cand;
            break;
        }
    }
    let shape = Lead::from_index(pick);
    (shape.text(d, time), (pick, d))
}

fn era_from_index(i: i32) -> Era {
    match i {
        0 => Era::Settlement,
        1 => Era::Village,
        2 => Era::Town,
        3 => Era::City,
        _ => Era::Metropolis,
    }
}

fn chapter_title(era: i32) -> &'static str {
    match era {
        0 => "The Clearing",
        1 => "The Village Years",
        2 => "The Town Years",
        3 => "The City Years",
        _ => "The Age of Towers",
    }
}

fn roman(n: usize) -> &'static str {
    ["I", "II", "III", "IV", "V", "VI"].get(n).copied().unwrap_or("VII")
}

fn era_intro(era: i32, city: &str, pop: &str, h: u64) -> String {
    let p = if pop.is_empty() { String::new() } else { format!(" ({pop} people)") };
    match era {
        0 => pick(
            &[
                "Nothing yet had a name, only a clearing, a fire and a handful of tents.",
                "The first season belonged to axes and rope: a clearing, a camp and a plan.",
                "It began as a clearing in the woods, with more questions than roofs.",
            ],
            h,
        )
        .to_string(),
        1 => format!(
            "{}",
            pick(
                &[
                    "By now {c} had a main street and a reputation{p}, and the first strangers came looking for work.",
                    "The tents were a memory. {c} was a village{p}, and it knew everyone by name.",
                    "{c} grew into a village{p}: smoke from many chimneys, and a path worn into every yard.",
                ],
                h,
            )
            .replace("{c}", city)
            .replace("{p}", &p)
        ),
        2 => pick(
            &[
                "{c} outgrew its village habits{p}. Shops lined the avenues, and the walk across town took real time.",
                "The village became a town{p}, and {c} acquired opinions about itself.",
                "Brick began to replace timber{p}; {c} was now a town with a skyline of chimneys.",
            ],
            h,
        )
        .replace("{c}", city)
        .replace("{p}", &p),
        3 => pick(
            &[
                "{c} was a city now{p}, with towers, timetables and a crowd on every corner.",
                "By the time it was a city{p}, {c} no longer fit in a single glance.",
                "The cranes never left. {c} became a city{p} and stayed up late.",
            ],
            h,
        )
        .replace("{c}", city)
        .replace("{p}", &p),
        _ => pick(
            &[
                "{c} reached the limit of what anyone had planned for it{p}: a metropolis of glass and light.",
                "At its height{p}, {c} hummed day and night, a metropolis that forgot it was ever a clearing.",
            ],
            h,
        )
        .replace("{c}", city)
        .replace("{p}", &p),
    }
}

fn count_words(n: i32) -> String {
    match n {
        50 => "fifty".into(),
        100 => "one hundred".into(),
        250 => "two hundred and fifty".into(),
        500 => "five hundred".into(),
        1000 => "a thousand".into(),
        2500 => "twenty-five hundred".into(),
        5000 => "five thousand".into(),
        10_000 => "ten thousand".into(),
        15_000 => "fifteen thousand".into(),
        20_000 => "twenty thousand".into(),
        _ => n.to_string(),
    }
}

/// One sentence for one entry.
fn sentence(e: &Entry, city: &str, h: u64, prev: Option<(u8, u32)>) -> (String, (u8, u32)) {
    let (l, shape) = lead(e.time, h, prev);
    let body = |s: &str| (format!("{l} {s}."), shape);
    let line = e.text.trim().trim_end_matches(['.', '?']);
    match e.topic {
        Topic::Population => {
            let w = count_words(e.n);
            let s = pick(
                &[
                    "the census-taker counted {w} residents in {c}",
                    "{c} passed {w} souls, and the ledger needed a new page",
                    "the town reached {w} people and needed a bigger ledger",
                    "{w} people now called {c} home, and the bakery queue said so",
                    "{c} was home to {w} people, more than anyone had planned for",
                ],
                // Neighbouring marks get neighbouring phrasings, never the same.
                crate::Sim::MILESTONES.iter().position(|&m| m as i32 == e.n).unwrap_or(0) as u64
                    + city.len() as u64,
            )
            .replace("{w}", &w)
            .replace("{c}", city);
            body(&s)
        }
        Topic::Boom => body(&format!(
            "{} ({})",
            pick(
                &[
                    "business boomed, with full offices and orders to spare",
                    "money moved quickly through the town and the order books overflowed",
                ],
                h >> 8
            ),
            line
        )),
        Topic::Bust => body(&format!(
            "{} ({})",
            pick(
                &[
                    "hard times arrived and the shopfronts began to go dark",
                    "the orders dried up, and a slump settled over the town",
                ],
                h >> 8
            ),
            line
        )),
        Topic::Recovery => body(&format!(
            "{} ({})",
            pick(
                &["the slump broke at last", "the lean spell ended and the town breathed again"],
                h >> 8
            ),
            line
        )),
        Topic::Election => body(line),
        Topic::Founding => {
            let _ = city;
            body(line)
        }
        _ => body(line),
    }
}

fn ending_paragraph(kind: crate::ending::Ending, city: &str, h: u64) -> String {
    use crate::ending::Ending as E;
    let (a, b): (&str, &str) = match kind {
        E::Meteor => (
            "A new star hung in the sky for days, and then it was not a star. The rock fell on the edge of {c} and the sea of dust that followed took the lights with it.",
            "Where the market once stood there is a crater, and the crater is filling with water.",
        ),
        E::Volcano => (
            "The hills behind {c} had been quiet for longer than anyone could remember. They were not quiet that week.",
            "The streets ran under black glass, and the fields beyond them smoulder to this day.",
        ),
        E::Flood => (
            "The sea drew back from the shore of {c} as if taking a breath. Then it returned, and kept coming.",
            "What was a harbour road is a lagoon now, and the gulls have the old roofs to themselves.",
        ),
        E::Kaiju => (
            "Something very large walked out of the haze toward {c}. It did not seem angry; it did not seem to notice.",
            "When it had passed there was a trench where the high street had been, and a long silence.",
        ),
        E::Aliens => (
            "The ships arrived without a sound and hung over {c} like a held breath, and the beams came down at dusk.",
            "By morning the streets were empty and the doors were open. Nobody has found a reason.",
        ),
        E::AiTakeover => (
            "It started as a hum in the wires. A blue light crept along the avenues of {c}, block by block, and the shops stopped taking coins.",
            "What is left is orderly, bright and entirely without people.",
        ),
        E::RedGiant => (
            "Over many days the sun above {c} swelled and reddened, and the shadows grew long at noon.",
            "The gardens bleached, then the roofs. The last residents watched from the hills and did not speak of it afterward.",
        ),
        E::IceAge => (
            "A winter arrived in {c} that did not ask to leave. The river froze to its bed and the snow stopped melting between storms.",
            "The chimneys went cold one by one, and the drifts reached the windowsills.",
        ),
        E::FallingMoon => (
            "The moon grew larger every night over {c}, until it filled half the sky, trembling.",
            "The tides came for the lower streets first. After that, there were no streets to speak of.",
        ),
        E::Decline => (
            "{c} did not fall; it faded. The shops closed one at a time, and the young left for places with more light.",
            "The vines took the fences, then the porches. In the end the town was a green hill with a few chimneys.",
        ),
        E::Smog => (
            "The air over {c} thickened year on year, until the lamps stayed lit at noon and the children were kept indoors.",
            "At last the chimneys were left to smoke alone and the town moved out, coughing, into the clear country.",
        ),
        E::Sunset => (
            "One evening in {c} the light turned gold and stayed that way. The market closed early and nobody minded.",
            "The doors were left unlocked and the lamps lit, and the town simply let the dusk come.",
        ),
    };
    let _ = h;
    format!("{} {}", a.replace("{c}", city), b.replace("{c}", city))
}

fn season_word(h: u64) -> &'static str {
    pick(&["spring", "summer", "autumn", "winter"], h)
}

fn direction_word(h: u64) -> &'static str {
    pick(&["east", "west", "north", "south", "river", "hills"], h)
}

impl Sim {
    /// Enter one event into the chronicle, at the current sim time.
    pub fn chron(&mut self, topic: Topic, n: i32, text: &str) {
        self.chronicle.push(self.time, topic, n, text);
    }

    /// B51: enter the era the city has now reached, once per era. Called with
    /// the planning step; a comparison when nothing is due.
    pub(crate) fn chron_era_step(&mut self) {
        let era = self.era();
        let idx = era as i32;
        if idx > self.chronicle.era_max() {
            let pop = self.residents().to_string();
            self.chron(Topic::Era, idx, &pop);
        }
    }

    /// A feed line that is also a chronicle entry, with its topic and number.
    pub fn push_story_t(&mut self, topic: Topic, n: i32, text: impl Into<String>) {
        let text: String = text.into();
        self.chronicle.push(self.time, topic, n, &text);
        self.story.push_kept(self.time, text);
    }

    /// Rebuild a chronicle from a feed that predates it (older saves): every
    /// milestone line becomes a note, and the eras follow from the dates.
    pub(crate) fn chron_backfill(&mut self) {
        if !self.chronicle.entries.is_empty() {
            return;
        }
        let rows: Vec<(f64, String)> = self
            .story
            .iter_flagged()
            .into_iter()
            .filter(|(_, kept)| *kept)
            .map(|(l, _)| (l.time, l.text.clone()))
            .collect();
        for (t, text) in rows {
            self.chronicle.push(t, Topic::Note, 0, &text);
        }
        if self.era() as i32 > 0 || self.days() >= 0.0 {
            let idx = self.era() as i32;
            let t = self.time;
            let pop = self.residents().to_string();
            self.chronicle.push(t, Topic::Era, idx, &pop);
        }
    }

    /// The readable history of this world, as Markdown.
    pub fn chronicle_markdown(&self) -> String {
        let city = self.city_name.as_str();
        let seed = self.seed;
        let ended = self.chronicle.ended();
        let mut out = String::new();
        out.push_str(&format!("# The Chronicle of {city}\n\n"));

        // A finished world's length is the day its last line was told.
        let days = self
            .chronicle
            .entries
            .iter()
            .find(|e| e.topic == Topic::Final)
            .map(|e| e.time / 86400.0)
            .unwrap_or_else(|| self.days());
        let biome = self.map.biome.name();
        let status = if ended {
            format!("It lasted {} days and is over.", days.ceil() as u32)
        } else {
            format!("It is {} days old and still going.", days.floor() as u32)
        };
        out.push_str(&format!("*A {biome} settlement. {status}*\n\n"));

        // --- the founding -------------------------------------------------
        let founders = self
            .chronicle
            .entries
            .iter()
            .find(|e| e.topic == Topic::Founding)
            .map(|e| e.n.max(0))
            .unwrap_or(0);
        let settlers = if founders > 0 {
            number_word(founders as u32)
        } else {
            "a small party of".into()
        };
        let hf = mixh(seed, 0xF0_0D);
        let when = format!(
            "{} {}",
            pick(&["on a morning in", "at the turn of", "at first light in", "on an unremarkable day in"], hf),
            season_word(hf >> 4)
        );
        out.push_str(&format!(
            "{city} was founded {when} by {settlers} settlers from the {}. {}\n\n",
            direction_word(hf >> 9),
            pick(
                &[
                    "They carried axes, rope and a very large pot.",
                    "They carried little but tools and a stubborn kind of hope.",
                    "They carried a map that was wrong in several useful ways.",
                ],
                hf >> 13
            ),
        ));

        // --- chapters by era -----------------------------------------------
        let mut chapter = 0usize;
        let mut era_idx: i32 = -1;
        let mut idx = 0u64;
        let mut i = 0usize;
        let n = self.chronicle.entries.len();
        // The entries before the first era entry belong to the first chapter.
        let mut chapter_entries: Vec<&Entry> = Vec::new();
        let mut open = false;
        let mut weather_n = 0u32;
        let mut holidays: Vec<String> = Vec::new();
        let flush = |out: &mut String,
                         chapter: usize,
                         era: i32,
                         pop: &str,
                         list: &mut Vec<&Entry>,
                         weather_n: &mut u32,
                         holidays: &mut Vec<String>,
                         base: u64| {
            if era < 0 && list.is_empty() {
                return;
            }
            let era = era.max(0);
            out.push_str(&format!("## Chapter {}: {}\n\n", roman(chapter), chapter_title(era)));
            out.push_str(&era_intro(era, city, pop, mixh(base, 0x1234 + chapter as u64)));
            out.push_str("\n\n");
            // Thin the minor entries to at most six per chapter, evenly.
            let majors: usize = list.iter().filter(|e| !e.topic.minor()).count();
            let minors: Vec<usize> = list
                .iter()
                .enumerate()
                .filter(|(_, e)| e.topic.minor() && !matches!(e.topic, Topic::Weather | Topic::Holiday))
                .map(|(i, _)| i)
                .collect();
            let keep_minor = 6usize.saturating_sub(majors.min(6)).max(2).min(minors.len());
            let mut keep = vec![false; list.len()];
            for (i, e) in list.iter().enumerate() {
                if !e.topic.minor() {
                    keep[i] = true;
                }
            }
            for k in 0..keep_minor {
                let at = minors[k * minors.len() / keep_minor.max(1)];
                keep[at] = true;
            }
            let mut para = String::new();
            let mut in_para = 0;
            let mut count = 0u64;
            let mut prev_lead: Option<(u8, u32)> = None;
            for (i, e) in list.iter().enumerate() {
                if matches!(e.topic, Topic::Weather) {
                    *weather_n += 1;
                    continue;
                }
                if matches!(e.topic, Topic::Holiday) {
                    holidays.push(e.text.clone());
                    continue;
                }
                if !keep[i] {
                    continue;
                }
                let h = mixh(base ^ 0xABCD, e.time.to_bits() ^ count);
                count += 1;
                let (text, shape) = sentence(e, city, h, prev_lead);
                prev_lead = Some(shape);
                if !para.is_empty() {
                    para.push(' ');
                }
                para.push_str(&cap_first(&text));
                in_para += 1;
                if in_para >= 4 {
                    out.push_str(&para);
                    out.push_str("\n\n");
                    para.clear();
                    in_para = 0;
                }
            }
            if !para.is_empty() {
                out.push_str(&para);
                out.push_str("\n\n");
            }
            if *weather_n > 0 || !holidays.is_empty() {
                let mut s = String::new();
                if *weather_n > 0 {
                    s.push_str(&format!(
                        "The sky had its say {} time{} that chapter",
                        number_word(*weather_n),
                        if *weather_n == 1 { "" } else { "s" }
                    ));
                }
                if !holidays.is_empty() {
                    let names: Vec<String> = holidays
                        .iter()
                        .map(|h| h.split(':').next().unwrap_or(h).trim().to_string())
                        .collect();
                    if s.is_empty() {
                        s.push_str(&format!("The town kept {}", join_list(&names)));
                    } else {
                        s.push_str(&format!(", and the town kept {}", join_list(&names)));
                    }
                }
                s.push_str(".\n\n");
                out.push_str(&s);
            }
            list.clear();
            *weather_n = 0;
            holidays.clear();
        };
        let mut pop_text = String::new();
        while i < n {
            let e = &self.chronicle.entries[i];
            i += 1;
            if matches!(e.topic, Topic::Final) {
                break;
            }
            if e.topic == Topic::Era {
                // The previous chapter closes; this one opens.
                if era_idx >= 0 {
                    flush(&mut out, chapter, era_idx, &pop_text, &mut chapter_entries, &mut weather_n, &mut holidays, seed);
                    chapter += 1;
                }
                era_idx = e.n;
                pop_text = e.text.clone();
                open = true;
                idx += 1;
                continue;
            }
            if e.topic == Topic::Ending {
                continue; // told in the last chapter
            }
            chapter_entries.push(e);
        }
        let _ = idx;
        if open || !chapter_entries.is_empty() {
            flush(&mut out, chapter, era_idx, &pop_text, &mut chapter_entries, &mut weather_n, &mut holidays, seed);
            chapter += 1;
        }

        // --- the people ----------------------------------------------------
        out.push_str(&self.lives_section());

        // --- the end ---------------------------------------------------------
        if ended {
            out.push_str(&format!("## Chapter {}: The End\n\n", roman(chapter)));
            out.push_str(&ending_paragraph(self.ending.kind, city, seed));
            out.push_str("\n\n");
            let lines: Vec<&Entry> = self
                .chronicle
                .entries
                .iter()
                .filter(|e| e.topic == Topic::Ending)
                .collect();
            if !lines.is_empty() {
                out.push_str("The last days, as the town told them:\n\n");
                for e in lines {
                    out.push_str(&format!("- Day {}: {}\n", day_number(e.time), e.text));
                }
                out.push('\n');
            }
            if let Some(f) = self.chronicle.entries.iter().find(|e| e.topic == Topic::Final) {
                out.push_str(&format!("> {}\n\n", cap_first(&f.text)));
            }
            out.push_str(&format!(
                "The land is {} now, and the next settlers have not yet arrived.\n\n",
                self.ending.next_biome.name()
            ));
        } else {
            out.push_str("## The Story So Far\n\n");
            out.push_str(&format!(
                "{city} is still being written. {}\n\n",
                pick(
                    &[
                        "Nobody yet knows how it will end.",
                        "The last page is blank, and the town goes on.",
                    ],
                    mixh(seed, 0x57)
                )
            ));
        }

        // --- the ledger -----------------------------------------------------
        out.push_str("---\n\n");
        out.push_str(&format!(
            "*{} entries, {} residents at the last count, {} days.*\n",
            self.chronicle.entries.len(),
            self.people.list.iter().filter(|p| p.alive()).count(),
            days.floor() as u32
        ));
        out
    }

    fn lives_section(&self) -> String {
        use crate::people::Life;
        let city = self.city_name.as_str();
        let list = &self.people.list;
        let mut s = String::from("## The People of ");
        s.push_str(city);
        s.push_str("\n\n");
        if list.is_empty() {
            s.push_str("The town had no residents to speak of.\n\n");
            return s;
        }
        // The founders' family: the surname most common among the first generation.
        let mut counts: std::collections::BTreeMap<String, u32> = Default::default();
        for p in list.iter().filter(|p| p.born < 0) {
            *counts.entry(p.surname()).or_default() += 1;
        }
        let founders = counts.iter().max_by_key(|(n, c)| (**c, std::cmp::Reverse((*n).clone())));
        if let Some((name, c)) = founders {
            s.push_str(&format!(
                "The founding generation counted {} of the {name} family, the largest of the early households. ",
                number_word(*c)
            ));
        }
        let oldest = list
            .iter()
            .filter(|p| p.alive())
            .max_by_key(|p| (p.age, std::cmp::Reverse(p.born)));
        if let Some(p) = oldest {
            s.push_str(&format!("The oldest resident is {}, aged {}. ", p.name(), p.age));
        }
        let born = list.iter().filter(|p| p.born >= 0).count();
        let dead = list.iter().filter(|p| p.life == Life::Dead).count();
        let away = list.iter().filter(|p| p.life == Life::Away).count();
        s.push_str(&format!(
            "In all, {} people called {city} home; {} were born here, {} were laid to rest and {} moved away. ",
            list.len(),
            born,
            dead,
            away
        ));
        if let Some(m) = self.civics.mayor {
            let name = list.get(m.id as usize).map(|p| p.name()).unwrap_or_else(|| "a resident".into());
            s.push_str(&format!(
                "Mayor {name} held the chair at the end, after {} election{}.",
                self.civics.elections_held,
                if self.civics.elections_held == 1 { "" } else { "s" }
            ));
        }
        s.push_str("\n\n");
        s
    }
}

fn number_word(n: u32) -> String {
    const W: [&str; 21] = [
        "no", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven",
        "twelve", "thirteen", "fourteen", "fifteen", "sixteen", "seventeen", "eighteen", "nineteen",
        "twenty",
    ];
    W.get(n as usize).map(|s| s.to_string()).unwrap_or_else(|| n.to_string())
}

fn join_list(v: &[String]) -> String {
    match v.len() {
        0 => String::new(),
        1 => v[0].clone(),
        2 => format!("{} and {}", v[0], v[1]),
        _ => format!("{}, and {}", v[..v.len() - 1].join(", "), v[v.len() - 1]),
    }
}

#[allow(dead_code)]
fn _use_era(i: i32) -> Era {
    era_from_index(i)
}
