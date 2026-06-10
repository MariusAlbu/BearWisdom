// =============================================================================
// ecosystem/bicep_runtime.rs — on-disk discovery of Bicep language grammar
//
// Bicep's built-in functions, decorators, and ARM resource API methods
// (`concat`, `format`, `description`, `getSecret`, `listKeys`, `resourceId`,
// ...) are defined inside the Azure/bicep compiler source — specifically
// `Bicep.Core/Semantics/Namespaces/{System,Az}NamespaceType.cs` and the
// `LanguageConstants.cs` file they reference. There's no machine-readable
// runtime surface (Bicep ships as a single self-contained .NET binary
// with embedded DLLs that aren't trivially walkable).
//
// **Discovery strategy** (project-driven, real on-disk only): search the
// project tree itself for a vendored Azure/bicep checkout — a directory
// whose `src/Bicep.Core/Bicep.Core.csproj` +
// `Semantics/Namespaces/SystemNamespaceType.cs` marker pair identifies it
// (`looks_like_bicep_clone`). The walk is depth-bounded and prunes dep
// caches / VCS dirs. No env-var override and no machine-path probing: a
// project resolves these names only when it carries the compiler source
// in its own tree.
//
// When discovery succeeds we synthesise a single `ParsedFile` whose
// symbols are the Bicep runtime grammar names, extracted by recognising
// the upstream `FunctionOverloadBuilder("name")` / `DecoratorBuilder(...)`
// registration calls in those `.cs` files. The symbols resolve through
// the ambient-package rung in the standard symbol index.
//
// **When discovery fails** (no clone anywhere in the tree): Bicep refs to
// builtin names stay unresolved. That's the honest state — we can't know
// the surface without the source on disk.
//
// Activation: `.bicep` files present in the project.
// =============================================================================

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::{Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::types::{ExtractedSymbol, ParsedFile, SymbolKind, Visibility};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("bicep-runtime");
const ECOSYSTEM_TAG: &str = "bicep-runtime";
const LANGUAGES: &[&str] = &["bicep"];

pub struct BicepRuntimeEcosystem;

impl Ecosystem for BicepRuntimeEcosystem {
    fn id(&self) -> EcosystemId {
        ID
    }
    fn kind(&self) -> EcosystemKind {
        EcosystemKind::Stdlib
    }
    fn languages(&self) -> &'static [&'static str] {
        LANGUAGES
    }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::LanguagePresent("bicep")
    }

    fn locate_roots(&self, ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_bicep_source(ctx.project_root)
    }

    fn walk_root(&self, _dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        // No source-walk path: parse_metadata_only synthesises the file
        // directly from the Bicep upstream .cs sources we located. C#
        // extraction over the same files would produce the wrong shape
        // — `FunctionOverloadBuilder("name")` is a registration call,
        // not a method definition the C# extractor would surface as a
        // Bicep-callable name.
        Vec::new()
    }

    fn uses_demand_driven_parse(&self) -> bool {
        true
    }

    fn parse_metadata_only(&self, dep: &ExternalDepRoot) -> Option<Vec<ParsedFile>> {
        Some(synthesise_bicep_namespace_file(&dep.root))
    }
}

impl ExternalSourceLocator for BicepRuntimeEcosystem {
    fn ecosystem(&self) -> &'static str {
        ECOSYSTEM_TAG
    }

    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_bicep_source(project_root)
    }

    fn walk_root(&self, _dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        Vec::new()
    }

    fn parse_metadata_only(&self, project_root: &Path) -> Option<Vec<ParsedFile>> {
        let roots = discover_bicep_source(project_root);
        let root = roots.first()?;
        Some(synthesise_bicep_namespace_file(&root.root))
    }
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<BicepRuntimeEcosystem>> = OnceLock::new();
    LOCATOR
        .get_or_init(|| Arc::new(BicepRuntimeEcosystem))
        .clone()
}

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

fn discover_bicep_source(project_root: &Path) -> Vec<ExternalDepRoot> {
    let Some(clone) = find_bicep_clone_in_tree(project_root) else {
        return Vec::new();
    };
    // The two namespace files plus LanguageConstants live under the same
    // Bicep.Core C# project. `looks_like_bicep_clone` already proved the
    // marker pair, so src/Bicep.Core exists.
    let bicep_core = clone.join("src").join("Bicep.Core");
    tracing::info!(
        "bicep-runtime: using Bicep source at {}",
        bicep_core.display()
    );
    vec![ExternalDepRoot {
        module_path: "bicep-core".to_string(),
        version: String::from("local"),
        root: bicep_core,
        ecosystem: ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    }]
}

