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
    self as engine, FileContext, ImportEntry, RefContext, Resolution,
    SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

pub struct HareResolver;

impl HareResolver {

    
    pub(crate) fn build_file_context(
        &self,
        file: &ParsedFile,
        project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
        build_file_context_inner(file, project_ctx)
    }
pub(crate) fn resolve(
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

        // Language-specific: import-based qualified lookup with `::` separator.
        for import in &file_ctx.imports {
            let Some(mod_path) = &import.module_path else {
                continue;
            };
            // Try full module path prefix: `{mod_path}::{target}`.
            let candidate = format!("{mod_path}::{target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "hare_import_qualified",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }

            // Try local module name prefix: `{local_name}::{target}`.
            let candidate2 = format!("{}::{}", import.imported_name, target);
            if let Some(sym) = lookup.by_qualified_name(&candidate2) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "hare_import_local",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
        }

        engine::resolve_common("hare", file_ctx, ref_ctx, lookup, |_, _| true)
    }

}

#[cfg(test)]
#[path = "resolve_tests.rs"]
mod tests;

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
pub(super) fn is_hare_primitive(name: &str) -> bool {
    matches!(
        name,
        "bool" | "void" | "never" | "null" | "opaque"
            | "int" | "i8" | "i16" | "i32" | "i64"
            | "uint" | "u8" | "u16" | "u32" | "u64"
            | "uintptr" | "size" | "f32" | "f64"
            | "rune" | "str" | "bytes"
    )
}

pub(crate) fn detect_flow_inner(
    _file_ctx: &FileContext,
    ref_ctx: &RefContext,
) -> Vec<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{
        ChannelRole, FlowEmission, HttpMethod, NamedChannelKind,
    };
    use crate::types::CallArg;
    let r = &ref_ctx.extracted_ref;
    if r.kind != EdgeKind::Calls {
        return Vec::new();
    }
    // Hare stdlib `net::http::client::get(url, ...)`.
    let module = r.module.as_deref().unwrap_or("");
    if !module.contains("http") {
        return Vec::new();
    }
    let target = r.target_name.as_str();
    if !matches!(target, "get" | "post" | "request" | "send") {
        return Vec::new();
    }
    let url = r.call_args.iter().find_map(|a| match a {
        CallArg::StringLit(s)
            if s.starts_with('/') || s.starts_with("http://") || s.starts_with("https://") =>
        {
            Some(s.as_str())
        }
        _ => None,
    });
    let Some(url) = url else { return Vec::new() };
    vec![FlowEmission::NamedChannel {
        kind: NamedChannelKind::HttpCall,
        name: crate::connectors::url_pattern::normalize(url),
        role: ChannelRole::Producer,
        method: Some(HttpMethod::Any),
    streaming: None,
    }]
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
