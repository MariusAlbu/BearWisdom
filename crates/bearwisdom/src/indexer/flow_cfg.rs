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
/// (for example, a language-provided type test may set `x: Single("Derived")`
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

    /// Multi-branch projection of the fact at `byte`: `Single(s)` projects to
    /// `[s]`, `Union(branches)` to its branches verbatim, `Never` to `None`
    /// (no callable type at an unreachable program point). Callers that can
    /// dispatch across union members use this instead of `fact_string_at`.
    pub fn fact_union_at(&self, name: &str, byte: u32) -> Option<Vec<String>> {
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
            Fact::Single(s) => Some(vec![s.clone()]),
            Fact::Union(b) => Some(b.clone()),
            Fact::Never => None,
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
    /// `Union` or `Never` fact; callers that can dispatch across union
    /// members consult `fact_union_at` instead.
    pub fn fact_string_at(&self, name: &str, byte: u32) -> Option<&str> {
        self.functions
            .iter()
            .find(|c| c.fn_byte_range.0 <= byte && byte < c.fn_byte_range.1)
            .and_then(|c| c.fact_string_at(name, byte))
    }

    /// Multi-branch projection; see `Cfg::fact_union_at`.
    pub fn fact_union_at(&self, name: &str, byte: u32) -> Option<Vec<String>> {
        self.functions
            .iter()
            .find(|c| c.fn_byte_range.0 <= byte && byte < c.fn_byte_range.1)
            .and_then(|c| c.fact_union_at(name, byte))
    }

    pub fn is_empty(&self) -> bool {
        self.functions.is_empty()
    }
}

/// Per-language node-kind table. Different grammars name the same construct
/// differently across grammars.
/// Languages plug their kinds in via this struct; the CFG algorithm itself
/// stays generic.
#[derive(Debug, Clone, Copy)]
pub struct CfgNodeKinds {
    /// Function-level nodes whose body should get its own CFG.
    pub function_kinds: &'static [&'static str],
    /// Statement-block node kinds (the body of a function, then-branch, etc.).
    /// A slice because some grammars use different node kinds for different
    /// block contexts. A language may use distinct body nodes for conditional and
    /// then-branch is `then` and the while body is `do`; R uses
    /// `braced_expression` instead of `block`. The first kind in the slice
    /// is treated as the canonical block kind in error / fallback paths;
    /// otherwise the slice is a contains-set.
    pub block_kinds: &'static [&'static str],
    /// If-statement node kind.
    pub if_kind: &'static str,
    /// Field name on `if_kind` for the consequence (then) branch.
    pub if_consequence_field: &'static str,
    /// Select the consequence body when the grammar exposes no named field.
    pub if_consequence_body: Option<for<'a> fn(Node<'a>) -> Option<Node<'a>>>,
    /// Field name on `if_kind` for the alternative (else) branch.
    pub if_alternative_field: &'static str,
    /// Select the executable alternative body from the grammar's alternative
    /// node. `None` means the alternative node is already the body.
    pub if_alternative_body: Option<for<'a> fn(Node<'a>) -> Option<Node<'a>>>,
    /// Field name on `if_kind` for the condition expression.
    pub if_condition_field: &'static str,
    /// Assignment node kinds (anything that *defines* a name).
    /// `lhs_field` is the field name on each that holds the LHS identifier.
    pub assignment_kind: &'static str,
    pub assignment_lhs_field: &'static str,
    /// Variable-declarator node kind (`let x = ...`).
    pub declarator_kind: &'static str,
    pub declarator_name_field: &'static str,
    /// Node kinds whose source text is a single function/declaration name.
    pub binding_name_kinds: &'static [&'static str],
    /// Name kinds admitted as CFG definition kills.
    pub definition_name_kinds: &'static [&'static str],
    /// Name kinds admitted as a bare return expression.
    pub bare_return_name_kinds: &'static [&'static str],
    /// Fields that may carry the source name of a function or of the
    /// declaration that immediately owns an anonymous function expression.
    pub function_name_fields: &'static [&'static str],
    /// Loop node kinds. The CFG models each as
    /// `pred → header → body → header (back-edge)` and `header → exit`.
    pub loop_kinds: &'static [&'static str],
    /// Field on a loop node holding the body statement.
    pub loop_body_field: &'static str,
    /// Field on a loop node holding the loop condition, when one exists.
    /// `None` denotes an iterator-style loop without a boolean test.
    pub loop_condition_field: Option<&'static str>,
    /// Switch-statement node kinds (some grammars split expression-switch /
    /// type-switch / pattern-match into separate kinds.
    /// `expression_switch_statement` and `type_switch_statement` as siblings).
    /// An empty slice disables the switch path for that language.
    pub switch_kinds: &'static [&'static str],
    /// Field on the switch holding the scrutinee value.
    pub switch_value_field: &'static str,
    /// Field on the switch holding the body (the wrapper of case clauses).
    /// `None` for grammars whose switch lists cases as direct children of
    /// the switch node.
    pub switch_body_field: Option<&'static str>,
    /// Case-clause node kinds inside the switch body. Multiple entries for
    /// languages whose switch flavors emit distinct case kinds.
    /// `expression_case` + `type_case`).
    pub switch_case_kinds: &'static [&'static str],
    /// Default-clause node kinds inside the switch body. Empty when the
    /// language has no separate default node (for example, wildcard patterns
    /// are subsumed by the case kind itself).
    pub switch_default_kinds: &'static [&'static str],
    /// Pass-through container kinds. When iterating a block's or case's
    /// statements, a child whose kind matches one of these is treated as
    /// transparent — the walker recurses into its named_children instead
    /// of dispatching it as a single statement. Some grammars wrap top-level statements
    /// in `statement_list`; other grammars may have analogous wrappers.
    pub transparent_kinds: &'static [&'static str],
    /// Whether the grammar treats a block body's final *expression* as the
    /// block's (and thus the function's) implicit return value. True for the
    /// expression-oriented languages where `fn f() -> T { e }` / `def f = { e }`
    /// returns `e` with no `return` keyword; false where a
    /// bare trailing expression is a statement that returns no value.
    /// Drives the tail-of-block return-inference pass: the last
    /// named child of the body block is taken as a return expression, excluding
    /// `*_statement` (semicolon-terminated, returns unit) and binding nodes
    /// (`*_declaration` / `*_definition`).
    /// Language-owned admission for a block's final named child as an implicit
    /// return. `None` disables tail returns for statement-oriented grammars.
    pub implicit_return_candidate: Option<fn(Node) -> bool>,
    /// Optional language-owned parser for extra condition facts not already
    /// emitted by the configured guard query.
    pub condition_true_guard: Option<fn(Node, &[u8]) -> FactMap>,
}

