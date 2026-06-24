// =============================================================================
// indexer/parse_file.rs  —  single-file parsing
//
// Reads a file, hashes its bytes, decodes to UTF-8, classifies it (generated
// header / vendored library), and dispatches to the language plugin to
// produce a `ParsedFile`. Embedded-region splicing and locals.scm filtering
// live in sibling modules (`embedded_regions`, `local_refs`) and are invoked
// from inside `parse_file_with_demand`.
// =============================================================================

use crate::languages::LanguageRegistry;
use crate::types::{ExtractedSymbol, ParsedFile, SymbolKind, Visibility};
use crate::walker::WalkedFile;
use anyhow::{Context, Result};
use once_cell::sync::Lazy;
use sha2::{Digest, Sha256};
use tracing::{debug, warn};

/// Per-thread stack for parse pools. Sized large because the extractors walk
/// the tree-sitter CST recursively, and generated sources nest CST nodes far
/// deeper than hand-written code — a `type X = 'a' | 'b' | …` union with tens
/// of thousands of members parses as that many nested `union_type` nodes, deep
/// enough to overflow an ordinary thread stack. The reservation is virtual;
/// only the pages a deep walk actually touches commit.
pub(crate) const PARSE_STACK_SIZE: usize = 128 * 1024 * 1024;

/// Build the rayon pool used for parsing files — both the main streaming pass
/// and the external chain-expansion pass parse on a pool from here so they
/// share one stack budget.
///
/// Thread count is capped at `min(logical_cores, 8)`: the default global pool
/// spawns one worker per logical core, and each active worker concurrently
/// holds a tree-sitter Tree + String content + in-flight `ParsedFile`, which
/// on a large project stacks into GB of transient RAM. The cap keeps ~95% of
/// parse throughput (parsing scales only modestly past 8 threads given
/// shared-grammar contention) and cuts peak memory roughly 3x. Override with
/// `BEARWISDOM_PARSE_THREADS` when a dedicated CI runner wants every core.
pub(crate) fn build_parse_pool() -> Result<rayon::ThreadPool> {
    let parse_threads = std::env::var("BEARWISDOM_PARSE_THREADS")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|&n| n > 0)
        .unwrap_or_else(|| {
            let cores = std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4);
            cores.min(8)
        });
    rayon::ThreadPoolBuilder::new()
        .num_threads(parse_threads)
        .thread_name(|i| format!("bw-parse-{i}"))
        .stack_size(PARSE_STACK_SIZE)
        .build()
        .context("Failed to build parse thread pool")
}

/// Pool the resolve pass runs on. Full width (one worker per core, unlike the
/// parse pool's memory-capped 8) but the deep [`PARSE_STACK_SIZE`] stack: the
/// pass lazily materializes external files by parsing them on its own workers,
/// and a generated or bundled external (`.d.ts`, minified JS) nests the CST far
/// past what the binary's ≈8 MB global-pool stack holds. Built once on first
/// use; `None` only if pool creation fails, in which case [`with_resolve_pool`]
/// runs the pass on the caller's stack.
static RESOLVE_POOL: Lazy<Option<rayon::ThreadPool>> = Lazy::new(|| {
    rayon::ThreadPoolBuilder::new()
        .thread_name(|i| format!("bw-resolve-{i}"))
        .stack_size(PARSE_STACK_SIZE)
        .build()
        .ok()
});

/// Run the resolve pass's parallel work on [`RESOLVE_POOL`] so the external
/// parses it performs inline get the deep parse stack. Installing the whole
/// pass (rather than each lazy parse) keeps every external parse inline on a
/// deep-stack worker — no cross-pool hand-off, which would block resolve
/// workers against the parse pool and deadlock the reduction.
///
/// Called from the resolve driver on the main thread; the caller blocks until
/// the pass completes. Best-effort: if the pool couldn't be built, the work
/// runs on the caller's stack — correct, only without the deep-stack guarantee.
pub(crate) fn with_resolve_pool<R: Send>(f: impl FnOnce() -> R + Send) -> R {
    match &*RESOLVE_POOL {
        Some(pool) => pool.install(f),
        None => f(),
    }
}

pub(crate) fn parse_file(walked: &WalkedFile, registry: &LanguageRegistry) -> Result<ParsedFile> {
    parse_file_with_demand(walked, registry, None)
}

