// =============================================================================
// go/tags.rs  —  Go struct tag parsing
//
// Go struct tags are raw string literals attached to field declarations:
//
//   Name string `json:"name" db:"user_name" validate:"required"`
//
// We parse the tag string and store the result in the field symbol's
// `doc_comment` field so it is queryable via the existing symbol search
// infrastructure without schema changes.
//
// Stored format: `[tags: json="name" db="user_name" validate="required"]`
// =============================================================================

/// Parse a Go struct tag string into key-value pairs.
///
/// Input: the raw tag text including surrounding backticks, e.g.
///   `` `json:"name" db:"user_name" validate:"required"` ``
///
/// Returns a `Vec<(key, value)>` in declaration order.
pub(super) fn parse_struct_tags(raw: &str) -> Vec<(String, String)> {
    // Strip surrounding backticks.
    let inner = raw.trim().trim_matches('`');
    let mut result = Vec::new();
    let mut rest = inner.trim();

    while !rest.is_empty() {
        // Find the colon that separates the key from its value.
        let colon_pos = match rest.find(':') {
            Some(pos) => pos,
            None => break,
        };

        let key = rest[..colon_pos].trim().to_string();
        if key.is_empty() {
            break;
        }

        rest = &rest[colon_pos + 1..];

        // The value must start with a double-quote.
        if !rest.starts_with('"') {
            // Malformed tag — skip to the next space boundary.
            match rest.find(' ') {
                Some(sp) => {
                    rest = rest[sp..].trim_start();
                    continue;
                }
                None => break,
            }
        }

        // Consume the opening quote.
        rest = &rest[1..];

        // Find the closing quote (not preceded by a backslash).
        let mut end = 0;
        let bytes = rest.as_bytes();
        loop {
            if end >= bytes.len() {
                break;
            }
            if bytes[end] == b'"' {
                break;
            }
            // Skip escaped characters.
            if bytes[end] == b'\\' {
                end += 1;
            }
            end += 1;
        }

        let value = rest[..end].to_string();
        result.push((key, value));

        if end < rest.len() {
            rest = rest[end + 1..].trim_start(); // skip closing quote then whitespace
        } else {
            break;
        }
    }

    result
}

/// Format parsed tags into the compact string we store in `doc_comment`.
///
/// Example: `[tags: json="name" db="user_name" validate="required"]`
pub(super) fn format_tags(tags: &[(String, String)]) -> String {
    let pairs: Vec<String> = tags
        .iter()
        .map(|(k, v)| format!("{k}=\"{v}\""))
        .collect();
    format!("[tags: {}]", pairs.join(" "))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_json_tag() {
        let tags = parse_struct_tags("`json:\"name\"`");
        assert_eq!(tags, vec![("json".to_string(), "name".to_string())]);
    }

    #[test]
    fn parse_multiple_tags() {
        let tags = parse_struct_tags(r#"`json:"name" db:"user_name" validate:"required"`"#);
        assert_eq!(tags.len(), 3);
        assert_eq!(tags[0], ("json".to_string(), "name".to_string()));
        assert_eq!(tags[1], ("db".to_string(), "user_name".to_string()));
        assert_eq!(tags[2], ("validate".to_string(), "required".to_string()));
    }

    #[test]
    fn parse_tag_with_options() {
        // json:"email,omitempty" — the comma-separated options are part of the value
        let tags = parse_struct_tags(r#"`json:"email,omitempty"`"#);
        assert_eq!(tags, vec![("json".to_string(), "email,omitempty".to_string())]);
    }

    #[test]
    fn parse_empty_tag() {
        let tags = parse_struct_tags("``");
        assert!(tags.is_empty());
    }

    #[test]
    fn parse_dash_tag() {
        // json:"-" means omit from JSON
        let tags = parse_struct_tags(r#"`json:"-"`"#);
        assert_eq!(tags, vec![("json".to_string(), "-".to_string())]);
    }

    #[test]
    fn format_tags_produces_bracketed_string() {
        let tags = vec![
            ("json".to_string(), "name".to_string()),
            ("db".to_string(), "user_name".to_string()),
        ];
        let formatted = format_tags(&tags);
        assert_eq!(formatted, r#"[tags: json="name" db="user_name"]"#);
    }
}
