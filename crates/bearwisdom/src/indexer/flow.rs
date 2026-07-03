// =============================================================================
// indexer/flow.rs — Shared per-language flow-typing query runner (R5 Sprint 2)
//
// Each language plugin that wants forward type inference, conditional
// narrowing, and call-site generics provides three tree-sitter queries via
// `FlowConfig`. This module runs them against a parsed source tree and
// produces a `FlowMeta` which the resolver consumes.
//
// Three queries, three outputs:
//
//   * `assignment_query` — matches `<lhs> = <rhs>` (initial declaration AND
//     reassignment). Captures:
//         @lhs   — the variable identifier
//         @rhs   — the expression whose type should bind to the LHS
//     Output: populates `flow.flow_binding_lhs[ref_idx] = lhs_symbol_idx`
//     for the ref whose byte range contains @rhs's outermost chain/call node.
//
//   * `type_guard_query` — matches type-narrowing expressions (TS
//     `instanceof`, `typeof x === "..."`, user-defined predicates; Python
//     `isinstance`; Rust `if let`). Captures:
//         @guard.local — the local being narrowed
//         @guard.type  — the type it narrows to (literal text)
//         @guard.body  — the node whose byte_range defines the narrowing scope
//     Output: appends a `Narrowing` with the body's byte range.
//
//   * `type_args_query` — matches explicit call-site type arguments
//     (TS `findOne<User>()`, Go `Do[User]()`). Captures:
//         @call.method   — the method node whose call site gets the args
//         @call.type_arg — one capture per generic argument, in declaration order
//     Output: sets `seg.type_args` on the matching MemberChain segment.
//
// Correlation: refs are correlated by `ExtractedRef::byte_offset`, which the
// per-language extractor must populate for refs that may be flow-bound. A ref
// matches a query capture if `ref.byte_offset` falls inside the capture's
// byte range and the ref's chain's final segment name matches the capture.
//
// Sprint 2 wires TypeScript. Sprint 3+ wires Python/Rust/etc.
// =============================================================================

use crate::types::{
    ChainSegment, DiscriminantNarrowing, EdgeKind, ExtractedRef, ExtractedSymbol, FlowMeta,
    Narrowing, SymbolKind,
};
use std::sync::{Arc, Mutex, OnceLock};
use tree_sitter::{Language, Node, Parser, Query, QueryCursor, StreamingIterator};

/// Per-language flow-typing configuration. Language plugins expose a
/// `&'static FlowConfig` via `LanguagePlugin::flow_config()` to opt into
/// flow-typing. Plugins that return `None` pay zero cost.
pub struct FlowConfig {
    pub strategy_prefix: &'static str,
    /// Tree-sitter query matching `<lhs> = <rhs>`. Captures `@lhs` (variable
    /// identifier) and `@rhs` (the expression whose type should propagate).
    pub assignment_query: &'static str,
    /// Tree-sitter query matching type-guard expressions whose true-branch
    /// narrows a local. Captures `@guard.local`, `@guard.type`, `@guard.body`.
    pub type_guard_query: &'static str,
    /// Tree-sitter query matching discriminated-union guards
    /// (`if (x.kind === "circle") { ... }`) whose true-branch narrows `x` to the
    /// union branch carrying that discriminant literal. Captures `@guard.local`
    /// (receiver), `@guard.prop` (discriminant property), `@guard.literal`
    /// (matched literal), and `@guard.body` (narrowed block). Empty = opt out.
    pub discriminant_guard_query: &'static str,
    /// Tree-sitter query matching explicit call-site type arguments.
    /// Captures `@call.method` and one `@call.type_arg` per generic argument.
    pub type_args_query: &'static str,
    /// Maps RHS tree-sitter node kinds to the wrapper type name they imply when
    /// no `@type` annotation and no resolvable ref are present. Empty for
    /// languages that do not use this inference. Populated by each language's
    /// `FlowConfig` literal; the runner inserts into `flow_binding_decl_type`
    /// when the RHS kind matches and no entry already exists for the binding.
    pub literal_type_kinds: &'static [(&'static str, &'static str)],
}

/// Skip flow queries on files larger than this threshold. Huge files are
/// dominated by generated bindings (`node_modules/**/*.d.ts`, machine-generated
/// Go struct tables, etc.) where local-variable flow adds no value, and the
/// tree-sitter query matcher can spend unbounded memory on deeply nested
/// captures. 512 KiB catches every real hand-written source file in the
/// quality baseline.
pub(crate) const MAX_FLOW_SOURCE_BYTES: usize = 512 * 1024;

/// Process-wide cache of compiled tree-sitter flow queries. `Query::new`
/// builds a matcher automaton — among the most expensive tree-sitter calls —
/// and the flow pass needs the same per-language queries for every file it
/// processes. Keyed by `(grammar, query-source-pointer)`: the
/// grammar because one `FlowConfig` source string can serve several grammars
/// (the TS config drives both the `.ts` and `.tsx` grammars, whose compiled
/// queries differ), the source pointer because each query source is a distinct
/// `&'static` literal. Both are stable for a run, so each `(grammar, query)`
/// pair compiles once. The lock is released across `Query::new`, so distinct
/// queries compile in parallel; a rare double-compile of the same key is
/// harmless (idempotent insert). The returned `Arc` is shared; each caller's
/// `QueryCursor` stays local.
fn cached_query(language: &Language, source: &'static str) -> Option<Arc<Query>> {
    static CACHE: OnceLock<Mutex<rustc_hash::FxHashMap<(Language, usize), Arc<Query>>>> =
        OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(rustc_hash::FxHashMap::default()));
    let key = (language.clone(), source.as_ptr() as usize);
    if let Some(query) = cache.lock().unwrap().get(&key) {
        return Some(Arc::clone(query));
    }
    let query = Arc::new(Query::new(language, source).ok()?);
    cache.lock().unwrap().insert(key, Arc::clone(&query));
    Some(query)
}

