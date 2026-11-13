//! The brief's "no allocation growth over 10 000 updates" test.
//!
//! Lives in its own integration-test binary (its own process) so a
//! counting `#[global_allocator]` only sees this test's traffic.

use std::alloc::{Layout, System};
use std::sync::atomic::{AtomicU64, Ordering};

use weather_fx::Fx;
use world::Weather;

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

fn heavy() -> Weather {
    Weather {
        code: 65,
        precip: 3.0,
        cloud: 100.0,
        wind: 30.0,
        gust: 45.0,
        wind_dir: Some(250.0),
        ..Default::default()
    }
}

#[test]
fn no_allocation_growth_over_10000_updates() {
    let mut fx = Fx::new(0xC17E_15E7, 960, 540);
    let w = heavy();
    let mut fb = alloc::frame::new_frame();

    // Warm up: first frame under heavy rain spawns the active prefixes.
    for _ in 0..100 {
        fx.update(1.0 / 30.0, &w);
        fx.draw(&mut fb);
    }

    let before = COUNT.load(Ordering::Relaxed);
    let mut drawn = 0u32;
    for i in 0..10_000 {
        fx.update(1.0 / 30.0, &w);
        if i % 30 == 0 {
            fx.draw(&mut fb); // drawing must not allocate either (333 draws)
            drawn += 1;
        }
    }
    let after = COUNT.load(Ordering::Relaxed);

    assert_eq!(
        before, after,
        "the pool leaked {drawn} allocations over 10 000 updates + 333 draws"
    );
    assert!(fx.is_active(), "heavy rain must be active");
}

#[test]
fn intensity_changes_do_not_reallocate() {
    let light = Weather {
        code: 61,
        precip: 0.3,
        cloud: 60.0,
        ..Default::default()
    };
    let storm = Weather {
        code: 96,
        precip: 4.0,
        cloud: 100.0,
        wind: 35.0,
        wind_dir: Some(230.0),
        ..Default::default()
    };
    let clear: Weather = Default::default();

    let mut fx = Fx::new(7, 960, 540);
    for _ in 0..50 {
        fx.update(1.0 / 30.0, &light);
    }
    for _ in 0..50 {
        fx.update(1.0 / 30.0, &storm); // pool grows to full
    }
    for _ in 0..50 {
        fx.update(1.0 / 30.0, &clear); // pool empties
    }
    for _ in 0..50 {
        fx.update(1.0 / 30.0, &storm); // and refills
    }
    let before = COUNT.load(Ordering::Relaxed);
    for i in 0..1_000 {
        let w = if i % 30 == 0 { &light } else if i % 30 == 15 { &storm } else { &clear };
        fx.update(1.0 / 30.0, w);
    }
    let after = COUNT.load(Ordering::Relaxed);
    assert_eq!(before, after, "weather regime changes must not allocate");
}

mod alloc {
    pub mod frame {
        use render::Framebuffer;
        /// One frame's worth of allocation for the test's buffer.
        pub fn new_frame() -> Framebuffer {
            Framebuffer::new(960, 540)
        }
    }
}
