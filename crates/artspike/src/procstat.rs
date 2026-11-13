//! CPU and memory readings straight from /proc (same as the M0 spike).

/// Total user+system CPU ticks for this process. On Linux USER_HZ is 100.
pub fn process_cpu_ticks() -> Option<u64> {
    let s = std::fs::read_to_string("/proc/self/stat").ok()?;
    let rest = s.rsplit_once(')')?.1;
    let f: Vec<&str> = rest.split_whitespace().collect();
    let ut: u64 = f.get(11)?.parse().ok()?;
    let st: u64 = f.get(12)?.parse().ok()?;
    Some(ut + st)
}

fn status_kb(field: &str) -> Option<u64> {
    let s = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in s.lines() {
        if let Some(v) = line.strip_prefix(field) {
            return v.split_whitespace().next()?.parse().ok();
        }
    }
    None
}

/// Peak resident set size (VmHWM), kB.
pub fn rss_peak_kb() -> Option<u64> {
    status_kb("VmHWM:")
}

/// Current resident set size (VmRSS), kB.
pub fn rss_kb() -> Option<u64> {
    status_kb("VmRSS:")
}

pub const USER_HZ: f64 = 100.0;