/// Build CFGs for every function in `root`. The returned `FileCfg`'s functions
/// are independent — each is its own dataflow.
///
/// `narrowings` are the per-language `type_guard_query` results from the
/// `flow` runner. They feed into edge guards on body-containing edges
/// (if-then, loop body, switch case) — any `Narrowing` whose byte range
/// fits inside a body becomes a `Single`-fact edge guard. This is the
/// generic, language-agnostic source of narrowing facts. A language adapter may
/// also contribute normalized facts through `condition_true_guard`.
pub fn build_file_cfg(
    root: &Node,
    src: &[u8],
    kinds: &CfgNodeKinds,
    narrowings: &[crate::types::Narrowing],
) -> FileCfg {
    let mut out = FileCfg::default();
    visit_for_functions(root, src, kinds, narrowings, &mut out);
    out
}

fn visit_for_functions(
    node: &Node,
    src: &[u8],
    kinds: &CfgNodeKinds,
    narrowings: &[crate::types::Narrowing],
    out: &mut FileCfg,
) {
    if kinds.function_kinds.contains(&node.kind()) {
        if let Some(cfg) = build_function_cfg(node, src, kinds, narrowings) {
            out.functions.push(cfg);
        }
        // Don't recurse — nested functions get their own CFG via the top
        // walk visiting siblings; recurse into the body so inner functions
        // are still discovered.
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        visit_for_functions(&child, src, kinds, narrowings, out);
    }
}