/// Run all three flow queries on `source` and return the populated `FlowMeta`.
///
/// The extractor's `symbols` and `refs` vecs are used to correlate captures
/// back to indices:
///   * LHS identifier text ↔ symbol_idx (by name + byte_offset check)
///   * RHS chain byte range ↔ ref_idx (via `ExtractedRef::byte_offset`)
///
/// Returns `FlowMeta::default()` when the grammar fails to load, the parse
/// doesn't produce a tree, or the source exceeds `MAX_FLOW_SOURCE_BYTES`.
pub fn run_flow_queries(
    source: &str,
    language: &Language,
    cfg: &FlowConfig,
    symbols: &[ExtractedSymbol],
    refs: &mut [ExtractedRef],
) -> FlowMeta {
    // Huge files (vendored .d.ts, generated code) bypass flow queries —
    // see `MAX_FLOW_SOURCE_BYTES` docstring for why.
    if source.len() > MAX_FLOW_SOURCE_BYTES {
        return flow_meta_offsets_only(refs);
    }

    let mut parser = Parser::new();
    if parser.set_language(language).is_err() {
        return FlowMeta::default();
    }
    let Some(tree) = parser.parse(source, None) else {
        return FlowMeta::default();
    };
    run_flow_queries_on_root(source, cfg, symbols, refs, tree.root_node())
}

/// Like `run_flow_queries`, but reuses a tree the caller already parsed for this
/// source + grammar. The indexer shares one parse across locals.scm filtering
/// and flow typing. Oversized files are still skipped — the query matcher, not
/// the parse, is the cost the size guard avoids.
pub fn run_flow_queries_with_tree(
    source: &str,
    cfg: &FlowConfig,
    symbols: &[ExtractedSymbol],
    refs: &mut [ExtractedRef],
    tree: &tree_sitter::Tree,
) -> FlowMeta {
    if source.len() > MAX_FLOW_SOURCE_BYTES {
        return flow_meta_offsets_only(refs);
    }
    run_flow_queries_on_root(source, cfg, symbols, refs, tree.root_node())
}

/// FlowMeta carrying only the ref byte offsets — the result for files skipped by
/// the size guard (the flow cursor downstream still needs the offsets).
fn flow_meta_offsets_only(refs: &[ExtractedRef]) -> FlowMeta {
    let mut meta = FlowMeta::default();
    meta.ref_byte_offsets = refs.iter().map(|r| r.byte_offset).collect();
    meta
}

/// Run every flow query against an already-parsed root. Shared by the parsing
/// (`run_flow_queries`) and tree-reusing (`run_flow_queries_with_tree`) entries.
fn run_flow_queries_on_root(
    source: &str,
    cfg: &FlowConfig,
    symbols: &[ExtractedSymbol],
    refs: &mut [ExtractedRef],
    root: Node,
) -> FlowMeta {
    let src_bytes = source.as_bytes();

    let mut meta = FlowMeta::default();
    meta.ref_byte_offsets = refs.iter().map(|r| r.byte_offset).collect();

    run_assignment_query(&root, src_bytes, cfg, symbols, refs, &mut meta);
    // Go table-driven anonymous-struct slices: type a `range` value variable as
    // the slice's anonymous-struct element. The element's fields are indexed as
    // members of the enclosing function, so the binding is a structural pass
    // over the Go AST rather than a flow query, and runs only for Go.
    if cfg.strategy_prefix == "go" {
        crate::languages::go::flow::bind_range_element_locals(&root, src_bytes, symbols, &mut meta);
    }
    run_type_guard_query(&root, src_bytes, cfg, &mut meta);
    run_discriminant_guard_query(&root, src_bytes, cfg, &mut meta);
    run_type_args_query(&root, src_bytes, cfg, refs);
    run_return_query(&root, src_bytes, cfg, symbols, refs, &mut meta);

    let reassignments = collect_reassignment_sites(&root, src_bytes, cfg);
    kill_narrowings_at_reassignments(&mut meta, &reassignments);

    // Per-function CFGs for the consumer's CFG-native lookup path. Dispatch
    // by `strategy_prefix` because `FlowConfig` does not yet carry the kind
    // table; the cleaner per-language plumbing (a `LanguagePlugin::cfg_node_kinds`
    // method) lands when more languages get tables.
    if let Some(kinds) = cfg_node_kinds_for(cfg.strategy_prefix) {
        meta.cfg =
            crate::indexer::flow_cfg::build_file_cfg(&root, src_bytes, kinds, &meta.narrowings);
    }

    meta
}

fn cfg_node_kinds_for(
    strategy_prefix: &str,
) -> Option<&'static crate::indexer::flow_cfg::CfgNodeKinds> {
    match strategy_prefix {
        "ts" | "js" => Some(&crate::indexer::flow_cfg::TS_CFG_KINDS),
        "java" => Some(&crate::indexer::flow_cfg::JAVA_CFG_KINDS),
        "python" => Some(&crate::indexer::flow_cfg::PYTHON_CFG_KINDS),
        "csharp" => Some(&crate::indexer::flow_cfg::CSHARP_CFG_KINDS),
        "rust" => Some(&crate::indexer::flow_cfg::RUST_CFG_KINDS),
        "go" => Some(&crate::indexer::flow_cfg::GO_CFG_KINDS),
        "c" => Some(&crate::indexer::flow_cfg::C_CFG_KINDS),
        "php" => Some(&crate::indexer::flow_cfg::PHP_CFG_KINDS),
        "lua" => Some(&crate::indexer::flow_cfg::LUA_CFG_KINDS),
        "groovy" => Some(&crate::indexer::flow_cfg::GROOVY_CFG_KINDS),
        "scala" => Some(&crate::indexer::flow_cfg::SCALA_CFG_KINDS),
        "kotlin" => Some(&crate::indexer::flow_cfg::KOTLIN_CFG_KINDS),
        "ruby" => Some(&crate::indexer::flow_cfg::RUBY_CFG_KINDS),
        "r" => Some(&crate::indexer::flow_cfg::R_CFG_KINDS),
        "dart" => Some(&crate::indexer::flow_cfg::DART_CFG_KINDS),
        "swift" => Some(&crate::indexer::flow_cfg::SWIFT_CFG_KINDS),
        "gdscript" => Some(&crate::indexer::flow_cfg::GDSCRIPT_CFG_KINDS),
        _ => None,
    }
}

