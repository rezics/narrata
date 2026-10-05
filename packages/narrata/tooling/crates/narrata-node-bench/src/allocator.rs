//! Counts requested Rust heap bytes, excluding allocator metadata and working-set pages.

// Only this module implements the standard allocator boundary. System owns every returned
// pointer and receives the same pointer/layout (or successful realloc layout) on release.
#![allow(unsafe_code)]

use serde::Serialize;
use std::{
    alloc::{GlobalAlloc, Layout, System},
    sync::atomic::{AtomicUsize, Ordering::Relaxed},
    time::Instant,
};

pub struct Counting;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
static ALLOCATED: AtomicUsize = AtomicUsize::new(0);

fn allocated(size: usize) {
    ALLOCATED.fetch_add(size, Relaxed);
    let live = LIVE.fetch_add(size, Relaxed) + size;
    PEAK.fetch_max(live, Relaxed);
}

// SAFETY: no pointer is inspected or altered here. All allocator operations delegate to
// System with the caller's exact arguments, and counters change only after success.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: forwarded unchanged from GlobalAlloc's caller.
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            allocated(layout.size());
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Relaxed);
        // SAFETY: forwarded unchanged from GlobalAlloc's caller.
        unsafe { System.dealloc(pointer, layout) };
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        // SAFETY: forwarded unchanged from GlobalAlloc's caller.
        let replacement = unsafe { System.realloc(pointer, layout, size) };
        if !replacement.is_null() {
            LIVE.fetch_sub(layout.size(), Relaxed);
            allocated(size);
        }
        replacement
    }
}

#[derive(Debug, Serialize)]
pub struct Measurement {
    pub elapsed_us: f64,
    pub allocated_bytes: usize,
    pub retained_extra_bytes: usize,
    pub peak_extra_bytes: usize,
    pub peak_live_bytes: usize,
    pub live_bytes: usize,
}

pub struct Interval {
    started: Instant,
    live: usize,
    allocated: usize,
}

impl Interval {
    /// Single-threaded runner only; no measurement interval overlaps another.
    pub fn start() -> Self {
        let live = LIVE.load(Relaxed);
        PEAK.store(live, Relaxed);
        Self {
            started: Instant::now(),
            live,
            allocated: ALLOCATED.load(Relaxed),
        }
    }

    pub fn finish(self) -> Measurement {
        let live = LIVE.load(Relaxed);
        Measurement {
            elapsed_us: self.started.elapsed().as_secs_f64() * 1_000_000.0,
            allocated_bytes: ALLOCATED.load(Relaxed) - self.allocated,
            retained_extra_bytes: live.saturating_sub(self.live),
            peak_extra_bytes: PEAK.load(Relaxed).saturating_sub(self.live),
            peak_live_bytes: PEAK.load(Relaxed),
            live_bytes: live,
        }
    }
}
