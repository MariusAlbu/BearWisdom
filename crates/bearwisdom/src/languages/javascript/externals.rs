// =============================================================================
// javascript/externals.rs — JavaScript external globals
// =============================================================================

use std::collections::HashSet;

/// Runtime globals always external for JavaScript (browser + Node.js).
///
/// These supplement the TypeScript EXTERNALS list with Node.js-specific
/// identifiers that TS doesn't need to special-case.
pub(crate) const EXTERNALS: &[&str] = &[
    // Browser / Universal globals
    "console",
    "setTimeout",
    "setInterval",
    "clearTimeout",
    "clearInterval",
    "setImmediate",
    "clearImmediate",
    "queueMicrotask",
    "fetch",
    "URL",
    "URLSearchParams",
    "AbortController",
    "TextEncoder",
    "TextDecoder",
    "structuredClone",
    "atob",
    "btoa",
    "crypto",
    "performance",
    "document",
    "window",
    "navigator",
    "location",
    "history",
    "localStorage",
    "sessionStorage",
    "XMLHttpRequest",
    // Node.js globals
    "process",
    "require",
    "module",
    "exports",
    "__dirname",
    "__filename",
    "global",
    "globalThis",
    "Buffer",
    // Common utility libraries (always global when present)
    "toastr",
    "bootbox",
    "moment",
    "dayjs",
    "lodash",
    "_",
];

/// Dependency-gated framework globals for JavaScript.
pub(crate) fn framework_globals(deps: &HashSet<String>) -> Vec<&'static str> {
    let mut globals = Vec::new();

    // Jest / Vitest / Mocha / Jasmine / AVA test globals
    for dep in ["jest", "vitest", "@jest/globals", "mocha", "jasmine", "ava"] {
        if deps.contains(dep) {
            globals.extend(JS_TEST_GLOBALS);
            break;
        }
    }
    for dep in ["playwright", "@playwright/test"] {
        if deps.contains(dep) {
            globals.extend(JS_TEST_GLOBALS);
            globals.extend(&["page", "browser"]);
            break;
        }
    }
    if deps.contains("cypress") {
        globals.extend(JS_TEST_GLOBALS);
        globals.push("cy");
    }
    for dep in ["jasmine", "jasmine-core", "karma-jasmine"] {
        if deps.contains(dep) {
            globals.extend(JASMINE_GLOBALS);
            break;
        }
    }
    if deps.contains("qunit") {
        globals.extend(QUNIT_GLOBALS);
    }

    // jQuery / AngularJS (classic, not Angular 2+)
    if deps.contains("jquery") || deps.contains("angular") {
        globals.extend(JQUERY_ANGULAR_GLOBALS);
        globals.extend(JQUERY_METHOD_NAMES);
    }

    // Express (adds `app`, `req`, `res`, `next` as well-known names but these
    // are locals, not true globals — skip to avoid false positives)

    // Node.js common npm packages (name appears as require() target, not as a
    // global identifier — handled via is_javascript_builtin for module names)

    globals
}

const JS_TEST_GLOBALS: &[&str] = &[
    "expect",
    "it",
    "describe",
    "test",
    "beforeEach",
    "afterEach",
    "beforeAll",
    "afterAll",
    "before",
    "after",
    "vi",
    "jest",
    "mock",
    "spy",
    "fn",
    "assert",
    "should",
    "sinon",
    "chai",
];

const JASMINE_GLOBALS: &[&str] = &[
    "spyOn",
    "jasmine",
    "jasmine.any",
    "jasmine.anything",
    "jasmine.objectContaining",
    "jasmine.arrayContaining",
    "jasmine.stringMatching",
    "jasmine.createSpy",
    "jasmine.createSpyObj",
];

const QUNIT_GLOBALS: &[&str] = &[
    "QUnit",
    "QUnit.test",
    "QUnit.module",
    "QUnit.skip",
    "QUnit.todo",
    "QUnit.only",
    "QUnit.start",
    "assert",
    "assert.expect",
    "assert.ok",
    "assert.notOk",
    "assert.equal",
    "assert.notEqual",
    "assert.strictEqual",
    "assert.notStrictEqual",
    "assert.deepEqual",
    "assert.notDeepEqual",
];

const JQUERY_ANGULAR_GLOBALS: &[&str] = &[
    // jQuery and AngularJS top-level globals
    "$",
    "jQuery",
    "angular",
    // AngularJS DI services (injected as function parameters — the chain
    // walker sees them as receiver identifiers that never resolve locally)
    "$scope",
    "$rootScope",
    "$http",
    "$state",
    "$stateParams",
    "$q",
    "$timeout",
    "$interval",
    "$window",
    "$document",
    "$compile",
    "$filter",
    "$location",
    "$log",
];

/// jQuery instance methods that appear as bare last-segment target_names
/// after the A.3 JS chain-flattening fix. These are the methods commonly
/// called on jQuery selections (`$el.addClass(...)`, `$(selector).on(...)`)
/// whose target_name is just the method name.
///
/// Only names strongly associated with jQuery are listed — generic names
/// that commonly appear on project classes (like `each`, `find`, `filter`,
/// `first`, `last`, `add`, `remove`, `map`) are intentionally excluded to
/// avoid false-positive external classification of real project methods.
const JQUERY_METHOD_NAMES: &[&str] = &[
    // Class manipulation
    "addClass",
    "removeClass",
    "hasClass",
    "toggleClass",
    // DOM manipulation
    "attr",
    "removeAttr",
    "prop",
    "removeProp",
    "html",
    "text",
    "val",
    "append",
    "prepend",
    "appendTo",
    "prependTo",
    "after",
    "before",
    "insertAfter",
    "insertBefore",
    "wrap",
    "wrapAll",
    "wrapInner",
    "unwrap",
    "replaceWith",
    "replaceAll",
    "detach",
    "clone",
    "empty",
    // Traversal (jQuery versions differ subtly from DOM versions —
    // classifying as external is the correct call either way since these
    // are never project-defined methods)
    "closest",
    "children",
    "parent",
    "parents",
    "parentsUntil",
    "siblings",
    "prev",
    "prevAll",
    "prevUntil",
    "next",
    "nextAll",
    "nextUntil",
    "contents",
    "not",
    "has",
    "eq",
    "slice",
    "end",
    // Events
    "on",
    "off",
    "one",
    "bind",
    "unbind",
    "trigger",
    "triggerHandler",
    "hover",
    "focusin",
    "focusout",
    // Effects
    "fadeIn",
    "fadeOut",
    "fadeToggle",
    "fadeTo",
    "slideUp",
    "slideDown",
    "slideToggle",
    "animate",
    // Dimensions
    "height",
    "width",
    "innerHeight",
    "innerWidth",
    "outerHeight",
    "outerWidth",
    "scrollTop",
    "scrollLeft",
    "offset",
    "position",
    // Static utilities ($.extend, $.isPlainObject, etc. — after A.3 these
    // appear as bare method names)
    "extend",
    "isPlainObject",
    "isEmptyObject",
    "isFunction",
    "isArray",
    "isNumeric",
    "isWindow",
    "isXMLDoc",
    "inArray",
    "grep",
    "noop",
    "now",
    "parseJSON",
    "parseXML",
    "trim",
    "when",
    "Deferred",
    "ajax",
    "getJSON",
    "getScript",
    // Data
    "data",
    "removeData",
    // AngularJS module / scope methods (`.module(...)`, `.directive(...)`,
    // `.controller(...)`, `.factory(...)`, `.service(...)`, `.config(...)`)
    "module",
    "directive",
    "controller",
    "factory",
    "service",
    "config",
    "run",
];
