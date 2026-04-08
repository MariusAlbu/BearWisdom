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

// ---------------------------------------------------------------------------
// ENTRYPOINT and CMD → SymbolKind::Function
// ---------------------------------------------------------------------------

/// entrypoint_instruction → SymbolKind::Function named "ENTRYPOINT"
#[test]
fn cov_entrypoint_instruction_emits_function() {
    let src = "FROM node:18 AS base\nENTRYPOINT [\"node\", \"server.js\"]\n";
    let r = extract::extract(src);
    let sym = r.symbols.iter().find(|s| s.name == "ENTRYPOINT");
    assert!(sym.is_some(), "expected Function 'ENTRYPOINT'; got: {:?}", r.symbols);
    assert_eq!(sym.unwrap().kind, SymbolKind::Function);
}

/// cmd_instruction → SymbolKind::Function named "CMD"
#[test]
fn cov_cmd_instruction_emits_function() {
    let src = "FROM python:3.11 AS base\nCMD [\"python\", \"app.py\"]\n";
    let r = extract::extract(src);
    let sym = r.symbols.iter().find(|s| s.name == "CMD");
    assert!(sym.is_some(), "expected Function 'CMD'; got: {:?}", r.symbols);
    assert_eq!(sym.unwrap().kind, SymbolKind::Function);
}

/// entrypoint_instruction shell form → SymbolKind::Function named "ENTRYPOINT"
#[test]
fn cov_entrypoint_instruction_shell_form_emits_function() {
    let src = "FROM alpine:latest AS base\nENTRYPOINT /usr/bin/start.sh\n";
    let r = extract::extract(src);
    assert!(
        r.symbols.iter().any(|s| s.name == "ENTRYPOINT" && s.kind == SymbolKind::Function),
        "expected Function 'ENTRYPOINT' from shell form; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// cmd_instruction and entrypoint_instruction both present → two Function symbols
#[test]
fn cov_cmd_and_entrypoint_both_emit_function() {
    let src = "FROM node:18 AS base\nENTRYPOINT [\"node\"]\nCMD [\"index.js\"]\n";
    let r = extract::extract(src);
    assert!(
        r.symbols.iter().any(|s| s.name == "ENTRYPOINT"),
        "expected ENTRYPOINT symbol; got: {:?}", r.symbols
    );
    assert!(
        r.symbols.iter().any(|s| s.name == "CMD"),
        "expected CMD symbol; got: {:?}", r.symbols
    );
}

// ---------------------------------------------------------------------------
// Test stage detection
// ---------------------------------------------------------------------------

/// from_instruction with AS test (case-insensitive) → SymbolKind::Test
#[test]
fn cov_from_instruction_test_stage_emits_test() {
    let src = "FROM node:18 AS test\nRUN npm test\n";
    let r = extract::extract(src);
    let sym = r.symbols.iter().find(|s| s.name == "test");
    assert!(sym.is_some(), "expected Test stage symbol; got: {:?}", r.symbols);
    assert_eq!(sym.unwrap().kind, SymbolKind::Test);
}

// ---------------------------------------------------------------------------
// Inherits edge (from_instruction)
// ---------------------------------------------------------------------------

/// from_instruction → EdgeKind::Inherits from stage to base image
#[test]
fn ref_from_instruction_inherits_edge() {
    let src = "FROM ubuntu:22.04 AS base\n";
    let r = extract::extract(src);
    let inherits: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::Inherits)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(
        inherits.iter().any(|n| n.contains("ubuntu")),
        "expected Inherits ref to base image 'ubuntu:22.04'; got: {inherits:?}"
    );
}

// ---------------------------------------------------------------------------
// ARG without default value
// ---------------------------------------------------------------------------

/// arg_instruction with no default → SymbolKind::Variable (name only)
#[test]
fn cov_arg_instruction_no_default_emits_variable() {
    let src = "FROM node:18 AS base\nARG BUILDKIT_INLINE_CACHE\n";
    let r = extract::extract(src);
    let sym = r.symbols.iter().find(|s| s.name == "BUILDKIT_INLINE_CACHE");
    assert!(sym.is_some(), "expected Variable 'BUILDKIT_INLINE_CACHE' from ARG without default; got: {:?}", r.symbols);
    assert_eq!(sym.unwrap().kind, SymbolKind::Variable);
}

/// arg_instruction before first FROM (global scope) → SymbolKind::Variable
#[test]
fn cov_global_arg_instruction_before_from_emits_variable() {
    let src = "ARG BASE_IMAGE=node:18\nFROM $BASE_IMAGE AS app\n";
    let r = extract::extract(src);
    let sym = r.symbols.iter().find(|s| s.name == "BASE_IMAGE");
    assert!(sym.is_some(), "expected Variable 'BASE_IMAGE' from global ARG before FROM; got: {:?}", r.symbols);
    assert_eq!(sym.unwrap().kind, SymbolKind::Variable);
}

/// env_instruction with multiple env_pairs → one Variable per pair
#[test]
fn cov_env_instruction_multiple_pairs() {
    let src = "FROM node:18 AS base\nENV NODE_ENV=production PORT=3000\n";
    let r = extract::extract(src);
    assert!(
        r.symbols.iter().any(|s| s.name == "NODE_ENV"),
        "expected 'NODE_ENV' from ENV; got: {:?}", r.symbols
    );
    assert!(
        r.symbols.iter().any(|s| s.name == "PORT"),
        "expected 'PORT' from ENV; got: {:?}", r.symbols
    );
}