/// Build a CFG for a single function. Returns `None` if the function has no
/// body the builder recognizes (e.g., an abstract method).
fn build_function_cfg(
    fn_node: &Node,
    src: &[u8],
    kinds: &CfgNodeKinds,
    narrowings: &[crate::types::Narrowing],
) -> Option<Cfg> {
    let body = find_function_body(fn_node, kinds)?;
    let mut cfg = Cfg {
        fn_byte_range: (fn_node.start_byte() as u32, fn_node.end_byte() as u32),
        ..Default::default()
    };
    let entry = new_block(&mut cfg, body.start_byte() as u32);
    cfg.entry = entry;
    let exit = build_block_sequence(&body, src, kinds, narrowings, &mut cfg, entry);
    // Close the last open block at the body's end so `fact_at` ranges cover
    // the whole body.
    close_block(&mut cfg, exit, body.end_byte() as u32);
    run_dataflow(&mut cfg, /*iter_cap=*/ 8);
    Some(cfg)
}

/// Collect any `Narrowing` whose byte range is contained in `(start, end)`
/// into a `FactMap` of `Single` facts. The CFG attaches these to edges
/// whose target block is the body the narrowing was emitted for.
fn guards_for_range(narrowings: &[crate::types::Narrowing], start: u32, end: u32) -> FactMap {
    let mut out = FactMap::default();
    for n in narrowings {
        if n.byte_start >= start && n.byte_end <= end && !n.name.is_empty() {
            // Innermost narrowing wins on ties — the input is pre-sorted
            // innermost-first by the resolver before reaching the CFG via
            // `LocalTypeCache::install_local_cache`, so a later insert here
            // would be the OUTER narrowing; skip when already present.
            if !out.0.contains_key(&n.name) {
                out.insert(n.name.clone(), Fact::Single(n.narrowed_type.clone()));
            }
        }
    }
    out
}

pub(crate) fn find_function_body<'a>(fn_node: &Node<'a>, kinds: &CfgNodeKinds) -> Option<Node<'a>> {
    // Direct match first — the common case (`function_declaration > block`).
    let mut c = fn_node.walk();
    for ch in fn_node.named_children(&mut c) {
        if kinds.block_kinds.contains(&ch.kind()) {
            return Some(ch);
        }
    }
    // Descend one level through a transparent wrapper — a language's
    // `function_declaration > function_body > block`, where `function_body`
    // is registered in `transparent_kinds`.
    let mut c = fn_node.walk();
    for ch in fn_node.named_children(&mut c) {
        if !kinds.transparent_kinds.contains(&ch.kind()) {
            continue;
        }
        let mut cc = ch.walk();
        for gc in ch.named_children(&mut cc) {
            if kinds.block_kinds.contains(&gc.kind()) {
                return Some(gc);
            }
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
///
/// Children whose kind is in `kinds.transparent_kinds` (a transparent statement wrapper)
/// are flattened — the walker recurses into their named_children so the real
/// if/loop/switch statements are visible to the dispatcher.
fn build_block_sequence(
    block_node: &Node,
    src: &[u8],
    kinds: &CfgNodeKinds,
    narrowings: &[crate::types::Narrowing],
    cfg: &mut Cfg,
    entry: BlockId,
) -> BlockId {
    let mut current = entry;
    let mut walker = block_node.walk();
    for child in block_node.named_children(&mut walker) {
        if kinds.transparent_kinds.contains(&child.kind()) {
            current = build_block_sequence(&child, src, kinds, narrowings, cfg, current);
            continue;
        }
        current = process_block_child(&child, src, kinds, narrowings, cfg, current);
    }
    current
}

/// Dispatch one statement-level child within a block / case. Extracted so
/// the transparent-wrapper case in `build_block_sequence` and the case-clause
/// walker in `build_switch` share one rule for what counts as if / loop /
/// switch vs. an opaque statement.
fn process_block_child(
    child: &Node,
    src: &[u8],
    kinds: &CfgNodeKinds,
    narrowings: &[crate::types::Narrowing],
    cfg: &mut Cfg,
    current: BlockId,
) -> BlockId {
    let ck = child.kind();
    if ck == kinds.if_kind {
        build_if(child, src, kinds, narrowings, cfg, current)
    } else if kinds.loop_kinds.contains(&ck) {
        build_loop(child, src, kinds, narrowings, cfg, current)
    } else if kinds.switch_kinds.contains(&ck) {
        build_switch(child, src, kinds, narrowings, cfg, current)
    } else {
        collect_defs_in(child, src, kinds, cfg, current);
        // Statement is wholly inside the current block; widen its range
        // to cover what's been seen so far.
        close_block(cfg, current, child.end_byte() as u32);
        current
    }
}

/// Build an if/else diamond:
///
///     pred ─true─▶  then_block ─▶ join
///          └false─▶ else_block ─▶ join
///
/// where the guard fact (if recognizable) rides the true-edge. Returns the
/// `join` block as the new current block.
fn if_consequence_node<'a>(if_node: &Node<'a>, kinds: &CfgNodeKinds) -> Option<Node<'a>> {
    if let Some(select) = kinds.if_consequence_body {
        return select(*if_node);
    }
    (!kinds.if_consequence_field.is_empty())
        .then(|| if_node.child_by_field_name(kinds.if_consequence_field))
        .flatten()
}