/// All assignment-LHS sites, as `(variable_name, byte_offset)`. Reuses the
/// `assignment_query`'s `@lhs` capture — the same node the binding correlation
/// reads — but keeps only the variable name and its byte offset. A site whose
/// offset falls strictly inside a narrowing range marks where a re-definition
/// kills that narrowing; an offset at the binding's own declaration sits before
/// any guard range, so it is filtered out by the strict-interior test in
/// `kill_narrowings_at_reassignments` rather than by node kind.
fn collect_reassignment_sites(root: &Node, src: &[u8], cfg: &FlowConfig) -> Vec<(String, u32)> {
    let Some(query) = cached_query(&root.language(), cfg.assignment_query) else {
        return Vec::new();
    };
    let Some(lhs_cap) = query.capture_index_for_name("lhs") else {
        return Vec::new();
    };

    let mut sites = Vec::new();
    let mut cursor = QueryCursor::new();
    let mut it = cursor.matches(&*query, *root, src);
    while let Some(m) = it.next() {
        for cap in m.captures {
            if cap.index != lhs_cap {
                continue;
            }
            if let Ok(name) = cap.node.utf8_text(src) {
                if !name.is_empty() {
                    sites.push((name.to_string(), cap.node.start_byte() as u32));
                }
            }
        }
    }
    sites
}

/// Truncate every narrowing whose range straddles a re-definition of the same
/// variable. A reassignment at byte `B` kills the narrowed fact from `B`
/// onward, so a narrowing `[start, end)` with a matching reassignment at
/// `start < B < end` is shortened to `[start, B)`. The earliest such `B` wins
/// — the first re-definition ends the narrowing. Uses of the variable before
/// `B` stay narrowed; uses after `B` fall back to the declared type.
fn kill_narrowings_at_reassignments(meta: &mut FlowMeta, reassignments: &[(String, u32)]) {
    let earliest_kill = |name: &str, start: u32, end: u32| -> Option<u32> {
        reassignments
            .iter()
            .filter(|(n, b)| n == name && *b > start && *b < end)
            .map(|(_, b)| *b)
            .min()
    };

    for n in &mut meta.narrowings {
        if let Some(b) = earliest_kill(&n.name, n.byte_start, n.byte_end) {
            n.byte_end = b;
        }
    }
    for d in &mut meta.discriminant_narrowings {
        if let Some(b) = earliest_kill(&d.name, d.byte_start, d.byte_end) {
            d.byte_end = b;
        }
    }
}

/// Correlate an LHS binding name to its extractor symbol index: the symbol of
/// that name whose start line is closest to (but not after) `line`. Shared by the
/// identifier-binding and destructure-binding paths.
fn correlate_lhs_symbol(name: &str, line: u32, symbols: &[ExtractedSymbol]) -> Option<usize> {
    symbols
        .iter()
        .enumerate()
        .filter(|(_, s)| s.name == name && s.start_line <= line)
        .max_by_key(|(_, s)| s.start_line)
        .map(|(i, _)| i)
}

/// Find the ref whose value is the OUTERMOST expression of `rhs` — the one whose
/// type the binding takes. Excludes refs inside functions nested STRICTLY within
/// `rhs` (callback bodies: `const x = render(() => <Page/>)` is typed by `render`,
/// not by `Page`). The selection key, minimized:
///   1. leftmost callee start — the outer expression begins at the RHS start; its
///      arguments (`render(<App/>)`, `render(p1(), p2())`) anchor to the RIGHT.
///   2. longest chain — at a shared start, the outermost expression carries the
///      most segments: `a.b().field` over the inner `a.b()`, so the binding takes
///      the field's type, not the call's return.
///   3. value-producing kind — a bare call emits both a `Calls`/`Instantiates` ref
///      AND a co-located equal-length `TypeRef`; the value ref wins.
/// Shared by the identifier-binding and destructure-binding paths.
fn correlate_rhs_ref(refs: &[ExtractedRef], rhs: &Node, strategy_prefix: &str) -> Option<usize> {
    let r_start = rhs.start_byte() as u32;
    let r_end = rhs.end_byte() as u32;
    let nested_fn_ranges = cfg_node_kinds_for(strategy_prefix)
        .map(|kinds| nested_function_ranges(rhs, kinds))
        .unwrap_or_default();
    let value_rank = |k: EdgeKind| -> u8 {
        match k {
            EdgeKind::Calls | EdgeKind::Instantiates => 0,
            _ => 1,
        }
    };
    refs.iter()
        .enumerate()
        .filter(|(_, r)| {
            r.byte_offset >= r_start
                && r.byte_offset < r_end
                && !nested_fn_ranges
                    .iter()
                    .any(|(s, e)| r.byte_offset >= *s && r.byte_offset < *e)
        })
        .min_by_key(|(_, r)| {
            let segments = r.chain.as_ref().map(|c| c.segments.len()).unwrap_or(0);
            (r.byte_offset, std::cmp::Reverse(segments), value_rank(r.kind))
        })
        .map(|(i, _)| i)
}

