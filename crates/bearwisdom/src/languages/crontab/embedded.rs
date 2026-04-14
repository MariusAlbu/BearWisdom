//! Crontab embedded regions — command column becomes bash.

use crate::types::{EmbeddedOrigin, EmbeddedRegion};

pub fn detect_regions(source: &str) -> Vec<EmbeddedRegion> {
    let mut regions = Vec::new();
    for (line_no, line) in source.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') { continue; }
        // Skip `KEY=value` env declarations.
        if let Some(eq) = trimmed.find('=') {
            let head = &trimmed[..eq];
            if !head.contains(char::is_whitespace) {
                continue;
            }
        }
        // Split into schedule fields + command.
        // System crontabs: `m h dom mon dow  user  command`
        // User crontabs:   `m h dom mon dow  command`
        // For MVP, assume 5 fields + rest is command.
        let mut parts = trimmed.split_whitespace();
        let mut consumed = 0;
        for _ in 0..5 { if parts.next().is_some() { consumed += 1; } }
        if consumed < 5 { continue; }
        // Skip one more if it looks like a user (alphanumeric only, no path chars).
        let rest_after_schedule: String = parts.collect::<Vec<_>>().join(" ");
        let cmd = if let Some(first_word) = rest_after_schedule.split_whitespace().next() {
            if first_word.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
                // Looks like a user; take the remainder.
                rest_after_schedule
                    .splitn(2, char::is_whitespace)
                    .nth(1)
                    .unwrap_or(&rest_after_schedule)
                    .trim()
                    .to_string()
            } else {
                rest_after_schedule.clone()
            }
        } else {
            continue;
        };
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
    }
    regions
}
