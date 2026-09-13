// =============================================================================
// ecosystem/maven/symbol_index.rs
// =============================================================================

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rayon::prelude::*;
use tracing::{debug, warn};
use tree_sitter::{Node, Parser};

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext, ManifestSpec,
    SymbolLocationIndex,
};
use crate::ecosystem::externals::ExternalDepRoot;
use crate::walker::WalkedFile;

use super::header::scan_jvm_header;

// ---------------------------------------------------------------------------
// Symbol-location index (demand-driven pipeline entry)
// ---------------------------------------------------------------------------
//
// Walks every reached Maven dep root, header-only tree-sitter parses each
// .java/.kt/.scala/.clj[cs]?/.groovy file, and records each top-level type /
// function name against the file that defines it. The Stage 2 loop queries
// this index to pull only the files a user ref actually demands — the rest of
// the extracted sources jar stays on disk.
//
// Key shape: every symbol is inserted twice.
//   * `(module_path, name)` — keyed by the Maven coordinate `group:artifact`
//     so find_by_name's module-agnostic fallback and ecosystem-internal
//     diagnostics both work.
//   * `(java_package, name)` — keyed by the Java/Kotlin/Scala/etc. package
//     path derived from the file's location under the dep root
//     (`dep.root/org/springframework/context/Ctx.java` →
//     `org.springframework.context`). The JVM language resolvers emit
//     `module=java_package` on refs, so `locate(java_package, name)` hits
//     directly when a user `import org.springframework.context.Ctx` is
//     seeded.

pub(crate) fn build_maven_symbol_index(dep_roots: &[ExternalDepRoot]) -> SymbolLocationIndex {
    // Collect every walked file + its owning dep metadata so each parallel
    // scan task is self-contained. We walk the FULL dep root (not the
    // R3-narrowed slice) so the index covers every symbol the jar publishes,
    // not just those the project already imports — chain-miss resolution
    // pulls further files via `find_by_name` once imports expand.
    let mut work: Vec<(String, PathBuf, WalkedFile)> = Vec::new();
    for dep in dep_roots {
        let root = dep.root.clone();
        for wf in super::walk_maven_root(dep) {
            work.push((dep.module_path.clone(), root.clone(), wf));
        }
    }
    if work.is_empty() {
        return SymbolLocationIndex::new();
    }

    // Parallel header-only scan. Each task emits `(module_key, name, file)`
    // tuples keyed by BOTH the Maven coordinate AND the derived Java package.
    let per_file: Vec<Vec<(String, String, PathBuf)>> = work
        .par_iter()
        .map(|(module_path, dep_root, wf)| {
            let Ok(src) = std::fs::read_to_string(&wf.absolute_path) else {
                return Vec::new();
            };
            let names = scan_jvm_header(&src, wf.language);
            if names.is_empty() {
                return Vec::new();
            }
            let java_package = java_package_from_rel_path(&wf.absolute_path, dep_root);
            let mut rows: Vec<(String, String, PathBuf)> = Vec::with_capacity(names.len() * 2);
            for name in names {
                rows.push((module_path.clone(), name.clone(), wf.absolute_path.clone()));
                if let Some(pkg) = java_package.as_ref() {
                    // A JVM import names the declaration by its fully qualified
                    // name, so the file is keyed under that spelling as well as
                    // under its package.
                    rows.push((
                        format!("{pkg}.{name}"),
                        name.clone(),
                        wf.absolute_path.clone(),
                    ));
                    rows.push((pkg.clone(), name, wf.absolute_path.clone()));
                }
            }
            rows
        })
        .collect();

    let mut index = SymbolLocationIndex::new();
    for batch in per_file {
        for (module, name, file) in batch {
            index.insert(module, name, file);
        }
    }
    index
}

/// Derive the Java/Kotlin/Scala package from a file's location under the dep
/// root. `dep.root/org/springframework/context/Ctx.java` yields
/// `"org.springframework.context"`. Returns None for files at the dep root
/// (no package segments) or paths not under `dep_root`.
pub(crate) fn java_package_from_rel_path(file: &Path, dep_root: &Path) -> Option<String> {
    let rel = file.strip_prefix(dep_root).ok()?;
    let mut segs: Vec<&str> = rel
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => s.to_str(),
            _ => None,
        })
        .collect();
    if segs.len() < 2 {
        return None;
    }
    segs.pop(); // drop the filename, keep directory segments.
    Some(segs.join("."))
}
