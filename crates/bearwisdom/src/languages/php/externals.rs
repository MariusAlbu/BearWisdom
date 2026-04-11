// =============================================================================
// php/externals.rs — PHP framework-injected global names
//
// PHP projects rely heavily on framework-provided helper functions that are
// globally available after a framework package is installed (Laravel's
// `route()`, `view()`, `config()`, `trans()`, Symfony's `dump()`, WordPress's
// `get_option()`, etc.). These helpers never appear as source-defined symbols
// in the project — they live inside the framework's own package, usually in
// a global `helpers.php` loaded at boot time.
//
// Two tiers:
//   EXTERNALS     — always-external identifiers (built-in PHP superglobals,
//                   common library globals that PHP projects universally use).
//   framework_globals — dep-gated. Only active when the corresponding
//                   composer package appears in composer.json `require`.
// =============================================================================

use std::collections::HashSet;

/// Always-external PHP names. Covers PHP superglobals and near-universal
/// vendor libraries whose identifiers should never resolve to project code.
pub(crate) const EXTERNALS: &[&str] = &[
    // PHP superglobals (arrays) — referenced as identifiers in many projects
    "_GET",
    "_POST",
    "_REQUEST",
    "_SERVER",
    "_SESSION",
    "_COOKIE",
    "_FILES",
    "_ENV",
    "GLOBALS",
    // Common Monolog / PSR logger facades that appear as method-call
    // receivers without corresponding use statements
    "LoggerInterface",
];

/// Dependency-gated PHP framework globals. Activated when the matching
/// composer package name appears in `require` / `require-dev`.
pub(crate) fn framework_globals(deps: &HashSet<String>) -> Vec<&'static str> {
    let mut globals: Vec<&'static str> = Vec::new();

    // -------------------------------------------------------------------------
    // Laravel (laravel/framework)
    // -------------------------------------------------------------------------
    if deps.contains("laravel/framework")
        || deps.contains("laravel/laravel")
        || deps.contains("illuminate/support")
    {
        globals.extend(LARAVEL_HELPERS);
        globals.extend(LARAVEL_ELOQUENT_QUERY);
        globals.extend(LARAVEL_COLLECTION);
        globals.extend(LARAVEL_FACADES);
    }

    // Inertia.js server side — adapter often bundled as laravel-specific
    if deps.contains("inertiajs/inertia-laravel") || deps.contains("inertiajs/inertia") {
        globals.extend(&["Inertia", "share", "version", "lazy", "render"]);
    }

    // -------------------------------------------------------------------------
    // Symfony (symfony/*)
    // -------------------------------------------------------------------------
    if deps.contains("symfony/framework-bundle") || deps.contains("symfony/symfony") {
        globals.extend(SYMFONY_HELPERS);
    }

    // -------------------------------------------------------------------------
    // PHPUnit test framework
    // -------------------------------------------------------------------------
    if deps.contains("phpunit/phpunit") {
        globals.extend(PHPUNIT_ASSERTIONS);
    }

    // -------------------------------------------------------------------------
    // Pest (alternative test framework built on PHPUnit)
    // -------------------------------------------------------------------------
    if deps.contains("pestphp/pest") {
        globals.extend(PEST_GLOBALS);
    }

    globals
}

// =============================================================================
// Laravel
// =============================================================================

/// Global helper functions auto-loaded by Laravel's `Illuminate\Foundation\helpers.php`
/// and `Illuminate\Support\helpers.php`. Always available in any Laravel app.
const LARAVEL_HELPERS: &[&str] = &[
    // Path helpers
    "app",
    "app_path",
    "base_path",
    "config_path",
    "database_path",
    "lang_path",
    "public_path",
    "resource_path",
    "storage_path",
    // Auth / session / config
    "auth",
    "config",
    "session",
    "request",
    "response",
    "cookie",
    "csrf_field",
    "csrf_token",
    "old",
    // HTTP / routing
    "route",
    "url",
    "secure_url",
    "action",
    "redirect",
    "back",
    "abort",
    "abort_if",
    "abort_unless",
    // Views / templates
    "view",
    "blade",
    // i18n
    "trans",
    "trans_choice",
    "__",
    "lang",
    // Events / queues / cache
    "event",
    "dispatch",
    "dispatch_sync",
    "dispatch_now",
    "bus",
    "cache",
    "cache_lock",
    "broadcast",
    "queue",
    // Validation / factories / models
    "validator",
    "factory",
    "report",
    "report_if",
    "report_unless",
    "rescue",
    "retry",
    "throw_if",
    "throw_unless",
    "tap",
    "with",
    "when",
    "optional",
    // Misc
    "bcrypt",
    "decrypt",
    "encrypt",
    "env",
    "logger",
    "now",
    "today",
    "collect",
    "data_get",
    "data_set",
    "data_fill",
    "head",
    "last",
    "value",
    "class_basename",
    "class_uses_recursive",
    "str",
    "e",
    "filled",
    "blank",
    "transform",
    "windows_os",
    "mix",
    "asset",
    "secure_asset",
    "storage",
    "info",
];

