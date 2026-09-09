use super::*;

#[test]
fn generic_parameter_syntax_does_not_erase_unmodeled_modifiers() {
    for (parameters, complete) in [
        ("T", true),
        ("T extends string = 'keep'", true),
        ("T, U = T", true),
        ("out T", false),
        ("in T", false),
        ("const T", false),
    ] {
        let source = format!("interface Catalog<{parameters}> {{ value: T }}");
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
            .unwrap();
        let tree = parser.parse(&source, None).unwrap();
        let node = tree.root_node().named_child(0).unwrap();
        assert_eq!(
            super::complete_type_parameters(node),
            complete,
            "{parameters}"
        );
        if tree.root_node().has_error() {
            assert!(
                !complete,
                "malformed parameters must not become complete evidence"
            );
            continue;
        }
        let (graph, _) = capture(&source);
        let declaration = graph
            .types
            .signatures
            .iter()
            .find(|s| s.syntax.type_parameters.len() > 0)
            .unwrap();
        assert_eq!(
            declaration.syntax.type_parameters_complete, complete,
            "{parameters}"
        );
    }
}

#[test]
fn overload_order_is_source_syntax_not_literal_value() {
    let (_, members) = capture("type Label = 'x'; interface Catalog { pick(x: string): void; pick(x: 'x'): void; pick(x: Label): void; pick(x: ('x')): void; pick(x: 42): void; pick(x: true): void; }");
    let group = members[0].signature.ordering.unwrap().group;
    assert!(members
        .iter()
        .all(|m| m.signature.ordering.unwrap().group == group));
    assert_eq!(
        members
            .iter()
            .map(|m| m.signature.ordering.unwrap().specialized)
            .collect::<Vec<_>>(),
        [false, true, false, false, true, true]
    );
    let signatures: Vec<_> = members.iter().map(|m| m.signature.clone()).collect();
    let json = serde_json::to_string(&signatures).unwrap();
    assert_eq!(
        serde_json::from_str::<Vec<Signature>>(&json).unwrap(),
        signatures
    );
}

fn capture(source: &str) -> (LexicalBindings, Vec<Member>) {
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
    let graph = crate::indexer::lexical::capture(
        tree.root_node(),
        source.as_bytes(),
        "ts",
        &mut vec![],
        &[],
        crate::indexer::flow_bindings::BindingSymbols::CorrelateOnly,
    )
    .unwrap();
    let part = graph
        .globals
        .as_ref()
        .unwrap()
        .roots
        .iter()
        .find(|p| p.kind == SymbolKind::Interface)
        .unwrap();
    (graph.clone(), part.surface.clone().unwrap())
}

#[test]
fn unnamed_and_computed_signatures_have_distinct_source_identities() {
    let source = "interface Catalog<T> { (item: T): T; new<U>(item: U): Catalog<U>; [index: number]: T; [Keys.iterator](): T; read(): T; read(value: T): T; }";
    let (graph, members) = capture(source);
    assert_eq!(
        members.iter().map(|m| m.kind).collect::<Vec<_>>(),
        [
            Kind::Call,
            Kind::Construct,
            Kind::Index,
            Kind::Method,
            Kind::Method,
            Kind::Method
        ]
    );
    assert!(matches!(members[0].key, Key::Call));
    assert!(matches!(members[1].key, Key::Construct));
    assert!(matches!(members[2].key, Key::Index));
    assert!(
        matches!(&members[3].key, Key::Computed { root: Root::Global(id), selectors, .. }
        if Some(*id) == graph.name_id("Keys") && selectors == &[graph.name_id("iterator").unwrap()])
    );
    assert_eq!(members[4].key, members[5].key);
    assert_ne!(
        members[4].span, members[5].span,
        "overload declarations are not one name-keyed row"
    );
    assert_eq!(members[1].signature.type_parameters.len(), 1);
    assert_eq!(members[0].signature.parameters.len(), 1);
    assert!(
        members.iter().all(|m| m.slot.is_none()),
        "source identity must survive absent navigation rows"
    );
    assert!(members.iter().all(|m| m.signature.result.is_some()));
    assert_eq!(
        &source[members[2].signature.parameters[0].type_span.unwrap().start as usize
            ..members[2].signature.parameters[0].type_span.unwrap().end as usize],
        "number"
    );
}

#[test]
fn computed_paths_bind_lexical_roots_and_do_not_guess_dynamic_keys() {
    let source = "declare const Keys: any; interface Catalog { [Keys.iterator](): void; [factory()](): void; ['literal'](): void; }";
    let (graph, members) = capture(source);
    let id = graph
        .reference_binding_at(
            source.find("Keys.iterator").unwrap() as u32,
            graph.name_id("Keys").unwrap(),
        )
        .unwrap();
    assert!(
        matches!(&members[0].key, Key::Computed { root: Root::Binding(binding), .. } if *binding == id)
    );
    assert!(
        matches!(&members[1].key, Key::Computed { root: Root::Unknown, selectors, .. } if selectors.is_empty())
    );
    assert!(matches!(
        &members[2].key,
        Key::Computed {
            root: Root::Unknown,
            ..
        }
    ));
    assert!(
        !graph
            .globals
            .as_ref()
            .unwrap()
            .roots
            .iter()
            .find(|p| p.kind == SymbolKind::Interface)
            .unwrap()
            .plain_merge
    );
}

