use super::*;
use crate::indexer::resolve::flow_emit::{FlowEmission, NamedChannelKind};

#[test]
fn graphql_strawberry_field() {
    let src = r#"
@strawberry.field
def me(): pass
"#;
    let points = extract_python_graphql(src);
    assert!(points.iter().any(|(_, e)| matches!(
        e,
        FlowEmission::NamedChannel {
            kind: NamedChannelKind::GraphQLOp,
            ..
        }
    )));
}