/// Depth cap for the project-tree clone search. A vendored compiler
/// checkout (`infra/vendor/bicep`, `third_party/bicep`, …) sits within a
/// few levels of the root; beyond that the search is wasted work.
const MAX_CLONE_SEARCH_DEPTH: u32 = 6;

/// Search the project tree for a directory that looks like an Azure/bicep
/// checkout. Returns the first match (depth-first), or None. Prunes dep
/// caches, build outputs, and dot/VCS dirs so the walk stays cheap; the
/// marker check is a two-file stat per directory.
fn find_bicep_clone_in_tree(project_root: &Path) -> Option<PathBuf> {
    if looks_like_bicep_clone(project_root) {
        return Some(project_root.to_path_buf());
    }
    walk_for_clone(project_root, 0)
}

fn walk_for_clone(dir: &Path, depth: u32) -> Option<PathBuf> {
    if depth >= MAX_CLONE_SEARCH_DEPTH {
        return None;
    }
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        if !ft.is_dir() {
            continue;
        }
        let path = entry.path();
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if is_pruned_search_dir(name) {
                continue;
            }
        }
        if looks_like_bicep_clone(&path) {
            return Some(path);
        }
        if let Some(found) = walk_for_clone(&path, depth + 1) {
            return Some(found);
        }
    }
    None
}

/// Dependency caches, build outputs, and VCS/hidden dirs that can't hold a
/// vendored compiler checkout — skipped to keep the search bounded.
fn is_pruned_search_dir(name: &str) -> bool {
    matches!(
        name,
        "node_modules" | "target" | "bin" | "obj" | "dist" | "build" | "out" | ".bearwisdom"
    ) || name.starts_with('.')
}

fn looks_like_bicep_clone(p: &Path) -> bool {
    p.is_dir()
        && p.join("src/Bicep.Core/Bicep.Core.csproj").is_file()
        && p.join("src/Bicep.Core/Semantics/Namespaces/SystemNamespaceType.cs")
            .is_file()
}

// ---------------------------------------------------------------------------
// Synthesis from on-disk Bicep source
// ---------------------------------------------------------------------------

