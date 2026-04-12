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
        for (artifact, _group) in extract_sbt_coords(trimmed) {
            if !artifact.is_empty() && !deps.contains(&artifact) {
                deps.push(artifact);
            }
        }
    }
    deps
}

/// Extract (artifact_id, group_id) pairs from a line of sbt DSL.
fn extract_sbt_coords(line: &str) -> Vec<(String, String)> {
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
                    if i < len && bytes[i] == b'"' {
                        if let Some((artifact, end2)) = extract_quoted(line, i) {
                            results.push((artifact, group));
                            i = end2;
                            continue;
                        }
                    }
                    // s"artifact-..." pattern
                    if i + 1 < len && bytes[i] == b's' && bytes[i + 1] == b'"' {
                        if let Some((artifact, end2)) = extract_quoted(line, i + 1) {
                            // Strip interpolation markers: take the static prefix
                            let static_part = artifact.split('$').next().unwrap_or("").trim_end_matches('-');
                            if !static_part.is_empty() {
                                results.push((static_part.to_string(), group));
                            }
                            i = end2;
                            continue;
                        }
                    }
                }
            }
        }
        i += 1;
    }
    results
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
}
