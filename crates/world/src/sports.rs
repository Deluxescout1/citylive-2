//! Live sports scores: ESPN's public scoreboard (no key), NFL — port of old
//! `city.js` "LIVE SPORTS SCORES" (city.js:2729-2761) plus its
//! "BUFFALO BILLS GAMEDAY MODE" data (city.js:2780-2801, `realGameFor` /
//! `billsMode`).  When the real local team is playing, the city can show the
//! real live score; otherwise a renderer's fake game stands in.  Refresh is
//! ~2 min ([`crate::feeds::SPORTS_REFRESH_MS`], the old `SCORE_BUCKET`).

use serde::Deserialize;
use std::fs;
use std::path::Path;

use crate::http::HttpClient;

/// ESPN scoreboard path segment for the NFL (old `SCORE_EP.NFL`).
pub const LEAGUE_NFL: &str = "football/nfl";

/// `https://site.api.espn.com/apis/site/v2/sports/{league}/scoreboard`
/// (old `fetchScores` URL).  No key, no personal data.
pub fn url_for(league: &str) -> String {
    format!("https://site.api.espn.com/apis/site/v2/sports/{league}/scoreboard")
}

/// Game state from ESPN's `status.type.state`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameState {
    /// Scheduled, not started (`pre`).
    Upcoming,
    /// In progress (`in`).
    Live,
    /// Finished (`post`).
    Final,
    /// Any other status string.
    Other,
}

impl GameState {
    fn from_state(s: &str) -> Self {
        match s {
            "pre" => Self::Upcoming,
            "in" => Self::Live,
            "post" => Self::Final,
            _ => Self::Other,
        }
    }
}

/// One game on today's scoreboard.
#[derive(Debug, Clone, PartialEq)]
pub struct Game {
    /// Team name, e.g. `NEW ENGLAND PATRIOTS`.
    pub home: String,
    pub away: String,
    /// Abbreviation, e.g. `NE`.
    pub home_abbr: String,
    pub away_abbr: String,
    pub home_score: u32,
    pub away_score: u32,
    pub state: GameState,
    /// The short status line, e.g. `1st 12:34` / `Pre-Game` / `Final`.
    pub detail: Option<String>,
    /// Kickoff in Unix milliseconds (UTC), when the feed gives it.
    pub kickoff_ms: Option<i64>,
}

/// Today's scoreboard for a league (the ESPN endpoint only returns today's
/// games without a date parameter).
#[derive(Debug, Clone, Default)]
pub struct Scoreboard {
    pub games: Vec<Game>,
}

/// The Bills' game today (old `realGameFor("NFL","BILLS")`), with gameday
/// state (old `billsMode`).
#[derive(Debug, Clone, PartialEq)]
pub struct BillsGame {
    pub state: GameState,
    /// The Bills' score.
    pub us: u32,
    /// The opponent's score.
    pub them: u32,
    /// The opponent's abbreviation, e.g. `NE`.
    pub opp: String,
    /// The short status line, e.g. `2nd 7:12`.
    pub detail: Option<String>,
    /// Today is Bills gameday.  A lookup only ever finds games on *today's*
    /// scoreboard, so a hit is gameday.
    pub gameday: bool,
    /// Kickoff in Unix milliseconds (UTC), when the feed gives it.
    pub kickoff_ms: Option<i64>,
}

// ---- ESPN scoreboard response shapes ----

#[derive(Debug, Deserialize)]
struct EspnResponse {
    #[serde(default)]
    events: Option<Vec<EspnEvent>>,
}

#[derive(Debug, Deserialize)]
struct EspnEvent {
    /// Kickoff, ISO 8601 UTC, e.g. `2026-11-08T18:00Z`.
    #[serde(default)]
    date: Option<String>,
    #[serde(default)]
    status: Option<EspnStatus>,
    #[serde(default)]
    competitions: Vec<EspnCompetition>,
}

#[derive(Debug, Deserialize)]
struct EspnStatus {
    #[serde(default)]
    #[serde(rename = "type")]
    kind: Option<EspnStatusType>,
}

#[derive(Debug, Deserialize)]
struct EspnStatusType {
    #[serde(default)]
    state: Option<String>,
    #[serde(default, rename = "shortDetail")]
    short_detail: Option<String>,
}

