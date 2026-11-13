//! B51: the chronicle's tests (determinism, the save round trip, an old save
//! backfilling, the size budget, and the text of a staged ending).

use crate::chronicle::*;
use crate::ending::{Ending, Phase};
use crate::{Pace, Sim};

fn world(seed: u64, days: f64) -> Sim {
    let mut s = Sim::new(seed, Pace::Month, 71, 71);
    s.run_to(days * 86400.0);
    s
}

fn tmp(tag: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("citylive-b51-{tag}.sav"))
}

#[test]
fn the_chronicle_is_deterministic() {
    let a = world(11, 8.0);
    let b = world(11, 8.0);
    assert!(!a.chronicle.entries.is_empty());
    assert_eq!(a.chronicle, b.chronicle);
    assert_eq!(a.chronicle_markdown(), b.chronicle_markdown());
    assert_eq!(a.hash(), b.hash());
    let c = world(12, 8.0);
    assert_ne!(a.chronicle_markdown(), c.chronicle_markdown(), "a different world reads differently");
}

#[test]
fn the_chronicle_round_trips_through_a_save() {
    let a = world(5, 8.0);
    let p = tmp("rt");
    a.save(&p).unwrap();
    let b = Sim::load(&p).unwrap();
    assert_eq!(a.chronicle, b.chronicle);
    assert_eq!(a.chronicle_markdown(), b.chronicle_markdown());
    assert_eq!(a.hash(), b.hash());
}

#[test]
fn an_older_save_loads_and_backfills_its_chronicle() {
    let a = world(6, 6.0);
    let p = tmp("old");
    a.save(&p).unwrap();
    let mut bytes = std::fs::read(&p).unwrap();
    // The chronicle is the last block of the file: cut it off and call the
    // file a v15 save.
    let mut block = 4usize;
    for e in &a.chronicle.entries {
        block += 8 + 1 + 4 + 4 + e.text.len();
    }
    bytes.truncate(bytes.len() - block);
    bytes[4..8].copy_from_slice(&15u32.to_le_bytes());
    std::fs::write(&p, &bytes).unwrap();
    let b = Sim::load(&p).expect("a v15 save still loads");
    assert!(!b.chronicle.entries.is_empty(), "the chronicle is rebuilt from the feed");
    assert!(b.chronicle_markdown().starts_with("# The Chronicle of "));
}

#[test]
fn the_chronicle_stays_small() {
    let s = world(9, 14.0);
    assert!(s.chronicle.entries.len() <= CHRONICLE_CAP);
    let bytes: usize = s.chronicle.entries.iter().map(|e| 17 + e.text.len()).sum();
    assert!(bytes < 64 * 1024, "{bytes} bytes");
    let mut c = Chronicle::default();
    for i in 0..2000 {
        c.push(i as f64, if i % 5 == 0 { Topic::Era } else { Topic::Note }, 0, &"x".repeat(500));
    }
    assert!(c.entries.len() <= CHRONICLE_CAP);
    assert!(c.entries.iter().all(|e| e.text.len() <= TEXT_CAP));
}

#[test]
fn a_staged_ending_is_written_up() {
    for kind in [Ending::Meteor, Ending::Decline, Ending::Aliens] {
        let mut s = world(7, 9.0);
        s.stage_ending(kind, Phase::Done, 0.5);
        s.run_to(s.time + 3600.0);
        let md = s.chronicle_markdown();
        assert!(md.contains(": The End"), "{kind:?}: {md}");
        assert!(md.contains(&s.city_name));
        assert!(s.chronicle.ended());
        assert!(md.contains("The last days"), "{kind:?}");
    }
}