/// Parse with access to a workspace `TypeArena`. Plugins that populate
/// `ExtractedSymbol`'s TypeId fields intern into this arena, so the same
/// canonical TypeIds flow into `SymbolIndex::build_with_context_and_arena`
/// later. Callers that don't share an arena across files should keep using
/// `parse_file` / `parse_file_with_demand` — those route through the
/// non-arena trait method and produce `None` TypeIds.
pub(crate) fn parse_file_with_arena(
    walked: &WalkedFile,
    registry: &LanguageRegistry,
    arena: &crate::type_checker::core::types::TypeArena,
) -> Result<ParsedFile> {
    parse_file_with_arena_and_demand(walked, registry, None, arena)
}

/// R6 entry point for demand-driven parsing. When `demand` is `Some`, the
/// language plugin's `extract_with_demand` is called instead of `extract`,
/// and top-level declarations whose name is not in the set may be dropped.
/// Used when parsing external sources (node_modules `.d.ts`, etc.).
pub(crate) fn parse_file_with_demand(
    walked: &WalkedFile,
    registry: &LanguageRegistry,
    demand: Option<&std::collections::HashSet<String>>,
) -> Result<ParsedFile> {
    parse_file_internal(walked, registry, demand, None)
}

/// Parse with both demand filtering and access to a workspace `TypeArena`.
/// Plugins that opt into TypeId population read this arena to intern type
/// expressions during extraction. Production indexer entries use this so
/// the same arena flows into `SymbolIndex::build_with_context_and_arena`.
pub(crate) fn parse_file_with_arena_and_demand(
    walked: &WalkedFile,
    registry: &LanguageRegistry,
    demand: Option<&std::collections::HashSet<String>>,
    arena: &crate::type_checker::core::types::TypeArena,
) -> Result<ParsedFile> {
    parse_file_internal(walked, registry, demand, Some(arena))
}

