// =============================================================================
// changeset_tests.rs — unit tests for the working-tree-aware GitDiff helpers
// =============================================================================

use super::*;

fn touch(rel: &str, lang: &'static str, root: &std::path::Path) -> WalkedFile {
    let abs = root.join(rel);
    if let Some(parent) = abs.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    if !abs.exists() {
        std::fs::write(&abs, "").unwrap();
    }
    WalkedFile {
        relative_path: rel.to_owned(),
        absolute_path: abs,
        language: lang,
    }
}

#[test]
fn deduplicate_drops_added_when_modified_present() {
    let tmp = std::env::temp_dir().join("bw-test-changeset-dedup-1");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();

    let mut cs = ChangeSet::default();
    cs.added.push(touch("src/lib.rs", "rust", &tmp));
    cs.modified.push(touch("src/lib.rs", "rust", &tmp));

    deduplicate_changeset(&mut cs);

    assert_eq!(cs.added.len(), 0, "added survived dedup: {:?}", cs.added);
    assert_eq!(cs.modified.len(), 1);
    assert_eq!(cs.modified[0].relative_path, "src/lib.rs");

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn deduplicate_drops_delete_when_path_is_live() {
    let tmp = std::env::temp_dir().join("bw-test-changeset-dedup-2");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();

    let mut cs = ChangeSet::default();
    cs.modified.push(touch("src/lib.rs", "rust", &tmp));
    cs.deleted.push("src/lib.rs".to_string());
    cs.deleted.push("src/gone.rs".to_string());

    deduplicate_changeset(&mut cs);

    assert_eq!(cs.deleted, vec!["src/gone.rs".to_string()]);
    assert_eq!(cs.modified.len(), 1);

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn deduplicate_collapses_repeats_within_a_bucket() {
    let tmp = std::env::temp_dir().join("bw-test-changeset-dedup-3");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();

    let mut cs = ChangeSet::default();
    cs.modified.push(touch("a.rs", "rust", &tmp));
    cs.modified.push(touch("a.rs", "rust", &tmp));
    cs.deleted.push("b.rs".to_string());
    cs.deleted.push("b.rs".to_string());

    deduplicate_changeset(&mut cs);

    assert_eq!(cs.modified.len(), 1, "modified bucket: {:?}", cs.modified);
    assert_eq!(cs.deleted.len(), 1, "deleted bucket: {:?}", cs.deleted);

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn apply_diff_line_parses_modified_added_deleted_and_typechange() {
    let tmp = std::env::temp_dir().join("bw-test-apply-diff-line");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::write(tmp.join("kept.rs"), "// kept").unwrap();
    std::fs::write(tmp.join("new.rs"), "// new").unwrap();

    let mut cs = ChangeSet::default();
    apply_diff_line("M\tkept.rs", &tmp, &mut cs);
    apply_diff_line("A\tnew.rs", &tmp, &mut cs);
    apply_diff_line("D\tgone.rs", &tmp, &mut cs);
    apply_diff_line("T\tkept.rs", &tmp, &mut cs);
    apply_diff_line("X\tunknown.rs", &tmp, &mut cs); // unknown — silently dropped
    apply_diff_line("", &tmp, &mut cs); // empty — silently dropped

    assert_eq!(cs.modified.len(), 2); // M + T both go to modified
    assert_eq!(cs.added.len(), 1);
    assert_eq!(cs.deleted, vec!["gone.rs".to_string()]);
    assert!(cs.added.iter().any(|w| w.relative_path == "new.rs"));

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn apply_diff_line_drops_files_with_unknown_language() {
    let tmp = std::env::temp_dir().join("bw-test-apply-diff-unknown-lang");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    // Some extension that walker::detect_language won't recognize.
    std::fs::write(tmp.join("data.bin"), "").unwrap();

    let mut cs = ChangeSet::default();
    apply_diff_line("M\tdata.bin", &tmp, &mut cs);

    // No source language → not eligible for indexing → dropped.
    assert!(cs.modified.is_empty());
    assert!(cs.added.is_empty());

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn apply_diff_line_drops_missing_files() {
    let tmp = std::env::temp_dir().join("bw-test-apply-diff-missing");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();

    let mut cs = ChangeSet::default();
    apply_diff_line("A\tnever_existed.rs", &tmp, &mut cs);

    // Race: file was added in diff output but isn't on disk anymore. Skip.
    assert!(cs.added.is_empty());

    let _ = std::fs::remove_dir_all(&tmp);
}

// ---------------------------------------------------------------------------
// Integration: working-tree pass picks up uncommitted modifications
// ---------------------------------------------------------------------------

/// Reproduces the bug: an indexed commit equal to HEAD with modified
/// working-tree files used to return an empty ChangeSet, leaving every
/// modified file's symbols stale. The fix runs the working-tree pass
/// regardless of commit equality so uncommitted edits flow through.
#[test]
fn working_tree_changes_are_detected_when_head_unchanged() {
    use std::process::Command;

    // Locate a git binary. If git isn't installed (CI image without it)
    // we treat the test as inconclusive — the bug we're testing only
    // manifests on git-backed projects anyway.
    let git_ok = Command::new("git")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !git_ok {
        eprintln!("working_tree_changes_are_detected_when_head_unchanged: git not available, skipping");
        return;
    }

    let tmp = std::env::temp_dir().join("bw-test-changeset-wt-pass");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();

    // Initialize git repo + initial commit with one tracked .rs file.
    let run = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .current_dir(&tmp)
            .output()
            .unwrap()
    };
    run(&["init", "-q", "-b", "main"]);
    run(&["config", "user.email", "test@example.com"]);
    run(&["config", "user.name", "Test"]);
    run(&["config", "commit.gpgsign", "false"]);
    std::fs::write(tmp.join("lib.rs"), "// initial content\n").unwrap();
    run(&["add", "lib.rs"]);
    let commit_out = run(&["commit", "-q", "-m", "initial"]);
    assert!(
        commit_out.status.success(),
        "git commit failed: {}",
        String::from_utf8_lossy(&commit_out.stderr)
    );
    let head = String::from_utf8_lossy(&run(&["rev-parse", "HEAD"]).stdout)
        .trim()
        .to_string();
    assert!(!head.is_empty());

    // Build a minimal BW DB through the public Database::open path so the
    // schema matches what the production code expects, then stamp
    // `indexed_commit` via the public `set_meta` helper.
    let db_path = tmp.join("index.db");
    let db = crate::db::Database::open(&db_path).unwrap();
    set_meta(&db, "indexed_commit", &head).unwrap();

    // Modify the tracked file in the working tree (no new commit — HEAD
    // is unchanged). This is the bug repro state.
    std::fs::write(tmp.join("lib.rs"), "// modified content\n").unwrap();

    let cs = git_diff(&db, &tmp).expect("git_diff should succeed");

    assert_eq!(
        cs.modified.len(),
        1,
        "expected 1 modified file from working tree, got: added={:?} modified={:?} deleted={:?}",
        cs.added.iter().map(|w| &w.relative_path).collect::<Vec<_>>(),
        cs.modified.iter().map(|w| &w.relative_path).collect::<Vec<_>>(),
        cs.deleted,
    );
    assert_eq!(cs.modified[0].relative_path, "lib.rs");
    assert!(cs.added.is_empty());
    assert!(cs.deleted.is_empty());

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn untracked_files_appear_as_added() {
    use std::process::Command;

    let git_ok = Command::new("git")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !git_ok {
        eprintln!("untracked_files_appear_as_added: git not available, skipping");
        return;
    }

    let tmp = std::env::temp_dir().join("bw-test-changeset-untracked");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();

    let run = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .current_dir(&tmp)
            .output()
            .unwrap()
    };
    run(&["init", "-q", "-b", "main"]);
    run(&["config", "user.email", "test@example.com"]);
    run(&["config", "user.name", "Test"]);
    run(&["config", "commit.gpgsign", "false"]);
    std::fs::write(tmp.join("seed.rs"), "// seed\n").unwrap();
    run(&["add", "seed.rs"]);
    run(&["commit", "-q", "-m", "seed"]);
    let head = String::from_utf8_lossy(&run(&["rev-parse", "HEAD"]).stdout)
        .trim()
        .to_string();

    let db_path = tmp.join("index.db");
    let db = crate::db::Database::open(&db_path).unwrap();
    set_meta(&db, "indexed_commit", &head).unwrap();

    // Drop a brand-new untracked source file alongside the seed.
    std::fs::write(tmp.join("brand_new.rs"), "// hello\n").unwrap();

    // Also drop a file matching .gitignore to confirm it's NOT picked up.
    std::fs::write(tmp.join(".gitignore"), "ignored.rs\n").unwrap();
    std::fs::write(tmp.join("ignored.rs"), "// should be skipped\n").unwrap();

    let cs = git_diff(&db, &tmp).expect("git_diff should succeed");

    let added_paths: Vec<&str> = cs
        .added
        .iter()
        .map(|w| w.relative_path.as_str())
        .collect();
    assert!(
        added_paths.contains(&"brand_new.rs"),
        "untracked source file missing from added; got {added_paths:?}"
    );
    assert!(
        !added_paths.contains(&"ignored.rs"),
        "gitignored file leaked into added: {added_paths:?}"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}
