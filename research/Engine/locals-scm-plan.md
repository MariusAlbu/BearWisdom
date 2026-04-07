# locals.scm Integration Plan — Query-Based Scope Resolution

## Problem

BearWisdom's extractors manually build scope chains and emit refs for every identifier. The resolution engine then tries to resolve each ref against the project-wide symbol index. Many "unresolved" refs are actually **local variables** — function parameters, loop variables, catch bindings — that were defined in the same scope but the extractor didn't track them.

Example: in `function foo(bar) { return bar.baz() }`, the ref to `bar` is local — it's defined as a parameter of `foo`. Currently BearWisdom emits `bar` as an `ExtractedRef` and the resolution engine either:
- Matches it heuristically to some unrelated `bar` elsewhere (wrong)
- Leaves it unresolved (inflates unresolved count)
- Gets lucky and matches it via same-file resolution

## Solution

Use tree-sitter `locals.scm` query files at **parse time** to identify:
- **Scopes** (`@local.scope`) — where variables are visible
- **Definitions** (`@local.definition`) — where a name is introduced
- **References** (`@local.reference`) — where a name is used

Any `@local.reference` that has a matching `@local.definition` in the same or enclosing scope is **locally resolved** — it should NOT be emitted as an `ExtractedRef` to the resolution engine.

## How locals.scm Works

Each grammar community maintains a `queries/locals.scm` file that defines scope rules using tree-sitter S-expression queries.

**JavaScript example:**
```scheme
; Scopes
[ (statement_block) (function_expression) (arrow_function)
  (function_declaration) (method_definition) ] @local.scope

; Definitions
(pattern/identifier) @local.definition
(variable_declarator name: (identifier) @local.definition)

; References
(identifier) @local.reference
```

This says: every `identifier` node is a reference, variable declarators define names, and function bodies / blocks create scopes. The tree-sitter query engine can resolve which references bind to which definitions by walking the scope tree.

**18 grammars have locals.scm:** Ada, Bicep, Dart, F#, Gleam, Hare, Haskell, JavaScript, Lua, Nix, OCaml, Odin, Pascal, Puppet, R, Ruby, Scala, Starlark, Swift, TypeScript.

## Architecture

### Current Flow
```
Source → tree-sitter parse → AST walk (per-language extractor)
  → ExtractedSymbol[] + ExtractedRef[] → Resolution Engine → edges/unresolved
```

### Proposed Flow
```
Source → tree-sitter parse → AST
  ↓
  locals.scm query → LocalScope tree (definitions + references + scopes)
  ↓
  AST walk (per-language extractor, enhanced)
    → skip emitting refs that are locally resolved
    → ExtractedSymbol[] + ExtractedRef[] (fewer, higher quality)
    → Resolution Engine → edges/unresolved (fewer false positives)
```

## Implementation Phases

### Phase 1: LocalResolver Infrastructure

**Create `parser/local_resolver.rs`:**

```rust
pub struct LocalResolver {
    /// Compiled tree-sitter query from locals.scm.
    query: tree_sitter::Query,
    /// Capture indices for scope/definition/reference.
    scope_capture: u32,
    definition_captures: Vec<u32>,
    reference_capture: u32,
}

pub struct LocalResolution {
    /// Map from byte offset of a reference → byte offset of its definition.
    /// If a reference is in this map, it's locally resolved and should NOT
    /// be emitted as an ExtractedRef.
    resolved: FxHashMap<usize, usize>,
    /// Set of byte offsets for all locally-defined names.
    definitions: FxHashSet<usize>,
}

impl LocalResolver {
    /// Build from a locals.scm query string and a tree-sitter Language.
    pub fn new(locals_scm: &str, language: tree_sitter::Language) -> Option<Self>;

    /// Run the query against a parsed tree and resolve local references.
    pub fn resolve(&self, tree: &tree_sitter::Tree, source: &[u8]) -> LocalResolution;
}
```

**Key algorithm in `resolve()`:**
1. Run the query, collecting all captures
2. Build a scope tree from `@local.scope` captures
3. For each `@local.definition` capture, record `(scope, name, byte_offset)`
4. For each `@local.reference` capture:
   - Walk up the scope tree from the reference's position
   - If a definition with the same name exists in any enclosing scope, mark as resolved
5. Return the resolution map

**Files to create:**
- `parser/local_resolver.rs`

**Files to modify:**
- `parser/mod.rs` — add `pub mod local_resolver;`

### Phase 2: Load locals.scm at Build Time

**Extend `build.rs`** to also embed the raw `locals.scm` content for each grammar that has one:

```rust
// In addition to query_builtins.rs, generate locals_queries.rs:
pub fn locals_scm_for_language(lang: &str) -> Option<&'static str> {
    match lang {
        "javascript" => Some(include_str!("...")),
        "dart" => Some(include_str!("...")),
        // ...
        _ => None,
    }
}
```

Since `include_str!` needs compile-time paths and we can't use cargo registry paths directly, the build.rs approach should:
1. Read the `.scm` file content
2. Write it as a Rust string literal into the generated file
3. The generated function returns `Option<&'static str>`

**Files to modify:**
- `build.rs` — add locals.scm content embedding
- New generated file: `src/indexer/locals_queries.rs`

### Phase 3: Wire Into Extraction Pipeline

**Modify the `LanguagePlugin` trait:**

```rust
trait LanguagePlugin {
    // ... existing methods ...

    /// Return a LocalResolver for this language, if locals.scm is available.
    /// Cached per-language — constructed once at registration time.
    fn local_resolver(&self) -> Option<&LocalResolver> { None }
}
```

