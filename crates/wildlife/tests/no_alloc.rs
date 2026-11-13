//! B22 test: no allocation growth over 10 000 updates (plus 333 draws).
//!
//! Own integration-test binary so a counting `#[global_allocator]` only sees
//! this test's traffic (same pattern as `weather_fx::tests/no_alloc`).

use std::alloc::{Layout, System};
use std::sync::atomic::{AtomicU64, Ordering};

use map::{world_for_viewport, generate};
use render::Framebuffer;
use wildlife::Wildlife;

#[global_allocator]
static ALLOC: Counter = Counter;

static COUNT: AtomicU64 = AtomicU64::new(0);

struct Counter;

unsafe impl std::alloc::GlobalAlloc for Counter {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        COUNT.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        COUNT.fetch_add(1, Ordering::Relaxed);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[test]
fn no_allocation_growth_over_10000_updates() {
    let vp = world_for_viewport(960, 540);
    let map = generate(7, vp.w, vp.h);
    let mut fb = Framebuffer::new(960, 540);
    let mut wl = Wildlife::new(0xBEEF, &map);

    // Noon: the full day population is live, flocks cross, perchers cycle,
    // fish jump — the busiest mix the layer has.
    // Warm up (first frames may touch lazy state).
    for i in 0..100 {
        wl.update(1.0 / 30.0, &map, &[], 12.0);
        if i % 30 == 0 {
            wl.draw(&mut fb, &vp, None);
        }
    }

    let before = COUNT.load(Ordering::Relaxed);
    let mut draws = 0u32;
    for i in 0..10_000 {
        wl.update(1.0 / 30.0, &map, &[], 12.0);
        if i % 30 == 0 {
            wl.draw(&mut fb, &vp, None); // 333 draws
            draws += 1;
        }
    }
    let after = COUNT.load(Ordering::Relaxed);

    assert_eq!(
        before, after,
        "the pools leaked {draws} allocations over 10 000 updates + {draws} draws"
    );
    assert!(wl.is_active(), "at noon the wildlife must be live");
    assert!(wl.active_count() >= 30, "a day-8 world should have ~60 animals: {}", wl.active_count());
}
