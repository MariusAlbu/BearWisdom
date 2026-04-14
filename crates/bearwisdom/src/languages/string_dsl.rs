//! Content-sniffing heuristics for string-DSL detection (E16 / E19).
//!
//! Host languages frequently carry entire SQL / HTML / JSON / CSS
//! payloads inside raw or verbatim string literals:
//!
//!   * C#  `@"SELECT * FROM t"`, `"""…"""`
//!   * Java `"""…"""` text blocks
//!   * Python `"""…"""` / `'''…'''` triple-quoted strings
//!   * Go  `` `…` `` raw strings
//!   * Kotlin / Swift `"""…"""` multiline strings
//!
//! This module exposes a single entry point — [`sniff`] — that accepts a
//! candidate string body and returns the canonical `language_id` to
//! dispatch to, or `None` when the content is opaque prose.
//!
//! Heuristics are intentionally conservative: false negatives are fine
//! (we just miss an embedded region); false positives are bad (we hand
//! random text to a grammar that will spray unresolved refs). Anything
//! below `MIN_BODY_LEN` or that doesn't match a recognisable DSL shape
//! returns `None`.

/// Minimum length (in bytes) for content sniffing to attempt detection.
/// Short strings like single-word identifiers, single SQL keywords, or
/// JSON scalars are excluded — we need enough structure to be confident.
const MIN_BODY_LEN: usize = 16;

/// Classify the contents of a raw/verbatim string body. Returns the
/// canonical sub-language id (`"sql"`, `"html"`, `"json"`, `"css"`) or
/// `None` when the body is not a recognisable DSL.
///
/// Call with the UNESCAPED body text — i.e. the content already stripped
/// of delimiters like `@"` / `"""` / backticks. Leading/trailing
/// whitespace is tolerated.
pub fn sniff(body: &str) -> Option<&'static str> {
    let trimmed = body.trim();
    if trimmed.len() < MIN_BODY_LEN {
        return None;
    }
    if looks_like_sql(trimmed) {
        return Some("sql");
    }
    if looks_like_json(trimmed) {
        return Some("json");
    }
    if looks_like_html(trimmed) {
        return Some("html");
    }
    if looks_like_css(trimmed) {
        return Some("css");
    }
    None
}

/// True when `body` opens with a DML/DDL keyword and contains enough
/// SQL-shaped punctuation to be confident. Comment-prefixed SQL (e.g.
/// a `-- header` line before `SELECT`) is also accepted.
pub fn looks_like_sql(body: &str) -> bool {
    let first_keyword = first_non_comment_line(body);
    let lower = first_keyword.to_ascii_lowercase();
    const KEYWORDS: &[&str] = &[
        "select ", "insert into ", "insert ", "update ", "delete from ",
        "delete ", "create table ", "create index ", "create view ",
        "create or replace ", "create procedure ", "create function ",
        "drop table ", "drop index ", "drop view ", "drop procedure ",
        "alter table ", "alter index ", "alter view ",
        "with ", "truncate ", "merge into ", "merge ",
    ];
    if !KEYWORDS.iter().any(|kw| lower.starts_with(kw)) {
        return false;
    }
    // Require at least one of: FROM, INTO, VALUES, SET, WHERE, JOIN,
    // ON, GROUP BY, to filter `SELECT x` one-liners and random prose
    // that happens to start with a keyword.
    let lower_full = body.to_ascii_lowercase();
    let structural_tokens = [" from ", " into ", " values", " set ", " where ",
        " join ", " on ", " group by", " order by", " returning "];
    structural_tokens.iter().any(|tok| lower_full.contains(tok))
}

/// True when `body` starts with `{` or `[` and ends with the matching
/// bracket, and the body contains at least one `:` (object separator)
/// or `,` (array separator) to rule out trivially-shaped prose.
pub fn looks_like_json(body: &str) -> bool {
    let b = body.as_bytes();
    let open = b.first().copied();
    let close = b.last().copied();
    let brackets_match = matches!(
        (open, close),
        (Some(b'{'), Some(b'}')) | (Some(b'['), Some(b']'))
    );
    if !brackets_match {
        return false;
    }
    let has_separator = body.contains(':') || body.contains(',');
    if !has_separator {
        return false;
    }
    // A naive "looks like JSON" test: must have at least one double-quoted
    // string token. This keeps Go / TS struct literals / dict dumps out.
    body.contains('"')
}