fn run_assignment_query(
    root: &Node,
    src: &[u8],
    cfg: &FlowConfig,
    symbols: &[ExtractedSymbol],
    refs: &[ExtractedRef],
    meta: &mut FlowMeta,
) {
    let Some(query) = cached_query(&root.language(), cfg.assignment_query) else {
        return;
    };
    let Some(lhs_cap) = query.capture_index_for_name("lhs") else {
        return;
    };
    // `@rhs` (forward inference from the initializer) and `@type` (explicit
    // annotation) are both optional — a query may capture either or both. A
    // binding with only `@type` (`let x: T;`) seeds a declared type with no
    // ref to resolve; one with only `@rhs` (`let x = expr`) drives forward
    // inference as before.
    let rhs_cap = query.capture_index_for_name("rhs");
    let type_cap = query.capture_index_for_name("type");
    // `@rhs_unwrap` is `@rhs` whose initializer applies a fallible-unwrap
    // operator (Rust `?`). Correlated to a ref like `@rhs`, but the binding is
    // additionally flagged so the resolver peels one wrapper layer.
    let unwrap_cap = query.capture_index_for_name("rhs_unwrap");
    // Object-destructure bindings: `@destruct.bind` is the bound identifier,
    // `@destruct.key` the source field name (absent for shorthand `{ a }`, where
    // the field equals the bound name). Languages whose query omits these
    // captures get `None` here and the destructure branch never fires.
    let bind_cap = query.capture_index_for_name("destruct.bind");
    let key_cap = query.capture_index_for_name("destruct.key");

    let mut cursor = QueryCursor::new();
    let mut it = cursor.matches(&*query, *root, src);
    while let Some(m) = it.next() {
        let mut lhs_node: Option<Node> = None;
        let mut rhs_node: Option<Node> = None;
        let mut type_node: Option<Node> = None;
        let mut unwrap_node: Option<Node> = None;
        let mut bind_node: Option<Node> = None;
        let mut key_node: Option<Node> = None;
        for cap in m.captures {
            if cap.index == lhs_cap {
                lhs_node = Some(cap.node);
            } else if Some(cap.index) == rhs_cap {
                rhs_node = Some(cap.node);
            } else if Some(cap.index) == type_cap {
                type_node = Some(cap.node);
            } else if Some(cap.index) == unwrap_cap {
                unwrap_node = Some(cap.node);
            } else if Some(cap.index) == bind_cap {
                bind_node = Some(cap.node);
            } else if Some(cap.index) == key_cap {
                key_node = Some(cap.node);
            }
        }

        // Object-destructure binding: `const { a, b: c } = f()`. Each match carries
        // one bound identifier; type it from the FIELD on the RHS's yield type
        // (`R["a"]`), recorded against the RHS ref for the seeding loop to resolve.
        if let Some(bind) = bind_node {
            if let (Ok(bind_name), Some(rhs)) = (bind.utf8_text(src), rhs_node) {
                let bind_line = bind.start_position().row as u32;
                if let Some(lhs_idx) = correlate_lhs_symbol(bind_name, bind_line, symbols) {
                    // Shorthand `{ a }` → field == bound name; `{ b: c }` → `@destruct.key`.
                    let field_key = key_node
                        .and_then(|k| k.utf8_text(src).ok())
                        .unwrap_or(bind_name)
                        .to_string();
                    if let Some(ref_idx) = correlate_rhs_ref(refs, &rhs, cfg.strategy_prefix) {
                        meta.flow_binding_destructure
                            .entry(ref_idx)
                            .or_default()
                            .push((lhs_idx, field_key));
                        if rhs.kind() == "await_expression" {
                            meta.flow_binding_destructure_await.insert(ref_idx);
                        }
                    }
                }
            }
            continue;
        }
        let Some(lhs) = lhs_node else { continue };
        let lhs_name = match lhs.utf8_text(src) {
            Ok(t) => t,
            Err(_) => continue,
        };

        // Correlate LHS name → symbol_idx: the same-named symbol whose start line
        // is closest to (not after) the LHS row.
        let lhs_line = lhs.start_position().row as u32;
        let Some(lhs_idx) = correlate_lhs_symbol(lhs_name, lhs_line, symbols) else {
            continue;
        };

        // Explicit annotation: record the declared type text verbatim (trimmed
        // only). `intern_type_str` decomposes `Vec<Item>` into `Apply(Vec,
        // [Item])` itself, so the head still keys the member lookup AND the
        // generic argument survives for element-type projection (`v[0]`).
        if let Some(ty) = type_node {
            if let Ok(text) = ty.utf8_text(src) {
                let text = text.trim();
                if !text.is_empty() {
                    meta.flow_binding_decl_type
                        .insert(lhs_idx, text.to_string());
                }
            }
        }

        // Initializer: correlate RHS byte range → ref_idx. The ref whose
        // byte_offset is within [rhs.start_byte, rhs.end_byte) AND whose chain
        // covers the RHS's final segment wins. For a chain `foo.bar()` the
        // refs emitter creates a Calls ref at the call node — match the ref
        // with byte_offset in range AND latest (furthest-right) start. A
        // `@rhs_unwrap` capture (fallible `?`) is correlated the same way and
        // additionally flags the binding for wrapper peeling.
        let (rhs, is_unwrap) = match (rhs_node, unwrap_node) {
            (Some(r), _) => (Some(r), false),
            (None, Some(u)) => (Some(u), true),
            (None, None) => (None, false),
        };
        if let Some(rhs) = rhs {
            let ref_idx = correlate_rhs_ref(refs, &rhs, cfg.strategy_prefix);
            if let Some(ref_idx) = ref_idx {
                meta.flow_binding_lhs.insert(ref_idx, lhs_idx);
                if is_unwrap {
                    meta.flow_binding_unwrap.insert(lhs_idx);
                }
                if rhs.kind() == "await_expression" {
                    meta.flow_binding_await.insert(lhs_idx);
                }
            } else if !meta.flow_binding_decl_type.contains_key(&lhs_idx) {
                // No resolvable RHS ref and no annotation: classify the literal
                // node kind against the language's wrapper-type table. Skips
                // bindings that already have a declared type from the `@type`
                // capture above so the annotation always wins.
                if let Some((_, wrapper)) = cfg
                    .literal_type_kinds
                    .iter()
                    .find(|(k, _)| *k == rhs.kind())
                {
                    meta.flow_binding_decl_type
                        .insert(lhs_idx, (*wrapper).to_string());
                }
            }
        }
    }
}

/// Return-expression query for a language, keyed by `strategy_prefix`. Mirrors
/// `cfg_node_kinds_for`: a grammar opts into body-based return-type inference
/// by supplying a query here rather than carrying it on every `FlowConfig`
/// literal. An empty string opts out (no return capture).
fn return_query_for(strategy_prefix: &str) -> &'static str {
    match strategy_prefix {
        "ts" | "js" => crate::languages::typescript::flow::TS_RETURN_QUERY,
        "python" => crate::languages::python::flow::PY_RETURN_QUERY,
        "go" => crate::languages::go::flow::GO_RETURN_QUERY,
        "java" => crate::languages::java::flow::JAVA_RETURN_QUERY,
        "rust" => crate::languages::rust_lang::flow::RUST_RETURN_QUERY,
        "csharp" => crate::languages::csharp::flow::CSHARP_RETURN_QUERY,
        "kotlin" => crate::languages::kotlin::flow::KOTLIN_RETURN_QUERY,
        "php" => crate::languages::php::flow::PHP_RETURN_QUERY,
        "scala" => crate::languages::scala::flow::SCALA_RETURN_QUERY,
        "ruby" => crate::languages::ruby::flow::RUBY_RETURN_QUERY,
        _ => "",
    }
}

