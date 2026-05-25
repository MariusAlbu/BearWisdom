# Generic-parameter unification — turning the substitution layer on

**Status:** U1 + U2a implemented (ungated); U2b + U3 (multi-level) remain. See §7.

**Thesis:** the engine's generic-substitution machinery (`substitute` / `GenericEnv` /
`bind_positional` over `Type::Generic`) is **production-dead** — its inputs are never the
canonical `Type::Generic` the code expects, because the workspace mints generic-parameter
ids in **three fragmented places** and the binder reads the one source nobody populates.
Unify on a single owner-scoped `Type::Generic` id space and the layer fires for the first
time. This is the prerequisite the multi-level-inheritance board item is blocked on, and
the reason single-level generic-inheritance substitution showed near-zero corpus delta.

Companion to `RESOLUTION-EXTERNAL-ROUTING.md` (external routing / origin-blindness). That
doc's S4 ("generics carried through chains, supertype walk over external bases — a
refinement") and D7 are downstream of this.

---

## 1. The finding — substitution never fires in production

Three grounded facts:

1. **No extractor mints generic params.** The only `intern_generic` calls are
   `build.rs:969` and `augment.rs:367`. Every extractor sets `generic_params: Vec::new()`
   on `ExtractedSymbol` (`types.rs:361`, `Vec<GenericParamId>`). So
   `SymbolTypeData.generic_params` (`symbol_types.rs:181,205`) is **always empty** in a
   real index.

2. **Both binders read that empty source.** The chain walker binds an owner's params in
   exactly two spots, both via `symbol_types.data_for_class(...).generic_params`:
   - `chain.rs:366` — inherited generic method (`extends Repository<User>`).
   - `chain.rs:620` — `bind_apply_args`, the direct `Repo<User>.first()` case.

   Empty params → `bind_positional` binds nothing → `substitute` is the identity.

3. **The one populated canonical-`Generic` source has a single consumer.** `build.rs`
   mints owner-scoped `Type::Generic` into `TypeInfo.generic_param_type_ids`
   (`build.rs:968-981`, `augment.rs:367-385`), exposed at `lookup_impl.rs:191`. It is read
   **only** by gap A (root receiver typing, `chain.rs:192`). The binders never see it.

```rust
// What the engine THINKS happens                 // What actually happens in prod
class Repo<T> { first(): T }
const r: Repo<User>;
r.first()   // ✅ bind T→User, yield User          // ❌ data.generic_params == []  (chain.rs:620)
                                                   //    → nothing bound → yield stays T / Class("T")
```

### Why even a populated binder wouldn't be enough

`substitute` rewrites `Type::Generic`. But the **inputs** it would substitute into are
nominal classes, because `intern_type_str` is param-blind:

```
return type   "T"        → intern_type_str → Class("T")    (not Generic)   build.rs:917
edge arg      "C<T>"     → intern_type_str → Apply{C,[Class("T")]}         supertype.rs:206
```

So a method's `T` return and an inheritance edge's `T` arg are both nominal — substitution
has nothing to grab even with the right env.

### Fragmentation map

| source | populated? | minted | consumed by |
|---|---|---|---|
| `ExtractedSymbol.generic_params` | ✗ never | (no extractor) | `symbol_types` → both binders — reads EMPTY |
| `TypeInfo.generic_param_type_ids` | ✓ | `intern_generic` (build.rs) | gap A root only — populated, underused |
| return/field/edge-arg strings | ✓ | `intern_type_str` (build.rs) | `substitute` input — param-blind → `Class("T")` |

---

## 2. Target

One canonical `Type::Generic` id space — `build.rs`'s `intern_generic`, owner-scoped (the
`owner_symbol_index` already distinguishes `T`-on-`A` from `T`-on-`B`) — referenced
consistently by:

- the binder's param list (so `bind_positional` has real params),
- return / field types that name a declared param (so the substitution input is `Generic`),
- inheritance edge args that name the **child's** declared param (so multi-level composes).

Lean on `build.rs` as the single source of truth, not extractor TypeIds — those can come
from a throwaway arena (`build.rs:890-894`) and the Phase-5 extractor migration that was
meant to populate them never landed.

---

## 3. Staged plan

