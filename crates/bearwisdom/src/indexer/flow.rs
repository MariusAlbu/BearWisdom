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
//     language-provided runtime tests and user-defined predicates; other
//     other language-owned predicate forms). Captures:
//         @guard.local — the local being narrowed
//         @guard.type  — the type it narrows to (literal text)
//         @guard.body  — the node whose byte_range defines the narrowing scope
//     Output: appends a `Narrowing` with the body's byte range.
//
//   * `type_args_query` — matches explicit call-site type arguments
//     (a language-owned type-argument call form). Captures:
//         @call.method   — the method node whose call site gets the args
//         @call.type_arg — one capture per generic argument, in declaration order
//     Output: sets `seg.type_args` on the matching MemberChain segment.
//
// Correlation: refs are correlated by `ExtractedRef::byte_offset`, which the
// per-language extractor must populate for refs that may be flow-bound. A ref
// matches a query capture if `ref.byte_offset` falls inside the capture's
// byte range and the ref's chain's final segment name matches the capture.
//
// Each participating language supplies its own flow facts.
// =============================================================================

use crate::types::{
    ChainSegment, DiscriminantNarrowing, ExtractedRef, ExtractedSymbol, FlowMeta, Narrowing,
    SymbolKind,
};
use std::sync::{Arc, Mutex, OnceLock};

use super::flow_bindings::nested_function_ranges;
pub use super::flow_bindings::BindingSymbols;
use tree_sitter::{Language, Node, Parser, Query, QueryCursor, StreamingIterator};

pub(crate) const TUPLE_INDEX_KEY_PREFIX: &str = super::flow_assignments::TUPLE_INDEX_KEY_PREFIX;

