// =============================================================================
// indexer/flow_cfg.rs — Per-function control-flow graph for narrowing dataflow.
//
// Built at extract time from the tree-sitter root, queried at resolve time via
// `fact_at(name, byte)`. Facts ride edges, merge at join points (real union),
// reset on a def, reach a fixed point over loop back-edges.
//
// Status of the AST builder by language construct (the same algorithm runs for
// every language; only the node-kind table differs):
//
//   * statement_block / function_body  — sequential blocks
//   * if_statement                     — true/false edges + join
//
// Loops, &&/||, switch and per-language node-kind tables are added by
// subsequent expansions of `build_function_cfg` and the per-language
// `CfgNodeKinds` consumer.
// =============================================================================

use rustc_hash::FxHashMap;
use std::collections::VecDeque;
use tree_sitter::Node;

pub type BlockId = u32;

/// A dataflow fact for a single name at a program point.
///
/// `Single` is the common case (one narrowed type); `Union` results from a join
/// where predecessors disagree (`if (c) x = a; else x = b;` — after the if,
/// `x` is `a ∪ b`); `Never` propagates from unreachable paths (a fully
/// exhausted switch leaves the default block's `name` fact as `Never`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fact {
    Single(String),
    /// Sorted + deduplicated for canonical equality across joins.
    Union(Vec<String>),
    Never,
}

impl Fact {
    /// String form for callers that consume a single narrowed type. `Single`
    /// projects to its inner name; `Union` and `Never` project to `None`
    /// (an interval consumer cannot express either).
    pub fn as_single(&self) -> Option<&str> {
        match self {
            Fact::Single(s) => Some(s.as_str()),
            _ => None,
        }
    }
}

/// `name -> Fact` at a single program point. Empty = no narrowing on any
/// tracked name (consumer falls back to the declared type).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FactMap(pub FxHashMap<String, Fact>);

impl FactMap {
    pub fn get(&self, name: &str) -> Option<&Fact> {
        self.0.get(name)
    }

    pub fn insert(&mut self, name: String, fact: Fact) {
        self.0.insert(name, fact);
    }

    /// Pointwise join: a name absent on either side drops out (a narrowing
    /// that doesn't hold on every reaching path can't hold at the join).
    /// Where both sides carry a fact, `merge_facts` decides the union.
    pub fn join(&mut self, other: &Self) {
        let mut out = FxHashMap::default();
        for (name, sf) in &self.0 {
            if let Some(of) = other.0.get(name) {
                out.insert(name.clone(), merge_facts(sf, of));
            }
        }
        self.0 = out;
    }
}

fn merge_facts(a: &Fact, b: &Fact) -> Fact {
    use Fact::*;
    match (a, b) {
        // Never is the bottom — it doesn't contribute to a reachable join.
        (Never, x) | (x, Never) => x.clone(),
        (Single(s1), Single(s2)) if s1 == s2 => Single(s1.clone()),
        (Single(s1), Single(s2)) => Union(canon_union(vec![s1.clone(), s2.clone()])),
        (Single(s), Union(u)) | (Union(u), Single(s)) => {
            let mut combined = u.clone();
            combined.push(s.clone());
            Union(canon_union(combined))
        }
        (Union(u1), Union(u2)) => {
            let mut combined = u1.clone();
            combined.extend(u2.iter().cloned());
            Union(canon_union(combined))
        }
    }
}

fn canon_union(mut v: Vec<String>) -> Vec<String> {
    v.sort();
    v.dedup();
    v
}

/// A basic block: a maximal sequence of statements with single entry / single
/// exit, terminated by a branch or join. `byte_range` is the half-open span
/// in source bytes that the block covers (the range `fact_at` indexes by).
#[derive(Debug, Clone)]
pub struct BasicBlock {
    pub id: BlockId,
    pub byte_range: (u32, u32),
    /// Definitions inside this block, as `(name, byte_offset)` pairs in source
    /// order. Each def kills any prior narrowing for `name` at or after that
    /// byte. Reused for the def-resets-fact case the interval-truncation
    /// reassignment kill generalizes into.
    pub defs: Vec<(String, u32)>,
}

