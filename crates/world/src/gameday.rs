//! Buffalo Bills gameday as a small state machine (B53 item 1, D68).
//!
//! The feed ([`crate::sports`]) says what the Bills' game looks like right now;
//! this module turns a stream of those readings into what the city shows:
//! the phase (none / pre-game / live / won / lost / tied), the mood (a win is a
//! celebration, a loss a quiet evening), whether fireworks go up that evening,
//! the one-line tray text, and a single feed line the first time a game is seen
//! final.  Pure logic: the caller passes the day, the local hour and the latest
//! reading, so tests drive it with a scripted feed.

use crate::sports::{BillsGame, GameState};

/// Where the Bills' game is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// No Bills game today (and none just finished).
    None,
    /// Scheduled for today, not started.
    Pregame,
    /// In progress.
    Live,
    /// Final, the Bills won.
    Won,
    /// Final, the Bills lost.
    Lost,
    /// Final, a tie.
    Tied,
}

/// The city's mood for the day.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mood {
    Normal,
    /// A win: bunting, cheering, fireworks in the evening.
    Celebrate,
    /// A loss or a tie: a quiet grey evening.
    Quiet,
}

/// What the city shows for the Bills right now.
#[derive(Clone, Debug, PartialEq)]
pub struct Gameday {
    pub phase: Phase,
    /// The Bills' score.
    pub us: u32,
    /// The opponent's score.
    pub them: u32,
    /// The opponent's abbreviation, e.g. `NE`.
    pub opp: String,
    /// The short status line from the feed, e.g. `2nd 7:12`.
    pub detail: Option<String>,
    /// Blue/red flags, tailgating and the stadium lot are on: a game is today.
    pub gameday: bool,
    pub mood: Mood,
    /// Fireworks over the stadium this evening (a win, final, after 18:00).
    pub fireworks: bool,
}

impl Gameday {
    /// No game and no recent result: the ordinary city.
    pub fn quiet_day() -> Self {
        Self {
            phase: Phase::None,
            us: 0,
            them: 0,
            opp: String::new(),
            detail: None,
            gameday: false,
            mood: Mood::Normal,
            fireworks: false,
        }
    }

    /// The tray's one-liner, or `None` when there is nothing to show.
    /// Live: `Bills 14 - 7 NE · 2nd 7:12`. Final: `Bills 31 - 14 GB · Final`.
    pub fn tray_line(&self) -> Option<String> {
        match self.phase {
            Phase::None => None,
            Phase::Pregame => Some(format!("Bills game today vs {}", self.opp)),
            Phase::Live => Some(match &self.detail {
                Some(d) => format!("Bills {} - {} {} · {}", self.us, self.them, self.opp, d),
                None => format!("Bills {} - {} {}", self.us, self.them, self.opp),
            }),
            Phase::Won | Phase::Lost | Phase::Tied => {
                Some(format!("Bills {} - {} {} · Final", self.us, self.them, self.opp))
            }
        }
    }
}

/// What one update produced: the state to show, and a feed line when a game
/// has just been seen final (once per game).
#[derive(Clone, Debug, PartialEq)]
pub struct Update {
    pub gameday: Gameday,
    pub feed_line: Option<String>,
}

/// Keeps the memory the state machine needs between readings.
#[derive(Clone, Debug, Default)]
pub struct Tracker {
    /// The last final result, with the day it was played.
    last_final: Option<(i64, Phase)>,
    /// Day for which the final feed line has already been made.
    announced: Option<i64>,
}

/// Fixed local hour from which a win gets its fireworks.
pub const FIREWORKS_FROM_HOUR: u8 = 18;

