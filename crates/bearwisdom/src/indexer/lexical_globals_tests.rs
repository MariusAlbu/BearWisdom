use super::*;

fn capture_source(source: &str) -> (LexicalBindings, Capture) {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
        .unwrap();
    let tree = parser.parse(source, None).unwrap();
    assert!(
        !tree.root_node().has_error(),
        "{}",
        tree.root_node().to_sexp()
    );
    let graph = super::super::capture(
        tree.root_node(),
        source.as_bytes(),
        "ts",
        &mut vec![],
        &[],
        crate::indexer::flow_bindings::BindingSymbols::CorrelateOnly,
    )
    .unwrap();
    let captured = graph.globals.clone().unwrap();
    (graph, captured)
}

#[test]
fn empty_exports_and_side_effect_imports_isolate_all_root_declarations() {
    for source in [
        "export {}; interface Box<T> {}",
        "import './side'; interface Box<T> {}",
        "export interface Box<T> {}",
    ] {
        let (_, globals) = capture_source(source);
        assert!(globals.isolated && globals.complete);
        assert!(globals.roots.is_empty());
    }
}

#[test]
fn explicit_augmentation_is_separate_from_module_locals_and_script_roots() {
    let (graph, globals) =
        capture_source("export {}; interface Hidden {} declare global { interface Box<T> {} }");
    assert!(globals.complete);
    assert!(globals.roots.is_empty());
    assert_eq!(globals.augmentations.len(), 1);
    assert_eq!(globals.augmentations[0].name, graph.name_id("Box").unwrap());
    assert_eq!(
        globals.augmentations[0].parameters,
        [graph.name_id("T").unwrap()]
    );
    assert!(globals.augmentations[0].plain_parameters);
    let (_, invalid) = capture_source("declare global { interface Box {} }");
    assert!(
        !invalid.isolated && !invalid.augmentations.is_empty(),
        "program binding must check augmentation context"
    );
}

#[test]
fn only_source_attested_root_declarations_contribute_and_complex_generics_are_marked() {
    let (graph, globals) = capture_source("interface Box<T extends object> {} declare var Factory: unknown; function run() { class Hidden {} } namespace N { interface Nested {} }");
    assert!(globals.complete);
    assert!(!globals
        .roots
        .iter()
        .any(|p| [graph.name_id("Hidden"), graph.name_id("Nested")].contains(&Some(p.name))));
    assert!(
        !globals
            .roots
            .iter()
            .find(|p| p.name == graph.name_id("Box").unwrap())
            .unwrap()
            .plain_parameters
    );
    assert!(globals
        .roots
        .iter()
        .any(|p| p.name == graph.name_id("Factory").unwrap() && !p.type_space));
    assert!(globals
        .roots
        .iter()
        .any(|p| p.name == graph.name_id("N").unwrap() && p.slot.is_none()));
}

#[test]
fn unsupported_nested_augmentations_are_not_silently_ignored_as_module_locals() {
    let (_, globals) = capture_source(
        "export {}; declare namespace Env { declare global { interface Catalog {} } }",
    );
    assert!(globals.isolated);
    assert!(!globals.complete);
    assert!(globals.roots.is_empty() && globals.augmentations.is_empty());
}

#[test]
fn function_and_block_containment_cannot_masquerade_as_top_level_module_ownership() {
    for source in [
        "export {}; function run() { declare global { interface Leaked {} } }",
        "export {}; { declare global { interface Leaked {} } }",
        "function run() { declare module 'provider' { global { interface Leaked {} } } }",
        "{ declare module 'provider' { global { interface Leaked {} } } }",
    ] {
        let (graph, globals) = capture_source(source);
        assert!(
            !globals.complete,
            "illegal lexical container was admitted: {source}"
        );
        assert!(
            globals.augmentations.is_empty(),
            "global contributions leaked: {source}"
        );
        assert!(graph.module.units.iter().any(|unit| !unit.container_valid));
    }
}

#[test]
#[ignore = "Read-only diagnosis of pinned configured-program provider barriers"]
fn diagnose_pinned_global_capture_barriers() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let manifest: crate::resolution_oracle::project::ProjectManifest =
        serde_json::from_slice(
            &std::fs::read(root.join(
                "resolution-documents/2026-09-07-query-core-configured-compiler-manifest.json",
            ))
            .unwrap(),
        )
        .unwrap();
    manifest.verify_inputs().unwrap();
    let report: serde_json::Value =
        serde_json::from_slice(
            &std::fs::read(root.join(
                "resolution-documents/2026-09-07-query-core-large-source-program-report.json",
            ))
            .unwrap(),
        )
        .unwrap();
    let gaps: std::collections::HashSet<u64> = report["configured_source_gaps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|g| g["file"].as_u64().unwrap())
        .collect();
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
        .unwrap();
    for file in manifest
        .files
        .iter()
        .filter(|f| gaps.contains(&(f.id.0 as u64)))
    {
        let source = std::fs::read_to_string(&file.path).unwrap();
        let tree = parser.parse(&source, None).unwrap();
        let root = tree.root_node();
        let mut nodes = vec![root];
        let mut errors = Vec::new();
        while let Some(node) = nodes.pop() {
            if node.is_error() || node.is_missing() {
                errors.push((
                    node.kind(),
                    node.start_byte(),
                    node.utf8_text(source.as_bytes())
                        .unwrap()
                        .chars()
                        .take(100)
                        .collect::<String>(),
                ));
            } else if node.has_error() {
                let mut cursor = node.walk();
                nodes.extend(node.children(&mut cursor));
            }
        }
        let mut cursor = root.walk();
        let shapes: Vec<_> = root
            .named_children(&mut cursor)
            .filter(|n| !n.is_extra())
            .map(|node| {
                let mut inner = node;
                if node.kind() == "ambient_declaration" {
                    inner = node.named_child(0).unwrap_or(node);
                }
                (
                    node.kind(),
                    inner.kind(),
                    inner.child_by_field_name("name").map(|n| n.kind()),
                    nested_augmentation(
                        node,
                        &crate::languages::typescript::flow::TS_LEXICAL_SYNTAX,
                    ),
                )
            })
            .collect();
        println!(
            "{}",
            serde_json::json!({"file":file.index_path,"parser_errors":errors,"root_shapes":shapes})
        );
    }
}