**Default implementation** (in `GenericPlugin` or a new trait default):
```rust
fn local_resolver(&self) -> Option<&LocalResolver> {
    // Try to load from the generated locals_queries module.
    let scm = locals_queries::locals_scm_for_language(self.id())?;
    let grammar = self.grammar(self.id())?;
    // Lazily construct and cache.
    Some(LocalResolver::new(scm, grammar)?)
}
```

Use `OnceLock<Option<LocalResolver>>` per plugin for lazy init.

**Modify the extraction call site** (in `indexer/full.rs`):

```rust
// Before calling plugin.extract():
let local_resolution = plugin.local_resolver()
    .map(|lr| lr.resolve(&tree, source.as_bytes()));

// Pass to extractor:
plugin.extract_with_locals(source, file_path, lang_id, local_resolution.as_ref())
```

**Modify extractors** to check local resolution before emitting refs:

```rust
// In any extractor, before pushing an ExtractedRef:
if let Some(locals) = local_resolution {
    if locals.is_locally_resolved(ref_byte_offset) {
        continue; // Skip — this is a local variable, not a cross-file ref
    }
}
```

**Files to modify:**
- `languages/mod.rs` — add `local_resolver()` to trait
- `indexer/full.rs` — call local resolver before extraction
- `languages/generic/extract.rs` — use local resolution in generic extractor
- Each dedicated extractor (optional, can be incremental)

### Phase 4: Integrate With Existing Scope System

The current `ScopeTree` (in `parser/scope_tree.rs`) builds scope chains for qualified names. The `LocalResolver` builds scope chains for variable resolution. These are complementary:

- **ScopeTree** → qualified names for symbols (`Foo.Bar.baz`)
- **LocalResolver** → which identifiers are locally defined vs. need cross-file resolution

They should coexist. The `ScopeTree` continues to provide `scope_path` for extracted symbols. The `LocalResolver` acts as a filter on which refs get emitted.

### Phase 5: Type Inference From Local Definitions

Once we know local definitions, we can extract type annotations from them:

```javascript
function process(user: User) {
    // @local.definition for "user" is at the parameter
    // The parameter has a type annotation "User"
    // So ref to user.name → we know user is of type User → resolve name on User
}
```

This requires extending `LocalResolution` to carry type information:

```rust
pub struct LocalDefinition {
    pub name: String,
    pub byte_offset: usize,
    /// Type annotation, if the definition has one.
    pub type_annotation: Option<String>,
}
```

The resolution engine can then use this for chain resolution: when `user.name` appears and `user` is locally defined with type `User`, look up `User.name` in the symbol index.

**This is the highest-impact phase** — it directly feeds the chain resolution system and eliminates the biggest class of unresolved refs (member access on typed locals).

## Impact Estimate

### Ref Count Reduction

For languages WITH locals.scm (18 languages):
- **30-50% of current unresolved refs** are local variables that would be filtered
- The remaining refs are genuine cross-file references — much higher signal

For languages WITHOUT locals.scm:
- No change (handcrafted extractors continue as-is)
- Community can contribute locals.scm files over time

### Resolution Rate Impact

| Language | Current | Estimated After | Delta |
|----------|---------|----------------|-------|
| JavaScript | 48% | 75-85% | +30pp (huge local var noise) |
| Ruby | 65% | 80-85% | +15pp |
| Lua | 62% | 80% | +18pp |
| Scala | 75% | 85% | +10pp |
| OCaml | 55% | 75% | +20pp |
| F# | 62% | 80% | +18pp |
| Dart | 91% | 95% | +4pp |
| Haskell | 68% | 80% | +12pp |

### Performance

- Query compilation: ~1ms per language (one-time, cached)
- Query execution: ~0.5ms per file (proportional to file size)
- Net impact: slight increase in parse time, significant decrease in resolution time (fewer refs to resolve)

## Risks

| Risk | Mitigation |
|------|-----------|
| locals.scm has bugs (wrong scopes) | Fallback: if local resolution reduces resolved edges, disable per-language |
| Query API changes between tree-sitter versions | Pin to tree-sitter 0.25 API; locals.scm format is stable |
| Some grammars use non-standard capture names | Detect both `@local.definition` and `@definition.var` etc. (Starlark, Puppet, Odin use the short form) |
| Performance regression on large files | Cap query execution at 10K nodes; skip for files > 50KB |
| Type inference from locals needs AST traversal | Phase 5 is optional — Phases 1-3 provide value without it |

## Priority and Dependencies

- **Phase 1** (LocalResolver) — standalone, no dependencies
- **Phase 2** (build.rs embedding) — depends on Phase 1
- **Phase 3** (extraction pipeline) — depends on Phase 2; biggest integration effort
- **Phase 4** (scope system coexistence) — parallel with Phase 3
- **Phase 5** (type inference) — depends on Phase 3; highest impact but most complex

**Recommended order:** 1 → 2 → 3 → 5 (skip 4 initially — existing scope system works fine alongside)

## Out of Scope

- **Replacing the existing ScopeTree** — it works well for qualified names; locals.scm is an addition, not a replacement
- **Writing locals.scm files for grammars that don't have them** — that's upstream grammar work
- **Cross-file local resolution** — locals.scm is strictly intra-file
- **Dynamic language features** — `eval()`, `method_missing`, `__getattr__` can't be resolved by any static analysis
