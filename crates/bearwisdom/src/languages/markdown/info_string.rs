//! Fenced-code info-string normalization.
//!
//! Maps the fuzzy info-string that follows a fence opener (`` ```ts ``,
//! `` ```python3 ``, `` ```rust,no_run ``, `` ```{r, echo=FALSE} ``) to
//! a canonical language id understood by the plugin registry, or `None`
//! when the info-string doesn't name a code language we can parse.
//!
//! Recognized forms:
//!
//! * Bare: `ts`, `python3`, `sh`
//! * Comma-separated modifiers: `rust,no_run`, `rust,ignore`
//! * Attribute syntax (Pandoc): `{.rust #anchor}`, `{.python}`
//! * Chunk syntax (RMarkdown/Quarto): `{r}`, `{python}`, `{r, echo=FALSE}`
//!
//! Unknown strings return `None` — the fence is skipped rather than
//! dispatched to the generic plugin (that produces noise for `mermaid`,
//! `text`, `plantuml`, `console`, etc. that aren't code).

/// Normalize a raw info-string to a language id. Returns `None` when
/// the info-string is empty, names a non-code format (mermaid, diff,
/// text), or is otherwise unrecognized.
pub fn normalize(info: &str) -> Option<&'static str> {
    let head = first_identifier(info)?;
    let head_lc = head.to_ascii_lowercase();
    match head_lc.as_str() {
        "ts" | "typescript" | "tsx" => Some("typescript"),
        "js" | "javascript" | "jsx" | "mjs" | "cjs" | "node" => Some("javascript"),
        "py" | "python" | "python3" | "python2" | "py3" => Some("python"),
        "rs" | "rust" => Some("rust"),
        "sh" | "bash" | "zsh" | "shell" | "console" | "shellsession" => Some("bash"),
        "ps" | "ps1" | "powershell" | "pwsh" => Some("powershell"),
        "cs" | "csharp" | "c#" => Some("csharp"),
        "fs" | "fsharp" | "f#" => Some("fsharp"),
        "vb" | "vbnet" | "vb.net" => Some("vbnet"),
        "sql" | "psql" | "sqlite" | "mysql" | "postgresql" | "postgres" | "tsql" => Some("sql"),
        "yaml" | "yml" => Some("yaml"),
        "toml" => Some("toml"),
        "dockerfile" | "docker" => Some("dockerfile"),
        "html" | "htm" => Some("html"),
        "css" => Some("css"),
        "scss" | "sass" => Some("scss"),
        "less" => Some("css"),
        "json" | "jsonc" | "json5" => Some("json"),
        "xml" => Some("xml"),
        "go" | "golang" => Some("go"),
        "java" => Some("java"),
        "kt" | "kotlin" => Some("kotlin"),
        "scala" => Some("scala"),
        "swift" => Some("swift"),
        "rb" | "ruby" => Some("ruby"),
        "php" => Some("php"),
        "c" => Some("c"),
        "cpp" | "c++" | "cxx" | "hpp" => Some("cpp"),
        "dart" => Some("dart"),
        "lua" => Some("lua"),
        "r" | "rlang" => Some("r"),
        "elixir" | "ex" | "exs" => Some("elixir"),
        "erlang" | "erl" => Some("erlang"),
        "haskell" | "hs" => Some("haskell"),
        "ocaml" | "ml" => Some("ocaml"),
        "zig" => Some("zig"),
        "nim" => Some("nim"),
        "nix" => Some("nix"),
        "graphql" | "gql" => Some("graphql"),
        "proto" | "protobuf" => Some("proto"),
        "hcl" | "terraform" | "tf" => Some("hcl"),
        "cmake" => Some("cmake"),
        "make" | "makefile" => Some("make"),
        "bicep" => Some("bicep"),
        "groovy" => Some("groovy"),
        "gleam" => Some("gleam"),
        "perl" | "pl" => Some("perl"),
        "ada" | "adb" | "ads" => Some("ada"),
        "fortran" | "f90" | "f95" => Some("fortran"),
        "matlab" => Some("matlab"),
        "clojure" | "clj" | "cljs" => Some("clojure"),
        "cobol" | "cbl" => Some("cobol"),
        "pascal" | "pas" | "delphi" => Some("pascal"),
        "prolog" => Some("prolog"),
        "starlark" | "bzl" | "bazel" => Some("starlark"),
        "odin" => Some("odin"),
        "gdscript" | "gd" => Some("gdscript"),
        "vba" => Some("vba"),
        _ => None,
    }
}

/// Pull the first identifier out of a raw info-string. Strips Pandoc
/// attribute braces (`{.rust #anchor}`), notebook chunk options
/// (`{r, echo=FALSE}`), and comma-separated modifiers (`rust,no_run`).
fn first_identifier(info: &str) -> Option<&str> {
    let info = info.trim();
    if info.is_empty() {
        return None;
    }
    // {.rust #anchor}  OR  {r, echo=FALSE}
    let body = info
        .strip_prefix('{')
        .and_then(|s| s.strip_suffix('}'))
        .unwrap_or(info);
    // Drop leading '.' for Pandoc attribute style.
    let body = body.strip_prefix('.').unwrap_or(body);
    // First identifier ends at whitespace, comma, or '{'.
    let end = body
        .find(|c: char| c.is_whitespace() || c == ',' || c == '{' || c == '}')
        .unwrap_or(body.len());
    let head = &body[..end];
    if head.is_empty() {
        None
    } else {
        Some(head)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_common_aliases() {
        assert_eq!(normalize("ts"), Some("typescript"));
        assert_eq!(normalize("typescript"), Some("typescript"));
        assert_eq!(normalize("python3"), Some("python"));
        assert_eq!(normalize("rust"), Some("rust"));
        assert_eq!(normalize("rs"), Some("rust"));
    }

    #[test]
    fn accepts_modifiers() {
        assert_eq!(normalize("rust,no_run"), Some("rust"));
        assert_eq!(normalize("rust,ignore"), Some("rust"));
        assert_eq!(normalize("ts,pragma"), Some("typescript"));
    }

    #[test]
    fn accepts_pandoc_attributes() {
        assert_eq!(normalize("{.rust #anchor}"), Some("rust"));
        assert_eq!(normalize("{.python .numberLines}"), Some("python"));
    }

    #[test]
    fn accepts_notebook_chunks() {
        assert_eq!(normalize("{r}"), Some("r"));
        assert_eq!(normalize("{r, echo=FALSE}"), Some("r"));
        assert_eq!(normalize("{python, eval=TRUE}"), Some("python"));
    }

    #[test]
    fn unknown_returns_none() {
        assert_eq!(normalize(""), None);
        assert_eq!(normalize("mermaid"), None);
        assert_eq!(normalize("text"), None);
        assert_eq!(normalize("plantuml"), None);
        assert_eq!(normalize("diff"), None);
    }

    #[test]
    fn case_insensitive() {
        assert_eq!(normalize("TypeScript"), Some("typescript"));
        assert_eq!(normalize("RUST"), Some("rust"));
    }
}
