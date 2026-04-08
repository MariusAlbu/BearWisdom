// =============================================================================
// languages/hare/resolve.rs — Hare resolution rules
//
// Hare uses a module system based on `use` declarations:
//
//   use fmt;              → imports the "fmt" module
//   use os::exec;         → imports "exec" from "os"
//   use strings = strings; → alias
//
// At call sites, qualified names look like `fmt::println(...)`.
// The extractor emits the full qualified name or the bare function name.
//
// Resolution strategy:
//   1. `use` imports → build import table mapping module name to path.
//   2. Same-file: all top-level declarations in the same file.
//   3. Import-based qualified lookup: `{module}::{target}`.
//   4. Global name fallback.
//
// External namespace: `"hare_stdlib"` for standard library modules
//   (fmt, os, rt, strings, io, bufio, etc.)
// =============================================================================

use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, LanguageResolver, RefContext, Resolution, SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

pub struct HareResolver;

impl LanguageResolver for HareResolver {
    fn language_ids(&self) -> &[&str] {
        &["hare"]
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
            let module_path = r.module.clone().unwrap_or_else(|| r.target_name.clone());
            // The local name is the last segment of the module path.
            let local_name = module_path
                .rsplit("::")
                .next()
                .unwrap_or(module_path.as_str())
                .to_string();
            imports.push(ImportEntry {
                imported_name: local_name,
                module_path: Some(module_path),
                alias: None,
                is_wildcard: false,
            });
        }

        FileContext {
            file_path: file.path.clone(),
            language: "hare".to_string(),
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

        // Import declarations themselves don't resolve to a symbol.
        if edge_kind == EdgeKind::Imports {
            return None;
        }

        // Skip Hare primitive types.
        if is_hare_primitive(target) {
            return None;
        }

        // Step 1: Same-file resolution.
        for sym in lookup.in_file(&file_ctx.file_path) {
            if sym.name == *target {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "hare_same_file",
                });
            }
        }

        // Step 2: Import-based qualified lookup.
        // Hare qualified names use `::` as separator.
        // Try: for each imported module, look up `{module}::{target}`.
        for import in &file_ctx.imports {
            let Some(mod_path) = &import.module_path else {
                continue;
            };
            let candidate = format!("{mod_path}::{target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "hare_import_qualified",
                });
            }

            // Also try just the local module name prefix.
            let candidate2 = format!("{}::{}", import.imported_name, target);
            if let Some(sym) = lookup.by_qualified_name(&candidate2) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "hare_import_local",
                });
            }
        }

        // Step 3: Global name fallback.
        if let Some(sym) = lookup.by_name(target).into_iter().next() {
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 0.8,
                strategy: "hare_global_name",
            });
        }

        None
    }

    fn infer_external_namespace(
        &self,
        _file_ctx: &FileContext,
        ref_ctx: &RefContext,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;

        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            let module = ref_ctx
                .extracted_ref
                .module
                .as_deref()
                .unwrap_or(target.as_str());
            if is_hare_stdlib_module(module) {
                return Some("hare_stdlib".to_string());
            }
        }

        if is_hare_primitive(target) {
            return Some("builtin".to_string());
        }

        None
    }
}

/// Hare standard library module names.
fn is_hare_stdlib_module(module: &str) -> bool {
    // Root module is the first segment before "::"
    let root = module.split("::").next().unwrap_or(module);
    matches!(
        root,
        "bufio" | "bytes" | "cmd" | "crypto" | "debug" | "dirs" | "encoding"
            | "errors" | "fmt" | "fs" | "getopt" | "hash" | "hare"
            | "io" | "log" | "math" | "mime" | "net" | "os" | "path"
            | "rt" | "shlex" | "slices" | "sort" | "strconv" | "strings"
            | "temp" | "time" | "types" | "unix" | "uuid"
    )
}

/// Hare primitive types.
fn is_hare_primitive(name: &str) -> bool {
    matches!(
        name,
        "bool" | "void" | "never" | "null" | "opaque"
            | "int" | "i8" | "i16" | "i32" | "i64"
            | "uint" | "u8" | "u16" | "u32" | "u64"
            | "uintptr" | "size" | "f32" | "f64"
            | "rune" | "str" | "bytes"
    )
}
