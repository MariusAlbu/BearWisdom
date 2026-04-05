use crate::languages::*;
use crate::types::LanguageDescriptor;

/// All registered language descriptors. Order is irrelevant — detection is by
/// extension/filename lookup, not array index.
pub static LANGUAGES: &[&LanguageDescriptor] = &[
    &RUST,
    &TYPESCRIPT,
    &JAVASCRIPT,
    &CSHARP,
    &PYTHON,
    &GO,
    &JAVA,
    &KOTLIN,
    &SWIFT,
    &RUBY,
    &PHP,
    &C,
    &CPP,
    &DART,
    &SCALA,
    &ELIXIR,
    &LUA,
    &HASKELL,
    &R,
    &HTML,
    &CSS,
    &SCSS,
    &JSON,
    &YAML,
    &XML,
    &MARKDOWN,
    &SQL,
    &SHELL,
    &DOCKERFILE,
    &TOML,
    // Extended language set — detected by walker, parsed by tree-sitter plugins.
    &POWERSHELL,
    &GROOVY,
    &ERLANG,
    &FSHARP,
    &GDSCRIPT,
    &VBNET,
    &NIM,
    &GLEAM,
    &NIX,
    &HCL,
    &PUPPET,
    &STARLARK,
    &PROTO,
    &GRAPHQL,
    &PRISMA,
    &BICEP,
    &CMAKE,
    &ADA,
    &FORTRAN,
    &PASCAL,
    &COBOL,
    &CLOJURE,
    &OCAML,
    &SVELTE,
    &ASTRO,
    &MATLAB,
    &PERL,
    &ODIN,
    &ZIG,
    &PROLOG,
    &VBA,
    &ROBOT,
    &MAKE,
    &HARE,
];

/// Find a language descriptor by its stable id (e.g. "rust", "typescript").
pub fn find_language(id: &str) -> Option<&'static LanguageDescriptor> {
    LANGUAGES.iter().copied().find(|l| l.id == id || l.aliases.contains(&id))
}

/// Find a language descriptor by a file extension (with leading dot, e.g. ".rs").
pub fn find_language_by_extension(ext: &str) -> Option<&'static LanguageDescriptor> {
    // Normalise: ensure leading dot for comparison.
    let ext = if ext.starts_with('.') {
        ext.to_owned()
    } else {
        format!(".{ext}")
    };
    LANGUAGES
        .iter()
        .copied()
        .find(|l| l.file_extensions.contains(&ext.as_str()))
}