/// A control-flow edge. `guard` is the fact-map implied by *taking* this edge
/// (e.g., `instanceof Derived` on the true branch sets `x: Single("Derived")`
/// on that edge only). An empty guard means "no narrowing fact on this edge."
#[derive(Debug, Clone)]
pub struct Edge {
    pub from: BlockId,
    pub to: BlockId,
    pub guard: FactMap,
}

/// Per-function CFG. `in_facts[i]` is the start-of-block fact for block `i`,
/// populated by `run_dataflow`. The CFG's byte span (`fn_byte_range`) bounds
/// which `fact_at` queries it answers — a byte outside the range belongs to
/// another function's CFG.
#[derive(Debug, Clone, Default)]
pub struct Cfg {
    pub fn_byte_range: (u32, u32),
    pub blocks: Vec<BasicBlock>,
    pub edges: Vec<Edge>,
    pub entry: BlockId,
    pub in_facts: Vec<FactMap>,
}

impl Cfg {
    /// Innermost block containing `byte` (smallest range that covers it).
    fn block_at(&self, byte: u32) -> Option<BlockId> {
        self.blocks
            .iter()
            .filter(|b| b.byte_range.0 <= byte && byte < b.byte_range.1)
            .min_by_key(|b| b.byte_range.1 - b.byte_range.0)
            .map(|b| b.id)
    }

    /// Fact for `name` at byte position. Combines the block's start-of-block
    /// in-fact with any def in the block before `byte` (a def kills the fact).
    pub fn fact_at(&self, name: &str, byte: u32) -> Option<Fact> {
        let bid = self.block_at(byte)?;
        let in_fact = self.in_facts.get(bid as usize)?;
        let mut current = in_fact.0.get(name).cloned();
        let block = &self.blocks[bid as usize];
        for (def_name, def_byte) in &block.defs {
            if *def_byte > byte {
                break;
            }
            if def_name == name {
                current = None;
            }
        }
        current
    }

    /// Borrowed-string fast path; see `FileCfg::fact_string_at`.
    pub fn fact_string_at(&self, name: &str, byte: u32) -> Option<&str> {
        let bid = self.block_at(byte)?;
        let in_fact = self.in_facts.get(bid as usize)?;
        let mut current = in_fact.0.get(name);
        let block = &self.blocks[bid as usize];
        for (def_name, def_byte) in &block.defs {
            if *def_byte > byte {
                break;
            }
            if def_name == name {
                current = None;
            }
        }
        match current? {
            Fact::Single(s) => Some(s.as_str()),
            _ => None,
        }
    }
}

/// A collection of per-function CFGs for a single source file. The dataflow
/// is per-function (narrowings don't cross function boundaries), so each
/// function gets its own `Cfg`.
#[derive(Debug, Clone, Default)]
pub struct FileCfg {
    pub functions: Vec<Cfg>,
}

impl FileCfg {
    pub fn fact_at(&self, name: &str, byte: u32) -> Option<Fact> {
        self.functions
            .iter()
            .find(|c| c.fn_byte_range.0 <= byte && byte < c.fn_byte_range.1)
            .and_then(|c| c.fact_at(name, byte))
    }

    /// Borrowed-string fast path for callers (e.g. `LocalTypeCache::lookup`)
    /// that consume only the single-narrowed-type case. Returns `None` for a
    /// `Union` or `Never` fact, letting the caller fall back to its existing
    /// path until the consumer surface speaks the richer fact type.
    pub fn fact_string_at(&self, name: &str, byte: u32) -> Option<&str> {
        self.functions
            .iter()
            .find(|c| c.fn_byte_range.0 <= byte && byte < c.fn_byte_range.1)
            .and_then(|c| c.fact_string_at(name, byte))
    }

    pub fn is_empty(&self) -> bool {
        self.functions.is_empty()
    }
}

