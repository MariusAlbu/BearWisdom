// =============================================================================
// indexer/resolve/rules/php.rs — PHP resolution rules
//
// Scope rules for PHP (7.4+, 8.x):
//
//   1. Chain-aware resolution: walk MemberChain following field/return types.
//   2. Scope chain walk: innermost scope → outermost, try {scope}.{target}
//   3. Same-namespace resolution: types in the same namespace are visible
//      without `use` (mirrors C# same-namespace visibility).
//   4. Use statement resolution: `use App\Models\User;` makes `User` visible.
//   5. Fully qualified names: backslash-separated names resolve directly
//      (normalized to dotted form in the index).
//
// PHP import model:
//   The PHP extractor emits EdgeKind::Imports refs for `use` declarations:
//     use App\Models\User;         → target_name = "User",  module = "App\Models\User"
//     use App\Models\User as U;    → target_name = "U",     module = "App\Models\User"
//
//   PHP namespaces use backslash as separator. The index normalizes these
//   to dotted form (e.g., "App\Models\User" → "App.Models.User") to be
//   consistent with the rest of the resolvers. We accept both forms in lookups.
//
// Adding new PHP features:
//   - Trait use → add to build_file_context (already EdgeKind::Imports in extractor).
//   - Enum backed types → extractor emits TypeRef; scope chain handles them.
// =============================================================================

use super::super::engine::{
    FileContext, ImportEntry, LanguageResolver, RefContext, Resolution, SymbolInfo, SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, MemberChain, ParsedFile, SegmentKind};

/// PHP language resolver.
pub struct PhpResolver;

