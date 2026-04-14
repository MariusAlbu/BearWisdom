//! systemd `Exec*=` directive values become bash regions.

use crate::types::{EmbeddedOrigin, EmbeddedRegion};

const EXEC_KEYS: &[&str] = &[
    "ExecStart", "ExecStartPre", "ExecStartPost",
    "ExecStop", "ExecStopPost", "ExecReload", "ExecCondition",
];

pub fn detect_regions(source: &str) -> Vec<EmbeddedRegion> {
    let mut regions = Vec::new();
    for (line_no, line) in source.lines().enumerate() {
        let trimmed = line.trim_start();
        for key in EXEC_KEYS {
            if let Some(rest) = trimmed.strip_prefix(&format!("{key}=")) {
                let cmd = rest.trim_start_matches(|c: char| matches!(c, '-' | '+' | '!' | ':' | '@')).trim();
                if !cmd.is_empty() {
                    regions.push(EmbeddedRegion {
                        language_id: "bash".into(),
                        text: format!("{cmd}\n"),
                        line_offset: line_no as u32,
                        col_offset: 0,
                        origin: EmbeddedOrigin::TemplateExpr,
                        holes: Vec::new(),
                        strip_scope_prefix: None,
                    });
                }
                break;
            }
        }
    }
    regions
}