fn parse_file_internal(
    walked: &WalkedFile,
    registry: &LanguageRegistry,
    demand: Option<&std::collections::HashSet<String>>,
    arena: Option<&crate::type_checker::core::types::TypeArena>,
) -> Result<ParsedFile> {
    let bytes = std::fs::read(&walked.absolute_path)
        .with_context(|| format!("Cannot read {}", walked.relative_path))?;

    // SHA-256 of the raw bytes for change detection.
    let hash = {
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        format!("{:x}", hasher.finalize())
    };

    // Fast path: valid UTF-8 avoids an allocation. Lossy fallback handles
    // legacy Windows-1252 / Latin-1 source files (Delphi, older C/Fortran);
    // invalid byte sequences become U+FFFD, which does not appear in any
    // identifier, so parsing and resolution are unaffected.
    let content = match String::from_utf8(bytes) {
        Ok(s) => s,
        Err(e) => {
            debug!(
                "Non-UTF-8 bytes in {} — using lossy decode",
                walked.relative_path
            );
            String::from_utf8_lossy(e.as_bytes()).into_owned()
        }
    };

    let size = content.len() as u64;
    let line_count = content.lines().count() as u32;

    // Capture mtime for fast change detection on next incremental pass.
    let mtime = std::fs::metadata(&walked.absolute_path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64);

    // Short-circuit: some projects vendor auto-generated platform headers
    // (Microsoft WebView2/WinRT MIDL output, etc.) that balloon the
    // unresolved_refs table with thousands of macro/typedef identifiers the
    // parser can't distinguish from real refs (STDMETHODCALLTYPE, IUnknown,
    // LPCWSTR, BEGIN_INTERFACE, …). These files are valid C but semantically
    // uninteresting and have no cross-project consumers. Record the file row
    // for hash tracking but emit zero symbols/refs.
    let is_generated = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        is_generated_platform_header(walked.language, &content)
    })) {
        Ok(flag) => flag,
        Err(e) => {
            let msg = panic_message(&e);
            warn!(
                "is_generated_platform_header panicked on {}: {msg} — treating as non-generated",
                walked.relative_path,
            );
            false
        }
    };
    if is_generated {
        return Ok(ParsedFile {
            path: walked.relative_path.clone(),
            language: walked.language.to_string(),
            content_hash: hash,
            size,
            line_count,
            mtime,
            package_id: None,
            symbols: Vec::new(),
            refs: Vec::new(),
            routes: Vec::new(),
            db_sets: Vec::new(),
            symbol_origin_languages: Vec::new(),
            ref_origin_languages: Vec::new(),
            symbol_from_snippet: Vec::new(),
            content: Some(content),
            has_errors: false,
            flow: crate::types::FlowMeta::default(),
            demand_contributions: Vec::new(),
            alias_targets: Vec::new(),
            component_selectors: Vec::new(),
            plugin_flow_emissions: Vec::new(),
        });
    }

    // Dispatch to the language plugin (dedicated or generic fallback).
    // When demand is Some, the plugin's demand-aware path runs; with None it
    // degrades to the regular `extract` via the trait's default impl.
    // The absolute path is passed as file_path so plugins that need filesystem
    // access (e.g. Fortran's fypp preprocessor) can locate sibling files.
    // The arena-aware trait method is preferred when the caller supplies one
    // so plugins can populate ExtractedSymbol TypeIds against the shared
    // workspace arena.
    let plugin = registry.get(walked.language);
    let abs_path_str = walked.absolute_path.to_string_lossy();
    let mut r = match arena {
        Some(a) => plugin.extract_with_arena_and_demand(
            &content,
            &abs_path_str,
            walked.language,
            demand,
            a,
        ),
        None => plugin.extract_with_demand(&content, &abs_path_str, walked.language, demand),
    };

    // Derive scope_path / qualified_name from the structural parent_index chain,
    // correcting any symbol whose stored qname dropped its package prefix. Only
    // symbols whose qname is structurally inconsistent with their parent are
    // rewritten, so well-qualified symbols (any separator) stay byte-identical.
    crate::containment::normalize_qnames_from_parents(&mut r.symbols);

    // Single tree-sitter parse shared by locals.scm filtering and flow typing.
    // The extractor parses internally and doesn't expose its tree, so this is a
    // separate parse, made once and handed to both stages, for any language that
    // has both a locals.scm and a FlowConfig (TS/JS/Python/Java/C#/Go/Rust).
    // Parsed only when a consumer needs it: locals.scm filtering parses at any
    // size, but flow skips files over MAX_FLOW_SOURCE_BYTES without parsing — so
    // a flow-only language over that size produces no shared tree.
    let shared_grammar = plugin.grammar(walked.language);
    let shared_tree = {
        let want_for_locals =
            crate::indexer::query_builtins::locals_scm_for_language(walked.language).is_some();
        let want_for_flow = plugin.flow_config().is_some()
            && content.len() <= crate::indexer::flow::MAX_FLOW_SOURCE_BYTES;
        if want_for_locals || want_for_flow {
            shared_grammar.as_ref().and_then(|g| {
                let mut parser = tree_sitter::Parser::new();
                parser.set_language(g).ok()?;
                parser.parse(content.as_bytes(), None)
            })
        } else {
            None
        }
    };

    // Run locals.scm query to filter out locally-resolved references.
    // This removes local variables, parameters, and other intra-scope names
    // that don't need cross-file resolution.
    if let Some(tree) = shared_tree.as_ref() {
        super::local_refs::filter_local_refs_with_tree(
            &content,
            walked.language,
            plugin,
            &mut r.refs,
            tree,
        );
    } else {
        super::local_refs::filter_local_refs(
            &content,
            walked.language,
            plugin,
            &r.symbols,
            &mut r.refs,
        );
    }
    super::local_refs::filter_operator_refs(&mut r.refs);

    // Synthesize symbols a code generator / annotation processor would emit but
    // that never appear in parsed text (Lombok getters/setters, derive impls).
    // Runs before embedded-region splicing so the origin-language parallel
    // vectors (still empty here) backfill the synthesized symbols as
    // host-language entries via the existing resize below. The synthesized refs
    // carry source_symbol_index RELATIVE to the synthesized symbols, so rebase
    // them onto the file's table by the pre-append symbol count.
    let synthesized = plugin.synthesize_symbols(&content, &r.symbols, &r.refs);
    if !synthesized.symbols.is_empty() {
        let base = r.symbols.len();
        r.symbols.extend(synthesized.symbols);
        for mut sref in synthesized.refs {
            sref.source_symbol_index += base;
            r.refs.push(sref);
        }
    }

    // Symbols produced by the host extractor all share the file's language,
    // so the origin vector starts empty and grows only when we splice in
    // sub-extracted regions below.
    let mut symbol_origin_languages: Vec<Option<String>> = Vec::new();
    // E3: parallel snippet flag — true for symbols spliced in from a
    // MarkdownFence region (fenced code in Markdown, Rust doctests, Python
    // docstring `>>>` lines). Used downstream to exclude these symbols'
    // unresolved references from aggregate resolution stats.
    let mut symbol_from_snippet: Vec<bool> = Vec::new();
    let mut ref_origin_languages: Vec<Option<String>> = Vec::new();

    // Dispatch embedded regions (Vue/Svelte/Astro/Razor/HTML/PHP/MDX) —
    // each region is sub-parsed by the declared language's plugin and the
    // results are spliced back with line/column offsets.
    let regions = plugin.embedded_regions(&content, &walked.relative_path, walked.language);
    if !regions.is_empty() {
        // Pad origin vecs so host symbols/refs are all None before embedded Some(..).
        symbol_origin_languages.resize(r.symbols.len(), None);
        symbol_from_snippet.resize(r.symbols.len(), false);
        ref_origin_languages.resize(r.refs.len(), None);
        super::embedded_regions::dispatch_embedded_regions(
            &walked.relative_path,
            &content,
            registry,
            regions,
            &mut r,
            &mut symbol_origin_languages,
            &mut symbol_from_snippet,
            &mut ref_origin_languages,
        );
    }

    // R5 Sprint 2: run flow-typing queries if the plugin provides a FlowConfig.
    // Populates FlowMeta (forward-inference binding map, conditional narrowings,
    // call-site type_args on chain segments). Plugins without flow_config pay
    // zero cost here.
    let mut flow_meta = match plugin.flow_config() {
        Some(flow_cfg) => {
            if let Some(tree) = shared_tree.as_ref() {
                crate::indexer::flow::run_flow_queries_with_tree(
                    &content,
                    flow_cfg,
                    &r.symbols,
                    &mut r.refs,
                    tree,
                )
            } else if let Some(grammar) = shared_grammar.as_ref() {
                crate::indexer::flow::run_flow_queries(
                    &content,
                    grammar,
                    flow_cfg,
                    &r.symbols,
                    &mut r.refs,
                )
            } else {
                crate::types::FlowMeta::default()
            }
        }
        None => crate::types::FlowMeta::default(),
    };

    // Materialize a `{fn}$Ret` object type for each function that returns an object
    // literal (recorded by the flow pass). A call to the function yields this type,
    // so `createLogger().info` resolves to the synthesized member. The members
    // carry no type of their own — their presence under `{fn}$Ret` is the resolve.
    if !flow_meta.flow_return_object.is_empty() {
        let mk = |name: &str,
                  qname: &str,
                  kind: SymbolKind,
                  parent: Option<usize>,
                  line: u32| ExtractedSymbol {
            name: name.to_string(),
            qualified_name: qname.to_string(),
            kind,
            visibility: Some(Visibility::Public),
            start_line: line,
            end_line: line,
            start_col: 0,
            end_col: 0,
            byte_offset: 0,
            signature: None,
            doc_comment: None,
            scope_path: None,
            parent_index: parent,
            declared_type: None,
            return_type: None,
            param_types: Vec::new(),
            generic_params: Vec::new(),
        };
        for (fn_idx, members) in std::mem::take(&mut flow_meta.flow_return_object) {
            let (ret_qname, ret_name, line) = match r.symbols.get(fn_idx) {
                Some(s) => (
                    format!("{}$Ret", s.qualified_name),
                    format!("{}$Ret", s.name),
                    s.start_line,
                ),
                None => continue,
            };
            let iface_idx = r.symbols.len();
            r.symbols
                .push(mk(&ret_name, &ret_qname, SymbolKind::Interface, None, line));
            for m in &members {
                let m_qname = format!("{}.{}", ret_qname, m);
                r.symbols.push(mk(
                    m.as_str(),
                    &m_qname,
                    SymbolKind::Property,
                    Some(iface_idx),
                    line,
                ));
            }
        }
        // Keep the parallel origin vectors aligned when they were populated
        // (embedded-region files); empty vectors mean "all host language".
        if !symbol_origin_languages.is_empty() {
            symbol_origin_languages.resize(r.symbols.len(), None);
            symbol_from_snippet.resize(r.symbols.len(), false);
        }
    }

    // Extractor-time `FlowEmission`s for plugins whose flow detection is
    // file-structure-based (SDL parsing, `.proto` file scan) and can't move
    // to chain-walk resolver-time emission. The `indexer/resolve/mod.rs`
    // adapter flushes these alongside resolver-emitted flows.
    let plugin_flow_emissions: Vec<(u32, crate::indexer::resolve::flow_emit::FlowEmission)> =
        match walked.language {
            "graphql" => crate::languages::graphql::connectors::extract_schema_starts(&content),
            "vue" => crate::languages::vue::connectors::extract_vue_graphql_points(&content),
            "svelte" => {
                crate::languages::svelte::connectors::extract_svelte_graphql_points(&content)
            }
            "ruby" => crate::languages::ruby::connectors::extract_ruby_graphql(
                &content,
                &walked.relative_path,
            ),
            "python" => crate::languages::python::connectors::extract_python_graphql(&content),
            "typescript" | "tsx" | "javascript" | "jsx" => {
                crate::languages::typescript::connectors::extract_typescript_graphql(&content)
            }
            "proto" => crate::languages::proto::connectors::extract_proto_grpc_starts(&content),
            _ => Vec::new(),
        };

    // Component selector metadata feeding the project-wide selector map. Two
    // declaration sources land here:
    //   * Angular `@Component({selector})` / Ivy metadata — TypeScript/Angular.
    //   * Web-platform `customElements.define('tag', Class)` — any JS/TS, so a
    //     plain-HTML `<tag>` binds to its defined class without kebab→Pascal
    //     guessing.
    let mut component_selectors = if matches!(walked.language, "typescript" | "angular") {
        crate::languages::typescript::selectors::extract_component_selectors(&content, &r.symbols)
    } else {
        Vec::new()
    };
    if matches!(walked.language, "typescript" | "angular" | "javascript") {
        component_selectors.extend(
            crate::languages::typescript::selectors::extract_custom_element_defines(
                &content, &r.symbols,
            ),
        );
    }

    let mut parsed = ParsedFile {
        path: walked.relative_path.clone(),
        language: walked.language.to_string(),
        content_hash: hash,
        size,
        line_count,
        mtime,
        package_id: None, // assigned later by assign_package_ids
        symbols: r.symbols,
        refs: r.refs,
        routes: r.routes,
        db_sets: r.db_sets,
        symbol_origin_languages,
        ref_origin_languages,
        symbol_from_snippet,
        content: Some(content),
        has_errors: r.has_errors,
        flow: flow_meta,
        demand_contributions: Vec::new(),
        alias_targets: r.alias_targets,
        component_selectors,
        plugin_flow_emissions,
    };

    // populate_positions runs against the workspace arena when the caller
    // supplied one — so any TypeIds it produces (ChainSegment.declared_type_id,
    // type_defining return_type) are canonical across the whole index run.
    // When no arena was supplied (parse_file / parse_file_with_demand paths),
    // we build a throwaway per-file arena; populate_positions still does its
    // structural work but its TypeIds are dropped with the arena.
    let local_canonical_arena;
    let canonical_arena_ref = match arena {
        Some(a) => a,
        None => {
            local_canonical_arena = crate::type_checker::core::types::TypeArena::new();
            &local_canonical_arena
        }
    };
    super::canonical_form::populate_positions(&mut parsed, canonical_arena_ref);

    #[cfg(feature = "canonical-form-checked")]
    crate::indexer::canonical_form::assert_canonical(&parsed, canonical_arena_ref);

    Ok(parsed)
}

