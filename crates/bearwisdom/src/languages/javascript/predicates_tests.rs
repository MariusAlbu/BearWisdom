use super::predicates;

#[test]
fn test_framework_globals_not_classified_as_js_builtin() {
    // Jest / Vitest / Mocha / Chai / Sinon globals used to short-circuit
    // through `is_javascript_builtin`, but they are third-party libraries
    // indexed via the npm externals walker. The names collide with common
    // method names (`test`, `it`, `before`, `after`, `describe`, `expect`)
    // and produced false fast-exits on user-defined methods.
    for name in &[
        "describe", "it", "test", "expect", "beforeEach", "afterEach",
        "beforeAll", "afterAll", "before", "after",
        "jest", "vi", "mocha", "chai", "sinon",
    ] {
        assert!(
            !predicates::is_javascript_builtin(name),
            "{name:?} should not be classified as a javascript builtin",
        );
    }
}

#[test]
fn real_js_builtins_still_classified() {
    // Sanity: ECMAScript globals + Node.js core modules + DOM globals
    // still match.
    for name in &[
        // ECMAScript
        "Object", "Array", "Promise", "JSON", "Math", "Date",
        "parseInt", "parseFloat", "isNaN",
        // Node.js globals
        "require", "module", "exports", "process", "__dirname", "console",
        // DOM
        "window", "document", "fetch",
    ] {
        assert!(
            predicates::is_javascript_builtin(name),
            "{name:?} must remain a javascript builtin",
        );
    }
}