impl Tracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// One reading.  `day` is a local day number (any monotonic count of
    /// days), `hour` the local hour 0..=23, `game` the feed's Bills game for
    /// today (`None` when the feed has none or is offline).
    pub fn update(&mut self, day: i64, hour: u8, game: Option<&BillsGame>) -> Update {
        let Some(g) = game else {
            return Update {
                gameday: self.quiet_mood(day),
                feed_line: None,
            };
        };
        let mut out = Gameday {
            phase: Phase::Pregame,
            us: g.us,
            them: g.them,
            opp: g.opp.clone(),
            detail: g.detail.clone(),
            gameday: true,
            mood: Mood::Normal,
            fireworks: false,
        };
        let mut feed_line = None;
        match g.state {
            GameState::Upcoming | GameState::Other => out.phase = Phase::Pregame,
            GameState::Live => out.phase = Phase::Live,
            GameState::Final => {
                out.phase = match g.us.cmp(&g.them) {
                    std::cmp::Ordering::Greater => Phase::Won,
                    std::cmp::Ordering::Less => Phase::Lost,
                    std::cmp::Ordering::Equal => Phase::Tied,
                };
                self.last_final = Some((day, out.phase));
                if self.announced != Some(day) {
                    self.announced = Some(day);
                    feed_line = Some(feed_text(out.phase, g.us, g.them, &g.opp));
                }
                out.mood = mood_for(out.phase);
                out.fireworks = out.phase == Phase::Won && hour >= FIREWORKS_FROM_HOUR;
            }
        }
        Update {
            gameday: out,
            feed_line,
        }
    }

    /// No game in the feed today: carry yesterday's (or today's earlier) result
    /// as the mood, so a win or loss still shows the next day (D68).
    fn quiet_mood(&self, day: i64) -> Gameday {
        let mut q = Gameday::quiet_day();
        if let Some((d, phase)) = self.last_final {
            if d == day || d == day - 1 {
                q.mood = mood_for(phase);
            }
        }
        q
    }
}

fn mood_for(phase: Phase) -> Mood {
    match phase {
        Phase::Won => Mood::Celebrate,
        Phase::Lost | Phase::Tied => Mood::Quiet,
        Phase::None | Phase::Pregame | Phase::Live => Mood::Normal,
    }
}

