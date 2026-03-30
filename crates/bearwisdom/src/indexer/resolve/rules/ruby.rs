// =============================================================================
// indexer/resolve/rules/ruby.rs — Ruby resolution rules
//
// Scope rules for Ruby:
//
//   1. Chain-aware resolution: walk MemberChain following field/return types.
//   2. Scope chain walk: innermost scope → outermost, try {scope}.{target}
//   3. Same-file / same-module resolution: Ruby classes/modules in the same
//      file are visible to each other at module scope.
//   4. Constant resolution: Ruby classes and modules are constants.
//      An unqualified `Foo` is looked up in the nesting chain (scope chain).
//
// Ruby import model:
//   The Ruby extractor emits EdgeKind::Imports refs for require statements:
//     require 'rails'          → target_name = "rails",   module = None
//     require_relative './foo' → target_name = "foo",     module = "./foo"
//
//   require gives access to library constants/classes by their top-level name
//   (e.g., requiring 'rails' makes `Rails::Application` available).
//   require_relative brings in local file symbols.
//
// Adding new Ruby features:
//   - autoload → add to build_file_context.
//   - include / extend (mixins) → update extractor to emit TypeRef with the
//     module name; this resolver picks them up via scope chain.
// =============================================================================

use super::super::engine::{
    FileContext, ImportEntry, LanguageResolver, RefContext, Resolution, SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, MemberChain, ParsedFile, SegmentKind};

/// Ruby language resolver.
pub struct RubyResolver;

impl LanguageResolver for RubyResolver {
    fn language_ids(&self) -> &[&str] {
        &["ruby"]
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
        let mut imports = Vec::new();

        // Extract require / require_relative imports from EdgeKind::Imports refs.
        // Ruby extractor emits:
        //   require 'foo'          → target_name = "foo",        module = None (bare gem)
        //   require_relative './x' → target_name = "x",          module = "./x" (relative)
        for r in &file.refs {
            if r.kind != EdgeKind::Imports {
                continue;
            }
            let module_path = r.module.clone().or_else(|| Some(r.target_name.clone()));
            imports.push(ImportEntry {
                imported_name: r.target_name.clone(),
                module_path,
                alias: None,
                is_wildcard: false,
            });
        }

        // Ruby has no file-level package/namespace declaration in the same sense —
        // classes are constants defined at file scope. The outermost module name
        // (if any) is extracted from the first Namespace/Module symbol.
        // Ruby modules are represented as Namespace in the SymbolKind enum.
        let file_namespace = file.symbols.iter().find_map(|sym| {
            if sym.kind == crate::types::SymbolKind::Namespace {
                Some(sym.qualified_name.clone())
            } else {
                None
            }
        });

        FileContext {
            file_path: file.path.clone(),
            language: "ruby".to_string(),
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

        // Skip import refs — they're not symbol references.
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
        // e.g., scope_chain = ["MyModule::MyClass::my_method", "MyModule::MyClass", "MyModule"]
        // Try "MyModule::MyClass::my_method::Target", etc.
        // Note: Ruby uses "::" as the namespace separator in qualified names.
        // The index may store qualified names with either "." or "::" depending on the extractor.
        for scope in &ref_ctx.scope_chain {
            // Try dotted form (how the extractor stores qualified names).
            let candidate = format!("{scope}.{target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "ruby_scope_chain",
                    });
                }
            }
        }

        // Step 2: Same-file resolution.
        // In Ruby, classes/methods in the same file are visible at module scope.
        for sym in lookup.in_file(&file_ctx.file_path) {
            if sym.name == *target && kind_compatible(edge_kind, &sym.kind) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "ruby_same_file",
                });
            }
        }

        // Step 3: Same-module resolution.
        // If we're inside a module, sibling constants are visible unqualified.
        if let Some(ns) = &file_ctx.file_namespace {
            let candidate = format!("{ns}.{target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "ruby_same_module",
                    });
                }
            }
        }

        // Step 4: Fully qualified name (target contains "::" or ".").
        if target.contains("::") || target.contains('.') {
            // Normalize "::" to "." for index lookup.
            let normalized = target.replace("::", ".");
            if let Some(sym) = lookup.by_qualified_name(&normalized) {
                if kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "ruby_qualified_name",
                    });
                }
            }
            if let Some(sym) = lookup.by_qualified_name(target) {
                if kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "ruby_qualified_name",
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

        // Import refs (require statements) — classify the require itself if external.
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            let require_path = ref_ctx.extracted_ref.module.as_deref().unwrap_or(target);
            if is_external_ruby_require(require_path, project_ctx) {
                return Some(require_path.to_string());
            }
            return None;
        }

        // Ruby built-ins — always external.
        if is_ruby_builtin(target) {
            return Some("ruby_core".to_string());
        }

        // Check file's require list for matching external gems.
        // If the name was brought in by a gem require, it's external.
        for import in &file_ctx.imports {
            let Some(module_path) = &import.module_path else {
                continue;
            };

            // Only bare (non-relative) requires can be gems.
            if module_path.starts_with('.') {
                continue;
            }

            if is_external_ruby_require(module_path, project_ctx) {
                return Some(module_path.clone());
            }
        }

        None
    }

    // is_visible: default (always true) is correct for Ruby.
    // Ruby access control (private/protected) is enforced at runtime, not
    // at the call site — all indexed symbols are accessible for our purposes.
}

