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

use crate::types::{ChainSegment, ExtractedRef, ExtractedSymbol, FlowMeta, Narrowing};
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
    /// Tree-sitter query matching explicit call-site type arguments.
    /// Captures `@call.method` and one `@call.type_arg` per generic argument.
    pub type_args_query: &'static str,
}

/// Skip flow queries on files larger than this threshold. Huge files are
/// dominated by generated bindings (`node_modules/**/*.d.ts`, machine-generated
/// Go struct tables, etc.) where local-variable flow adds no value, and the
/// tree-sitter query matcher can spend unbounded memory on deeply nested
/// captures. 512 KiB catches every real hand-written source file in the
/// quality baseline.
const MAX_FLOW_SOURCE_BYTES: usize = 512 * 1024;

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
        let mut meta = FlowMeta::default();
        meta.ref_byte_offsets = refs.iter().map(|r| r.byte_offset).collect();
        return meta;
    }

    let mut parser = Parser::new();
    if parser.set_language(language).is_err() {
        return FlowMeta::default();
    }
    let Some(tree) = parser.parse(source, None) else {
        return FlowMeta::default();
    };
    let root = tree.root_node();
    let src_bytes = source.as_bytes();

    let mut meta = FlowMeta::default();
    meta.ref_byte_offsets = refs.iter().map(|r| r.byte_offset).collect();

    run_assignment_query(&root, src_bytes, cfg, symbols, refs, &mut meta);
    run_type_guard_query(&root, src_bytes, cfg, &mut meta);
    run_type_args_query(&root, src_bytes, cfg, refs);

    meta
}

fn run_assignment_query(
    root: &Node,
    src: &[u8],
    cfg: &FlowConfig,
    symbols: &[ExtractedSymbol],
    refs: &[ExtractedRef],
    meta: &mut FlowMeta,
) {
    let Ok(query) = Query::new(&root.language(), cfg.assignment_query) else {
        return;
    };
    let lhs_cap = query.capture_index_for_name("lhs");
    let rhs_cap = query.capture_index_for_name("rhs");
    let (Some(lhs_cap), Some(rhs_cap)) = (lhs_cap, rhs_cap) else {
        return;
    };

    let mut cursor = QueryCursor::new();
    let mut it = cursor.matches(&query, *root, src);
    while let Some(m) = it.next() {
        let mut lhs_node: Option<Node> = None;
        let mut rhs_node: Option<Node> = None;
        for cap in m.captures {
            if cap.index == lhs_cap {
                lhs_node = Some(cap.node);
            } else if cap.index == rhs_cap {
                rhs_node = Some(cap.node);
            }
        }
        let (Some(lhs), Some(rhs)) = (lhs_node, rhs_node) else {
            continue;
        };
        let lhs_name = match lhs.utf8_text(src) {
            Ok(t) => t,
            Err(_) => continue,
        };

        // Correlate LHS name → symbol_idx. For a local variable the extractor
        // emits a Variable symbol at the LHS start line; match by name and
        // closest start-line ≤ lhs row.
        let lhs_line = lhs.start_position().row as u32;
        let lhs_symbol_idx = symbols
            .iter()
            .enumerate()
            .filter(|(_, s)| s.name == lhs_name && s.start_line <= lhs_line)
            .max_by_key(|(_, s)| s.start_line)
            .map(|(i, _)| i);
        let Some(lhs_idx) = lhs_symbol_idx else {
            continue;
        };

        // Correlate RHS byte range → ref_idx. The ref whose byte_offset is
        // within [rhs.start_byte, rhs.end_byte) AND whose chain covers the
        // RHS's final segment wins. For a chain expression `foo.bar()`, the
        // refs emitter creates a Calls ref at the call node's position —
        // typically the start of the outer call. A looser heuristic: match
        // the ref with byte_offset in range AND latest (furthest-right)
        // start.
        let r_start = rhs.start_byte() as u32;
        let r_end = rhs.end_byte() as u32;
        let ref_idx = refs
            .iter()
            .enumerate()
            .filter(|(_, r)| r.byte_offset >= r_start && r.byte_offset < r_end)
            .max_by_key(|(_, r)| r.byte_offset)
            .map(|(i, _)| i);
        if let Some(ref_idx) = ref_idx {
            meta.flow_binding_lhs.insert(ref_idx, lhs_idx);
        }
    }
}

fn run_type_guard_query(
    root: &Node,
    src: &[u8],
    cfg: &FlowConfig,
    meta: &mut FlowMeta,
) {
    if cfg.type_guard_query.trim().is_empty() {
        return;
    }
    let Ok(query) = Query::new(&root.language(), cfg.type_guard_query) else {
        return;
    };
    let local_cap = query.capture_index_for_name("guard.local");
    let type_cap = query.capture_index_for_name("guard.type");
    let body_cap = query.capture_index_for_name("guard.body");
    let (Some(local_cap), Some(type_cap), Some(body_cap)) = (local_cap, type_cap, body_cap) else {
        return;
    };

    let mut cursor = QueryCursor::new();
    let mut it = cursor.matches(&query, *root, src);
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

fn run_type_args_query(
    root: &Node,
    src: &[u8],
    cfg: &FlowConfig,
    refs: &mut [ExtractedRef],
) {
    if cfg.type_args_query.trim().is_empty() {
        return;
    }
    let Ok(query) = Query::new(&root.language(), cfg.type_args_query) else {
        return;
    };
    let method_cap = query.capture_index_for_name("call.method");
    let arg_cap = query.capture_index_for_name("call.type_arg");
    let (Some(method_cap), Some(arg_cap)) = (method_cap, arg_cap) else {
        return;
    };

    let mut cursor = QueryCursor::new();
    let mut it = cursor.matches(&query, *root, src);
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
            let Some(chain) = r.chain.as_mut() else { continue };
            let Some(last) = chain.segments.last_mut() else { continue };
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
    let _ = ChainSegment { // silence unused import in release builds
        name: String::new(),
        node_kind: String::new(),
        kind: crate::types::SegmentKind::Identifier,
        declared_type: None,
        type_args: Vec::new(),
        optional_chaining: false,
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
