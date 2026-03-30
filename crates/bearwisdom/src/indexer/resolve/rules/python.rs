// =============================================================================
// indexer/resolve/rules/python.rs — Python resolution rules
//
// Scope rules for Python:
//
//   1. Chain-aware resolution: walk MemberChain step-by-step following
//      field types / return types.
//   2. Scope chain walk: innermost → outermost, try {scope}.{target}.
//   3. Same-file resolution: symbols defined in the same file are visible
//      at module scope without any import.
//   4. Import-based resolution: `from app.models import User` → `User` resolves.
//   5. Module-qualified: `models.User` → look up via imports.
//
// Python import forms:
//   `import os`               → import_name = "os",   module = None
//   `from foo import Bar`     → import_name = "Bar",  module = "foo"
//   `from foo.bar import Baz` → import_name = "Baz",  module = "foo.bar"
//   `import foo as f`         → import_name = "f",    module = "foo", alias = "f"
//
// The extractor emits EdgeKind::Imports for `import` and `from ... import`
// statements, with:
//   target_name = the bound local name (or module for bare `import`)
//   module      = the source module path (for `from ... import`)
//
// `self` is handled exactly like TypeScript's `this`: SelfRef segments
// trigger find_enclosing_class which walks the scope_chain for a class.
// =============================================================================

use super::super::engine::{
    FileContext, ImportEntry, LanguageResolver, RefContext, Resolution, SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, MemberChain, ParsedFile, SegmentKind};
use tracing::debug;

/// Python language resolver.
pub struct PythonResolver;

