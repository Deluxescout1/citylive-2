use super::*;
use std::cell::RefCell;
use world::HttpError;

/// A mock HTTP client: URL -> canned answer, everything else fails. No network.
struct Mock {
    answers: Vec<(String, String)>,
    log: RefCell<Vec<String>>,
}

impl Mock {
    fn new(answers: &[(&str, &str)]) -> Self {
        Self {
            answers: answers.iter().map(|(u, b)| (u.to_string(), b.to_string())).collect(),
            log: RefCell::new(Vec::new()),
        }
    }
    fn called(&self) -> Vec<String> {
        self.log.borrow().clone()
    }
}

impl HttpClient for Mock {
    fn get(&self, url: &str) -> Result<String, HttpError> {
        self.log.borrow_mut().push(url.to_string());
        self.answers
            .iter()
            .find(|(u, _)| u == url)
            .map(|(_, b)| b.clone())
            .ok_or_else(|| HttpError::Network("mock: down".to_string()))
    }
}

struct FakeOs(Result<OsFix, String>);
impl OsLocator for FakeOs {
    fn name(&self) -> &'static str {
        "FakeOS"
    }
    fn locate(&self, _t: Duration) -> Result<OsFix, String> {
        self.0.clone()
    }
}

const IPAPI: &str = r#"{"ip":"1.2.3.4","city":"Sydney","region":"New South Wales","country":"AU","country_name":"Australia","latitude":-33.8688,"longitude":151.2093}"#;
const IPAPI_LIMITED: &str = r#"{"error":true,"reason":"RateLimited","message":"Too many"}"#;
const IPWHO: &str = r#"{"ip":"5.6.7.8","success":true,"city":"Brooklyn","region":"New York","country":"United States","latitude":40.6782,"longitude":-73.9442}"#;
const IPWHO_FAIL: &str = r#"{"ip":"5.6.7.8","success":false,"message":"Invalid IP address"}"#;
const GEOJS: &str = r#"{"country":"Germany","latitude":"52.5200","longitude":"13.4050","city":"Berlin","region":"Berlin","ip":"9.9.9.9"}"#;
const REALLYFREE: &str = r#"{"ip":"8.8.4.4","country_code":"NL","country_name":"Netherlands","region_name":"North Holland","city":"Amsterdam","latitude":52.3676,"longitude":4.9041}"#;

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join("citylive-locate-tests").join(format!(
        "{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d.join("location.json")
}

fn svc(name: &str) -> &'static IpService {
    SERVICES.iter().find(|s| s.name == name).unwrap()
}

#[test]
fn parses_ipapi() {
    let (lat, lon, name) = parse_service(svc("ipapi.co"), IPAPI).unwrap();
    assert!((lat + 33.8688).abs() < 1e-6 && (lon - 151.2093).abs() < 1e-6);
    assert_eq!(name, "Sydney, New South Wales, Australia");
    assert!(parse_service(svc("ipapi.co"), IPAPI_LIMITED).is_err());
}

#[test]
fn parses_ipwho() {
    let (lat, _, name) = parse_service(svc("ipwho.is"), IPWHO).unwrap();
    assert!((lat - 40.6782).abs() < 1e-6);
    assert_eq!(name, "Brooklyn, New York, United States");
    assert!(parse_service(svc("ipwho.is"), IPWHO_FAIL).is_err());
}

#[test]
fn parses_geojs_string_numbers() {
    let (lat, lon, name) = parse_service(svc("geojs.io"), GEOJS).unwrap();
    assert!((lat - 52.52).abs() < 1e-6 && (lon - 13.405).abs() < 1e-6);
    // City and region are the same word: it is not repeated.
    assert_eq!(name, "Berlin, Germany");
}

#[test]
fn parses_reallyfreegeoip() {
    let (lat, lon, name) = parse_service(svc("reallyfreegeoip.org"), REALLYFREE).unwrap();
    assert!((lat - 52.3676).abs() < 1e-6 && (lon - 4.9041).abs() < 1e-6);
    assert_eq!(name, "Amsterdam, North Holland, Netherlands");
}

#[test]
fn rejects_garbage() {
    for s in SERVICES.iter() {
        assert!(parse_service(s, "<html>captive portal</html>").is_err());
        assert!(parse_service(s, "{}").is_err());
    }
}

