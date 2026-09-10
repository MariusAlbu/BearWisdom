// =============================================================================
// engine/include_closure — fail-closed C/C++ header visibility
//
// C declarations live in a flat namespace, but an external header declaration
// is visible only when the translation unit reaches that header through its
// transitive #include graph. This store retains raw include spellings and
// derives source-file -> reachable-file paths whenever the Compilation grows.
// =============================================================================

use std::collections::VecDeque;

use rustc_hash::{FxHashMap, FxHashSet};

use crate::types::{EdgeKind, ParsedFile};

#[derive(Default)]
pub(super) struct IncludeClosure {
    known_files: FxHashSet<String>,
    direct_specs: FxHashMap<String, Vec<String>>,
    parsed_sources: FxHashSet<String>,
    reachable: FxHashMap<String, FxHashSet<String>>,
}

impl IncludeClosure {
    pub(super) fn ingest_parsed(&mut self, files: &[ParsedFile]) {
        for file in files.iter().filter(|file| is_c_family(&file.language)) {
            self.known_files.insert(file.path.clone());
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
        files: impl IntoIterator<Item = String>,
        specs: FxHashMap<String, Vec<String>>,
    ) {
        self.known_files.extend(files);
        for (source, includes) in specs {
            if !self.parsed_sources.contains(&source) {
                self.direct_specs.insert(source, includes);
            }
        }
        self.rebuild();
    }

    pub(super) fn reaches(&self, source_file: &str, candidate_file: &str) -> bool {
        self.reachable
            .get(source_file)
            .is_some_and(|paths| paths.contains(candidate_file))
    }

    fn rebuild(&mut self) {
        let adjacency: FxHashMap<String, Vec<String>> = self
            .direct_specs
            .iter()
            .map(|(source, specs)| {
                let targets = specs
                    .iter()
                    .filter_map(|spec| self.resolve_unique(source, spec))
                    .collect();
                (source.clone(), targets)
            })
            .collect();

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
        let source = normalize(source);
        let explicit_relative = spec
            .replace('\\', "/")
            .split('/')
            .next()
            .is_some_and(|part| matches!(part, "." | ".."));
        let spec = normalize(spec);
        if spec.is_empty() {
            return None;
        }

        // Include refs currently retain the path but not the quote/angle
        // delimiter. Only an explicit ./ or ../ spelling can therefore use
        // the including file's directory without risking that <stdio.h>
        // incorrectly selects a same-named project header over an SDK header.
        if explicit_relative {
            let source_dir = source.rsplit_once('/').map_or("", |(dir, _)| dir);
            let relative = normalize(&format!("{source_dir}/{spec}"));
            let exact: Vec<&String> = self
                .known_files
                .iter()
                .filter(|path| normalize(path) == relative && normalize(path) != source)
                .collect();
            return (exact.len() == 1).then(|| exact[0].clone());
        }

        // Supplied SDK headers keep an external virtual-path prefix. Match the
        // compiler-visible include spelling only at a path boundary and only
        // when it identifies one indexed file. Ambiguity deliberately fails.
        let suffix = format!("/{spec}");
        let matches: Vec<&String> = self
            .known_files
            .iter()
            .filter(|path| {
                let path = normalize(path);
                path != source && (path == spec || path.ends_with(&suffix))
            })
            .collect();
        (matches.len() == 1 && matches[0].starts_with("ext:")).then(|| matches[0].clone())
    }
}

fn is_c_family(language: &str) -> bool {
    matches!(language, "c" | "cpp")
}

fn normalize(path: &str) -> String {
    let replaced = path.replace('\\', "/");
    let mut parts: Vec<&str> = Vec::new();
    for part in replaced.split('/') {
        match part {
            "" | "." => {}
            ".." if parts.last().is_some_and(|prior| *prior != "..") => {
                parts.pop();
            }
            ".." => parts.push(part),
            _ => parts.push(part),
        }
    }
    parts.join("/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ExtractedRef, FlowMeta};

    fn file(path: &str, includes: &[&str]) -> ParsedFile {
        ParsedFile {
            path: path.into(),
            language: "c".into(),
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
                "src/main.c".to_string(),
                "ext:idx:/sdk/include/stdio.h".to_string(),
            ],
            FxHashMap::from_iter([("src/main.c".to_string(), vec!["stdio.h".to_string()])]),
        );

        assert!(!closure.reaches("src/main.c", "ext:idx:/sdk/include/stdio.h"));
    }
}
