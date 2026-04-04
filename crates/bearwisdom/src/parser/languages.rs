// =============================================================================
// parser/languages.rs  —  grammar loader
//
// Maps a language identifier string (as produced by walker::detect_language)
// to a tree-sitter Language value that can be passed to Parser::set_language.
//
// API compatibility notes:
//   - Modern grammar crates (tree-sitter 0.22+) expose a `LANGUAGE: LanguageFn`
//     constant.  Call `LANGUAGE.into()` to get a `tree_sitter::Language`.
//   - Older grammar crates (tree-sitter 0.19/0.20 era) expose a `fn language()`
//     that returns a `tree_sitter::Language` from a *different* tree-sitter
//     semver.  These types are not compatible with the 0.25 `Language` type and
//     cannot be used directly without unsafe transmutation.
//
//   Crates currently excluded due to old ABI:
//     - tree-sitter-kotlin 0.3.5   (TODO: bump to a 0.23+ release when available)
//     - tree-sitter-markdown 0.7.1 (TODO: bump to a 0.23+ release when available)
//     - tree-sitter-dockerfile 0.2 (TODO: bump to a 0.23+ release when available)
//     - tree-sitter-prisma 0.1.1   (TODO: requires >= 0.19, < 0.21 — bump when available)
//     - tree-sitter-hare 0.20.7    (TODO: requires 0.20.6 — bump when available)
//
//   These crates are still listed in Cargo.toml so they can be compiled against;
//   they are simply not wired into get_language() until a compatible version
//   is published.
// =============================================================================

use tree_sitter::Language;

/// Return the tree-sitter [`Language`] for the given language identifier.
///
/// Returns `None` if the language is known but its grammar is not available
/// in this build (e.g. excluded old-ABI crates).
pub fn get_language(lang: &str) -> Option<Language> {
    let l: Language = match lang {
        // ---- C# and TypeScript (pre-existing) --------------------------------
        "csharp" => tree_sitter_c_sharp::LANGUAGE.into(),
        "typescript" => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        "tsx" => tree_sitter_typescript::LANGUAGE_TSX.into(),

        // ---- JavaScript (separate from typescript crate) ----------------------
        "javascript" | "jsx" => tree_sitter_javascript::LANGUAGE.into(),

        // ---- Compiled/systems languages ----------------------------------------
        "python" => tree_sitter_python::LANGUAGE.into(),
        "java" => tree_sitter_java::LANGUAGE.into(),
        "go" => tree_sitter_go::LANGUAGE.into(),
        "rust" => tree_sitter_rust::LANGUAGE.into(),
        "ruby" => tree_sitter_ruby::LANGUAGE.into(),
        "php" => tree_sitter_php::LANGUAGE_PHP.into(),
        "cpp" => tree_sitter_cpp::LANGUAGE.into(),
        "c" => tree_sitter_c::LANGUAGE.into(),
        "swift" => tree_sitter_swift::LANGUAGE.into(),
        "scala" => tree_sitter_scala::LANGUAGE.into(),
        "haskell" => tree_sitter_haskell::LANGUAGE.into(),
        "elixir" => tree_sitter_elixir::LANGUAGE.into(),
        "dart" => tree_sitter_dart::LANGUAGE.into(),
        "lua" => tree_sitter_lua::LANGUAGE.into(),
        "r" => tree_sitter_r::LANGUAGE.into(),

        // ---- Web / markup / data -----------------------------------------------
        "html" => tree_sitter_html::LANGUAGE.into(),
        "css" | "scss" => tree_sitter_css::LANGUAGE.into(),
        "json" => tree_sitter_json::LANGUAGE.into(),
        "yaml" => tree_sitter_yaml::LANGUAGE.into(),
        "xml" => tree_sitter_xml::LANGUAGE_XML.into(),

        // ---- Shell / scripting -------------------------------------------------
        "shell" | "bash" => tree_sitter_bash::LANGUAGE.into(),

        // ---- SQL ---------------------------------------------------------------
        "sql" => tree_sitter_sequel::LANGUAGE.into(),

        // ---- Kotlin (via tree-sitter-kotlin-ng) --------------------------------
        "kotlin" => tree_sitter_kotlin_ng::LANGUAGE.into(),

        // ---- Markdown (via tree-sitter-md) ------------------------------------
        "markdown" => tree_sitter_md::LANGUAGE.into(),

        // ---- Dockerfile (via local wrapper crate) ---------------------------------
        "dockerfile" => tree_sitter_dockerfile_0_25::LANGUAGE.into(),

        // ---- Newly wired grammars -----------------------------------------------
        "graphql" => tree_sitter_graphql::LANGUAGE.into(),
        "hcl" | "terraform" => tree_sitter_hcl::LANGUAGE.into(),
        "proto" => tree_sitter_proto::LANGUAGE.into(),
        "nix" => tree_sitter_nix::LANGUAGE.into(),
        "zig" => tree_sitter_zig::LANGUAGE.into(),
        "cmake" => tree_sitter_cmake::LANGUAGE.into(),
        "make" => tree_sitter_make::LANGUAGE.into(),
        "gleam" => tree_sitter_gleam::LANGUAGE.into(),
        "bicep" => tree_sitter_bicep::LANGUAGE.into(),
        "odin" => tree_sitter_odin::LANGUAGE.into(),
        "starlark" => tree_sitter_starlark::LANGUAGE.into(),
        // ---- Wave 7: SO 2025 top languages ------------------------------------
        "powershell" => tree_sitter_powershell::LANGUAGE.into(),
        "groovy" => tree_sitter_groovy::LANGUAGE.into(),
        // "perl" — excluded: tree-sitter-perl 1.1 has hard-dep on tree-sitter 0.26
        "erlang" => tree_sitter_erlang::LANGUAGE.into(),
        "fsharp" => tree_sitter_fsharp::LANGUAGE_FSHARP.into(),
        "gdscript" => tree_sitter_gdscript::LANGUAGE.into(),
        // prisma (tree-sitter-prisma 0.1.1) and hare (tree-sitter-hare 0.20.7) are
        // excluded: both require old tree-sitter ABI (< 0.21 / 0.20.6).
        // TODO: wire in when updated crates are available on crates.io.

        // ---- Wave 3: Pascal/Delphi -----------------------------------------------
        "pascal" | "delphi" => tree_sitter_pascal::LANGUAGE.into(),
        // cobol (tree-sitter-cobol 0.1.0) is a stub binary crate — no LANGUAGE export.
        // vba and prolog: no tree-sitter grammar crates available; both use line scanners.

        // ---- Wave 2: SO 2025 survey languages -----------------------------------
        "vbnet" => tree_sitter_vb_dotnet::LANGUAGE.into(),
        "matlab" => tree_sitter_matlab::LANGUAGE.into(),
        "clojure" => tree_sitter_clojure::LANGUAGE.into(),
        "ocaml" => tree_sitter_ocaml::LANGUAGE_OCAML.into(),
        "ada" => tree_sitter_ada::LANGUAGE.into(),
        "fortran" => tree_sitter_fortran::LANGUAGE.into(),

        _ => return None,
    };
    Some(l)
}

/// Returns `true` if the language has a full dedicated symbol extractor,
/// not just grammar-based generic extraction.
///
/// Used by the indexer to decide whether to run a specialised extractor or
/// fall back to the generic DFS walker.
pub fn has_extractor(lang: &str) -> bool {
    matches!(
        lang,
        "csharp" | "typescript" | "tsx" | "rust" | "python" | "go" | "java"
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "languages_tests.rs"]
mod tests;
