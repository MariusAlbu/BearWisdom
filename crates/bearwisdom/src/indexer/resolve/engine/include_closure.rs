// =============================================================================
// engine/include_closure — fail-closed include-edge visibility
//
// A declaration supplied through an include edge is visible only when the
// source reaches that file through the transitive include graph. Language
// plugins resolve source spellings to exact indexed paths; this store retains
// those edges and computes their transitive closure.
// =============================================================================

use std::collections::VecDeque;

use rustc_hash::{FxHashMap, FxHashSet};

use crate::types::{EdgeKind, ParsedFile};

#[derive(Default)]
pub(super) struct IncludeClosure {
    file_languages: FxHashMap<String, String>,
    direct_specs: FxHashMap<String, Vec<String>>,
    parsed_sources: FxHashSet<String>,
    reachable: FxHashMap<String, FxHashSet<String>>,
    placed_specs: FxHashMap<String, FxHashSet<String>>,
}

impl IncludeClosure {
    pub(super) fn ingest_parsed(&mut self, files: &[ParsedFile]) {
        for file in files {
            self.file_languages
                .insert(file.path.clone(), file.language.clone());
            self.parsed_sources.insert(file.path.clone());
            self.direct_specs.insert(
                file.path.clone(),
                file.refs
                    .iter()
                    .filter(|reference| reference.kind == EdgeKind::Imports && reference.is_include)
                    .map(|reference| {
                        reference
                            .module
                            .clone()
                            .unwrap_or_else(|| reference.target_name.clone())
                    })
                    .collect(),
            );
        }
        self.rebuild();
    }

    /// Restore unchanged files from durable import rows. Freshly parsed files
    /// remain authoritative when an incremental build overlays the database.
    pub(super) fn ingest_persisted(
        &mut self,
        files: impl IntoIterator<Item = (String, String)>,
        specs: FxHashMap<String, Vec<String>>,
    ) {
        self.file_languages.extend(files);
        for (source, includes) in specs {
            if !self.parsed_sources.contains(&source) {
                self.direct_specs.insert(source, includes);
            }
        }
        self.rebuild();
    }

