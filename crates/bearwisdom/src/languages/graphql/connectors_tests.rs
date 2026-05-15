use super::*;

fn name_of(emission: &FlowEmission) -> &str {
    match emission {
        FlowEmission::NamedChannel { name, .. } => name.as_str(),
        _ => "",
    }
}

#[test]
fn emits_one_start_per_query_field() {
    let src = r#"
type Query {
  user(id: ID!): User
  users: [User!]!
}

type Mutation {
  createUser(input: UserInput!): User
}
"#;
    let points = extract_schema_starts(src);
    assert_eq!(points.len(), 3);
    assert_eq!(name_of(&points[0].1), "user");
    assert_eq!(name_of(&points[1].1), "users");
    assert_eq!(name_of(&points[2].1), "createUser");
    for (_, e) in &points {
        match e {
            FlowEmission::NamedChannel { kind, role, .. } => {
                assert_eq!(*kind, NamedChannelKind::GraphQLOp);
                assert_eq!(*role, ChannelRole::Producer);
            }
            _ => panic!("expected NamedChannel"),
        }
    }
}

#[test]
fn ignores_introspection_fields() {
    let src = "type Query {\n  __schema: Schema\n  real: String\n}\n";
    let points = extract_schema_starts(src);
    assert_eq!(points.len(), 1);
    assert_eq!(name_of(&points[0].1), "real");
}

#[test]
fn empty_source_produces_no_points() {
    assert!(extract_schema_starts("").is_empty());
}
