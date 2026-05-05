# Ecosystem layer — architecture and authoring guide

This is the contract every `Ecosystem` impl in this directory must follow. The architecture exists to make external dependency discovery **project-driven** — not machine-driven. Read this before adding or modifying anything in `ecosystem/`.

## What an ecosystem is

One ecosystem = one external-source provider:

- A **package world** (one install location, one dep format, one artefact shape): npm, Cargo, Maven, Hex, vcpkg, Conan, NuGet.
- A **stdlib / runtime source** (language toolchain headers + types): rust-stdlib, jdk-src, qt-runtime, posix-headers, dotnet-stdlib.

One ecosystem may serve several languages — Maven covers Java + Kotlin + Scala + Clojure + Groovy; npm covers JS + TS + Vue + Svelte. The `languages()` method declares capability, not assignment. Per-file language detection routes each walked file to the right plugin.

## The trait

`Ecosystem` (`mod.rs`) is the surface every implementation fills. The methods break into three groups:

**Identity and capability**
- `id()` — stable `EcosystemId` (used as registry key)
- `kind()` — `Package` (third-party) or `Stdlib` (language runtime)
- `languages()` — language ids appearing in this ecosystem's packages
- `manifest_specs()` — manifest filename globs + parsers this ecosystem owns
- `workspace_package_files()` / `workspace_package_extensions()` — package-root markers
- `pruned_dir_names()` — dep cache / build output dirs to skip during scans

**Activation and discovery**
- `activation()` — when does this ecosystem fire for a project (see "the activation rule" below)
- `locate_roots(ctx)` — discover on-disk dep roots; `ctx.manifests` carries the project's parsed manifests
- `walk_root(dep)` — yield WalkedFiles from a dep root (eager path)

**Reachability and demand**
- `resolve_import(dep, package, symbols)` — narrow walk for a specific import statement
- `resolve_symbol(dep, fqn)` — chain-walk step: pull the file defining one fqn
- `build_symbol_index(dep_roots)` — cheap header-only `(module, name) → file` map for Stage 2 demand
- `demand_pre_pull(dep_roots)` — bounded entry files to surface ahead of demand
- `parse_metadata_only(dep)` — for binary-only deps (NuGet DLLs, jmod files)
- `supports_reachability()` / `uses_demand_driven_parse()` — opt-in flags

The full trait signature lives in `mod.rs:336-546`. Read it once before starting.

## The activation rule

> **An ecosystem fires when the project declares it needs the ecosystem — not when the machine happens to have it installed.**

The `EcosystemActivation` enum (`mod.rs:261-300`) gives you the primitives:

| Variant | Use when |
|---|---|
| `ManifestMatch` | Project has a manifest this ecosystem parses (`Cargo.toml`, `package.json`, `pom.xml`, `vcpkg.json`, `CMakeLists.txt` with `find_package(Qt5)`). **The default for project deps.** |
| `ManifestFieldContains { glob, field_path, value }` | Finer grained: ecosystem fires only when a specific manifest field has a specific value. (`ts-lib-dom` activates iff `tsconfig.json.compilerOptions.lib` contains `"DOM"`.) |
| `LanguagePresent(id)` | Acceptable **only** for true implicit toolchain deps — the language runtime that every project in that language needs unconditionally (rust-stdlib for Rust, cpython-stdlib for Python, posix headers for unix C). |
| `AlwaysOnPlatform(platform)` | Same as `LanguagePresent` for platform-bound implicit toolchains. Use sparingly. |
| `TransitiveOn(other_id)` | This ecosystem is meaningful only when another is active (`android-sdk` requires Maven; `kotlin-stdlib` requires Maven or `.kt`). |
| `All(&[...])` / `Any(&[...])` | Composition. The right shape for stdlib-with-version-pin: `All([ManifestMatch, LanguagePresent("c")])`. |
| `Always`, `Never` | Test fixtures and explicit disable. Don't use in production. |

### Project deps must use `ManifestMatch`

Anything that is *not* a language's built-in toolchain — Qt, Boost, OpenSSL, vcpkg packages, Conan packages, .NET NuGet, Maven artefacts, Gradle deps — is a project dep. Project deps activate via `ManifestMatch` (or `ManifestFieldContains`), reading from `ctx.manifests` to learn what version + components the project declared.

### Implicit toolchains may use `LanguagePresent`

The narrow exception is the *language's* substrate that every project in that language needs:

- `rust-stdlib` — every `.rs` file uses the prelude. `LanguagePresent("rust")` is correct.
- `cpython-stdlib` — every `.py` file imports stdlib transitively. `LanguagePresent("python")` is correct.
- `posix-headers` (POSIX side) — every `.c`/`.cpp` file on Linux uses `/usr/include`. Correct.

The test for whether `LanguagePresent` is appropriate: **does every project in this language need this dep, regardless of project intent?** If yes, it's a substrate. If no — it's a project dep, and `LanguagePresent` is wrong.

## Anti-pattern: probe-and-pray

The forbidden shape:

```rust
fn activation(&self) -> EcosystemActivation {
    EcosystemActivation::Any(&[
        EcosystemActivation::LanguagePresent("c"),
        EcosystemActivation::LanguagePresent("cpp"),
    ])
}

fn locate_roots(&self, _ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
    // Probe a hardcoded list of machine paths, return whichever exist.
    discover_qt_install_on_machine()
}
```

Three concrete failures this produces:

1. **Wrong activation.** Fires on every C/C++ project even when the project has nothing to do with this dep. Inflates work, blurs metrics.
2. **Wrong version.** Returns whatever install happens to be on the machine. A project pinning Qt 5.15 silently resolves against Qt 6.7 if that's what's installed.
3. **Silent fallthrough.** No diagnostic when the declared dep can't be found. The user sees "low resolution rate" with no hint that a missing SDK is the cause.

