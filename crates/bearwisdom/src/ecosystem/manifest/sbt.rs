// indexer/manifest/sbt.rs — build.sbt reader (Scala)

use std::path::Path;

use super::{ManifestData, ManifestKind, ManifestReader};

pub struct SbtManifest;

impl ManifestReader for SbtManifest {
    fn kind(&self) -> ManifestKind {
        ManifestKind::Sbt
    }

    fn read(&self, project_root: &Path) -> Option<ManifestData> {
        let build_sbt = project_root.join("build.sbt");
        if !build_sbt.is_file() {
            return None;
        }
        let content = std::fs::read_to_string(&build_sbt).ok()?;
        let mut data = ManifestData::default();

        for dep in parse_sbt_deps(&content) {
            data.dependencies.insert(dep);
        }

        // Also scan project/Dependencies.scala if it exists.
        let deps_scala = project_root.join("project").join("Dependencies.scala");
        if let Ok(deps_content) = std::fs::read_to_string(&deps_scala) {
            for dep in parse_sbt_deps(&deps_content) {
                data.dependencies.insert(dep);
            }
        }

        Some(data)
    }
}

/// Extract dependency artifact names from sbt build files.
///
/// Matches patterns like:
///   `"org.typelevel" %% "cats-core" % "2.12.0"`
///   `"org.http4s" %% "http4s-dsl" % V.http4s`
///   `"io.circe" %%% "circe-core" % V.circe`
///   `"com.google.guava" % "guava" % "33.0"`
///
/// Returns the artifact name (second quoted string after `%` or `%%` or `%%%`).
pub fn parse_sbt_deps(content: &str) -> Vec<String> {
    let mut deps = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("//") {
            continue;
        }
        // Match: "group" %{1,3} "artifact" % "version"
        // Also match: "group" %{1,3} s"artifact-$name" for string interpolation
        for (artifact, _group, _version) in extract_sbt_coords(trimmed) {
            if !artifact.is_empty() && !deps.contains(&artifact) {
                deps.push(artifact);
            }
        }
    }
    deps
}

/// Parse every `"group" %% "artifact"` pair from an sbt manifest. Returns
/// `(group_id, artifact_id)` so callers can construct full `MavenCoord`s
/// — sbt versions often reference Scala-side `val` definitions
/// (`V.cats`, `Versions.akka`) which we can't evaluate, so the version is
/// left for the resolver's directory-scan fallback.
pub fn parse_sbt_coord_pairs(content: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("//") { continue; }
        for (artifact, group, _version) in extract_sbt_coords(trimmed) {
            if artifact.is_empty() || group.is_empty() { continue; }
            let key = (group.clone(), artifact.clone());
            if seen.insert(key.clone()) {
                out.push(key);
            }
        }
    }
    out
}

/// Parse `(group, artifact, Option<version>)` triples from an sbt manifest.
/// Versions can come from two places:
///
///   1. Inline string literals: `"org.typelevel" %% "cats-core" % "2.12.0"`.
///   2. References to `val NAME = "VERSION"` bindings — sbt convention is
///      `object V { val cats = "2.12.0" }` or scattered `val` definitions
///      anywhere in the manifest. The `vars` map carries those bindings,
///      typically built up across every sbt manifest in the project (root
///      `build.sbt`, `project/Dependencies.scala`, sub-module `build.sbt`s).
///
/// When the version reference doesn't resolve (transitive deps, unknown
/// `Versions.foo`), the third tuple element is `None` and the resolver
/// falls back to its directory-scan logic.
pub fn parse_sbt_coord_triples(
    content: &str,
    vars: &std::collections::HashMap<String, String>,
) -> Vec<(String, String, Option<String>)> {
    let mut out = Vec::new();
    let mut seen: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("//") { continue; }
        for (artifact, group, version_token) in extract_sbt_coords(trimmed) {
            if artifact.is_empty() || group.is_empty() { continue; }
            let key = (group.clone(), artifact.clone());
            if !seen.insert(key) { continue; }
            let version = resolve_version_token(&version_token, vars);
            out.push((group, artifact, version));
        }
    }
    out
}

