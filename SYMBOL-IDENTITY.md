# Symbol identity — stable `SymbolId` via intern-by-key

The reference for the symbol-graph refactor: make the persisted id-edge graph the source of
truth, give every cross-file-addressable symbol a **stable id** that survives incremental
reparse, and demote the in-memory string maps (`by_qname`, `members_by_parent`, `inherits_map`,
`type_info`, qname-keyed `containing_scope`) to typed views over it.

## North star

> A symbol's **key** changes if and only if its **public contract** changes. Key-stable ⇒
> consumers are never re-resolved. The key boundary *is* the consumer-invalidation boundary.

Everything below derives from that one invariant.

## 1. The stable key

Computed from the `ExtractedSymbol` alone — **syntactic, pre-resolution** (no resolve↔identity
chicken-and-egg; `qualified_name`, `kind`, `param_types`, `generic_params`, modifiers are all
available at extract time).

```
core      = qualified_name # kind # generic_arity [# "(" param_spellings ")"]   // params only for overloadable kinds
key(sym)  = mergeable(sym) ? core                       // file-independent → one id, multi-location
                           : file_id ":" core           // file-scoped → distinct per file
```

| kind | core components |
|---|---|
| class / interface / struct / enum / record | `qname # kind # generic_arity` |
| method / function / constructor | `qname # kind # generic_arity # (param_spellings)` |
| field / property / parameter / namespace / module | `qname # kind` |

### param spellings (ruling: normalize whitespace + `_`)

- **Written source text**, never resolved types (`foo(n: User)` → `"User"`). Renaming `User`
  does not change `foo`'s key — `foo` is the same method; only its param's `typeref` edge
  re-resolves.
- **Whitespace-normalized**: collapse all internal whitespace, trim. `List< int , string >`
  → `List<int,string>`.
- **Inferred / un-spelled params → `_`**, one per param, preserving arity:
  `function(a, b)` → `(_,_)`. Dynamically-typed languages collapse overloads to arity-only,
  which is correct — they have no type-based overloading.

### why these and not others

- **Body never in the key** — edit a body, key stable, zero consumers re-resolved. The point.
- **Signature in the key** — change a signature, key changes, symbol churns, consumers
  re-resolve. Also correct: the contract changed.
- Store the **full key string** (`symbol_key TEXT`, indexed) — queryable, debuggable, no
  collision math. ~80 B × 1M ≈ 80 MB.

## 2. Mergeable + the multi-location model (ruling: from day one)

`mergeable(sym)` = the symbol can be declared across multiple files as **one logical symbol**.
Computed at extract time from kind + modifiers, stored as a column. Driven by profile data, not
hooks:

```
always mergeable:   Namespace, Module, Package          // every language spreads these across files
profile-declared:   C# `partial` class/struct/interface/record;
                    Ruby class/module (always reopenable); TS interface/namespace (declaration
                    merging); any language whose profile lists the (kind, modifier) as multi-decl
not mergeable:      everything else — a method body, a field, a non-partial class — lives in one file
```

Note the composition: a method `Foo.Bar` is **not** mergeable (one body, one file) even when its
parent `Foo` **is** (a partial class). `Bar` gets a file-scoped key; its `containing_id` points
at `Foo`'s single shared id. Members spread across files (Rust `impl` blocks, Go methods) need
**no** multi-location — they are single-decl members whose `containing_id` crosses files to the
parent's stable id. Multi-location is only for symbols declared multiple times *themselves*.

### canonical row + locations

```sql
-- one row per LOGICAL symbol; mergeable symbols have ONE row regardless of declaring-file count
ALTER TABLE symbols ADD COLUMN symbol_key    TEXT;                                   -- the stable key
ALTER TABLE symbols ADD COLUMN mergeable      INTEGER NOT NULL DEFAULT 0;
ALTER TABLE symbols ADD COLUMN containing_id  INTEGER REFERENCES symbols(id) ON DELETE CASCADE;
-- symbols.file_id stays = the PRIMARY declaration file (fast filtering / origin, the common path)

CREATE TABLE symbol_locations (                 -- every declaration site
    symbol_id INTEGER NOT NULL REFERENCES symbols(id) ON DELETE CASCADE,
    file_id   INTEGER NOT NULL REFERENCES files(id)   ON DELETE CASCADE,
    line INTEGER NOT NULL, col INTEGER NOT NULL,
    PRIMARY KEY (symbol_id, file_id)
);
CREATE INDEX idx_symbols_key_global ON symbols(symbol_key) WHERE mergeable = 1;
CREATE INDEX idx_symbols_key_local  ON symbols(file_id, symbol_key) WHERE mergeable = 0;
CREATE INDEX idx_symbols_containing ON symbols(containing_id);
CREATE INDEX idx_symloc_file        ON symbol_locations(file_id);
```