#[derive(Debug, Deserialize)]
struct EspnCompetition {
    #[serde(default)]
    competitors: Vec<EspnCompetitor>,
}

#[derive(Debug, Deserialize)]
struct EspnCompetitor {
    #[serde(default, rename = "homeAway")]
    home_away: Option<String>,
    #[serde(default)]
    score: Option<String>,
    #[serde(default)]
    team: Option<EspnTeam>,
}

#[derive(Debug, Deserialize)]
struct EspnTeam {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    abbreviation: Option<String>,
}

/// Parse a recorded ESPN scoreboard body (the old `fetchScores` parse
/// block).  Games without both a home and an away competitor are dropped.
pub fn parse(body: &str) -> Result<Scoreboard, String> {
    let v: EspnResponse =
        serde_json::from_str(body).map_err(|e| format!("espn scoreboard json: {e}"))?;
    let mut games = Vec::new();
    for ev in v.events.as_deref().unwrap_or(&[]) {
        let (state, detail) = ev
            .status
            .as_ref()
            .and_then(|s| s.kind.as_ref())
            .map(|t| {
                (
                    GameState::from_state(t.state.as_deref().unwrap_or("")),
                    t.short_detail.clone(),
                )
            })
            .unwrap_or((GameState::Other, None));
        for c in &ev.competitions {
            let (mut home, mut away) = (None, None);
            for comp in &c.competitors {
                match comp.home_away.as_deref() {
                    Some("home") => home = Some(comp),
                    Some("away") => away = Some(comp),
                    _ => {}
                }
            }
            let (h, a) = match (home, away) {
                (Some(h), Some(a)) => (h, a),
                _ => continue,
            };
            games.push(Game {
                home: h
                    .team
                    .as_ref()
                    .and_then(|t| t.name.clone())
                    .unwrap_or_default()
                    .to_uppercase(),
                away: a
                    .team
                    .as_ref()
                    .and_then(|t| t.name.clone())
                    .unwrap_or_default()
                    .to_uppercase(),
                home_abbr: h
                    .team
                    .as_ref()
                    .and_then(|t| t.abbreviation.clone())
                    .unwrap_or_default(),
                away_abbr: a
                    .team
                    .as_ref()
                    .and_then(|t| t.abbreviation.clone())
                    .unwrap_or_default(),
                home_score: h
                    .score
                    .as_deref()
                    .and_then(|s| s.parse::<u32>().ok())
                    .unwrap_or(0),
                away_score: a
                    .score
                    .as_deref()
                    .and_then(|s| s.parse::<u32>().ok())
                    .unwrap_or(0),
                state,
                detail: detail.clone(),
                kickoff_ms: ev.date.as_deref().and_then(parse_iso_ms),
            });
        }
    }
    Ok(Scoreboard { games })
}

impl Scoreboard {
    /// The Bills' game today, if the feed has one (old
    /// `realGameFor("NFL","BILLS")` / `billsMode`).  Matches by name
    /// (`…BILLS`) or the `BUF` abbreviation.  The old JS compared the full
    /// team name to the bare string `"BILLS"` and so never actually matched;
    /// this is the corrected port (noted in the H44 handback).
    pub fn bills_game(&self) -> Option<BillsGame> {
        for g in &self.games {
            let ours_home = is_bills(&g.home, &g.home_abbr);
            let ours_away = !ours_home && is_bills(&g.away, &g.away_abbr);
            if !ours_home && !ours_away {
                continue;
            }
            let (us, them, opp) = if ours_home {
                (g.home_score, g.away_score, g.away_abbr.clone())
            } else {
                (g.away_score, g.home_score, g.home_abbr.clone())
            };
            return Some(BillsGame {
                state: g.state,
                us,
                them,
                opp,
                detail: g.detail.clone(),
                gameday: true,
                kickoff_ms: g.kickoff_ms,
            });
        }
        None
    }
}

/// The local hour the gameday "day" turns over at. ESPN's NFL scoreboard lists
/// the whole week (Thursday to Monday), so a game must be matched to the local
/// day; a late game still belongs to its evening until this hour next morning.
pub const GAMEDAY_ROLLOVER_H: i64 = 4;

