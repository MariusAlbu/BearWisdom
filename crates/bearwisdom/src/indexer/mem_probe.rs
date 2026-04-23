//! Per-phase memory sampling for `full_index` diagnostics.
//!
//! Emits `MEM_PROBE` tracing events with working-set + private bytes +
//! delta-from-prev-sample. Zero-cost when the `BEARWISDOM_MEM_PROBE` env var
//! is unset; cheap enough (one syscall) to leave wired permanently.
//!
//! Non-Windows platforms return zeros — good enough for now; we only run the
//! baseline on Windows.

use std::cell::Cell;
use std::sync::atomic::{AtomicU64, Ordering};
use tracing::info;

static PREV_WS: AtomicU64 = AtomicU64::new(0);
static PREV_PRIV: AtomicU64 = AtomicU64::new(0);

thread_local! {
    static ENABLED: Cell<Option<bool>> = Cell::new(None);
}

fn enabled() -> bool {
    ENABLED.with(|slot| {
        if let Some(v) = slot.get() { return v; }
        let v = std::env::var("BEARWISDOM_MEM_PROBE").is_ok();
        slot.set(Some(v));
        v
    })
}

/// Sample RSS + private bytes for the current process and emit a
/// `MEM_PROBE phase=<phase>` tracing line. Deltas are reported against the
/// previous sample so the log localizes allocation surges without the reader
/// having to subtract pairs by hand.
pub fn probe(phase: &str) {
    if !enabled() { return; }
    let (ws, priv_bytes) = read_process_memory();
    let prev_ws = PREV_WS.swap(ws, Ordering::Relaxed);
    let prev_priv = PREV_PRIV.swap(priv_bytes, Ordering::Relaxed);
    let ws_mb = (ws / (1024 * 1024)) as i64;
    let priv_mb = (priv_bytes / (1024 * 1024)) as i64;
    let dws = ws_mb - (prev_ws / (1024 * 1024)) as i64;
    let dpriv = priv_mb - (prev_priv / (1024 * 1024)) as i64;
    info!(
        "MEM_PROBE phase={phase:<28} ws={ws_mb:>6} MB (Δ{dws:+6}) priv={priv_mb:>6} MB (Δ{dpriv:+6})"
    );
}

#[cfg(windows)]
fn read_process_memory() -> (u64, u64) {
    use windows_sys::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;

    unsafe {
        let mut counters: PROCESS_MEMORY_COUNTERS_EX = std::mem::zeroed();
        let size = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
        let ok = GetProcessMemoryInfo(
            GetCurrentProcess(),
            (&mut counters as *mut PROCESS_MEMORY_COUNTERS_EX) as *mut PROCESS_MEMORY_COUNTERS,
            size,
        );
        if ok == 0 {
            return (0, 0);
        }
        (counters.WorkingSetSize as u64, counters.PrivateUsage as u64)
    }
}

#[cfg(not(windows))]
fn read_process_memory() -> (u64, u64) {
    (0, 0)
}