// ---------------------------------------------------------------------------
// Auto-generated vendored-header detection
// ---------------------------------------------------------------------------

/// Returns `true` for C/C++ header files that were produced by a platform
/// code generator (MIDL, IDL, …) rather than hand-written application code.
///
/// These files are commonly vendored into `docs/` or `third_party/` sub-
/// directories as API references but carry thousands of platform-specific
/// macro and typedef identifiers that the C extractor cannot distinguish
/// from real references. Indexing them produces huge numbers of spurious
/// `unresolved_refs` rows (STDMETHODCALLTYPE, IUnknown, BEGIN_INTERFACE,
/// LPCWSTR, UINT32, …) with zero resolvable cross-project value.
///
/// Detection is content-based (not path-based) so we don't have to guess
/// which directories a given test project chooses to vendor under. We scan
/// the first 2048 bytes only — every known generator emits its marker in
/// the top banner comment.
pub(super) fn is_generated_platform_header(language: &str, content: &str) -> bool {
    if !matches!(language, "c" | "cpp" | "c++") {
        return false;
    }
    let mut end = content.len().min(2048);
    while end > 0 && !content.is_char_boundary(end) {
        end -= 1;
    }
    let head = &content[..end];
    const MARKERS: &[&str] = &[
        "File created by MIDL compiler", // Microsoft MIDL (WebView2, WinRT, COM)
        "ALWAYS GENERATED file contains", // MIDL banner variant
        "Created by: flatc compiler",    // FlatBuffers generator
        "Generated by the protocol buffer compiler", // protoc C++ output
    ];
    MARKERS.iter().any(|m| head.contains(m))
}

