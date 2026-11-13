//! Live feeds smoke run (B44): one fetch per world feed, each printed —
//! flights, aurora Kp, the ISS, and the NFL scoreboard (+ Bills gameday).
//! Graceful offline: a dead feed just says so and prints nothing for it.
//!
//! ```text
//! cargo run --release -p world --features net --example feeds_live
//! cargo run --release -p world --features net --example feeds_live -- 51.48 0.0
//! ```

#[cfg(not(feature = "net"))]
fn main() {
    eprintln!("this example needs --features net");
}

#[cfg(feature = "net")]
fn main() {
    let mut args = std::env::args().skip(1);
    let lat = args
        .next()
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(world::DEFAULT_LATLON.0);
    let lon = args
        .next()
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(world::DEFAULT_LATLON.1);
    let client = world::UreqClient::new();
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let cache = |name: &str| std::env::temp_dir().join(format!("citylive-{name}"));
    let feeds = world::Feeds::default();
    println!("viewer: ({lat}, {lon})  feeds: {feeds:?}");

    // ---- FLIGHTS (airplanes.live, no key) ----
    if feeds.flights {
        let url = world::flights::url_for(lat, lon, world::flights::DEFAULT_RADIUS_NM);
        println!("\n[flights] {url}");
        let flights =
            world::flights::fetch(&client, lat, lon, world::flights::DEFAULT_RADIUS_NM, &cache("airplanes-cache.json"));
        if flights.is_empty() {
            println!("  (offline or no aircraft in range — nothing to show)");
        }
        for f in &flights {
            println!(
                "  {cs:<10} {cat} {plane:<6} alt {alt:>6.0} m  track {track:>5.0}°  {spd:>5.1} kt  {dist:>7.0} m away (E {e:>7.0} / N {n:>7.0})",
                cs = f.callsign,
                cat = f.category,
                plane = f.plane,
                alt = f.altitude_m,
                track = f.heading,
                spd = f.speed_kt,
                dist = f.distance_m,
                e = f.east_m,
                n = f.north_m
            );
        }
    }

    // ---- AURORA (NOAA SWPC Kp) ----
    if feeds.aurora {
        println!("\n[aurora] {}", world::aurora::url_for());
        let a = world::aurora::fetch(&client, &cache("kp-cache.json"));
        let level_at_lon = world::aurora::Aurora::level_for(a.kp, lat, lon);
        println!(
            "  Kp {kp:.1} ({source:?})  visible_at(lat) {va:.2}  level at ({lat},{lon}) {ll:.2}  active {active}",
            kp = a.kp,
            source = a.source,
            va = a.visible_at(lat),
            ll = level_at_lon,
            active = a.active_at(lat)
        );
    }

    // ---- ISS (open-notify) ----
    if feeds.iss {
        println!("\n[iss] {}", world::iss::url_for());
        let iss = world::iss::fetch(&client, now_ms, &cache("iss-cache.json"));
        match &iss {
            Some(i) => {
                let look = i.look(lat, lon);
                println!(
                    "  sub-point ({ila:.4}, {ilo:.4})  from viewer: alt {alt:.1}° az {az:.1}°  overhead pass: {overhead}",
                    ila = i.lat,
                    ilo = i.lon,
                    alt = look.alt,
                    az = look.az,
                    overhead = i.overhead_pass(lat, lon)
                );
            }
            None => println!("  (feed unreachable — a scripted pass stands in)"),
        }
    }

    // ---- SPORTS (ESPN NFL scoreboard) ----
    if feeds.sports {
        println!("\n[sports] {}", world::sports::url_for(world::sports::LEAGUE_NFL));
        let sb = world::sports::fetch(&client, &cache("espn-cache.json"));
        match &sb {
            Some(sb) => {
                println!("  {} game(s) today:", sb.games.len());
                for g in &sb.games {
                    let state = match g.state {
                        world::GameState::Live => "LIVE",
                        world::GameState::Upcoming => "upcoming",
                        world::GameState::Final => "final",
                        world::GameState::Other => "—",
                    };
                    println!(
                        "  {away:<28} {ascore:>3} – {hscore:<3} {home:<28} {state}",
                        away = g.away,
                        ascore = g.away_score,
                        hscore = g.home_score,
                        home = g.home,
                        state = state,
                    );
                }
                match &sb.bills_game() {
                    Some(b) => println!(
                        "  Bills gameday: yes — {us} vs {opp} {them} ({state:?}){detail}",
                        us = b.us,
                        opp = b.opp,
                        them = b.them,
                        state = b.state,
                        detail = b.detail.as_deref().unwrap_or(""),
                    ),
                    None => println!("  Bills gameday: no"),
                }
            }
            None => println!("  (feed unreachable — a fake game plays instead)"),
        }
    }
}
