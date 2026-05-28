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

use crate::indexer::resolve::engine::strip_generic_args;
use crate::types::{
    ChainSegment, DiscriminantNarrowing, ExtractedRef, ExtractedSymbol, FlowMeta, Narrowing,
};
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
    run_discriminant_guard_query(&root, src_bytes, cfg, &mut meta);
    run_type_args_query(&root, src_bytes, cfg, refs);

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
    let Ok(query) = Query::new(&root.language(), cfg.assignment_query) else {
        return Vec::new();
    };
    let Some(lhs_cap) = query.capture_index_for_name("lhs") else {
        return Vec::new();
    };

    let mut sites = Vec::new();
    let mut cursor = QueryCursor::new();
    let mut it = cursor.matches(&query, *root, src);
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

    let mut cursor = QueryCursor::new();
    let mut it = cursor.matches(&query, *root, src);
    while let Some(m) = it.next() {
        let mut lhs_node: Option<Node> = None;
        let mut rhs_node: Option<Node> = None;
        let mut type_node: Option<Node> = None;
        let mut unwrap_node: Option<Node> = None;
        for cap in m.captures {
            if cap.index == lhs_cap {
                lhs_node = Some(cap.node);
            } else if Some(cap.index) == rhs_cap {
                rhs_node = Some(cap.node);
            } else if Some(cap.index) == type_cap {
                type_node = Some(cap.node);
            } else if Some(cap.index) == unwrap_cap {
                unwrap_node = Some(cap.node);
            }
        }
        let Some(lhs) = lhs_node else { continue };
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

        // Explicit annotation: record the declared type directly. The base
        // type (generic args stripped) keys the member lookup, so `Vec<T>`
        // and `Vec` resolve the same members.
        if let Some(ty) = type_node {
            if let Ok(text) = ty.utf8_text(src) {
                let base = strip_generic_args(text.trim());
                if !base.is_empty() {
                    meta.flow_binding_decl_type.insert(lhs_idx, base);
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
                if is_unwrap {
                    meta.flow_binding_unwrap.insert(lhs_idx);
                }
            }
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

fn run_discriminant_guard_query(
    root: &Node,
    src: &[u8],
    cfg: &FlowConfig,
    meta: &mut FlowMeta,
) {
    if cfg.discriminant_guard_query.trim().is_empty() {
        return;
    }
    let Ok(query) = Query::new(&root.language(), cfg.discriminant_guard_query) else {
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
    let mut it = cursor.matches(&query, *root, src);
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
        byte_offset: 0,
            declared_type_id: None,
        is_call: false,
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