// ---------------------------------------------------------------------------
// Vendored C/C++ library detection
// ---------------------------------------------------------------------------

const VENDORED_PATH_SEGMENTS: &[&str] = &["third_party", "vendor", "deps", "external", "extern"];

/// Content markers that appear in the opening banner of well-known single-header
/// vendored libraries — specific enough that they don't appear in project code
/// that merely includes/uses the library.
const VENDORED_CONTENT_MARKERS: &[&str] = &[
    "JSON for Modern C++",        // nlohmann/json (banner in the ASCII logo)
    "raylib v",                   // raylib — "raylib v5.5 - A simple..."
    "raymath v",                  // raymath companion header
    "Sean Barrett",               // STB single-file libs (all list this author)
    "Catch v",                    // Catch2 test framework (v1/v2 banner)
    "Catch2 v",                   // Catch2 (v3 banner)
    "dear imgui,",                // Dear ImGui — "dear imgui, v1.X..."
    "GLFW 3",                     // GLFW — "GLFW 3" in main header comment
    "miniaudio - Audio playback", // miniaudio — banner line
    "termbox2.h --",              // termbox2 self-documentation comment
    // Sokol: the distinctive self-doc format used in ALL sokol headers.
    // The banner is "sokol_<name>.h -- description" in the first comment block.
    // This does NOT appear in files that merely include sokol headers.
    ".h -- Drop-in",
    ".h -- drop-in",
    ".h -- Minimal",
    ".h -- minimal",
    ".h -- Simple",
    ".h -- simple",
];

