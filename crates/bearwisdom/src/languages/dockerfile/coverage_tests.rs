// =============================================================================
// dockerfile/coverage_tests.rs — Node-kind coverage tests for the Dockerfile extractor
//
// symbol_node_kinds:
//   from_instruction, arg_instruction, env_instruction, label_instruction
//
// ref_node_kinds:
//   copy_instruction, from_instruction
// =============================================================================

use super::extract;
use crate::types::{EdgeKind, SymbolKind};

// ---------------------------------------------------------------------------
// symbol_node_kinds
// ---------------------------------------------------------------------------

/// from_instruction with AS alias → SymbolKind::Class  (named build stage)
#[test]
fn cov_from_instruction_named_stage_emits_class() {
    let r = extract::extract("FROM node:18 AS builder");
    let sym = r.symbols.iter().find(|s| s.name == "builder");
    assert!(sym.is_some(), "expected Class 'builder' from FROM...AS; got: {:?}", r.symbols);
    assert_eq!(sym.unwrap().kind, SymbolKind::Class);
}

/// from_instruction without alias → SymbolKind::Variable  (unnamed stage)
#[test]
fn cov_from_instruction_unnamed_stage_emits_variable() {
    let r = extract::extract("FROM ubuntu:22.04");
    let sym = r.symbols.iter().find(|s| s.kind == SymbolKind::Variable);
    assert!(sym.is_some(), "expected Variable from unnamed FROM stage; got: {:?}", r.symbols);
}

/// from_instruction → EdgeKind::Imports  (base image reference)
#[test]
fn cov_from_instruction_emits_imports_ref() {
    let r = extract::extract("FROM node:18 AS builder");
    let imports: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::Imports)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(
        imports.iter().any(|n| n.contains("node")),
        "expected Imports ref to base image 'node:18'; got: {imports:?}"
    );
}

/// arg_instruction → SymbolKind::Variable  (ARG declaration)
#[test]
fn cov_arg_instruction_emits_variable() {
    let src = "FROM node:18 AS base\nARG NODE_VERSION=18\n";
    let r = extract::extract(src);
    let sym = r.symbols.iter().find(|s| s.name == "NODE_VERSION");
    assert!(sym.is_some(), "expected Variable 'NODE_VERSION' from ARG; got: {:?}", r.symbols);
    assert_eq!(sym.unwrap().kind, SymbolKind::Variable);
}

/// env_instruction → SymbolKind::Variable  (ENV declaration via env_pair)
#[test]
fn cov_env_instruction_emits_variable() {
    let src = "FROM node:18 AS base\nENV PORT=3000\n";
    let r = extract::extract(src);
    let sym = r.symbols.iter().find(|s| s.name == "PORT");
    assert!(sym.is_some(), "expected Variable 'PORT' from ENV; got: {:?}", r.symbols);
    assert_eq!(sym.unwrap().kind, SymbolKind::Variable);
}

/// label_instruction with key=value syntax → SymbolKind::Variable per label_pair
#[test]
fn cov_label_instruction_emits_variable() {
    let src = "FROM node:18 AS base\nLABEL maintainer=\"dev@example.com\"\n";
    let r = extract::extract(src);
    let sym = r.symbols.iter().find(|s| s.name == "maintainer");
    assert!(sym.is_some(), "expected Variable 'maintainer' from LABEL; got: {:?}", r.symbols);
    assert_eq!(sym.unwrap().kind, SymbolKind::Variable);
}

/// label_instruction with multiple pairs → one Variable per pair
#[test]
fn cov_label_instruction_multiple_pairs() {
    let src = "FROM node:18 AS base\nLABEL version=\"1.0\" description=\"My app\"\n";
    let r = extract::extract(src);
    assert!(
        r.symbols.iter().any(|s| s.name == "version"),
        "expected 'version' label; got: {:?}", r.symbols
    );
    assert!(
        r.symbols.iter().any(|s| s.name == "description"),
        "expected 'description' label; got: {:?}", r.symbols
    );
}

/// label_instruction with double-quoted key → SymbolKind::Variable
#[test]
fn cov_label_instruction_quoted_key() {
    // `LABEL "docker_run_flags"="-d ..."` — key is a double_quoted_string
    let src = "FROM alpine:latest\nLABEL \"registry_image\"=\"r.j3ss.co/couchpotato\"\n";
    let r = extract::extract(src);
    let sym = r.symbols.iter().find(|s| s.name == "registry_image");
    assert!(sym.is_some(), "expected Variable 'registry_image' from quoted-key LABEL; got: {:?}", r.symbols);
    assert_eq!(sym.unwrap().kind, SymbolKind::Variable);
}

// ---------------------------------------------------------------------------
// ref_node_kinds
// ---------------------------------------------------------------------------

/// copy_instruction with --from param → EdgeKind::Calls  (cross-stage copy)
#[test]
fn cov_copy_instruction_from_param_emits_calls() {
    let src = "FROM node:18 AS builder\nRUN npm build\nFROM nginx:alpine AS prod\nCOPY --from=builder /app/dist /usr/share/nginx/html\n";
    let r = extract::extract(src);
    let calls: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::Calls)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(
        calls.contains(&"builder"),
        "expected Calls ref to 'builder' from COPY --from=builder; got: {calls:?}"
    );
}

/// copy_instruction without --from → no crash (regular COPY)
#[test]
fn cov_copy_instruction_regular_does_not_crash() {
    let src = "FROM node:18 AS base\nCOPY package.json .\n";
    let r = extract::extract(src);
    // Regular COPY without --from should produce no Calls refs but not crash.
    let _ = r;
}

/// COPY --from=<N> (numeric stage index) → Calls edge resolved to the stage name
#[test]
fn cov_copy_instruction_numeric_from_resolves_to_stage_name() {
    // Stage 0 is "builder", stage 1 is "prod".
    // COPY --from=0 should produce a Calls ref targeting "builder".
    let src = "FROM node:18 AS builder\nRUN npm run build\nFROM nginx:alpine AS prod\nCOPY --from=0 /app/dist /usr/share/nginx/html\n";
    let r = extract::extract(src);
    let calls: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::Calls)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(
        calls.contains(&"builder"),
        "expected Calls ref to 'builder' from COPY --from=0; got: {calls:?}"
    );
}