/// Read the namespace .cs files from a located Bicep clone and emit a
/// single synthetic `ParsedFile` whose symbols are Bicep's runtime grammar
/// names. The names are extracted by recognising the upstream
/// registration patterns:
///
///   * `FunctionOverloadBuilder("name")` / `BannedFunctionBuilder("name")`
///     / `BannedFunction("name")` — literal-string registration.
///   * `FunctionOverloadBuilder(LanguageConstants.AnyFunction)` — constant
///     reference; resolve via a pass-1 scan of `public const string X = "..."`
///     declarations across all three files.
///   * `BannedFunction.CreateForOperator("name", "+")` — operator pseudo-fns
///     (`add`, `equals`, etc.) that ARM accepts but Bicep rejects with a
///     diagnostic.
///   * `DecoratorBuilder(LanguageConstants.X)` — parameter / output
///     decorators like `description`, `minLength`, `metadata`,
///     `discriminator`.
///
/// **No regex-into-vendored-JSON path.** Names come from the user's actual
/// Bicep clone at index time. If the clone is stale, the names match
/// whatever that revision defined.
fn synthesise_bicep_namespace_file(bicep_core: &Path) -> Vec<ParsedFile> {
    let semantics_ns = bicep_core.join("Semantics").join("Namespaces");
    let sys_path = semantics_ns.join("SystemNamespaceType.cs");
    let az_path = semantics_ns.join("AzNamespaceType.cs");
    let lang_constants = bicep_core.join("LanguageConstants.cs");

    let read = |p: &Path| std::fs::read_to_string(p).ok();
    let sys_src = read(&sys_path);
    let az_src = read(&az_path);
    let lang_const_src = read(&lang_constants);

    let mut consts: HashMap<String, String> = HashMap::new();
    for src in [
        sys_src.as_deref(),
        az_src.as_deref(),
        lang_const_src.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        collect_string_consts(src, &mut consts);
    }

    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut emitted: std::collections::HashSet<(String, &'static str)> =
        std::collections::HashSet::new();
    let mut emit =
        |name: String,
         module: &'static str,
         kind: SymbolKind,
         symbols: &mut Vec<ExtractedSymbol>,
         emitted: &mut std::collections::HashSet<(String, &'static str)>| {
            if name.is_empty() {
                return;
            }
            if !emitted.insert((name.clone(), module)) {
                return;
            }
            symbols.push(ExtractedSymbol {
                name: name.clone(),
                qualified_name: format!("{module}.{name}"),
                kind,
                visibility: Some(Visibility::Public),
                start_line: 0,
                end_line: 0,
                start_col: 0,
                end_col: 0,
                signature: Some(format!("from {} (Bicep upstream)", module)),
                doc_comment: None,
                scope_path: Some(module.to_string()),
                parent_index: None,
                byte_offset: 0,
                declared_type: None,
                return_type: None,
                param_types: Vec::new(),
                generic_params: Vec::new(),
            });
        };

    for src in [sys_src.as_deref(), az_src.as_deref()]
        .into_iter()
        .flatten()
    {
        for name in extract_function_names(src, &consts) {
            emit(
                name,
                "bicep.builtins",
                SymbolKind::Function,
                &mut symbols,
                &mut emitted,
            );
        }
        for name in extract_decorator_names(src, &consts) {
            emit(
                name,
                "bicep.decorators",
                SymbolKind::Function,
                &mut symbols,
                &mut emitted,
            );
        }
    }
    // Namespace aliases — derived from the file names that are present
    // (`SystemNamespaceType.cs` → `sys`, `AzNamespaceType.cs` → `az`).
    // Convention from upstream Bicep: the file's class name has a `Type`
    // suffix and the namespace alias is the lowercase prefix before it.
    if sys_src.is_some() {
        emit(
            "sys".to_string(),
            "bicep.namespace",
            SymbolKind::Class,
            &mut symbols,
            &mut emitted,
        );
    }
    if az_src.is_some() {
        emit(
            "az".to_string(),
            "bicep.namespace",
            SymbolKind::Class,
            &mut symbols,
            &mut emitted,
        );
    }

    if symbols.is_empty() {
        tracing::warn!(
            "bicep-runtime: located clone at {} but extracted 0 names — \
             upstream layout may have changed",
            bicep_core.display()
        );
        return Vec::new();
    }
    tracing::info!(
        "bicep-runtime: extracted {} runtime grammar names from {}",
        symbols.len(),
        bicep_core.display()
    );

    let n = symbols.len();
    vec![ParsedFile {
        path: "ext:bicep-runtime:namespace.bicep".to_string(),
        language: "bicep".to_string(),
        content_hash: format!("bicep-runtime-{n}"),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        symbols,
        refs: Vec::new(),
        routes: Vec::new(),
        db_sets: Vec::new(),
        symbol_origin_languages: vec![None; n],
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: vec![false; n],
        content: None,
        has_errors: false,
        flow: crate::types::FlowMeta::default(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),

        plugin_flow_emissions: Vec::new(),
    }]
}

fn collect_string_consts(source: &str, out: &mut HashMap<String, String>) {
    // `public const string X = "literal"` → X → literal.
    // Skip values starting with `__` (Bicep internal markers).
    let bytes = source.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Match marker.
        let after = match find_str_after(bytes, i, b"public const string ") {
            Some(p) => p,
            None => break,
        };
        i = after;
        let (name, after_name) = match read_ident(bytes, after) {
            Some(v) => v,
            None => continue,
        };
        let after_eq = match skip_whitespace_eq(bytes, after_name) {
            Some(p) => p,
            None => continue,
        };
        if after_eq >= bytes.len() || bytes[after_eq] != b'"' {
            continue;
        }
        if let Some((value, after_value)) = read_string_literal(bytes, after_eq) {
            if !value.starts_with("__") {
                out.insert(name.to_string(), value.to_string());
            }
            i = after_value;
        }
    }
}

