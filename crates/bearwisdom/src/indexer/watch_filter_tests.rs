use std::path::PathBuf;

use notify::event::{CreateKind, ModifyKind};
use notify::{Event, EventKind};

use super::WatchFilter;
use crate::indexer::changeset::ChangeKind;

fn root() -> PathBuf {
    let dir = std::env::temp_dir().join("bw-watch-filter-tests");
    std::fs::create_dir_all(&dir).expect("temp root");
    dir
}

fn created(root: &PathBuf, rel: &str) -> Event {
    Event::new(EventKind::Create(CreateKind::File)).add_path(root.join(rel))
}

fn modified(root: &PathBuf, rel: &str) -> Event {
    Event::new(EventKind::Modify(ModifyKind::Any)).add_path(root.join(rel))
}

#[test]
fn source_file_under_the_root_is_a_change() {
    let root = root();
    let filter = WatchFilter::new(root.clone());
    let changes = filter.changes(&[modified(&root, "src/lib.rs")]);
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].relative_path, "src/lib.rs");
    assert_eq!(changes[0].change_kind, ChangeKind::Modified);
}

/// A worktree checkout under `.claude/` is a full repo copy the walk never
/// visits; its files must not enter the index through creation events either.
#[test]
fn file_under_an_excluded_directory_is_dropped() {
    let root = root();
    let filter = WatchFilter::new(root.clone());
    let changes = filter.changes(&[
        created(&root, ".claude/worktrees/agent-x/crates/bw/src/lib.rs"),
        created(&root, "node_modules/pkg/index.js"),
    ]);
    assert!(changes.is_empty(), "got {changes:?}");
}

#[test]
fn same_path_across_events_is_reported_once() {
    let root = root();
    let filter = WatchFilter::new(root.clone());
    let changes = filter.changes(&[created(&root, "src/a.ts"), modified(&root, "src/a.ts")]);
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].change_kind, ChangeKind::Created);
}

#[test]
fn non_source_extension_and_foreign_path_are_dropped() {
    let root = root();
    let filter = WatchFilter::new(root.clone());
    let outside = Event::new(EventKind::Create(CreateKind::File))
        .add_path(PathBuf::from("/elsewhere/src/lib.rs"));
    let changes = filter.changes(&[created(&root, "assets/logo.png"), outside]);
    assert!(changes.is_empty(), "got {changes:?}");
}
