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

    // -----------------------------------------------------------------------
    // New tests for added extractions
    // -----------------------------------------------------------------------

    #[test]
    fn require_emits_imports_edge() {
        let src = r#"
const express = require('express');
const path = require('path');
"#;
        let r = extract(src);
        let imports: Vec<_> = r.refs.iter().filter(|r| r.kind == EdgeKind::Imports).collect();
        let modules: Vec<&str> = imports.iter().map(|r| r.target_name.as_str()).collect();
        eprintln!("Imports refs: {modules:?}");
        assert!(modules.contains(&"express"), "missing express import: {modules:?}");
        assert!(modules.contains(&"path"), "missing path import: {modules:?}");
    }

    #[test]
    fn dynamic_import_emits_imports_edge() {
        let src = r#"
async function load() {
    const mod = await import('./utils');
}
"#;
        let r = extract(src);
        let imports: Vec<_> = r.refs.iter().filter(|r| r.kind == EdgeKind::Imports).collect();
        let modules: Vec<&str> = imports.iter().map(|r| r.target_name.as_str()).collect();
        eprintln!("Dynamic import refs: {modules:?}");
        assert!(modules.contains(&"./utils"), "missing ./utils import: {modules:?}");
    }

    #[test]
    fn module_exports_assignment_emits_imports_edge() {
        let src = r#"
function myFn() {}
module.exports = myFn;
"#;
        let r = extract(src);
        let refs: Vec<_> = r.refs.iter().filter(|r| r.kind == EdgeKind::Imports).collect();
        let targets: Vec<&str> = refs.iter().map(|r| r.target_name.as_str()).collect();
        eprintln!("module.exports refs: {targets:?}");
        assert!(
            targets.contains(&"myFn"),
            "expected Imports ref for module.exports = myFn: {targets:?}"
        );
    }

    #[test]
    fn exports_x_assignment_emits_imports_edge() {
        let src = r#"
exports.Router = Router;
exports.Model = Model;
"#;
        let r = extract(src);
        let refs: Vec<_> = r.refs.iter().filter(|r| r.kind == EdgeKind::Imports).collect();
        let targets: Vec<&str> = refs.iter().map(|r| r.target_name.as_str()).collect();
        eprintln!("exports.X refs: {targets:?}");
        assert!(targets.contains(&"Router"), "missing Router: {targets:?}");
        assert!(targets.contains(&"Model"), "missing Model: {targets:?}");
    }

    #[test]
    fn class_expression_emits_class_symbol() {
        let src = r#"
const EventEmitter = class {
    on(event, listener) {}
    emit(event) {}
};
"#;
        let r = extract(src);
        let names: Vec<&str> = r.symbols.iter().map(|s| s.name.as_str()).collect();
        eprintln!("Symbols: {names:?}");
        let sym = r
            .symbols
            .iter()
            .find(|s| s.name == "EventEmitter")
            .expect("EventEmitter class");
        assert_eq!(sym.kind, SymbolKind::Class);
        // Methods inside the class body should also be extracted.
        assert!(
            r.symbols.iter().any(|s| s.name == "on" && s.kind == SymbolKind::Method),
            "expected 'on' method: {names:?}"
        );
    }

    #[test]
    fn function_expression_emits_function_symbol() {
        let src = r#"
const add = function(a, b) {
    return a + b;
};
"#;
        let r = extract(src);
        let sym = r
            .symbols
            .iter()
            .find(|s| s.name == "add")
            .expect("add function");
        assert_eq!(sym.kind, SymbolKind::Function);
    }

    #[test]
    fn arrow_function_emits_function_symbol() {
        let src = r#"
const double = (x) => x * 2;
const identity = x => x;
"#;
        let r = extract(src);
        let names: Vec<&str> = r.symbols.iter().map(|s| s.name.as_str()).collect();
        eprintln!("Symbols: {names:?}");

        let double = r.symbols.iter().find(|s| s.name == "double").expect("double");
        assert_eq!(double.kind, SymbolKind::Function);

        let identity = r.symbols.iter().find(|s| s.name == "identity").expect("identity");
        assert_eq!(identity.kind, SymbolKind::Function);
    }

    #[test]
    fn generator_function_declaration_emits_function_symbol() {
        let src = r#"
function* counter() {
    let i = 0;
    while (true) yield i++;
}
"#;
        let r = extract(src);
        let sym = r
            .symbols
            .iter()
            .find(|s| s.name == "counter")
            .expect("counter generator");
        assert_eq!(sym.kind, SymbolKind::Function);
    }

    #[test]
    fn generator_function_expression_emits_function_symbol() {
        let src = r#"
const gen = function* () {
    yield 1;
    yield 2;
};
"#;
        let r = extract(src);
        let sym = r.symbols.iter().find(|s| s.name == "gen").expect("gen");
        assert_eq!(sym.kind, SymbolKind::Function);
    }

    #[test]
    fn tagged_template_emits_calls_edge() {
        let src = r#"
function render() {
    return html`<div>${name}</div>`;
}
"#;
        let r = extract(src);
        let calls: Vec<_> = r.refs.iter().filter(|r| r.kind == EdgeKind::Calls).collect();
        let targets: Vec<&str> = calls.iter().map(|r| r.target_name.as_str()).collect();
        eprintln!("Calls: {targets:?}");
        assert!(targets.contains(&"html"), "expected html tagged template call: {targets:?}");
    }

    #[test]
    fn new_expression_emits_calls_edge() {
        let src = r#"
function build() {
    const server = new Server(8080);
    const db = new Database.Pool({ max: 10 });
}
"#;
        let r = extract(src);
        let calls: Vec<_> = r.refs.iter().filter(|r| r.kind == EdgeKind::Calls).collect();
        let targets: Vec<&str> = calls.iter().map(|r| r.target_name.as_str()).collect();
        eprintln!("Calls: {targets:?}");
        assert!(targets.contains(&"Server"), "expected new Server call: {targets:?}");
    }

    #[test]
    fn destructuring_rest_emits_variable() {
        let src = r#"
const { a, b, ...rest } = options;
"#;
        let r = extract(src);
        let names: Vec<&str> = r.symbols.iter().map(|s| s.name.as_str()).collect();
        eprintln!("Symbols: {names:?}");
        assert!(names.contains(&"a"), "missing a: {names:?}");
        assert!(names.contains(&"b"), "missing b: {names:?}");
        assert!(names.contains(&"rest"), "missing rest: {names:?}");
        assert!(r.symbols.iter().all(|s| s.kind == SymbolKind::Variable));
    }

    #[test]
    fn catch_clause_emits_variable() {
        let src = r#"
function safe() {
    try {
        doThing();
    } catch (err) {
        console.error(err);
    }
}
"#;
        let r = extract(src);
        let names: Vec<&str> = r.symbols.iter().map(|s| s.name.as_str()).collect();
        eprintln!("Symbols: {names:?}");
        assert!(
            r.symbols.iter().any(|s| s.name == "err" && s.kind == SymbolKind::Variable),
            "expected 'err' variable from catch clause: {names:?}"
        );
    }

    #[test]
    fn for_of_loop_variable_extracted() {
        let src = r#"
function process(items) {
    for (const item of items) {
        console.log(item);
    }
}
"#;
        let r = extract(src);
        let names: Vec<&str> = r.symbols.iter().map(|s| s.name.as_str()).collect();
        eprintln!("Symbols: {names:?}");
        assert!(
            r.symbols.iter().any(|s| s.name == "item" && s.kind == SymbolKind::Variable),
            "expected 'item' loop variable: {names:?}"
        );
    }

    #[test]
    fn for_in_loop_variable_extracted() {
        let src = r#"
function printKeys(obj) {
    for (const key in obj) {
        console.log(key);
    }
}
"#;
        let r = extract(src);
        let names: Vec<&str> = r.symbols.iter().map(|s| s.name.as_str()).collect();
        eprintln!("Symbols: {names:?}");
        assert!(
            r.symbols.iter().any(|s| s.name == "key" && s.kind == SymbolKind::Variable),
            "expected 'key' loop variable: {names:?}"
        );
    }

    #[test]
    fn array_destructuring_extracted() {
        let src = r#"
const [first, second, ...tail] = items;
"#;
        let r = extract(src);
        let names: Vec<&str> = r.symbols.iter().map(|s| s.name.as_str()).collect();
        eprintln!("Symbols: {names:?}");
        assert!(names.contains(&"first"), "missing first: {names:?}");
        assert!(names.contains(&"second"), "missing second: {names:?}");
        assert!(names.contains(&"tail"), "missing tail: {names:?}");
    }

    #[test]
    fn export_default_class_extracted() {
        let src = r#"
export default class Router {
    get(path, handler) {}
    post(path, handler) {}
}
"#;
        let r = extract(src);
        let names: Vec<&str> = r.symbols.iter().map(|s| s.name.as_str()).collect();
        eprintln!("Symbols: {names:?}");
        assert!(
            r.symbols.iter().any(|s| s.name == "Router" && s.kind == SymbolKind::Class),
            "expected Router class from export default: {names:?}"
        );
        assert!(
            r.symbols.iter().any(|s| s.name == "get" && s.kind == SymbolKind::Method),
            "expected get method: {names:?}"
        );
    }

    #[test]
    fn export_named_function_extracted() {
        let src = r#"
export function connect(url) {
    return new Connection(url);
}
"#;
        let r = extract(src);
        let sym = r
            .symbols
            .iter()
            .find(|s| s.name == "connect")
            .expect("connect");
        assert_eq!(sym.kind, SymbolKind::Function);
        // Also verify `new Connection` emits a Calls edge.
        let calls: Vec<_> = r.refs.iter().filter(|r| r.kind == EdgeKind::Calls).collect();
        let targets: Vec<&str> = calls.iter().map(|r| r.target_name.as_str()).collect();
        assert!(targets.contains(&"Connection"), "expected new Connection call: {targets:?}");
    }

    #[test]
    fn calls_inside_arrow_function_extracted() {
        let src = r#"
const handler = async (req, res) => {
    const data = await fetchData(req.params.id);
    res.json(data);
};
"#;
        let r = extract(src);
        let calls: Vec<_> = r.refs.iter().filter(|r| r.kind == EdgeKind::Calls).collect();
        let targets: Vec<&str> = calls.iter().map(|r| r.target_name.as_str()).collect();
        eprintln!("Calls in arrow: {targets:?}");
        assert!(targets.contains(&"fetchData"), "expected fetchData call: {targets:?}");
    }