fn provider_grammar_tree(source: &str, language: tree_sitter::Language) -> tree_sitter::Tree {
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&language).unwrap();
    let tree = parser.parse(source, None).unwrap();
    assert!(
        !tree.root_node().has_error(),
        "{source}\n{}",
        tree.root_node().to_sexp()
    );
    tree
}

#[test]
fn provider_grammar_bare_global_block_retains_augmentation_node() {
    let source = "declare module 'provider' { global { interface Catalog { read(): string; } } }";
    for language in [
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT,
        tree_sitter_typescript::LANGUAGE_TSX,
    ] {
        let tree = provider_grammar_tree(source, language.into());
        let module = tree
            .root_node()
            .named_child(0)
            .unwrap()
            .named_child(0)
            .unwrap();
        let global = module
            .child_by_field_name("body")
            .unwrap()
            .named_child(0)
            .unwrap();
        assert_eq!(global.kind(), "ambient_declaration");
        assert_eq!(global.child(0).unwrap().kind(), "global");
        // A real global contributor must remain visible to the current barrier.
        assert!(nested_augmentation(
            module,
            &crate::languages::typescript::flow::TS_LEXICAL_SYNTAX
        ));
    }
}

#[test]
fn provider_grammar_keyof_readonly_mapped_type_retains_operand() {
    let source =
        "interface Catalog { readonly keys: { readonly [K in keyof readonly any[]]?: boolean }; }";
    for language in [
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT,
        tree_sitter_typescript::LANGUAGE_TSX,
    ] {
        let tree = provider_grammar_tree(source, language.into());
        assert!(tree
            .root_node()
            .to_sexp()
            .contains("(index_type_query (readonly_type (array_type"));
    }
}

#[test]
fn provider_grammar_generic_import_type_retains_member_and_type_arguments() {
    let source = "type Stream<T> = typeof globalThis extends { onmessage: any } ? {} : import('provider').Stream<T>;";
    for language in [
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT,
        tree_sitter_typescript::LANGUAGE_TSX,
    ] {
        let tree = provider_grammar_tree(source, language.into());
        let sexp = tree.root_node().to_sexp();
        assert!(
            sexp.contains("type_arguments: (type_arguments (type_identifier))"),
            "{sexp}"
        );
        assert!(
            sexp.contains("(member_expression object: (call_expression function: (import)"),
            "{sexp}"
        );
    }
}

#[test]
#[ignore = "Read-only full-source parser gate for the pinned compiler program"]
fn pinned_provider_grammar_has_no_parse_errors() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let manifest: crate::resolution_oracle::project::ProjectManifest =
        serde_json::from_slice(
            &std::fs::read(root.join(
                "resolution-documents/2026-09-07-query-core-configured-compiler-manifest.json",
            ))
            .unwrap(),
        )
        .unwrap();
    manifest.verify_inputs().unwrap();
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
        .unwrap();
    let mut failures = Vec::new();
    for file in &manifest.files {
        let source = std::fs::read(&file.path).unwrap();
        let tree = parser.parse(&source, None).unwrap();
        if tree.root_node().has_error() {
            failures.push(file.index_path.clone());
        }
    }
    println!(
        "Pinned compiler program: {} supplied sources, {} parser failures",
        manifest.files.len(),
        failures.len()
    );
    assert!(failures.is_empty(), "{failures:#?}");
    manifest.verify_inputs().unwrap();
}

#[test]
#[ignore = "Read-only diagnosis of remaining source-owned module capture barriers"]
fn diagnose_current_program_module_capture_barriers() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let manifest: crate::resolution_oracle::project::ProjectManifest =
        serde_json::from_slice(
            &std::fs::read(root.join(
                "resolution-documents/2026-09-07-query-core-configured-compiler-manifest.json",
            ))
            .unwrap(),
        )
        .unwrap();
    manifest.verify_inputs().unwrap();
    let mut failures = 0;
    for file in &manifest.files {
        let source = std::fs::read_to_string(&file.path).unwrap();
        let (graph, globals) = capture_source(&source);
        let incomplete: Vec<_> = graph.module.units.iter().filter(|u| !u.complete || !u.container_valid).map(|u|
            serde_json::json!({"unit":u.id.0,"parent":u.parent.0,"kind":format!("{:?}",u.kind),
                "name":u.name.and_then(|id| graph.interned_names().find_map(|(key, name)| (key == id).then_some(name))),
                "complete":u.complete,"container_valid":u.container_valid,
                "line":source[..u.range.start as usize].bytes().filter(|&b| b == b'\n').count() + 1})).collect();
        if globals.complete && graph.module.complete && incomplete.is_empty() {
            continue;
        }
        failures += 1;
        println!(
            "{}",
            serde_json::json!({"path":file.index_path,"global_complete":globals.complete,
            "root_module_complete":graph.module.complete,"incomplete_units":incomplete})
        );
    }
    println!(
        "Supplied {} source files; {failures} files with incomplete source-module evidence",
        manifest.files.len()
    );
    manifest.verify_inputs().unwrap();
}