    /// Restore persisted include evidence without exposing the storage schema
    /// to the compilation coordinator.
    pub(super) fn ingest_from_db(&mut self, conn: &rusqlite::Connection) {
        let mut files: Vec<(String, String)> = Vec::new();
        if let Ok(mut statement) = conn.prepare(
            "SELECT path, language FROM files
             WHERE language IN (
                 SELECT DISTINCT f.language
                 FROM imports i JOIN files f ON f.id = i.file_id
                 WHERE i.is_include = 1
             )",
        ) {
            if let Ok(rows) = statement.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            }) {
                files.extend(rows.flatten());
            }
        }

        let mut specs: FxHashMap<String, Vec<String>> = files
            .iter()
            .map(|(path, _)| (path.clone(), Vec::new()))
            .collect();
        if let Ok(mut statement) = conn.prepare(
            "SELECT f.path, COALESCE(i.module_path, i.imported_name) \
             FROM imports i JOIN files f ON f.id = i.file_id \
             WHERE i.is_include = 1",
        ) {
            if let Ok(rows) = statement.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            }) {
                for (source, spec) in rows.flatten() {
                    specs.entry(source).or_default().push(spec);
                }
            }
        }
        self.ingest_persisted(files, specs);
    }

    pub(super) fn reaches(&self, source_file: &str, candidate_file: &str) -> bool {
        self.reachable
            .get(source_file)
            .is_some_and(|paths| paths.contains(candidate_file))
    }

    /// Whether `source_file`'s include spelling `spec` was placed on exactly
    /// one indexed file by the owning plugin.
    pub(super) fn spec_resolves(&self, source_file: &str, spec: &str) -> bool {
        self.placed_specs
            .get(source_file)
            .is_some_and(|specs| specs.contains(spec))
    }

    fn rebuild(&mut self) {
        let mut placed: FxHashMap<String, FxHashSet<String>> = FxHashMap::default();
        let adjacency: FxHashMap<String, Vec<String>> = self
            .direct_specs
            .iter()
            .map(|(source, specs)| {
                let mut targets = Vec::new();
                for spec in specs {
                    if let Some(target) = self.resolve_unique(source, spec) {
                        targets.push(target);
                        placed
                            .entry(source.clone())
                            .or_default()
                            .insert(spec.clone());
                    }
                }
                (source.clone(), targets)
            })
            .collect();
        self.placed_specs = placed;

        self.reachable.clear();
        for source in self.direct_specs.keys() {
            let mut seen = FxHashSet::default();
            let mut queue: VecDeque<String> = adjacency
                .get(source)
                .into_iter()
                .flatten()
                .cloned()
                .collect();
            while let Some(path) = queue.pop_front() {
                if !seen.insert(path.clone()) {
                    continue;
                }
                if let Some(next) = adjacency.get(&path) {
                    queue.extend(next.iter().cloned());
                }
            }
            self.reachable.insert(source.clone(), seen);
        }
    }

    fn resolve_unique(&self, source: &str, spec: &str) -> Option<String> {
        let source_language = self.file_languages.get(source)?;
        let indexed_files: Vec<(&str, &str)> = self
            .file_languages
            .iter()
            .map(|(path, language)| (path.as_str(), language.as_str()))
            .collect();
        crate::languages::default_registry()
            .get(source_language)
            .resolve_include_target(source, spec, &indexed_files)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ExtractedRef, FlowMeta};

    fn file(path: &str, includes: &[&str]) -> ParsedFile {
        file_with_language(path, "c", includes)
    }

    fn file_with_language(path: &str, language: &str, includes: &[&str]) -> ParsedFile {
        ParsedFile {
            path: path.into(),
            language: language.into(),
            content_hash: String::new(),
            size: 0,
            line_count: 0,
            mtime: None,
            package_id: None,
            symbols: Vec::new(),
            refs: includes
                .iter()
                .map(|module| ExtractedRef {
                    source_symbol_index: 0,
                    target_name: module.rsplit('/').next().unwrap().to_string(),
                    kind: EdgeKind::Imports,
                    line: 0,
                    col: 0,
                    module: Some((*module).to_string()),
                    namespace_segments: Vec::new(),
                    chain: None,
                    byte_offset: 0,
                    call_args: Vec::new(),
                    is_import_binding: false,
                    is_reexport: false,
                    is_include: true,
                })
                .collect(),
            routes: Vec::new(),
            db_sets: Vec::new(),
            symbol_origin_languages: Vec::new(),
            ref_origin_languages: Vec::new(),
            symbol_from_snippet: Vec::new(),
            content: None,
            has_errors: false,
            flow: FlowMeta::default(),
            demand_contributions: Vec::new(),
            alias_targets: Vec::new(),
            component_selectors: Vec::new(),
            plugin_flow_emissions: Vec::new(),
            declared_modules: Vec::new(),
        }
    }

    #[test]
    fn follows_transitive_headers_and_terminates_cycles() {
        let mut closure = IncludeClosure::default();
        closure.ingest_parsed(&[
            file("src/main.c", &["stdio.h"]),
            file("ext:idx:/sdk/include/stdio.h", &["bits/types.h"]),
            file("ext:idx:/sdk/include/bits/types.h", &["../stdio.h"]),
        ]);
        assert!(closure.reaches("src/main.c", "ext:idx:/sdk/include/stdio.h"));
        assert!(closure.reaches("src/main.c", "ext:idx:/sdk/include/bits/types.h"));
    }

    #[test]
    fn ambiguous_suffix_does_not_create_visibility() {
        let mut closure = IncludeClosure::default();
        closure.ingest_parsed(&[
            file("src/main.c", &["stdio.h"]),
            file("ext:idx:/sdk-a/include/stdio.h", &[]),
            file("ext:idx:/sdk-b/include/stdio.h", &[]),
        ]);
        assert!(!closure.reaches("src/main.c", "ext:idx:/sdk-a/include/stdio.h"));
        assert!(!closure.reaches("src/main.c", "ext:idx:/sdk-b/include/stdio.h"));
    }

    #[test]
    fn explicit_relative_path_wins_over_suffix_candidates() {
        let mut closure = IncludeClosure::default();
        closure.ingest_parsed(&[
            file("src/main.c", &["./local.h"]),
            file("src/local.h", &[]),
            file("ext:idx:/sdk/local.h", &[]),
        ]);
        assert!(closure.reaches("src/main.c", "src/local.h"));
        assert!(!closure.reaches("src/main.c", "ext:idx:/sdk/local.h"));
    }

    #[test]
    fn bare_include_does_not_prefer_same_directory_over_sdk_candidate() {
        let mut closure = IncludeClosure::default();
        closure.ingest_parsed(&[
            file("src/main.c", &["stdio.h"]),
            file("src/stdio.h", &[]),
            file("ext:idx:/sdk/include/stdio.h", &[]),
        ]);
        assert!(!closure.reaches("src/main.c", "src/stdio.h"));
        assert!(!closure.reaches("src/main.c", "ext:idx:/sdk/include/stdio.h"));
    }

    #[test]
    fn bare_include_does_not_use_unattested_internal_header_as_external_bridge() {
        let mut closure = IncludeClosure::default();
        closure.ingest_parsed(&[
            file("src/main.c", &["foo.h"]),
            file("src/foo.h", &["vendor/api.h"]),
            file("ext:idx:/sdk/include/vendor/api.h", &[]),
        ]);
        assert!(!closure.reaches("src/main.c", "src/foo.h"));
        assert!(!closure.reaches("src/main.c", "ext:idx:/sdk/include/vendor/api.h"));
    }

    #[test]
    fn fresh_file_without_include_overrides_stale_persisted_edge() {
        let mut closure = IncludeClosure::default();
        closure.ingest_parsed(&[
            file("src/main.c", &[]),
            file("ext:idx:/sdk/include/stdio.h", &[]),
        ]);
        closure.ingest_persisted(
            [
                ("src/main.c".to_string(), "c".to_string()),
                ("ext:idx:/sdk/include/stdio.h".to_string(), "c".to_string()),
            ],
            FxHashMap::from_iter([("src/main.c".to_string(), vec!["stdio.h".to_string()])]),
        );

        assert!(!closure.reaches("src/main.c", "ext:idx:/sdk/include/stdio.h"));
    }

    #[test]
    fn languages_without_include_policy_fail_closed() {
        let mut closure = IncludeClosure::default();
        closure.ingest_parsed(&[
            file_with_language("src/main.custom", "custom", &["api.h"]),
            file_with_language("ext:idx:/custom/include/api.h", "custom", &[]),
            file_with_language("ext:idx:/unrelated/include/api.h", "unrelated", &[]),
        ]);

        assert!(!closure.reaches("src/main.custom", "ext:idx:/custom/include/api.h"));
        assert!(!closure.reaches("src/main.custom", "ext:idx:/unrelated/include/api.h"));
    }
}
