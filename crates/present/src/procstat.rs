//! CPU and memory readings of this process: straight from /proc on Linux (no
//! libc, no crates), from the process-times and memory-counter calls on
//! Windows.

pub const USER_HZ: f64 = 100.0;

#[cfg(target_os = "linux")]
mod imp {
    /// Total user+system CPU ticks for this process. On Linux `USER_HZ` is 100.
    pub fn process_cpu_ticks() -> Option<u64> {
        let s = std::fs::read_to_string("/proc/self/stat").ok()?;
        let rest = s.rsplit_once(')')?.1;
        let f: Vec<&str> = rest.split_whitespace().collect();
        // Field 3 is `f[0]` (state); utime is field 14 -> f[11], stime 15 -> f[12].
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

    /// Peak resident set size (`VmHWM`), kB.
    pub fn rss_peak_kb() -> Option<u64> {
        status_kb("VmHWM:")
    }

    /// Current resident set size (`VmRSS`), kB.
    pub fn rss_kb() -> Option<u64> {
        status_kb("VmRSS:")
    }
}

#[cfg(windows)]
mod imp {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

    fn ft(f: FILETIME) -> u64 {
        (f.dwHighDateTime as u64) << 32 | f.dwLowDateTime as u64
    }

    /// User+kernel CPU in 10 ms ticks (so `USER_HZ` is 100 here as well).
    pub fn process_cpu_ticks() -> Option<u64> {
        // SAFETY: the out-parameters are plain structs we own.
        unsafe {
            let (mut c, mut e, mut k, mut u): (FILETIME, FILETIME, FILETIME, FILETIME) =
                (std::mem::zeroed(), std::mem::zeroed(), std::mem::zeroed(), std::mem::zeroed());
            if GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u) == 0 {
                return None;
            }
            // FILETIME is in 100 ns units: 100_000 of them make a 10 ms tick.
            Some((ft(k) + ft(u)) / 100_000)
        }
    }

    fn counters() -> Option<PROCESS_MEMORY_COUNTERS> {
        // SAFETY: `cb` is set to the structure's size as the call requires.
        unsafe {
            let mut m: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
            m.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
            (K32GetProcessMemoryInfo(GetCurrentProcess(), &mut m, m.cb) != 0).then_some(m)
        }
    }

    /// Peak working set, kB.
    pub fn rss_peak_kb() -> Option<u64> {
        counters().map(|m| m.PeakWorkingSetSize as u64 / 1024)
    }

    /// Current working set, kB.
    pub fn rss_kb() -> Option<u64> {
        counters().map(|m| m.WorkingSetSize as u64 / 1024)
    }
}

pub use imp::{process_cpu_ticks, rss_kb, rss_peak_kb};
