use super::extract;
use crate::types::{ExtractedRef, ExtractedSymbol, EdgeKind, SymbolKind, Visibility};

    #[test]
    fn extracts_posix_function() {
        let src = r#"
greet() {
    echo "hello"
}
"#;
        let r = extract::extract(src);
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
        let r = extract::extract(src);
        let build = r.symbols.iter().find(|s| s.name == "build_release").expect("build_release");
        assert_eq!(build.kind, SymbolKind::Function);
        assert_eq!(build.visibility, Some(Visibility::Public));

        let helper = r.symbols.iter().find(|s| s.name == "_internal_helper").expect("_internal_helper");
        assert_eq!(helper.visibility, Some(Visibility::Private));
    }

    #[test]
    fn extracts_file_scope_variable() {
        let src = "APP_NAME=myapp\nVERSION=1.0\n";
        let r = extract::extract(src);
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
        let r = extract::extract(src);
        let imports: Vec<_> = r.refs.iter().filter(|r| r.kind == EdgeKind::Imports).collect();
        assert!(!imports.is_empty(), "expected import refs, got none");
        let targets: Vec<&str> = imports.iter().map(|r| r.target_name.as_str()).collect();
        assert!(targets.contains(&"utils"), "expected 'utils': {targets:?}");
        assert!(targets.contains(&"config"), "expected 'config': {targets:?}");
    }

    #[test]
    fn top_level_command_call_produces_calls_ref() {
        let src = r#"
deploy_app
notify_slack done
"#;
        let r = extract::extract(src);
        let calls: Vec<&str> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Calls)
            .map(|r| r.target_name.as_str())
            .collect();
        assert!(calls.contains(&"deploy_app"), "expected Calls ref to 'deploy_app'; got: {calls:?}");
        assert!(calls.contains(&"notify_slack"), "expected Calls ref to 'notify_slack'; got: {calls:?}");
    }

    #[test]
    fn extracts_variable_inside_if_statement() {
        let src = "if [ -n \"$VAR\" ]; then\n    RESULT=yes\nfi\n";
        let r = extract::extract(src);
        let sym = r.symbols.iter().find(|s| s.name == "RESULT");
        assert!(sym.is_some(), "expected Variable 'RESULT' inside if; got: {:?}", r.symbols);
    }

    #[test]
    fn extracts_declaration_command_at_file_scope() {
        // declare/export/readonly at file scope → Variable symbol
        let src = "declare -r MAX=100\nexport PATH_EXT=/usr/local/bin\n";
        let r = extract::extract(src);
        let max = r.symbols.iter().find(|s| s.name == "MAX");
        assert!(max.is_some(), "expected Variable 'MAX' from declare; got: {:?}", r.symbols);
        assert_eq!(max.unwrap().kind, SymbolKind::Variable);
        let path = r.symbols.iter().find(|s| s.name == "PATH_EXT");
        assert!(path.is_some(), "expected Variable 'PATH_EXT' from export; got: {:?}", r.symbols);
    }

    #[test]
    fn extracts_local_declaration_inside_function() {
        let src = "setup() {\n    local TMPDIR=/tmp/work\n    declare -i COUNT=0\n}\n";
        let r = extract::extract(src);
        let tmpdir = r.symbols.iter().find(|s| s.name == "TMPDIR");
        assert!(tmpdir.is_some(), "expected Variable 'TMPDIR' from local; got: {:?}", r.symbols);
        let count = r.symbols.iter().find(|s| s.name == "COUNT");
        assert!(count.is_some(), "expected Variable 'COUNT' from declare -i; got: {:?}", r.symbols);
    }

    #[test]
    fn extracts_command_substitution_ref() {
        // result=$(get_value) — the command inside $(...) should produce a Calls ref
        let src = "result=$(get_value)\n";
        let r = extract::extract(src);
        let calls: Vec<&str> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Calls)
            .map(|r| r.target_name.as_str())
            .collect();
        assert!(calls.contains(&"get_value"), "expected Calls ref to 'get_value' from $(...); got: {calls:?}");
    }

    #[test]
    fn builtin_commands_produce_calls_refs() {
        // Common tools like git, make, curl are not syntax keywords — they emit Calls
        let src = "git commit -m \"msg\"\nmake all\ncurl https://example.com\n";
        let r = extract::extract(src);
        let calls: Vec<&str> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Calls)
            .map(|r| r.target_name.as_str())
            .collect();
        assert!(calls.contains(&"git"), "expected Calls ref to 'git'; got: {calls:?}");
        assert!(calls.contains(&"make"), "expected Calls ref to 'make'; got: {calls:?}");
        assert!(calls.contains(&"curl"), "expected Calls ref to 'curl'; got: {calls:?}");
    }


    #[test]
    fn extracts_command_substitution_inside_command_arg() {
        // dump_$(date ...) - command substitution embedded in a concatenation argument
        let src = "docker-compose exec pg pg_dumpall >dump_$(date +%Y-%m-%d).sql\n";
        let r = extract::extract(src);
        let calls: Vec<&str> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Calls)
            .map(|r| r.target_name.as_str())
            .collect();
        assert!(calls.contains(&"date"), "expected Calls ref to 'date' from embedded $(...); got: {calls:?}");
    }
