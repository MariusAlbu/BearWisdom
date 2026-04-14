//! TypeScript / JavaScript embedded regions — tagged template literals.
//!
//! Detects string DSLs embedded as tagged template literals:
//!
//!   * `` gql`…` `` / `` graphql`…` ``        → `graphql`
//!   * `` sql`…` ``                           → `sql`
//!   * `` css`…` `` / `` styled.div`…` ``     → `css`
//!   * `` html`…` ``                          → `html`
//!
//! `${expr}` interpolations are recorded as `holes` (byte spans inside
//! `EmbeddedRegion::text`) so the sub-language grammar sees valid-shaped
//! text where the expression used to be (the emitter keeps whatever the
//! source had between `${` and `}` but its refs are dropped via holes).

use crate::types::{EmbeddedOrigin, EmbeddedRegion, Span};
use tree_sitter::{Node, Parser};

/// Detect all tagged-template-literal embedded regions in a TS/JS source.
/// `lang_id` selects the right grammar (typescript / tsx); JS callers pass
/// "javascript" which falls back to TS.
pub fn detect_regions(source: &str, lang_id: &str) -> Vec<EmbeddedRegion> {
    let grammar: tree_sitter::Language = match lang_id {
        "tsx" => tree_sitter_typescript::LANGUAGE_TSX.into(),
        "javascript" | "jsx" => tree_sitter_javascript::LANGUAGE.into(),
        _ => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
    };
    let mut parser = Parser::new();
    if parser.set_language(&grammar).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source, None) else {
        return Vec::new();
    };
    let mut regions = Vec::new();
    walk(&tree.root_node(), source, &mut regions);
    regions
}

fn walk(node: &Node, source: &str, regions: &mut Vec<EmbeddedRegion>) {
    // tree-sitter-typescript parses `gql\`...\`` as `call_expression` where
    // `arguments` points at the `template_string` (rather than a separate
    // `tagged_template_expression` node). Both shapes are handled.
    let kind = node.kind();
    if kind == "tagged_template_expression" || kind == "call_expression" {
        if let Some(r) = try_extract(node, source) {
            regions.push(r);
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk(&child, source, regions);
    }
}

fn try_extract(node: &Node, source: &str) -> Option<EmbeddedRegion> {
    let function = node.child_by_field_name("function")?;
    let tag_text = source.get(function.start_byte()..function.end_byte())?;
    let tag_name = canonical_tag_name(tag_text);
    let language_id = tag_to_language(tag_name)?;

    // Template shape differs between node kinds:
    //   * `tagged_template_expression` exposes `quasi` field.
    //   * `call_expression` has an `arguments` field pointing at the
    //     `template_string` directly.
    let template = node
        .child_by_field_name("quasi")
        .or_else(|| node.child_by_field_name("arguments"))?;
    if template.kind() != "template_string" {
        return None;
    }

    // The template_string spans `` ` … ` ``. Body starts after the opening
    // backtick and ends before the closing one.
    let open = template.start_byte() + 1;
    let close = template.end_byte().saturating_sub(1);
    if close <= open {
        return None;
    }
    let body = source.get(open..close)?.to_string();

    // Walk the template_string's children to find `template_substitution`
    // nodes (`${ … }`) and record their byte positions as holes in `body`.
    let mut holes: Vec<Span> = Vec::new();
    let mut cursor = template.walk();
    for child in template.children(&mut cursor) {
        if child.kind() == "template_substitution" {
            let s = child.start_byte().saturating_sub(open);
            let e = child.end_byte().saturating_sub(open);
            if e <= body.len() && s < e {
                holes.push(Span { start: s, end: e });
            }
        }
    }

    let (line_offset, col_offset) = byte_to_line_col(source, open);
    Some(EmbeddedRegion {
        language_id: language_id.to_string(),
        text: body,
        line_offset,
        col_offset,
        origin: EmbeddedOrigin::StringDsl,
        holes,
        strip_scope_prefix: None,
    })
}

/// Reduce a tag expression to its canonical tag name for dispatch:
/// `styled.div` → `styled`, `gql` → `gql`, `String.raw` → `String.raw`.
fn canonical_tag_name(tag: &str) -> &str {
    // Trim whitespace then take everything before the first `.` (so member
    // expressions like `styled.div` collapse to `styled`).
    let trimmed = tag.trim();
    match trimmed.split_once('.') {
        Some((head, _)) => head,
        None => trimmed,
    }
}

fn tag_to_language(tag: &str) -> Option<&'static str> {
    match tag {
        "gql" | "graphql" | "GraphQL" => Some("graphql"),
        "sql" | "SQL" | "Sql" => Some("sql"),
        "css" | "styled" | "createGlobalStyle" | "keyframes" | "tw" => Some("css"),
        "html" => Some("html"),
        _ => None,
    }
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
    fn gql_tag_emits_graphql_region() {
        let src = "const Q = gql`query { user { id name } }`;";
        let regions = detect_regions(src, "typescript");
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].language_id, "graphql");
        assert_eq!(regions[0].origin, EmbeddedOrigin::StringDsl);
        assert!(regions[0].text.contains("query { user"));
    }

    #[test]
    fn sql_tag_emits_sql_region_with_holes() {
        let src = "const q = sql`SELECT * FROM users WHERE id = ${userId}`;";
        let regions = detect_regions(src, "typescript");
        assert_eq!(regions.len(), 1);
        let r = &regions[0];
        assert_eq!(r.language_id, "sql");
        assert_eq!(r.holes.len(), 1, "expected one ${{userId}} hole");
        // The hole span should cover `${userId}` in `text`.
        let slice = &r.text[r.holes[0].start..r.holes[0].end];
        assert_eq!(slice, "${userId}");
    }

    #[test]
    fn styled_member_expression_tag_maps_to_css() {
        let src = "const Button = styled.div`color: red; padding: 10px;`;";
        let regions = detect_regions(src, "typescript");
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].language_id, "css");
    }

    #[test]
    fn plain_backtick_without_tag_ignored() {
        let src = "const s = `hello ${name}`;";
        let regions = detect_regions(src, "typescript");
        assert!(regions.is_empty());
    }

    #[test]
    fn unknown_tag_ignored() {
        let src = "const x = customTag`payload`;";
        let regions = detect_regions(src, "typescript");
        assert!(regions.is_empty());
    }

    #[test]
    fn multiple_interpolations_all_recorded_as_holes() {
        let src = "const q = sql`INSERT INTO t(a,b) VALUES (${a}, ${b})`;";
        let regions = detect_regions(src, "typescript");
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].holes.len(), 2);
    }

    #[test]
    fn graphql_alias_recognized() {
        let src = "const Q = graphql`{ me { id } }`;";
        let regions = detect_regions(src, "typescript");
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].language_id, "graphql");
    }
}
