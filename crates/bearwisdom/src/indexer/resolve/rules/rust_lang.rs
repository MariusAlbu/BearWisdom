// =============================================================================
// indexer/resolve/rules/rust_lang.rs — Rust resolution rules
//
// Scope rules for Rust:
//
//   1. Chain-aware resolution: walk MemberChain step-by-step following
//      field types / return types.
//   2. Scope chain walk: innermost → outermost, try {scope}.{target}.
//   3. Same-module resolution: symbols whose qualified_name shares the same
//      module path are visible without a `use` statement.
//   4. Import-based resolution: `use foo::bar::Baz` makes `Baz` visible.
//   5. Crate-qualified: `foo::bar::Baz` with `::` separators → convert to
//      dot form and resolve directly.
//
// Rust visibility:
//   `pub`      → Public  (visible everywhere)
//   `pub(crate)` → Internal (visible within the crate)
//   `pub(super)` → similar to Internal; approximated as "same dir"
//   (none)     → Private (visible in the same module only)
//
// Import format from the Rust extractor:
//   For `use serde::Deserialize;`:
//     target_name = "Deserialize", module = "serde"
//   For `use crate::models::User;`:
//     target_name = "User",        module = "crate::models"
//   EdgeKind::Imports is used for use-declarations.
//
// Call/type-ref format:
//   `User::new()` → target_name = "User::new" or "new", module = None
//   `SomeStruct`  → target_name = "SomeStruct",            module = None
//
// Key constraint: The extractor may represent `::` paths either as
// `target_name` with embedded `::` OR as a simple name with a `module`
// field. Both forms are handled here.
// =============================================================================

use super::super::engine::{
    FileContext, ImportEntry, LanguageResolver, RefContext, Resolution, SymbolInfo, SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, MemberChain, ParsedFile, SegmentKind};

/// Rust language resolver.
pub struct RustResolver;

