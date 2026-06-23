// =============================================================================
// engine/trace.rs — zero-cost-when-off per-ref resolution trace
//
// Global flag `TRACE_ACTIVE` gates all instrumentation with a single relaxed
// atomic load. When false, every call site returns immediately — no heap
// allocation, no format, no thread-local read. When true (set only by the
// `bw why-unresolved` command before a full index run), the matched file/line
// combination installs a per-thread collector on the rayon worker resolving
// that ref and accumulates diagnostic lines. Collected traces are pushed to a
// process-global `Mutex<Vec<TracedRef>>` so the CLI can drain them after the
// index returns.
// =============================================================================

use std::cell::RefCell;
use std::fmt;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};

/// Gates ALL trace instrumentation. `false` in normal operation.
/// Instrumented call sites check this first with `Ordering::Relaxed` —
/// one atomic load, zero cost, no branch misprediction in the common case.
pub static TRACE_ACTIVE: AtomicBool = AtomicBool::new(false);

// ---------------------------------------------------------------------------
// Per-thread collector
// ---------------------------------------------------------------------------

struct TraceCollector {
    lines: Vec<String>,
}

thread_local! {
    static TL: RefCell<Option<TraceCollector>> = const { RefCell::new(None) };
}

/// Emit one trace line to the thread-local collector, when one is active.
///
/// Guard: checks `TRACE_ACTIVE` first (relaxed load) — zero cost when off.
pub fn trace(args: fmt::Arguments<'_>) {
    if !TRACE_ACTIVE.load(Ordering::Relaxed) {
        return;
    }
    TL.with(|t| {
        if let Some(c) = t.borrow_mut().as_mut() {
            c.lines.push(fmt::format(args));
        }
    });
}

/// Emit a formatted trace line. Expands to a `trace()` call with format args.
#[macro_export]
macro_rules! tracef {
    ($($arg:tt)*) => {
        $crate::indexer::resolve::engine::trace::trace(format_args!($($arg)*))
    };
}

// ---------------------------------------------------------------------------
// Filter — file/line/target to match
// ---------------------------------------------------------------------------

struct TraceFilter {
    file_suffix: String,
    line: u32,
    target: String,
}

static TRACE_FILTER: Mutex<Option<TraceFilter>> = Mutex::new(None);

/// Set the file/line/target filter before activating trace.
/// `file_suffix` is matched with `str::ends_with` against the ParsedFile path.
/// `target` is empty to match any ref on that line.
pub fn set_filter(file_suffix: String, line: u32, target: String) {
    if let Ok(mut f) = TRACE_FILTER.lock() {
        *f = Some(TraceFilter { file_suffix, line, target });
    }
}

/// Clear the filter after the index run completes.
pub fn clear_filter() {
    if let Ok(mut f) = TRACE_FILTER.lock() {
        *f = None;
    }
}

/// Returns `(file_suffix, line, target)` when a filter is installed, else `None`.
pub fn get_filter() -> Option<(String, u32, String)> {
    TRACE_FILTER
        .lock()
        .ok()?
        .as_ref()
        .map(|f| (f.file_suffix.clone(), f.line, f.target.clone()))
}

// ---------------------------------------------------------------------------
// Per-ref lifecycle
// ---------------------------------------------------------------------------

/// Install a per-thread collector on the current thread. Call before resolving
/// the matched ref. No-op when `TRACE_ACTIVE` is false.
pub fn begin_ref() {
    if !TRACE_ACTIVE.load(Ordering::Relaxed) {
        return;
    }
    TL.with(|t| {
        *t.borrow_mut() = Some(TraceCollector { lines: Vec::new() });
    });
}

/// Take and return the collected trace lines, clearing the thread-local.
/// Returns an empty vec when no collector was installed or it was already taken.
pub fn take_ref() -> Vec<String> {
    TL.with(|t| t.borrow_mut().take().map(|c| c.lines).unwrap_or_default())
}

// ---------------------------------------------------------------------------
// Global / activation
// ---------------------------------------------------------------------------

/// Activate global tracing. Call before the resolve pass starts.
pub fn activate() {
    TRACE_ACTIVE.store(true, Ordering::SeqCst);
}

/// Deactivate global tracing. Call after the resolve pass completes.
pub fn deactivate() {
    TRACE_ACTIVE.store(false, Ordering::SeqCst);
}

// ---------------------------------------------------------------------------
// Collected trace output
// ---------------------------------------------------------------------------

/// One traced ref: the file, line, target name, and the trace lines collected
/// during its resolution pass.
#[derive(Debug)]
pub struct TracedRef {
    pub file: String,
    pub line: u32,
    pub target: String,
    pub trace_lines: Vec<String>,
}

static COLLECTED: Mutex<Vec<TracedRef>> = Mutex::new(Vec::new());

/// Push a completed `TracedRef` into the global collection.
/// Only called when `TRACE_ACTIVE` is true and trace lines were collected.
pub fn push_traced(tr: TracedRef) {
    if let Ok(mut v) = COLLECTED.lock() {
        v.push(tr);
    }
}

/// Drain all collected traces, clearing the global buffer.
pub fn drain_collected() -> Vec<TracedRef> {
    COLLECTED
        .lock()
        .map(|mut v| v.drain(..).collect())
        .unwrap_or_default()
}