/// Capture each `return <expr>` (at arbitrary depth inside its function body)
/// AND each concise expression-body (`() => expr`, `def f = expr`, `fun f() =
/// expr`) as a candidate for its function's inferred return type.
///
/// The query captures the returned expression two ways:
///   * `@return.expr` — the operand of an explicit `return` keyword, at
///     arbitrary depth inside the body.
///   * `@return.tail` — the body field of a function whose body is a single
///     expression with no `return` keyword. A tail capture whose kind is a
///     `CfgNodeKinds.block_kinds` node is SKIPPED: a block body's return value
///     comes from its explicit `return` (caught by `@return.expr`) or its
///     final statement (a deeper sub-case), never from the block node itself.
///
/// Function IDENTITY is resolved structurally by an ancestor-walk, because
/// tree-sitter queries have no descendant/ancestor axis — a single pattern
/// matching a `return` at arbitrary depth inside a function node is a structure
/// error. Walking parents to the nearest `CfgNodeKinds.function_kinds` node
/// both finds the owning function AND enforces soundness: a return inside a
/// nested arrow/callback resolves to the lambda — not the outer named function
/// — so a callback return is never misattributed even though descendant
/// capture now reaches it. Returns nested in `if`/`for`/`switch` resolve to the
/// enclosing named function and ARE attributed (the widening over the prior
/// direct-body query).
///
/// The owning function's name is its `name` field; an unnamed function
/// (arrow/lambda/closure) borrows the name of an immediately-wrapping
/// declarator (`const f = () => …`), which the extractor emits as a Function
/// symbol at the declarator row — so the same name + nearest-line correlation
/// covers both forms. An unnamed function with no naming wrapper yields no
/// candidate (Unknown over a guess).
///
/// Gated on `cfg_node_kinds_for(cfg.strategy_prefix)`: a language with no
/// `CfgNodeKinds` table has no function-boundary set to walk, so no return
/// query is honored for it. Populates `flow_return_lhs[ref_idx] =
/// fn_symbol_idx`. No-op when the language supplies no return query.
fn run_return_query(
    root: &Node,
    src: &[u8],
    cfg: &FlowConfig,
    symbols: &[ExtractedSymbol],
    refs: &[ExtractedRef],
    meta: &mut FlowMeta,
) {
    let query_src = return_query_for(cfg.strategy_prefix);
    if query_src.is_empty() {
        return;
    }
    // The ancestor-walk that resolves function identity needs the
    // function-boundary node-kind set. Without it no return query is honored.
    let Some(kinds) = cfg_node_kinds_for(cfg.strategy_prefix) else {
        return;
    };
    let Some(query) = cached_query(&root.language(), query_src) else {
        return;
    };
    let expr_cap = query.capture_index_for_name("return.expr");
    let tail_cap = query.capture_index_for_name("return.tail");
    if expr_cap.is_none() && tail_cap.is_none() {
        return;
    }

    let mut cursor = QueryCursor::new();
    let mut it = cursor.matches(&*query, *root, src);
    while let Some(m) = it.next() {
        let mut expr_node: Option<Node> = None;
        for cap in m.captures {
            if Some(cap.index) == expr_cap {
                expr_node = Some(cap.node);
            } else if Some(cap.index) == tail_cap {
                // A concise expression-body return. A block body is not a tail
                // expression — its return value comes from an explicit `return`
                // (the `@return.expr` arm) or its last statement (a deeper
                // sub-case), so skip a tail capture that is itself a block.
                if !kinds.block_kinds.contains(&cap.node.kind()) {
                    expr_node = Some(cap.node);
                }
            }
        }
        let Some(expr) = expr_node else { continue };
        attribute_return_expr(&expr, kinds, src, symbols, refs, meta);
    }

    run_tail_block_return(root, src, kinds, symbols, refs, meta);
}

