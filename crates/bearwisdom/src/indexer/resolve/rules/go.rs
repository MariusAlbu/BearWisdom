// =============================================================================
// indexer/resolve/rules/go.rs — Go resolution rules
//
// Scope rules for Go:
//
//   1. Same-package resolution: all symbols declared in files with the same
//      package name are visible to each other without any import. This is the
//      dominant rule for intra-package calls.
//   2. Import resolution: `import "pkg/path"` makes exported symbols available
//      as `lastSegment.Symbol`. The ref target_name holds just the symbol name;
//      the Go extractor does NOT emit a module hint on call refs for selector
//      expressions — only the field identifier (method/function name) is stored.
//   3. Scope chain walk: for methods defined on a receiver type, walk the
//      qualified-name scope chain trying `{scope}.{target}`.
//   4. Fully qualified: dotted target_name resolved directly against the index.
//
// Go visibility:
//   Exported = first character uppercase → Public in our model.
//   Unexported = first character lowercase → Private.
//   Cross-package access requires the target to be exported (Public).
//
// Import format from the Go extractor (emit_import_ref):
//   target_name = last path segment (e.g., "gin" for "github.com/gin-gonic/gin")
//   module      = full import path    (e.g., "github.com/gin-gonic/gin")
//
// Call format from the Go extractor (extract_call_ref):
//   For `gin.Default()`:  target_name = "Default", module = None
//   For `fmt.Println()`:  target_name = "Println", module = None
//   For `localFunc()`:    target_name = "localFunc", module = None
//
// Key constraint: the extractor drops the package qualifier from call refs
// (it captures only the field_identifier). So "gin.Default()" becomes
// target_name = "Default" with no module. Disambiguation happens via imports.
// =============================================================================

use super::super::engine::{
    FileContext, ImportEntry, LanguageResolver, RefContext, Resolution, SymbolInfo, SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, MemberChain, ParsedFile, SegmentKind};

/// Go language resolver.
pub struct GoResolver;

impl LanguageResolver for GoResolver {
    fn language_ids(&self) -> &[&str] {
        &["go"]
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
        let mut imports = Vec::new();

        // Derive the package name from symbols' scope_path or qualified_name prefix.
        // The Go extractor sets scope_path = Some(package_name) for top-level symbols,
        // and qualified_name = "package.SymbolName". We take the first segment.
        let file_namespace = extract_package_name(file);

        // Build import entries from EdgeKind::Imports refs.
        // The Go extractor emits:
        //   target_name = last path segment (the package alias by convention)
        //   module      = full import path
        for r in &file.refs {
            if r.kind != EdgeKind::Imports {
                continue;
            }
            let full_path = match &r.module {
                Some(m) => m.clone(),
                None => r.target_name.clone(),
            };

            // Detect alias, dot-import, and blank import by examining target_name
            // relative to the last segment of the full path.
            let last_segment = full_path.rsplit('/').next().unwrap_or(&full_path);

            // Blank import (`import _ "path"`) — side effects only, skip.
            if r.target_name == "_" {
                continue;
            }

            // Dot import (`import . "path"`) — all exported names enter scope directly.
            let is_dot_import = r.target_name == ".";

            // The alias used in source code: explicit alias overrides the last segment.
            let alias = if is_dot_import || r.target_name == last_segment {
                None
            } else {
                Some(r.target_name.clone())
            };

            imports.push(ImportEntry {
                imported_name: alias.clone().unwrap_or_else(|| last_segment.to_string()),
                module_path: Some(full_path),
                alias,
                // Dot imports bring all exported names into scope without qualification.
                // Regular imports require `pkg.Symbol` — not a wildcard in our model.
                is_wildcard: is_dot_import,
            });
        }

        FileContext {
            file_path: file.path.clone(),
            language: "go".to_string(),
            imports,
            file_namespace,
        }
    }

