    use super::*;
    use crate::types::{EdgeKind, SymbolKind};

    #[test]
    fn extracts_class_and_methods() {
        let src = r#"
class Animal {
    constructor(name) {
        this.name = name;
    }

    speak() {
        return '...';
    }
}
"#;
        let r = extract(src);
        let cls = r.symbols.iter().find(|s| s.name == "Animal").expect("Animal");
        assert_eq!(cls.kind, SymbolKind::Class);

        assert!(r.symbols.iter().any(|s| s.name == "constructor" && s.kind == SymbolKind::Constructor));
        assert!(r.symbols.iter().any(|s| s.name == "speak" && s.kind == SymbolKind::Method));
    }

    #[test]
    fn extracts_top_level_function() {
        let src = r#"
function greet(name) {
    return 'Hello, ' + name;
}
"#;
        let r = extract(src);
        let func = r.symbols.iter().find(|s| s.name == "greet").expect("greet");
        assert_eq!(func.kind, SymbolKind::Function);
    }

    #[test]
    fn extracts_const_variable() {
        let src = r#"
const getters = {
    sidebar: state => state.app.sidebar,
    size: state => state.app.size,
}
export default getters
"#;
        let r = extract(src);
        let names: Vec<&str> = r.symbols.iter().map(|s| s.name.as_str()).collect();
        eprintln!("Symbols: {names:?}");
        let kinds: Vec<_> = r.symbols.iter().map(|s| (s.name.as_str(), s.kind)).collect();
        eprintln!("Kinds: {kinds:?}");
        assert!(
            r.symbols.iter().any(|s| s.name == "getters"),
            "Expected 'getters' variable, got: {names:?}"
        );
    }

    #[test]
    fn extracts_module_exports_function() {
        let src = r#"
const install = function(Vue) {
    Vue.directive('Clipboard', Clipboard)
}
module.exports = install
"#;
        let r = extract(src);
        let names: Vec<&str> = r.symbols.iter().map(|s| s.name.as_str()).collect();
        eprintln!("Symbols: {names:?}");
        assert!(
            r.symbols.iter().any(|s| s.name == "install"),
            "Expected 'install' variable, got: {names:?}"
        );
    }

    #[test]
    fn import_statement_produces_type_ref() {
        let src = "import { useState, useEffect } from 'react';\n";
        let r = extract(src);
        let imports: Vec<_> = r.refs.iter().filter(|r| r.kind == EdgeKind::TypeRef).collect();
        let targets: Vec<&str> = imports.iter().map(|r| r.target_name.as_str()).collect();
        assert!(targets.contains(&"useState"), "missing useState: {targets:?}");
        assert!(targets.contains(&"useEffect"), "missing useEffect: {targets:?}");
    }
