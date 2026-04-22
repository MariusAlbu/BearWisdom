use super::*;

#[test]
fn package_type_is_synthesized() {
    let pf = synthesize_file();
    assert!(
        pf.symbols.iter().any(|s| s.qualified_name == "PackageDescription.Package"
            && s.kind == SymbolKind::Struct),
        "PackageDescription.Package must be synthesized as Struct"
    );
}

#[test]
fn factory_methods_present() {
    let pf = synthesize_file();
    for name in ["library", "executable", "target", "testTarget", "package", "product"] {
        let qname = format!("PackageDescription.{name}");
        let s = pf.symbols.iter().find(|s| s.qualified_name == qname);
        assert!(s.is_some(), "{qname} must be present");
        // Kind is Struct (not Function) so EdgeKind::TypeRef resolves; see
        // note in synthesize_file().
        assert_eq!(s.unwrap().kind, SymbolKind::Struct);
    }
}

#[test]
fn spm_specific_argument_labels_present() {
    let pf = synthesize_file();
    for name in ["dependencies", "targets", "products", "platforms", "swiftSettings"] {
        let qname = format!("PackageDescription.{name}");
        assert!(
            pf.symbols.iter().any(|s| s.qualified_name == qname),
            "{qname} must be synthesized so argument-label refs resolve"
        );
    }
}

#[test]
fn generic_labels_are_not_synthesized() {
    // Deliberately excluded to avoid false-positive resolution in non-SPM
    // Swift code. `name`, `url`, `path`, `type`, `version`, `from` are
    // common identifiers across the entire Swift ecosystem.
    let pf = synthesize_file();
    for skip in ["name", "url", "path", "type", "version", "from"] {
        let qname = format!("PackageDescription.{skip}");
        assert!(
            !pf.symbols.iter().any(|s| s.qualified_name == qname),
            "{qname} must NOT be synthesized — too generic, would mask real refs"
        );
    }
}

#[test]
fn platform_shorthand_factories_present() {
    let pf = synthesize_file();
    for name in ["iOS", "macOS", "tvOS", "watchOS", "visionOS"] {
        let qname = format!("PackageDescription.{name}");
        assert!(
            pf.symbols.iter().any(|s| s.qualified_name == qname),
            "{qname} must be synthesized (used via `.iOS(.v18)` shorthand)"
        );
    }
}

#[test]
fn parallel_vecs_are_consistent() {
    let pf = synthesize_file();
    assert_eq!(pf.symbols.len(), pf.symbol_origin_languages.len());
    assert_eq!(pf.symbols.len(), pf.symbol_from_snippet.len());
}

#[test]
fn activation_is_swift() {
    let e = SwiftPmDslStubsEcosystem;
    assert_eq!(e.languages(), &["swift"]);
    assert_eq!(e.kind(), EcosystemKind::Stdlib);
}
