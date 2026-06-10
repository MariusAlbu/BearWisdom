// =============================================================================
// languages/ada/hooks.rs — Ada engine hooks.
//
// Resolution runs entirely through the generic DefaultResolver driven by
// ADA_PROFILE data (kind table, ambient_namespace_prefixes, builtin_skip,
// name_normalization). There is no Ada-specific resolve_ref or classify_external
// any more. What remains are two engine *feeders* that data cannot yet express:
//   - build_file_context: Ada `with`/`use` clauses both become wildcard imports.
//   - detect_flow_emissions: GNATCOLL.SQL / Execute_Query DB-query flow.
// =============================================================================

use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{FileContext, ImportEntry, RefContext, SymbolLookup};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile, SymbolKind};

/// Given an Ada body path (`foo/bar.adb`), return the sibling spec path
/// (`foo/bar.ads`). Returns None for any non-.adb file.
pub(crate) fn spec_for_body(file_path: &str) -> Option<String> {
    let normalized = file_path.replace('\\', "/");
    if normalized.ends_with(".adb") {
        let stem = &normalized[..normalized.len() - 4];
        Some(format!("{stem}.ads"))
    } else {
        None
    }
}

pub(crate) fn detect_flow_inner(
    _file_ctx: &FileContext,
    ref_ctx: &RefContext,
) -> Vec<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    use crate::types::CallArg;
    let r = &ref_ctx.extracted_ref;
    if r.kind != EdgeKind::Calls {
        return Vec::new();
    }
    let target = r.target_name.as_str();
    // GNATCOLL.SQL.Exec / AdaSQL Execute_Query.
    if matches!(
        target,
        "Exec" | "Execute_Query" | "Execute" | "Query" | "Prepare"
    ) {
        let sql = r.call_args.iter().find_map(|a| match a {
            CallArg::StringLit(s) => Some(s.as_str()),
            _ => None,
        });
        if let Some(sql) = sql {
            let upper = sql.to_ascii_uppercase();
            let op = if upper.contains("INSERT INTO") {
                DbQueryOp::Insert
            } else if upper.contains("UPDATE ") {
                DbQueryOp::Update
            } else if upper.contains("DELETE FROM") {
                DbQueryOp::Delete
            } else if upper.contains(" FROM ") || upper.starts_with("SELECT") {
                DbQueryOp::Select
            } else {
                return Vec::new();
            };
            return vec![FlowEmission::DbQuery {
                entity_name: "ada.*".to_string(),
                operation: op,
            }];
        }
    }
    Vec::new()
}

pub(crate) fn build_file_context_inner(
    file: &ParsedFile,
    _project_ctx: Option<&ProjectContext>,
) -> FileContext {
    let mut imports = Vec::new();

    // Outermost package/namespace qname (e.g. `Alr.Commands.Run`). Ada
    // body/spec files declare exactly one top-level package.
    let file_namespace = file
        .symbols
        .iter()
        .find(|s| s.kind == SymbolKind::Namespace && s.parent_index.is_none())
        .map(|s| s.qualified_name.clone());

    for r in &file.refs {
        if r.kind != EdgeKind::Imports {
            continue;
        }
        // Both `with` and `use` clauses produce Imports edges.
        // package_renaming_declaration sets `module` to the renamed-target
        // package (for `package Trace renames Simple_Logging;` the ref
        // carries target_name="Trace" and module=Some("Simple_Logging")).
        let module_path = r.module.clone().unwrap_or_else(|| r.target_name.clone());
        imports.push(ImportEntry {
            imported_name: r.target_name.clone(),
            module_path: Some(module_path),
            alias: None,
            is_wildcard: true, // Ada `use` makes all names visible
        });
    }

    FileContext {
        file_path: file.path.clone(),
        language: "ada".to_string(),
        imports,
        file_namespace,
    }
}

pub struct AdaHooks;

impl LanguageEngineHooks for AdaHooks {
    fn detect_flow_emissions(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext<'_>,
        lookup: &dyn SymbolLookup,
    ) -> Vec<crate::indexer::resolve::flow_emit::FlowEmission> {
        let _ = lookup;
        detect_flow_inner(file_ctx, ref_ctx)
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<FileContext> {
        Some(build_file_context_inner(file, project_ctx))
    }
}

pub static ADA_HOOKS: AdaHooks = AdaHooks;