impl BillsGame {
    /// Is this game today's, at `now_ms`, for a viewer `utc_offset_ms` from UTC?
    /// A game without a kickoff time is taken at its word (old behaviour).
    pub fn is_today(&self, now_ms: i64, utc_offset_ms: i64) -> bool {
        let day = |t: i64| (t + utc_offset_ms - GAMEDAY_ROLLOVER_H * 3_600_000).div_euclid(86_400_000);
        match self.kickoff_ms {
            Some(k) => day(k) == day(now_ms),
            None => true,
        }
    }
}

/// `2026-11-08T18:00Z` / `2026-11-08T18:00:00Z` → Unix ms. `None` for
/// anything else.
pub fn parse_iso_ms(s: &str) -> Option<i64> {
    let s = s.trim().trim_end_matches('Z');
    let (date, time) = s.split_once('T')?;
    let mut d = date.split('-');
    let y: i32 = d.next()?.parse().ok()?;
    let m: i32 = d.next()?.parse().ok()?;
    let day: i32 = d.next()?.parse().ok()?;
    let mut t = time.split(':');
    let hh: i64 = t.next()?.parse().ok()?;
    let mm: i64 = t.next()?.parse().ok()?;
    let ss: i64 = t.next().map(|x| x.split('.').next().unwrap_or("0").parse().unwrap_or(0)).unwrap_or(0);
    if !(1..=12).contains(&m) || !(1..=31).contains(&day) || hh > 23 || mm > 59 {
        return None;
    }
    let days = crate::model::days_from_civil(y, m, day);
    Some(((days * 24 + hh) * 60 + mm) * 60_000 + ss * 1000)
}

fn is_bills(name: &str, abbr: &str) -> bool {
    name.contains("BILLS") || abbr.eq_ignore_ascii_case("BUF")
}

/// Fetch today's NFL scoreboard.  Tries the network, then the cache file,
/// then `None` (a fake game plays instead — old behaviour).  The refresh
/// runs on the ~2 min bucket ([`crate::feeds::SPORTS_REFRESH_MS`]).
pub fn fetch(client: &dyn HttpClient, cache_path: &Path) -> Option<Scoreboard> {
    let url = url_for(LEAGUE_NFL);
    match client.get(&url) {
        Ok(body) => {
            if let Ok(sb) = parse(&body) {
                write_cache(cache_path, &body);
                return Some(sb);
            }
            // malformed → fall through to cache
        }
        Err(_) => {}
    }
    read_cache(cache_path).and_then(|body| parse(&body).ok())
}

fn write_cache(path: &Path, body: &str) {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(path, body);
}

