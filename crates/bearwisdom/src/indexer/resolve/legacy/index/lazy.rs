// =============================================================================
// indexer/resolve/engine/index/lazy.rs — materialize-on-miss driver
//
// When a lookup misses the eager-internal maps, this drives the lazy pull: the
// external location index says which file defines the name, the file is parsed
// once (registry + arena, NO DB write), and its symbols are interned into the
// MaterializedStore. The lookup then re-queries and answers from the store.
//
// This runs UNDER `&self` inside the resolve pass's rayon workers. Parsing is
// pure; interning is the store's lock-free `&self` push; DB rows for the
// materialized symbols are written later, deferred, by the resolve flush.
// =============================================================================

use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use rustc_hash::FxHashMap;

use crate::ecosystem::jar_walker;
use crate::ecosystem::nuget::crack_one_dll_type;
use crate::indexer::external_parse_cache;
use crate::indexer::full::parse_file_with_arena_and_demand;
use crate::indexer::resolve::legacy::{
    find_matching_bracket, is_jvm_language, merge_where_bounds, parse_generic_param_clause,
    parse_return_type_from_jvm_descriptor, parse_return_type_from_signature,
    parse_return_type_positional, parse_type_head_and_args, resolve_type_name_in_scope, SymbolInfo,
    TypeInfo,
};
use super::common_prefix_len;
use crate::languages::default_registry;
use crate::types::{EdgeKind, ParsedFile, SymbolKind};
use crate::walker::WalkedFile;

use super::SymbolIndex;

impl SymbolIndex {
    /// Write the materialized external files to the DB and return a
    /// synthetic-id → real-id map for the edges that bound to them during the
    /// pass. Empty when nothing was materialized. Runs AFTER the resolve pass,
    /// outside any open transaction (it opens its own), so the FK-enforced edge
    /// flush that follows finds the materialized symbols' rows in place.
    pub fn flush_materialized_externals(
        &self,
        db: &crate::db::Database,
    ) -> anyhow::Result<std::collections::HashMap<i64, i64>> {
        let parsed = self.materialized.drain_parsed_files();
        if parsed.is_empty() {
            return Ok(std::collections::HashMap::new());
        }
        let (_files, symbol_id_map) = crate::indexer::write::write_parsed_files_with_origin(
            db,
            &parsed,
            "external",
            Some(&self.type_arena),
        )?;
        // The in-pass intern gave each materialized symbol a synthetic id; the
        // write just assigned the real DB id. Map one to the other by (path,
        // qname) so the buffered edges can be rewritten to the real targets.
        let mut remap = std::collections::HashMap::with_capacity(symbol_id_map.len());
        for sym in self.materialized.all_symbols() {
            if let Some(&real) =
                symbol_id_map.get(&(sym.file_path.to_string(), sym.qualified_name.clone()))
            {
                remap.insert(sym.id, real);
            }
        }
        Ok(remap)
    }

    /// Materialize every external file the location index says defines `name`.
    /// No-op when there is no external location index (internal-only project)
    /// or the name is unknown to it. Idempotent + concurrency-safe: each file
    /// is parsed at most once via the per-file guard.
    pub fn materialize_by_name(&self, name: &str) {
        if self.loc.is_empty() {
            return;
        }
        for (_module, file) in self.loc.find_by_name(name) {
            self.materialize_file(file);
        }
    }

    /// Parse + intern one external file, guarded so concurrent hitters parse it
    /// at most once.
    fn materialize_file(&self, file: &Path) {
        let guard = self.materialized.file_guard(file);
        guard.get_or_init(|| self.do_materialize_file(file));
    }

