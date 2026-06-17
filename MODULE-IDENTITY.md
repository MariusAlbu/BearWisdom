# Module identity — cross-language symbol disambiguation

Sibling of `SYMBOL-IDENTITY.md`. Defines how a symbol's **enclosing module /
container** participates in its resolution identity, so the same simple/qualified
name in different modules (monorepo packages, separate files, separate
assemblies) does not collide.

## Problem

Resolution keys on the qualified name string. Module-level symbols are qualified
only by their in-file scope chain, never by their module, so two modules that
export the same name produce the same qname:

```
react-query/src/QueryClientProvider.tsx   →  qname "useQueryClient"
vue-query/src/useQueryClient.ts           →  qname "useQueryClient"   ← same key, collides
```

Consequences, both observed in the TanStack Query monorepo:

- `join_inferred_returns` (loop_body.rs) collision-skips any qname owned by more
  than one `db_id` — a soundness guard — so no return is inferred for these.
- `callee_return_type` / `value_root_type` pick a `by_name` candidate with no
  module context, so a call resolves to an arbitrary same-named definition (or
  none), and the whole `client.getQueryCache()` / `.defaultQueryOptions()` chain
  fails even though the members are indexed.

This is the gate for the two largest unresolved buckets in that project
(~145 + ~174 internal refs).

## Model — Roslyn-shaped

Roslyn never uses a bare name as identity. A symbol's identity is its **containing
chain** rooted at a logical container, and that container is uniform whether the
symbol came from source or a referenced assembly:

```
SourceAssemblySymbol : IAssemblySymbol   // your compiled project
PEAssemblySymbol     : IAssemblySymbol   // a referenced .dll — NO source file
// List<T> from CoreLib and your own List<T> are distinct because ContainingAssembly
// differs. Identity never depends on a file existing.
```

The container is **logical**, not "the file path". In TS the file *is* the module
(ES module = file), so the file is the container; in C# the namespace is the
container (partial classes span files, so file would be wrong). The engine roots
on a container; the language says what the container is.

## Identity

```
resolution identity = (ModuleId, qname [, arity])

today:   key = "useQueryClient"                       ← collides
design:  key = (Mod(react-query/QCP.tsx), "useQueryClient")
               (Mod(vue-query/uQC.ts),    "useQueryClient")   ← distinct

by_name (the simple-name leaf, = ISymbol.Name) is UNCHANGED — still "useQueryClient".
```

- **Structured, not string-concat.** A `ModuleId` is interned (the `TypeId` /
  `packages` precedent), kept as a separate field — mirroring Roslyn keeping
  `ContainingAssembly` a property and concatenating only for the DocID string.
  CLI/MCP keep displaying the clean qname; only the resolution maps key on the
  tuple. (Per the structured-over-strings rule.)
- **arity / parameter types** fold into the key for overload languages
  (C#/Java/C++), as Roslyn's SymbolKey does. Defined now, populated per-language
  when those languages take finer granularity.

## Per-language container — data first, code only when forced

The language owns the **source** derivation; the ecosystem walker owns the
**external/metadata** container. Three tiers, per the engine's design contract:

```
[profile data]   a granularity rule, NO code — the generic engine derives the id
                 from symbol.file + the rule:
                   TS / JS / Python → File        (module = the source file)
                   C# / F#          → Namespace   (file-independent; partials span files)
                   Go / Java        → Package
[hook]           only when the rule can't capture it:
                   Rust   → crate root (Cargo.toml) + `mod` tree
                   Python → dotted module across __init__.py packages
                 LanguageEngineHooks::module_id_for(symbol, file, ctx) -> ModuleKey
[walker]         EXTERNAL / metadata symbols — the ecosystem walker that already
                 located the dep stamps the container as it emits the symbol:
                   .NET .dll → assembly identity   (dotscope already reads it — the PEAssembly case)
                   Java jar  → artifact coordinates
                   npm       → package + subpath
                 No separate algorithm; a byproduct of walking, set at emit time.
```

The engine makes one call per symbol at index time:

```rust
let module_id = match symbol.origin {
    External => symbol.module_id,                       // walker already stamped it
    Internal => granularity.derive(symbol, file_path)   // data path (most langs)
        .or_else(|| hooks.module_id_for(symbol, file_path, ctx)),  // hook path (Rust/Python)
};
```

The .NET DLL case falls out for free: a `PEAssemblySymbol`-equivalent has no
file, so `File`/`Namespace` data rules don't apply — its container comes from the
walker, the only place with the assembly metadata.

## What already exists (no new data needed for the TS case)

- `Symbol` carries `file_path: Arc<str>` and `package_id: Option<i64>`
  (contract/types.rs). The **File** container for TS source is `symbol.file_path`
  — already on the symbol.
- `Compilation` (engine's Roslyn `Compilation`, compilation.rs) already groups
  `by_package`; `packages` table + `package_id` is the coarse container.
- `symbol_key` (stable identity) and `containing_id` (the containment chain) exist
  — `ModuleId` is the root of that chain.

## Storage

```
modules intern table (sibling of `packages`):
  id │ kind(File|Namespace|Package|Crate|Assembly) │ key │ origin
symbols.module_id, edges/refs carry it (or, for the File case, reuse file_path
  until the intern earns its keep for the Assembly/Namespace cases).
```

## The load-bearing invariant

Roslyn gets this for free via `ISymbol` object identity; we enforce it on the
tuple:

```
symbol-emission ModuleId  ==  ref-resolution target ModuleId      (byte-identical)
```

Every place that builds a qname for a **symbol** and every place that builds the
target qname for a **ref** must produce the same `ModuleId`. The import / using /
use resolver already maps a module specifier → file/package, so it can produce
the target container. This is the audit surface where regressions hide.

### Map surfaces to tuple-key (Compilation, compilation.rs)

```
by_qname · by_qname_all · members_by_parent · type_info (field/return) · inherits
· enclosing_type · enclosing_namespace · alias_target · join_inferred_returns
· by_name candidate selection (import → target ModuleId → pick matching candidate)
```

## Roll-out (risk-contained, measurable per step)

1. **Foundation (zero-change).** `ModuleGranularity` + `ModuleKey` + a pure
   `module_root(file, package, granularity)` with tests. No schema, no behavior.
2. **Tuple-keying with a single default container.** Resolution maps become
   `(ModuleId, qname)` with every symbol assigned ONE container per project →
   byte-identical to today across the corpus. Pure refactor, validated green.
3. **TS → File granularity.** Assign per-file ModuleId for TS internal symbols;
   teach `by_name` candidate selection + import resolution to use it. Validate:
   single-package TS projects unchanged; TanStack monorepo collisions
   disambiguate. Measure buckets ① / ②.
4. **C# → Namespace, then the rest**, each behind the granularity rule, each
   measured. The `LanguageProfile` granularity field is added here (94 profile
   literals — mechanical; default `File`, override the exceptions).
5. **External containers** via the walkers (.NET assembly first — the DLL case).

## Cost notes

- 94 `LanguageProfile` const literals (no default-spread in const) → the
  granularity field is a 94-file mechanical edit; deferred to step 4 so the
  scaffold doesn't force it.
- Steps 1–2 are corpus-safe by construction (single container = today). Risk
  enters at step 3+, contained to the languages flipped, validated per step.
