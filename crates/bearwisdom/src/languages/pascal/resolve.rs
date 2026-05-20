// =============================================================================
// pascal/resolve.rs — Pascal/Delphi resolution rules
//
// Scope rules for Pascal/Delphi:
//
//   1. Scope chain walk: innermost procedure/function → class → unit.
//   2. Same-file resolution: all declarations in the same unit are visible.
//   3. Import-based resolution:
//        `uses Unit1, Unit2;` → all public symbols from each unit enter scope
//   4. Include-file wildcard: Pascal codebases split a unit across multiple
//        `{$I sub.inc}` files. When a wildcard import for unit "Foo" is active,
//        also match symbols in files whose stem begins with "foo_" (e.g.
//        "foo_bar.inc"). This covers the FPC/Lazarus convention of one `.pas`
//        shell plus many `unit_section.inc` implementation files.
//
// The extractor emits EdgeKind::Imports with:
//   target_name = unit name (e.g., "SysUtils", "Classes")
//   module      = None (Pascal `uses` clauses always name the unit directly)
// =============================================================================

#[cfg(test)]
#[path = "resolve_tests.rs"]
mod tests;

use super::predicates;
use crate::indexer::resolve::engine::{
    self, FileContext, ImportEntry, LanguageResolver, RefContext, Resolution, SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

/// Pascal/Delphi language resolver.
pub struct PascalResolver;

impl LanguageResolver for PascalResolver {
    fn language_ids(&self) -> &[&str] {
        &["pascal", "delphi"]
    }

    
    fn build_file_context(
        &self,
        file: &ParsedFile,
        project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
        build_file_context_inner(file, project_ctx)
    }
fn resolve(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            return None;
        }

        let edge_kind = ref_ctx.extracted_ref.kind;

        // Pascal is case-insensitive: check same-file with lowercased comparison
        // before delegating to resolve_common (which is case-sensitive).
        let target_lower = ref_ctx.extracted_ref.target_name.to_lowercase();
        for sym in lookup.in_file(&file_ctx.file_path) {
            if sym.name.to_lowercase() == target_lower
                && predicates::kind_compatible(edge_kind, &sym.kind)
            {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "pascal_same_file",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
        }

        // Wildcard-import lookup extended for the FPC include-split convention:
        // a unit "Foo" may be split across "foo_bar.inc", "foo_baz.inc", etc.
        // resolve_common's import step uses case-sensitive file_stem_matches and
        // only matches the exact stem; this pass adds both case-insensitive exact
        // matching and the "foo_*" prefix variant.
        let target_orig = &ref_ctx.extracted_ref.target_name;
        if let Some(res) =
            resolve_pascal_wildcard(edge_kind, target_orig, &target_lower, file_ctx, lookup)
        {
            return Some(res);
        }

        engine::resolve_common("pascal", file_ctx, ref_ctx, lookup, predicates::kind_compatible)
    }

}

pub(crate) fn detect_pascal_http_producer(
    target: &str,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{
        ChannelRole, FlowEmission, HttpMethod, NamedChannelKind,
    };
    use crate::types::CallArg;
    // TIdHTTP / TNetHTTPClient / THttpClient methods.
    let method = match target {
        "Get" => HttpMethod::Get,
        "Post" => HttpMethod::Post,
        "Put" => HttpMethod::Put,
        "Patch" => HttpMethod::Patch,
        "Delete" => HttpMethod::Delete,
        "Head" => HttpMethod::Head,
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

pub(crate) fn detect_pascal_db_query(
    target: &str,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    use crate::types::CallArg;
    // FireDAC / ZeosLib / TADOQuery `.Open` / `.ExecSQL` with `.SQL.Text := 'SELECT ...'`.
    // Cheap heuristic: if any arg is a SQL-shaped string, emit.
    if !matches!(target, "ExecSQL" | "Open" | "Execute" | "Query") {
        return None;
    }
    let sql = call_args.iter().find_map(|a| match a {
        CallArg::StringLit(s) => Some(s.as_str()),
        _ => None,
    })?;
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
        return None;
    };
    Some(FlowEmission::DbQuery {
        entity_name: "pas.*".to_string(),
        operation: op,
    })
}

/// Returns `true` when the file's imports include at least one Delphi
/// dotted-namespace unit. These prefixes are exclusive to Delphi (Embarcadero
/// RAD Studio / VCL / FMX); FPC and Lazarus do not emit them.
pub(super) fn is_delphi_namespaced_file(file_ctx: &FileContext) -> bool {
    const DELPHI_PREFIXES: &[&str] = &[
        "vcl.", "winapi.", "firedac.", "data.", "fmx.", "xml.",
        "system.generics.", "system.classes", "system.sysutils",
        "system.win.", "system.ioutils", "system.dateutils",
        "system.contnrs", "system.strutils", "system.variants",
        "system.math", "system.types", "system.uriparser",
    ];
    file_ctx.imports.iter().any(|imp| {
        imp.module_path.as_deref().map_or(false, |m| {
            let ml = m.to_lowercase();
            DELPHI_PREFIXES.iter().any(|p| ml.starts_with(p))
        })
    })
}

/// Wildcard-import resolution for Pascal, covering both case-insensitivity and
/// the FPC include-split convention.
///
/// For each wildcard import with module path `M` (lowercased to `m`), searches
/// for symbols whose name matches `target` (case-insensitively) and that live
/// in a file whose stem either equals `m` exactly or starts with `m_`. The
/// `m_` prefix covers the FPC convention where unit `CastleUtils` is split
/// across `castleutils_math.inc`, `castleutils_filenames.inc`, etc.
pub(super) fn resolve_pascal_wildcard(
    edge_kind: EdgeKind,
    target_orig: &str,
    target_lower: &str,
    file_ctx: &FileContext,
    lookup: &dyn SymbolLookup,
) -> Option<Resolution> {
    // by_name is case-sensitive. Pascal is case-insensitive, so probe multiple
    // casing forms: original, lowercase, first-char-uppercase (TitleCase), and
    // all-uppercase. This covers refs like `integer` (→ `Integer` in FPC source)
    // and `hresult` (→ `HRESULT` in FPC source).
    let target_upper = target_orig.to_uppercase();
    let target_title: String = {
        let mut c = target_orig.chars();
        match c.next() {
            None => String::new(),
            Some(f) => f.to_uppercase().collect::<String>() + &target_lower[f.len_utf8()..],
        }
    };

    let probes: [&str; 4] = [
        target_orig,
        target_lower,
        target_title.as_str(),
        target_upper.as_str(),
    ];

    for import in &file_ctx.imports {
        if !import.is_wildcard {
            continue;
        }
        let Some(module_path) = &import.module_path else {
            continue;
        };
        let mod_lower = module_path.to_lowercase();

        for probe in probes {
            for sym in lookup.by_name(probe) {
                if sym.name.to_lowercase() != target_lower {
                    continue;
                }
                if !predicates::kind_compatible(edge_kind, &sym.kind) {
                    continue;
                }
                if pascal_stem_matches(&sym.file_path, &mod_lower) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 0.95,
                        strategy: "pascal_wildcard_import",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }
        }
    }

    None
}

/// Returns true when `file_path`'s stem (lowercased, extension stripped) either
/// equals `module_lower` exactly or starts with `module_lower` followed by `_`.
///
/// This covers both the direct case (`castleutils.pas` for import `CastleUtils`)
/// and the FPC include-split case (`castleutils_math.inc` for the same import).
pub(super) fn pascal_stem_matches(file_path: &str, module_lower: &str) -> bool {
    let normalized = file_path.replace('\\', "/");
    let basename = normalized.rsplit('/').next().unwrap_or(&normalized);
    // Strip extension: take everything before the last '.'
    let stem = basename.rsplit_once('.').map(|(s, _)| s).unwrap_or(basename);
    let stem_lower = stem.to_lowercase();
    stem_lower == module_lower || stem_lower.starts_with(&format!("{module_lower}_"))
}

pub(crate) fn detect_flow_inner(
    _file_ctx: &FileContext,
    ref_ctx: &RefContext,
) -> Vec<crate::indexer::resolve::flow_emit::FlowEmission> {
    let r = &ref_ctx.extracted_ref;
    if r.kind != EdgeKind::Calls {
        return Vec::new();
    }
    let target = r.target_name.as_str();
    if let Some(em) = detect_pascal_http_producer(target, &r.call_args) {
        return vec![em];
    }
    if let Some(em) = detect_pascal_db_query(target, &r.call_args) {
        return vec![em];
    }
    Vec::new()
}

pub(crate) fn build_file_context_inner(
    file: &ParsedFile,
    _project_ctx: Option<&ProjectContext>,
) -> FileContext {
    let mut imports = Vec::new();

    for r in &file.refs {
        if r.kind != EdgeKind::Imports {
            continue;
        }
        // `uses UnitName` → each unit is a wildcard import (all public names visible).
        imports.push(ImportEntry {
            imported_name: r.target_name.clone(),
            module_path: Some(r.target_name.clone()),
            alias: None,
            is_wildcard: true,
        });
    }

    FileContext {
        file_path: file.path.clone(),
        language: "pascal".to_string(),
        imports,
        file_namespace: None,
    }
}