/// Eloquent query builder methods that appear as unresolved member calls on
/// model receivers (`User::where(...)->whereIn(...)->orderBy(...)`). The
/// A.3 chain fix emitted these as bare last-segment names.
const LARAVEL_ELOQUENT_QUERY: &[&str] = &[
    // Where clauses
    "where",
    "whereIn",
    "whereNotIn",
    "whereNull",
    "whereNotNull",
    "whereBetween",
    "whereNotBetween",
    "whereDate",
    "whereTime",
    "whereYear",
    "whereMonth",
    "whereDay",
    "whereColumn",
    "whereRaw",
    "whereExists",
    "whereHas",
    "whereDoesntHave",
    "orWhere",
    "orWhereIn",
    "orWhereNull",
    "orWhereNotNull",
    "orWhereHas",
    "whereKey",
    "whereKeyNot",
    "firstWhere",
    // Ordering / grouping
    "orderBy",
    "orderByDesc",
    "orderByRaw",
    "latest",
    "oldest",
    "inRandomOrder",
    "groupBy",
    "having",
    "havingRaw",
    // Joins
    "join",
    "leftJoin",
    "rightJoin",
    "crossJoin",
    "joinSub",
    // Selection / counts / aggregates
    "select",
    "selectRaw",
    "selectSub",
    "addSelect",
    "distinct",
    "count",
    "min",
    "max",
    "sum",
    "avg",
    "average",
    "exists",
    "doesntExist",
    "pluck",
    "withCount",
    "withSum",
    "withAvg",
    "withMax",
    "withMin",
    "withExists",
    // Fetching
    "get",
    "first",
    "firstOrFail",
    "firstOrNew",
    "firstOrCreate",
    "updateOrCreate",
    "find",
    "findOrFail",
    "findOrNew",
    "findMany",
    "value",
    "chunk",
    "chunkById",
    "each",
    "lazy",
    "lazyById",
    "cursor",
    // Writes
    "create",
    "forceCreate",
    "insert",
    "insertGetId",
    "insertOrIgnore",
    "update",
    "updateOrInsert",
    "upsert",
    "delete",
    "forceDelete",
    "destroy",
    "restore",
    "save",
    "push",
    "touch",
    // Eager loading / relations
    "with",
    "without",
    "withTrashed",
    "onlyTrashed",
    "withoutTrashed",
    "load",
    "loadMissing",
    "loadCount",
    "loadSum",
    "has",
    "doesntHave",
    "hasMany",
    "hasOne",
    "belongsTo",
    "belongsToMany",
    "morphTo",
    "morphMany",
    "morphOne",
    "morphToMany",
    "morphedByMany",
    // Paging
    "paginate",
    "simplePaginate",
    "cursorPaginate",
    "forPage",
    "skip",
    "take",
    "limit",
    "offset",
    // Aliases
    "newQuery",
    "query",
    "toSql",
    "toRawSql",
    "dump",
    "dd",
];