fn read_cache(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join("citylive-world-test").join(format!(
            "{}-{}-{}",
            tag,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/data")
                .join(name),
        )
        .unwrap()
    }

    #[test]
    fn url_is_old_endpoint() {
        assert_eq!(
            url_for(LEAGUE_NFL),
            "https://site.api.espn.com/apis/site/v2/sports/football/nfl/scoreboard"
        );
    }

    #[test]
    fn parse_fixture_games() {
        let sb = parse(&fixture("nfl-scoreboard.json")).unwrap();
        assert_eq!(sb.games.len(), 2);
        let g0 = &sb.games[0];
        assert_eq!(g0.home, "NEW ENGLAND PATRIOTS");
        assert_eq!(g0.away, "BUFFALO BILLS");
        assert_eq!(g0.home_abbr, "NE");
        assert_eq!(g0.home_score, 10);
        assert_eq!(g0.away_score, 7);
        assert_eq!(g0.state, GameState::Live);
        assert_eq!(g0.detail.as_deref(), Some("1st 12:34"));
        let g1 = &sb.games[1];
        assert_eq!(g1.home, "MIAMI DOLPHINS");
        assert_eq!(g1.state, GameState::Upcoming);
        assert_eq!(g1.home_score, 0);
    }

    #[test]
    fn bills_game_away_fixture_is_gameday() {
        let sb = parse(&fixture("nfl-scoreboard.json")).unwrap();
        let g = sb.bills_game().unwrap();
        assert_eq!(g.state, GameState::Live);
        assert_eq!(g.us, 7, "Bills are away: their score is the away score");
        assert_eq!(g.them, 10);
        assert_eq!(g.opp, "NE");
        assert_eq!(g.detail.as_deref(), Some("1st 12:34"));
        assert!(g.gameday);
    }

    #[test]
    fn bills_game_home_and_final() {
        // The fixture has the Bills away; a home/final game is covered here.
        let body = r#"{
            "events": [{
                "status": {"type": {"state": "post", "shortDetail": "Final"}},
                "competitions": [{
                    "competitors": [
                        {"homeAway": "home", "score": "31",
                         "team": {"name": "Buffalo Bills", "abbreviation": "BUF"}},
                        {"homeAway": "away", "score": "14",
                         "team": {"name": "Green Bay Packers", "abbreviation": "GB"}}
                    ]
                }]
            }]
        }"#;
        let sb = parse(body).unwrap();
        let g = sb.bills_game().unwrap();
        assert_eq!(g.state, GameState::Final);
        assert_eq!(g.us, 31);
        assert_eq!(g.them, 14);
        assert_eq!(g.opp, "GB");
        assert!(g.gameday);
    }

    #[test]
    fn no_bills_game_is_none() {
        // A scoreboard without the Bills has no gameday (old `billsMode` → null).
        let body = r#"{
            "events": [{
                "status": {"type": {"state": "pre", "shortDetail": "Pre-Game"}},
                "competitions": [{
                    "competitors": [
                        {"homeAway": "home", "score": "0",
                         "team": {"name": "Cleveland Browns", "abbreviation": "CLE"}},
                        {"homeAway": "away", "score": "0",
                         "team": {"name": "Baltimore Ravens", "abbreviation": "BAL"}}
                    ]
                }]
            }]
        }"#;
        assert!(parse(body).unwrap().bills_game().is_none());
    }

    #[test]
    fn fetch_uses_network_then_cache_then_none() {
        let d = tmpdir("scores");
        let cache = d.join("scores.json");

        let body = fixture("nfl-scoreboard.json");
        let url = url_for(LEAGUE_NFL);
        let client = crate::http::tests::MapClient::new().ok(&url, &body);
        let sb = fetch(&client, &cache).unwrap();
        assert_eq!(sb.games.len(), 2);
        assert_eq!(std::fs::read_to_string(&cache).unwrap(), body, "cache written");

        // Offline → cache.
        let client = crate::http::tests::MapClient::new();
        let sb = fetch(&client, &cache).unwrap();
        assert_eq!(sb.bills_game().unwrap().opp, "NE");

        // Offline, no cache → None (fake game stands in).
        let d2 = tmpdir("scores2");
        assert!(fetch(&client, &d2.join("scores.json")).is_none());
        let _ = (d, d2);
    }

    #[test]
    fn a_game_counts_only_near_its_kickoff() {
        let k = parse_iso_ms("2026-11-08T18:00Z").unwrap();
        assert_eq!(parse_iso_ms("2026-11-08T18:00:00Z"), Some(k));
        let g = BillsGame {
            state: GameState::Final,
            us: 23,
            them: 6,
            opp: "MIN".into(),
            detail: None,
            gameday: true,
            kickoff_ms: Some(k),
        };
        let et = -5 * 3_600_000; // US Eastern, winter
        assert!(g.is_today(k + 5 * 3_600_000, et), "the evening after kickoff");
        assert!(!g.is_today(k + 50 * 3_600_000, et), "Tuesday is not gameday");
        assert!(!g.is_today(k - 72 * 3_600_000, et), "Thursday before is not gameday");
        // Monday night: kickoff 2026-11-10T01:15Z is 20:15 Monday in Buffalo.
        let mnf = parse_iso_ms("2026-11-10T01:15Z").unwrap();
        let tue_0328_et = parse_iso_ms("2026-11-10T08:28Z").unwrap();
        let tue_0928_et = parse_iso_ms("2026-11-10T14:28Z").unwrap();
        let m = BillsGame { kickoff_ms: Some(mnf), ..g.clone() };
        assert!(m.is_today(mnf + 3 * 3_600_000, et), "Monday 23:15 is still gameday");
        assert!(m.is_today(tue_0328_et, et), "before 04:00 the night still belongs to Monday");
        assert!(!m.is_today(tue_0928_et, et), "Tuesday morning is not gameday");
        assert_eq!(parse_iso_ms("nope"), None);
    }
}
