//! World logic for CityLive Iso.
//!
//! Deps-light port of the pure logic in old CityLive `city.js`
//! (`reference/CityLive-old/.../js/city.js`): real sun/moon astronomy,
//! Open-Meteo weather, IP geolocation, holidays and seasons.  Everything that
//! can be pure is pure; networking is isolated behind [`http::HttpClient`] so
//! the weather/geo paths are testable offline.
//!
//! The parity with the original JS is proven by `tools/world-check/` (see the
//! `h5` tests): a Node harness runs the *verbatim* JS functions against ~300
//! fixed cases and this crate mirrors them within tolerance.

pub mod aurora;
pub mod astro;
pub mod calendar;
pub mod dates;
pub mod feeds;
pub mod gameday;
pub mod flights;
pub mod geo;
pub mod http;
pub mod iss;
pub mod moon;
pub mod model;
pub mod season;
pub mod spectacle;
pub mod sports;
pub mod starcat;
pub mod sun;
pub mod weather;

/// Local calendar stamp (year/month/day/hour).
pub use model::Date;

// Human-oriented top-level aliases so call sites read like the old CityLive API.
pub use calendar::{decor, decorated, easter_date, holidays, season, Decor, Holiday, Season};
pub use dates::{on_date, DateStyle, SpecialDate};
pub use feeds::Feeds;
pub use gameday::{Gameday, Mood, Phase as GamedayPhase, Tracker as GamedayTracker};
pub use geo::{locate, Location, DEFAULT_LATLON};
pub use http::{HttpClient, HttpError};
#[cfg(feature = "net")]
pub use http::UreqClient;
pub use moon::{alt_az, lst_hours, ra_dec, Phase, RaDec};
pub use season::{day_of_year, SeasonLook};
pub use sports::{GameState, Game, Scoreboard};
pub use sun::{position as sun_position, times as sun_times, Polar, SunTimes};
pub use weather::{url_for as weather_url, parse as weather_parse, Weather};

/// Live world feeds (B44): flights, aurora Kp, ISS, NFL scores — one module
/// each, all off-switchable via [`Feeds`], all degrading gracefully offline.
pub use aurora::Aurora;
pub use flights::Flight;
pub use iss::{Iss, Look};