impl LanguageResolver for RustResolver {
    fn language_ids(&self) -> &[&str] {
        &["rust"]
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
        let mut imports = Vec::new();

        // Derive the module path for this file. The Rust extractor sets
        // scope_path on top-level symbols to reflect the module path
        // (e.g., "crate::models" for a symbol in src/models.rs).
        // We take it from the first top-level symbol's scope_path.
        let file_namespace = extract_module_path(file);

        // Build import entries from EdgeKind::Imports refs.
        // The Rust extractor emits one ref per `use` item brought into scope:
        //   use serde::Deserialize;
        //     → ref { target_name: "Deserialize", module: Some("serde"), kind: Imports }
        //   use crate::models::User;
        //     → ref { target_name: "User", module: Some("crate::models"), kind: Imports }
        //   use std::collections::HashMap;
        //     → ref { target_name: "HashMap", module: Some("std::collections"), kind: Imports }
        for r in &file.refs {
            if r.kind != EdgeKind::Imports {
                continue;
            }

            let module_path = r.module.clone().or_else(|| {
                // If no module field, try splitting target_name on "::"
                // e.g., target_name = "serde::Deserialize"
                if r.target_name.contains("::") {
                    let (mod_part, _name) = r.target_name.rsplit_once("::")?;
                    Some(mod_part.to_string())
                } else {
                    None
                }
            });

            // The imported name is the last segment of the path.
            let imported_name = if r.target_name.contains("::") {
                r.target_name
                    .rsplit("::")
                    .next()
                    .unwrap_or(&r.target_name)
                    .to_string()
            } else {
                r.target_name.clone()
            };

            // Wildcard import: `use foo::bar::*`
            let is_wildcard = imported_name == "*";

            imports.push(ImportEntry {
                imported_name,
                module_path,
                alias: None,
                is_wildcard,
            });
        }

        FileContext {
            file_path: file.path.clone(),
            language: "rust".to_string(),
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

        // Rust stdlib builtins are never in our index — fast exit.
        if is_rust_builtin(target) {
            return None;
        }

        // Chain-aware resolution.
        if let Some(chain) = &ref_ctx.extracted_ref.chain {
            if let Some(res) = resolve_via_chain(chain, edge_kind, ref_ctx, lookup) {
                return Some(res);
            }
        }

        // Normalize `::` separators to `.` for index lookup.
        let normalized = normalize_path(target);
        let effective_target = &normalized;

        // Step 1: Scope chain walk (innermost → outermost).
        for scope in &ref_ctx.scope_chain {
            let candidate = format!("{scope}.{effective_target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if self.is_visible(file_ctx, ref_ctx, sym)
                    && kind_compatible(edge_kind, &sym.kind)
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "rust_scope_chain",
                    });
                }
            }
        }

        // Step 2: Same-module resolution.
        // Symbols in the same module are visible without `use`.
        if let Some(module) = &file_ctx.file_namespace {
            let candidate = format!("{module}.{effective_target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if self.is_visible(file_ctx, ref_ctx, sym)
                    && kind_compatible(edge_kind, &sym.kind)
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "rust_same_module",
                    });
                }
            }

            // By simple name, preferring same module.
            let candidates = lookup.by_name(effective_target);
            for sym in candidates {
                if sym_module(sym) == module.as_str()
                    && self.is_visible(file_ctx, ref_ctx, sym)
                    && kind_compatible(edge_kind, &sym.kind)
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "rust_same_module_by_name",
                    });
                }
            }
        }

        // Step 3: Import-based resolution.
        // `use foo::bar::Baz` → look for `Baz` in the symbol index,
        // preferring symbols whose qualified_name starts with the import's module path.
        for import in &file_ctx.imports {
            if import.is_wildcard {
                // Wildcard: find anything in the imported module matching the name.
                if let Some(ref mod_path) = import.module_path {
                    let dot_mod = normalize_path(mod_path);
                    let candidate = format!("{dot_mod}.{effective_target}");
                    if let Some(sym) = lookup.by_qualified_name(&candidate) {
                        if self.is_visible(file_ctx, ref_ctx, sym)
                            && kind_compatible(edge_kind, &sym.kind)
                        {
                            return Some(Resolution {
                                target_symbol_id: sym.id,
                                confidence: 1.0,
                                strategy: "rust_wildcard_import",
                            });
                        }
                    }
                }
                continue;
            }

            // Named import: the imported_name must match.
            if import.imported_name != *effective_target {
                continue;
            }

            let Some(ref mod_path) = import.module_path else {
                continue;
            };

            let dot_mod = normalize_path(mod_path);

            // Try {module}.{name} — most common Rust qualified name form.
            let candidate = format!("{dot_mod}.{effective_target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if self.is_visible(file_ctx, ref_ctx, sym)
                    && kind_compatible(edge_kind, &sym.kind)
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "rust_import",
                    });
                }
            }

            // Also try: just the name, scoped to the module prefix.
            for sym in lookup.by_name(effective_target) {
                if sym.qualified_name.starts_with(dot_mod.as_str())
                    && self.is_visible(file_ctx, ref_ctx, sym)
                    && kind_compatible(edge_kind, &sym.kind)
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 0.95,
                        strategy: "rust_import_prefix",
                    });
                }
            }
        }

        // Step 4: Crate-qualified resolution.
        // `crate::foo::Bar` or `foo::Bar` with `::` separators.
        if target.contains("::") || effective_target.contains('.') {
            if let Some(sym) = lookup.by_qualified_name(effective_target) {
                if kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "rust_qualified_name",
                    });
                }
            }

            // Strip leading "crate." and try again.
            let stripped = effective_target
                .strip_prefix("crate.")
                .unwrap_or(effective_target);
            if stripped != effective_target {
                if let Some(sym) = lookup.by_qualified_name(stripped) {
                    if kind_compatible(edge_kind, &sym.kind) {
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: 1.0,
                            strategy: "rust_qualified_name_stripped",
                        });
                    }
                }
            }
        }

        None
    }

    fn infer_external_namespace(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;

        // Import refs: `use serde::Deserialize` → classify by the first crate segment.
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            let import_path = ref_ctx.extracted_ref.module.as_deref().unwrap_or(target);
            // First segment of a `::` path identifies the crate.
            let first = import_path.split("::").next().unwrap_or(import_path);
            match first {
                "crate" | "self" | "super" => return None, // internal
                "std" | "core" | "alloc" => return Some("std".to_string()),
                name => {
                    let is_ext = match project_ctx {
                        Some(ctx) => ctx.is_external_rust_crate(name),
                        None => true, // conservative: treat as external
                    };
                    if is_ext {
                        return Some(first.to_string());
                    }
                    return None;
                }
            }
        }

        // Builtin calls / stdlib items — always external.
        if is_rust_builtin(target) {
            return Some("std".to_string());
        }

        // For non-import refs, check if the target came from an external import.
        // Walk the file's import list for a matching imported_name.
        let normalized = normalize_path(target);
        let simple = normalized.split('.').next_back().unwrap_or(&normalized);

        for import in &file_ctx.imports {
            if import.imported_name != simple {
                continue;
            }
            let Some(ref mod_path) = import.module_path else {
                continue;
            };
            let first = mod_path.split("::").next().unwrap_or(mod_path);
            match first {
                "crate" | "self" | "super" => continue,
                "std" | "core" | "alloc" => return Some("std".to_string()),
                name => {
                    let is_ext = match project_ctx {
                        Some(ctx) => ctx.is_external_rust_crate(name),
                        None => true,
                    };
                    if is_ext {
                        return Some(first.to_string());
                    }
                }
            }
        }

        None
    }

    fn is_visible(
        &self,
        file_ctx: &FileContext,
        _ref_ctx: &RefContext,
        target: &SymbolInfo,
    ) -> bool {
        let vis = target.visibility.as_deref().unwrap_or("public");

        match vis {
            "private" => {
                // Private in Rust = visible only within the same module (same file
                // or same module path). Approximate: same file is always ok.
                target.file_path == file_ctx.file_path
            }
            "internal" => {
                // pub(crate) / pub(super) — same directory as an approximation.
                let target_dir = parent_dir(&target.file_path);
                let source_dir = parent_dir(&file_ctx.file_path);
                target_dir == source_dir || target.file_path == file_ctx.file_path
            }
            _ => true, // public
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extract the module path from a parsed Rust file.
/// The Rust extractor sets scope_path on top-level symbols to the module path,
/// e.g., "crate::models" or "crate::api::handlers".
fn extract_module_path(file: &ParsedFile) -> Option<String> {
    for sym in &file.symbols {
        if let Some(ref sp) = sym.scope_path {
            if !sp.is_empty() {
                // scope_path may use `::` or `.` separators — normalize to `.`
                let dot_path = sp.replace("::", ".");
                return Some(dot_path);
            }
        }
        // If no scope_path, check qualified_name prefix.
        if let Some(dot) = sym.qualified_name.rfind('.') {
            let prefix = &sym.qualified_name[..dot];
            if !prefix.is_empty() {
                return Some(prefix.to_string());
            }
        }
    }
    None
}

/// Normalize a Rust `::` path to the `.`-separated form used in the symbol index.
/// "crate::models::User" → "crate.models.User"
/// "serde::Deserialize"  → "serde.Deserialize"
fn normalize_path(s: &str) -> String {
    s.replace("::", ".")
}

/// Extract the module prefix from a symbol's qualified_name.
/// "crate.models.User" → "crate.models"
fn sym_module(sym: &SymbolInfo) -> &str {
    match sym.qualified_name.rfind('.') {
        Some(pos) => &sym.qualified_name[..pos],
        None => "",
    }
}

/// Return the directory portion of a file path.
fn parent_dir(path: &str) -> &str {
    match path.rfind('/') {
        Some(pos) => &path[..pos],
        None => "",
    }
}

/// Rust built-in names that are always in scope (prelude, macros, primitive ops,
/// Iterator/Option/Result adapters, Vec/str methods, and Diesel ORM methods).
fn is_rust_builtin(name: &str) -> bool {
    // Strip leading `::` if present
    let name = name.trim_start_matches("::");
    // Also handle single-segment names in chains like "Some", "Ok"
    let simple = name.rsplit("::").next().unwrap_or(name);
    matches!(
        simple,
        // Prelude types / enums
        "Some"
            | "None"
            | "Ok"
            | "Err"
            | "Box"
            | "Vec"
            | "String"
            | "Option"
            | "Result"
            // Common trait methods (always available via auto-deref/prelude)
            | "clone"
            | "to_string"
            | "to_owned"
            | "into"
            | "from"
            | "default"
            | "fmt"
            // Iterator adapters / consumers
            | "map"
            | "filter"
            | "collect"
            | "iter"
            | "into_iter"
            | "unwrap"
            | "expect"
            | "ok"
            | "err"
            | "chain"
            | "enumerate"
            | "peekable"
            | "skip"
            | "take"
            | "zip"
            | "by_ref"
            | "rev"
            | "cycle"
            | "sum"
            | "product"
            | "all"
            | "any"
            | "find"
            | "position"
            | "max_by_key"
            | "min_by_key"
            | "for_each"
            | "flat_map"
            | "inspect"
            | "partition"
            | "unzip"
            | "fold"
            | "scan"
            | "take_while"
            | "skip_while"
            // Option / Result combinators
            | "unwrap_or_default"
            | "unwrap_or"
            | "unwrap_or_else"
            | "ok_or"
            | "ok_or_else"
            | "and_then"
            | "or_else"
            | "map_or"
            | "map_or_else"
            | "as_ref"
            | "as_mut"
            | "is_some"
            | "is_none"
            | "is_ok"
            | "is_err"
            | "transpose"
            | "flatten"
            // String / str methods
            | "as_str"
            | "as_bytes"
            | "to_lowercase"
            | "to_uppercase"
            | "trim"
            | "trim_start"
            | "trim_end"
            | "starts_with"
            | "ends_with"
            | "contains"
            | "replace"
            | "replacen"
            | "split"
            | "rsplit"
            | "splitn"
            | "lines"
            | "chars"
            | "bytes"
            // Vec / slice methods
            | "len"
            | "is_empty"
            | "push"
            | "pop"
            | "insert"
            | "remove"
            | "extend"
            | "truncate"
            | "drain"
            | "retain"
            | "dedup"
            | "sort_by"
            | "sort_by_key"
            | "binary_search"
            | "windows"
            | "chunks"
            | "split_at"
            | "get"
            // Diesel ORM DSL methods
            | "select"
            | "left_join"
            | "inner_join"
            | "or"
            | "and"
            | "set"
            | "on"
            | "first"
            | "load"
            | "get_result"
            | "execute"
            | "eq"
            | "ne"
            | "gt"
            | "lt"
            // Macros
            | "println"
            | "eprintln"
            | "format"
            | "write"
            | "writeln"
            | "assert"
            | "assert_eq"
            | "assert_ne"
            | "panic"
            | "todo"
            | "unimplemented"
            | "unreachable"
            | "vec"
            | "dbg"
            | "print"
            | "eprint"
    )
}

/// Check that the edge kind is compatible with the symbol kind.
fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::Calls => matches!(
            sym_kind,
            "method" | "function" | "constructor" | "test"
        ),
        EdgeKind::Inherits => matches!(sym_kind, "struct" | "interface" | "trait"),
        EdgeKind::Implements => matches!(sym_kind, "interface" | "trait"),
        EdgeKind::TypeRef => matches!(
            sym_kind,
            "class" | "struct" | "interface" | "enum" | "type_alias" | "trait"
        ),
        EdgeKind::Instantiates => matches!(sym_kind, "struct" | "class"),
        _ => true,
    }
}

