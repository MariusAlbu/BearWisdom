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

    fn infer_external_namespace(
        &self,
        _file_ctx: &FileContext,
        ref_ctx: &RefContext,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        // Pascal identifiers are case-insensitive; fold both sides so that
        // `SIZEOF`, `fillchar`, etc. classify identically to `SizeOf`.
        let target_lower = ref_ctx.extracted_ref.target_name.to_lowercase();
        let keywords = super::keywords::KEYWORDS;
        if keywords.iter().any(|k| k.to_lowercase() == target_lower) {
            return Some("primitive".to_string());
        }
        None
    }

    fn infer_external_namespace_with_lookup(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        project_ctx: Option<&ProjectContext>,
        lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        // Keyword check first (does not need the index).
        if let Some(ns) = self.infer_external_namespace(file_ctx, ref_ctx, project_ctx) {
            return Some(ns);
        }

        // Classify bare RTL/VCL/LCL calls that resolve to the FPC runtime
        // walker's ext:pascal: virtual paths. Pascal is case-insensitive but
        // FPC source files use canonical mixed case (e.g. `NativeInt`, `HRESULT`)
        // while project code may use any casing variant. by_name is an exact
        // (case-sensitive) lookup, so we probe multiple forms:
        //   - original casing from the project ref
        //   - fully lowercased (finds all-lowercase defs)
        //   - first-char-uppercased (finds TitleCase defs like `Integer`)
        //   - fully uppercased (finds ALLCAPS defs like `HRESULT`)
        let target = &ref_ctx.extracted_ref.target_name;
        let target_lower = target.to_lowercase();
        let target_upper = target.to_uppercase();
        let target_title: String = {
            let mut c = target.chars();
            match c.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().collect::<String>() + &target_lower[f.len_utf8()..],
            }
        };

        for probe in [
            target.as_str(),
            target_lower.as_str(),
            target_title.as_str(),
            target_upper.as_str(),
        ] {
            for sym in lookup.by_name(probe) {
                if sym.file_path.starts_with("ext:pascal:")
                    && sym.name.to_lowercase() == target_lower
                {
                    return Some("fpc-runtime".to_string());
                }
            }
        }

        // Delphi-exclusive namespace detection: when the source file imports
        // units from Delphi's dotted-package namespaces (`Vcl.*`, `Winapi.*`,
        // `FireDAC.*`, `System.*`, `Data.*`, `FMX.*`, `Xml.*`), it is a
        // Delphi/VCL project. FPC does not use these namespace prefixes; they
        // cannot appear in FPC source. Any ref that remains unresolved in such
        // a file is a Delphi SDK or VCL symbol that the FPC walker cannot
        // supply. Classify it as `delphi-vcl` so it lands in
        // `unresolved_external` rather than `unresolved`.
        if is_delphi_namespaced_file(file_ctx) {
            return Some("delphi-vcl".to_string());
        }

        None
    }
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
