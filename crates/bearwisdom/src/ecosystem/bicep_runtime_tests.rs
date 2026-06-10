use super::*;

#[test]
fn looks_like_bicep_clone_rejects_arbitrary_directory() {
    let tmp = std::env::temp_dir().join("bw-bicep-rejects");
    std::fs::create_dir_all(&tmp).unwrap();
    assert!(!looks_like_bicep_clone(&tmp));
    std::fs::remove_dir_all(&tmp).unwrap();
}

// ---------------------------------------------------------------------------
// Project-tree discovery — the locator must find an Azure/bicep clone
// vendored anywhere inside the project tree, with no env-var and no
// machine-path probe.
// ---------------------------------------------------------------------------

/// Build a minimal Azure/bicep checkout under `clone_root` so
/// `looks_like_bicep_clone` accepts it and synthesis finds real names.
fn write_minimal_clone(clone_root: &std::path::Path) {
    let core = clone_root.join("src").join("Bicep.Core");
    let ns = core.join("Semantics").join("Namespaces");
    std::fs::create_dir_all(&ns).unwrap();
    std::fs::write(core.join("Bicep.Core.csproj"), "<Project/>").unwrap();
    std::fs::write(
        ns.join("SystemNamespaceType.cs"),
        r#"
            new FunctionOverloadBuilder("resourceId").Build();
            new DecoratorBuilder("description").Build();
        "#,
    )
    .unwrap();
    std::fs::write(
        ns.join("AzNamespaceType.cs"),
        r#"new FunctionOverloadBuilder("resourceGroup").Build();"#,
    )
    .unwrap();
}

