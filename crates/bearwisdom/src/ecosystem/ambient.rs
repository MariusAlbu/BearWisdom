// =============================================================================
// ecosystem/ambient.rs — framework ambient-declaration path markers
//
// Build-tool / framework generated files and runtime declaration files whose
// exports the framework's compiler treats as ambient (available in user code
// without an explicit `import`): Nuxt/`unplugin-*` auto-import declarations,
// SvelteKit `$app`/`$env` ambient types, Next.js env types, and the Vue 3
// runtime declarations the SFC compiler injects (`Transition`, `defineProps`,
// …).
//
// These are *disk-location markers* (like `Ecosystem::pruned_dir_names`), not
// symbol lists — they say "files matching this shape are ambient providers",
// and they self-gate by existence (`node_modules/vue/dist/*.d.ts` only exists
// when Vue is installed). Kept here, in the ecosystem layer that owns external
// on-disk discovery, rather than hardcoded inside the generic resolver's
// classification path.
//
// `ambient_global_qnames` / `locate_ambient_global` are the engine-facing
// surface: the resolve engine hands a parsed batch here and gets back a plain
// qualified-name set for its `ambient_scope`, so the generic store never
// matches an ecosystem path or names a synthetic module key itself.
// =============================================================================

use std::collections::HashSet;
use std::path::Path;

use crate::ecosystem::symbol_index::SymbolLocationIndex;
use crate::types::ParsedFile;

/// A path is an ambient provider when it contains `contains` AND ends with
/// `ends_with`. An empty `contains` matches any path (suffix-only rule).
pub struct AmbientPathMarker {
    pub contains: &'static str,
    pub ends_with: &'static str,
}

impl AmbientPathMarker {
    /// `path` is expected pre-lowercased with `\` normalised to `/`.
    pub fn matches(&self, normalized_lower_path: &str) -> bool {
        normalized_lower_path.contains(self.contains)
            && normalized_lower_path.ends_with(self.ends_with)
    }
}

/// Framework-generated and runtime ambient declaration markers. Matched
/// against a candidate file path to decide whether its symbols are ambient.
pub const FRAMEWORK_AMBIENT_MARKERS: &[AmbientPathMarker] = &[
    // Build-tool auto-import declarations (nuxt prepare, unplugin-*).
    AmbientPathMarker {
        contains: "/.nuxt/",
        ends_with: "imports.d.ts",
    },
    AmbientPathMarker {
        contains: "/.nuxt/",
        ends_with: "components.d.ts",
    },
    AmbientPathMarker {
        contains: "/.svelte-kit/",
        ends_with: "ambient.d.ts",
    },
    AmbientPathMarker {
        contains: "/.next/",
        ends_with: "next-env.d.ts",
    },
    // No-leading-slash forms for project-root-relative DB paths.
    AmbientPathMarker {
        contains: "",
        ends_with: ".nuxt/imports.d.ts",
    },
    AmbientPathMarker {
        contains: "",
        ends_with: ".nuxt/components.d.ts",
    },
    AmbientPathMarker {
        contains: "",
        ends_with: ".svelte-kit/ambient.d.ts",
    },
    AmbientPathMarker {
        contains: "",
        ends_with: ".next/next-env.d.ts",
    },
    // Vue 3 runtime declarations whose exports the SFC compiler injects.
    AmbientPathMarker {
        contains: "node_modules/vue/dist/",
        ends_with: ".d.ts",
    },
    AmbientPathMarker {
        contains: "node_modules/@vue/runtime-core/dist/",
        ends_with: ".d.ts",
    },
    AmbientPathMarker {
        contains: "node_modules/@vue/runtime-dom/dist/",
        ends_with: ".d.ts",
    },
    AmbientPathMarker {
        contains: "node_modules/@vue/reactivity/dist/",
        ends_with: ".d.ts",
    },
    // Bicep runtime grammar symbols (built-in functions, decorators, the
    // `sys`/`az` namespace markers) the language compiler treats as ambient.
    AmbientPathMarker {
        contains: "ext:bicep-runtime:",
        ends_with: ".bicep",
    },
    // Bazel built-in rules and the `ctx` / `env` API namespaces, available in
    // BUILD/.bzl files without an explicit `load()`.
    AmbientPathMarker {
        contains: "ext:bazel-builtins:",
        ends_with: ".bzl",
    },
    // Rust prelude: every module gets `std::prelude::v1` (`Vec`, `Box`, `Some`,
    // `Result`, `Default`, `From`, …) without a `use`. The stdlib walker keys
    // these under the sysroot source tree `…/rustlib/src/rust/library/<crate>/…`.
    // Scoped to that subtree so cargo-registry crates (also `ext:rust:`, but
    // under `/registry/src/`) stay non-ambient — they need an explicit `use`.
    AmbientPathMarker {
        contains: "/rustlib/src/rust/library/",
        ends_with: ".rs",
    },
    // Dart implicit import: every library gets `dart:core` (`String`, `List`,
    // `Map`, `Exception`, `ArgumentError`, …) without an `import`. The dart-sdk
    // and Flutter's bundled sky_engine ship it under `…/lib/core/`. Scoped to
    // that directory so the other `dart:` libraries (`dart:async`,
    // `dart:collection`, under `…/lib/async|collection/`) stay non-ambient —
    // they require an explicit import. dart:core types are stored with a bare
    // qualified name, so the path is the only namespace signal.
    AmbientPathMarker {
        contains: "/dart-sdk/lib/core/",
        ends_with: ".dart",
    },
    AmbientPathMarker {
        contains: "/sky_engine/lib/core/",
        ends_with: ".dart",
    },
    // Haskell Prelude: every module implicitly imports `Prelude` (`Just`,
    // `maybe`, `mapM`, `fromMaybe`, `fmap`, …) without an `import`. GHC ships
    // these under the `ghc-internal` package's `GHC/Internal/` source tree.
    // Prelude symbols are stored with a bare qualified name; the path scopes the
    // bind to GHC's base and keeps same-named third-party symbols out.
    AmbientPathMarker {
        contains: "/ghc/internal/",
        ends_with: ".hs",
    },
];

