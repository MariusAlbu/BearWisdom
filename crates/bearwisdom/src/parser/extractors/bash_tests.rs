    use super::*;
    use crate::types::{EdgeKind, SymbolKind, Visibility};

    #[test]
    fn extracts_posix_function() {
        let src = r#"
greet() {
    echo "hello"
}
"#;
        let r = extract(src);
        let sym = r.symbols.iter().find(|s| s.name == "greet").expect("greet");
        assert_eq!(sym.kind, SymbolKind::Function);
        assert_eq!(sym.visibility, Some(Visibility::Public));
    }

    #[test]
    fn extracts_function_keyword_form() {
        let src = r#"
function build_release {
    cargo build --release
}

function _internal_helper() {
    true
}
"#;
        let r = extract(src);
        let build = r.symbols.iter().find(|s| s.name == "build_release").expect("build_release");
        assert_eq!(build.kind, SymbolKind::Function);
        assert_eq!(build.visibility, Some(Visibility::Public));

        let helper = r.symbols.iter().find(|s| s.name == "_internal_helper").expect("_internal_helper");
        assert_eq!(helper.visibility, Some(Visibility::Private));
    }

    #[test]
    fn extracts_file_scope_variable() {
        let src = "APP_NAME=myapp\nVERSION=1.0\n";
        let r = extract(src);
        let app = r.symbols.iter().find(|s| s.name == "APP_NAME").expect("APP_NAME");
        assert_eq!(app.kind, SymbolKind::Variable);
        let ver = r.symbols.iter().find(|s| s.name == "VERSION").expect("VERSION");
        assert_eq!(ver.kind, SymbolKind::Variable);
    }

    #[test]
    fn source_command_produces_import_ref() {
        let src = r#"
source ./lib/utils.sh
. ./config.sh
"#;
        let r = extract(src);
        let imports: Vec<_> = r.refs.iter().filter(|r| r.kind == EdgeKind::Imports).collect();
        assert!(!imports.is_empty(), "expected import refs, got none");
        let targets: Vec<&str> = imports.iter().map(|r| r.target_name.as_str()).collect();
        assert!(targets.contains(&"utils"), "expected 'utils': {targets:?}");
        assert!(targets.contains(&"config"), "expected 'config': {targets:?}");
    }
