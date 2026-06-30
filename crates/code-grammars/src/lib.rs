// =============================================================================
// code-grammars — language-id → tree-sitter grammar registry
//
// Maps a language identifier string to a tree-sitter Language value that can be
// passed to Parser::set_language. Shared between BearWisdom's indexer and editor
// frontends that need the same grammar stack for syntax highlighting.
//
// API compatibility notes:
//   - Modern grammar crates (tree-sitter 0.22+) expose a `LANGUAGE: LanguageFn`
//     constant.  Call `LANGUAGE.into()` to get a `tree_sitter::Language`.
//   - Older grammar crates (tree-sitter 0.19/0.20 era) expose a `fn language()`
//     that returns a `tree_sitter::Language` from a *different* tree-sitter
//     semver.  These types are not compatible with the 0.25 `Language` type and
//     cannot be used directly without unsafe transmutation.
//
//   Crates excluded due to old ABI (kept compilable in Cargo.toml, not wired in):
//     - tree-sitter-kotlin 0.3.5, tree-sitter-markdown 0.7.1,
//       tree-sitter-dockerfile 0.2, tree-sitter-prisma 0.1.1, tree-sitter-hare 0.20.7
// =============================================================================

use std::borrow::Cow;

use tree_sitter::Language;

/// Return the tree-sitter [`Language`] for the given language identifier.
///
/// Returns `None` if the language is known but its grammar is not available
/// in this build (e.g. excluded old-ABI crates).
pub fn get_language(lang: &str) -> Option<Language> {
    let l: Language = match lang {
        // ---- C# and TypeScript -------------------------------------------------
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

        // ---- Dockerfile (via local wrapper crate) ------------------------------
        "dockerfile" => tree_sitter_dockerfile_0_25::LANGUAGE.into(),

        // ---- Newly wired grammars ----------------------------------------------
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

        // ---- SO 2025 top languages ---------------------------------------------
        "powershell" => tree_sitter_powershell::LANGUAGE.into(),
        "groovy" => tree_sitter_groovy::LANGUAGE.into(),
        "erlang" => tree_sitter_erlang::LANGUAGE.into(),
        "fsharp" => tree_sitter_fsharp::LANGUAGE_FSHARP.into(),
        "gdscript" => tree_sitter_gdscript::LANGUAGE.into(),

        // ---- Pascal/Delphi -----------------------------------------------------
        "pascal" | "delphi" => tree_sitter_pascal::LANGUAGE.into(),

        // ---- SO 2025 survey languages ------------------------------------------
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

/// Return the bundled tree-sitter highlights query for `lang`, if available.
///
/// Returns `None` for unknown ids and for grammars whose crate ships no highlights
/// query (or keeps the constant commented out for an old ABI). Pairs with
/// [`get_language`] over the same id space: build a highlighter from the
/// `(Language, query)` pair.
///
/// Some grammars layer their highlights on a base language (TypeScript on JavaScript,
/// C++ on C); for those the base and specific queries are concatenated, base first, so
/// the more-specific patterns take precedence. The standalone case borrows, no alloc.
pub fn highlights_query(lang: &str) -> Option<Cow<'static, str>> {
    match lang {
        "typescript" | "tsx" => Some(Cow::Owned(format!(
            "{}\n{}",
            tree_sitter_javascript::HIGHLIGHT_QUERY,
            tree_sitter_typescript::HIGHLIGHTS_QUERY,
        ))),
        "cpp" => Some(Cow::Owned(format!(
            "{}\n{}",
            tree_sitter_c::HIGHLIGHT_QUERY,
            tree_sitter_cpp::HIGHLIGHT_QUERY,
        ))),
        _ => single_highlights_query(lang).map(Cow::Borrowed),
    }
}

/// The standalone highlights query a single grammar crate exposes. The constant's name
/// differs across crates (`HIGHLIGHTS_QUERY` vs `HIGHLIGHT_QUERY`); this normalizes it.
fn single_highlights_query(lang: &str) -> Option<&'static str> {
    let q = match lang {
        // NOTE: tree-sitter-c-sharp 0.23.1 (the locked version) ships its highlights
        // query commented out; "csharp" stays None until the grammar is bumped.
        "javascript" | "jsx" => tree_sitter_javascript::HIGHLIGHT_QUERY,
        "python" => tree_sitter_python::HIGHLIGHTS_QUERY,
        "java" => tree_sitter_java::HIGHLIGHTS_QUERY,
        "go" => tree_sitter_go::HIGHLIGHTS_QUERY,
        "rust" => tree_sitter_rust::HIGHLIGHTS_QUERY,
        "ruby" => tree_sitter_ruby::HIGHLIGHTS_QUERY,
        "php" => tree_sitter_php::HIGHLIGHTS_QUERY,
        "c" => tree_sitter_c::HIGHLIGHT_QUERY,
        "swift" => tree_sitter_swift::HIGHLIGHTS_QUERY,
        "scala" => tree_sitter_scala::HIGHLIGHTS_QUERY,
        "haskell" => tree_sitter_haskell::HIGHLIGHTS_QUERY,
        "elixir" => tree_sitter_elixir::HIGHLIGHTS_QUERY,
        "dart" => tree_sitter_dart::HIGHLIGHTS_QUERY,
        "lua" => tree_sitter_lua::HIGHLIGHTS_QUERY,
        "r" => tree_sitter_r::HIGHLIGHTS_QUERY,
        "html" => tree_sitter_html::HIGHLIGHTS_QUERY,
        "css" | "scss" => tree_sitter_css::HIGHLIGHTS_QUERY,
        "json" => tree_sitter_json::HIGHLIGHTS_QUERY,
        "yaml" => tree_sitter_yaml::HIGHLIGHTS_QUERY,
        "shell" | "bash" => tree_sitter_bash::HIGHLIGHT_QUERY,
        "sql" => tree_sitter_sequel::HIGHLIGHTS_QUERY,
        "zig" => tree_sitter_zig::HIGHLIGHTS_QUERY,
        "nix" => tree_sitter_nix::HIGHLIGHTS_QUERY,
        "powershell" => tree_sitter_powershell::HIGHLIGHTS_QUERY,
        "starlark" => tree_sitter_starlark::HIGHLIGHTS_QUERY,
        "odin" => tree_sitter_odin::HIGHLIGHTS_QUERY,
        "erlang" => tree_sitter_erlang::HIGHLIGHTS_QUERY,
        "fsharp" => tree_sitter_fsharp::HIGHLIGHTS_QUERY,
        "gleam" => tree_sitter_gleam::HIGHLIGHT_QUERY,
        "bicep" => tree_sitter_bicep::HIGHLIGHTS_QUERY,
        "make" => tree_sitter_make::HIGHLIGHTS_QUERY,
        "ocaml" => tree_sitter_ocaml::HIGHLIGHTS_QUERY,
        _ => return None,
    };
    Some(q)
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
