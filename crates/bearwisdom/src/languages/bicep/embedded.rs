//! Bicep deployment-script payload detection (E24).
//!
//! Azure deployment scripts (`Microsoft.Resources/deploymentScripts`)
//! take a `scriptContent: '…'` property whose body is a PowerShell or
//! Bash payload executed during ARM deployment. The kind is picked by
//! the sibling `kind:` property (`'AzurePowerShell'` → powershell,
//! `'AzureCLI'` → bash).
//!
//! For MVP we scan linearly for `scriptContent:` assignments and emit
//! a [`BuildToolShell`] region. The language id is `bash` — detecting
//! the `kind:` co-sibling across arbitrary source positions adds
//! complexity we can defer.
//!
//! [`BuildToolShell`]: crate::types::EmbeddedOrigin::BuildToolShell

use crate::types::{EmbeddedOrigin, EmbeddedRegion};

pub fn detect_regions(source: &str) -> Vec<EmbeddedRegion> {
    let mut regions = Vec::new();
    let lines: Vec<&str> = source.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("scriptContent:") {
            let value = rest.trim_start();
            // Multiline: `'''` ... `'''`.
            if value.starts_with("'''") {
                let (body, consumed) = read_triple_quoted(&lines, i, value);
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
            // Single-quoted inline string.
            if let Some(body) = value.strip_prefix('\'').and_then(|r| r.strip_suffix('\'')) {
                if !body.trim().is_empty() {
                    regions.push(EmbeddedRegion {
                        language_id: "bash".into(),
                        text: format!("{body}\n"),
                        line_offset: i as u32,
                        col_offset: 0,
                        origin: EmbeddedOrigin::BuildToolShell,
                        holes: Vec::new(),
                        strip_scope_prefix: None,
                    });
                }
            }
        }
        i += 1;
    }
    regions
}

/// Read a `'''`-delimited multiline string starting at `start_line`.
/// Opening `'''` is on `start_line` as part of `first_line_value`.
/// Returns the body and the line index AFTER the closing `'''`.
fn read_triple_quoted(
    lines: &[&str],
    start_line: usize,
    first_line_value: &str,
) -> (String, usize) {
    // Strip the opening `'''` from the first line.
    let first_rest = first_line_value.strip_prefix("'''").unwrap_or(first_line_value);
    // If the closing `'''` is also on the same line, return the slice.
    if let Some(closing_idx) = first_rest.find("'''") {
        let body = &first_rest[..closing_idx];
        return (format!("{body}\n"), start_line + 1);
    }
    let mut body = String::new();
    if !first_rest.is_empty() {
        body.push_str(first_rest);
        body.push('\n');
    }
    let mut i = start_line + 1;
    while i < lines.len() {
        if let Some(close_idx) = lines[i].find("'''") {
            body.push_str(&lines[i][..close_idx]);
            body.push('\n');
            return (body, i + 1);
        }
        body.push_str(lines[i]);
        body.push('\n');
        i += 1;
    }
    (body, i)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bicep_multiline_script_content_emits_bash() {
        let src = "resource s 'Microsoft.Resources/deploymentScripts@2020-10-01' = {\n  properties: {\n    scriptContent: '''\naz login --identity\naz group create --name rg --location eastus\n'''\n  }\n}\n";
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].language_id, "bash");
        assert_eq!(regions[0].origin, EmbeddedOrigin::BuildToolShell);
        assert!(regions[0].text.contains("az login"));
    }

    #[test]
    fn bicep_inline_single_quoted_script_content() {
        let src = "resource s '...' = {\n  properties: {\n    scriptContent: 'echo hello'\n  }\n}\n";
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 1);
        assert!(regions[0].text.contains("echo hello"));
    }

    #[test]
    fn no_scriptcontent_no_regions() {
        let src = "resource s '...' = { properties: { kind: 'AzureCLI' } }\n";
        let regions = detect_regions(src);
        assert!(regions.is_empty());
    }
}