    /// The one-shot body run under the file guard: parse the file and intern
    /// its symbols into the materialized store, each with a fresh external id.
    fn do_materialize_file(&self, file: &Path) {
        let path_str = file.to_string_lossy();

        // Binary format dispatch: virtual paths produced by the demand-driven
        // JAR and DLL ecosystems encode the archive + entry/type, not a source
        // file path. Detect them first and short-circuit to the binary crack
        // functions, which produce a ParsedFile without touching tree-sitter.
        if path_str.starts_with("ext:jar:") {
            if let Some(pf) = materialize_jar_class(&path_str) {
                self.intern_parsed_file(pf);
            }
            return;
        }
        if path_str.starts_with("ext:dotnet-type:") {
            // Language id is not encoded in the path; infer from context. The
            // materialize step only needs it for the ParsedFile.language tag —
            // "csharp" is the dominant .NET language and a safe default.
            if let Some(pf) = crack_one_dll_type(&path_str, "csharp") {
                self.intern_parsed_file(pf);
            }
            return;
        }

        let Some(language) = language_from_file_ext(file) else {
            return;
        };
        let virtual_path = virtual_path_for_indexed_file(file, language);
        // Consult the persistent parse cache: a content-hash hit rebuilds the
        // extraction without tree-sitter. On a miss, parse, TS-post-process, and
        // store the result so a future run (or another project sharing this dep)
        // skips the parse. Best-effort — a read failure just falls through.
        let Ok(bytes) = std::fs::read(file) else {
            return;
        };
        let hash = external_parse_cache::content_hash(&bytes);
        let size = bytes.len() as u64;
        let parsed = match external_parse_cache::get(file, &hash, &virtual_path, size) {
            Some(cached) => cached,
            None => {
                let walked = WalkedFile {
                    relative_path: virtual_path,
                    absolute_path: file.to_path_buf(),
                    language,
                };
                let mut pf = match parse_file_with_arena_and_demand(
                    &walked,
                    default_registry(),
                    None,
                    &self.type_arena,
                ) {
                    Ok(pf) => pf,
                    Err(_) => return,
                };
                // Mirror the expand pull's TS post-process so external `.d.ts`
                // symbols carry the `<pkg>.` prefix the resolver keys on. No-op
                // for non-TS. Cache the POST-processed shape.
                crate::ecosystem::npm::ts_post_process_external(&mut pf);
                external_parse_cache::put(file, &hash, &pf);
                pf
            }
        };

        self.intern_parsed_file(parsed);
    }

    /// Intern a pre-built `ParsedFile` into the materialized store. Shared by
    /// the tree-sitter source path and the binary-format crack paths (JAR class
    /// files, DLL type slices).
    fn intern_parsed_file(&self, parsed: ParsedFile) {
        let file_path: Arc<str> = Arc::from(parsed.path.as_str());
        for sym in &parsed.symbols {
            let id = self.next_ext_id.fetch_add(1, Ordering::Relaxed);
            self.materialized.intern(SymbolInfo {
                id,
                name: sym.name.clone(),
                qualified_name: sym.qualified_name.clone(),
                kind: sym.kind.as_str().to_string(),
                visibility: sym
                    .visibility
                    .as_ref()
                    .map(|v| format!("{v:?}").to_lowercase()),
                file_path: Arc::clone(&file_path),
                scope_path: sym.scope_path.clone(),
                package_id: parsed.package_id,
                signature: sym.signature.clone(),
            });
        }
        self.populate_materialized_type_info(&parsed);
        self.populate_materialized_inherits(&parsed);
        self.materialized.stash_parsed(parsed);
    }

