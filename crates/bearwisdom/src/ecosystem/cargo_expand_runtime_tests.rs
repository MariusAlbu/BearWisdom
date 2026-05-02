use super::*;
use std::fs;
use std::sync::Mutex;

// Tests in this module mutate a single process-wide env var. Cargo's
// default test runner spreads tests across threads, so without a guard
// they race. Hold the mutex for the duration of any test that touches
// `BEARWISDOM_CARGO_EXPAND`.
static ENV_GUARD: Mutex<()> = Mutex::new(());

#[test]
fn env_opt_in_recognises_truthy_values() {
    let _g = ENV_GUARD.lock().unwrap_or_else(|p| p.into_inner());
    let original = std::env::var(ENV_OPT_IN).ok();
    for value in &["1", "true", "TRUE", "yes"] {
        std::env::set_var(ENV_OPT_IN, value);
        assert!(env_opt_in_set(), "expected {value} to opt in");
    }
    for value in &["0", "false", "no", ""] {
        std::env::set_var(ENV_OPT_IN, value);
        assert!(!env_opt_in_set(), "{value} must NOT opt in");
    }
    match original {
        Some(v) => std::env::set_var(ENV_OPT_IN, v),
        None => std::env::remove_var(ENV_OPT_IN),
    }
}

#[test]
fn cache_freshness_false_when_output_missing() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("Cargo.toml"), "[package]\nname=\"x\"\n").unwrap();
    let expanded = dir.path().join("nonexistent.rs");
    assert!(!cache_is_fresh(dir.path(), &expanded));
}

#[test]
fn cache_freshness_false_when_cargo_toml_newer() {
    let dir = tempfile::tempdir().unwrap();
    let expanded = dir.path().join("expanded.rs");
    fs::write(&expanded, "// stub\n").unwrap();
    // Sleep then write Cargo.toml so its mtime is later.
    std::thread::sleep(std::time::Duration::from_millis(50));
    fs::write(dir.path().join("Cargo.toml"), "[package]\nname=\"x\"\n").unwrap();
    assert!(!cache_is_fresh(dir.path(), &expanded));
}

#[test]
fn cache_freshness_true_when_output_newer() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("Cargo.toml"), "[package]\nname=\"x\"\n").unwrap();
    fs::create_dir_all(dir.path().join("src")).unwrap();
    fs::write(dir.path().join("src/lib.rs"), "").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(50));
    let expanded = dir.path().join("expanded.rs");
    fs::write(&expanded, "// stub\n").unwrap();
    assert!(cache_is_fresh(dir.path(), &expanded));
}

#[test]
fn ensure_expanded_for_no_op_without_env() {
    let _g = ENV_GUARD.lock().unwrap_or_else(|p| p.into_inner());
    let original = std::env::var(ENV_OPT_IN).ok();
    std::env::remove_var(ENV_OPT_IN);
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("Cargo.toml"), "[package]\nname=\"x\"\n").unwrap();
    assert!(ensure_expanded_for(dir.path()).is_empty());
    if let Some(v) = original {
        std::env::set_var(ENV_OPT_IN, v);
    }
}