#[test]
fn os_location_wins_over_ip() {
    let cache = tmp("os");
    let http = Mock::new(&[("https://ipapi.co/json", IPAPI)]);
    let os = FakeOs(Ok((48.85, 2.35, Some("Paris".to_string()))));
    let r = resolve(Some(&os), Some(&http), &cache, 1000);
    assert_eq!(r.source, Source::Os);
    assert_eq!(r.name, "Paris");
    assert_eq!(r.describe(), "detected via FakeOS");
    assert!(http.called().is_empty(), "no IP lookup when the OS answered");
    assert_eq!(load_cache(&cache).unwrap(), r);
}

#[test]
fn os_fix_without_a_name_gets_coordinates() {
    let cache = tmp("osname");
    let os = FakeOs(Ok((-33.87, 151.21, None)));
    let r = resolve(Some(&os), None, &cache, 1);
    assert_eq!(r.name, "33.87\u{b0}S, 151.21\u{b0}E");
}

#[test]
fn falls_back_through_every_service_in_order() {
    let cache = tmp("chain");
    let os = FakeOs(Err("access denied".to_string()));
    // First three down or refusing; the fourth answers.
    let http = Mock::new(&[
        ("https://ipapi.co/json", IPAPI_LIMITED),
        ("https://ipwho.is/", IPWHO_FAIL),
        ("https://reallyfreegeoip.org/json/", REALLYFREE),
    ]);
    let r = resolve(Some(&os), Some(&http), &cache, 5);
    assert_eq!(r.source, Source::Ip);
    assert_eq!(r.detail, "reallyfreegeoip.org");
    assert_eq!(r.describe(), "detected via IP lookup (reallyfreegeoip.org)");
    assert_eq!(
        http.called(),
        vec![
            "https://ipapi.co/json",
            "https://ipwho.is/",
            "https://get.geojs.io/v1/ip/geo.json",
            "https://reallyfreegeoip.org/json/"
        ]
    );
}

#[test]
fn an_ip_answer_off_the_map_is_skipped() {
    let cache = tmp("bad");
    let bad = r#"{"latitude":123.0,"longitude":10.0,"city":"Nowhere"}"#;
    let http = Mock::new(&[("https://ipapi.co/json", bad), ("https://ipwho.is/", IPWHO)]);
    let r = resolve(None, Some(&http), &cache, 5);
    assert_eq!(r.detail, "ipwho.is");
}

#[test]
fn offline_uses_the_last_known_place() {
    let cache = tmp("last");
    let old = Resolved {
        lat: 40.0,
        lon: -74.0,
        name: "Somewhere".into(),
        source: Source::Ip,
        detail: "ipapi.co".into(),
        at: 100,
    };
    save_cache(&cache, &old);
    let http = Mock::new(&[]);
    let os = FakeOs(Err("no service".to_string()));
    let r = resolve(Some(&os), Some(&http), &cache, 999_999);
    assert_eq!(r.source, Source::Cache);
    assert_eq!((r.lat, r.lon), (40.0, -74.0));
    assert_eq!(r.at, 100, "the age of the detection is kept");
}

#[test]
fn nothing_at_all_gives_the_default() {
    let cache = tmp("none");
    let r = resolve(None, Some(&Mock::new(&[])), &cache, 1);
    assert_eq!(r.source, Source::Default);
    assert_eq!((r.lat, r.lon), (DEFAULT_LAT, DEFAULT_LON));
    assert!(!cache.exists(), "a default is not cached as a detection");
}

#[test]
fn manual_place_never_detects() {
    let cache = tmp("manual");
    save_cache(&cache, &Resolved::default_place());
    let r = effective(false, -33.87, 151.21, "Sydney", &cache);
    assert_eq!(r.source, Source::Manual);
    assert_eq!((r.lat, r.lon), (-33.87, 151.21));
    assert_eq!(r.describe(), "set by you");
    assert!(!refresh_due(false, None, 1_000_000), "don't detect means don't detect");
}

#[test]
fn auto_uses_the_cache_then_the_config_values() {
    let cache = tmp("auto");
    let r = effective(true, 10.0, 20.0, "Config", &cache);
    assert_eq!((r.source, r.lat), (Source::Default, 10.0));
    save_cache(&cache, &Resolved { lat: 52.0, lon: 5.0, name: "Utrecht".into(), source: Source::Ip, detail: "x".into(), at: 7 });
    let r = effective(true, 10.0, 20.0, "Config", &cache);
    assert_eq!((r.lat, r.name.as_str()), (52.0, "Utrecht"));
}