/// Attribute one return expression to its owning function and record the bind.
///
/// Resolves the owning function structurally — the nearest enclosing
/// `function_kinds` node — so a return inside a nested lambda resolves to the
/// lambda (never the outer named function). The owner's name → symbol_idx is
/// correlated by name + nearest function-like declaration at/above the name's
/// row (the arrow-const form extracts as a Function at the declarator row, so
/// the same correlation covers it). The return expression's byte range → the
/// furthest-right ref it contains, mirroring the assignment-RHS correlation.
/// A return whose owner is anonymous (no name node) or whose body carries no
/// ref is a silent no-op.
fn attribute_return_expr(
    expr: &Node,
    kinds: &crate::indexer::flow_cfg::CfgNodeKinds,
    src: &[u8],
    symbols: &[ExtractedSymbol],
    refs: &[ExtractedRef],
    meta: &mut FlowMeta,
) {
    let Some(enclosing_fn) = nearest_function_ancestor(expr, kinds) else {
        return;
    };
    let Some(fn_name_node) = function_name_node(&enclosing_fn, kinds) else {
        return;
    };
    let Ok(fn_name) = fn_name_node.utf8_text(src) else {
        return;
    };

    let fn_line = fn_name_node.start_position().row as u32;
    let fn_idx = symbols
        .iter()
        .enumerate()
        .filter(|(_, s)| {
            s.name == fn_name
                && s.start_line <= fn_line
                && matches!(
                    s.kind,
                    SymbolKind::Function | SymbolKind::Method | SymbolKind::Constructor
                )
        })
        .max_by_key(|(_, s)| s.start_line)
        .map(|(i, _)| i);
    let Some(fn_idx) = fn_idx else { return };

    // An object-literal return (`return { info, error }`) IS the function's
    // structural return type. Record its property names so a synthetic `{fn}$Ret`
    // type carrying those members is materialized, rather than mis-attributing the
    // return to the last property's ref below.
    if expr.kind() == "object" {
        let members = object_property_names(expr, src);
        if !members.is_empty() {
            meta.flow_return_object.push((fn_idx, members));
        }
        return;
    }

    // A ref inside a NESTED function within the return expression belongs to
    // that nested scope, not the owner — `return makeThing(x => x.foo())`
    // returns `makeThing`'s type, not `x.foo`'s. Exclude any ref whose offset
    // falls inside a `function_kinds` descendant of the expression so the
    // furthest-right ref picked is the owner's own, never a nested callback's.
    let nested_fn_ranges = nested_function_ranges(expr, kinds);
    let e_start = expr.start_byte() as u32;
    let e_end = expr.end_byte() as u32;
    let ref_idx = refs
        .iter()
        .enumerate()
        .filter(|(_, r)| {
            r.byte_offset >= e_start
                && r.byte_offset < e_end
                && !nested_fn_ranges
                    .iter()
                    .any(|(s, e)| r.byte_offset >= *s && r.byte_offset < *e)
        })
        .max_by_key(|(_, r)| r.byte_offset)
        .map(|(i, _)| i);
    if let Some(ref_idx) = ref_idx {
        meta.flow_return_lhs.insert(ref_idx, fn_idx);
        return;
    }
    // No ref inside the return expression. A bare identifier return
    // (`return queryClient` / `return client`) carries no ref — a local/param
    // read isn't a cross-symbol reference — so the ref-based path above misses
    // it. Record `(fn, identifier)` so the resolver can type the identifier
    // against the function's parameters / typed locals and harvest that as a
    // return-type candidate. Only a single bare identifier qualifies; a
    // compound expression with no ref carries no nameable type here.
    if expr.kind() == "identifier" {
        if let Ok(ident) = expr.utf8_text(src) {
            if !ident.is_empty() {
                meta.flow_return_ident.push((fn_idx, ident.to_string()));
            }
        }
    }
}

/// The property names declared directly in an object literal — `info`, `error`
/// from `{ info, error, warn }`. Covers shorthand (`info`), keyed pairs
/// (`info: x`), and method shorthand (`info() {}`); skips spreads / computed keys.
fn object_property_names(object_node: &Node, src: &[u8]) -> Vec<String> {
    let mut names = Vec::new();
    let mut cursor = object_node.walk();
    for child in object_node.named_children(&mut cursor) {
        let name = match child.kind() {
            "shorthand_property_identifier" | "property_identifier" => {
                child.utf8_text(src).ok().map(|s| s.to_string())
            }
            "pair" => child
                .child_by_field_name("key")
                .filter(|k| matches!(k.kind(), "property_identifier" | "identifier"))
                .and_then(|k| k.utf8_text(src).ok())
                .map(|s| s.to_string()),
            "method_definition" => child
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(src).ok())
                .map(|s| s.to_string()),
            _ => None,
        };
        if let Some(n) = name {
            if !n.is_empty() {
                names.push(n);
            }
        }
    }
    names
}

/// Byte ranges of every `function_kinds` node nested inside `expr` (lambdas /
/// closures within a returned call). Refs inside these belong to the nested
/// scope, not the function whose return `expr` is.
fn nested_function_ranges(
    expr: &Node,
    kinds: &crate::indexer::flow_cfg::CfgNodeKinds,
) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    let mut c = expr.walk();
    let mut stack: Vec<Node> = expr.named_children(&mut c).collect();
    while let Some(n) = stack.pop() {
        if kinds.function_kinds.contains(&n.kind()) {
            out.push((n.start_byte() as u32, n.end_byte() as u32));
            // Don't descend into a nested function's own nested functions —
            // the outer range already covers every ref inside it.
            continue;
        }
        let mut cc = n.walk();
        for ch in n.named_children(&mut cc) {
            stack.push(ch);
        }
    }
    out
}

/// Tail-of-block implicit return for expression-oriented grammars
/// (`block_tail_returns`): `fn f() -> T { …; e }` / `def f = { …; e }` /
/// Ruby method bodies return their block's FINAL expression with no `return`
/// keyword. A tree-sitter query has no "last named child of a block" axis, so
/// this is a structural pass — the inverse of `nearest_function_ancestor`:
/// for every function node, find its body block and take the block's last
/// named child as a return expression.
///
/// Soundness (widening-only): the pass fires only when `block_tail_returns` is
/// set — the languages where a bare trailing expression is genuinely the return
/// value. The last named child is rejected when it is
///   * a statement (`*_statement`) — a semicolon-terminated expression returns
///     unit (Rust `build();`), not the call's type;
///   * a binding (`*_declaration` / `*_definition`) — a block ending in a
///     `let`/`val` binds and returns unit;
///   * an explicit `return` node — already attributed by the `@return.expr` arm
///     (avoids a double-fire);
///   * a nested block (`block_kinds`) — its own tail is handled when the walk
///     reaches its enclosing function, not here.
/// Otherwise the child is the tail expression and is attributed exactly like a
/// `return` operand. The ref correlation only binds when the tail expression
/// actually contains a ref, so a tail that is a literal/identifier is a no-op.
fn run_tail_block_return(
    root: &Node,
    src: &[u8],
    kinds: &crate::indexer::flow_cfg::CfgNodeKinds,
    symbols: &[ExtractedSymbol],
    refs: &[ExtractedRef],
    meta: &mut FlowMeta,
) {
    if !kinds.block_tail_returns {
        return;
    }
    for fn_node in descendant_function_nodes(root, kinds) {
        let Some(body) = crate::indexer::flow_cfg::find_function_body(&fn_node, kinds) else {
            continue;
        };
        let mut c = body.walk();
        let Some(last) = body.named_children(&mut c).last() else {
            continue;
        };
        if is_non_return_tail(&last, kinds) {
            continue;
        }
        attribute_return_expr(&last, kinds, src, symbols, refs, meta);
    }
}