// ---------------------------------------------------------------------------
// Chain-aware resolution
// ---------------------------------------------------------------------------

/// Walk a MemberChain step-by-step following field/return types.
///
/// For `self.repo.find_one()` with chain `[self, repo, find_one]`:
/// 1. `self` → find the enclosing struct/impl from scope_chain
/// 2. `repo` → look up "StructName.repo" field → field_type_name = "UserRepo"
/// 3. `find_one` → look up "UserRepo.find_one" → resolved
fn resolve_via_chain(
    chain: &MemberChain,
    edge_kind: EdgeKind,
    ref_ctx: &RefContext,
    lookup: &dyn SymbolLookup,
) -> Option<Resolution> {
    let segments = &chain.segments;
    if segments.len() < 2 {
        return None;
    }

    // Phase 1: Determine the root type.
    let root_type = match segments[0].kind {
        SegmentKind::SelfRef => {
            // `self` → find the enclosing struct/impl from scope_chain.
            find_enclosing_type(&ref_ctx.scope_chain, lookup)
        }
        SegmentKind::Identifier => {
            let name = &segments[0].name;

            // Is it a known type (static/enum access: `MyEnum::Variant`, `MyStruct::new()`)?
            let is_type = lookup.by_name(name).iter().any(|s| {
                matches!(
                    s.kind.as_str(),
                    "struct" | "enum" | "trait" | "type_alias" | "class"
                )
            });
            if is_type {
                Some(normalize_path(name))
            } else {
                // Is it a field on the enclosing type?
                let mut found = None;
                for scope in &ref_ctx.scope_chain {
                    let field_qname = format!("{scope}.{name}");
                    if let Some(type_name) = lookup.field_type_name(&field_qname) {
                        found = Some(normalize_path(type_name));
                        break;
                    }
                }
                found.or_else(|| segments[0].declared_type.as_ref().map(|t| normalize_path(t)))
            }
        }
        _ => None,
    };

    let mut current_type = root_type?;

    // Phase 2: Walk intermediate segments.
    for seg in &segments[1..segments.len() - 1] {
        let member_qname = format!("{current_type}.{}", seg.name);

        if let Some(next_type) = lookup.field_type_name(&member_qname) {
            current_type = normalize_path(next_type);
            continue;
        }
        if let Some(next_type) = lookup.return_type_name(&member_qname) {
            current_type = normalize_path(next_type);
            continue;
        }

        let mut found = false;
        for sym in lookup.by_name(&seg.name) {
            if sym.qualified_name.starts_with(&current_type) {
                if let Some(ft) = lookup.field_type_name(&sym.qualified_name) {
                    current_type = normalize_path(ft);
                    found = true;
                    break;
                }
                if let Some(rt) = lookup.return_type_name(&sym.qualified_name) {
                    current_type = normalize_path(rt);
                    found = true;
                    break;
                }
            }
        }
        if found {
            continue;
        }

        return None;
    }

    // Phase 3: Resolve the final segment.
    let last = &segments[segments.len() - 1];
    let candidate = format!("{current_type}.{}", last.name);

    if let Some(sym) = lookup.by_qualified_name(&candidate) {
        if kind_compatible(edge_kind, &sym.kind) {
            tracing::debug!(
                strategy = "rust_chain_resolution",
                chain_len = segments.len(),
                resolved_type = %current_type,
                target = %last.name,
                "resolved"
            );
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 1.0,
                strategy: "rust_chain_resolution",
            });
        }
    }

    for sym in lookup.by_name(&last.name) {
        if sym.qualified_name.starts_with(&current_type) && kind_compatible(edge_kind, &sym.kind) {
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 0.95,
                strategy: "rust_chain_resolution",
            });
        }
    }

    None
}

/// Find the enclosing struct/impl/trait name from the scope chain.
/// scope_chain = ["crate.handlers.MyHandler.process", "crate.handlers.MyHandler",
///                "crate.handlers", "crate"]
/// → we want "crate.handlers.MyHandler"
fn find_enclosing_type(scope_chain: &[String], lookup: &dyn SymbolLookup) -> Option<String> {
    for scope in scope_chain {
        if let Some(sym) = lookup.by_qualified_name(scope) {
            if matches!(sym.kind.as_str(), "struct" | "enum" | "trait" | "class") {
                return Some(scope.clone());
            }
        }
    }
    // Fallback: the penultimate scope is often the impl type.
    if scope_chain.len() >= 2 {
        return Some(scope_chain[scope_chain.len() - 2].clone());
    }
    scope_chain.last().cloned()
}