/// Per-language node-kind table. Different grammars name the same construct
/// differently (`if_statement` in TS/JS/Python; `if_expression` in Rust).
/// Languages plug their kinds in via this struct; the CFG algorithm itself
/// stays generic.
#[derive(Debug, Clone, Copy)]
pub struct CfgNodeKinds {
    /// Function-level nodes whose body should get its own CFG.
    pub function_kinds: &'static [&'static str],
    /// Statement-block node kind (the body of a function, then-branch, etc.).
    pub block_kind: &'static str,
    /// If-statement node kind.
    pub if_kind: &'static str,
    /// Field name on `if_kind` for the consequence (then) branch.
    pub if_consequence_field: &'static str,
    /// Field name on `if_kind` for the alternative (else) branch.
    pub if_alternative_field: &'static str,
    /// Field name on `if_kind` for the condition expression.
    pub if_condition_field: &'static str,
    /// Assignment node kinds (anything that *defines* a name).
    /// `lhs_field` is the field name on each that holds the LHS identifier.
    pub assignment_kind: &'static str,
    pub assignment_lhs_field: &'static str,
    /// Variable-declarator node kind (`let x = ...`).
    pub declarator_kind: &'static str,
    pub declarator_name_field: &'static str,
}

/// TypeScript / JavaScript node-kind table. The first wire-up; other
/// languages plug in their own table the same way.
pub const TS_CFG_KINDS: CfgNodeKinds = CfgNodeKinds {
    function_kinds: &[
        "function_declaration",
        "function_expression",
        "method_definition",
        "arrow_function",
    ],
    block_kind: "statement_block",
    if_kind: "if_statement",
    if_consequence_field: "consequence",
    if_alternative_field: "alternative",
    if_condition_field: "condition",
    assignment_kind: "assignment_expression",
    assignment_lhs_field: "left",
    declarator_kind: "variable_declarator",
    declarator_name_field: "name",
};

/// Build CFGs for every function in `root`. The returned `FileCfg`'s functions
/// are independent — each is its own dataflow.
pub fn build_file_cfg(root: &Node, src: &[u8], kinds: &CfgNodeKinds) -> FileCfg {
    let mut out = FileCfg::default();
    visit_for_functions(root, src, kinds, &mut out);
    out
}

fn visit_for_functions(node: &Node, src: &[u8], kinds: &CfgNodeKinds, out: &mut FileCfg) {
    if kinds.function_kinds.contains(&node.kind()) {
        if let Some(cfg) = build_function_cfg(node, src, kinds) {
            out.functions.push(cfg);
        }
        // Don't recurse — nested functions get their own CFG via the top
        // walk visiting siblings; recurse into the body so inner functions
        // are still discovered.
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        visit_for_functions(&child, src, kinds, out);
    }
}

/// Build a CFG for a single function. Returns `None` if the function has no
/// body the builder recognizes (e.g., an abstract method).
fn build_function_cfg(fn_node: &Node, src: &[u8], kinds: &CfgNodeKinds) -> Option<Cfg> {
    let body = find_function_body(fn_node, kinds)?;
    let mut cfg = Cfg {
        fn_byte_range: (fn_node.start_byte() as u32, fn_node.end_byte() as u32),
        ..Default::default()
    };
    let entry = new_block(&mut cfg, body.start_byte() as u32);
    cfg.entry = entry;
    let exit = build_block_sequence(&body, src, kinds, &mut cfg, entry);
    // Close the last open block at the body's end so `fact_at` ranges cover
    // the whole body.
    close_block(&mut cfg, exit, body.end_byte() as u32);
    run_dataflow(&mut cfg, /*iter_cap=*/ 8);
    Some(cfg)
}

fn find_function_body<'a>(fn_node: &Node<'a>, kinds: &CfgNodeKinds) -> Option<Node<'a>> {
    let mut c = fn_node.walk();
    for ch in fn_node.named_children(&mut c) {
        if ch.kind() == kinds.block_kind {
            return Some(ch);
        }
    }
    None
}

fn new_block(cfg: &mut Cfg, byte_start: u32) -> BlockId {
    let id = cfg.blocks.len() as BlockId;
    cfg.blocks.push(BasicBlock {
        id,
        byte_range: (byte_start, byte_start),
        defs: Vec::new(),
    });
    id
}