The correct shape uses `ctx.manifests` to read what the project declared, and emits a diagnostic when the install can't be matched:

```rust
fn activation(&self) -> EcosystemActivation {
    EcosystemActivation::ManifestMatch
}

fn locate_roots(&self, ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
    let required = read_qt_requirement_from_manifests(ctx.manifests);
    let Some(req) = required else { return Vec::new() };
    match find_install_matching(req) {
        Some(root) => vec![root],
        None => {
            tracing::warn!(
                "qt-runtime: project declares Qt {} but no matching install found",
                req.version
            );
            Vec::new()
        }
    }
}
```

## Suppression precedence

When several signals could drive an ecosystem, the order is:

1. **`compile_commands.json` is hard supremacy.** It lists every `-I` path the build system actually uses. When present, no other heuristic for that ecosystem fires (`compile_commands.rs` already implements this for the C/C++ side; replicate the pattern when adding similar build-system outputs).
2. **Build manifest with explicit dep** (`vcpkg.json`, `find_package(Qt5)` in CMakeLists, `<PackageReference>` in `.csproj`, `[dependencies]` in `Cargo.toml`). Drives version + component selection.
3. **Implicit-toolchain `LanguagePresent`** — only for true substrates (rust-stdlib, cpython-stdlib, posix libc).
4. **Probe fallback** — last resort, only when zero project config is detectable. Must emit a diagnostic so the user knows the resolution is speculative.

## Known limitation: workspace-flat activation

Activation runs once per project and produces a single workspace-wide `active_ecosystems` set. `ManifestMatch` queries the unioned `ctx.manifests` map; `ManifestFieldContains` walks the entire `project_root`. Per-package activation does not exist.

Consequences for polyglot monorepos and microservice repos:

- An ecosystem activates for the whole workspace when only one package declares it. Indexing happens on packages that don't need it — wasted work, not wrong edges.
- Two packages that pin different versions of the same SDK collapse: the walker picks one install, and refs in the other package resolve against the wrong version.
- `ManifestFieldContains` can fire for a field declared in one package and apply across the workspace (a frontend `tsconfig.json` declaring `"DOM"` activates `ts_lib_dom` for an unrelated backend package in the same repo).

`ProjectContext::by_package` exists for the per-package case. Ecosystems that care about per-package precision can do per-package gating inside `locate_roots` — the escape hatch. The activation phase itself does not yet take a `PackageId`.

For the in-flight refactor, treat workspace-flat activation as the contract. Document any per-package precision your `locate_roots` adds. Don't attempt to fix the activation phase locally; that's a separate workstream.

## Locators only — no synthetics, no predicates

Files in `ecosystem/` discover external dependencies on disk and tell the indexer where to find them. They do NOT:

- Hard-code library symbol lists ("jQuery's $, vitest's describe, …").
- Provide synthetic stubs to plug holes ("we'll just hand-write a fake `<assert.h>`").
- Embed `is_*_builtin` predicates that decide whether a name is "internal" or "external".

If a project needs a runtime API that isn't on disk, the answer is a manifest reader pulling that dep from a real source — not a locator file pretending the API exists. The `feedback_ecosystem_files_are_locators_only` memory captures this; the no-synthetics rule is hard.

## Adding a new ecosystem

1. **Confirm it isn't a violator-in-disguise.** If you're tempted to write `LanguagePresent` activation + machine-path probing, stop and read "the activation rule" again. The right answer is almost always a manifest reader.
2. **Decide the manifest source.** What file in a project declares this ecosystem? `Cargo.toml`? `vcpkg.json`? An XML element in `*.csproj`? If no manifest exists in any common build system for this ecosystem — strongly reconsider whether the ecosystem is real or a probe-and-pray in waiting.
3. **Add the manifest reader** if it doesn't exist. Place it in `ecosystem/manifest/` (shared parsers) or inline in the ecosystem file (single-ecosystem readers). Implement `ManifestReader` and add a variant to `ManifestKind`.
4. **Implement `Ecosystem`** in `ecosystem/<name>.rs`. Use `ManifestMatch` activation. Read `ctx.manifests` in `locate_roots` to learn version + components. Walk only the matching install. Diagnose missing-install with `tracing::warn!` and an empty result.
5. **Tests in a sibling `<name>_tests.rs` file** (per the project-wide rule). Cover: matched, version-mismatch, missing-install, multiple-installs-on-machine.
6. **Register in `default_registry()`** in `mod.rs`.

Per the `feedback_no_walkers_without_discussion` rule: surface the design at architect-review level before implementing — most "I need a new walker" requests turn out to be either (a) extending an existing manifest reader, or (b) a violator pattern that shouldn't exist in the first place.

## Worked examples to mirror

Manifest-driven ecosystems to read for the canonical pattern:

- `cargo.rs` — `ManifestMatch` on `Cargo.toml`; `locate_roots` consumes manifest data.
- `npm.rs` — `ManifestMatch` on `package.json`.
- `cabal.rs` — `ManifestMatch` on `*.cabal`.
- `composer.rs` — `ManifestMatch` on `composer.json`.
- `compile_commands.rs` — fires only when `compile_commands.json` is found in `locate_roots`; suppresses heuristic walkers for the same languages.

If you find an existing ecosystem whose `activation()` is `LanguagePresent` and whose `locate_roots` ignores `ctx.manifests` while probing hardcoded machine paths, that's a violator and it should be converted before adding adjacent functionality.

## When this guide and the trait disagree

The trait signature in `mod.rs` is the source of truth. If you find a discrepancy between this doc and the code, trust the code and update the doc in the same change.