- Non-mergeable symbol: one `symbols` row (`file_id` = its file) + one `symbol_locations` row.
- Mergeable symbol: one `symbols` row (`file_id` = primary site) + N `symbol_locations` rows.
- `members` = `WHERE containing_id = ?` (inverse, free). `params` = members with `kind = Parameter`.
- `base` = the existing `inherits` edge. Type refs = the existing `typeref` edge. `edges` is unchanged.

## 3. Interning scope — what gets a stable id

```
INTERNED (stable id, survivor-matched):  top-level decls + members of named types + namespaces
CHURNED  (delete + reinsert, free):       locals, lambdas, local-fn params, block temps
```

Locals have no stable qname (insert a line, positional names shift) and are never cross-file edge
targets, so churning them is both necessary and harmless. This confines survivor-matching to the
addressable set — the only set where a stable id pays.

## 4. Survivor-matching on reparse of file F

```
1. extract F → new symbols; compute key + mergeable for each interned one
2. lookups:
     non-mergeable:  existing rows WHERE file_id = F            keyed by symbol_key
     mergeable:      existing rows globally                     keyed by symbol_key
3. reconcile:
     key matches      → SURVIVOR: reuse id; UPDATE mutable cols (line/col/signature/scope);
                        for mergeable, upsert symbol_locations(id, F)
     new key          → INSERT (fresh id, file_id = F) + symbol_locations(id, F)
     old key gone:
        non-mergeable → DELETE (cascade drops outgoing edges)
        mergeable     → remove symbol_locations(id, F); if NO locations remain → DELETE the symbol;
                        else if F was primary → RE-HOME (promote another location to primary,
                        UPDATE symbols.file_id)
4. always re-resolve F's OWN outgoing refs (its body may have changed what it calls/uses)
5. re-resolve DEPENDENTS only of symbols whose key VANISHED or CHANGED, or that were fully deleted.
   surviving-key symbols trigger NO dependent re-resolution.
```

### the blast-radius rule (the whole point)

```
reparse F ⇒ re-resolve F (always)
          + re-resolve { dependents of s | s ∈ F, s.key removed/changed, or s fully deleted }
          NOT dependents of surviving-key symbols
```

Editing a method body in a 5000-file project re-resolves **one file**, not every caller.
Today (churn-all) re-resolves every dependent on every keystroke; this caps it to API changes.

## 5. containing_id write order

Within F, a symbol's parent is in F (or a mergeable parent elsewhere). Resolve after F's ids are
known: `parent_index → parent's id`; for a cross-file mergeable parent, look it up by the parent's
global `symbol_key`. `containing_id` is itself an id-edge — cascade-safe.

## 6. In-memory arena (the consumer side)

`SymbolId = symbols.id`. Load `(symbols, symbol_key, containing_id, edges)` into a `Symbol` arena;
relationships are id edges. `ContainingScope` becomes a **view**: `containing_chain(id)` walks
`containing_id` upward. Delete the parallel string maps one at a time:

```
inherits_map        → the `inherits` edge (already redundant — delete first)
members_by_parent   → containing_id inverse query
type_info           → typeref edges + per-symbol columns
containing_scope(qname-keyed)  → containing_id walk
by_qname            → shrinks to the ONE remaining string boundary: qname → SymbolId
```

The qname survives at exactly one place — matching a name to a surrogate id, at name-lookup and
at reparse survivor-matching. That is the surrogate-key pattern (Roslyn `SymbolKey`, Clang USR),
not a leak.

## 7. Staging

1. **Columns + writing, on the current delete+reinsert path.** Add `symbol_key`, `mergeable`,
   `containing_id`, `symbol_locations`; compute and write them. Ids still churn — but keys and
   containment land and can be validated against real reparse churn. Low-risk, independently
   shippable.