#[test]
fn discover_finds_clone_vendored_in_project_tree() {
    let root = std::env::temp_dir().join("bw-bicep-vendored-in-tree");
    let _ = std::fs::remove_dir_all(&root);
    // Clone sits a few levels down, alongside dirs that must be pruned.
    let clone = root.join("infra").join("vendor").join("bicep");
    write_minimal_clone(&clone);
    std::fs::create_dir_all(root.join("node_modules").join("junk")).unwrap();
    std::fs::create_dir_all(root.join(".git")).unwrap();

    let roots = discover_bicep_source(&root);
    assert_eq!(
        roots.len(),
        1,
        "exactly one dep root from the vendored clone"
    );
    let located = &roots[0].root;
    assert!(
        located.ends_with(std::path::Path::new("src").join("Bicep.Core")),
        "dep root points at the clone's src/Bicep.Core, got {}",
        located.display()
    );
    assert!(
        located.starts_with(&clone),
        "dep root is inside the vendored clone"
    );

    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn discover_returns_empty_when_no_clone_in_tree() {
    let root = std::env::temp_dir().join("bw-bicep-no-clone-in-tree");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("src").join("app")).unwrap();
    std::fs::write(root.join("main.bicep"), "param x string").unwrap();

    let roots = discover_bicep_source(&root);
    assert!(roots.is_empty(), "no Bicep.Core in tree → honest empty");

    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn looks_like_bicep_clone_accepts_synthetic_layout() {
    let tmp = std::env::temp_dir().join("bw-bicep-synthetic");
    let core = tmp.join("src").join("Bicep.Core");
    let ns = core.join("Semantics").join("Namespaces");
    std::fs::create_dir_all(&ns).unwrap();
    std::fs::write(core.join("Bicep.Core.csproj"), "<Project/>").unwrap();
    std::fs::write(ns.join("SystemNamespaceType.cs"), "namespace Bicep.Core;").unwrap();
    assert!(looks_like_bicep_clone(&tmp));
    std::fs::remove_dir_all(&tmp).unwrap();
}

// ---------------------------------------------------------------------------
// Synthesis helpers (no Bicep clone required — these test the parsers
// directly against representative C# fragments)
// ---------------------------------------------------------------------------

#[test]
fn collect_string_consts_picks_up_simple_declarations() {
    let src = r#"
        public const string MetadataDescriptionPropertyName = "description";
        public const string MetadataResourceDerivedTypePropertyName = "__bicep_resource_derived_type!";
        public const string AnyFunction = "any";
    "#;
    let mut consts = HashMap::new();
    collect_string_consts(src, &mut consts);
    assert_eq!(
        consts.get("MetadataDescriptionPropertyName"),
        Some(&"description".to_string())
    );
    assert_eq!(consts.get("AnyFunction"), Some(&"any".to_string()));
    assert!(
        !consts.contains_key("MetadataResourceDerivedTypePropertyName"),
        "internal __bicep_ markers must be skipped"
    );
}

#[test]
fn extract_function_names_handles_literal_and_constant_args() {
    let src = r#"
        new FunctionOverloadBuilder("environment").Build();
        new FunctionOverloadBuilder(ResourceIdFunctionName).Build();
        new FunctionOverloadBuilder(LanguageConstants.AnyFunction).Build();
        new BannedFunction("parameters", b => b.X());
        BannedFunction.CreateForOperator("add", "+");
    "#;
    let mut consts = HashMap::new();
    consts.insert(
        "ResourceIdFunctionName".to_string(),
        "resourceId".to_string(),
    );
    consts.insert("AnyFunction".to_string(), "any".to_string());

    let names = extract_function_names(src, &consts);
    assert!(names.contains(&"environment".to_string()));
    assert!(names.contains(&"resourceId".to_string()));
    assert!(names.contains(&"any".to_string()));
    assert!(names.contains(&"parameters".to_string()));
    assert!(names.contains(&"add".to_string()));
}

#[test]
fn extract_decorator_names_resolves_constant_references() {
    let src = r#"
        new DecoratorBuilder(LanguageConstants.MetadataDescriptionPropertyName).Build();
        new DecoratorBuilder(BatchSizePropertyName).Build();
        new DecoratorBuilder("export").Build();
    "#;
    let mut consts = HashMap::new();
    consts.insert(
        "MetadataDescriptionPropertyName".to_string(),
        "description".to_string(),
    );
    consts.insert("BatchSizePropertyName".to_string(), "batchSize".to_string());

    let names = extract_decorator_names(src, &consts);
    assert!(names.contains(&"description".to_string()));
    assert!(names.contains(&"batchSize".to_string()));
    assert!(names.contains(&"export".to_string()));
}

#[test]
fn synthesise_emits_namespace_aliases_even_with_empty_source_dir() {
    // Synthesis runs even if the .cs files are missing — but the result
    // is empty (no symbols → empty Vec), preserving the architectural
    // honesty that "no clone, no symbols".
    let tmp = std::env::temp_dir().join("bw-bicep-empty-synth");
    std::fs::create_dir_all(&tmp).unwrap();
    let files = synthesise_bicep_namespace_file(&tmp);
    assert!(files.is_empty(), "no .cs files → no synthetic ParsedFile");
    std::fs::remove_dir_all(&tmp).unwrap();
}

#[test]
fn synthesise_extracts_real_names_from_minimal_clone() {
    let tmp = std::env::temp_dir().join("bw-bicep-minimal-synth");
    let core = tmp.clone();
    let ns = core.join("Semantics").join("Namespaces");
    std::fs::create_dir_all(&ns).unwrap();
    std::fs::write(
        core.join("LanguageConstants.cs"),
        r#"
            public const string AnyFunction = "any";
            public const string MetadataDescriptionPropertyName = "description";
        "#,
    )
    .unwrap();
    std::fs::write(
        ns.join("SystemNamespaceType.cs"),
        r#"
            new FunctionOverloadBuilder("concat").Build();
            new FunctionOverloadBuilder(LanguageConstants.AnyFunction).Build();
            new DecoratorBuilder(LanguageConstants.MetadataDescriptionPropertyName).Build();
            new BannedFunction("parameters", b => b.X());
        "#,
    )
    .unwrap();
    std::fs::write(
        ns.join("AzNamespaceType.cs"),
        r#"new FunctionOverloadBuilder("resourceGroup").Build();"#,
    )
    .unwrap();

    let files = synthesise_bicep_namespace_file(&core);
    assert_eq!(files.len(), 1);
    let names: Vec<_> = files[0].symbols.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"concat"), "literal-string fn registered");
    assert!(names.contains(&"any"), "constant-resolved fn registered");
    assert!(names.contains(&"resourceGroup"), "az ns fn registered");
    assert!(names.contains(&"description"), "decorator via const");
    assert!(names.contains(&"parameters"), "BannedFunction registered");
    assert!(
        names.contains(&"sys"),
        "namespace alias `sys` always present"
    );
    assert!(names.contains(&"az"), "namespace alias `az` always present");

    std::fs::remove_dir_all(&tmp).unwrap();
}