#[test]
fn modifiers_parameters_and_unknown_syntax_are_evidence_not_merge_permission() {
    let source = "interface Catalog { readonly value?: string; read<T extends object = {}>(value?: T, ...rest: T[]): T; get count(): number; set count(value: number); }";
    let (_, members) = capture(source);
    assert_eq!(
        members.iter().map(|m| m.kind).collect::<Vec<_>>(),
        [Kind::Property, Kind::Method, Kind::Getter, Kind::Setter]
    );
    assert!(members[0].modifiers.contains(&Modifier::Readonly));
    assert!(members[0].modifiers.contains(&Modifier::Optional));
    assert!(members[1].signature.parameters[0].optional);
    assert!(members[1].signature.parameters[1].rest);
    assert_eq!(members[1].signature.type_parameters.len(), 1);
    assert!(members[3].signature.result.is_none());
}

#[test]
fn unique_symbol_shape_and_constructor_named_methods_remain_distinct() {
    let (_, members) = capture("interface Catalog { readonly brand: unique symbol; value: symbol; constructor(): string; }");
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
        .unwrap();
    let tree = parser
        .parse("interface Catalog { readonly brand: unique symbol; }", None)
        .unwrap();
    let ty = tree
        .root_node()
        .named_child(0)
        .unwrap()
        .child_by_field_name("body")
        .unwrap()
        .named_child(0)
        .unwrap()
        .child_by_field_name("type")
        .unwrap()
        .named_child(0)
        .unwrap();
    let mut cursor = ty.walk();
    let tokens: Vec<_> = ty
        .children(&mut cursor)
        .map(|n| (n.kind(), n.is_named()))
        .collect();
    assert!(
        members[0].signature.unique_symbol,
        "{}: {tokens:?}",
        ty.kind()
    );
    assert!(!members[1].signature.unique_symbol);
    assert_eq!(
        members[2].kind,
        Kind::Method,
        "an interface method named constructor is not a construct signature"
    );
}

#[test]
fn index_parameter_names_are_not_ordinary_member_merge_evidence() {
    let (graph, members) = capture("interface Catalog { [first: string]: number; }");
    let part = graph
        .globals
        .as_ref()
        .unwrap()
        .roots
        .iter()
        .find(|p| p.kind == SymbolKind::Interface)
        .unwrap();
    assert!(!part.plain_merge);
    assert!(part.members.is_empty());
    assert!(matches!(members[0].key, Key::Index));
    assert!(members[0].key_span.is_none());
}

#[test]
#[ignore = "Export source member inventory for the independent pinned compiler AST verifier"]
fn export_compiler_member_surface_inventory() {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let fixtures: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "../resolution_oracle/member_surface_fixtures.json"
    ))
    .unwrap();
    let mut files = Vec::new();
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
        .unwrap();
    let mut inventory = |name: &str, source: &str, path: Option<&str>| {
        let tree = parser.parse(source, None).unwrap();
        assert!(!tree.root_node().has_error(), "{name}");
        let mut graph = LexicalBindings::default();
        graph.add_scope(None, 0, source.len() as u32, true);
        let mut members = Vec::new();
        let mut cursor = tree.root_node().walk();
        for mut node in tree.root_node().named_children(&mut cursor) {
            if node.kind() == "ambient_declaration" {
                node = node.named_child(0).unwrap();
            }
            if let Some(surface) = super::capture(
                node,
                source.as_bytes(),
                &crate::languages::typescript::flow::TS_LEXICAL_SYNTAX,
                &Anchors::new(),
                &mut graph,
            ) {
                for member in surface {
                    let lowered = Member::<usize, usize> {
                        span: member.span,
                        key_span: member.key_span,
                        kind: member.kind,
                        key: Key::Unknown,
                        modifiers: member.modifiers,
                        signature: member.signature,
                        slot: None,
                    };
                    members.push(lowered);
                }
            }
        }
        use sha2::Digest;
        let hash = format!("{:x}", sha2::Sha256::digest(source.as_bytes()));
        let mut file = serde_json::json!({"name":name,"sha256":hash,"members":members});
        if let Some(path) = path {
            file["path"] = path.into();
        } else {
            file["source"] = source.into();
        }
        files.push(file);
    };
    for file in fixtures {
        inventory(
            file["name"].as_str().unwrap(),
            file["source"].as_str().unwrap(),
            None,
        );
    }
    let manifest: crate::resolution_oracle::project::ProjectManifest =
        serde_json::from_slice(
            &std::fs::read(directory.join(
                "resolution-documents/2026-09-07-query-core-configured-compiler-manifest.json",
            ))
            .unwrap(),
        )
        .unwrap();
    manifest.verify_inputs().unwrap();
    for file in &manifest.files {
        if !file
            .path
            .to_string_lossy()
            .replace('\\', "/")
            .contains("/typescript/lib/lib.")
        {
            continue;
        }
        let source = std::fs::read_to_string(&file.path).unwrap();
        inventory(&file.index_path, &source, Some(file.path.to_str().unwrap()));
    }
    assert!(
        files.len() > 6,
        "actual pinned standard-library supply must be exercised"
    );
    let path = std::env::var("BW_MEMBER_ORACLE_OUTPUT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            directory
                .join("resolution-documents/2026-09-08-configured-member-surface-inventory.json")
        });
    let output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .unwrap();
    let mut output = std::io::BufWriter::new(output);
    serde_json::to_writer_pretty(&mut output, &serde_json::json!({"files": files})).unwrap();
    std::io::Write::flush(&mut output).unwrap();
    manifest.verify_inputs().unwrap();
    println!("Source member inventory: {}", path.display());
}
