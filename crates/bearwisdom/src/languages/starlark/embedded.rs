//! Starlark / Bazel BUILD file build-tool shell detection (E24).
//!
//! `genrule(cmd = "<shell command>", …)` carries a bash script that
//! Bazel executes at build time. Emit it as a [`BuildToolShell`] region.
//!
//! The detector is line-shaped and captures `cmd = "…"` or
//! `cmd = """…"""` inside a `genrule(...)` call. It does not verify
//! the enclosing call is actually `genrule` — many Bazel rule macros
//! use `cmd` identically (`run_shell`, `native.genrule`, etc.), so
//! any `cmd = …` assignment at source-line start qualifies.
//!
//! [`BuildToolShell`]: crate::types::EmbeddedOrigin::BuildToolShell

use crate::types::{EmbeddedOrigin, EmbeddedRegion};

pub fn detect_regions(source: &str) -> Vec<EmbeddedRegion> {
    let mut regions = Vec::new();
    let bytes = source.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        // Scan for the `cmd =` or `cmd="..."` assignment pattern.
        let Some(start) = find_cmd_assignment(bytes, i) else {
            break;
        };
        i = start.after_equals;
        // Skip whitespace.
        while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        // Accept `"""..."""` triple-quoted first, then `"..."`.
        let body_result = if bytes[i..].starts_with(b"\"\"\"") {
            read_triple_string(bytes, i + 3)
        } else if bytes[i] == b'"' {
            read_double_string(bytes, i + 1)
        } else {
            None
        };
        let Some((body_start, body_end)) = body_result else {
            i += 1;
            continue;
        };
        let body = match std::str::from_utf8(&bytes[body_start..body_end]) {
            Ok(s) if !s.trim().is_empty() => s.to_string(),
            _ => {
                i = body_end + 1;
                continue;
            }
        };
        let (line_offset, col_offset) = byte_to_line_col(source, body_start);
        regions.push(EmbeddedRegion {
            language_id: "bash".into(),
            text: body,
            line_offset,
            col_offset,
            origin: EmbeddedOrigin::BuildToolShell,
            holes: Vec::new(),
            strip_scope_prefix: None,
        });
        i = body_end + 1;
    }
    regions
}

struct CmdAssign {
    after_equals: usize,
}

/// Find the next `cmd\s*=\s*` starting at or after `from`, where `cmd`
/// is preceded by a non-identifier char (so `kwd_cmd =` doesn't match)
/// and followed by a non-identifier char (so `cmd_extra =` doesn't
/// match).
fn find_cmd_assignment(bytes: &[u8], from: usize) -> Option<CmdAssign> {
    let mut i = from;
    while i + 3 < bytes.len() {
        let is_cmd = bytes[i] == b'c' && bytes[i + 1] == b'm' && bytes[i + 2] == b'd';
        let start_ok = i == 0 || !is_id_byte(bytes[i - 1]);
        let after = i + 3;
        let end_ok = after >= bytes.len() || !is_id_byte(bytes[after]);
        if is_cmd && start_ok && end_ok {
            let mut j = after;
            while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b'\t') {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'=' {
                return Some(CmdAssign { after_equals: j + 1 });
            }
        }
        i += 1;
    }
    None
}

fn is_id_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn read_triple_string(bytes: &[u8], start: usize) -> Option<(usize, usize)> {
    let mut i = start;
    while i + 2 < bytes.len() {
        if bytes[i] == b'"' && bytes[i + 1] == b'"' && bytes[i + 2] == b'"' {
            return Some((start, i));
        }
        i += 1;
    }
    None
}

fn read_double_string(bytes: &[u8], start: usize) -> Option<(usize, usize)> {
    let mut i = start;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'"' => return Some((start, i)),
            b'\n' => return None,
            _ => i += 1,
        }
    }
    None
}

fn byte_to_line_col(source: &str, byte: usize) -> (u32, u32) {
    let prefix = &source[..byte.min(source.len())];
    let line = prefix.bytes().filter(|&b| b == b'\n').count() as u32;
    let col = match prefix.rfind('\n') {
        Some(nl) => (byte - nl - 1) as u32,
        None => byte as u32,
    };
    (line, col)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn genrule_cmd_string_emits_build_tool_shell() {
        let src = "genrule(\n    name = \"gen\",\n    cmd = \"echo hello > $@\",\n    outs = [\"out.txt\"],\n)\n";
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].language_id, "bash");
        assert_eq!(regions[0].origin, EmbeddedOrigin::BuildToolShell);
        assert!(regions[0].text.contains("echo hello"));
    }

    #[test]
    fn genrule_cmd_triple_quoted_emits_build_tool_shell() {
        let src = "genrule(\n    cmd = \"\"\"\nfor x in a b c; do echo $x; done\n    \"\"\",\n)\n";
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 1);
        assert!(regions[0].text.contains("for x in a b c"));
    }

    #[test]
    fn unrelated_cmd_identifier_is_not_matched() {
        // `cmd_extra` is an identifier, not an assignment to `cmd`.
        let src = "cmd_extra = \"not a command\"\n";
        let regions = detect_regions(src);
        assert!(regions.is_empty());
    }
}
