//! Cumulative per-phase wall-clock accounting for `full_index` diagnostics.
//!
//! A scope records its elapsed time and a call count into a global table keyed
//! by phase name. `dump()` emits one `PHASE_TIMER` tracing line per phase,
//! sorted by total time descending, so the dominant cost is the first row.
//!
//! Accumulation is always on — an atomic add per scope, cheap enough to leave
//! wired permanently. The cost lives in phases that run thousands of times
//! (the per-augment external-admission passes), where a single wall-clock
//! reading per call is lost in the work it measures.
//!
//! A phase that runs N times reports `calls=N` so a per-iteration pass is
//! visibly distinguished from a once-per-index one — the call count is the
//! signal that separates a constant-factor cost from a quadratic one.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use tracing::info;

/// One phase's running totals: summed nanoseconds and the number of scopes
/// that contributed. Stored behind a `Mutex<Vec<..>>` keyed by name rather than
/// a `HashMap` so the table stays allocation-stable across threads and the dump
/// order is insertion order before the sort.
struct PhaseEntry {
    name: &'static str,
    total_ns: AtomicU64,
    calls: AtomicU64,
}

static TABLE: Mutex<Vec<PhaseEntry>> = Mutex::new(Vec::new());

/// Add `elapsed` to the named phase's running total and bump its call count.
/// Creates the entry on first sight. The lock is held only long enough to find
/// or push the entry; the counters themselves are atomic so concurrent rayon
/// workers timing the same phase don't contend on the lock for the add.
fn record(name: &'static str, elapsed_ns: u64) {
    let mut table = TABLE.lock().expect("phase-timer table poisoned");
    if let Some(entry) = table.iter().find(|e| e.name == name) {
        entry.total_ns.fetch_add(elapsed_ns, Ordering::Relaxed);
        entry.calls.fetch_add(1, Ordering::Relaxed);
        return;
    }
    table.push(PhaseEntry {
        name,
        total_ns: AtomicU64::new(elapsed_ns),
        calls: AtomicU64::new(1),
    });
}

/// RAII scope: times from construction to drop, recording into `name`'s total.
/// Use via `let _t = phase_timer::scope("name");` at the top of the region to
/// measure — the drop at end-of-scope does the recording.
pub struct Scope {
    name: &'static str,
    start: Instant,
}

impl Scope {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            start: Instant::now(),
        }
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        record(self.name, self.start.elapsed().as_nanos() as u64);
    }
}

/// Open a timing scope. Equivalent to `Scope::new` but reads as a verb at the
/// call site: `let _t = phase_timer::scope("admit_reachable_externals");`.
pub fn scope(name: &'static str) -> Scope {
    Scope::new(name)
}

/// Emit one `PHASE_TIMER` line per recorded phase, sorted by total time
/// descending, then reset the table. Called once at the end of `full_index`
/// so a batch run reports each project independently rather than a running
/// sum across every project indexed in the process.
pub fn dump_and_reset() {
    let mut table = TABLE.lock().expect("phase-timer table poisoned");
    if table.is_empty() {
        return;
    }
    let mut rows: Vec<(&'static str, u64, u64)> = table
        .iter()
        .map(|e| {
            (
                e.name,
                e.total_ns.load(Ordering::Relaxed),
                e.calls.load(Ordering::Relaxed),
            )
        })
        .collect();
    rows.sort_by(|a, b| b.1.cmp(&a.1));
    for (name, total_ns, calls) in rows {
        let ms = total_ns / 1_000_000;
        let per_call_ms = if calls > 0 { ms / calls } else { 0 };
        info!(
            "PHASE_TIMER phase={name:<34} total={ms:>7} ms  calls={calls:>4}  per_call={per_call_ms:>6} ms"
        );
    }
    table.clear();
}

#[cfg(test)]
#[path = "phase_timer_tests.rs"]
mod tests;