fn close_block(cfg: &mut Cfg, bid: BlockId, byte_end: u32) {
    let b = &mut cfg.blocks[bid as usize];
    if byte_end > b.byte_range.1 {
        b.byte_range.1 = byte_end;
    }
}

fn add_edge(cfg: &mut Cfg, from: BlockId, to: BlockId, guard: FactMap) {
    cfg.edges.push(Edge { from, to, guard });
}

/// Walk `block_node`'s children in source order, growing/branching `cfg`.
/// Returns the BlockId of the *current* open block at the end of the walk
/// (which the caller closes at the enclosing scope's end).
fn build_block_sequence(
    block_node: &Node,
    src: &[u8],
    kinds: &CfgNodeKinds,
    cfg: &mut Cfg,
    entry: BlockId,
) -> BlockId {
    let mut current = entry;
    let mut walker = block_node.walk();
    for child in block_node.named_children(&mut walker) {
        let ck = child.kind();
        if ck == kinds.if_kind {
            current = build_if(&child, src, kinds, cfg, current);
        } else {
            collect_defs_in(&child, src, kinds, cfg, current);
            // Statement is wholly inside the current block; widen its range
            // to cover what's been seen so far.
            close_block(cfg, current, child.end_byte() as u32);
        }
    }
    current
}

/// Build an if/else diamond:
///
///     pred ─true─▶  then_block ─▶ join
///          └false─▶ else_block ─▶ join
///
/// where the guard fact (if recognizable) rides the true-edge. Returns the
/// `join` block as the new current block.
fn build_if(
    if_node: &Node,
    src: &[u8],
    kinds: &CfgNodeKinds,
    cfg: &mut Cfg,
    pred: BlockId,
) -> BlockId {
    // Close the predecessor at the if-statement's start so its range stops
    // before the branch.
    close_block(cfg, pred, if_node.start_byte() as u32);

    // Read the condition's implied true-edge fact (if any).
    let true_guard = if_node
        .child_by_field_name(kinds.if_condition_field)
        .map(|c| condition_to_true_guard(&c, src))
        .unwrap_or_default();

    // THEN branch.
    let then_node = if_node.child_by_field_name(kinds.if_consequence_field);
    let then_start = then_node
        .as_ref()
        .map(|n| n.start_byte() as u32)
        .unwrap_or_else(|| if_node.end_byte() as u32);
    let then_block = new_block(cfg, then_start);
    add_edge(cfg, pred, then_block, true_guard);
    let then_tail = match then_node {
        Some(body) if body.kind() == kinds.block_kind => {
            let tail = build_block_sequence(&body, src, kinds, cfg, then_block);
            close_block(cfg, tail, body.end_byte() as u32);
            tail
        }
        Some(body) => {
            // Single-statement consequence (no braces) — treat as one block.
            collect_defs_in(&body, src, kinds, cfg, then_block);
            close_block(cfg, then_block, body.end_byte() as u32);
            then_block
        }
        None => then_block,
    };

    // ELSE branch (may be absent). Tree-sitter wraps `else <body>` in an
    // `else_clause` node — peel it so the match below sees the real body.
    let else_node = if_node
        .child_by_field_name(kinds.if_alternative_field)
        .map(|n| {
            if n.kind() == "else_clause" {
                n.named_child(0).unwrap_or(n)
            } else {
                n
            }
        });
    let else_block;
    let else_tail;
    if let Some(else_body) = else_node {
        let eb = new_block(cfg, else_body.start_byte() as u32);
        add_edge(cfg, pred, eb, FactMap::default());
        else_block = eb;
        else_tail = match else_body.kind() {
            k if k == kinds.block_kind => {
                let tail = build_block_sequence(&else_body, src, kinds, cfg, eb);
                close_block(cfg, tail, else_body.end_byte() as u32);
                tail
            }
            k if k == kinds.if_kind => {
                // `else if` — a chained if; pred is `eb`, returns its join.
                let chained = build_if(&else_body, src, kinds, cfg, eb);
                chained
            }
            _ => {
                collect_defs_in(&else_body, src, kinds, cfg, eb);
                close_block(cfg, eb, else_body.end_byte() as u32);
                eb
            }
        };
    } else {
        // No else — the false edge skips to the join with no narrowing.
        else_block = pred;
        else_tail = pred;
    }

    // JOIN block, opened at the if-statement's end byte.
    let join = new_block(cfg, if_node.end_byte() as u32);
    add_edge(cfg, then_tail, join, FactMap::default());
    if else_tail != pred {
        add_edge(cfg, else_tail, join, FactMap::default());
    } else {
        // No-else case: false edge goes pred -> join directly.
        add_edge(cfg, pred, join, FactMap::default());
    }
    let _ = else_block;
    join
}