/// Laravel Collection methods (`collect(...)->map()->filter()->...`).
/// Many overlap with Array / Eloquent; this list covers the Collection-
/// specific ones that don't appear in Eloquent.
const LARAVEL_COLLECTION: &[&str] = &[
    "collapse",
    "combine",
    "concat",
    "containsStrict",
    "countBy",
    "crossJoin",
    "dd",
    "diff",
    "diffAssoc",
    "diffKeys",
    "dump",
    "duplicates",
    "duplicatesStrict",
    "eachSpread",
    "ensure",
    "except",
    "filter",
    "firstOrFail",
    "firstWhere",
    "flatMap",
    "flatten",
    "flip",
    "forget",
    "groupBy",
    "implode",
    "intersect",
    "intersectAssoc",
    "intersectByKeys",
    "isEmpty",
    "isNotEmpty",
    "join",
    "keyBy",
    "keys",
    "makeHidden",
    "makeVisible",
    "mapInto",
    "mapSpread",
    "mapToGroups",
    "mapWithKeys",
    "median",
    "merge",
    "mergeRecursive",
    "mode",
    "nth",
    "only",
    "pad",
    "partition",
    "pipe",
    "pipeInto",
    "pipeThrough",
    "pluck",
    "pop",
    "prepend",
    "pull",
    "push",
    "put",
    "random",
    "range",
    "reduce",
    "reduceSpread",
    "reject",
    "replace",
    "replaceRecursive",
    "reverse",
    "search",
    "shift",
    "shuffle",
    "skipUntil",
    "skipWhile",
    "sliding",
    "sole",
    "sort",
    "sortBy",
    "sortByDesc",
    "sortDesc",
    "sortKeys",
    "sortKeysDesc",
    "splice",
    "split",
    "splitIn",
    "sum",
    "takeUntil",
    "takeWhile",
    "times",
    "toArray",
    "toJson",
    "transform",
    "undot",
    "union",
    "unique",
    "uniqueStrict",
    "unless",
    "unlessEmpty",
    "unlessNotEmpty",
    "unwrap",
    "values",
    "when",
    "whenEmpty",
    "whenNotEmpty",
    "whereInstanceOf",
    "whereNotBetween",
    "whereStrict",
    "wrap",
    "zip",
];

/// Laravel facade class names — these appear as receivers in
/// `Auth::user()`, `Route::get(...)`, `DB::table(...)` calls.
const LARAVEL_FACADES: &[&str] = &[
    "Auth",
    "Blade",
    "Broadcast",
    "Bus",
    "Cache",
    "Config",
    "Cookie",
    "Crypt",
    "Date",
    "DB",
    "Event",
    "File",
    "Gate",
    "Hash",
    "Http",
    "Lang",
    "Log",
    "Mail",
    "Notification",
    "Password",
    "Pipeline",
    "Process",
    "Queue",
    "Redirect",
    "Redis",
    "Request",
    "Response",
    "Route",
    "Schema",
    "Session",
    "Storage",
    "URL",
    "Validator",
    "View",
    "Artisan",
    "Model",
    "Eloquent",
    "Str",
    "Arr",
    "Collection",
    "Carbon",
];

// =============================================================================
// Symfony
// =============================================================================

const SYMFONY_HELPERS: &[&str] = &[
    "dump",
    "dd",
    "u",
    "b",
    "t",
    "parameter",
];

// =============================================================================
// PHPUnit
// =============================================================================

const PHPUNIT_ASSERTIONS: &[&str] = &[
    "assertTrue",
    "assertFalse",
    "assertNull",
    "assertNotNull",
    "assertEmpty",
    "assertNotEmpty",
    "assertEquals",
    "assertNotEquals",
    "assertSame",
    "assertNotSame",
    "assertInstanceOf",
    "assertNotInstanceOf",
    "assertCount",
    "assertContains",
    "assertNotContains",
    "assertStringContainsString",
    "assertStringNotContainsString",
    "assertMatchesRegularExpression",
    "assertDoesNotMatchRegularExpression",
    "assertArrayHasKey",
    "assertArrayNotHasKey",
    "assertFileExists",
    "assertFileDoesNotExist",
    "assertDirectoryExists",
    "assertJson",
    "assertJsonStringEqualsJsonString",
    "expectException",
    "expectExceptionMessage",
    "expectExceptionCode",
    "expectExceptionMessageMatches",
    "expectNotToPerformAssertions",
    "fail",
    "markTestIncomplete",
    "markTestSkipped",
    "setUp",
    "tearDown",
    "setUpBeforeClass",
    "tearDownAfterClass",
];

// =============================================================================
// Pest
// =============================================================================

const PEST_GLOBALS: &[&str] = &[
    "test",
    "it",
    "expect",
    "describe",
    "beforeEach",
    "afterEach",
    "beforeAll",
    "afterAll",
    "uses",
    "pest",
    "dataset",
    "mock",
    "spy",
    "fake",
];
