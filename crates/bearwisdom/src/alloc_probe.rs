// =============================================================================
// alloc_probe — global-allocator wrapper that logs oversize allocations
//
// When `BEARWISDOM_ALLOC_PROBE_MB` is set, every allocation whose requested
// size is >= that many MiB prints a one-line banner plus a full Rust
// backtrace to stderr BEFORE forwarding to the real allocator. The request
// proceeds regardless — we just observe the call site.
//
// Intended as a one-shot diagnostic for the indexer's 768 MiB allocation
// failure. Leave the wrapper installed in release builds; the threshold
// stays at zero (disabled) unless the env var is set, so overhead is a
// single relaxed atomic load per alloc.
//
// Install at the binary level:
//
//   #[global_allocator]
//   static ALLOCATOR: bearwisdom::alloc_probe::ProbingAllocator =
//       bearwisdom::alloc_probe::ProbingAllocator;
//
//   fn main() {
//       bearwisdom::alloc_probe::install_from_env();
//       ...
//   }
// =============================================================================

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Threshold in bytes. 0 = probe disabled (no-op path, one relaxed load).
static THRESHOLD_BYTES: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    /// Re-entry guard. `Backtrace::force_capture` and `eprintln!` both
    /// allocate; without this flag the probe would recurse on every frame.
    static INSIDE_PROBE: Cell<bool> = const { Cell::new(false) };
}

pub struct ProbingAllocator;

unsafe impl GlobalAlloc for ProbingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        maybe_report("alloc", layout.size(), 0);
        System.alloc(layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout)
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        maybe_report("alloc_zeroed", layout.size(), 0);
        System.alloc_zeroed(layout)
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        maybe_report("realloc", new_size, layout.size());
        System.realloc(ptr, layout, new_size)
    }
}

#[inline]
fn maybe_report(kind: &str, size: usize, old_size: usize) {
    let threshold = THRESHOLD_BYTES.load(Ordering::Relaxed);
    if threshold == 0 || size < threshold {
        return;
    }
    // Re-entry guard: reporting allocates; skip nested events.
    let should_report = INSIDE_PROBE.with(|slot| {
        if slot.get() {
            false
        } else {
            slot.set(true);
            true
        }
    });
    if !should_report {
        return;
    }
    report(kind, size, old_size);
    INSIDE_PROBE.with(|slot| slot.set(false));
}

#[cold]
fn report(kind: &str, size: usize, old_size: usize) {
    let bt = std::backtrace::Backtrace::force_capture();
    let mb = size / (1024 * 1024);
    let stderr = std::io::stderr();
    let mut out = stderr.lock();
    let _ = writeln!(
        out,
        "[alloc_probe] BIG {kind}: {size} bytes ({mb} MiB) old={old_size}"
    );
    let _ = writeln!(out, "{bt}");
    let _ = out.flush();
}

/// Set the reporting threshold in MiB. A value of 0 disables probing.
pub fn set_threshold_mb(mb: usize) {
    THRESHOLD_BYTES.store(mb.saturating_mul(1024 * 1024), Ordering::Relaxed);
}

/// Read `BEARWISDOM_ALLOC_PROBE_MB` and install the threshold.
/// Unset or unparseable = probe stays disabled.
pub fn install_from_env() {
    let Ok(raw) = std::env::var("BEARWISDOM_ALLOC_PROBE_MB") else {
        return;
    };
    let Ok(mb) = raw.trim().parse::<usize>() else {
        eprintln!(
            "[alloc_probe] ignoring BEARWISDOM_ALLOC_PROBE_MB={raw:?} — not a non-negative integer"
        );
        return;
    };
    set_threshold_mb(mb);
    if mb > 0 {
        eprintln!("[alloc_probe] enabled — reporting allocs >= {mb} MiB");
    }
}