impl LanguageResolver for PhpResolver {
    fn language_ids(&self) -> &[&str] {
        &["php"]
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
        let mut imports = Vec::new();

        // Extract the current namespace from the first Namespace symbol.
        let file_namespace = file.symbols.iter().find_map(|sym| {
            if sym.kind == crate::types::SymbolKind::Namespace {
                Some(sym.qualified_name.clone())
            } else {
                None
            }
        });

        // Extract `use` declarations from EdgeKind::Imports refs.
        // PHP extractor emits:
        //   use App\Models\User;       → target_name = "User",  module = "App\Models\User"
        //   use App\Models\User as U;  → target_name = "U",     module = "App\Models\User"
        for r in &file.refs {
            if r.kind != EdgeKind::Imports {
                continue;
            }
            let module = r.module.as_deref().unwrap_or(&r.target_name);

            // Normalize backslash separators to dots for index lookup consistency.
            let normalized_module = normalize_php_ns(module);

            imports.push(ImportEntry {
                imported_name: r.target_name.clone(),
                module_path: Some(normalized_module),
                alias: None,
                // PHP `use` is always an exact type import, not a wildcard.
                is_wildcard: false,
            });
        }

        FileContext {
            file_path: file.path.clone(),
            language: "php".to_string(),
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

        // Skip import refs themselves — they're not symbol references.
        if edge_kind == EdgeKind::Imports {
            return None;
        }

        // Chain-aware resolution: if we have a structured MemberChain, walk it
        // step-by-step following field types.
        if let Some(chain) = &ref_ctx.extracted_ref.chain {
            if let Some(res) = resolve_via_chain(chain, edge_kind, file_ctx, ref_ctx, lookup) {
                return Some(res);
            }
        }

        // Normalize: strip `$this->` or `this.` prefix for member access.
        let effective_target = target
            .strip_prefix("$this->")
            .or_else(|| target.strip_prefix("this."))
            .unwrap_or(target);

        // Also normalize any backslash separators in the target itself.
        let normalized_target = normalize_php_ns(effective_target);
        let effective_target = normalized_target.as_str();

        // Step 1: Scope chain walk (innermost → outermost).
        // e.g., scope_chain = ["App.Controllers.UserController.store",
        //                       "App.Controllers.UserController",
        //                       "App.Controllers"]
        for scope in &ref_ctx.scope_chain {
            let candidate = format!("{scope}.{effective_target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if self.is_visible(file_ctx, ref_ctx, sym)
                    && kind_compatible(edge_kind, &sym.kind)
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "php_scope_chain",
                    });
                }
            }
        }

        // Step 2: Same-namespace resolution.
        // In PHP, classes in the same namespace are visible without `use`.
        if let Some(ns) = &file_ctx.file_namespace {
            let candidate = format!("{ns}.{effective_target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if self.is_visible(file_ctx, ref_ctx, sym)
                    && kind_compatible(edge_kind, &sym.kind)
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "php_same_namespace",
                    });
                }
            }
        }

        // Step 3: Use statement resolution.
        // `use App\Models\User;` → target "User" resolves to "App.Models.User"
        for import in &file_ctx.imports {
            if import.imported_name == effective_target {
                if let Some(module) = &import.module_path {
                    if let Some(sym) = lookup.by_qualified_name(module) {
                        if kind_compatible(edge_kind, &sym.kind) {
                            return Some(Resolution {
                                target_symbol_id: sym.id,
                                confidence: 1.0,
                                strategy: "php_use_statement",
                            });
                        }
                    }
                }
            }
        }

        // Step 4: Fully qualified name (target contains "\" or ".").
        if effective_target.contains('.') || effective_target.contains('\\') {
            if let Some(sym) = lookup.by_qualified_name(effective_target) {
                if kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "php_qualified_name",
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

        // Import refs (`use` statements) — classify the `use` as external if the namespace is.
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            let import_path = ref_ctx
                .extracted_ref
                .module
                .as_deref()
                .unwrap_or(target);
            let normalized = normalize_php_ns(import_path);
            if is_external_php_namespace(&normalized, project_ctx) {
                return Some(normalized);
            }
            return None;
        }

        // PHP built-in functions — always external.
        if is_php_builtin(target) {
            return Some("php_core".to_string());
        }

        // Check use statement list for external namespaces.
        let mut best: Option<&str> = None;

        for import in &file_ctx.imports {
            let ns = import.module_path.as_deref().unwrap_or("");
            if ns.is_empty() {
                continue;
            }

            if is_external_php_namespace(ns, project_ctx) {
                if best.is_none() || ns.len() > best.unwrap().len() {
                    best = Some(ns);
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
        match vis {
            "public" => true,
            "protected" => {
                // Accessible from same class or subclasses — approximate: allow.
                true
            }
            "private" => {
                // Only visible within the same file (same class).
                target.file_path == file_ctx.file_path
            }
            _ => true,
        }
    }
}

// ---------------------------------------------------------------------------
// Chain-aware resolution
// ---------------------------------------------------------------------------

/// Walk a MemberChain step-by-step, following field types to resolve the final segment.
///
/// For `$this->repo->findOne()` with chain `[this, repo, findOne]`:
/// 1. `this` → find enclosing class from scope_chain (e.g., "App.Controllers.UserController")
/// 2. `repo` → look up "App.Controllers.UserController.repo" field → declared_type = "UserRepo"
/// 3. `findOne` → look up "App.Controllers.UserRepo.findOne" → resolved!
fn resolve_via_chain(
    chain: &MemberChain,
    edge_kind: EdgeKind,
    file_ctx: &FileContext,
    ref_ctx: &RefContext,
    lookup: &dyn SymbolLookup,
) -> Option<Resolution> {
    let segments = &chain.segments;
    if segments.len() < 2 {
        return None;
    }

    // Phase 1: Determine the root type from the first segment.
    let root_type = match segments[0].kind {
        SegmentKind::SelfRef => find_enclosing_class(&ref_ctx.scope_chain, lookup),
        SegmentKind::Identifier => {
            let name = &segments[0].name;

            // Is it a known class/type? (static access: `ClassName::method()`)
            // PHP traits use "class" kind in the index.
            let is_type = lookup.by_name(name).iter().any(|s| {
                matches!(
                    s.kind.as_str(),
                    "class" | "interface" | "enum" | "type_alias"
                )
            });
            if is_type {
                Some(name.clone())
            } else {
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

    // Phase 2: Walk intermediate segments.
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

        // Try via use statement namespaces.
        let mut found = false;
        for import in &file_ctx.imports {
            if let Some(module) = &import.module_path {
                let qualified_member = format!("{module}.{member_qname}");
                if let Some(next_type) = lookup.field_type_name(&qualified_member) {
                    current_type = next_type.to_string();
                    found = true;
                    break;
                }
                if let Some(next_type) = lookup.return_type_name(&qualified_member) {
                    current_type = next_type.to_string();
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
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 1.0,
                strategy: "php_chain_resolution",
            });
        }
    }

    // Try via use-statement namespaces.
    for import in &file_ctx.imports {
        if let Some(module) = &import.module_path {
            let ns_candidate = format!("{module}.{candidate}");
            if let Some(sym) = lookup.by_qualified_name(&ns_candidate) {
                if kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 0.95,
                        strategy: "php_chain_resolution",
                    });
                }
            }
        }
    }

    for sym in lookup.by_name(&last.name) {
        if sym.qualified_name.starts_with(&current_type) && kind_compatible(edge_kind, &sym.kind) {
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 0.90,
                strategy: "php_chain_resolution",
            });
        }
    }

    None
}

