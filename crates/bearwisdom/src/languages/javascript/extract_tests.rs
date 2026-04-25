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
        let r = extract::extract(src);
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
        let r = extract::extract(src);
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
        let r = extract::extract(src);
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
        let r = extract::extract(src);
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
        let r = extract::extract(src);
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
        let r = extract::extract(src);
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
        let r = extract::extract(src);
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
        let r = extract::extract(src);
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
        let r = extract::extract(src);
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
        let r = extract::extract(src);
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
        let r = extract::extract(src);
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
        let r = extract::extract(src);
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
        let r = extract::extract(src);
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
        let r = extract::extract(src);
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
        let r = extract::extract(src);
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
        let r = extract::extract(src);
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
        let r = extract::extract(src);
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
        let r = extract::extract(src);
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
        let r = extract::extract(src);
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
        let r = extract::extract(src);
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
        let r = extract::extract(src);
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
        let r = extract::extract(src);
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
        let r = extract::extract(src);
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
        let r = extract::extract(src);
        let calls: Vec<_> = r.refs.iter().filter(|r| r.kind == EdgeKind::Calls).collect();
        let targets: Vec<&str> = calls.iter().map(|r| r.target_name.as_str()).collect();
        eprintln!("Calls in arrow: {targets:?}");
        assert!(targets.contains(&"fetchData"), "expected fetchData call: {targets:?}");
    }

    #[test]
    fn member_call_produces_chain_ref() {
        let src = "vi.spyOn(obj, 'method');";
        let r = extract::extract(src);
        let spy_ref = r
            .refs
            .iter()
            .find(|r| r.target_name == "spyOn" && r.kind == EdgeKind::Calls)
            .expect("spyOn Calls ref");
        let chain = spy_ref.chain.as_ref().expect("chain must be Some for member call");
        assert_eq!(chain.segments.len(), 2, "expected [vi, spyOn], got {:?}", chain.segments);
        assert_eq!(chain.segments[0].name, "vi");
        assert_eq!(chain.segments[1].name, "spyOn");
    }

    #[test]
    fn chained_assertion_produces_chain_ref() {
        let src = "expect(x).toHaveBeenCalledOnce();";
        let r = extract::extract(src);
        let matcher_ref = r
            .refs
            .iter()
            .find(|r| r.target_name == "toHaveBeenCalledOnce" && r.kind == EdgeKind::Calls)
            .expect("toHaveBeenCalledOnce Calls ref");
        let chain = matcher_ref.chain.as_ref().expect("chain must be Some for chained assertion");
        assert!(chain.segments.len() >= 2, "expected at least [expect, toHaveBeenCalledOnce], got {:?}", chain.segments);
        assert_eq!(chain.segments.last().unwrap().name, "toHaveBeenCalledOnce");
    }

    #[test]
    fn long_chai_chain_produces_chain_ref() {
        let src = "expect(x).to.deep.equal(y);";
        let r = extract::extract(src);
        let equal_ref = r
            .refs
            .iter()
            .find(|r| r.target_name == "equal" && r.kind == EdgeKind::Calls)
            .expect("equal Calls ref");
        let chain = equal_ref.chain.as_ref().expect("chain must be Some for chai chain");
        assert!(chain.segments.len() >= 2, "expected multi-segment chain, got {:?}", chain.segments);
        assert_eq!(chain.segments.last().unwrap().name, "equal");
    }

    #[test]
    fn bare_function_call_chain_is_none_or_single_segment() {
        let src = "setupScratch();";
        let r = extract::extract(src);
        let setup_ref = r
            .refs
            .iter()
            .find(|r| r.target_name == "setupScratch" && r.kind == EdgeKind::Calls)
            .expect("setupScratch Calls ref");
        // Bare calls either have no chain or a single-segment chain.
        if let Some(chain) = &setup_ref.chain {
            assert_eq!(
                chain.segments.len(),
                1,
                "bare call should have at most 1 segment, got {:?}",
                chain.segments
            );
        }
    }

    #[test]
    fn iife_window_assignment_harvested_as_top_level_global() {
        // Classic jQuery-style IIFE that installs globals on `root` / `window`.
        let src = r#"
(function(root, factory) {
    root.jQuery = root.$ = factory();
})(typeof window !== "undefined" ? window : this, function() {
    return function jQuery(selector) { return selector; };
});
"#;
        let r = extract::extract(src);
        let names: Vec<&str> = r
            .symbols
            .iter()
            .filter(|s| s.parent_index.is_none())
            .map(|s| s.name.as_str())
            .collect();
        assert!(
            names.contains(&"jQuery"),
            "expected jQuery global, got {names:?}"
        );
        assert!(names.contains(&"$"), "expected $ global, got {names:?}");
    }

    #[test]
    fn direct_window_assignment_harvested() {
        // Some libraries skip the IIFE and assign globals directly.
        let src = "window.myLib = { init() {} };\n";
        let r = extract::extract(src);
        let names: Vec<&str> = r
            .symbols
            .iter()
            .filter(|s| s.parent_index.is_none())
            .map(|s| s.name.as_str())
            .collect();
        assert!(names.contains(&"myLib"));
    }

    #[test]
    fn non_global_assignment_not_harvested() {
        // Assigning to a project-local object must NOT pollute globals.
        let src = "const obj = {}; obj.foo = 1;\n";
        let r = extract::extract(src);
        let has_foo_global = r
            .symbols
            .iter()
            .any(|s| s.name == "foo" && s.parent_index.is_none());
        assert!(!has_foo_global, "foo was wrongly hoisted as a global");
    }

    #[test]
    fn iife_emits_no_garbage_call_ref() {
        // Classic AngularJS module wrapper. The outer IIFE `(function(){...})()`
        // has no named callee — emitting a ref for it used to dump the whole
        // function body into `target_name` (via `rsplit('.').next()` on the
        // full source), producing garbage rows in `unresolved_refs`. The
        // inner named calls (`angular.module(...).factory(...)`) must still
        // be emitted.
        let src = r#"
(function () {
    angular
        .module('simplAdmin.contacts')
        .factory('contactAreaService', ['$http', contactAreaService]);

    function contactAreaService($http) {
        return {
            getContactArea: function (id) {
                return $http.get('api/contact-area/' + id);
            }
        };
    }
})();
"#;
        let r = extract::extract(src);
        let call_targets: Vec<&str> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Calls)
            .map(|r| r.target_name.as_str())
            .collect();

        // No multi-line or multi-character garbage from the IIFE body.
        for target in &call_targets {
            assert!(
                !target.contains('\n') && !target.contains(';') && !target.contains('}'),
                "garbage IIFE-body target leaked into unresolved_refs: {target:?}"
            );
        }
        // The named inner calls must still be captured.
        assert!(
            call_targets.contains(&"factory") || call_targets.contains(&"module"),
            "expected at least one inner named call, got {call_targets:?}"
        );
    }

    #[test]
    fn arrow_fn_parameter_callee_does_not_leak_unresolved() {
        // `(setter) => setter(e.target.value)` — the `setter` call is a
        // parameter binding, not a declared function. Must not emit a
        // Calls ref that pollutes unresolved_refs.
        let src = r#"
const handleChange = (setter) => (e) => {
    setter(e.target.value);
};
"#;
        let r = extract::extract(src);
        let setter_refs: Vec<_> = r
            .refs
            .iter()
            .filter(|r| r.target_name == "setter")
            .collect();
        assert!(
            setter_refs.is_empty(),
            "`setter` is a parameter of the enclosing arrow; must not emit refs, got: {:?}",
            setter_refs
                .iter()
                .map(|r| (r.kind, &r.target_name))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn iife_parameter_chain_receiver_filtered() {
        // `(function ($, opts) { for (k in opts) { opts.hasOwnProperty(k); } })(…)`
        // — `opts` is an IIFE parameter. Chains with `opts` as receiver
        // must not emit TypeRef (for_in iterable) or Calls (chain
        // receiver-originating noise).
        let src = r#"
(function ($, currentSearchOption) {
    for (var key in currentSearchOption) {
        if (currentSearchOption.hasOwnProperty(key)) {
            console.log(currentSearchOption[key]);
        }
    }
})(jQuery, window.opts);
"#;
        let r = extract::extract(src);
        let leaked: Vec<_> = r
            .refs
            .iter()
            .filter(|r| r.target_name == "currentSearchOption")
            .collect();
        assert!(
            leaked.is_empty(),
            "`currentSearchOption` is an IIFE param; no refs must leak, got: {:?}",
            leaked
                .iter()
                .map(|r| (r.kind, &r.target_name))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn destructured_parameter_filter_works() {
        // Destructured arrow-function parameter — still must be treated
        // as a local binding for ref-emission purposes.
        let src = r#"
const makeHandler = ({ onChange }) => (e) => onChange(e.target.value);
"#;
        let r = extract::extract(src);
        assert!(
            !r.refs.iter().any(|r| r.target_name == "onChange"),
            "destructured param `onChange` must not leak as a Calls ref"
        );
    }

    #[test]
    fn real_call_not_filtered_by_same_name_in_unrelated_scope() {
        // Guard: a function that DOES call an external `setter` from a
        // scope where `setter` is NOT a parameter must still emit a ref.
        let src = r#"
function doWork() {
    setter(42);
}
"#;
        let r = extract::extract(src);
        assert!(
            r.refs
                .iter()
                .any(|r| r.kind == EdgeKind::Calls && r.target_name == "setter"),
            "a real call to `setter` outside any enclosing-param scope must emit a Calls ref"
        );
    }

    #[test]
    fn jsx_context_provider_emits_chain_in_js_files() {
        // React Context pattern in a `.jsx` file. The member-expression
        // tag `<AuthContext.Provider>` must emit a Calls ref with
        // target_name="Provider" and a structured chain `[AuthContext,
        // Provider]` — NOT the full dotted string as target_name.
        let src = r#"
import React from 'react';
const AuthContext = React.createContext(null);
function AuthProvider({ children }) {
    return <AuthContext.Provider value={{}}>{children}</AuthContext.Provider>;
}
"#;
        let r = extract::extract(src);
        let provider = r
            .refs
            .iter()
            .find(|r| r.kind == EdgeKind::Calls && r.target_name == "Provider")
            .unwrap_or_else(|| {
                panic!(
                    "expected Calls ref with target_name=Provider; got {:?}",
                    r.refs
                        .iter()
                        .map(|r| (r.kind, r.target_name.clone()))
                        .collect::<Vec<_>>()
                )
            });
        let chain = provider.chain.as_ref().expect("Provider ref must carry a chain");
        let seg_names: Vec<&str> = chain.segments.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(seg_names, vec!["AuthContext", "Provider"]);
        // Full dotted string must NOT appear as a target_name anywhere.
        assert!(
            !r.refs
                .iter()
                .any(|r| r.target_name == "AuthContext.Provider"),
            "full dotted `AuthContext.Provider` must not leak as target_name"
        );
    }

    #[test]
    fn angular_service_registration_emits_class_symbol() {
        // Vendor script-tag JS (e.g. ng-file-upload.js) registers DI tokens
        // via `module.service('Name', [...])`. Consumer code then references
        // `Name.method()` and expects `Name` to resolve as a TypeRef target.
        let src = r#"
var ngFileUpload = angular.module('ngFileUpload', []);
ngFileUpload.service('Upload', ['$parse', function ($parse) {
    var upload = this;
    upload.upload = function () {};
}]);
"#;
        let r = extract::extract(src);
        let upload = r
            .symbols
            .iter()
            .find(|s| s.name == "Upload")
            .expect("expected Upload DI token to be emitted");
        assert_eq!(upload.kind, SymbolKind::Class);
        assert!(
            upload.parent_index.is_none(),
            "Upload should be a top-level symbol (consumers resolve by simple name)"
        );
    }

    #[test]
    fn angular_factory_controller_directive_filter_value_constant_registrations() {
        let src = r#"
angular.module('app', []);
angular.module('app').factory('MyFactory', ['dep', function () {}]);
angular.module('app').controller('MyCtrl', function () {});
angular.module('app').directive('myDirective', function () {});
angular.module('app').filter('myFilter', function () {});
angular.module('app').value('myValue', 42);
angular.module('app').constant('MY_CONST', 'x');
angular.module('app').provider('myProvider', function () {});
angular.module('app').component('myComponent', {});
"#;
        let r = extract::extract(src);
        let by_name: std::collections::HashMap<&str, SymbolKind> = r
            .symbols
            .iter()
            .filter(|s| s.parent_index.is_none())
            .map(|s| (s.name.as_str(), s.kind))
            .collect();
        assert_eq!(by_name.get("MyFactory"), Some(&SymbolKind::Class));
        assert_eq!(by_name.get("MyCtrl"), Some(&SymbolKind::Class));
        assert_eq!(by_name.get("myDirective"), Some(&SymbolKind::Function));
        assert_eq!(by_name.get("myFilter"), Some(&SymbolKind::Function));
        assert_eq!(by_name.get("myValue"), Some(&SymbolKind::Variable));
        assert_eq!(by_name.get("MY_CONST"), Some(&SymbolKind::Variable));
        assert_eq!(by_name.get("myProvider"), Some(&SymbolKind::Class));
        assert_eq!(by_name.get("myComponent"), Some(&SymbolKind::Class));
    }

    #[test]
    fn angular_registration_ignores_non_string_first_arg() {
        // `.service(someVar, ...)` is not a static registration — must not
        // emit anything.
        let src = r#"
angular.module('app').service(dynamicName, ['$q', function () {}]);
"#;
        let r = extract::extract(src);
        assert!(
            !r.symbols.iter().any(|s| s.name == "dynamicName"),
            "non-string first arg must not emit a symbol"
        );
    }

    #[test]
    fn umd_iife_subscript_export_emits_function_symbol() {
        // The classic slugify / dayjs UMD shape:
        //   ;(function (name, root, factory) {
        //     root[name] = factory();
        //   }('slugify', this, function () { ... }))
        // The `root` param is bound to `this` (global), the `name` param to
        // the string literal 'slugify'. The assignment `root[name] = ...`
        // exports `slugify` as a top-level callable.
        let src = r#"
;(function (name, root, factory) {
    if (typeof exports === 'object') {
        module.exports = factory();
    } else {
        root[name] = factory();
    }
}('slugify', this, function () {
    return function replace(s) { return s; };
}));
"#;
        let r = extract::extract(src);
        let slug = r
            .symbols
            .iter()
            .find(|s| s.name == "slugify" && s.parent_index.is_none())
            .expect("expected slugify UMD export to be emitted");
        assert_eq!(slug.kind, SymbolKind::Function);
    }

    #[test]
    fn umd_iife_window_root_binding_also_works() {
        // Same pattern but the root arg is `window` (identifier) instead of
        // `this` — equally valid.
        let src = r#"
(function (root, name, factory) {
    root[name] = factory();
})(window, 'dayjs', function () {
    return function () {};
});
"#;
        let r = extract::extract(src);
        assert!(
            r.symbols
                .iter()
                .any(|s| s.name == "dayjs" && s.kind == SymbolKind::Function && s.parent_index.is_none()),
            "expected dayjs UMD export emitted as Function, got {:?}",
            r.symbols
                .iter()
                .map(|s| (s.name.as_str(), s.kind))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn prototype_method_assignment_emits_method_symbol() {
        // ES5 "class" pattern produced by webpack / TypeScript `target: es5`
        // transpile. Every `X.prototype.Y = function() { ... }` should surface
        // as a Method symbol under qualified name `X.Y` so chain walking
        // `new X().Y()` can resolve the callee.
        let src = r#"
var HubConnectionBuilder = (function () {
    function HubConnectionBuilder() {}
    HubConnectionBuilder.prototype.withUrl = function (url) {
        this.url = url;
        return this;
    };
    HubConnectionBuilder.prototype.build = function () {
        return new Connection(this.url);
    };
    return HubConnectionBuilder;
}());
"#;
        let r = extract::extract(src);

        let with_url = r
            .symbols
            .iter()
            .find(|s| s.qualified_name == "HubConnectionBuilder.withUrl")
            .unwrap_or_else(|| panic!(
                "expected HubConnectionBuilder.withUrl; symbols: {:?}",
                r.symbols
                    .iter()
                    .map(|s| (s.qualified_name.as_str(), s.kind))
                    .collect::<Vec<_>>()
            ));
        assert_eq!(with_url.kind, SymbolKind::Method);
        assert_eq!(with_url.name, "withUrl");

        let build = r
            .symbols
            .iter()
            .find(|s| s.qualified_name == "HubConnectionBuilder.build")
            .expect("expected HubConnectionBuilder.build method symbol");
        assert_eq!(build.kind, SymbolKind::Method);
    }

    #[test]
    fn prototype_method_with_arrow_function_also_extracted() {
        // Modern mixed-style: constructor function with arrow-function
        // prototype methods. Still the same shape, still a Method symbol.
        let src = r#"
function Widget() {}
Widget.prototype.render = (ctx) => ctx.draw(this);
"#;
        let r = extract::extract(src);
        let render = r
            .symbols
            .iter()
            .find(|s| s.qualified_name == "Widget.render")
            .expect("expected Widget.render symbol");
        assert_eq!(render.kind, SymbolKind::Method);
    }

    #[test]
    fn prototype_field_with_non_function_rhs_is_not_emitted() {
        // `X.prototype.y = []` is a property initializer, not a method. We
        // must NOT emit it as Method — doing so would misreport the class's
        // surface and create phantom callable targets.
        let src = r#"
function Registry() {}
Registry.prototype.items = [];
Registry.prototype.count = 0;
"#;
        let r = extract::extract(src);
        assert!(
            !r.symbols
                .iter()
                .any(|s| s.qualified_name == "Registry.items"),
            "expected no Registry.items (array literal is not a method)"
        );
        assert!(
            !r.symbols
                .iter()
                .any(|s| s.qualified_name == "Registry.count"),
            "expected no Registry.count (number literal is not a method)"
        );
    }
