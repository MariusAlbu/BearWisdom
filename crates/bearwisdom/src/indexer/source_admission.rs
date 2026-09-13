// =============================================================================
// indexer/source_admission.rs — is this source admitted as project code?
//
// Two questions, both answered by the owning language plugin: is the file
// machine-generated, and is it a vendored copy of a dependency. Languages
// without an admission policy fail closed (not generated, not vendored).
// Both scanners read file content, so each answer is delivered through a
// panic guard that degrades to "not admitted-out" instead of unwinding.
// =============================================================================

use tracing::warn;

/// Whether the owning language classifies `content` as generated source.
pub(crate) fn is_generated_source_file(language: &str, content: &str) -> bool {
    crate::languages::default_registry()
        .get(language)
        .is_generated_source_file(content)
}

/// Whether the owning language classifies the file as a vendored copy of a
/// dependency rather than project source.
pub(crate) fn is_vendored_source_file(language: &str, path: &str, content: &str) -> bool {
    crate::languages::default_registry()
        .get(language)
        .is_vendored_source_file(path, content)
}

/// [`is_generated_source_file`] behind a panic guard. A scanner that panics
/// yields `false` and a warning naming `path`.
pub(crate) fn is_generated_guarded(language: &str, path: &str, content: &str) -> bool {
    guarded(
        || is_generated_source_file(language, content),
        "is_generated_source_file",
        path,
    )
}

/// [`is_vendored_source_file`] behind a panic guard. A scanner that panics
/// yields `false` and a warning naming `path`.
pub(crate) fn is_vendored_guarded(language: &str, path: &str, content: &str) -> bool {
    guarded(
        || is_vendored_source_file(language, path, content),
        "is_vendored_source_file",
        path,
    )
}

/// Run `scan` so a panic inside it becomes `false` plus a warning, never an
/// unwind. The parse pipeline hands results across channel boundaries, so an
/// escaping panic strands workers against a vanished receiver.
fn guarded(scan: impl FnOnce() -> bool, scanner: &str, path: &str) -> bool {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(scan)) {
        Ok(flag) => flag,
        Err(payload) => {
            let msg = panic_message(payload.as_ref());
            warn!("{scanner} panicked on {path}: {msg} — treating as not admitted-out");
            false
        }
    }
}

/// A short human-readable description of a `catch_unwind` payload, so a
/// panicking scanner produces a legible warning instead of `<opaque Any>`.
fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&'static str>() {
        return (*s).to_string();
    }
    if let Some(s) = payload.downcast_ref::<String>() {
        return s.clone();
    }
    "<non-string panic payload>".to_string()
}