/// Walk all `val NAME = "VERSION"` bindings in an sbt manifest. Strips
/// `object V { ... }` wrappers, ignores nested template / string-interp
/// values, and only emits bare quoted strings (the version-pin shape).
/// Caller folds these into one map per project.
pub fn parse_sbt_version_vars(content: &str) -> std::collections::HashMap<String, String> {
    let mut out = std::collections::HashMap::new();
    for raw in content.lines() {
        let line = raw.trim();
        if line.starts_with("//") { continue; }
        // Match `val NAME = "VERSION"` and `val NAME: String = "VERSION"`.
        // Inside `object V { val cats = "..." }` the `val` lines are
        // indented but trim() handles that.
        let Some(rest) = line.strip_prefix("val ") else { continue; };
        let bytes = rest.as_bytes();
        // Walk to either '=' or ':' (type annotation).
        let mut name_end = 0;
        while name_end < bytes.len()
            && (bytes[name_end].is_ascii_alphanumeric() || bytes[name_end] == b'_')
        {
            name_end += 1;
        }
        if name_end == 0 { continue; }
        let name = &rest[..name_end];
        let after_name = rest[name_end..].trim_start();
        // Skip optional `: Type` annotation.
        let value_part = if let Some(colon_rest) = after_name.strip_prefix(':') {
            // Find the `=` after the type annotation.
            let Some(eq_pos) = colon_rest.find('=') else { continue; };
            colon_rest[eq_pos + 1..].trim_start()
        } else if let Some(eq_rest) = after_name.strip_prefix('=') {
            eq_rest.trim_start()
        } else {
            continue;
        };
        // Only accept a bare quoted-string value: `"X.Y.Z"`. Skip anything
        // that does string interpolation or arithmetic.
        let Some(value) = value_part.strip_prefix('"') else { continue; };
        let Some(end_quote) = value.find('"') else { continue; };
        let version = &value[..end_quote];
        // After the closing quote, only whitespace / comment / line end is
        // acceptable. `"X" + suffix` and similar are not version pins.
        let tail = value[end_quote + 1..].trim_start();
        if !tail.is_empty() && !tail.starts_with("//") { continue; }
        out.insert(name.to_string(), version.to_string());
    }
    out
}

/// Resolve a version token captured next to a `% V.foo` reference. The
/// token is either:
///   - `"X.Y.Z"` — already a literal; strip the quotes.
///   - `V.cats` / `Versions.cats` — look up the rightmost dotted segment
///      in `vars`. (sbt code wraps version constants in objects of various
///      names; only the trailing identifier is the binding.)
///   - bare `cats` — same as above, no qualifier.
///   - empty — no version on this dep line.
fn resolve_version_token(
    token: &str,
    vars: &std::collections::HashMap<String, String>,
) -> Option<String> {
    let trimmed = token.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(stripped) = trimmed.strip_prefix('"') {
        if let Some(end_quote) = stripped.find('"') {
            return Some(stripped[..end_quote].to_string());
        }
    }
    let last_segment = trimmed.rsplit('.').next()?;
    vars.get(last_segment).cloned()
}

/// Extract `(artifact_id, group_id, version_token)` triples from a line of
/// sbt DSL. The version token is the raw text immediately following the
/// `% / %% / %%%` operator after the artifact — caller resolves it against
/// a `val NAME = "X.Y.Z"` map. Empty when the dep line has no version.
fn extract_sbt_coords(line: &str) -> Vec<(String, String, String)> {
    let mut results = Vec::new();
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        // Find a quoted string (group id)
        if bytes[i] == b'"' {
            if let Some((group, end)) = extract_quoted(line, i) {
                i = end;
                // Skip whitespace
                while i < len && bytes[i].is_ascii_whitespace() { i += 1; }
                // Match %% or %%% or %
                if i < len && bytes[i] == b'%' {
                    i += 1;
                    while i < len && bytes[i] == b'%' { i += 1; }
                    // Skip whitespace
                    while i < len && bytes[i].is_ascii_whitespace() { i += 1; }
                    // Extract artifact (quoted string or s"..." interpolation)
                    let mut artifact_opt: Option<(String, usize)> = None;
                    if i < len && bytes[i] == b'"' {
                        if let Some((artifact, end2)) = extract_quoted(line, i) {
                            artifact_opt = Some((artifact, end2));
                        }
                    }
                    // s"artifact-..." pattern
                    if artifact_opt.is_none()
                        && i + 1 < len
                        && bytes[i] == b's'
                        && bytes[i + 1] == b'"'
                    {
                        if let Some((artifact, end2)) = extract_quoted(line, i + 1) {
                            let static_part = artifact
                                .split('$')
                                .next()
                                .unwrap_or("")
                                .trim_end_matches('-');
                            if !static_part.is_empty() {
                                artifact_opt = Some((static_part.to_string(), end2));
                            }
                        }
                    }
                    if let Some((artifact, end2)) = artifact_opt {
                        let version_token = extract_version_token(line, end2);
                        results.push((artifact, group, version_token));
                        i = end2;
                        continue;
                    }
                }
            }
        }
        i += 1;
    }
    results
}