/// The one feed line a final makes: cheerful for a win, quiet for a loss.
pub fn feed_text(phase: Phase, us: u32, them: u32, opp: &str) -> String {
    match phase {
        Phase::Won => format!(
            "Bills win {us}-{them} over {opp}! Horns on the streets and fireworks over the stadium tonight."
        ),
        Phase::Lost => format!("Bills lose {us}-{them} to {opp}. A quiet evening in town."),
        Phase::Tied => format!("Bills and {opp} finish {us}-{them}. Nobody leaves happy, nobody leaves angry."),
        Phase::None | Phase::Pregame | Phase::Live => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    /// A feed that replays a script of readings, one per poll.
    struct Scripted(VecDeque<Option<BillsGame>>);

    impl Scripted {
        fn new(items: Vec<Option<BillsGame>>) -> Self {
            Self(items.into())
        }
        fn poll(&mut self) -> Option<BillsGame> {
            self.0.pop_front().flatten()
        }
    }

    fn game(state: GameState, us: u32, them: u32, detail: Option<&str>) -> BillsGame {
        BillsGame {
            state,
            us,
            them,
            opp: "NE".into(),
            detail: detail.map(str::to_string),
            gameday: true,
            kickoff_ms: None,
        }
    }

    #[test]
    fn no_game_is_an_ordinary_day() {
        let mut t = Tracker::new();
        let u = t.update(100, 12, None);
        assert_eq!(u.gameday.phase, Phase::None);
        assert!(!u.gameday.gameday);
        assert_eq!(u.gameday.mood, Mood::Normal);
        assert!(u.feed_line.is_none());
        assert!(u.gameday.tray_line().is_none());
    }

    #[test]
    fn pre_live_final_win_sequence() {
        let mut feed = Scripted::new(vec![
            Some(game(GameState::Upcoming, 0, 0, Some("Pre-Game"))),
            Some(game(GameState::Live, 7, 3, Some("2nd 7:12"))),
            Some(game(GameState::Final, 24, 17, Some("Final"))),
            Some(game(GameState::Final, 24, 17, Some("Final"))),
        ]);
        let mut t = Tracker::new();
        let mut seen = Vec::new();
        let mut lines = Vec::new();
        for hour in [9u8, 14, 19, 22] {
            let u = t.update(200, hour, feed.poll().as_ref());
            seen.push(u.gameday.phase);
            if let Some(l) = u.feed_line {
                lines.push(l);
            }
            if hour == 19 {
                assert!(u.gameday.fireworks, "win after 18:00 gets fireworks");
                assert_eq!(u.gameday.mood, Mood::Celebrate);
            }
        }
        assert_eq!(
            seen,
            vec![Phase::Pregame, Phase::Live, Phase::Won, Phase::Won],
        );
        assert_eq!(lines.len(), 1, "the final is announced once, not per poll");
        assert!(lines[0].starts_with("Bills win 24-17"));
    }

    #[test]
    fn loss_is_quiet_with_no_fireworks() {
        let mut t = Tracker::new();
        let u = t.update(300, 20, Some(&game(GameState::Final, 10, 13, Some("Final"))));
        assert_eq!(u.gameday.phase, Phase::Lost);
        assert_eq!(u.gameday.mood, Mood::Quiet);
        assert!(!u.gameday.fireworks);
        assert!(u.feed_line.unwrap().starts_with("Bills lose 10-13"));
    }

    #[test]
    fn win_before_evening_has_no_fireworks_yet() {
        let mut t = Tracker::new();
        let u = t.update(300, 15, Some(&game(GameState::Final, 27, 20, Some("Final"))));
        assert_eq!(u.gameday.phase, Phase::Won);
        assert!(!u.gameday.fireworks);
        assert_eq!(u.gameday.mood, Mood::Celebrate);
    }

    #[test]
    fn tie_is_quiet() {
        let mut t = Tracker::new();
        let u = t.update(300, 21, Some(&game(GameState::Final, 20, 20, Some("Final"))));
        assert_eq!(u.gameday.phase, Phase::Tied);
        assert_eq!(u.gameday.mood, Mood::Quiet);
        assert!(!u.gameday.fireworks);
    }

    #[test]
    fn result_carries_to_the_next_day() {
        let mut t = Tracker::new();
        t.update(400, 21, Some(&game(GameState::Final, 31, 14, Some("Final"))));
        // Next day: the feed has no Bills game, the city still celebrates.
        let next = t.update(401, 9, None);
        assert_eq!(next.gameday.phase, Phase::None);
        assert_eq!(next.gameday.mood, Mood::Celebrate);
        // Two days later the mood has gone.
        let later = t.update(402, 9, None);
        assert_eq!(later.gameday.mood, Mood::Normal);
    }

    #[test]
    fn offline_feed_during_live_game_is_not_a_result() {
        let mut feed = Scripted::new(vec![
            Some(game(GameState::Live, 3, 0, Some("1st 4:00"))),
            None, // the feed drops out mid-game
            Some(game(GameState::Live, 3, 7, Some("3rd 2:00"))),
        ]);
        let mut t = Tracker::new();
        let mut phases = Vec::new();
        for _ in 0..3 {
            phases.push(t.update(500, 16, feed.poll().as_ref()).gameday.phase);
        }
        assert_eq!(phases, vec![Phase::Live, Phase::None, Phase::Live]);
        let line = t.update(500, 17, None).gameday.mood;
        assert_eq!(line, Mood::Normal, "a live game never sets a mood");
    }

    #[test]
    fn tray_lines() {
        let mut t = Tracker::new();
        let live = t.update(600, 16, Some(&game(GameState::Live, 14, 7, Some("2nd 7:12"))));
        assert_eq!(
            live.gameday.tray_line().as_deref(),
            Some("Bills 14 - 7 NE · 2nd 7:12")
        );
        let fin = t.update(600, 22, Some(&game(GameState::Final, 14, 7, Some("Final"))));
        assert_eq!(
            fin.gameday.tray_line().as_deref(),
            Some("Bills 14 - 7 NE · Final")
        );
    }

    #[test]
    fn feed_line_only_once_per_day() {
        let mut t = Tracker::new();
        let a = t.update(700, 21, Some(&game(GameState::Final, 10, 3, Some("Final"))));
        let b = t.update(700, 22, Some(&game(GameState::Final, 10, 3, Some("Final"))));
        assert!(a.feed_line.is_some());
        assert!(b.feed_line.is_none());
    }
}