#[path = "flow_identity.rs"]
pub(super) mod identity;

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
    /// Tree-sitter query matching discriminated-union guards whose positive
    /// branch narrows a local to the union branch carrying a discriminant
    /// literal. Captures `@guard.local`
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
/// declaration tables, etc.) where local-variable flow adds no value, and the
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
pub(super) fn cached_query(language: &Language, source: &'static str) -> Option<Arc<Query>> {
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
/// Grammar/parse failure returns empty metadata. Large sources retain identity
/// recipes, while optional flow queries remain behind `MAX_FLOW_SOURCE_BYTES`.
pub fn run_flow_queries(
    source: &str,
    language: &Language,
    cfg: &FlowConfig,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut [ExtractedRef],
    bindings: BindingSymbols,
) -> FlowMeta {
    let Some(plugin) = crate::languages::default_registry().flow_plugin(cfg.strategy_prefix) else {
        return FlowMeta::default();
    };
    run_flow_queries_with_plugin(source, language, cfg, Some(plugin), symbols, refs, bindings)
}

/// Like [`run_flow_queries`], with language-owned identity syntax supplied by
/// the active plugin. Production indexing supplies the plugin directly;
/// callers with only a flow configuration recover it through its
/// adapter-declared strategy alias.
pub fn run_flow_queries_with_plugin(
    source: &str,
    language: &Language,
    cfg: &FlowConfig,
    plugin: Option<&dyn crate::languages::LanguagePlugin>,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut [ExtractedRef],
    bindings: BindingSymbols,
) -> FlowMeta {
    let mut parser = Parser::new();
    if parser.set_language(language).is_err() {
        return FlowMeta::default();
    }
    let Some(tree) = parser.parse(source, None) else {
        return FlowMeta::default();
    };
    run_flow_queries_on_root(
        source,
        cfg,
        plugin,
        symbols,
        refs,
        tree.root_node(),
        bindings,
    )
}

/// Like `run_flow_queries`, but reuses a tree the caller already parsed for this
/// source + grammar. The indexer shares one parse across locals.scm filtering
/// and flow typing. Oversized files retain source IDs but skip query matching.
pub fn run_flow_queries_with_tree(
    source: &str,
    cfg: &FlowConfig,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut [ExtractedRef],
    tree: &tree_sitter::Tree,
    bindings: BindingSymbols,
) -> FlowMeta {
    let Some(plugin) = crate::languages::default_registry().flow_plugin(cfg.strategy_prefix) else {
        return FlowMeta::default();
    };
    run_flow_queries_with_tree_and_plugin(source, cfg, Some(plugin), symbols, refs, tree, bindings)
}

/// Like [`run_flow_queries_with_tree`], with language-owned identity syntax
/// supplied by the active plugin.
pub fn run_flow_queries_with_tree_and_plugin(
    source: &str,
    cfg: &FlowConfig,
    plugin: Option<&dyn crate::languages::LanguagePlugin>,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut [ExtractedRef],
    tree: &tree_sitter::Tree,
    bindings: BindingSymbols,
) -> FlowMeta {
    run_flow_queries_on_root(
        source,
        cfg,
        plugin,
        symbols,
        refs,
        tree.root_node(),
        bindings,
    )
}

/// Run every flow query against an already-parsed root. Shared by the parsing
/// (`run_flow_queries`) and tree-reusing (`run_flow_queries_with_tree`) entries.
fn run_flow_queries_on_root(
    source: &str,
    cfg: &FlowConfig,
    plugin: Option<&dyn crate::languages::LanguagePlugin>,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut [ExtractedRef],
    root: Node,
    bindings: BindingSymbols,
) -> FlowMeta {
    let src_bytes = source.as_bytes();

    let mut meta = identity::capture(
        source,
        plugin,
        cfg.strategy_prefix,
        symbols,
        refs,
        root,
        bindings,
    );
    if source.len() > MAX_FLOW_SOURCE_BYTES {
        return meta;
    }
    let cfg_kinds = plugin.and_then(|plugin| plugin.flow_cfg_node_kinds());
    run_assignment_query(
        &root, src_bytes, cfg, symbols, refs, &mut meta, bindings, plugin, cfg_kinds,
    );
    if let Some(plugin) = plugin {
        plugin.augment_flow(root, src_bytes, symbols, refs, &mut meta);
    }
    run_type_guard_query(&root, src_bytes, cfg, plugin, &mut meta);
    run_discriminant_guard_query(&root, src_bytes, cfg, plugin, &mut meta);
    run_type_args_query(&root, src_bytes, cfg, refs);
    let return_query = plugin.and_then(|plugin| plugin.flow_return_query());
    run_return_query(
        &root,
        src_bytes,
        return_query,
        cfg_kinds,
        plugin,
        symbols,
        refs,
        &mut meta,
    );

    let reassignments = collect_reassignment_sites(&root, src_bytes, cfg);
    kill_narrowings_at_reassignments(&mut meta, &reassignments);

    // Per-function CFGs use the active plugin's explicitly supplied node
    // kinds. Missing ownership evidence fails closed.
    if let Some(kinds) = cfg_kinds {
        meta.cfg =
            crate::indexer::flow_cfg::build_file_cfg(&root, src_bytes, kinds, &meta.narrowings);
    }

    meta
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

use super::flow_assignments::run_assignment_query;

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
/// Gated on plugin-owned CFG node kinds: a language with no
/// `CfgNodeKinds` table has no function-boundary set to walk, so no return
/// query is honored for it. Populates `flow_return_lhs[ref_idx] =
/// fn_symbol_idx`. No-op when the language supplies no return query.
fn run_return_query(
    root: &Node,
    src: &[u8],
    query_src: Option<&'static str>,
    kinds: Option<&'static crate::indexer::flow_cfg::CfgNodeKinds>,
    plugin: Option<&dyn crate::languages::LanguagePlugin>,
    symbols: &[ExtractedSymbol],
    refs: &[ExtractedRef],
    meta: &mut FlowMeta,
) {
    // The ancestor-walk that resolves function identity needs the
    // function-boundary node-kind set. Without it no return query is honored.
    let (Some(query_src), Some(kinds)) = (query_src, kinds) else {
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
        attribute_return_expr(&expr, kinds, plugin, src, symbols, refs, meta);
    }

    run_tail_block_return(root, src, kinds, plugin, symbols, refs, meta);
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
    plugin: Option<&dyn crate::languages::LanguagePlugin>,
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
    if let Some(members) = plugin.and_then(|plugin| plugin.flow_return_object_members(*expr, src)) {
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
    if kinds.bare_return_name_kinds.contains(&expr.kind()) {
        if let Ok(ident) = expr.utf8_text(src) {
            if !ident.is_empty() {
                meta.flow_return_ident.push((fn_idx, ident.to_string()));
            }
        }
    }
}

/// Tail-of-block implicit return for expression-oriented grammars
/// (`implicit_return_candidate`): `fn f() -> T { …; e }` /
/// `def f = { …; e }` /
/// Expression-oriented function bodies return their final expression with no `return`
/// keyword. A tree-sitter query has no "last named child of a block" axis, so
/// this is a structural pass — the inverse of `nearest_function_ancestor`:
/// for every function node, find its body block and take the block's last
/// named child as a return expression.
///
/// Soundness (widening-only): statement-oriented languages provide no
/// predicate. Expression-oriented adapters decide which of their concrete
/// statement, binding, and explicit-return nodes are eligible.
/// Otherwise the child is the tail expression and is attributed exactly like a
/// `return` operand. The ref correlation only binds when the tail expression
/// actually contains a ref, so a tail that is a literal/identifier is a no-op.
fn run_tail_block_return(
    root: &Node,
    src: &[u8],
    kinds: &crate::indexer::flow_cfg::CfgNodeKinds,
    plugin: Option<&dyn crate::languages::LanguagePlugin>,
    symbols: &[ExtractedSymbol],
    refs: &[ExtractedRef],
    meta: &mut FlowMeta,
) {
    let Some(is_candidate) = kinds.implicit_return_candidate else {
        return;
    };
    for fn_node in descendant_function_nodes(root, kinds) {
        let Some(body) = crate::indexer::flow_cfg::find_function_body(&fn_node, kinds) else {
            continue;
        };
        let mut c = body.walk();
        let Some(last) = body.named_children(&mut c).last() else {
            continue;
        };
        if kinds.block_kinds.contains(&last.kind()) || !is_candidate(last) {
            continue;
        }
        attribute_return_expr(&last, kinds, plugin, src, symbols, refs, meta);
    }
}

/// Whether `node` (a block's last named child) is NOT a tail-return expression:
/// a statement, a binding declaration/definition, an explicit `return` already
/// caught by the `@return.expr` arm, or a nested block whose own tail is handled
/// by its enclosing function.
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

/// Find the source node that names a function. Both eligible fields and leaf
/// node kinds are declared by the owning language's CFG descriptor.
fn function_name_node<'a>(
    fn_node: &Node<'a>,
    kinds: &crate::indexer::flow_cfg::CfgNodeKinds,
) -> Option<Node<'a>> {
    for field in kinds.function_name_fields {
        if let Some(name) = binding_name_node(fn_node.child_by_field_name(field), kinds) {
            return Some(name);
        }
    }
    // Unnamed function: borrow the binding name from a wrapping declarator
    // (`const f = () => …`, `val f = { … }`). Stop at the next function
    // boundary — an arrow nested in another function is anonymous.
    let mut cur = fn_node.parent();
    while let Some(n) = cur {
        if kinds.function_kinds.contains(&n.kind()) {
            break;
        }
        for field in kinds.function_name_fields {
            if let Some(name) = binding_name_node(n.child_by_field_name(field), kinds) {
                return Some(name);
            }
        }
        cur = n.parent();
    }
    None
}

fn binding_name_node<'a>(
    node: Option<Node<'a>>,
    kinds: &crate::indexer::flow_cfg::CfgNodeKinds,
) -> Option<Node<'a>> {
    let node = node?;
    if kinds.binding_name_kinds.contains(&node.kind()) {
        return Some(node);
    }
    let mut cursor = node.walk();
    let found = node
        .named_children(&mut cursor)
        .find(|child| kinds.binding_name_kinds.contains(&child.kind()));
    found
}

fn run_type_guard_query(
    root: &Node,
    src: &[u8],
    cfg: &FlowConfig,
    plugin: Option<&dyn crate::languages::LanguagePlugin>,
    meta: &mut FlowMeta,
) {
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
            Ok(t) => plugin
                .and_then(|plugin| plugin.normalize_flow_guard_type(t))
                .unwrap_or_default(),
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

fn run_discriminant_guard_query(
    root: &Node,
    src: &[u8],
    cfg: &FlowConfig,
    plugin: Option<&dyn crate::languages::LanguagePlugin>,
    meta: &mut FlowMeta,
) {
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
    // `@guard.body` scopes a positive guard to its selected branch;
    // `@guard.early_exit` scopes a negated guard after the guarded exit. Either
    // may be absent depending on which captures a language's query supplies.
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
            let Some((start, end)) =
                plugin.and_then(|plugin| plugin.flow_discriminant_early_exit_scope(exit))
            else {
                continue;
            };
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