impl LanguageResolver for PythonResolver {
    fn language_ids(&self) -> &[&str] {
        &["python"]
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
        let mut imports = Vec::new();

        // Collect import entries from EdgeKind::Imports refs.
        //
        // The Python extractor emits:
        //   `import os`
        //     → ref { target_name: "os", module: None,    kind: Imports }
        //   `from foo.bar import Baz`
        //     → ref { target_name: "Baz", module: "foo.bar", kind: Imports }
        //   `from . import something` (relative)
        //     → ref { target_name: "something", module: ".", kind: Imports }
        for r in &file.refs {
            if r.kind != EdgeKind::Imports {
                continue;
            }

            // Bare `import os`: module_path = "os", imported_name = "os"
            // `from foo import Bar`: module_path = "foo", imported_name = "Bar"
            let module_path = r.module.clone().or_else(|| Some(r.target_name.clone()));
            let imported_name = r.target_name.clone();
            let is_wildcard = imported_name == "*";

            imports.push(ImportEntry {
                imported_name,
                module_path,
                alias: None,
                is_wildcard,
            });
        }

        // Python has no explicit file-level namespace — identity is the file path.
        FileContext {
            file_path: file.path.clone(),
            language: "python".to_string(),
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
        let target = &ref_ctx.extracted_ref.target_name;
        let edge_kind = ref_ctx.extracted_ref.kind;

        // Skip import refs.
        if edge_kind == EdgeKind::Imports {
            return None;
        }

        // Python builtins are never in our index.
        if is_python_builtin(target) {
            return None;
        }

        // Chain-aware resolution.
        if let Some(chain) = &ref_ctx.extracted_ref.chain {
            if let Some(res) = resolve_via_chain(chain, edge_kind, ref_ctx, lookup) {
                return Some(res);
            }
        }

        // If the ref carries a module path, it came from an import statement.
        if let Some(module) = &ref_ctx.extracted_ref.module {
            if is_relative_import(module) {
                // Relative import — try to resolve in the target file.
                for sym in lookup.in_file(module) {
                    if sym.name == *target && kind_compatible(edge_kind, &sym.kind) {
                        debug!(
                            strategy = "python_import_file",
                            file = %module,
                            target = %target,
                            "resolved"
                        );
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: 1.0,
                            strategy: "python_import_file",
                        });
                    }
                }

                let candidate = format!("{module}.{target}");
                if let Some(sym) = lookup.by_qualified_name(&candidate) {
                    if kind_compatible(edge_kind, &sym.kind) {
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: 1.0,
                            strategy: "python_import",
                        });
                    }
                }
            }
            // External package or unresolvable — skip.
            return None;
        }

        // Strip `self.` prefix — `self.method` → `method`, scope_chain handles it.
        let effective_target = target.strip_prefix("self.").unwrap_or(target);

        // Step 1: Scope chain walk.
        for scope in &ref_ctx.scope_chain {
            let candidate = format!("{scope}.{effective_target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if kind_compatible(edge_kind, &sym.kind) {
                    debug!(
                        strategy = "python_scope_chain",
                        candidate = %candidate,
                        "resolved"
                    );
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "python_scope_chain",
                    });
                }
            }
        }

        // Step 2: Same-file resolution.
        for sym in lookup.in_file(&file_ctx.file_path) {
            if sym.name == effective_target && kind_compatible(edge_kind, &sym.kind) {
                debug!(
                    strategy = "python_same_file",
                    qualified_name = %sym.qualified_name,
                    "resolved"
                );
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "python_same_file",
                });
            }
        }

        // Step 3: Fully qualified name (dotted target like "models.User").
        if effective_target.contains('.') {
            if let Some(sym) = lookup.by_qualified_name(effective_target) {
                if kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "python_qualified_name",
                    });
                }
            }

            // Dotted: split into module alias + symbol, look up via imports.
            if let Some(dot) = effective_target.find('.') {
                let alias = &effective_target[..dot];
                let rest = &effective_target[dot + 1..];

                // Find the import whose imported_name matches the alias.
                for import in &file_ctx.imports {
                    if import.imported_name != alias {
                        continue;
                    }
                    let Some(ref mod_path) = import.module_path else {
                        continue;
                    };

                    let candidate = format!("{mod_path}.{rest}");
                    if let Some(sym) = lookup.by_qualified_name(&candidate) {
                        if kind_compatible(edge_kind, &sym.kind) {
                            return Some(Resolution {
                                target_symbol_id: sym.id,
                                confidence: 1.0,
                                strategy: "python_module_qualified",
                            });
                        }
                    }

                    // Also try searching by name within that module.
                    let method_name = rest.split('.').next().unwrap_or(rest);
                    for sym in lookup.by_name(method_name) {
                        if sym.qualified_name.starts_with(mod_path.as_str())
                            && kind_compatible(edge_kind, &sym.kind)
                        {
                            return Some(Resolution {
                                target_symbol_id: sym.id,
                                confidence: 0.90,
                                strategy: "python_module_qualified_by_name",
                            });
                        }
                    }
                }
            }
        }

        // Step 4: Import-based resolution for simple names.
        // `from app.models import User` → `User` resolves to `app.models.User`.
        for import in &file_ctx.imports {
            if import.is_wildcard {
                if let Some(ref mod_path) = import.module_path {
                    if !is_relative_import(mod_path) {
                        continue;
                    }
                    let candidate = format!("{mod_path}.{effective_target}");
                    if let Some(sym) = lookup.by_qualified_name(&candidate) {
                        if kind_compatible(edge_kind, &sym.kind) {
                            return Some(Resolution {
                                target_symbol_id: sym.id,
                                confidence: 0.90,
                                strategy: "python_wildcard_import",
                            });
                        }
                    }
                }
                continue;
            }

            if import.imported_name != effective_target {
                continue;
            }
            let Some(ref mod_path) = import.module_path else {
                continue;
            };

            // `from foo.bar import Baz` → try `foo.bar.Baz`
            let candidate = format!("{mod_path}.{effective_target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if kind_compatible(edge_kind, &sym.kind) {
                    debug!(
                        strategy = "python_from_import",
                        candidate = %candidate,
                        "resolved"
                    );
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "python_from_import",
                    });
                }
            }

            // Also search by simple name scoped to the module.
            for sym in lookup.by_name(effective_target) {
                if sym.qualified_name.starts_with(mod_path.as_str())
                    && kind_compatible(edge_kind, &sym.kind)
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 0.95,
                        strategy: "python_from_import_prefix",
                    });
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

        // Import refs: classify the module as external or stdlib.
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            let module = ref_ctx.extracted_ref.module.as_deref().unwrap_or(target);
            // Relative imports are internal.
            if is_relative_import(module) {
                return None;
            }
            let root = module.split('.').next().unwrap_or(module);
            if is_python_stdlib(root) {
                return Some("stdlib".to_string());
            }
            let is_ext = match project_ctx {
                Some(ctx) => ctx.is_external_python_package(root),
                None => true,
            };
            if is_ext {
                return Some(module.to_string());
            }
            return None;
        }

        // Python builtins (built-in functions, types, and common method names).
        if is_python_builtin(target) {
            return Some("python_builtins".to_string());
        }

        // Walk file imports for a match.
        let simple = target.split('.').next().unwrap_or(target);
        for import in &file_ctx.imports {
            if import.imported_name != simple {
                continue;
            }
            let Some(ref mod_path) = import.module_path else {
                continue;
            };
            if is_relative_import(mod_path) {
                continue;
            }
            let root = mod_path.split('.').next().unwrap_or(mod_path);
            if is_python_stdlib(root) {
                return Some("stdlib".to_string());
            }
            let is_ext = match project_ctx {
                Some(ctx) => ctx.is_external_python_package(root),
                None => true,
            };
            if is_ext {
                return Some(mod_path.clone());
            }
        }

        None
    }

    // is_visible: default (always true). Python has no enforced access control
    // at runtime — `_private` is convention only and we don't track it.
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// A relative import starts with a dot (`./`, `../`) or is an internal module
/// path (no domain-style host segment, not a stdlib name).
///
/// We approximate: if the module path starts with `.` or contains a `/` it's
/// relative/local. Otherwise it might be an installed package.
fn is_relative_import(module: &str) -> bool {
    module.starts_with('.') || module.starts_with('/')
}