/// The date phrase a sentence opens with: its shape and, when the phrase
/// prints one, the day number.
fn opener(s: &str) -> Option<(String, Option<u32>)> {
    let w: Vec<&str> = s.split_whitespace().collect();
    let first = w.first()?.trim_end_matches(',');
    let second = w.get(1).map(|t| t.trim_end_matches(',')).unwrap_or("");
    let names_day = matches!((first, second), ("On", "day") | ("By", "day") | ("Around", "day") | ("Then", "on"));
    let shape = match (first, second) {
        ("On", "day") | ("By", "day") | ("Around", "day") | ("Then", "on") | ("In", "the")
        | ("Sometime", "in") => format!("{first} {second}"),
        _ => return None,
    };
    let day = if names_day {
        w.iter()
            .skip(1)
            .find_map(|t| t.trim_matches(',').parse::<u32>().ok())
    } else {
        None
    };
    Some((shape, day))
}

/// B65: two sentences in a row never open with the same date phrase —
/// "On day 3, … By day 3, …" read as a stutter in review.
#[test]
fn consecutive_sentences_never_repeat_a_date_phrase() {
    for kind in [Ending::Meteor, Ending::Decline, Ending::Sunset] {
        for seed in [3u64, 7, 11, 19] {
            let mut s = world(seed, 11.0);
            s.stage_ending(kind, Phase::Done, 0.5);
            s.run_to(s.time + 3600.0);
            let md = s.chronicle_markdown();
            // The markdown's paragraphs are separated by blank lines, not full
            // stops, so flatten the line breaks before splitting sentences.
            let flat = md.replace('\n', " ");
            let mut prev: Option<(String, Option<u32>)> = None;
            let mut prev_sent = String::new();
            let mut checked = 0;
            for sent in flat.split(". ") {
                let Some(now) = opener(sent.trim()) else {
                    prev = None;
                    continue;
                };
                if let Some(p) = &prev {
                    assert_ne!(now.0, p.0, "seed {seed}: two sentences open with {}:\n  {:?}\n  {:?}", now.0, prev_sent, sent);
                    if let (Some(a), Some(b)) = (now.1, p.1) {
                        assert_ne!(a, b, "seed {seed}: day {a} named twice in a row. {md}");
                    }
                }
                checked += 1;
                prev = Some(now);
                prev_sent = sent.trim().to_string();
            }
            assert!(checked > 6, "seed {seed}: only {checked} dated sentences to check");
        }
    }
}

/// B65: the chronicle and the tray count days the same way — from day 0.
#[test]
fn the_chronicle_counts_days_like_the_tray() {
    for t in [0.0, 3600.0, 86_399.0, 86_400.0, 3.5 * 86400.0, 30.9 * 86400.0] {
        let line = crate::story::Line {
            time: t,
            text: "the first cabin is staked".into(),
        };
        let tray = crate::story::tray_line(&line);
        let want = day_number(t);
        assert_eq!(day_number(t), (t / 86400.0) as u32, "one definition of a day");
        assert!(
            tray.starts_with(&format!("day {want} — ")),
            "{tray} should say day {want}"
        );
    }
    // No sentence may name a day later than the world is old: with the old
    // 1-based count the last day's lines all printed one day too many.
    let mut s = world(11, 12.0);
    let md = s.chronicle_markdown();
    let today = day_number(s.time);
    let flat = md.replace('\n', " ");
    let mut named = 0;
    for sent in flat.split(". ") {
        if let Some((_, Some(day))) = opener(sent.trim()) {
            named += 1;
            assert!(day <= today, "day {day} is in the future of a {today}-day world: {sent}");
        }
    }
    assert!(named > 3, "only {named} dated sentences to check");
    // The ending's own list is numbered from the same count.
    s.stage_ending(Ending::Decline, Phase::Done, 0.5);
    s.run_to(s.time + 3600.0);
    let md = s.chronicle_markdown();
    let last = s
        .chronicle
        .entries
        .iter()
        .filter(|e| e.topic == Topic::Ending)
        .next_back()
        .expect("the ending is in the chronicle");
    assert!(
        md.contains(&format!("- Day {}: ", day_number(last.time))),
        "the ending list should say day {}: {md}",
        day_number(last.time)
    );
}