// ---------------------------------------------------------------------------
// Chain-aware resolution
// ---------------------------------------------------------------------------

/// Walk a MemberChain step-by-step, following field types to resolve the final segment.
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

    // Phase 1: Determine the root type from the first segment.
    let root_type = match segments[0].kind {
        SegmentKind::SelfRef => find_enclosing_class(&ref_ctx.scope_chain, lookup),
        SegmentKind::Identifier => {
            let name = &segments[0].name;

            // Is it a known class/namespace? (constant access: `ClassName.method`)
            // Ruby modules are stored as "namespace".
            let is_type = lookup.by_name(name).iter().any(|s| {
                matches!(s.kind.as_str(), "class" | "namespace" | "interface" | "type_alias")
            });
            if is_type {
                Some(name.clone())
            } else {
                // Is it an instance variable / field on the enclosing class?
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

        // Try by_name fallback.
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
                strategy: "ruby_chain_resolution",
            });
        }
    }

    for sym in lookup.by_name(&last.name) {
        if sym.qualified_name.starts_with(&current_type) && kind_compatible(edge_kind, &sym.kind) {
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 0.90,
                strategy: "ruby_chain_resolution",
            });
        }
    }

    None
}

/// Find the enclosing class/namespace from the scope chain.
/// Ruby modules are stored as `namespace` in the index.
fn find_enclosing_class(scope_chain: &[String], lookup: &dyn SymbolLookup) -> Option<String> {
    for scope in scope_chain {
        if let Some(sym) = lookup.by_qualified_name(scope) {
            // "namespace" covers both Ruby modules and packages.
            if matches!(sym.kind.as_str(), "class" | "namespace" | "interface") {
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
        // Ruby modules (mixins) are stored as "namespace" in the index.
        EdgeKind::Implements => matches!(sym_kind, "namespace" | "interface"),
        EdgeKind::TypeRef => matches!(
            sym_kind,
            "class" | "namespace" | "interface" | "enum" | "type_alias"
        ),
        EdgeKind::Instantiates => matches!(sym_kind, "class"),
        _ => true,
    }
}

/// Ruby stdlib module names — always external regardless of Gemfile.
const RUBY_STDLIB: &[&str] = &[
    "json",
    "net/http",
    "uri",
    "fileutils",
    "set",
    "csv",
    "yaml",
    "erb",
    "cgi",
    "digest",
    "base64",
    "open-uri",
    "socket",
    "logger",
    "optparse",
    "benchmark",
    "tempfile",
    "pathname",
    "date",
    "time",
    "pp",
    "forwardable",
    "singleton",
    "ostruct",
    "struct",
];

/// Check whether a require path refers to an external gem or stdlib.
fn is_external_ruby_require(require_path: &str, project_ctx: Option<&ProjectContext>) -> bool {
    // Stdlib — always external.
    if RUBY_STDLIB.contains(&require_path) {
        return true;
    }
    // Strip ruby_gems stored in project_ctx.
    // Since ProjectContext uses external_prefixes for .NET/Java namespaces,
    // Ruby gem names are stored in ruby_gems on the context.
    // Use the generic external_prefixes check if gems were stored there.
    if let Some(ctx) = project_ctx {
        // The Ruby resolver stores gem names as external_prefixes entries
        // (root name, e.g., "rails", "devise").
        let gem_root = require_path.split('/').next().unwrap_or(require_path);
        if ctx.external_prefixes.contains(gem_root) {
            return true;
        }
    }
    false
}

/// Ruby built-in methods and kernel functions always in scope.
///
/// Covers Object, Enumerable, Array, String, Hash built-ins, Rails/ActiveSupport
/// convenience methods, and Kernel functions. Used in `infer_external_namespace`
/// to classify unresolved calls as `ruby_core` rather than leaving them unknown.
fn is_ruby_builtin(name: &str) -> bool {
    let root = name.split('.').next().unwrap_or(name);
    matches!(
        root,
        // Kernel / top-level functions
        "puts"
            | "print"
            | "p"
            | "pp"
            | "raise"
            | "require"
            | "require_relative"
            | "sleep"
            | "rand"
            | "exit"
            | "abort"
            | "lambda"
            | "proc"
            | "block_given?"
            | "yield"
            // Class-definition helpers (always available at class scope)
            | "include"
            | "extend"
            | "attr_reader"
            | "attr_writer"
            | "attr_accessor"
            | "define_method"
            // Object methods (on every Ruby object)
            | "nil?"
            | "is_a?"
            | "respond_to?"
            | "send"
            | "class"
            | "freeze"
            | "frozen?"
            | "dup"
            | "clone"
            | "to_s"
            | "to_i"
            | "to_f"
            | "to_a"
            | "to_h"
            | "inspect"
            | "hash"
            | "equal?"
            // Enumerable methods (mixed into Array, Hash, Range, etc.)
            | "each"
            | "map"
            | "select"
            | "reject"
            | "find"
            | "detect"
            | "collect"
            | "reduce"
            | "inject"
            | "any?"
            | "all?"
            | "none?"
            | "count"
            | "min"
            | "max"
            | "sort"
            | "sort_by"
            | "group_by"
            | "flat_map"
            | "zip"
            | "first"
            | "last"
            | "take"
            | "drop"
            | "each_with_object"
            | "each_with_index"
            // Array methods
            | "push"
            | "pop"
            | "shift"
            | "unshift"
            | "flatten"
            | "compact"
            | "uniq"
            | "reverse"
            | "join"
            | "length"
            | "size"
            | "empty?"
            | "include?"
            | "index"
            | "sample"
            | "shuffle"
            // String methods
            | "strip"
            | "chomp"
            | "chop"
            | "gsub"
            | "sub"
            | "split"
            | "upcase"
            | "downcase"
            | "capitalize"
            | "start_with?"
            | "end_with?"
            | "match?"
            | "scan"
            | "encode"
            | "bytes"
            | "chars"
            | "lines"
            // Hash methods
            | "keys"
            | "values"
            | "merge"
            | "merge!"
            | "fetch"
            | "delete"
            | "has_key?"
            | "has_value?"
            | "each_pair"
            | "transform_keys"
            | "transform_values"
            | "slice"
            | "except"
            // Rails/ActiveSupport convenience methods
            | "present?"
            | "blank?"
            | "presence"
            | "try"
            | "in?"
            // Top-level constants always available
            | "Array"
            | "Integer"
            | "Float"
            | "String"
            | "Hash"
            | "Kernel"
            | "Object"
            | "BasicObject"
            | "Module"
            | "Class"
            | "Comparable"
            | "Enumerable"
            | "Enumerator"
            | "nil"
            | "true"
            | "false"
            | "self"
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "ruby_tests.rs"]
mod tests;