/// Python standard library top-level module names.
fn is_python_stdlib(name: &str) -> bool {
    matches!(
        name,
        "os"
            | "sys"
            | "json"
            | "re"
            | "datetime"
            | "pathlib"
            | "typing"
            | "collections"
            | "functools"
            | "itertools"
            | "logging"
            | "unittest"
            | "dataclasses"
            | "abc"
            | "io"
            | "math"
            | "hashlib"
            | "uuid"
            | "copy"
            | "enum"
            | "http"
            | "urllib"
            | "socket"
            | "threading"
            | "multiprocessing"
            | "subprocess"
            | "argparse"
            | "configparser"
            | "csv"
            | "sqlite3"
            // Additional commonly imported stdlib modules
            | "time"
            | "random"
            | "string"
            | "struct"
            | "base64"
            | "pickle"
            | "shutil"
            | "tempfile"
            | "glob"
            | "fnmatch"
            | "traceback"
            | "warnings"
            | "weakref"
            | "contextlib"
            | "inspect"
            | "ast"
            | "dis"
            | "types"
            | "operator"
            | "decimal"
            | "fractions"
            | "statistics"
            | "textwrap"
            | "pprint"
            | "heapq"
            | "bisect"
            | "array"
            | "queue"
            | "asyncio"
            | "concurrent"
            | "signal"
            | "mimetypes"
            | "email"
            | "html"
            | "xml"
            | "zipfile"
            | "tarfile"
            | "gzip"
            | "bz2"
            | "lzma"
            | "zlib"
            | "platform"
    )
}