/// Collect any defs (assignment LHS, variable declarators) reachable inside
/// `node` and append them to `block`. Walks shallowly — does not descend into
/// nested functions or into nested if-statements (those are CFG branches and
/// their defs belong to their own blocks).
fn collect_defs_in(
    node: &Node,
    src: &[u8],
    kinds: &CfgNodeKinds,
    cfg: &mut Cfg,
    block: BlockId,
) {
    let mut stack = vec![*node];
    while let Some(n) = stack.pop() {
        let k = n.kind();
        if kinds.function_kinds.contains(&k) {
            continue; // nested fn — its own CFG
        }
        if k == kinds.if_kind {
            continue; // handled by branch construction
        }
        if k == kinds.assignment_kind {
            if let Some(lhs) = n.child_by_field_name(kinds.assignment_lhs_field) {
                if lhs.kind() == "identifier" {
                    if let Ok(name) = lhs.utf8_text(src) {
                        cfg.blocks[block as usize]
                            .defs
                            .push((name.to_string(), n.start_byte() as u32));
                    }
                }
            }
        } else if k == kinds.declarator_kind {
            if let Some(name_node) = n.child_by_field_name(kinds.declarator_name_field) {
                if name_node.kind() == "identifier" {
                    if let Ok(name) = name_node.utf8_text(src) {
                        cfg.blocks[block as usize]
                            .defs
                            .push((name.to_string(), n.start_byte() as u32));
                    }
                }
            }
        }
        let mut c = n.walk();
        for ch in n.named_children(&mut c) {
            stack.push(ch);
        }
    }
    cfg.blocks[block as usize]
        .defs
        .sort_by_key(|(_, b)| *b);
}