/// Read a version token after the `% "X.Y.Z"` or `% V.foo` operator that
/// follows the artifact identifier. Returns the raw text up to the next
/// terminator (`,`, `)`, ` cross `, end-of-line) so the caller can resolve
/// quoted literals or `V.NAME` references in one place.
fn extract_version_token(line: &str, start: usize) -> String {
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut i = start;
    while i < len && bytes[i].is_ascii_whitespace() { i += 1; }
    if i >= len || bytes[i] != b'%' {
        return String::new();
    }
    i += 1;
    while i < len && bytes[i] == b'%' { i += 1; }
    while i < len && bytes[i].is_ascii_whitespace() { i += 1; }
    if i >= len {
        return String::new();
    }
    if bytes[i] == b'"' {
        return match extract_quoted(line, i) {
            Some((v, _)) => format!("\"{v}\""),
            None => String::new(),
        };
    }
    // Identifier / dotted-path token.
    let token_start = i;
    while i < len {
        let c = bytes[i];
        if c.is_ascii_alphanumeric() || c == b'_' || c == b'.' {
            i += 1;
        } else {
            break;
        }
    }
    line[token_start..i].to_string()
}

fn extract_quoted(s: &str, start: usize) -> Option<(String, usize)> {
    if s.as_bytes().get(start) != Some(&b'"') {
        return None;
    }
    let rest = &s[start + 1..];
    let end_quote = rest.find('"')?;
    Some((rest[..end_quote].to_string(), start + 1 + end_quote + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_double_percent() {
        let line = r#"  "org.typelevel" %% "cats-core" % "2.12.0""#;
        let deps = parse_sbt_deps(line);
        assert_eq!(deps, vec!["cats-core"]);
    }

    #[test]
    fn parse_triple_percent() {
        let line = r#"  "org.typelevel" %%% "cats-core" % V.cats"#;
        let deps = parse_sbt_deps(line);
        assert_eq!(deps, vec!["cats-core"]);
    }

    #[test]
    fn parse_single_percent() {
        let line = r#"  "com.google.guava" % "guava" % "33.0""#;
        let deps = parse_sbt_deps(line);
        assert_eq!(deps, vec!["guava"]);
    }

    #[test]
    fn parse_multiple_deps_in_list() {
        let content = r#"
libraryDependencies ++= List(
  "org.http4s" %% "http4s-dsl" % V.http4s,
  "org.http4s" %% "http4s-ember-server" % V.http4s,
  "io.circe" %% "circe-core" % V.circe,
)
"#;
        let deps = parse_sbt_deps(content);
        assert_eq!(deps, vec!["http4s-dsl", "http4s-ember-server", "circe-core"]);
    }

    #[test]
    fn parse_interpolated_artifact() {
        let content = r#"  def circe(artifact: String) = Def.setting("io.circe" %%% s"circe-$artifact" % V.circe)"#;
        let deps = parse_sbt_deps(content);
        assert_eq!(deps, vec!["circe"]);
    }

    #[test]
    fn skips_comments() {
        let content = "// \"org.fake\" %% \"fake-lib\" % \"1.0\"\n\"org.real\" %% \"real-lib\" % \"1.0\"";
        let deps = parse_sbt_deps(content);
        assert_eq!(deps, vec!["real-lib"]);
    }

    #[test]
    fn parse_version_vars_basic() {
        let content = r#"
            object V {
              val cats       = "2.12.0"
              val catsEffect = "3.6.1"
              val http4s     = "1.0.0-M43"
            }
        "#;
        let vars = parse_sbt_version_vars(content);
        assert_eq!(vars.get("cats"), Some(&"2.12.0".to_string()));
        assert_eq!(vars.get("catsEffect"), Some(&"3.6.1".to_string()));
        assert_eq!(vars.get("http4s"), Some(&"1.0.0-M43".to_string()));
    }

    #[test]
    fn parse_version_vars_skips_non_literal() {
        let content = r#"
            val a = "1.0"
            val b = a + ".0"
            val c = s"$a.$b"
            val d: String = "2.0"
        "#;
        let vars = parse_sbt_version_vars(content);
        assert_eq!(vars.get("a"), Some(&"1.0".to_string()));
        assert_eq!(vars.get("b"), None);
        assert_eq!(vars.get("c"), None);
        assert_eq!(vars.get("d"), Some(&"2.0".to_string()));
    }

    #[test]
    fn coord_triples_resolve_v_dot_name() {
        let mut vars = std::collections::HashMap::new();
        vars.insert("cats".to_string(), "2.12.0".to_string());
        vars.insert("http4s".to_string(), "1.0.0-M43".to_string());
        let content = r#"
            "org.typelevel" %% "cats-core" % V.cats,
            "org.http4s"    %% "http4s-dsl" % V.http4s,
            "io.circe"      %% "circe-core" % "0.14.10",
        "#;
        let triples = parse_sbt_coord_triples(content, &vars);
        assert!(triples.contains(&(
            "org.typelevel".to_string(),
            "cats-core".to_string(),
            Some("2.12.0".to_string())
        )));
        assert!(triples.contains(&(
            "org.http4s".to_string(),
            "http4s-dsl".to_string(),
            Some("1.0.0-M43".to_string())
        )));
        assert!(triples.contains(&(
            "io.circe".to_string(),
            "circe-core".to_string(),
            Some("0.14.10".to_string())
        )));
    }

    #[test]
    fn coord_triples_unresolved_version_is_none() {
        let vars = std::collections::HashMap::new();
        let content = r#""com.example" %% "ghost" % V.unknown"#;
        let triples = parse_sbt_coord_triples(content, &vars);
        assert_eq!(
            triples,
            vec![("com.example".to_string(), "ghost".to_string(), None)]
        );
    }

    #[test]
    fn end_to_end_dependencies_scala_pattern() {
        // Mirrors a real-world `project/Dependencies.scala`: an `object V`
        // with version pins and an `object Libraries` whose entries reference
        // those pins via `%% "art" % V.name`. parse_sbt_version_vars feeds
        // the substitutions; parse_sbt_coord_triples consumes them.
        let content = r#"
object Dependencies {
  object V {
    val cats          = "2.12.0"
    val catsEffect    = "3.6.1"
    val fs2Core       = "3.12.0"
    val http4s        = "1.0.0-M43"
  }
  object Libraries {
    val cats       = Def.setting("org.typelevel" %%% "cats-core" % V.cats)
    val catsEffect = Def.setting("org.typelevel" %%% "cats-effect" % V.catsEffect)
    val fs2Core    = Def.setting("co.fs2" %%% "fs2-core" % V.fs2Core)
    val http4sDsl  = "org.http4s" %% "http4s-dsl" % V.http4s
  }
}
"#;
        let vars = parse_sbt_version_vars(content);
        let triples = parse_sbt_coord_triples(content, &vars);
        let by_artifact: std::collections::HashMap<&str, &Option<String>> = triples
            .iter()
            .map(|(_, a, v)| (a.as_str(), v))
            .collect();
        assert_eq!(
            by_artifact.get("cats-core").and_then(|v| v.as_ref()).map(|s| s.as_str()),
            Some("2.12.0")
        );
        assert_eq!(
            by_artifact.get("cats-effect").and_then(|v| v.as_ref()).map(|s| s.as_str()),
            Some("3.6.1")
        );
        assert_eq!(
            by_artifact.get("fs2-core").and_then(|v| v.as_ref()).map(|s| s.as_str()),
            Some("3.12.0")
        );
        assert_eq!(
            by_artifact.get("http4s-dsl").and_then(|v| v.as_ref()).map(|s| s.as_str()),
            Some("1.0.0-M43")
        );
    }
}
