use super::*;

#[test]
fn parses_default_output_block() {
    let schema = r#"
generator client {
  provider = "prisma-client-js"
  output   = "../generated/db"
}

datasource db {
  provider = "postgresql"
}

model User {
  id Int @id
}
"#;
    let outs = parse_generator_outputs(schema);
    assert_eq!(outs, vec!["../generated/db".to_string()]);
}

#[test]
fn handles_multiple_generators() {
    let schema = r#"
generator client {
  output = "./client"
}

generator zod {
  output = "./zod-out"
}
"#;
    let outs = parse_generator_outputs(schema);
    assert_eq!(outs, vec!["./client".to_string(), "./zod-out".to_string()]);
}

#[test]
fn returns_empty_when_no_generator_block() {
    let outs = parse_generator_outputs("model X { id Int @id }\n");
    assert!(outs.is_empty());
}

#[test]
fn falls_back_to_default_when_output_missing() {
    // generator block exists but no `output =` field — default path
    // (`node_modules/.prisma/client`) is added unconditionally by the
    // caller side, so parse_generator_outputs itself returns empty.
    let schema = "generator client {\n  provider = \"prisma-client-js\"\n}\n";
    let outs = parse_generator_outputs(schema);
    assert!(outs.is_empty());
}