fn build_if(
    if_node: &Node,
    src: &[u8],
    kinds: &CfgNodeKinds,
    narrowings: &[crate::types::Narrowing],
    cfg: &mut Cfg,
    pred: BlockId,
) -> BlockId {
    // Close the predecessor at the if-statement's start so its range stops
    // before the branch.
    close_block(cfg, pred, if_node.start_byte() as u32);

    // Read the condition's implied true-edge fact. Two sources combined:
    //   * the per-language `type_guard_query` results (`narrowings`) whose
    //     byte range fits inside the then-body — generic, all languages.
    //   * an optional language-owned syntactic recognizer for patterns the
    //     query may not capture.
    let then_range = if_consequence_node(if_node, kinds)
        .map(|n| (n.start_byte() as u32, n.end_byte() as u32))
        .unwrap_or((if_node.end_byte() as u32, if_node.end_byte() as u32));
    let mut true_guard = guards_for_range(narrowings, then_range.0, then_range.1);
    if let Some(c) = if_node.child_by_field_name(kinds.if_condition_field) {
        let syntactic = kinds
            .condition_true_guard
            .map(|guard| guard(c, src))
            .unwrap_or_default();
        for (name, fact) in &syntactic.0 {
            true_guard.insert(name.clone(), fact.clone());
        }
    }

    // THEN branch.
    let then_node = if_consequence_node(if_node, kinds);
    let then_start = then_node
        .as_ref()
        .map(|n| n.start_byte() as u32)
        .unwrap_or_else(|| if_node.end_byte() as u32);
    let then_block = new_block(cfg, then_start);
    add_edge(cfg, pred, then_block, true_guard);
    let then_tail = match then_node {
        Some(body) if kinds.block_kinds.contains(&body.kind()) => {
            let tail = build_block_sequence(&body, src, kinds, narrowings, cfg, then_block);
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

    let else_node = if_node
        .child_by_field_name(kinds.if_alternative_field)
        .and_then(|node| match kinds.if_alternative_body {
            Some(select) => select(node),
            None => Some(node),
        });
    let else_block;
    let else_tail;
    if let Some(else_body) = else_node {
        let eb = new_block(cfg, else_body.start_byte() as u32);
        add_edge(cfg, pred, eb, FactMap::default());
        else_block = eb;
        else_tail = match else_body.kind() {
            k if kinds.block_kinds.contains(&k) => {
                let tail = build_block_sequence(&else_body, src, kinds, narrowings, cfg, eb);
                close_block(cfg, tail, else_body.end_byte() as u32);
                tail
            }
            k if k == kinds.if_kind => {
                // `else if` — a chained if; pred is `eb`, returns its join.
                let chained = build_if(&else_body, src, kinds, narrowings, cfg, eb);
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

/// Build a loop diamond:
///
///     pred → header ─true─▶ body ──▶ header (back-edge)
///                  └false─▶ exit
///
/// The worklist iterates to a fixed point: if the body defines a name, the
/// back-edge invalidates any prior narrowing for that name at the header
/// (and hence inside the body, since the body's in-fact joins through the
/// header). The visited-pred skip in `compute_in_fact` ensures the first
/// pass propagates pred → header → body before the back-edge fires, so an
/// untouched name's narrowing survives the loop. Returns `exit` as the new
/// current block.
fn build_loop(
    loop_node: &Node,
    src: &[u8],
    kinds: &CfgNodeKinds,
    narrowings: &[crate::types::Narrowing],
    cfg: &mut Cfg,
    pred: BlockId,
) -> BlockId {
    close_block(cfg, pred, loop_node.start_byte() as u32);

    let header = new_block(cfg, loop_node.start_byte() as u32);
    add_edge(cfg, pred, header, FactMap::default());

    let body_node = loop_node.child_by_field_name(kinds.loop_body_field);
    let body_range = body_node
        .as_ref()
        .map(|n| (n.start_byte() as u32, n.end_byte() as u32))
        .unwrap_or((loop_node.end_byte() as u32, loop_node.end_byte() as u32));
    let mut true_guard = guards_for_range(narrowings, body_range.0, body_range.1);
    if let Some(c) = kinds
        .loop_condition_field
        .and_then(|f| loop_node.child_by_field_name(f))
    {
        let syntactic = kinds
            .condition_true_guard
            .map(|guard| guard(c, src))
            .unwrap_or_default();
        for (name, fact) in &syntactic.0 {
            true_guard.insert(name.clone(), fact.clone());
        }
    }

    let body_block = new_block(cfg, body_range.0);
    add_edge(cfg, header, body_block, true_guard);
    let body_tail = match body_node {
        Some(body) if kinds.block_kinds.contains(&body.kind()) => {
            let tail = build_block_sequence(&body, src, kinds, narrowings, cfg, body_block);
            close_block(cfg, tail, body.end_byte() as u32);
            tail
        }
        Some(body) => {
            collect_defs_in(&body, src, kinds, cfg, body_block);
            close_block(cfg, body_block, body.end_byte() as u32);
            body_block
        }
        None => body_block,
    };
    add_edge(cfg, body_tail, header, FactMap::default());

    let exit = new_block(cfg, loop_node.end_byte() as u32);
    add_edge(cfg, header, exit, FactMap::default());
    exit
}

/// Build a switch diamond:
///
///     pred → scrutinee → case_1 ─┐
///                      → case_2 ─┤
///                      → default ┴→ exit
///
/// Cases are disjoint blocks — fall-through across cases is not modeled.
/// Per-case discriminant guards are not yet attached to edges (the existing
/// `discriminant_guard_query` produces those on the interval path); the CFG
/// structure here is what the exhaustiveness-aware enrichment plugs into.
fn build_switch(
    switch_node: &Node,
    src: &[u8],
    kinds: &CfgNodeKinds,
    narrowings: &[crate::types::Narrowing],
    cfg: &mut Cfg,
    pred: BlockId,
) -> BlockId {
    close_block(cfg, pred, switch_node.start_byte() as u32);

    let scrutinee = new_block(cfg, switch_node.start_byte() as u32);
    add_edge(cfg, pred, scrutinee, FactMap::default());

    let exit = new_block(cfg, switch_node.end_byte() as u32);

    // Some grammars wrap cases in a `body` node (some grammars),
    // others list them as direct children of the switch. Pick the right
    // iteration root based on the language's `switch_body_field`.
    let case_root = match kinds.switch_body_field {
        Some(f) => switch_node.child_by_field_name(f),
        None => Some(*switch_node),
    };
    if let Some(body) = case_root {
        let mut walker = body.walk();
        for clause in body.named_children(&mut walker) {
            let ck = clause.kind();
            if kinds.switch_case_kinds.contains(&ck) || kinds.switch_default_kinds.contains(&ck) {
                let case_block = new_block(cfg, clause.start_byte() as u32);
                let case_guard = guards_for_range(
                    narrowings,
                    clause.start_byte() as u32,
                    clause.end_byte() as u32,
                );
                add_edge(cfg, scrutinee, case_block, case_guard);
                let tail = walk_case_statements(&clause, src, kinds, narrowings, cfg, case_block);
                close_block(cfg, tail, clause.end_byte() as u32);
                add_edge(cfg, tail, exit, FactMap::default());
            }
        }
    }
    if !cfg.edges.iter().any(|e| e.to == exit) {
        add_edge(cfg, scrutinee, exit, FactMap::default());
    }
    exit
}

/// Walk the statements inside a switch case clause. Same dispatch rule as
/// `build_block_sequence` (if/loop/switch routed to their builders, other
/// nodes treated as opaque defs) but rooted at the clause node, recursing
/// through any `transparent_kinds` wrapper (a transparent statement wrapper).
fn walk_case_statements(
    clause: &Node,
    src: &[u8],
    kinds: &CfgNodeKinds,
    narrowings: &[crate::types::Narrowing],
    cfg: &mut Cfg,
    case_block: BlockId,
) -> BlockId {
    let mut tail = case_block;
    let mut walker = clause.walk();
    for stmt in clause.named_children(&mut walker) {
        if kinds.transparent_kinds.contains(&stmt.kind()) {
            tail = walk_case_statements(&stmt, src, kinds, narrowings, cfg, tail);
            continue;
        }
        tail = process_block_child(&stmt, src, kinds, narrowings, cfg, tail);
    }
    tail
}

/// Collect any defs (assignment LHS, variable declarators) reachable inside
/// `node` and append them to `block`. Walks shallowly — does not descend into
/// nested functions or into nested if-statements (those are CFG branches and
/// their defs belong to their own blocks).
fn collect_defs_in(node: &Node, src: &[u8], kinds: &CfgNodeKinds, cfg: &mut Cfg, block: BlockId) {
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
                if kinds.definition_name_kinds.contains(&lhs.kind()) {
                    if let Ok(name) = lhs.utf8_text(src) {
                        cfg.blocks[block as usize]
                            .defs
                            .push((name.to_string(), n.start_byte() as u32));
                    }
                }
            }
        } else if k == kinds.declarator_kind {
            if let Some(name_node) = n.child_by_field_name(kinds.declarator_name_field) {
                if kinds.definition_name_kinds.contains(&name_node.kind()) {
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
    cfg.blocks[block as usize].defs.sort_by_key(|(_, b)| *b);
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
    // Visited bit per block — `compute_in_fact` skips preds whose source
    // block hasn't been visited yet. Without this, a loop header would
    // compute on the first pop with its body's in-fact still empty, then
    // join-with-empty drops every narrowing the pred carried — even names
    // the body never touches. Entry seeds the propagation.
    let mut visited = vec![false; cfg.blocks.len()];
    if !cfg.blocks.is_empty() {
        visited[cfg.entry as usize] = true;
    }
    let mut work: VecDeque<BlockId> = (0..cfg.blocks.len() as BlockId).collect();
    let max_pops = iter_cap.saturating_mul(cfg.blocks.len().max(1));
    let mut pops = 0;
    while let Some(bid) = work.pop_front() {
        pops += 1;
        if pops > max_pops {
            break;
        }
        let new_in = compute_in_fact(cfg, bid, &preds, &visited);
        if new_in != cfg.in_facts[bid as usize] || !visited[bid as usize] {
            cfg.in_facts[bid as usize] = new_in;
            visited[bid as usize] = true;
            for e in &cfg.edges {
                if e.from == bid {
                    work.push_back(e.to);
                }
            }
        }
    }
}

fn compute_in_fact(cfg: &Cfg, bid: BlockId, preds: &[Vec<usize>], visited: &[bool]) -> FactMap {
    let pred_edges = &preds[bid as usize];
    if pred_edges.is_empty() {
        return FactMap::default();
    }
    let mut acc: Option<FactMap> = None;
    for &eid in pred_edges {
        let e = &cfg.edges[eid];
        if !visited[e.from as usize] {
            // First-pass-skip: an unvisited pred has no real in-fact yet, so
            // its contribution is "unknown" rather than "empty." The worklist
            // re-visits this block once the pred lands.
            continue;
        }
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
pub(super) fn _test_build_with_narrowings(
    src: &str,
    kinds: &CfgNodeKinds,
    lang: &tree_sitter::Language,
    narrowings: &[crate::types::Narrowing],
) -> FileCfg {
    use tree_sitter::Parser;
    let mut p = Parser::new();
    p.set_language(lang).expect("set lang");
    let tree = p.parse(src, None).expect("parse");
    build_file_cfg(&tree.root_node(), src.as_bytes(), kinds, narrowings)
}

#[cfg(test)]
#[path = "flow_cfg_tests.rs"]
mod tests;