/// Returns `true` if a C/C++ file should be classified as a vendored
/// third-party library rather than first-party project code.
///
/// Two-tier: path segment check (conventional vendor dirs) then content
/// banner check (common single-header libraries dropped anywhere in the tree).
pub(crate) fn is_c_vendored_file(language: &str, path: &str, content: &str) -> bool {
    if !matches!(language, "c" | "cpp" | "c++") {
        return false;
    }
    let norm = path.replace('\\', "/");
    for seg in VENDORED_PATH_SEGMENTS {
        if norm.contains(&format!("/{seg}/"))
            || norm.ends_with(&format!("/{seg}"))
            || norm.starts_with(&format!("{seg}/"))
        {
            return true;
        }
    }
    // Slice the first ~4 KiB of content to scan for vendor banners.  Naive
    // byte-slicing panics when the cut falls inside a multi-byte UTF-8 char
    // (e.g. the box-drawing glyphs some Redis deps use in comments). Walk
    // back to the nearest char boundary.
    let mut end = content.len().min(4096);
    while end > 0 && !content.is_char_boundary(end) {
        end -= 1;
    }
    let head = &content[..end];
    VENDORED_CONTENT_MARKERS.iter().any(|m| head.contains(m))
}

/// Extract a short human-readable description from a `catch_unwind` payload.
/// Used by the pipeline's panic guards so a scanner that misbehaves still
/// produces a legible warning instead of `<opaque Any>`.
pub(super) fn panic_message(payload: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&'static str>() {
        return (*s).to_string();
    }
    if let Some(s) = payload.downcast_ref::<String>() {
        return s.clone();
    }
    "<non-string panic payload>".to_string()
}