/// Find the enclosing class/interface from the scope chain.
/// PHP traits use SymbolKind::Class in the index.
fn find_enclosing_class(scope_chain: &[String], lookup: &dyn SymbolLookup) -> Option<String> {
    for scope in scope_chain {
        if let Some(sym) = lookup.by_qualified_name(scope) {
            if matches!(sym.kind.as_str(), "class" | "interface") {
                return Some(scope.clone());
            }
        }
    }
    if scope_chain.len() >= 2 {
        return Some(scope_chain[scope_chain.len() - 2].clone());
    }
    scope_chain.last().cloned()
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::Calls => matches!(
            sym_kind,
            "method" | "function" | "constructor" | "test" | "property"
        ),
        EdgeKind::Inherits => matches!(sym_kind, "class"),
        EdgeKind::Implements => matches!(sym_kind, "interface"),
        // PHP traits use "class" kind in the index.
        EdgeKind::TypeRef => matches!(
            sym_kind,
            "class" | "interface" | "enum" | "type_alias" | "namespace"
        ),
        EdgeKind::Instantiates => matches!(sym_kind, "class"),
        _ => true,
    }
}

/// Normalize PHP namespace separator `\` to `.` for index consistency.
/// "App\\Models\\User" → "App.Models.User"
fn normalize_php_ns(ns: &str) -> String {
    // Trim leading backslash (global namespace qualifier: `\App\Models\User`).
    let trimmed = ns.trim_start_matches('\\');
    trimmed.replace('\\', ".")
}

/// Always-external PHP namespace roots (frameworks + major libraries).
const ALWAYS_EXTERNAL: &[&str] = &[
    "Illuminate",   // Laravel
    "Symfony",      // Symfony
    "Doctrine",     // Doctrine ORM
    "PHPUnit",      // PHPUnit
    "Psr",          // PSR interfaces
    "GuzzleHttp",   // Guzzle HTTP
    "Carbon",       // Carbon date
    "Monolog",      // Monolog logging
];

/// Check whether a PHP namespace (dotted form) is external.
fn is_external_php_namespace(ns: &str, project_ctx: Option<&ProjectContext>) -> bool {
    // Always-external first.
    for prefix in ALWAYS_EXTERNAL {
        if ns == *prefix || ns.starts_with(&format!("{prefix}.")) {
            return true;
        }
    }

    // Check ProjectContext (from composer.json).
    if let Some(ctx) = project_ctx {
        // composer.json package names like "laravel/framework" are stored
        // in external_prefixes. PHP package names often map to namespace roots
        // (e.g., "laravel/framework" → "Illuminate").
        return ctx.is_external_namespace(ns);
    }

    false
}