    fn resolve(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        let target = &ref_ctx.extracted_ref.target_name;
        let edge_kind = ref_ctx.extracted_ref.kind;

        // Skip import refs — they declare scope, not symbol references.
        if edge_kind == EdgeKind::Imports {
            return None;
        }

        // Chain-aware resolution: if we have a structured MemberChain, walk it
        // step-by-step following field types.
        if let Some(chain) = &ref_ctx.extracted_ref.chain {
            if let Some(res) = resolve_via_chain(chain, edge_kind, ref_ctx, lookup) {
                return Some(res);
            }
        }

        // Step 1: Scope chain walk (innermost → outermost).
        // Handles methods calling sibling methods on the same receiver:
        //   scope_chain = ["main.Server", "main"]
        //   try "main.Server.Foo", "main.Foo"
        for scope in &ref_ctx.scope_chain {
            let candidate = format!("{scope}.{target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if self.is_visible(file_ctx, ref_ctx, sym)
                    && kind_compatible(edge_kind, &sym.kind)
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "go_scope_chain",
                    });
                }
            }
        }

        // Step 2: Same-package resolution.
        // All symbols with the same package name are visible without import.
        // Try `{package}.{target}`.
        if let Some(pkg) = &file_ctx.file_namespace {
            let candidate = format!("{pkg}.{target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if self.is_visible(file_ctx, ref_ctx, sym)
                    && kind_compatible(edge_kind, &sym.kind)
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "go_same_package",
                    });
                }
            }

            // Also check: if file_namespace is available, look at all symbols
            // by simple name and prefer ones in the same package.
            let candidates = lookup.by_name(target);
            for sym in candidates {
                if sym_package(sym) == pkg.as_str()
                    && self.is_visible(file_ctx, ref_ctx, sym)
                    && kind_compatible(edge_kind, &sym.kind)
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "go_same_package_by_name",
                    });
                }
            }
        }

        // Step 3: Import-based resolution.
        // For `gin.Default()`, the extractor emits target_name = "Default".
        // We need to find an import whose alias/last_segment maps to a package
        // that exports a symbol named `target`.
        for import in &file_ctx.imports {
            let Some(full_path) = &import.module_path else {
                continue;
            };

            // Dot import: all exported names from this package are directly visible.
            if import.is_wildcard {
                let last_seg = full_path.rsplit('/').next().unwrap_or(full_path.as_str());
                let candidate = format!("{last_seg}.{target}");
                if let Some(sym) = lookup.by_qualified_name(&candidate) {
                    if self.is_visible(file_ctx, ref_ctx, sym)
                        && kind_compatible(edge_kind, &sym.kind)
                    {
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: 1.0,
                            strategy: "go_dot_import",
                        });
                    }
                }
                continue;
            }

            // The package alias used in source: explicit alias, otherwise last segment.
            let pkg_alias = import
                .alias
                .as_deref()
                .unwrap_or_else(|| full_path.rsplit('/').next().unwrap_or(full_path.as_str()));

            // The symbol index uses the Go package name (last segment of import path
            // by convention, unless the package declares a different name).
            // Try the conventional qualified name: `{last_segment}.{target}`.
            let last_seg = full_path.rsplit('/').next().unwrap_or(full_path.as_str());
            let candidate = format!("{last_seg}.{target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if self.is_visible(file_ctx, ref_ctx, sym)
                    && kind_compatible(edge_kind, &sym.kind)
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "go_import",
                    });
                }
            }

            // If an explicit alias was used and differs from last_seg, also try alias.
            if pkg_alias != last_seg {
                let candidate = format!("{pkg_alias}.{target}");
                if let Some(sym) = lookup.by_qualified_name(&candidate) {
                    if self.is_visible(file_ctx, ref_ctx, sym)
                        && kind_compatible(edge_kind, &sym.kind)
                    {
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: 1.0,
                            strategy: "go_import_alias",
                        });
                    }
                }
            }
        }

        // Step 4: Fully qualified name (target contains dots, e.g., "pkg.Func").
        if target.contains('.') {
            if let Some(sym) = lookup.by_qualified_name(target) {
                if kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "go_qualified_name",
                    });
                }
            }
        }

        // Could not resolve deterministically — fall back to heuristic.
        None
    }

    fn infer_external_namespace(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;

        // Import refs (e.g., `import "fmt"`, `import "mymodule/pkg"`).
        // These are namespace declarations, not symbol references — they don't
        // map to a single target symbol. Classify them all with their module path
        // so they move out of unresolved_refs (we know what they are).
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            let import_path = ref_ctx.extracted_ref.module.as_deref().unwrap_or(target);
            return Some(import_path.to_string());
        }

        // Go built-in functions and types — always external (runtime/stdlib).
        if is_go_builtin(target) {
            return Some("builtin".to_string());
        }

        // Go composite literal types: []string, map[string]int, []*Foo, etc.
        if is_go_composite_type(target) {
            return Some("builtin".to_string());
        }

        // For non-import refs, "external namespace" means the import path of
        // the package this ref likely comes from. Only exported (capitalized)
        // names can come from external packages.
        let is_exported = target.chars().next().is_some_and(|c| c.is_uppercase());
        if !is_exported {
            return None;
        }

        let mut best: Option<&str> = None;

        for import in &file_ctx.imports {
            let Some(full_path) = &import.module_path else {
                continue;
            };

            let external = match project_ctx {
                Some(ctx) => ctx.is_external_go_import(full_path),
                None => is_external_go_import_fallback(full_path),
            };

            if external {
                // Prefer longer (more specific) paths.
                if best.is_none() || full_path.len() > best.unwrap().len() {
                    best = Some(full_path.as_str());
                }
            }
        }

        best.map(|s| s.to_string())
    }

    fn is_visible(
        &self,
        file_ctx: &FileContext,
        _ref_ctx: &RefContext,
        target: &SymbolInfo,
    ) -> bool {
        let vis = target.visibility.as_deref().unwrap_or("public");

        // Private (unexported) symbols are only visible within the same package,
        // which in Go means files in the same directory with the same package name.
        // We approximate with same file_path prefix (same directory).
        if vis == "private" {
            // Same file is always fine.
            if target.file_path == file_ctx.file_path {
                return true;
            }
            // Same package: compare directories.
            let target_dir = parent_dir(&target.file_path);
            let source_dir = parent_dir(&file_ctx.file_path);
            return target_dir == source_dir;
        }

        // Public (exported) symbols are always visible from anywhere.
        true
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extract the Go package name from a parsed file.
///
/// Strategy: look at the scope_path of the first non-import symbol.
/// The extractor sets scope_path = Some(package_name) for all top-level symbols,
/// and qualified_name = "package.Name" for all symbols. We take the first segment
/// of any symbol's qualified_name.
fn extract_package_name(file: &ParsedFile) -> Option<String> {
    for sym in &file.symbols {
        // The qualified_name is "pkg.Name" — take everything before the first dot.
        if let Some(dot) = sym.qualified_name.find('.') {
            let pkg = &sym.qualified_name[..dot];
            if !pkg.is_empty() {
                return Some(pkg.to_string());
            }
        }
        // If no dot (bare name with empty prefix), fall back to scope_path.
        if let Some(ref sp) = sym.scope_path {
            if !sp.is_empty() {
                return Some(sp.split('.').next().unwrap_or(sp.as_str()).to_string());
            }
        }
    }
    None
}

/// Extract the Go package name from a symbol's qualified_name.
/// "main.Server" → "main", "handlers.Handler" → "handlers".
fn sym_package(sym: &SymbolInfo) -> &str {
    sym.qualified_name
        .split('.')
        .next()
        .unwrap_or(sym.qualified_name.as_str())
}

/// Return the directory portion of a file path (everything up to the last '/').
fn parent_dir(path: &str) -> &str {
    match path.rfind('/') {
        Some(pos) => &path[..pos],
        None => "",
    }
}

/// Check whether a Go import path is external (heuristic: host with a dot).
/// Used when no ProjectContext is available.
fn is_external_go_import_fallback(import_path: &str) -> bool {
    let first_segment = import_path.split('/').next().unwrap_or(import_path);
    first_segment.contains('.')
}

/// Go built-in functions and types that are always in scope without import.
/// These come from the `builtin` pseudo-package.
fn is_go_builtin(name: &str) -> bool {
    matches!(
        name,
        "len" | "cap" | "make" | "new" | "append" | "copy" | "delete"
            | "close" | "panic" | "recover" | "print" | "println"
            | "complex" | "real" | "imag" | "clear" | "min" | "max"
            // Built-in type conversions used as calls
            | "string" | "int" | "int8" | "int16" | "int32" | "int64"
            | "uint" | "uint8" | "uint16" | "uint32" | "uint64"
            | "float32" | "float64" | "byte" | "rune" | "bool"
            | "error" | "any" | "comparable"
    )
}

/// Detect Go composite literal types that the extractor captures as target_name.
/// Examples: `[]string`, `map[string]int`, `[]*Foo`, `[]tests.ApiScenario`.
fn is_go_composite_type(name: &str) -> bool {
    name.starts_with("[]")
        || name.starts_with("[]*")
        || name.starts_with("map[")
        || name.starts_with("chan ")
        || name.starts_with("*[")
}

// ---------------------------------------------------------------------------
// Chain-aware resolution
// ---------------------------------------------------------------------------

/// Walk a MemberChain step-by-step, following field types to resolve the final segment.
///
/// For `s.repo.FindOne()` with chain `[s, repo, FindOne]`:
/// 1. `s` (Identifier) → look up as a field on the enclosing receiver type
///    (e.g., scope_chain contains "main.Server" → look for "main.Server.s")
/// 2. `repo` → look up "ResolvedType.repo" field → field_type_name = "UserRepo"
/// 3. `FindOne` → look up "UserRepo.FindOne" in the symbol index → resolved!
///
/// Go has no `this`/`self` keyword — the first segment is always an identifier.
fn resolve_via_chain(
    chain: &MemberChain,
    edge_kind: EdgeKind,
    ref_ctx: &RefContext,
    lookup: &dyn SymbolLookup,
) -> Option<Resolution> {
    let segments = &chain.segments;
    if segments.len() < 2 {
        // Single-segment chains are handled by the regular scope-chain strategies.
        return None;
    }

    // Phase 1: Determine the root type from the first segment.
    // In Go, the first segment is always an Identifier (receiver var, package name, or
    // local variable). No SelfRef — Go has no `this`.
    let root_type = match segments[0].kind {
        SegmentKind::Identifier => {
            let name = &segments[0].name;

            // Is it a known type? (static/package-level access: `pkg.Func()`)
            let is_type = lookup.by_name(name).iter().any(|s| {
                matches!(
                    s.kind.as_str(),
                    "struct" | "interface" | "enum" | "type_alias"
                )
            });
            if is_type {
                Some(name.clone())
            } else {
                // Is it a field/variable on the enclosing receiver type?
                let mut found = None;
                for scope in &ref_ctx.scope_chain {
                    let field_qname = format!("{scope}.{name}");
                    if let Some(type_name) = lookup.field_type_name(&field_qname) {
                        found = Some(type_name.to_string());
                        break;
                    }
                }
                found.or_else(|| segments[0].declared_type.clone())
            }
        }
        _ => None,
    };

    let mut current_type = root_type?;

    // Phase 2: Walk intermediate segments, following field types or return types.
    for seg in &segments[1..segments.len() - 1] {
        let member_qname = format!("{current_type}.{}", seg.name);

        if let Some(next_type) = lookup.field_type_name(&member_qname) {
            current_type = next_type.to_string();
            continue;
        }
        if let Some(next_type) = lookup.return_type_name(&member_qname) {
            current_type = next_type.to_string();
            continue;
        }

        // Try by_name fallback with namespace prefix.
        let mut found = false;
        for sym in lookup.by_name(&seg.name) {
            if sym.qualified_name.starts_with(&current_type) {
                if let Some(ft) = lookup.field_type_name(&sym.qualified_name) {
                    current_type = ft.to_string();
                    found = true;
                    break;
                }
                if let Some(rt) = lookup.return_type_name(&sym.qualified_name) {
                    current_type = rt.to_string();
                    found = true;
                    break;
                }
            }
        }
        if found {
            continue;
        }

        // Lost the chain — can't determine the next type.
        return None;
    }

    // Phase 3: Resolve the final segment on the resolved type.
    let last = &segments[segments.len() - 1];
    let candidate = format!("{current_type}.{}", last.name);

    // Direct qualified name match.
    if let Some(sym) = lookup.by_qualified_name(&candidate) {
        if kind_compatible(edge_kind, &sym.kind) {
            tracing::debug!(
                strategy = "go_chain_resolution",
                chain_len = segments.len(),
                resolved_type = %current_type,
                target = %last.name,
                "resolved"
            );
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 1.0,
                strategy: "go_chain_resolution",
            });
        }
    }

    // Try by name, scoped to the resolved type.
    for sym in lookup.by_name(&last.name) {
        if sym.qualified_name.starts_with(&current_type) && kind_compatible(edge_kind, &sym.kind) {
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 0.95,
                strategy: "go_chain_resolution",
            });
        }
    }

    None
}

