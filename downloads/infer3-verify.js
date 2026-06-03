export const meta = {
  name: 'infer3-fixpoint-verify',
  description: 'Adversarial review of the INFER-3/2 return-type-inference fixpoint',
  phases: [{ title: 'Verify', detail: 'four read-only lenses on the fixpoint' }],
}

const DIFF = String.raw`
INFER-3/2 return-type-inference fixpoint. Capture slice (flow_return_lhs: ref_idx to fn_symbol_idx,
for DIRECT-body returns in named TS functions/methods) already committed. This is the harvest +
join + gap-fill + bounded re-resolve.

--- full.rs (orchestrator, AFTER the externals chain-expansion loop, BEFORE finalize_resolution) ---
const MAX_RETURN_ITERATIONS: usize = 3;
for ret_iteration in 0..MAX_RETURN_ITERATIONS {
    let mut applied = 0usize;
    if let Some(idx) = cached_index.as_mut() {
        for (qname, ty) in rstats.inferred_returns.iter() {
            if idx.set_inferred_return(qname.clone(), ty.clone()) { applied += 1; }
        }
    }
    if applied == 0 { break; }
    db.conn().execute("DELETE FROM unresolved_refs", [])?;
    db.conn().execute("DELETE FROM external_refs", [])?;
    rstats = resolve::resolve_iteration_with_cached_index_and_arena(db, parsed, ..., cached_index, &[], arena)?;
}
NOTE: edges use INSERT OR IGNORE and survive across re-resolves; external_refs/unresolved_refs are
CLEARED and fully rebuilt by each re-resolve. The existing externals loop above uses the SAME
clear-then-re-resolve pattern. The externals loop drives on rstats.converged() (chain_misses empty), unchanged.

--- SymbolIndex::set_inferred_return (augment.rs) ---
pub fn set_inferred_return(qname: String, ty: String) -> bool {
    let entry = self.type_info.entry(qname).or_default();
    if entry.return_type.is_none() { entry.return_type = Some(ty); true } else { false }
}
NOTE: return_type_name(qname) reads type_info[qname].return_type. type_info is also built from
signatures/TypeRefs (declared returns). set_inferred_return NEVER overrides an existing return.

--- loop_body.rs harvest (per ref, after the assignment forward-inference write) ---
if flow_return_lhs has ref_idx (giving fn_idx):
    fn_sym = pf.symbols[fn_idx]
    if fn_sym.return_type is None AND index.return_type_name(fn_sym.qname) is None:
        yield_str = format_type(resolution.resolved_yield_type)  OR  target symbol return/field type
        if yield_str non-empty and not "Unknown":
            buf.inferred_returns.push((fn_sym.qname, yield_str))

--- loop_body.rs join (after the parallel reduce, per pass) ---
by_fn: map qname to (Some(agreed) | None=conflict)
for (qname, ty) in combined_buf.inferred_returns:
    none so far -> Some(ty); already Some(prev) and prev != ty -> None (conflict); else nothing
for (qname, agreed) in by_fn:
    if agreed is Some(ty) and index.return_type_name(qname) is None:
        stats.inferred_returns.insert(qname, ty)

KEY FACTS:
- resolve_iteration_body runs once per resolve pass (parallel over files; per-worker FileWriteBuf merged
  in reduce). The join runs single-threaded after the merge.
- yield_str: resolved_yield_type via arena.format_type, else the target symbol declared return/field type.
  Skipped when empty or "Unknown".
- qname for free functions may NOT be globally unique (two files can each define helper). The
  type_info / return_type_name map is keyed by qname.
- Mutual recursion f returns g(), g returns f() -> neither yields a concrete type -> skipped (Unknown).
- Capture scope: only DIRECT-body returns of NAMED functions/methods (not nested in if/for, not arrows).
`