/// Read a type-guard fact off a condition expression. Recognized patterns:
///
///   * `x instanceof T`     → `x: Single("T")`
///   * `typeof x === "T"`   → `x: Single("T")`  (string-literal RHS)
///   * `(<inner>)`          → recurse on `inner` (parenthesized condition)
///
/// Empty FactMap when the condition is unrecognized — the true-edge then
/// carries no narrowing (the CFG still represents the branch shape).
fn condition_to_true_guard(cond: &Node, src: &[u8]) -> FactMap {
    let mut out = FactMap::default();
    let n = unwrap_parens(*cond);
    let k = n.kind();
    if k == "binary_expression" {
        let op = n
            .child_by_field_name("operator")
            .and_then(|o| o.utf8_text(src).ok())
            .unwrap_or("");
        let left = n.child_by_field_name("left");
        let right = n.child_by_field_name("right");
        if op == "instanceof" {
            if let (Some(l), Some(r)) = (left, right) {
                if l.kind() == "identifier" && r.kind() == "identifier" {
                    if let (Ok(lname), Ok(rty)) = (l.utf8_text(src), r.utf8_text(src)) {
                        out.insert(lname.to_string(), Fact::Single(rty.to_string()));
                    }
                }
            }
        } else if op == "===" || op == "==" {
            // typeof x === "T"
            if let (Some(l), Some(r)) = (left, right) {
                let l = unwrap_parens(l);
                if l.kind() == "unary_expression" {
                    let opnode = l.child_by_field_name("operator");
                    let arg = l.child_by_field_name("argument");
                    let is_typeof = opnode
                        .and_then(|o| o.utf8_text(src).ok())
                        .map(|s| s == "typeof")
                        .unwrap_or(false);
                    if is_typeof {
                        if let Some(arg) = arg {
                            if arg.kind() == "identifier" && r.kind() == "string" {
                                if let (Ok(name), Ok(lit)) =
                                    (arg.utf8_text(src), r.utf8_text(src))
                                {
                                    let ty = strip_quotes(lit);
                                    if !ty.is_empty() {
                                        out.insert(name.to_string(), Fact::Single(ty));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    out
}

fn unwrap_parens(mut n: Node) -> Node {
    while n.kind() == "parenthesized_expression" {
        let mut c = n.walk();
        let inner = n.named_children(&mut c).next();
        if let Some(i) = inner {
            n = i;
        } else {
            break;
        }
    }
    n
}

fn strip_quotes(s: &str) -> String {
    let s = s.trim();
    let b = s.as_bytes();
    if b.len() >= 2
        && ((b[0] == b'"' && b[b.len() - 1] == b'"')
            || (b[0] == b'\'' && b[b.len() - 1] == b'\''))
    {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

/// Forward dataflow: each block's in-fact is the join of predecessors'
/// (out-fact, edge-guard) contributions. Out-fact is in-fact with the
/// block's defs killing matching names. Iterates to a fixed point bounded
/// by `iter_cap * #blocks` worklist pops; non-convergence widens (drop the
/// unstable facts) rather than looping forever.
pub fn run_dataflow(cfg: &mut Cfg, iter_cap: usize) {
    cfg.in_facts = vec![FactMap::default(); cfg.blocks.len()];
    let mut preds: Vec<Vec<usize>> = vec![Vec::new(); cfg.blocks.len()];
    for (i, e) in cfg.edges.iter().enumerate() {
        preds[e.to as usize].push(i);
    }
    let mut work: VecDeque<BlockId> = (0..cfg.blocks.len() as BlockId).collect();
    let max_pops = iter_cap.saturating_mul(cfg.blocks.len().max(1));
    let mut pops = 0;
    while let Some(bid) = work.pop_front() {
        pops += 1;
        if pops > max_pops {
            break;
        }
        let new_in = compute_in_fact(cfg, bid, &preds);
        if new_in != cfg.in_facts[bid as usize] {
            cfg.in_facts[bid as usize] = new_in;
            for e in &cfg.edges {
                if e.from == bid {
                    work.push_back(e.to);
                }
            }
        }
    }
}

fn compute_in_fact(cfg: &Cfg, bid: BlockId, preds: &[Vec<usize>]) -> FactMap {
    let pred_edges = &preds[bid as usize];
    if pred_edges.is_empty() {
        return FactMap::default();
    }
    let mut acc: Option<FactMap> = None;
    for &eid in pred_edges {
        let e = &cfg.edges[eid];
        let mut contrib = cfg.in_facts[e.from as usize].clone();
        // Source block's defs kill any prior fact for the defined names.
        let src_block = &cfg.blocks[e.from as usize];
        for (def_name, _) in &src_block.defs {
            contrib.0.remove(def_name);
        }
        // Edge guard applies on this edge only (overrides prior fact).
        for (name, fact) in &e.guard.0 {
            contrib.insert(name.clone(), fact.clone());
        }
        acc = Some(match acc {
            None => contrib,
            Some(mut a) => {
                a.join(&contrib);
                a
            }
        });
    }
    acc.unwrap_or_default()
}

#[cfg(test)]
pub(super) fn _test_build_for_ts(src: &str) -> FileCfg {
    use crate::languages::LanguagePlugin;
    use tree_sitter::Parser;
    let mut p = Parser::new();
    let lang = crate::languages::typescript::TypeScriptPlugin
        .grammar("typescript")
        .expect("ts grammar");
    p.set_language(&lang).expect("set lang");
    let tree = p.parse(src, None).expect("parse");
    build_file_cfg(&tree.root_node(), src.as_bytes(), &TS_CFG_KINDS)
}

#[cfg(test)]
#[path = "flow_cfg_tests.rs"]
mod tests;