/// Whether `node` (a block's last named child) is NOT a tail-return expression:
/// a statement, a binding declaration/definition, an explicit `return` already
/// caught by the `@return.expr` arm, or a nested block whose own tail is handled
/// by its enclosing function.
fn is_non_return_tail(node: &Node, kinds: &crate::indexer::flow_cfg::CfgNodeKinds) -> bool {
    let k = node.kind();
    k.ends_with("_statement")
        || k.ends_with("_declaration")
        || k.ends_with("_definition")
        || k.contains("return")
        || kinds.block_kinds.contains(&k)
}

/// Every `function_kinds` node in the tree, in document order. The structural
/// counterpart to `nearest_function_ancestor` for the tail-of-block pass, which
/// must visit each function rather than walk up from a query capture.
fn descendant_function_nodes<'a>(
    root: &Node<'a>,
    kinds: &crate::indexer::flow_cfg::CfgNodeKinds,
) -> Vec<Node<'a>> {
    let mut out = Vec::new();
    let mut stack = vec![*root];
    while let Some(n) = stack.pop() {
        if kinds.function_kinds.contains(&n.kind()) {
            out.push(n);
        }
        let mut c = n.walk();
        for ch in n.named_children(&mut c) {
            stack.push(ch);
        }
    }
    out
}

/// Walk parents from `node` to the first ancestor whose kind is in
/// `kinds.function_kinds`. `None` when the node has no enclosing function (a
/// top-level return, structurally absent in the grammars that have function
/// boundaries).
/// Whether a node kind names a bound identifier: any `*identifier` kind
/// (`identifier` / `field_identifier` / `property_identifier` / `simple_identifier`)
/// or the bare `name` kind some grammars use for declaration names.
fn is_identifier_kind(kind: &str) -> bool {
    kind.contains("identifier") || kind == "name"
}

fn nearest_function_ancestor<'a>(
    node: &Node<'a>,
    kinds: &crate::indexer::flow_cfg::CfgNodeKinds,
) -> Option<Node<'a>> {
    let mut cur = node.parent();
    while let Some(n) = cur {
        if kinds.function_kinds.contains(&n.kind()) {
            return Some(n);
        }
        cur = n.parent();
    }
    None
}

/// The identifier node naming `fn_node`. A named function (declaration /
/// method / def) carries a `name` field. An unnamed function (arrow / lambda /
/// closure / func_literal) has none — its name is borrowed from an
/// immediately-wrapping declarator: the nearest ancestor between the function
/// and the next function boundary whose `declarator_name_field` (or `name`
/// field) yields an identifier. `None` when no name can be resolved (an
/// anonymous callback passed inline), which drops the return candidate.
fn function_name_node<'a>(
    fn_node: &Node<'a>,
    kinds: &crate::indexer::flow_cfg::CfgNodeKinds,
) -> Option<Node<'a>> {
    if let Some(name) = fn_node.child_by_field_name("name") {
        return Some(name);
    }
    // Unnamed function: borrow the binding name from a wrapping declarator
    // (`const f = () => …`, `val f = { … }`). Stop at the next function
    // boundary — an arrow nested in another function is anonymous.
    let mut cur = fn_node.parent();
    while let Some(n) = cur {
        if kinds.function_kinds.contains(&n.kind()) {
            break;
        }
        for field in [kinds.declarator_name_field, "name"] {
            let Some(name) = n.child_by_field_name(field) else {
                continue;
            };
            if is_identifier_kind(name.kind()) {
                return Some(name);
            }
            // The name field may wrap the identifier (Kotlin
            // `variable_declaration > identifier`); descend one level.
            let mut c = name.walk();
            let inner = name
                .named_children(&mut c)
                .find(|ch| is_identifier_kind(ch.kind()));
            if let Some(inner) = inner {
                return Some(inner);
            }
        }
        cur = n.parent();
    }
    None
}

fn run_type_guard_query(root: &Node, src: &[u8], cfg: &FlowConfig, meta: &mut FlowMeta) {
    if cfg.type_guard_query.trim().is_empty() {
        return;
    }
    let Some(query) = cached_query(&root.language(), cfg.type_guard_query) else {
        return;
    };
    let local_cap = query.capture_index_for_name("guard.local");
    let type_cap = query.capture_index_for_name("guard.type");
    let body_cap = query.capture_index_for_name("guard.body");
    let (Some(local_cap), Some(type_cap), Some(body_cap)) = (local_cap, type_cap, body_cap) else {
        return;
    };

    let mut cursor = QueryCursor::new();
    let mut it = cursor.matches(&*query, *root, src);
    while let Some(m) = it.next() {
        let mut local: Option<Node> = None;
        let mut ty: Option<Node> = None;
        let mut body: Option<Node> = None;
        for cap in m.captures {
            if cap.index == local_cap {
                local = Some(cap.node);
            } else if cap.index == type_cap {
                ty = Some(cap.node);
            } else if cap.index == body_cap {
                body = Some(cap.node);
            }
        }
        let (Some(local), Some(ty), Some(body)) = (local, ty, body) else {
            continue;
        };
        let name = match local.utf8_text(src) {
            Ok(t) => t.to_string(),
            Err(_) => continue,
        };
        let narrowed = match ty.utf8_text(src) {
            Ok(t) => strip_type_literal(t),
            Err(_) => continue,
        };
        if name.is_empty() || narrowed.is_empty() {
            continue;
        }
        meta.narrowings.push(Narrowing {
            name,
            narrowed_type: narrowed,
            byte_start: body.start_byte() as u32,
            byte_end: body.end_byte() as u32,
        });
    }
}

