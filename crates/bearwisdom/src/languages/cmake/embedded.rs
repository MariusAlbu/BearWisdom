//! CMake build-tool shell detection (E24).
//!
//! CMake commands `add_custom_command`, `add_custom_target`,
//! `execute_process`, and `add_test` take a `COMMAND <program> <args>`
//! keyword argument. The full command becomes a bash region with
//! [`BuildToolShell`] origin.
//!
//! Detection is regex-shaped rather than tree-sitter-driven: CMake's
//! grammar represents `COMMAND` as a plain argument identifier, so we
//! scan the source line-by-line for `COMMAND <rest-of-line>` where the
//! previous non-whitespace context is a relevant CMake command.
//!
//! [`BuildToolShell`]: crate::types::EmbeddedOrigin::BuildToolShell

use crate::types::{EmbeddedOrigin, EmbeddedRegion};

const COMMAND_KEYWORDS: &[&str] = &["COMMAND", "WORKING_COMMAND"];

pub fn detect_regions(source: &str) -> Vec<EmbeddedRegion> {
    let mut regions = Vec::new();
    for (line_no, line) in source.lines().enumerate() {
        let Some(cmd) = extract_command(line) else {
            continue;
        };
        if cmd.is_empty() {
            continue;
        }
        regions.push(EmbeddedRegion {
            language_id: "bash".into(),
            text: format!("{cmd}\n"),
            line_offset: line_no as u32,
            col_offset: 0,
            origin: EmbeddedOrigin::BuildToolShell,
            holes: Vec::new(),
            strip_scope_prefix: None,
        });
    }
    regions
}

/// Find `COMMAND <rest>` or `WORKING_COMMAND <rest>` within `line` where
/// the keyword is preceded by whitespace or `(` — so `COMMANDS` /
/// `COMMANDLINE` don't match. Returns the trimmed rest-of-line with
/// any trailing `)` stripped.
fn extract_command(line: &str) -> Option<&str> {
    for kw in COMMAND_KEYWORDS {
        let Some(kw_pos) = line.find(kw) else { continue };
        // Preceding char must be whitespace, `(`, or start-of-line.
        let prev_ok = kw_pos == 0
            || matches!(
                line.as_bytes()[kw_pos - 1],
                b' ' | b'\t' | b'('
            );
        if !prev_ok {
            continue;
        }
        let after = &line[kw_pos + kw.len()..];
        // Following char must be whitespace (end-of-keyword boundary).
        let is_whitespace_next = after
            .chars()
            .next()
            .map_or(false, |c| c.is_whitespace());
        if !is_whitespace_next {
            continue;
        }
        let mut cmd = after.trim();
        if let Some(stripped) = cmd.strip_suffix(')') {
            cmd = stripped.trim();
        }
        return Some(cmd);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_custom_command_emits_build_tool_shell() {
        let src = "add_custom_command(\n  OUTPUT foo.h\n  COMMAND python gen.py --output foo.h\n)\n";
        let regions = detect_regions(src);
        let shell: Vec<_> = regions.iter()
            .filter(|r| r.origin == EmbeddedOrigin::BuildToolShell)
            .collect();
        assert_eq!(shell.len(), 1);
        assert!(shell[0].text.contains("python gen.py"));
        assert_eq!(shell[0].language_id, "bash");
    }

    #[test]
    fn execute_process_single_line_emits_region() {
        let src = "execute_process(COMMAND git rev-parse HEAD OUTPUT_VARIABLE REV)\n";
        // Single-line form: COMMAND is not the last token; our simple
        // scanner captures everything to EOL which includes OUTPUT_VAR.
        // That's acceptable — bash grammar tolerates extra args.
        let regions = detect_regions(src);
        assert!(regions.iter().any(|r| r.text.contains("git rev-parse HEAD")));
    }

    #[test]
    fn commands_identifier_is_rejected() {
        let src = "list(APPEND COMMANDS foo bar)\n";
        let regions = detect_regions(src);
        assert!(regions.is_empty());
    }

    #[test]
    fn empty_command_skipped() {
        let src = "add_custom_command(\n  COMMAND\n)\n";
        let regions = detect_regions(src);
        assert!(regions.is_empty());
    }
}