    /// Compute per-symbol type metadata for a materialized file and intern it
    /// into the materialized type store. A faithful port of the eager build's
    /// Pass 2 (augment.rs), scoped to one external file. Type-name resolution
    /// uses the eager `by_qname` — externals resolve their declared types
    /// against the project + already-materialized symbols.
    fn populate_materialized_type_info(&self, pf: &ParsedFile) {
        let mut type_refs_by_sym: Vec<Vec<&str>> = vec![Vec::new(); pf.symbols.len()];
        for r in &pf.refs {
            if r.kind != EdgeKind::TypeRef || r.is_import_binding {
                continue;
            }
            let idx = r.source_symbol_index;
            if idx < type_refs_by_sym.len() {
                type_refs_by_sym[idx].push(r.target_name.as_str());
            }
        }

        let mut acc: FxHashMap<String, TypeInfo> = FxHashMap::default();
        for (sym_idx, sym) in pf.symbols.iter().enumerate() {
            let type_refs = &type_refs_by_sym[sym_idx];
            match sym.kind {
                SymbolKind::Property
                | SymbolKind::Field
                | SymbolKind::Variable
                | SymbolKind::Parameter => {
                    if let Some(first) = type_refs.first() {
                        let resolved =
                            resolve_type_name_in_scope(first, sym.scope_path.as_deref(), &self.by_qname);
                        acc.entry(sym.qualified_name.clone()).or_default().field_type = Some(resolved);
                        if type_refs.len() > 1 {
                            acc.entry(sym.qualified_name.clone()).or_default().type_args =
                                type_refs[1..].iter().map(|s| s.to_string()).collect();
                        }
                    } else if is_jvm_language(&pf.language) {
                        if let Some(decoded) = sym
                            .signature
                            .as_deref()
                            .and_then(parse_return_type_from_jvm_descriptor)
                        {
                            let resolved = resolve_type_name_in_scope(
                                &decoded,
                                sym.scope_path.as_deref(),
                                &self.by_qname,
                            );
                            acc.entry(sym.qualified_name.clone()).or_default().field_type =
                                Some(resolved);
                        }
                    }
                }
                SymbolKind::TypeAlias => {
                    if let Some(first) = type_refs.first() {
                        acc.entry(sym.qualified_name.clone()).or_default().field_type =
                            Some(first.to_string());
                    }
                }
                SymbolKind::Method | SymbolKind::Function | SymbolKind::Constructor => {
                    let sig_rt: Option<String> = sym.signature.as_deref().and_then(|s| {
                        parse_return_type_from_signature(s)
                            .or_else(|| {
                                if sym.kind == SymbolKind::Constructor {
                                    None
                                } else {
                                    parse_return_type_positional(s)
                                }
                            })
                            .or_else(|| {
                                if is_jvm_language(&pf.language) {
                                    parse_return_type_from_jvm_descriptor(s)
                                } else {
                                    None
                                }
                            })
                    });
                    let sig_generic: Option<(String, Vec<String>)> = sig_rt.as_deref().and_then(|rt| {
                        let (head, args) = parse_type_head_and_args(rt);
                        let head_is_name = !head.is_empty()
                            && head.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '.');
                        if args.is_empty() || !head_is_name {
                            None
                        } else {
                            Some((head.to_string(), args.iter().map(|s| s.to_string()).collect()))
                        }
                    });
                    if let Some((head, args)) = sig_generic {
                        let resolved =
                            resolve_type_name_in_scope(&head, sym.scope_path.as_deref(), &self.by_qname);
                        let ti = acc.entry(sym.qualified_name.clone()).or_default();
                        ti.return_type = Some(resolved);
                        ti.return_type_args = args;
                    } else {
                        if let Some(&last) = type_refs.last() {
                            let resolved = resolve_type_name_in_scope(
                                last,
                                sym.scope_path.as_deref(),
                                &self.by_qname,
                            );
                            acc.entry(sym.qualified_name.clone()).or_default().return_type =
                                Some(resolved);
                        }
                        let already = acc
                            .get(&sym.qualified_name)
                            .and_then(|ti| ti.return_type.as_ref())
                            .is_some();
                        if !already {
                            if let Some(rt) = &sig_rt {
                                let resolved = resolve_type_name_in_scope(
                                    rt,
                                    sym.scope_path.as_deref(),
                                    &self.by_qname,
                                );
                                acc.entry(sym.qualified_name.clone()).or_default().return_type =
                                    Some(resolved);
                            }
                        }
                    }
                }
                SymbolKind::Class => {
                    acc.entry(sym.qualified_name.clone()).or_default().return_type =
                        Some(sym.qualified_name.clone());
                }
                _ => {}
            }
        }

