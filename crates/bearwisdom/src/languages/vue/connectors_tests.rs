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
fn emits_nothing_without_gql_marker() {
    assert!(extract_vue_graphql_points("<template/><script>1</script>").is_empty());
}

#[test]
fn emits_start_from_schema_block() {
    let src = "type Mutation {\n  createUser(input: U): U\n}\n";
    let points = extract_vue_graphql_points(src);
    assert_eq!(points.len(), 1);
    assert_eq!(name_of(&points[0].1), "createUser");
    assert_eq!(role_of(&points[0].1), ChannelRole::Producer);
}