/// True when `body` contains at least one well-formed `<tag>…</tag>`
/// pair (or `<tag/>` self-closing) and opens with `<` after trimming.
pub fn looks_like_html(body: &str) -> bool {
    if !body.starts_with('<') {
        return false;
    }
    // Look for `<word` followed by `>` and at least one closing tag
    // `</word>` OR a self-closing `/>` to avoid `<T>` Rust/TS generics.
    let has_open = body.as_bytes().windows(2).any(|w| {
        w[0] == b'<' && (w[1].is_ascii_alphabetic() || w[1] == b'!')
    });
    let has_close = body.contains("</") || body.contains("/>");
    has_open && has_close
}

/// True when `body` matches a loose CSS selector+declaration shape:
/// `selector { prop: value; }` where `selector` contains `.`, `#`,
/// or a tag name.
pub fn looks_like_css(body: &str) -> bool {
    if !body.contains('{') || !body.contains('}') {
        return false;
    }
    // Count `prop: value;` segments inside the body. Require at least one.
    let body_bytes = body.as_bytes();
    let mut colons: usize = 0;
    let mut semis: usize = 0;
    for &b in body_bytes {
        match b {
            b':' => colons += 1,
            b';' => semis += 1,
            _ => {}
        }
    }
    colons >= 1 && semis >= 1
}

/// Return the first line of `body` after skipping blank lines and
/// SQL-style `--` / `#` comment lines. Used to make `looks_like_sql`
/// tolerate a leading comment header.
fn first_non_comment_line(body: &str) -> &str {
    for line in body.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with("--") || t.starts_with('#') {
            continue;
        }
        return t;
    }
    ""
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffs_select_as_sql() {
        let body = "SELECT id, name FROM users WHERE id = 1";
        assert_eq!(sniff(body), Some("sql"));
    }

    #[test]
    fn sniffs_insert_as_sql() {
        let body = "INSERT INTO orders(a, b) VALUES (1, 2)";
        assert_eq!(sniff(body), Some("sql"));
    }

    #[test]
    fn sniffs_update_as_sql() {
        let body = "UPDATE users SET name = 'x' WHERE id = 1";
        assert_eq!(sniff(body), Some("sql"));
    }

    #[test]
    fn sniffs_with_cte_as_sql() {
        let body = "WITH t AS (SELECT * FROM users) SELECT * FROM t";
        assert_eq!(sniff(body), Some("sql"));
    }

    #[test]
    fn sql_with_leading_comment() {
        let body = "-- Fetch active users\nSELECT * FROM users WHERE active = 1";
        assert_eq!(sniff(body), Some("sql"));
    }

    #[test]
    fn short_string_not_sniffed() {
        assert_eq!(sniff("SELECT 1"), None);
    }

    #[test]
    fn prose_not_sniffed() {
        assert_eq!(sniff("hello world this is just a long sentence"), None);
    }

    #[test]
    fn json_object_sniffed() {
        let body = r#"{"name": "alice", "age": 30}"#;
        assert_eq!(sniff(body), Some("json"));
    }

    #[test]
    fn json_array_sniffed() {
        let body = r#"["one", "two", "three"]"#;
        assert_eq!(sniff(body), Some("json"));
    }

    #[test]
    fn html_sniffed() {
        let body = "<div class=\"foo\"><p>hello world</p></div>";
        assert_eq!(sniff(body), Some("html"));
    }

    #[test]
    fn html_self_closing_sniffed() {
        let body = "<Component prop=\"value\" other=\"foo\"/>";
        assert_eq!(sniff(body), Some("html"));
    }

    #[test]
    fn generic_angle_bracket_not_html() {
        // `<T>` with no matching `</T>` and no `/>` — rejected.
        assert_eq!(sniff("<T> somearg description of a type"), None);
    }

    #[test]
    fn css_sniffed() {
        let body = ".button { color: red; padding: 10px; }";
        assert_eq!(sniff(body), Some("css"));
    }

    #[test]
    fn bare_keyword_without_structure_not_sql() {
        // "SELECT thing" alone is ambiguous (could be prose) — reject.
        assert_eq!(sniff("SELECT thing is interesting and"), None);
    }
}
