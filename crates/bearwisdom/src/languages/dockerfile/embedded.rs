//! Dockerfile embedded regions.
//!
//! `RUN`, `CMD`, `ENTRYPOINT`, `HEALTHCHECK` in shell form embed
//! bash. The exec form (`["cmd", "arg"]` JSON array) is not a shell
//! and is skipped. Line continuations (`\` at end of line) are
//! folded into a single multi-line command.

use crate::types::{EmbeddedOrigin, EmbeddedRegion};

pub fn detect_regions(source: &str) -> Vec<EmbeddedRegion> {
    let mut regions = Vec::new();
    let lines: Vec<&str> = source.lines().collect();
    let mut i = 0usize;
    while i < lines.len() {
        let raw = lines[i];
        let trimmed = raw.trim_start();
        let upper = trimmed.to_ascii_uppercase();
        for kw in &["RUN ", "CMD ", "ENTRYPOINT ", "HEALTHCHECK "] {
            if upper.starts_with(kw) {
                let rest = &trimmed[kw.len()..];
                let rest_trim = rest.trim_start();
                // HEALTHCHECK can be followed by CMD — extract the CMD.
                let (cmd_body, cmd_line) = if *kw == "HEALTHCHECK " {
                    let up = rest_trim.to_ascii_uppercase();
                    if let Some(idx) = up.find("CMD ") {
                        (rest_trim[idx + 4..].to_string(), i as u32)
                    } else { (String::new(), i as u32) }
                } else {
                    (rest_trim.to_string(), i as u32)
                };
                if cmd_body.trim_start().starts_with('[') {
                    // Exec form — skip.
                    break;
                }
                if cmd_body.is_empty() { break; }
                // Fold line continuations.
                let mut body = cmd_body.clone();
                let mut j = i;
                while body.trim_end().ends_with('\\') {
                    body = body.trim_end().trim_end_matches('\\').to_string();
                    j += 1;
                    if j >= lines.len() { break; }
                    body.push('\n');
                    body.push_str(lines[j]);
                }
                regions.push(EmbeddedRegion {
                    language_id: "bash".into(),
                    text: format!("{}\n", body.trim()),
                    line_offset: cmd_line,
                    col_offset: 0,
                    origin: EmbeddedOrigin::TemplateExpr,
                    holes: Vec::new(),
                    strip_scope_prefix: None,
                });
                i = j;
                break;
            }
        }
        i += 1;
    }
    regions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_line_becomes_bash() {
        let src = "FROM alpine\nRUN apk add curl\n";
        let regions = detect_regions(src);
        assert!(regions.iter().any(|r| r.language_id == "bash" && r.text.contains("apk add curl")));
    }

    #[test]
    fn exec_form_skipped() {
        let src = "ENTRYPOINT [\"node\", \"server.js\"]\n";
        let regions = detect_regions(src);
        assert!(regions.is_empty());
    }

    #[test]
    fn line_continuation_folded() {
        let src = "RUN apt-get update && \\\n    apt-get install -y curl\n";
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 1);
        assert!(regions[0].text.contains("apt-get update"));
        assert!(regions[0].text.contains("curl"));
    }

    #[test]
    fn healthcheck_shell_form_captured() {
        let src = "HEALTHCHECK CMD curl -f http://localhost/ || exit 1\n";
        let regions = detect_regions(src);
        assert!(regions.iter().any(|r| r.text.contains("curl -f")));
    }
}
