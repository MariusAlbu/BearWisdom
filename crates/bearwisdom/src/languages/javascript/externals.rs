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
    // -----------------------------------------------------------------
    // DOM Element / Event / Node API — method names that appear as bare
    // last-segment targets after A.3. These are universal browser APIs
    // that projects never redefine, so unconditional classification is
    // safe (a `setAttribute` call is always DOM).
    // -----------------------------------------------------------------
    // DOM traversal / selection
    "querySelector",
    "querySelectorAll",
    "getElementById",
    "getElementsByTagName",
    "getElementsByClassName",
    "getElementsByName",
    "closest",
    "matches",
    // Attribute / content
    "getAttribute",
    "setAttribute",
    "removeAttribute",
    "hasAttribute",
    "getAttributeNode",
    "setAttributeNS",
    "getAttributeNS",
    "hasAttributes",
    "toggleAttribute",
    "innerHTML",
    "outerHTML",
    "innerText",
    "textContent",
    "nodeValue",
    // Tree mutation
    "appendChild",
    "removeChild",
    "replaceChild",
    "insertBefore",
    "insertAdjacentHTML",
    "insertAdjacentText",
    "insertAdjacentElement",
    "cloneNode",
    "normalize",
    "contains",
    // Events
    "addEventListener",
    "removeEventListener",
    "dispatchEvent",
    "preventDefault",
    "stopPropagation",
    "stopImmediatePropagation",
    // Focus / selection / clipboard
    "scrollTo",
    "scrollBy",
    "scrollIntoView",
    // Form / input helpers
    "checkValidity",
    "reportValidity",
    "setCustomValidity",
    // CSSOM / geometry
    "getBoundingClientRect",
    "getClientRects",
    "getComputedStyle",
    // Function prototype — apply/call are shared by every function value
    "bind",
    "apply",
    "Reflect",
    // Object / Array / Promise / Symbol static builder methods that often
    // appear as `Object.defineProperty`, `Promise.resolve`, etc. — last-
    // segment form after A.3
    "defineProperty",
    "defineProperties",
    "getOwnPropertyDescriptor",
    "getOwnPropertyNames",
    "getOwnPropertySymbols",
    "getPrototypeOf",
    "setPrototypeOf",
    "assign",
    "freeze",
    "isFrozen",
    "seal",
    "isSealed",
    "entries",
    "fromEntries",
    "keys",
    "values",
    "hasOwn",
    "create",
    "is",
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

    // Vue 2 instance API — `$t`, `$nextTick`, `$emit`, `$refs`, `$store`,
    // `$router`, `$route`, `$parent`, `$el`, etc. These are Vue 2's instance-
    // level properties/methods that appear inside `<script>` blocks of .vue
    // files (and inside component `.js`/`.ts` files). After S11 the script
    // blocks are sub-extracted as JS/TS, so the refs land in this resolver's
    // namespace. They're strongly $-prefixed and never collide with project
    // identifiers, so unconditional classification as external is safe.
    if deps.contains("vue")
        || deps.contains("vue-i18n")
        || deps.contains("vuex")
        || deps.contains("vue-router")
        || deps.contains("@vue/composition-api")
        || deps.contains("nuxt")
    {
        globals.extend(VUE2_INSTANCE_GLOBALS);
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
    // Jest / Vitest / Sinon spy matcher family — appear as bare last-segment
    // targets after A.3 chain flattening (`expect(spy).toHaveBeenCalled()` →
    // target_name `toHaveBeenCalled`). Adding them here makes spy-heavy test
    // files resolve cleanly regardless of which specific matcher is used.
    "toBe",
    "toEqual",
    "toStrictEqual",
    "toBeCalled",
    "toBeCalledWith",
    "toBeCalledTimes",
    "toHaveBeenCalled",
    "toHaveBeenCalledOnce",
    "toHaveBeenCalledTimes",
    "toHaveBeenCalledWith",
    "toHaveBeenLastCalledWith",
    "toHaveBeenNthCalledWith",
    "toHaveReturned",
    "toHaveReturnedTimes",
    "toHaveReturnedWith",
    "toHaveLastReturnedWith",
    "toHaveNthReturnedWith",
    "toHaveLength",
    "toHaveProperty",
    "toHaveBeenCalledBefore",
    "toHaveBeenCalledAfter",
    "toMatch",
    "toMatchObject",
    "toMatchSnapshot",
    "toMatchInlineSnapshot",
    "toThrow",
    "toThrowError",
    "toThrowErrorMatchingSnapshot",
    "toContain",
    "toContainEqual",
    "toBeCloseTo",
    "toBeDefined",
    "toBeUndefined",
    "toBeNull",
    "toBeNaN",
    "toBeTruthy",
    "toBeFalsy",
    "toBeGreaterThan",
    "toBeGreaterThanOrEqual",
    "toBeLessThan",
    "toBeLessThanOrEqual",
    "toBeInstanceOf",
    "rejects",
    "resolves",
    "not",
    "mockImplementation",
    "mockImplementationOnce",
    "mockReturnValue",
    "mockReturnValueOnce",
    "mockResolvedValue",
    "mockResolvedValueOnce",
    "mockRejectedValue",
    "mockRejectedValueOnce",
    "mockClear",
    "mockReset",
    "mockRestore",
    "mockName",
    "getMockName",
    "mockReturnThis",
    // Sinon spy / stub chain
    "returns",
    "throws",
    "resolves",
    "rejects",
    "yields",
    "callsFake",
    "callsArg",
    "withArgs",
    "onCall",
    "onFirstCall",
    "onSecondCall",
    "onThirdCall",
    "restore",
    "calledWith",
    "calledOnce",
    "calledTwice",
    "calledThrice",
    "calledBefore",
    "calledAfter",
    "getCall",
    "getCalls",
    // Chai BDD chain starters
    "to",
    "be",
    "been",
    "is",
    "that",
    "which",
    "and",
    "has",
    "have",
    "with",
    "at",
    "of",
    "same",
    "but",
    "does",
    "still",
    "also",
    "deep",
    "nested",
    "ordered",
    "any",
    "all",
    "a",
    "an",
    "include",
    "ok",
    "true",
    "false",
    "null",
    "undefined",
    "exist",
    "empty",
    "arguments",
    "equal",
    "equals",
    "eql",
    "above",
    "below",
    "gt",
    "gte",
    "lt",
    "lte",
    "within",
    "instanceof",
    "property",
    "ownProperty",
    "nested",
    "throw",
    "respondTo",
    "itself",
    "satisfy",
    "closeTo",
    "members",
    "oneOf",
    "change",
    "changes",
    "increase",
    "increases",
    "decrease",
    "decreases",
    "fulfilled",
    "rejected",
    "eventually",
    "notify",
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

/// Vue 2 instance API — `$`-prefixed properties and methods that Vue injects
/// onto every component instance. These appear in `<script>` blocks and in
/// `.js`/`.ts` component files as `this.$t(...)`, `this.$nextTick()`, etc.
/// After the A.3 chain-flattening fix they're stored as bare property names.
///
/// Gated on detection of any Vue-family dependency (vue, vue-i18n, vuex,
/// vue-router, nuxt). The `$`-prefix makes collision with real project code
/// extremely unlikely — adding them unconditionally within the Vue gate is
/// safe.
const VUE2_INSTANCE_GLOBALS: &[&str] = &[
    // vue-i18n
    "$t",
    "$tc",
    "$te",
    "$d",
    "$n",
    "$i18n",
    // Vue core lifecycle / reactivity
    "$nextTick",
    "$forceUpdate",
    "$set",
    "$delete",
    "$watch",
    "$on",
    "$off",
    "$once",
    "$emit",
    "$mount",
    "$destroy",
    "$createElement",
    // Vue instance properties
    "$refs",
    "$parent",
    "$children",
    "$root",
    "$el",
    "$data",
    "$props",
    "$slots",
    "$scopedSlots",
    "$attrs",
    "$listeners",
    "$options",
    "$vnode",
    // Vuex
    "$store",
    // Vue Router
    "$router",
    "$route",
    // Common Vue 2 plugin extensions
    "$axios",   // nuxt axios module
    "$toast",
    "$notify",
    "$confirm",
    "$alert",
    "$prompt",
    "$cookies",
    "$loading",
    "$message",
    "$msgbox",
    "$bvModal",       // bootstrap-vue
    "$bvToast",       // bootstrap-vue
    "$fetch",         // nuxt
    "$fetchState",    // nuxt
    "$config",        // nuxt runtime config
    "$nuxt",
    "$auth",          // nuxt auth module
    "$apollo",        // vue-apollo
    "$gtag",          // vue-gtag
    // Inertia.js (shipped as a Vue plugin in Laravel monorepos)
    "$inertia",
    "$page",
    "$headManager",
    "$flash",
];