/// Python built-in functions, types, and common method names.
///
/// Covers the built-in namespace (functions/types always in scope), common
/// str/list/dict instance methods, unittest assert helpers, and a few Django
/// model convenience methods. Used both in `resolve` (fast exit) and in
/// `infer_external_namespace` (classify as `python_builtins`).
fn is_python_builtin(name: &str) -> bool {
    // Strip `self.` prefix if present.
    let name = name.strip_prefix("self.").unwrap_or(name);
    matches!(
        name,
        // Built-in functions always in scope
        "len"
            | "print"
            | "str"
            | "int"
            | "float"
            | "bool"
            | "list"
            | "dict"
            | "set"
            | "tuple"
            | "range"
            | "enumerate"
            | "zip"
            | "map"
            | "filter"
            | "sorted"
            | "reversed"
            | "isinstance"
            | "issubclass"
            | "hasattr"
            | "getattr"
            | "setattr"
            | "delattr"
            | "super"
            | "type"
            | "property"
            | "staticmethod"
            | "classmethod"
            | "object"
            | "open"
            | "input"
            | "repr"
            | "hash"
            | "id"
            | "abs"
            | "max"
            | "min"
            | "sum"
            | "any"
            | "all"
            | "next"
            | "iter"
            | "callable"
            | "vars"
            | "dir"
            | "globals"
            | "locals"
            | "exec"
            | "eval"
            | "compile"
            | "breakpoint"
            | "round"
            | "pow"
            | "divmod"
            | "chr"
            | "ord"
            | "hex"
            | "oct"
            | "bin"
            | "bytes"
            | "bytearray"
            | "memoryview"
            | "frozenset"
            | "complex"
            | "format"
            | "NotImplemented"
            | "Ellipsis"
            | "None"
            | "True"
            | "False"
            // Common exception types
            | "Exception"
            | "ValueError"
            | "TypeError"
            | "KeyError"
            | "IndexError"
            | "AttributeError"
            | "RuntimeError"
            | "StopIteration"
            | "OSError"
            | "IOError"
            | "ImportError"
            | "NameError"
            | "NotImplementedError"
            | "AssertionError"
            | "OverflowError"
            | "ZeroDivisionError"
            | "FileNotFoundError"
            | "PermissionError"
            // str instance methods
            | "strip"
            | "lstrip"
            | "rstrip"
            | "split"
            | "rsplit"
            | "join"
            | "replace"
            | "lower"
            | "upper"
            | "title"
            | "capitalize"
            | "startswith"
            | "endswith"
            | "find"
            | "rfind"
            | "index"
            | "count"
            | "encode"
            | "decode"
            | "isdigit"
            | "isalpha"
            | "isnumeric"
            // list / dict instance methods
            | "append"
            | "extend"
            | "insert"
            | "remove"
            | "pop"
            | "clear"
            | "sort"
            | "reverse"
            | "copy"
            | "get"
            | "keys"
            | "values"
            | "items"
            | "update"
            | "setdefault"
            // unittest assert helpers
            | "assertEqual"
            | "assertIn"
            | "assertTrue"
            | "assertFalse"
            | "assertIsNone"
            | "assertIsNotNone"
            | "assertRaises"
            | "assertAlmostEqual"
            | "assertGreater"
            | "assertLess"
            | "assert_called_once"
            | "assert_called_with"
            | "assert_not_called"
            // Django model convenience methods
            | "refresh_from_db"
            | "save"
            | "delete"
            | "get_absolute_url"
            | "full_clean"
            | "clean"
    )
}

/// Check that the edge kind is compatible with the symbol kind.
fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::Calls => matches!(
            sym_kind,
            "method" | "function" | "constructor" | "test" | "class"
        ),
        EdgeKind::Inherits => matches!(sym_kind, "class"),
        EdgeKind::Implements => matches!(sym_kind, "class" | "interface"),
        EdgeKind::TypeRef => matches!(
            sym_kind,
            "class" | "interface" | "enum" | "type_alias" | "function" | "variable"
        ),
        EdgeKind::Instantiates => matches!(sym_kind, "class" | "function"),
        _ => true,
    }
}

// ---------------------------------------------------------------------------
// Chain-aware resolution
// ---------------------------------------------------------------------------

/// Walk a MemberChain step-by-step following field/return types.
///
/// For `self.db.query()` with chain `[self, db, query]`:
/// 1. `self` → find the enclosing class from scope_chain
/// 2. `db`   → look up "ClassName.db" field → field_type_name = "DatabaseSession"
/// 3. `query` → look up "DatabaseSession.query" → resolved
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
        SegmentKind::SelfRef => find_enclosing_class(&ref_ctx.scope_chain, lookup),
        SegmentKind::Identifier => {
            let name = &segments[0].name;

            let is_type = lookup.by_name(name).iter().any(|s| {
                matches!(
                    s.kind.as_str(),
                    "class" | "struct" | "interface" | "enum" | "type_alias"
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
            debug!(
                strategy = "python_chain_resolution",
                chain_len = segments.len(),
                resolved_type = %current_type,
                target = %last.name,
                "resolved"
            );
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 1.0,
                strategy: "python_chain_resolution",
            });
        }
    }

    for sym in lookup.by_name(&last.name) {
        if sym.qualified_name.starts_with(&current_type) && kind_compatible(edge_kind, &sym.kind) {
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 0.95,
                strategy: "python_chain_resolution",
            });
        }
    }

    None
}

/// Find the enclosing class name from the scope chain.
/// scope_chain = ["MyService.process", "MyService"] → "MyService"
fn find_enclosing_class(scope_chain: &[String], lookup: &dyn SymbolLookup) -> Option<String> {
    for scope in scope_chain {
        if let Some(sym) = lookup.by_qualified_name(scope) {
            if matches!(sym.kind.as_str(), "class" | "struct" | "interface") {
                return Some(scope.clone());
            }
        }
    }
    scope_chain.last().cloned()
}
