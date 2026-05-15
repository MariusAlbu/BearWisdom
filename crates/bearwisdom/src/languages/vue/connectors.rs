// =============================================================================
// languages/vue/connectors.rs — Vue GraphQL FlowEmission detection
//
// Scans `.vue` sources for embedded GraphQL SDL blocks and Apollo resolver
// maps, emitting Producer / Consumer `FlowEmission::NamedChannel { kind:
// GraphQLOp, .. }` entries that the resolve loop pairs into `flow_edges`.
// =============================================================================

use regex::Regex;

use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};

/// Scan a `.vue` source for embedded GraphQL schema definitions and
/// resolver-map entries.
pub fn extract_vue_graphql_points(source: &str) -> Vec<(u32, FlowEmission)> {
    let re_type_block =
        Regex::new(r"type\s+(Query|Mutation|Subscription)\s*\{").expect("vue gql type block regex");
    let re_field = Regex::new(r"^\s+(\w+)(?:\([^)]*\))?\s*:").expect("vue gql field regex");
    let re_resolver_key =
        Regex::new(r#"['"`]?(\w+)['"`]?\s*:\s*(?:async\s+)?\([^)]*\)\s*=>"#)
            .expect("vue graphql resolver key regex");

    if !re_type_block.is_match(source) && !source.contains("gql`") {
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
    }
    out
}

#[cfg(test)]
#[path = "connectors_tests.rs"]
mod tests;
