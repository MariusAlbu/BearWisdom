// =============================================================================
// languages/typescript/connectors_graphql.rs — per-file GraphQL flow scan
//
// Per-file TS/JS GraphQL scan: SDL `type Query/Mutation/Subscription` blocks
// produce Producer emissions, Apollo resolver maps and type-graphql
// `@Query() / @Mutation()` decorators produce Consumer emissions, all keyed
// by GraphQL field name.
// =============================================================================

use regex::Regex;

/// Per-file TS/JS GraphQL scan: SDL type blocks produce Producer emissions,
/// Apollo resolver maps and type-graphql `@Query() / @Mutation()` decorators
/// produce Consumer emissions, all keyed by GraphQL field name.
pub fn extract_typescript_graphql(
    source: &str,
) -> Vec<(u32, crate::indexer::resolve::flow_emit::FlowEmission)> {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};

    let re_type_block = Regex::new(r"type\s+(Query|Mutation|Subscription)\s*\{")
        .expect("gql type block regex");
    let re_field = Regex::new(r"^\s+(\w+)(?:\([^)]*\))?\s*:")
        .expect("gql field regex");
    let re_resolver_key = Regex::new(
        r#"['"`]?(\w+)['"`]?\s*:\s*(?:async\s+)?\([^)]*\)\s*=>"#,
    )
    .expect("ts graphql resolver key regex");
    let re_typegraphql_op = Regex::new(
        r#"@(?:Query|Mutation|Subscription)\s*\(\s*\([^)]*\)\s*=>\s*\w+\s*\)"#,
    )
    .expect("ts type-graphql op regex");

    // Fast filter: no GraphQL markers → no points.
    if !re_type_block.is_match(source)
        && !source.contains("@Query")
        && !source.contains("@Mutation")
    {
        return Vec::new();
    }

    let mut out: Vec<(u32, FlowEmission)> = Vec::new();
    let mut in_op_block = false;
    let mut brace_depth: u32 = 0;

    for (line_idx, line_text) in source.lines().enumerate() {
        let line_no = (line_idx + 1) as u32;

        if re_type_block.is_match(line_text) {
            in_op_block = true;
            brace_depth = 1;
            continue;
        }

        if in_op_block {
            for ch in line_text.chars() {
                match ch {
                    '{' => brace_depth += 1,
                    '}' => {
                        if brace_depth > 0 { brace_depth -= 1; }
                    }
                    _ => {}
                }
            }
            if brace_depth == 0 {
                in_op_block = false;
                continue;
            }
            if brace_depth == 1 {
                if let Some(cap) = re_field.captures(line_text) {
                    let field_name = cap[1].to_string();
                    if !field_name.starts_with("__") {
                        out.push((
                            line_no,
                            FlowEmission::NamedChannel {
                                kind: NamedChannelKind::GraphQLOp,
                                name: field_name,
                                role: ChannelRole::Producer,
                                method: None,
                                streaming: None,
                            },
                        ));
                    }
                }
            }
            continue;
        }

        for cap in re_resolver_key.captures_iter(line_text) {
            let name = cap[1].to_string();
            if matches!(
                name.as_str(),
                "then" | "catch" | "finally" | "map" | "filter" | "reduce"
            ) {
                continue;
            }
            out.push((
                line_no,
                FlowEmission::NamedChannel {
                    kind: NamedChannelKind::GraphQLOp,
                    name,
                    role: ChannelRole::Consumer,
                    method: None,
                    streaming: None,
                },
            ));
        }

        if re_typegraphql_op.is_match(line_text) {
            out.push((
                line_no,
                FlowEmission::NamedChannel {
                    kind: NamedChannelKind::GraphQLOp,
                    name: String::new(),
                    role: ChannelRole::Consumer,
                    method: None,
                    streaming: None,
                },
            ));
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::indexer::resolve::flow_emit::{FlowEmission, NamedChannelKind};

    #[test]
    fn graphql_sdl_type_block_emits_producer() {
        let src = r#"
const Schema = gql`
  type Query {
    me: User
  }
`
"#;
        let points = extract_typescript_graphql(src);
        assert!(points.iter().any(|(_, e)| matches!(
            e,
            FlowEmission::NamedChannel {
                kind: NamedChannelKind::GraphQLOp,
                ..
            }
        )));
    }
}
