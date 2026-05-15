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
        // OCaml auto-opens `Stdlib` in every compilation unit. Bare calls
        // like `close_in oc` or `open_in path` carry no module qualifier
        // and no explicit `open` ref, so the wildcard-import step in
        // resolve_common needs an implicit entry pointing at the Stdlib
        // module file (file stem `stdlib` → ext:ocaml:ocaml/stdlib.ml).
        let mut imports = vec![ImportEntry {
            imported_name: "Stdlib".to_string(),
            module_path: Some("stdlib".to_string()),
            alias: None,
            is_wildcard: true,
        }];

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

    fn detect_flow_emission(
        &self,
        _file_ctx: &FileContext,
        ref_ctx: &RefContext,
    ) -> Vec<crate::indexer::resolve::flow_emit::FlowEmission> {
        let r = &ref_ctx.extracted_ref;
        if r.kind != EdgeKind::Calls {
            return Vec::new();
        }
        let module = r.module.as_deref().unwrap_or("");
        let target = r.target_name.as_str();
        if let Some(em) = detect_ocaml_dream_route(module, target, &r.call_args) {
            return vec![em];
        }
        if let Some(em) = detect_ocaml_cohttp_producer(module, target, &r.call_args) {
            return vec![em];
        }
        if let Some(em) = detect_ocaml_caqti_emission(module, target) {
            return vec![em];
        }
        Vec::new()
    }
}

pub(crate) fn detect_ocaml_dream_route(
    module: &str,
    target_name: &str,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{
        ChannelRole, FlowEmission, HttpMethod, NamedChannelKind,
    };
    use crate::types::CallArg;
    // Dream.get "/x" handler / Opium `App.get "/x" handler`.
    let m_last = module.rsplit('.').next().unwrap_or(module);
    if !matches!(m_last, "Dream" | "App" | "Opium") {
        return None;
    }
    let method = match target_name {
        "get" => HttpMethod::Get,
        "post" => HttpMethod::Post,
        "put" => HttpMethod::Put,
        "delete" => HttpMethod::Delete,
        "patch" => HttpMethod::Patch,
        "head" => HttpMethod::Head,
        "options" => HttpMethod::Options,
        _ => return None,
    };
    let url = call_args.iter().find_map(|a| match a {
        CallArg::StringLit(s) if s.starts_with('/') => Some(s.as_str()),
        _ => None,
    })?;
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::HttpCall,
        name: crate::connectors::url_pattern::normalize(url),
        role: ChannelRole::Consumer,
        method: Some(method),
    streaming: None,
    })
}

pub(crate) fn detect_ocaml_cohttp_producer(
    module: &str,
    target_name: &str,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{
        ChannelRole, FlowEmission, HttpMethod, NamedChannelKind,
    };
    use crate::types::CallArg;
    if !module.contains("Cohttp") && !module.contains("Piaf") && !module.contains("Httpaf") {
        return None;
    }
    let method = match target_name {
        "get" | "call" => HttpMethod::Any,
        "post" => HttpMethod::Post,
        "put" => HttpMethod::Put,
        "delete" => HttpMethod::Delete,
        _ => return None,
    };
    let url = call_args.iter().find_map(|a| match a {
        CallArg::StringLit(s)
            if s.starts_with('/') || s.starts_with("http://") || s.starts_with("https://") =>
        {
            Some(s.as_str())
        }
        _ => None,
    })?;
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::HttpCall,
        name: crate::connectors::url_pattern::normalize(url),
        role: ChannelRole::Producer,
        method: Some(method),
    streaming: None,
    })
}

pub(crate) fn detect_ocaml_caqti_emission(
    module: &str,
    target_name: &str,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    detect_ocaml_caqti_with_imports(module, target_name, &[])
}

/// Caqti-style query operation detection that recognises module aliases.
///
/// OCaml code commonly aliases `Caqti_request.Infix` (or another Caqti
/// submodule) via `module Q = Caqti_request.Infix` or `open Caqti_lwt`,
/// and then writes `Db.find Q.find_user param`. The plain `module ==
/// "Db"` substring check in the original implementation was a known
/// false-positive source (matches `Database`, `Dbg`, `Adobe.Db`, …).
///
/// This variant checks the module name first against canonical Caqti
/// roots, then against the file's `open <Pkg>`/`module X = <Caqti...>`
/// alias declarations supplied in `aliases`. Each entry in `aliases`
/// is `(local_name, resolved_target)` — when `module` matches a
/// `local_name` whose `resolved_target` contains `Caqti`, treat the
/// call as a Caqti operation.
pub(crate) fn detect_ocaml_caqti_with_imports(
    module: &str,
    target_name: &str,
    aliases: &[(String, String)],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    let canonical_match = module.contains("Caqti")
        || matches!(module, "Db" | "Database" | "Repo" | "Q" | "Conn");
    let m_root = module.split('.').next().unwrap_or(module);
    let alias_match = aliases
        .iter()
        .any(|(local, target)| local == m_root && target.contains("Caqti"));
    if !canonical_match && !alias_match {
        return None;
    }
    let op = match target_name {
        "find" | "find_opt" | "collect_list" | "fold" | "iter" | "rev_collect_list" => {
            DbQueryOp::Select
        }
        "exec" => DbQueryOp::Other,
        _ => return None,
    };
    Some(FlowEmission::DbQuery {
        entity_name: "ml.*".to_string(),
        operation: op,
    })
}

#[cfg(test)]
#[path = "resolve_tests.rs"]
mod tests;
