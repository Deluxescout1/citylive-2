//! B101: a sampling profiler for hosts where `perf` is not allowed.
//!
//! `CITYLIVE_SAMPLE=<file>` arms a 1 kHz `SIGPROF` timer (CPU time of the
//! process); the handler stores the interrupted instruction pointer, and at exit
//! the addresses are written to `<file>`, one hex number per line, relative to
//! the executable's load address. `tools/b101-perf/symbolize.sh` turns them into
//! a ranked table with `addr2line` (build with `CARGO_PROFILE_RELEASE_STRIP=none
//! CARGO_PROFILE_RELEASE_DEBUG=1` for names and lines). Off unless the variable
//! is set; nothing runs and nothing is allocated otherwise.

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod imp {
    use std::sync::atomic::{AtomicUsize, Ordering};

    const CAP: usize = 200_000;
    static N: AtomicUsize = AtomicUsize::new(0);
    static mut PCS: [usize; CAP] = [0; CAP];

    extern "C" fn on_prof(_sig: libc::c_int, _info: *mut libc::siginfo_t, ctx: *mut libc::c_void) {
        // SAFETY: the kernel passes a valid ucontext_t to an SA_SIGINFO handler.
        let pc = unsafe {
            let uc = &*(ctx as *const libc::ucontext_t);
            uc.uc_mcontext.gregs[libc::REG_RIP as usize] as usize
        };
        let i = N.fetch_add(1, Ordering::Relaxed);
        if i < CAP {
            // SAFETY: each index is claimed once by the fetch_add above.
            unsafe { (*std::ptr::addr_of_mut!(PCS))[i] = pc };
        }
    }

    pub fn arm() {
        // SAFETY: plain sigaction/setitimer calls with a handler that only
        // touches atomics and a pre-sized static array.
        unsafe {
            let mut sa: libc::sigaction = std::mem::zeroed();
            sa.sa_sigaction = on_prof as usize;
            sa.sa_flags = libc::SA_SIGINFO | libc::SA_RESTART;
            libc::sigemptyset(&mut sa.sa_mask);
            libc::sigaction(libc::SIGPROF, &sa, std::ptr::null_mut());
            let it = libc::itimerval {
                it_interval: libc::timeval { tv_sec: 0, tv_usec: 1000 },
                it_value: libc::timeval { tv_sec: 0, tv_usec: 1000 },
            };
            libc::setitimer(libc::ITIMER_PROF, &it, std::ptr::null_mut());
        }
    }

    pub fn dump(path: &str) {
        // SAFETY: stop the timer before reading what the handler wrote.
        unsafe {
            let it: libc::itimerval = std::mem::zeroed();
            libc::setitimer(libc::ITIMER_PROF, &it, std::ptr::null_mut());
        }
        // The executable's load address (PIE), from the first mapping of it.
        let exe = std::fs::read_link("/proc/self/exe").ok();
        let base = std::fs::read_to_string("/proc/self/maps")
            .ok()
            .and_then(|m| {
                m.lines()
                    .find(|l| exe.as_ref().is_some_and(|e| l.ends_with(e.to_string_lossy().as_ref())))
                    .and_then(|l| usize::from_str_radix(l.split('-').next()?, 16).ok())
            })
            .unwrap_or(0);
        let n = N.load(Ordering::Relaxed).min(CAP);
        let mut out = String::new();
        for i in 0..n {
            // SAFETY: the timer is stopped; indices below n were written.
            let pc = unsafe { (*std::ptr::addr_of!(PCS))[i] };
            out.push_str(&format!("{:x}\n", pc.wrapping_sub(base)));
        }
        let _ = std::fs::write(path, out);
        eprintln!("sampler: {n} samples -> {path}");
    }
}

/// The file `CITYLIVE_SAMPLE` names, if any.
pub fn path_from_env() -> Option<String> {
    std::env::var("CITYLIVE_SAMPLE").ok()
}

/// Start the timer (call once, after the start-up work: the city build is not
/// what a steady frame costs).
pub fn arm() {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    imp::arm();
}

/// Write the samples (if armed).
pub fn dump(path: &Option<String>) {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    if let Some(p) = path {
        imp::dump(p);
    }
    #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
    let _ = path;
}