        for sym in &pf.symbols {
            if !matches!(
                sym.kind,
                SymbolKind::Class
                    | SymbolKind::Interface
                    | SymbolKind::Struct
                    | SymbolKind::TypeAlias
                    | SymbolKind::Function
                    | SymbolKind::Method
            ) {
                continue;
            }
            let Some(sig) = &sym.signature else { continue };
            let bracket_pairs: &[(char, char)] = &[('<', '>'), ('[', ']')];
            for &(open, close) in bracket_pairs {
                if let Some(start) = sig.find(open) {
                    if let Some(relative_end) = find_matching_bracket(&sig[start..], open, close) {
                        let end = start + relative_end;
                        let mut parsed = parse_generic_param_clause(&sig[start + 1..end]);
                        merge_where_bounds(&mut parsed, sig);
                        if !parsed.is_empty() {
                            let (params, bounds): (Vec<String>, Vec<Option<String>>) =
                                parsed.into_iter().unzip();
                            for key in [&sym.name, &sym.qualified_name] {
                                let ti = acc.entry(key.clone()).or_default();
                                ti.generic_params = params.clone();
                                ti.generic_param_bounds = bounds.clone();
                            }
                            break;
                        }
                    }
                }
            }
        }

        for (key, ti) in acc {
            self.materialized.intern_type(&key, ti);
        }
    }

    /// Record inheritance edges for a materialized file's types: child qname →
    /// best parent qname. Port of the eager Pass 5 (augment.rs:366), resolving
    /// the parent simple name against eager + already-materialized candidates
    /// with the namespace-prefix tiebreak.
    fn populate_materialized_inherits(&self, pf: &ParsedFile) {
        for r in &pf.refs {
            if r.kind != EdgeKind::Inherits {
                continue;
            }
            let Some(child) = pf.symbols.get(r.source_symbol_index) else {
                continue;
            };
            if !matches!(
                child.kind,
                SymbolKind::Class | SymbolKind::Interface | SymbolKind::Trait | SymbolKind::Struct
            ) {
                continue;
            }
            let parent_simple = r.target_name.trim_start_matches('\\');
            let child_ns = child
                .qualified_name
                .rfind('.')
                .map(|i| &child.qualified_name[..i])
                .unwrap_or("");
            let eager = self
                .by_name
                .get(parent_simple)
                .map(|v| v.as_slice())
                .unwrap_or(&[]);
            let mat = self.materialized.by_name(parent_simple);
            let mut best: Option<&str> = None;
            let mut best_score = 0usize;
            for cand in eager.iter().chain(mat.iter().copied()) {
                let cns = cand
                    .qualified_name
                    .rfind('.')
                    .map(|i| &cand.qualified_name[..i])
                    .unwrap_or("");
                let score = common_prefix_len(child_ns, cns);
                if best.is_none() || score > best_score {
                    best = Some(cand.qualified_name.as_str());
                    best_score = score;
                }
            }
            if let Some(parent_qname) = best {
                self.materialized
                    .intern_inherits(&child.qualified_name, parent_qname.to_string());
            }
        }
    }
}

/// Virtual path for a pulled external file — the `ext:` shape the resolver and
/// `is_external_file` key on, falling back to `ext:idx:<abs>`.
fn virtual_path_for_indexed_file(path: &Path, language: &str) -> String {
    crate::indexer::stage_link::virtual_path_for_pulled(path, language)
        .unwrap_or_else(|| format!("ext:idx:{}", path.to_string_lossy().replace('\\', "/")))
}

/// Language id for a pulled file, via the registry's extension table. `None`
/// for extensions the indexer can't parse.
fn language_from_file_ext(path: &Path) -> Option<&'static str> {
    let name = path.file_name().and_then(|n| n.to_str())?;
    default_registry().language_by_extension(name)
}

/// Crack a single `.class` entry from the JAR encoded in a virtual path of
/// the form `ext:jar:<archive_abs>!<entry_name>`. Returns `None` on any
/// decode or I/O error so the caller can skip silently.
fn materialize_jar_class(virtual_path: &str) -> Option<ParsedFile> {
    // Strip the "ext:jar:" prefix → "<archive_abs>!<entry_name>"
    let payload = virtual_path.strip_prefix("ext:jar:")?;
    // The `!` separating archive from entry cannot appear in a filesystem path
    // on Windows or Unix, so the first `!` is unambiguous.
    let bang = payload.find('!')?;
    let archive_str = &payload[..bang];
    let entry_name = &payload[bang + 1..];
    let archive = std::path::Path::new(archive_str);
    jar_walker::crack_one_class(archive, entry_name)
}