fn run_discriminant_guard_query(root: &Node, src: &[u8], cfg: &FlowConfig, meta: &mut FlowMeta) {
    if cfg.discriminant_guard_query.trim().is_empty() {
        return;
    }
    let Some(query) = cached_query(&root.language(), cfg.discriminant_guard_query) else {
        return;
    };
    let local_cap = query.capture_index_for_name("guard.local");
    let prop_cap = query.capture_index_for_name("guard.prop");
    let lit_cap = query.capture_index_for_name("guard.literal");
    let (Some(local_cap), Some(prop_cap), Some(lit_cap)) = (local_cap, prop_cap, lit_cap) else {
        return;
    };
    // `@guard.body` scopes a positive guard (`if (x.kind === "lit") { ... }`);
    // `@guard.early_exit` (the `if` node of `if (x.kind !== "lit") return;`)
    // scopes a negated guard over the rest of the enclosing block. Either may
    // be absent depending on which arms a language's query ships.
    let body_cap = query.capture_index_for_name("guard.body");
    let exit_cap = query.capture_index_for_name("guard.early_exit");

    let mut cursor = QueryCursor::new();
    let mut it = cursor.matches(&*query, *root, src);
    while let Some(m) = it.next() {
        let mut local: Option<Node> = None;
        let mut prop: Option<Node> = None;
        let mut lit: Option<Node> = None;
        let mut body: Option<Node> = None;
        let mut exit: Option<Node> = None;
        for cap in m.captures {
            if cap.index == local_cap {
                local = Some(cap.node);
            } else if cap.index == prop_cap {
                prop = Some(cap.node);
            } else if cap.index == lit_cap {
                lit = Some(cap.node);
            } else if Some(cap.index) == body_cap {
                body = Some(cap.node);
            } else if Some(cap.index) == exit_cap {
                exit = Some(cap.node);
            }
        }
        let (Some(local), Some(prop), Some(lit)) = (local, prop, lit) else {
            continue;
        };
        let name = match local.utf8_text(src) {
            Ok(t) => t.to_string(),
            Err(_) => continue,
        };
        let prop = match prop.utf8_text(src) {
            Ok(t) => t.to_string(),
            Err(_) => continue,
        };
        let literal = match lit.utf8_text(src) {
            Ok(t) => t.to_string(),
            Err(_) => continue,
        };
        if name.is_empty() || prop.is_empty() || literal.is_empty() {
            continue;
        }

        // Negated early-exit guard scopes AFTER the `if`, over the rest of the
        // enclosing block, and only when the consequence actually exits.
        let (byte_start, byte_end, negate) = if let Some(exit) = exit {
            if !is_early_exit_consequence(&exit) {
                continue;
            }
            let start = exit.end_byte() as u32;
            let end = exit.parent().map(|p| p.end_byte() as u32).unwrap_or(start);
            if end <= start {
                continue;
            }
            (start, end, true)
        } else if let Some(body) = body {
            (body.start_byte() as u32, body.end_byte() as u32, false)
        } else {
            continue;
        };

        meta.discriminant_narrowings.push(DiscriminantNarrowing {
            name,
            prop,
            literal,
            byte_start,
            byte_end,
            negate,
        });
    }
}

/// True when an `if` node's consequence is an early exit (`return` / `throw` /
/// `break` / `continue`), bare or as the sole statement of a block. Gates the
/// negated discriminant guard: only an exit makes the guard's negation hold for
/// the rest of the enclosing block.
fn is_early_exit_consequence(if_node: &Node) -> bool {
    fn is_exit(kind: &str) -> bool {
        matches!(
            kind,
            "return_statement" | "throw_statement" | "break_statement" | "continue_statement"
        )
    }
    let Some(cons) = if_node.child_by_field_name("consequence") else {
        return false;
    };
    if is_exit(cons.kind()) {
        return true;
    }
    if cons.kind() == "statement_block" {
        let mut c = cons.walk();
        return cons.named_children(&mut c).any(|n| is_exit(n.kind()));
    }
    false
}

fn run_type_args_query(root: &Node, src: &[u8], cfg: &FlowConfig, refs: &mut [ExtractedRef]) {
    if cfg.type_args_query.trim().is_empty() {
        return;
    }
    let Some(query) = cached_query(&root.language(), cfg.type_args_query) else {
        return;
    };
    let method_cap = query.capture_index_for_name("call.method");
    let arg_cap = query.capture_index_for_name("call.type_arg");
    let (Some(method_cap), Some(arg_cap)) = (method_cap, arg_cap) else {
        return;
    };

    let mut cursor = QueryCursor::new();
    let mut it = cursor.matches(&*query, *root, src);
    while let Some(m) = it.next() {
        let mut method_node: Option<Node> = None;
        let mut type_args: Vec<String> = Vec::new();
        for cap in m.captures {
            if cap.index == method_cap {
                method_node = Some(cap.node);
            } else if cap.index == arg_cap {
                if let Ok(text) = cap.node.utf8_text(src) {
                    type_args.push(text.to_string());
                }
            }
        }
        let (Some(method), false) = (method_node, type_args.is_empty()) else {
            continue;
        };
        let method_name = match method.utf8_text(src) {
            Ok(t) => t,
            Err(_) => continue,
        };
        let m_start = method.start_byte() as u32;
        let m_end = method.end_byte() as u32;

        // Locate the ref whose chain's last segment matches this method node.
        for r in refs.iter_mut() {
            let Some(chain) = r.chain.as_mut() else {
                continue;
            };
            let Some(last) = chain.segments.last_mut() else {
                continue;
            };
            if last.name != method_name {
                continue;
            }
            // Ref byte_offset should fall near the method node's range.
            if r.byte_offset < m_start || r.byte_offset > m_end {
                continue;
            }
            // Don't overwrite existing type_args populated by the extractor.
            if last.type_args.is_empty() {
                last.type_args = type_args.clone();
            }
            break;
        }
    }
    let _ = ChainSegment {
        // silence unused import in release builds
        name: String::new(),
        node_kind: String::new(),
        kind: crate::types::SegmentKind::Identifier,
        declared_type: None,
        type_args: Vec::new(),
        optional_chaining: false,
        byte_offset: 0,
        declared_type_id: None,
        is_call: false,
        call_args: Vec::new(),
        type_arg_ids: Vec::new(),
    };
}

/// Strip surrounding quotes from a literal type string (used in
/// `typeof x === "string"`) or leave a type_identifier untouched.
fn strip_type_literal(s: &str) -> String {
    let trimmed = s.trim();
    let bytes = trimmed.as_bytes();
    if bytes.len() >= 2
        && ((bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[bytes.len() - 1] == b'\''))
    {
        return trimmed[1..trimmed.len() - 1].to_string();
    }
    trimmed.to_string()
}
