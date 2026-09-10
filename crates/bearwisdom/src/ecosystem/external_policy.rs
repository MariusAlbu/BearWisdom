//! Ecosystem-owned policy used while materializing external source files.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::type_checker::core::types::TypeArena;
use crate::types::{ExtractedRef, ParsedFile};

#[path = "npm/external_policy.rs"]
mod npm;

type PostProcess = fn(&mut ParsedFile, &TypeArena) -> bool;
type CollectRelativeSupertypes =
    fn(&str, &Path, &[ExtractedRef], &mut HashSet<PathBuf>, &mut Vec<PathBuf>) -> bool;
type ResolveRelativeModule = fn(&str, &Path, &str) -> Option<PathBuf>;

struct Adapter {
    post_process: PostProcess,
    collect_relative_supertypes: CollectRelativeSupertypes,
    resolve_relative_module: ResolveRelativeModule,
}

const ADAPTERS: &[Adapter] = &[Adapter {
    post_process: npm::post_process,
    collect_relative_supertypes: npm::collect_relative_supertypes,
    resolve_relative_module: npm::resolve_relative_module,
}];

pub(crate) fn post_process(parsed: &mut ParsedFile, arena: &TypeArena) {
    let _ = ADAPTERS
        .iter()
        .find(|adapter| (adapter.post_process)(parsed, arena));
}

pub(crate) fn collect_relative_supertypes(
    language: &str,
    importer: &Path,
    refs: &[ExtractedRef],
    seen: &mut HashSet<PathBuf>,
    out: &mut Vec<PathBuf>,
) {
    let _ = ADAPTERS
        .iter()
        .find(|adapter| (adapter.collect_relative_supertypes)(language, importer, refs, seen, out));
}

pub(crate) fn resolve_relative_module(
    language: &str,
    directory: &Path,
    specifier: &str,
) -> Option<PathBuf> {
    ADAPTERS
        .iter()
        .find_map(|adapter| (adapter.resolve_relative_module)(language, directory, specifier))
}