/// Find the enclosing struct/interface name from the scope chain.
///
/// scope_chain for a method `(s *Server) Handle()` is
/// `["main.Server.Handle", "main.Server", "main"]` — we want `"main.Server"`.
#[allow(dead_code)]
fn find_enclosing_class(scope_chain: &[String], lookup: &dyn SymbolLookup) -> Option<String> {
    for scope in scope_chain {
        if let Some(sym) = lookup.by_qualified_name(scope) {
            if matches!(sym.kind.as_str(), "struct" | "interface") {
                return Some(scope.clone());
            }
        }
    }
    // Fallback: the penultimate scope entry is often the receiver type.
    if scope_chain.len() >= 2 {
        return Some(scope_chain[scope_chain.len() - 2].clone());
    }
    scope_chain.last().cloned()
}

/// Check that the edge kind is compatible with the symbol kind.
fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::Calls => matches!(
            sym_kind,
            "method" | "function" | "constructor" | "test"
        ),
        EdgeKind::Inherits => matches!(sym_kind, "struct" | "interface"),
        EdgeKind::Implements => matches!(sym_kind, "interface"),
        EdgeKind::TypeRef => matches!(
            sym_kind,
            "class" | "struct" | "interface" | "enum" | "type_alias"
        ),
        EdgeKind::Instantiates => matches!(sym_kind, "struct" | "class"),
        _ => true,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "go_tests.rs"]
mod tests;
