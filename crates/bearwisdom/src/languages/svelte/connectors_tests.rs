use super::*;

fn name_of(emission: &FlowEmission) -> &str {
    match emission {
        FlowEmission::NamedChannel { name, .. } => name.as_str(),
        _ => "",
    }
}

fn role_of(emission: &FlowEmission) -> ChannelRole {
    match emission {
        FlowEmission::NamedChannel { role, .. } => *role,
        _ => panic!("not NamedChannel"),
    }
}

#[test]
fn returns_empty_without_gql_markers() {
    assert!(extract_svelte_graphql_points("<script>let x = 1;</script>").is_empty());
}

#[test]
fn emits_schema_starts_from_type_blocks() {
    let src = "type Query {\n  me: User\n}\n";
    let points = extract_svelte_graphql_points(src);
    assert_eq!(points.len(), 1);
    assert_eq!(name_of(&points[0].1), "me");
    assert_eq!(role_of(&points[0].1), ChannelRole::Producer);
}

#[test]
fn emits_resolver_stops() {
    let src = "gql`\nconst resolvers = {\n  myField: async (a, b) => null,\n};\n";
    let points = extract_svelte_graphql_points(src);
    assert!(
        points.iter().any(|(_, e)| name_of(e) == "myField" && role_of(e) == ChannelRole::Consumer),
        "expected myField Consumer, got {points:?}",
    );
}
