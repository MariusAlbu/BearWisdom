use std::collections::HashSet;

/// Runtime globals always external for JS/TS (browser + Node.js).
pub(crate) const EXTERNALS: &[&str] = &[
    // Browser globals
    "console", "setTimeout", "setInterval", "clearTimeout", "clearInterval",
    "requestAnimationFrame", "cancelAnimationFrame",
    "document", "window", "navigator", "location", "history",
    "localStorage", "sessionStorage",
    "fetch", "XMLHttpRequest",
    // Node.js globals
    "process", "require", "module", "exports", "__dirname", "__filename",
    "global", "globalThis",
    // Common utility libraries (always global when present)
    "toastr", "bootbox", "moment", "dayjs", "lodash", "_",
];

/// Dependency-gated framework globals for JS/TS.
pub(crate) fn framework_globals(deps: &HashSet<String>) -> Vec<&'static str> {
    let mut globals = Vec::new();

    // Jest / Vitest / Mocha / Jasmine test globals
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
    if deps.contains("benchmark") {
        globals.extend(&["Benchmark", "Benchmark.Suite", "Benchmark.options"]);
    }

    // jQuery / Angular.js
    if deps.contains("jquery") || deps.contains("angular") {
        globals.extend(JQUERY_ANGULAR_GLOBALS);
    }

    // Svelte / SvelteKit
    if deps.contains("svelte") || deps.contains("@sveltejs/kit") {
        globals.extend(SVELTE_GLOBALS);
    }
    if deps.contains("@sveltejs/kit") {
        globals.extend(SVELTEKIT_GLOBALS);
    }

    // i18n
    for dep in ["svelte-i18n", "i18next", "next-i18next", "vue-i18n", "@ngx-translate/core"] {
        if deps.contains(dep) {
            globals.extend(&["$t", "t", "$i18n", "i18n", "$locale", "$format"]);
            break;
        }
    }

    globals
}

const JS_TEST_GLOBALS: &[&str] = &[
    "expect", "it", "describe", "test", "beforeEach", "afterEach", "beforeAll", "afterAll",
    "vi", "jest", "mock", "spy", "fn", "assert", "should", "before", "after",
];

const JASMINE_GLOBALS: &[&str] = &[
    "spyOn", "jasmine", "jasmine.any", "jasmine.anything",
    "jasmine.objectContaining", "jasmine.arrayContaining",
    "jasmine.stringMatching", "jasmine.createSpy", "jasmine.createSpyObj",
    "fixtureEl", "EventHandler",
];

const QUNIT_GLOBALS: &[&str] = &[
    "QUnit", "QUnit.test", "QUnit.module", "QUnit.skip",
    "QUnit.todo", "QUnit.only", "QUnit.start",
    "assert", "assert.expect", "assert.ok", "assert.notOk",
    "assert.equal", "assert.notEqual", "assert.strictEqual",
    "assert.notStrictEqual", "assert.deepEqual", "assert.notDeepEqual",
    "assert.propEqual", "assert.notPropEqual", "assert.propContains",
    "assert.true", "assert.false", "assert.throws", "assert.rejects",
    "assert.step", "assert.verifySteps", "assert.timeout",
];

const JQUERY_ANGULAR_GLOBALS: &[&str] = &[
    "$", "jQuery",
    "angular", "angular.module", "angular.element", "angular.isObject",
    "angular.isArray", "angular.isString", "angular.isFunction",
    "angular.forEach", "angular.copy", "angular.extend",
    "$scope", "$rootScope", "$http", "$state", "$stateParams",
    "$q", "$timeout", "$interval", "$window", "$document",
    "$compile", "$filter", "$location", "$log",
];

const SVELTE_GLOBALS: &[&str] = &[
    "$state", "$derived", "$effect", "$props", "$bindable", "$inspect",
    "$host", "$state.raw", "$derived.by", "$effect.pre", "$effect.root",
    "$:", "$$props", "$$restProps", "$$slots",
];

const SVELTEKIT_GLOBALS: &[&str] = &[
    "PageLoad", "PageData", "PageServerLoad", "PageServerData",
    "LayoutLoad", "LayoutData", "LayoutServerLoad", "LayoutServerData",
    "Actions", "ActionData", "RequestHandler",
    "EntryGenerator", "ParamMatcher",
    "goto", "invalidate", "invalidateAll", "prefetch", "beforeNavigate",
    "afterNavigate", "onNavigate", "pushState", "replaceState",
    "page", "navigating", "updated",
    "browser", "building", "dev", "version",
    "enhance", "applyAction", "deserialize",
    "base", "assets", "resolveRoute",
    "env",
];