const LENSES = [
  { key: 'soundness', prompt: 'LENS: SOUNDNESS / WRONG INFERENCE. Find any input where this infers a WRONG return type for a function (mis-resolving a callers chain). Scrutinize: the conflict-join (does it infer NOTHING when returns disagree? handle >2 returns, same-type-twice-then-different?); the Unknown/empty filter (any other sentinel format_type emits for an un-inferred type that should be skipped but is not, e.g. a bare generic param name, an empty object, a question mark?); cross-file qname collisions (two distinct free functions both named helper in different files share a qname, so their candidates merge in the single by_fn map and in the type_info map -> wrong-bind or safe miss?); the gap-fill never overriding a declared return. Classify each as is_bug=true (wrong inference) vs is_bug=false (safe miss).' },
  { key: 'termination', prompt: 'LENS: TERMINATION / CYCLES / CONVERGENCE. Prove or break termination. The loop is bounded by MAX_RETURN_ITERATIONS=3 AND exits when applied==0. set_inferred_return returns true only when filling a previously-empty slot. Walk: self-recursion; mutual recursion; a deep transitive chain (a returns b() returns c() ... five levels) under a cap of 3 -- converge or silently truncate, and is truncation a wrong result or a sound incomplete miss? Can applied stay positive forever (oscillation)? Can the same qname be re-gap-filled across iterations (should not -- once set, returns false)? Is three iterations enough, and what is the failure mode if a project needs more (wrong vs incomplete)?' },
  { key: 'data-integrity', prompt: 'LENS: DB / EDGE / TABLE INTEGRITY ACROSS RE-RESOLVES. Each iteration re-runs a full resolve pass: re-flush edges (INSERT OR IGNORE) and CLEAR+rebuild unresolved_refs/external_refs. Find integrity bugs: can clearing external_refs/unresolved_refs then re-resolving LOSE rows the new pass will not re-derive (e.g. an external_ref depending on a chain miss that no longer fires because a return is now inferred)? Could a previously-resolved edge become unresolved (regression)? Does INSERT OR IGNORE leave STALE edges from a prior pass the new pass would NOT produce (edges additive-only across passes -- correct here)? Is incoming_edge_count finalize correct after extra passes? Compare to the existing externals loop which uses the identical clear+re-resolve pattern.' },
  { key: 'perf-regression', prompt: 'LENS: PERF / REGRESSION. Claim: non-TS or no-inferable-returns projects pay ZERO (inferred_returns empty -> applied 0 -> immediate break). Verify (could inferred_returns be non-empty spuriously?). For TS: up to 3 extra FULL resolve passes (re-resolve all refs, re-flush edges, rebuild tables). Assess worst-case cost on a large TS project (ts-nextjs ~41min reindex per the memory) and whether 3 full re-resolves is acceptable or needs a tighter/targeted bound. Does the harvest add per-ref cost in the hot loop for NON-return refs (a hashmap lookup per ref)? Confirm the existing assignment forward-inference path is untouched.' },
]

const SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['lens', 'real_issue', 'severity', 'findings'],
  properties: {
    lens: { type: 'string' },
    real_issue: { type: 'boolean' },
    severity: { type: 'string', enum: ['none', 'low', 'medium', 'high'] },
    findings: { type: 'array', items: {
      type: 'object', additionalProperties: false,
      required: ['title', 'is_bug', 'explanation', 'repro', 'suggested_fix'],
      properties: {
        title: { type: 'string' },
        is_bug: { type: 'boolean' },
        explanation: { type: 'string' },
        repro: { type: 'string' },
        suggested_fix: { type: 'string' },
      },
    } },
  },
}

phase('Verify')
const results = await parallel(LENSES.map((l) => () =>
  agent(
    'Adversarially review a Rust change to BearWisdom resolve pipeline: the INFER-3/INFER-2 return-type-inference fixpoint. Repo at F:/Work/Projects/BearWisdom. READ the real source to ground claims (indexer/full.rs return loop, indexer/resolve/loop_body.rs, indexer/resolve/engine/index/augment.rs set_inferred_return, indexer/resolve/engine/index/lookup_impl.rs return_type_name, indexer/flow.rs) but be STRICTLY READ-ONLY: do NOT edit and do NOT run cargo/build/tests (single-cargo-at-a-time rule). Reason from the code.\n\n' + DIFF + '\n\n' + l.prompt + '\n\nBe a harsh skeptic but distinguish a GENUINE bug (is_bug=true: wrong inference, real regression, nontermination, data corruption) from a safe miss / acceptable limitation (is_bug=false). Only set real_issue=true and severity above none if you found at least one is_bug=true finding. Verify any claim about format_type / return_type_name / the externals loop in the actual files before asserting it.',
    { label: 'verify:' + l.key, phase: 'Verify', schema: SCHEMA, agentType: 'Explore' }
  )
))
return results.filter(Boolean)