| Stage | Change | File(s) | Size |
|---|---|---|---|
| **U1** | **Feed the binder from the populated source.** Copy `generic_param_type_ids`' param ids into `SymbolTypeData.generic_params` at build (bridge `sym_id` ↔ qname), or route `bind_apply_args` + `chain.rs:366` to read `generic_param_type_ids` directly. *Turns substitution on at all.* Validates immediately against the existing fix-#1 unit cases — and against a corpus chain like `Repo<User>.first()`. | `symbol_types.rs`, `build.rs` (or `chain.rs`) | small |
| **U2** | **Param-aware interning.** A variant of `intern_type_str` that, given the owner's declared param names, interns matching tokens as the canonical `Type::Generic` (owner-scoped) instead of `Class`. Applied to return/field types and to inheritance edge args in `build_explicit`. *Makes substitution inputs bindable.* Corpus-wide → **gate it** (A/B like the routing spine). | `build.rs`, `chain_walker.rs` (the `intern_type_str` parser), `supertype.rs:206` | medium |
| **U3** | **Multi-level falls out.** With real `Generic` edge args (U2) and a populated binder (U1), add the `walk_up_with_args` composition: substitute each edge's args through the binding the child's args established, so `A: B<X>, B<T>: C<T>` arrives at `C` with `[X]`. | `supertype.rs:82` (`walk_up_with_args` + `arena`), `members.rs` (`find_on_chain` + `arena`) | small |

`U3`'s composition was the original "multi-level" ask; it is inert without `U1`+`U2`.

---

## 4. Validation

- **The metric is substitution-fires, not rate.** Count generic-chain yields that resolve
  to a concrete type (vs. staying `T` / `Class("T")`) before → after. Currently ~0;
  success is a positive delta.
- U1 alone: the fix-#1 family (`Repo<User>.first()`, `extends Repository<User>`) resolves
  on a real corpus, not just unit tests.
- U2/U3 gated; gate-off must be byte-identical to baseline (same discipline as the routing
  spine). A/B on a generic-heavy TS project (tRPC / typeorm builders) and a Rust generic
  project.

## 5. Risk

- **U2 changes interning corpus-wide.** Param-aware interning touches every generic return
  type in every language. Gate and A/B before it touches the default path.
- **Owner scoping must be correct.** `owner_symbol_index` keys the param id; a wrong/zero
  owner collapses `T`-on-`A` and `T`-on-`B` into one id and cross-binds. `build.rs:967`
  derives the owner from `by_qname` — verify it's non-zero for the types that matter.
- **Throwaway-arena TypeIds** (`build.rs:890`): do not resurrect extractor TypeId fields as
  a shortcut; they are not reliably workspace-arena. Single source = `build.rs`.

## 6. Relation to the board & routing doc

- **Board #1** (generic args through inheritance): unit-passing, **inert in production** —
  U1 turns it on.
- **Board #2** (bounded generic): already works — it resolves `T` to its *bound*, never
  needing substitution.
- **Multi-level inheritance**: blocked on this; = U1 + U2 + U3.
- **D7 / bidirectional** (`RESOLUTION-EXTERNAL-ROUTING.md`): separate, larger; not gated on
  this but shares the "production type pipeline must produce `Type::Generic`" premise.

---

## 7. Implementation status — U1 + U2a landed (ungated)

The realization deviated from §3's predicted locus: both stages live in `chain.rs` +
`types.rs`, **not** `build.rs`. The reason is that the chain walker consumes a generic
method's return via the **string-yield path** (`yield_type_of`, where the extractor's
TypeId return is `None` and the fallback formats `TypeInfo.return_type` → `"T"`), not via
`TypeInfo.return_type_id`. So param-awareness had to be applied at the walk, not the store.

- **`TypeArena::rebind_class_params`** (`types.rs`) — recursive `Class(name) → Generic(id)`
  rewriter, keyed on a `{name → Type::Generic id}` map.
- **U1** (`chain.rs` `owner_generic_params`): the two binders (`bind_apply_args` + inherited
  generic method) now read the owner's params from `symbol_types` and **fall back to the
  canonical `generic_param_type_ids`** when empty (the prod case).
- **U2a** (`chain.rs` `owner_param_type_map`): `yield_type_of`'s string fallback rebinds the
  interned return through the owner's canonical params before `substitute`.

Ungated (same neutral-or-positive risk profile as the bounded-generic / forward-inference
fixes; `Class("T")` never resolved to a real type except correct shadowing). Unit-proven
end-to-end (`substitution_fires_from_canonical_params_and_string_yield`): `Repo<User>`'s
`.first()` yields `User` through the canonical params + string-yield rebind, where it
yielded `Class("T")` before.

**Corpus (ts-rallly):** internal_edges 22396 → 22392 (−4), unresolved flat (390), rate flat
(98.29%). Net-neutral — the −4 is edges shifting (more-precise yields routing to
external/dedup), not refs falling to unresolved. No edge win because the gain is *yield
precision*, which only adds edges on 3+ hop generic chains; ts-rallly has few (as predicted
— the win is on tRPC / typeorm-builder / Rust-generic corpora).

**Remaining for multi-level inheritance:** U2b (intern an inheritance edge arg that names
the **child's** own param as `Generic`, in `build_explicit`) + U3 (`walk_up_with_args`
composition). With U1 live, those now have a populated binder to compose against.