#[test]
fn refresh_is_daily() {
    let r = Resolved { at: 1000, ..Resolved::default_place() };
    assert!(refresh_due(true, None, 1000));
    assert!(!refresh_due(true, Some(&r), 1000 + 3600));
    assert!(refresh_due(true, Some(&r), 1000 + MAX_AGE_SECS));
    // A clock that went backwards refreshes rather than trusting the stamp.
    assert!(refresh_due(true, Some(&r), 10));
}

#[test]
fn cache_round_trip_and_garbage() {
    let cache = tmp("rt");
    let r = Resolved { lat: -33.87, lon: 151.21, name: "Sydney".into(), source: Source::Os, detail: "GeoClue".into(), at: 42 };
    save_cache(&cache, &r);
    assert_eq!(load_cache(&cache), Some(r));
    std::fs::write(&cache, "not json").unwrap();
    assert_eq!(load_cache(&cache), None);
    std::fs::write(&cache, r#"{"lat":999,"lon":0,"name":"x","source":"ip","detail":"","at":1}"#).unwrap();
    assert_eq!(load_cache(&cache), None, "an off-the-map cache is ignored");
}

#[test]
fn cache_holds_only_place_and_source() {
    let cache = tmp("privacy");
    let http = Mock::new(&[("https://ipapi.co/json", IPAPI)]);
    resolve(None, Some(&http), &cache, 1);
    let text = std::fs::read_to_string(&cache).unwrap();
    assert!(!text.contains("1.2.3.4"), "the IP address is never stored: {text}");
}

#[test]
fn place_search_parsing() {
    let body = r#"{"results":[
        {"id":1,"name":"Sydney","latitude":-33.86785,"longitude":151.20732,"country":"Australia","admin1":"New South Wales"},
        {"id":2,"name":"Sydney","latitude":46.1351,"longitude":-60.1831,"country":"Canada","admin1":"Nova Scotia"},
        {"id":3,"name":"Broken","latitude":999.0,"longitude":1.0}
    ],"generationtime_ms":0.5}"#;
    let p = parse_search(body).unwrap();
    assert_eq!(p.len(), 2);
    assert_eq!(p[0].name, "Sydney, New South Wales, Australia");
    assert!((p[0].lat + 33.86785).abs() < 1e-9);
    // No match is an empty list, not an error.
    assert_eq!(parse_search(r#"{"generationtime_ms":0.2}"#).unwrap(), vec![]);
    assert!(parse_search("oops").is_err());
}

#[test]
fn place_search_url_and_short_queries() {
    assert_eq!(
        search_url("S\u{e3}o Paulo"),
        "https://geocoding-api.open-meteo.com/v1/search?name=S%C3%A3o%20Paulo&count=8&language=en&format=json"
    );
    let http = Mock::new(&[]);
    assert!(search(&http, " a ").unwrap().is_empty());
    assert!(http.called().is_empty(), "a one-letter query sends nothing");
    assert!(search(&http, "Sydney").is_err(), "offline search reports the error");
}

/// The place found drives the seasons: Sydney has summer in December and
/// winter in July, while the northern default has it the other way round (D93 item 8).
#[test]
fn southern_hemisphere_seasons_follow_the_place() {
    let dec = world::Date::new(2026, 12, 15, 12);
    let jul = world::Date::new(2026, 7, 15, 12);
    let (sy_lat, _, _) = parse_service(svc("ipapi.co"), IPAPI).unwrap();
    assert_eq!(world::SeasonLook::at(&dec, sy_lat).name, "summer");
    assert_eq!(world::SeasonLook::at(&jul, sy_lat).name, "winter");
    assert_eq!(world::SeasonLook::at(&dec, DEFAULT_LAT).name, "winter");
    assert_eq!(world::SeasonLook::at(&jul, DEFAULT_LAT).name, "summer");
    // The sun: a December day is long in Sydney and short in Greenwich.
    let ms = 1_797_336_000_000i64; // 2026-12-15 12:00 UTC
    let len = |lat: f64, lon: f64| {
        let t = world::sun_times(ms, lat, lon);
        (t.set.unwrap() - t.rise.unwrap()).rem_euclid(86_400_000) as f64 / 3_600_000.0
    };
    assert!(len(sy_lat, 151.21) > 14.0, "Sydney December day");
    assert!(len(DEFAULT_LAT, DEFAULT_LON) < 10.0, "Greenwich December day");
}
