// =============================================================================
// ocaml/resolve.rs — OCaml resolution rules
//
// Scope rules for OCaml:
//
//   1. Scope chain walk: innermost let/module → top-level.
//   2. Same-file resolution: all top-level bindings and modules are visible.
//   3. Import-based resolution:
//        `open Module`       → wildcard open; all public names in scope
//        `include Module`    → structural include (treated as wildcard open)
//        `module M = Module` → alias (M is a local name for Module)
//
// OCaml import model:
//   target_name = the module being opened/included or the local alias
//   module      = the source module when an alias is introduced
// =============================================================================

use super::predicates;
use crate::indexer::resolve::engine::{
    self as engine, FileContext, ImportEntry, LanguageResolver, RefContext, Resolution,
    SymbolInfo, SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

/// OCaml language resolver.
pub struct OcamlResolver;

impl LanguageResolver for OcamlResolver {
    fn language_ids(&self) -> &[&str] {
        &["ocaml"]
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
        let mut imports = Vec::new();

        for r in &file.refs {
            if r.kind != EdgeKind::Imports {
                continue;
            }
            // target_name is the opened/included module or local alias.
            // module is the original module when an alias is present.
            let source_module = r.module.as_deref().unwrap_or(&r.target_name);
            let alias = if r.module.is_some() && r.target_name != source_module {
                Some(r.target_name.clone())
            } else {
                None
            };

            imports.push(ImportEntry {
                imported_name: source_module.to_string(),
                module_path: Some(source_module.to_string()),
                alias,
                is_wildcard: true,
            });
        }

        FileContext {
            file_path: file.path.clone(),
            language: "ocaml".to_string(),
            imports,
            file_namespace: None,
        }
    }

    fn resolve(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        let edge_kind = ref_ctx.extracted_ref.kind;

        if edge_kind == EdgeKind::Imports {
            return None;
        }

        if let Some(res) = engine::resolve_common("ocaml", file_ctx, ref_ctx, lookup, predicates::kind_compatible) {
            return Some(res);
        }

        // OCaml files implicitly define a module named after the file stem (e.g.
        // `command.ml` → module `Command`). Refs like `Command.Args.S` are
        // split into `module=Some("Command.Args"), target="S"` by the extractor.
        // The symbols inside `command.ml` are indexed without the `Command.`
        // file-stem prefix, so `Args.S` exists but `Command.Args.S` doesn't.
        // Strip the leading component from the module path and retry the
        // qualified lookup: `Command.Args.S` → try `Args.S`.
        if let Some(module) = &ref_ctx.extracted_ref.module {
            let target = &ref_ctx.extracted_ref.target_name;
            if let Some(dot) = module.find('.') {
                let stripped_module = &module[dot + 1..];
                let candidate = format!("{stripped_module}.{target}");
                if let Some(sym) = lookup.by_qualified_name(&candidate) {
                    if predicates::kind_compatible(edge_kind, &sym.kind) {
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: 0.90,
                            strategy: "ocaml_stem_stripped",
                            resolved_yield_type: None,
                            flow_emit: None,
                        });
                    }
                }
                // Fallback: name-only lookup restricted to files whose path
                // contains the stripped module's last segment.
                let stripped_lower = stripped_module.to_lowercase();
                let last_seg = stripped_lower.rsplit('.').next().unwrap_or(&stripped_lower);
                let by_name = lookup.by_name(target);
                if let Some(sym) = by_name.iter().find(|s: &&SymbolInfo| {
                    let fl = s.file_path.to_lowercase().replace('\\', "/");
                    (fl.contains(&format!("/{last_seg}.")) || fl.contains(&format!("/{last_seg}/")))
                        && predicates::kind_compatible(edge_kind, &s.kind)
                }) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 0.88,
                        strategy: "ocaml_stem_stripped_name",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            } else {
                // Single-segment module: `List.fold_left`, `String.length`,
                // `Printf.printf`. The symbol is indexed inside `list.ml`/
                // `string.ml`/`printf.ml` with no qname prefix (file-stem-as-
                // module convention). Match by name + file-stem.
                let module_lower = module.to_lowercase();
                let by_name = lookup.by_name(target);
                if let Some(sym) = by_name.iter().find(|s: &&SymbolInfo| {
                    let fl = s.file_path.to_lowercase().replace('\\', "/");
                    (fl.ends_with(&format!("/{module_lower}.ml"))
                        || fl.ends_with(&format!("/{module_lower}.mli"))
                        || fl.contains(&format!("/{module_lower}/")))
                        && predicates::kind_compatible(edge_kind, &s.kind)
                }) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 0.92,
                        strategy: "ocaml_module_to_file_stem",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }
        }

        None
    }

    fn infer_external_namespace(
        &self,
        _file_ctx: &FileContext,
        _ref_ctx: &RefContext,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        // OCaml Stdlib classifies via the engine's keywords() set
        // populated from ocaml/mod.rs::keywords(); opam walker emits
        // real symbols for declared deps.
        None
    }
}
