//! npm external-declaration materialization policy.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::type_checker::core::types::TypeArena;
use crate::types::{ExtractedRef, ParsedFile};
use crate::walker::WalkedFile;

const LANGUAGES: &[&str] = &[
    "typescript",
    "tsx",
    "javascript",
    "vue",
    "svelte",
    "angular",
    "astro",
    "scss",
];

fn owns(language: &str) -> bool {
    LANGUAGES.contains(&language)
}

pub(super) fn post_process(parsed: &mut ParsedFile, arena: &TypeArena) -> bool {
    if !owns(&parsed.language) {
        return false;
    }
    crate::ecosystem::npm::ts_post_process_external(parsed, arena);
    true
}

pub(super) fn collect_relative_supertypes(
    language: &str,
    importer: &Path,
    refs: &[ExtractedRef],
    seen: &mut HashSet<PathBuf>,
    out: &mut Vec<PathBuf>,
) -> bool {
    if !owns(language) {
        return false;
    }
    crate::ecosystem::npm::relative_imports::collect_relative_supertype_imports(
        importer, refs, seen, out,
    );
    true
}

pub(super) fn resolve_relative_module(
    language: &str,
    directory: &Path,
    specifier: &str,
) -> Option<PathBuf> {
    owns(language)
        .then(|| {
            crate::ecosystem::npm::relative_imports::resolve_relative_ts_module(
                directory, specifier,
            )
        })
        .flatten()
}

pub(super) fn secondary_scan(project_root: &Path, primary: &[WalkedFile]) -> Vec<WalkedFile> {
    crate::ecosystem::npm::pull_gitignored_imports(project_root, primary)
}
