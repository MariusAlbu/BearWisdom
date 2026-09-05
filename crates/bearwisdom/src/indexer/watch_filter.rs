// =============================================================================
// indexer/watch_filter — filesystem events → indexable file changes
//
// The watcher reports every path the OS touches. Only a subset is the
// indexer's business: source files, under the project root, outside every
// directory the full scan excludes. An event stream carries no gitignore
// layer, so the exclusion here is the walk's hard directory set — a file the
// full scan would never visit must not enter the index through a change
// event, whatever created it.
// =============================================================================

use std::path::{Path, PathBuf};

use notify::{Event, EventKind};
use rustc_hash::FxHashSet;

use super::changeset::{ChangeKind, FileChangeEvent};

/// Turns a debounced batch of filesystem events into the deduped list of
/// project-relative file changes the incremental indexer accepts.
pub(super) struct WatchFilter {
    project_root: PathBuf,
    source_exts: FxHashSet<String>,
    exclude_dirs: Vec<&'static str>,
}

impl WatchFilter {
    pub(super) fn new(project_root: PathBuf) -> Self {
        let exclude_dirs = bearwisdom_profile::exclusions::project_exclude_dirs(&project_root);
        Self {
            project_root,
            source_exts: registered_source_extensions(),
            exclude_dirs,
        }
    }

    /// The changes worth reindexing from `events`, one per distinct relative
    /// path, in first-seen order. Non-source extensions, paths outside the
    /// root, and paths under an excluded directory are dropped.
    pub(super) fn changes(&self, events: &[Event]) -> Vec<FileChangeEvent> {
        let mut seen: FxHashSet<String> = FxHashSet::default();
        let mut changes: Vec<FileChangeEvent> = Vec::new();
        for event in events {
            let change_kind = match event.kind {
                EventKind::Create(_) => ChangeKind::Created,
                EventKind::Modify(_) => ChangeKind::Modified,
                EventKind::Remove(_) => ChangeKind::Deleted,
                _ => continue,
            };
            for path in &event.paths {
                let Some(rel) = self.relative_source_path(path) else {
                    continue;
                };
                if seen.insert(rel.clone()) {
                    changes.push(FileChangeEvent {
                        relative_path: rel,
                        change_kind,
                    });
                }
            }
        }
        changes
    }

    /// `path` as a forward-slashed project-relative path when it is a source
    /// file under the root and outside every excluded directory.
    fn relative_source_path(&self, path: &Path) -> Option<String> {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if !self.source_exts.contains(ext) {
            return None;
        }
        let rel = path
            .strip_prefix(&self.project_root)
            .ok()?
            .to_string_lossy()
            .replace('\\', "/");
        if bearwisdom_profile::exclusions::is_under_excluded_dir(Path::new(&rel), &self.exclude_dirs)
        {
            return None;
        }
        Some(rel)
    }
}

/// The last extension segment of every registered language extension,
/// lowercased. A multi-dot extension (`.d.ts`, `.blade.php`) contributes its
/// final segment: the watcher only decides *whether* to reindex, the indexer's
/// longest-suffix matcher picks the right plugin.
pub(super) fn registered_source_extensions() -> FxHashSet<String> {
    bearwisdom_profile::LANGUAGES
        .iter()
        .flat_map(|lang| lang.file_extensions.iter())
        .filter_map(|ext| {
            let last = ext.rsplit('.').next()?;
            if last.is_empty() {
                None
            } else {
                Some(last.to_ascii_lowercase())
            }
        })
        .collect()
}

#[cfg(test)]
#[path = "watch_filter_tests.rs"]
mod tests;