/// True when `normalized_lower_path` (pre-lowercased, `/`-normalised) matches
/// any framework ambient marker.
pub fn is_framework_ambient_path(normalized_lower_path: &str) -> bool {
    FRAMEWORK_AMBIENT_MARKERS
        .iter()
        .any(|m| m.matches(normalized_lower_path))
}

/// Qualified names of the ambient globals a parsed batch contributes. The
/// resolve engine indexes the matching symbols into its `ambient_scope` keyed by
/// simple name, then binds bare references to them — without itself knowing any
/// ecosystem convention.
///
/// Two structural sources: a symbol under the npm synthetic-globals module
/// (`declare global` / test-runner globals), and a top-level declaration in an
/// ambient-global lib source (TS `lib.*.d.ts` / `@types` globals, a language
/// `<lang>-stdlib` runtime).
pub fn ambient_global_qnames(parsed: &[ParsedFile]) -> HashSet<String> {
    let globals_prefix = format!("{}.", crate::ecosystem::npm::NPM_GLOBALS_MODULE);
    let mut out = HashSet::new();
    for pf in parsed {
        let lib_source = is_ambient_global_lib_path(&pf.path);
        for sym in &pf.symbols {
            let qname = &sym.qualified_name;
            if qname.starts_with(&globals_prefix) {
                out.insert(qname.clone());
            } else if lib_source && !qname.contains('.') {
                // Top-level declaration in an ambient lib source; nested members
                // (dotted qnames) are not globals.
                out.insert(qname.clone());
            }
        }
    }
    out
}

/// Locate the file defining an import-free ambient global named `name`, for
/// demand-driven materialization of a bare reference. Probes the npm synthetic
/// globals module, where `declare global` / test-runner globals are registered.
pub fn locate_ambient_global<'a>(loc: &'a SymbolLocationIndex, name: &str) -> Option<&'a Path> {
    loc.locate(crate::ecosystem::npm::NPM_GLOBALS_MODULE, name)
}

/// Detect an ambient-global declaration file — a runtime surface a project can
/// name without an explicit import. Two shapes qualify:
///
/// - TypeScript's `lib.*.d.ts` / `@types/node` (`is_ts_ambient_global_lib_path`).
/// - A language stdlib whose symbols carry a `<lang>-stdlib`-tagged external
///   path. These libraries are language substrate — every project in the
///   language reaches their names unqualified-by-import (Lua's `string`,
///   `table`, `math`, `os`, …).
fn is_ambient_global_lib_path(path: &str) -> bool {
    let normalized = path.replace('\\', "/");
    is_ts_ambient_global_lib_path(&normalized) || is_stdlib_external_path(&normalized)
}

/// Detect a TypeScript ambient-global declaration file — `lib.*.d.ts` shipped
/// with the TypeScript compiler, or any file under `@types/node/`. Methods
/// declared in these files are the JS/DOM/ES runtime surface and need no
/// explicit import to call.
///
/// Recognises both the historical absolute-path form
/// (`.../typescript/lib/lib.dom.d.ts`) and the synthetic-module form emitted
/// post-Pass-A (`ext:ts:__ts_lib__/lib.dom.d.ts`,
/// `ext:ts:@types/node/process.d.ts`). The substring matchers stay so older
/// indexes built before the path rewrite still classify correctly. The input
/// is already forward-slash normalised.
fn is_ts_ambient_global_lib_path(normalized: &str) -> bool {
    let synthetic_prefix = format!(
        "ext:ts:{}/",
        crate::ecosystem::ts_lib_dom::TS_LIB_SYNTHETIC_MODULE
    );
    normalized.starts_with(&synthetic_prefix)
        || normalized.starts_with("ext:ts:@types/node/")
        || normalized.contains("/typescript/lib/lib.")
        || normalized.contains("/@types/node/")
}

/// True for a language-stdlib external path — `ext:<lang>-stdlib:...`. The
/// stdlib ecosystems tag their synthetic file path with the `<lang>-stdlib`
/// ecosystem id, so an `ext:` external whose ecosystem segment ends in
/// `-stdlib` is the language's runtime substrate.
fn is_stdlib_external_path(normalized: &str) -> bool {
    let Some(rest) = normalized.strip_prefix("ext:") else {
        return false;
    };
    let Some((ecosystem, _)) = rest.split_once(':') else {
        return false;
    };
    ecosystem.ends_with("-stdlib")
}

#[cfg(test)]
#[path = "ambient_tests.rs"]
mod tests;
