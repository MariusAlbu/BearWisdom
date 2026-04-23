// =============================================================================
// panic_hook — fail-fast panic handling for the indexer pipeline
//
// Why this exists
// ---------------
// The streaming-parse pipeline in `indexer::full` spawns rayon workers that
// feed a bounded `mpsc::sync_channel`, and drains it on the main thread.  If
// the main thread panics mid-drain, Rust's default panic handler begins
// unwinding.  The scope's drop impl then waits for the spawned parser
// thread — which is parked on `rayon::ThreadPool::install` and may itself
// be waiting on the now-dropped channel's receiver end.  On Windows this
// has been observed to result in the worker thread spinning on SEH-wait
// waits indefinitely; we've seen 56-minute hangs with no CPU activity and
// no output, where the process was technically alive but making no
// progress (repro: c-redis/src/keymeta.c, a UTF-8 slice through a
// box-drawing glyph triggered the panic, and the WAL filled up with an
// uncommitted transaction that nothing ever rolled back).
//
// Hangs that look like hangs are bugs, but hangs that look like hangs
// because a panic was silenced are *worse* — they hide the real cause
// behind a "something's stuck" symptom.  This hook converts every panic
// into an immediate process exit with a stable exit code, so the real
// message surfaces and the harness (CI, the user's shell, bw-bench's
// timeout) can observe a clean failure.
//
// Install this at the start of `main()` in every bw binary.  Library
// callers that embed bearwisdom (AlphaT, Lynx) get defense-in-depth from
// the pipeline-local `catch_unwind` guards in `indexer::full`.
// =============================================================================

use std::io::Write;
use std::sync::Once;

const BW_PANIC_EXIT_CODE: i32 = 101;

static INSTALL_ONCE: Once = Once::new();

/// Install a process-wide panic hook that prints a short panic summary to
/// stderr and calls `std::process::exit(101)` immediately.  Safe to call
/// multiple times — subsequent calls are no-ops.
///
/// Preserves the previous hook's message via `panic::take_hook`; the hook
/// runs `exit(..)` after, bypassing any unwinding.  Tests that expect
/// `should_panic` still run — this hook is only installed from binaries,
/// not from the library's test harness.
pub fn install_fail_fast_panic_hook() {
    INSTALL_ONCE.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            // Let the default hook format the panic message so we don't
            // lose the location + payload.
            previous(info);
            // Flush explicitly — the process is about to die and any
            // half-written stderr would otherwise be lost.
            let _ = std::io::stderr().flush();
            let _ = std::io::stdout().flush();
            std::process::exit(BW_PANIC_EXIT_CODE);
        }));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_is_idempotent() {
        // Two back-to-back calls should not double-install, panic, or leak.
        install_fail_fast_panic_hook();
        install_fail_fast_panic_hook();
    }
}