/// PHP built-in functions, Laravel Collection methods, and Eloquent model methods.
///
/// Covers the PHP standard library functions (always available without `use`),
/// plus the Laravel Collection fluent API and Eloquent ORM methods that appear
/// heavily in PHP project code but are never in the project's own symbol index.
fn is_php_builtin(name: &str) -> bool {
    let root = name.split(['.', ':']).next().unwrap_or(name);
    matches!(
        root,
        // Array functions
        "array_map"
            | "array_filter"
            | "array_reduce"
            | "array_merge"
            | "array_push"
            | "array_pop"
            | "array_shift"
            | "array_unshift"
            | "array_keys"
            | "array_values"
            | "array_unique"
            | "array_reverse"
            | "array_slice"
            | "array_splice"
            | "array_search"
            | "array_flip"
            | "array_walk"
            | "array_chunk"
            | "array_combine"
            | "array_diff"
            | "array_intersect"
            | "count"
            | "sizeof"
            | "in_array"
            | "sort"
            | "asort"
            | "ksort"
            | "usort"
            // String functions
            | "strlen"
            | "strpos"
            | "strrpos"
            | "substr"
            | "str_replace"
            | "str_contains"
            | "str_starts_with"
            | "str_ends_with"
            | "strtolower"
            | "strtoupper"
            | "trim"
            | "ltrim"
            | "rtrim"
            | "explode"
            | "implode"
            | "sprintf"
            | "printf"
            | "print"
            | "number_format"
            | "ucfirst"
            | "lcfirst"
            // General functions
            | "isset"
            | "empty"
            | "is_null"
            | "is_array"
            | "is_string"
            | "is_numeric"
            | "is_int"
            | "is_float"
            | "is_bool"
            | "is_object"
            | "json_encode"
            | "json_decode"
            | "var_dump"
            | "print_r"
            | "var_export"
            | "die"
            | "exit"
            | "header"
            | "setcookie"
            | "session_start"
            | "intval"
            | "floatval"
            | "strval"
            | "boolval"
            | "date"
            | "time"
            | "mktime"
            | "strtotime"
            | "file_get_contents"
            | "file_put_contents"
            | "file_exists"
            | "ob_start"
            | "ob_get_clean"
            | "class_exists"
            | "interface_exists"
            | "method_exists"
            | "property_exists"
            | "get_class"
            | "get_parent_class"
            | "is_a"
            | "instanceof"
            // Exception types (always available without import)
            | "Exception"
            | "RuntimeException"
            | "InvalidArgumentException"
            | "BadMethodCallException"
            | "LogicException"
            | "Throwable"
            | "Error"
            // Laravel Collection fluent API methods
            | "map"
            | "filter"
            | "where"
            | "first"
            | "last"
            | "each"
            | "pluck"
            | "collect"
            | "toArray"
            | "toJson"
            | "isEmpty"
            | "isNotEmpty"
            | "push"
            | "sortBy"
            | "groupBy"
            | "flatten"
            | "unique"
            | "values"
            | "keys"
            | "merge"
            | "reduce"
            | "reject"
            | "contains"
            | "sum"
            | "avg"
            | "min"
            | "max"
            | "chunk"
            | "take"
            | "skip"
            | "tap"
            | "pipe"
            // Eloquent ORM / Query Builder methods
            | "findOrFail"
            | "find"
            | "create"
            | "update"
            | "delete"
            | "save"
            | "refresh"
            | "orderBy"
            | "limit"
            | "offset"
            | "paginate"
            | "get"
            | "all"
            | "exists"
            | "doesntExist"
            | "with"
            | "has"
            | "whereHas"
            | "belongsTo"
            | "hasMany"
            | "hasOne"
            | "belongsToMany"
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "php_tests.rs"]
mod tests;
