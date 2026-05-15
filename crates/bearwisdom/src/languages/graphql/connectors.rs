// =============================================================================
// languages/graphql/connectors.rs — GraphQL schema FlowEmission detection
//
// Parses .graphql / .gql files for Query / Mutation / Subscription type blocks
// and emits Producer `FlowEmission::NamedChannel { kind: GraphQLOp, .. }` for
// each field. Called at parse time from `indexer/full.rs::parse_file`, which
// stores the result on `ParsedFile.plugin_flow_emissions`; the resolve loop's
// `plugin_flow_emissions_to_emissions` adapter flushes them alongside
// resolver-emitted flows.
// =============================================================================

use regex::Regex;

use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};

/// Scan a `.graphql` / `.gql` source for top-level `type Query { … }`,
/// `type Mutation { … }`, `type Subscription { … }` blocks and emit a
/// Producer `FlowEmission::NamedChannel { kind: GraphQLOp, .. }` per field.
pub fn extract_schema_starts(source: &str) -> Vec<(u32, FlowEmission)> {
    let re_type_block =
        Regex::new(r"type\s+(Query|Mutation|Subscription)\s*\{").expect("graphql type block regex");
    let re_field =
        Regex::new(r"^\s+(\w+)(?:\([^)]*\))?\s*:").expect("graphql field regex");

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

        if !in_op_block {
            continue;
        }

        for ch in line_text.chars() {
            match ch {
                '{' => brace_depth += 1,
                '}' => {
                    if brace_depth > 0 {
                        brace_depth -= 1;
                    }
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
                if field_name.starts_with("__") {
                    continue;
                }
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

    out
}

#[cfg(test)]
#[path = "connectors_tests.rs"]
mod tests;