fn extract_function_names(source: &str, consts: &HashMap<String, String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let bytes = source.as_bytes();
    // Three constructor names. `find_str_after` returns the byte index
    // immediately after the matched name, so `pos` already sits at the
    // first char *after* the ctor.
    for ctor in [
        "FunctionOverloadBuilder",
        "BannedFunctionBuilder",
        "BannedFunction",
    ] {
        let mut search = 0;
        while let Some(pos) = find_str_after(bytes, search, ctor.as_bytes()) {
            // Special-case `BannedFunction.CreateForOperator("name", "+")` —
            // a static call, not a constructor.
            if ctor == "BannedFunction"
                && bytes.get(pos) == Some(&b'.')
                && bytes[pos + 1..].starts_with(b"CreateForOperator")
            {
                let after_method = pos + 1 + b"CreateForOperator".len();
                if let Some(open) = skip_to_paren(bytes, after_method) {
                    if let Some(name) = read_first_arg(bytes, open, consts) {
                        out.push(name);
                    }
                }
                search = pos + 1;
                continue;
            }
            // Standard `<Ctor>(...)` shape.
            let Some(after_paren) = skip_to_paren(bytes, pos) else {
                search = pos + 1;
                continue;
            };
            if let Some(name) = read_first_arg(bytes, after_paren, consts) {
                out.push(name);
            }
            search = pos + 1;
        }
    }
    out
}

fn extract_decorator_names(source: &str, consts: &HashMap<String, String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let bytes = source.as_bytes();
    let mut search = 0;
    while let Some(pos) = find_str_after(bytes, search, b"DecoratorBuilder") {
        // `pos` is already past the name — skip whitespace + `(` directly.
        let Some(after_paren) = skip_to_paren(bytes, pos) else {
            search = pos + 1;
            continue;
        };
        if let Some(name) = read_first_arg(bytes, after_paren, consts) {
            out.push(name);
        }
        search = pos + 1;
    }
    out
}

// --- byte-level helpers (avoid pulling in regex) ---

fn find_str_after(haystack: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    if from > haystack.len() || needle.is_empty() {
        return None;
    }
    haystack[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| from + p + needle.len())
}

fn read_ident(bytes: &[u8], from: usize) -> Option<(&str, usize)> {
    let mut start = from;
    while start < bytes.len() && (bytes[start] == b' ' || bytes[start] == b'\t') {
        start += 1;
    }
    let begin = start;
    while start < bytes.len() {
        let b = bytes[start];
        if b.is_ascii_alphanumeric() || b == b'_' {
            start += 1
        } else {
            break;
        }
    }
    if begin == start {
        return None;
    }
    std::str::from_utf8(&bytes[begin..start])
        .ok()
        .map(|s| (s, start))
}

fn skip_whitespace_eq(bytes: &[u8], from: usize) -> Option<usize> {
    let mut i = from;
    while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
        i += 1
    }
    if i < bytes.len() && bytes[i] == b'=' {
        i += 1;
        while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
            i += 1
        }
        Some(i)
    } else {
        None
    }
}

fn skip_to_paren(bytes: &[u8], from: usize) -> Option<usize> {
    let mut i = from;
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1
    }
    if i < bytes.len() && bytes[i] == b'(' {
        Some(i + 1)
    } else {
        None
    }
}

fn read_string_literal(bytes: &[u8], from: usize) -> Option<(&str, usize)> {
    if from >= bytes.len() || bytes[from] != b'"' {
        return None;
    }
    let begin = from + 1;
    let mut i = begin;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            i += 2;
            continue;
        }
        if bytes[i] == b'"' {
            break;
        }
        i += 1;
    }
    if i >= bytes.len() {
        return None;
    }
    std::str::from_utf8(&bytes[begin..i])
        .ok()
        .map(|s| (s, i + 1))
}

/// Read the first argument of a builder call. Either a string literal or
/// an identifier (which we resolve via `consts`). The identifier may be
/// `LanguageConstants.<X>` or just `<X>` — accept both.
fn read_first_arg(bytes: &[u8], from: usize, consts: &HashMap<String, String>) -> Option<String> {
    let mut i = from;
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1
    }
    if i >= bytes.len() {
        return None;
    }
    if bytes[i] == b'"' {
        return read_string_literal(bytes, i).map(|(s, _)| s.to_string());
    }
    // Identifier, possibly qualified by `LanguageConstants.`.
    let (mut name, end) = read_ident(bytes, i)?;
    if name == "LanguageConstants" && bytes.get(end) == Some(&b'.') {
        let (member, _) = read_ident(bytes, end + 1)?;
        name = member;
    }
    consts.get(name).cloned()
}

#[cfg(test)]
#[path = "bicep_runtime_tests.rs"]
mod tests;