2. **Survivor-matching write path.** Switch incremental to §4; ids stabilize; add the
   key-vanished/changed dependent rule and mergeable re-homing.
3. **Arena as a view.** Build the `Symbol` arena over the persisted graph; delete the string maps
   in the order of §6.

## 8. Parked / to-decide during build

- **Mergeable predicate per language** — the exact (kind, modifier) set per `LanguageProfile`.
  Seed: Namespace/Module/Package everywhere; C# partial; Ruby class/module; TS interface/namespace.
- **Overload key collisions within one file** — two new symbols computing the same key (extractor
  ambiguity). Detect, log, positional-tiebreak or churn both; never crash.
- **Weak-qname languages** — interning reliability inherits qname quality; degrades gracefully to
  churn where qnames are poor (per-language, not global).
- **Re-home cost** — promoting a mergeable symbol's primary on primary-file delete; bounded to
  mergeable symbols, but verify it's not a hot path on bulk deletes.

## Stage 1 landed — deviations from the spec above

Two changes forced by implementation reality (full lib suite green, 6732 tests):

- **`containing_id` is `ON DELETE SET NULL`, not `CASCADE`.** A deleted parent must orphan
  its children (NULL their `containing_id`, to be re-resolved), never delete them — correct
  for the multi-location/merged model, and CASCADE on a self-referential FK is hazardous.
- **The `idx_symbols_containing` index is deferred to Stage 3.** Creating an index on the
  self-referential FK column `containing_id` deadlocks the full-index pipeline (a SQLite
  self-FK + indexed-FK-column interaction — reproduced deterministically, isolated by bisection;
  an index on the non-FK `symbol_key` column is unaffected). Stage 1 ships without it (no
  consumer yet — member lookup is Stage 3). **Before Stage 3 adds member-by-`containing_id`
  queries, resolve this** — most likely by dropping the FK and keeping `containing_id` a plain
  indexed integer, with integrity maintained by survivor-matching. **This also gates nothing in
  Stage 2** (survivor-matching keys on `symbol_key`, whose indexes are safe).

## Stage 2 landed — survivor-matching write path

The incremental path matches each symbol to its existing row by `symbol_key` instead of
deleting and reinserting (`write_parsed_files_incremental`; the old churn entry
`write_parsed_files` is gone). Survivors keep their id, so a body-only edit re-resolves the
edited file alone and **zero** dependents — proven end-to-end by
`body_only_change_does_not_reresolve_dependents`. Full lib suite green (6738 passed).

Implementation notes beyond the §4 sketch:

- **Survivors need explicit outgoing-ref clearing.** The churn cascade used to drop a changed
  file's outgoing edges for free; a survivor keeps its id, so its stale `edges` /
  `unresolved_refs` / `external_refs` (where `source_id` is one of F's symbols) are deleted
  explicitly before re-resolution. Inbound edges to survivors are never touched — the win.
- **Blast radius is two precise sets, not "all dependents".** Deleted files keep the old
  `find_dependent_files` path (their symbols vanish wholesale via CASCADE; computed pre-delete).
  Modified/added files contribute, through the returned `SurvivorReport`, only the dependents of
  symbols whose key actually vanished, plus a `newly_resolvable` scan keyed off genuinely-new
  keys. `find_dependent_files` was split into `(target_paths, exclude_paths)` to serve the
  deleted-file case without re-introducing the full radius for modified files.
- **Mergeable re-homing is implemented.** When the primary file drops a mergeable symbol that
  still lives elsewhere, the primary is promoted to a surviving location (id stable) rather than
  deleted; a drop from a non-primary file just removes that `symbol_locations` row. The mergeable
  global lookup (`symbol_key`, `mergeable = 1`) lets a declaration in a new file reuse the
  existing logical id within the same write transaction.
- **Overload key collisions degrade safely** (§8). A `used_ids` guard makes the first new symbol
  claim a survivor id and any same-key sibling fall back to a fresh insert — never a
  double-assignment, never a crash.
- **NULL-key legacy rows migrate by churning once.** Rows written before Stage 1 (or by the
  no-arena path) carry a NULL key, can't match a keyed new symbol, and so vanish + reinsert
  keyed on the first reparse — a one-time id churn per such file.
- `containing_id` is still un-indexed (Stage 3 self-FK item, unchanged).
