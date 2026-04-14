//! HCL / Terraform build-tool shell detection (E24).
//!
//! Terraform AWS / Azure instance blocks frequently carry shell payloads:
//!
//!   * `user_data = <<EOT … EOT` or `user_data = file("bootstrap.sh")`
//!     (file() is not hole-punched; only inline heredocs are handled).
//!   * `provisioner "local-exec" { command = "<shell cmd>" }`
//!   * `provisioner "remote-exec" { inline = ["cmd1", "cmd2"] }`
//!
//! For MVP we pick up heredocs assigned to `user_data` or `command` and
//! emit them as bash [`BuildToolShell`] regions.
//!
//! [`BuildToolShell`]: crate::types::EmbeddedOrigin::BuildToolShell

use crate::types::{EmbeddedOrigin, EmbeddedRegion};

const SHELL_KEYS: &[&str] = &["user_data", "command", "script"];

pub fn detect_regions(source: &str) -> Vec<EmbeddedRegion> {
    let mut regions = Vec::new();
    let lines: Vec<&str> = source.lines().collect();
    let mut i = 0usize;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim_start();
        if let Some((key, rest)) = split_key_equals(trimmed) {
            if SHELL_KEYS.contains(&key) {
                let value = rest.trim_start();
                // Heredoc form: `<<TAG` or `<<-TAG`.
                if let Some(tag) = parse_heredoc_tag(value) {
                    let (body, consumed) = read_heredoc_body(&lines, i + 1, &tag);
                    if !body.trim().is_empty() {
                        regions.push(EmbeddedRegion {
                            language_id: "bash".into(),
                            text: body,
                            line_offset: (i + 1) as u32,
                            col_offset: 0,
                            origin: EmbeddedOrigin::BuildToolShell,
                            holes: Vec::new(),
                            strip_scope_prefix: None,
                        });
                    }
                    i = consumed;
                    continue;
                }
                // Quoted single-line command.
                if let Some(cmd) = trim_quotes(value) {
                    if !cmd.is_empty() {
                        regions.push(EmbeddedRegion {
                            language_id: "bash".into(),
                            text: format!("{cmd}\n"),
                            line_offset: i as u32,
                            col_offset: 0,
                            origin: EmbeddedOrigin::BuildToolShell,
                            holes: Vec::new(),
                            strip_scope_prefix: None,
                        });
                    }
                }
            }
        }
        i += 1;
    }
    regions
}

fn split_key_equals(line: &str) -> Option<(&str, &str)> {
    let eq = line.find('=')?;
    let key = line[..eq].trim();
    if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    Some((key, &line[eq + 1..]))
}

/// Parse `<<EOT` or `<<-EOT` and return the closing tag. Returns `None`
/// when the value doesn't open a heredoc.
fn parse_heredoc_tag(value: &str) -> Option<String> {
    let rest = value.strip_prefix("<<")?;
    let rest = rest.strip_prefix('-').unwrap_or(rest);
    let tag: String = rest
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    if tag.is_empty() {
        None
    } else {
        Some(tag)
    }
}

/// Collect heredoc body lines until the closing tag line. Returns the
/// joined body and the line index AFTER the closing tag.
fn read_heredoc_body(lines: &[&str], start: usize, tag: &str) -> (String, usize) {
    let mut body = String::new();
    let mut i = start;
    while i < lines.len() {
        if lines[i].trim() == tag {
            return (body, i + 1);
        }
        body.push_str(lines[i]);
        body.push('\n');
        i += 1;
    }
    (body, i)
}

fn trim_quotes(s: &str) -> Option<&str> {
    let t = s.trim();
    let stripped = t.strip_prefix('"').and_then(|r| r.strip_suffix('"'))?;
    Some(stripped.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_data_heredoc_emits_build_tool_shell() {
        let src = "resource \"aws_instance\" \"web\" {\n  user_data = <<EOT\n#!/bin/bash\napt-get update && apt-get install -y nginx\nsystemctl enable nginx\nEOT\n}\n";
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].language_id, "bash");
        assert_eq!(regions[0].origin, EmbeddedOrigin::BuildToolShell);
        assert!(regions[0].text.contains("apt-get update"));
    }

    #[test]
    fn indented_heredoc_also_detected() {
        let src = "resource \"x\" \"y\" {\n  user_data = <<-EOT\n    echo hi\n    EOT\n}\n";
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 1);
        assert!(regions[0].text.contains("echo hi"));
    }

    #[test]
    fn local_exec_command_emits_region() {
        let src = "resource \"null_resource\" \"r\" {\n  provisioner \"local-exec\" {\n    command = \"terraform output -json\"\n  }\n}\n";
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 1);
        assert!(regions[0].text.contains("terraform output"));
    }

    #[test]
    fn unrelated_keys_ignored() {
        let src = "resource \"x\" \"y\" {\n  name = \"db\"\n  size = 100\n}\n";
        let regions = detect_regions(src);
        assert!(regions.is_empty());
    }
}
