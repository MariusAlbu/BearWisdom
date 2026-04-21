// =============================================================================
// ecosystem/laravel_stubs.rs — Laravel framework synthetic stubs
//
// Laravel's Eloquent Query Builder uses PHP `__call` magic to forward dozens
// of `where*`, `with*`, `order*` etc. methods that are never declared as real
// PHP methods in the source.  Even with the full composer vendor tree indexed,
// `Builder.whereIn` has no return_type — so the chain walker dies there.
//
// This ecosystem emits synthetic ParsedFile entries (no on-disk source) that
// supply:
//   1. Illuminate\Database\Eloquent\Builder  — all fluent methods returning
//      Builder + terminal methods returning Collection / Model.
//   2. Illuminate\Support\Collection  — key fluent + terminal methods.
//   3. Laravel global helper functions (trans, __, config, route, redirect,
//      view, auth, abort, session, put, response, request, app, event,
//      dispatch, cache, cookie, back).
//
// Activation: any PHP file present in the project.  Degrades gracefully if
// Composer has already indexed a richer Laravel vendor tree — the chain walker
// tries DB-backed symbols first; synthetics are fallback.
//
// Pattern: identical to ecosystem/node_builtins.rs — see that file for the
// reference implementation.
// =============================================================================

use std::path::Path;

use super::{Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::types::{ExtractedSymbol, ParsedFile, SymbolKind, Visibility};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("laravel-stubs");
const LEGACY_ECOSYSTEM_TAG: &str = "laravel-stubs";
const LANGUAGES: &[&str] = &["php"];

// =============================================================================
// Surface definitions
// =============================================================================

struct FluentMethod {
    name: &'static str,
    return_type: &'static str,
    params: &'static str,
}

const fn fluent(name: &'static str, params: &'static str) -> FluentMethod {
    FluentMethod { name, return_type: "Builder", params }
}

const fn terminal(name: &'static str, return_type: &'static str, params: &'static str) -> FluentMethod {
    FluentMethod { name, return_type, params }
}

// ---------------------------------------------------------------------------
// Eloquent Query Builder surface
//
// Fluent methods all return Builder (self-closing chain).
// Terminal methods return Collection, mixed, int, bool, or Model.
// ---------------------------------------------------------------------------

static BUILDER_METHODS: &[FluentMethod] = &[
    // Constraints
    fluent("where",            "$column, $operator = null, $value = null, $boolean = 'and'"),
    fluent("orWhere",          "$column, $operator = null, $value = null"),
    fluent("whereNot",         "$column, $operator = null, $value = null"),
    fluent("orWhereNot",       "$column, $operator = null, $value = null"),
    fluent("whereIn",          "$column, $values, $boolean = 'and', $not = false"),
    fluent("orWhereIn",        "$column, $values"),
    fluent("whereNotIn",       "$column, $values, $boolean = 'and'"),
    fluent("orWhereNotIn",     "$column, $values"),
    fluent("whereBetween",     "$column, $values, $boolean = 'and', $not = false"),
    fluent("orWhereBetween",   "$column, $values"),
    fluent("whereNull",        "$column, $boolean = 'and', $not = false"),
    fluent("orWhereNull",      "$column"),
    fluent("whereNotNull",     "$column, $boolean = 'and'"),
    fluent("orWhereNotNull",   "$column"),
    fluent("whereLike",        "$column, $value, $caseSensitive = false"),
    fluent("whereColumn",      "$first, $operator = null, $second = null, $boolean = 'and'"),
    fluent("orWhereColumn",    "$first, $operator = null, $second = null"),
    fluent("whereDate",        "$column, $operator, $value = null, $boolean = 'and'"),
    fluent("whereMonth",       "$column, $operator, $value = null, $boolean = 'and'"),
    fluent("whereYear",        "$column, $operator, $value = null, $boolean = 'and'"),
    fluent("whereDay",         "$column, $operator, $value = null, $boolean = 'and'"),
    fluent("whereTime",        "$column, $operator, $value = null, $boolean = 'and'"),
    fluent("whereHas",         "$relation, $callback = null, $operator = '>=', $count = 1"),
    fluent("orWhereHas",       "$relation, $callback = null, $operator = '>=', $count = 1"),
    fluent("whereDoesntHave",  "$relation, $callback = null"),
    fluent("whereRelation",    "$relation, $column, $operator = null, $value = null"),
    fluent("whereMorphRelation","$relation, $types, $column, $operator = null, $value = null"),
    fluent("whereMorphedTo",   "$relation, $model, $boolean = 'and'"),
    // Eager loading
    fluent("with",             "$relations, $callback = null"),
    fluent("withCount",        "$relations"),
    fluent("withSum",          "$relation, $column"),
    fluent("withAvg",          "$relation, $column"),
    fluent("withMax",          "$relation, $column"),
    fluent("withMin",          "$relation, $column"),
    fluent("withExists",       "$relation"),
    fluent("without",          "$relations"),
    fluent("withOnly",         "$relations"),
    fluent("withTrashed",      ""),
    fluent("onlyTrashed",      ""),
    fluent("withoutTrashed",   ""),
    // Ordering / grouping
    fluent("orderBy",          "$column, $direction = 'asc'"),
    fluent("orderByDesc",      "$column"),
    fluent("orderByRaw",       "$sql, $bindings = []"),
    fluent("latest",           "$column = 'created_at'"),
    fluent("oldest",           "$column = 'created_at'"),
    fluent("groupBy",          "$groups"),
    fluent("having",           "$column, $operator = null, $value = null, $boolean = 'and'"),
    fluent("havingRaw",        "$sql, $bindings = [], $boolean = 'and'"),
    fluent("skip",             "$value"),
    fluent("take",             "$value"),
    fluent("offset",           "$value"),
    fluent("limit",            "$value"),
    fluent("forPage",          "$page, $perPage = 15"),
    // Joins
    fluent("join",             "$table, $first, $operator = null, $second = null, $type = 'inner'"),
    fluent("leftJoin",         "$table, $first, $operator = null, $second = null"),
    fluent("rightJoin",        "$table, $first, $operator = null, $second = null"),
    fluent("crossJoin",        "$table, $first = null, $operator = null, $second = null"),
    fluent("joinSub",          "$query, $as, $first, $operator = null, $second = null"),
    fluent("leftJoinSub",      "$query, $as, $first, $operator = null, $second = null"),
    // Selection
    fluent("select",           "$columns = ['*']"),
    fluent("selectRaw",        "$expression, $bindings = []"),
    fluent("selectSub",        "$query, $as"),
    fluent("addSelect",        "$column"),
    fluent("distinct",         ""),
    // Locking
    fluent("lockForUpdate",    ""),
    fluent("sharedLock",       ""),
    // Misc fluent
    fluent("when",             "$condition, $callback, $default = null"),
    fluent("unless",           "$condition, $callback, $default = null"),
    fluent("tap",              "$callback"),
    fluent("scopes",           "$scopes"),
    fluent("withGlobalScope",  "$identifier, $scope"),
    fluent("withoutGlobalScope","$scope"),
    fluent("withoutGlobalScopes","$scopes = null"),
    fluent("fromSub",          "$query, $as"),
    fluent("fromRaw",          "$expression, $bindings = []"),
    fluent("whereRaw",         "$sql, $bindings = [], $boolean = 'and'"),
    fluent("orWhereRaw",       "$sql, $bindings = []"),
    // Terminal methods — return type is not Builder
    terminal("get",            "Collection", "$columns = ['*']"),
    terminal("first",          "mixed",      "$columns = ['*']"),
    terminal("firstOrFail",    "mixed",      "$columns = ['*']"),
    terminal("find",           "mixed",      "$id, $columns = ['*']"),
    terminal("findOrFail",     "mixed",      "$id, $columns = ['*']"),
    terminal("findMany",       "Collection", "$ids, $columns = ['*']"),
    terminal("all",            "Collection", "$columns = ['*']"),
    terminal("pluck",          "Collection", "$column, $key = null"),
    terminal("value",          "mixed",      "$column"),
    terminal("count",          "int",        "$columns = '*'"),
    terminal("sum",            "mixed",      "$column"),
    terminal("min",            "mixed",      "$column"),
    terminal("max",            "mixed",      "$column"),
    terminal("avg",            "mixed",      "$column"),
    terminal("average",        "mixed",      "$column"),
    terminal("exists",         "bool",       ""),
    terminal("doesntExist",    "bool",       ""),
    terminal("paginate",       "mixed",      "$perPage = null, $columns = ['*']"),
    terminal("simplePaginate", "mixed",      "$perPage = 15, $columns = ['*']"),
    terminal("cursorPaginate", "mixed",      "$perPage = null, $columns = ['*']"),
    terminal("chunk",          "bool",       "$count, $callback"),
    terminal("each",           "bool",       "$callback, $count = 1000"),
    terminal("eachById",       "bool",       "$callback, $count = 1000"),
    terminal("chunkById",      "bool",       "$count, $callback"),
    terminal("lazy",           "mixed",      "$chunkSize = 1000"),
    terminal("lazyById",       "mixed",      "$chunkSize = 1000"),
    terminal("create",         "mixed",      "$attributes = []"),
    terminal("forceCreate",    "mixed",      "$attributes = []"),
    terminal("firstOrCreate",  "mixed",      "$attributes, $values = []"),
    terminal("firstOrNew",     "mixed",      "$attributes, $values = []"),
    terminal("updateOrCreate", "mixed",      "$attributes, $values = []"),
    terminal("insert",         "bool",       "$values"),
    terminal("update",         "int",        "$values"),
    terminal("delete",         "mixed",      ""),
    terminal("forceDelete",    "mixed",      ""),
    terminal("restore",        "int",        ""),
    terminal("truncate",       "void",       ""),
    terminal("toSql",          "string",     ""),
    terminal("toBase",         "Builder",    ""),
    terminal("getQuery",       "Builder",    ""),
];

// ---------------------------------------------------------------------------
// Collection surface
// ---------------------------------------------------------------------------

static COLLECTION_METHODS: &[FluentMethod] = &[
    fluent("filter",    "$callback = null"),
    fluent("reject",    "$callback = false"),
    fluent("map",       "$callback"),
    fluent("flatMap",   "$callback"),
    fluent("each",      "$callback"),
    fluent("tap",       "$callback"),
    fluent("sortBy",    "$callback, $options = SORT_REGULAR, $descending = false"),
    fluent("sortByDesc","$callback, $options = SORT_REGULAR"),
    fluent("groupBy",   "$groupBy, $preserveKeys = false"),
    fluent("keyBy",     "$keyBy"),
    fluent("unique",    "$key = null, $strict = false"),
    fluent("take",      "$limit"),
    fluent("skip",      "$offset"),
    fluent("slice",     "$offset, $length = null"),
    fluent("chunk",     "$size"),
    fluent("flatten",   "$depth = INF"),
    fluent("merge",     "$items"),
    fluent("only",      "$keys"),
    fluent("except",    "$keys"),
    fluent("where",     "$key, $operator = null, $value = null"),
    fluent("whereIn",   "$key, $values"),
    fluent("whereNotIn","$key, $values"),
    fluent("whereNull", "$key"),
    fluent("whereNotNull","$key"),
    fluent("when",      "$condition, $callback, $default = null"),
    fluent("unless",    "$condition, $callback, $default = null"),
    fluent("diff",      "$items"),
    fluent("intersect", "$items"),
    fluent("zip",       "$items"),
    fluent("pad",       "$size, $value"),
    fluent("reverse",   ""),
    fluent("shuffle",   ""),
    fluent("combine",   "$values"),
    fluent("prepend",   "$value, $key = null"),
    fluent("push",      "$value"),
    fluent("put",       "$key, $value"),
    fluent("forget",    "$keys"),
    terminal("all",       "array",   ""),
    terminal("first",     "mixed",   "$callback = null, $default = null"),
    terminal("last",      "mixed",   "$callback = null, $default = null"),
    terminal("count",     "int",     ""),
    terminal("sum",       "mixed",   "$callback = null"),
    terminal("avg",       "mixed",   "$callback = null"),
    terminal("min",       "mixed",   "$callback = null"),
    terminal("max",       "mixed",   "$callback = null"),
    terminal("pluck",     "Collection", "$value, $key = null"),
    terminal("contains",  "bool",    "$key, $operator = null, $value = null"),
    terminal("doesntContain","bool", "$key, $operator = null, $value = null"),
    terminal("has",       "bool",    "$key"),
    terminal("isEmpty",   "bool",    ""),
    terminal("isNotEmpty","bool",    ""),
    terminal("toArray",   "array",   ""),
    terminal("toJson",    "string",  "$options = 0"),
    terminal("values",    "Collection", ""),
    terminal("keys",      "Collection", ""),
    terminal("implode",   "string",  "$value, $glue = null"),
    terminal("join",      "string",  "$glue, $finalGlue = ''"),
    terminal("reduce",    "mixed",   "$callback, $initial = null"),
    terminal("each",      "Collection", "$callback"),
    terminal("search",    "mixed",   "$value, $strict = false"),
    terminal("get",       "mixed",   "$key, $default = null"),
    terminal("offsetGet", "mixed",   "$offset"),
    terminal("pop",       "mixed",   ""),
    terminal("shift",     "mixed",   ""),
    terminal("pull",      "mixed",   "$key, $default = null"),
    terminal("random",    "mixed",   "$number = null"),
    terminal("nth",       "Collection", "$step, $offset = 0"),
    terminal("paginate",  "mixed",   "$perPage, $page = null"),
];

// ---------------------------------------------------------------------------
// Global Laravel helper functions
// ---------------------------------------------------------------------------

struct GlobalHelper {
    name: &'static str,
    return_type: &'static str,
    params: &'static str,
}

const fn helper(name: &'static str, return_type: &'static str, params: &'static str) -> GlobalHelper {
    GlobalHelper { name, return_type, params }
}

static GLOBAL_HELPERS: &[GlobalHelper] = &[
    helper("trans",          "string",  "$key = null, $replace = [], $locale = null"),
    helper("__",             "string",  "$key = null, $replace = [], $locale = null"),
    helper("config",         "mixed",   "$key = null, $default = null"),
    helper("route",          "string",  "$name, $parameters = [], $absolute = true"),
    helper("redirect",       "mixed",   "$to = null, $status = 302, $headers = []"),
    helper("view",           "mixed",   "$view = null, $data = [], $mergeData = []"),
    helper("auth",           "mixed",   "$guard = null"),
    helper("abort",          "void",    "$code = 404, $message = '', $headers = []"),
    helper("abort_if",       "void",    "$boolean, $code, $message = '', $headers = []"),
    helper("abort_unless",   "void",    "$boolean, $code, $message = '', $headers = []"),
    helper("session",        "mixed",   "$key = null, $default = null"),
    helper("request",        "mixed",   "$key = null, $default = null"),
    helper("response",       "mixed",   "$content = '', $status = 200, $headers = []"),
    helper("app",            "mixed",   "$abstract = null, $parameters = []"),
    helper("resolve",        "mixed",   "$abstract, $parameters = []"),
    helper("event",          "mixed",   "$event"),
    helper("dispatch",       "mixed",   "$job"),
    helper("cache",          "mixed",   "$key = null, $default = null"),
    helper("cookie",         "mixed",   "$name = null, $value = null, $minutes = 0"),
    helper("back",           "mixed",   ""),
    helper("url",            "string",  "$path = null, $parameters = [], $secure = null"),
    helper("asset",          "string",  "$path, $secure = null"),
    helper("secure_asset",   "string",  "$path"),
    helper("env",            "mixed",   "$key, $default = null"),
    helper("storage_path",   "string",  "$path = ''"),
    helper("public_path",    "string",  "$path = ''"),
    helper("resource_path",  "string",  "$path = ''"),
    helper("base_path",      "string",  "$path = ''"),
    helper("app_path",       "string",  "$path = ''"),
    helper("database_path",  "string",  "$path = ''"),
    helper("config_path",    "string",  "$path = ''"),
    helper("lang_path",      "string",  "$path = ''"),
    helper("mix",            "string",  "$path, $manifestDirectory = ''"),
    helper("now",            "mixed",   "$tz = null"),
    helper("today",          "mixed",   "$tz = null"),
    helper("logger",         "mixed",   "$message = null, $context = []"),
    helper("info",           "void",    "$message, $context = []"),
    helper("report",         "void",    "$exception"),
    helper("rescue",         "mixed",   "$callback, $rescue = null, $report = true"),
    helper("retry",          "mixed",   "$times, $callback, $sleep = 0, $when = null"),
    helper("throw_if",       "mixed",   "$condition, $exception, $message = ''"),
    helper("throw_unless",   "mixed",   "$condition, $exception, $message = ''"),
    helper("value",          "mixed",   "$value"),
    helper("with",           "mixed",   "$value, $callback = null"),
    helper("filled",         "bool",    "$value"),
    helper("blank",          "bool",    "$value"),
    helper("optional",       "mixed",   "$value = null, $callback = null"),
    helper("data_get",       "mixed",   "$target, $key, $default = null"),
    helper("data_set",       "mixed",   "$target, $key, $value, $overwrite = true"),
    helper("head",           "mixed",   "$array"),
    helper("last",           "mixed",   "$array"),
    helper("collect",        "Collection", "$value = null"),
    helper("str",            "mixed",   "$string = null"),
    helper("put",            "mixed",   "$key, $value"),
    helper("encrypt",        "string",  "$value, $serialize = true"),
    helper("decrypt",        "mixed",   "$value, $unserialize = true"),
    helper("bcrypt",         "string",  "$value, $options = []"),
    helper("hash",           "mixed",   "$driver = null"),
    helper("validator",      "mixed",   "$data = [], $rules = [], $messages = [], $attributes = []"),
    helper("old",            "mixed",   "$key = null, $default = null"),
    helper("errors",         "mixed",   ""),
    helper("csrf_token",     "string",  ""),
    helper("csrf_field",     "string",  ""),
    helper("method_field",   "string",  "$method"),
];

// =============================================================================
// Synthesis
// =============================================================================

fn synth_class(
    class_short: &str,
    class_qname: &str,
    methods: &[FluentMethod],
    parent_idx: usize,
) -> Vec<ExtractedSymbol> {
    let mut out = Vec::with_capacity(methods.len() + 1);

    out.push(ExtractedSymbol {
        name: class_short.to_string(),
        qualified_name: class_qname.to_string(),
        kind: SymbolKind::Class,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        signature: Some(format!("class {class_short}")),
        doc_comment: None,
        scope_path: None,
        parent_index: None,
    });

    for m in methods {
        out.push(ExtractedSymbol {
            name: m.name.to_string(),
            qualified_name: format!("{class_qname}.{}", m.name),
            kind: SymbolKind::Method,
            visibility: Some(Visibility::Public),
            start_line: 0,
            end_line: 0,
            start_col: 0,
            end_col: 0,
            signature: Some(format!("{}({}): {}", m.name, m.params, m.return_type)),
            doc_comment: None,
            scope_path: Some(class_qname.to_string()),
            parent_index: Some(parent_idx),
        });
    }

    out
}

fn synth_globals(parent_file_idx: usize) -> Vec<ExtractedSymbol> {
    GLOBAL_HELPERS
        .iter()
        .map(|h| ExtractedSymbol {
            name: h.name.to_string(),
            qualified_name: h.name.to_string(),
            kind: SymbolKind::Function,
            visibility: Some(Visibility::Public),
            start_line: 0,
            end_line: 0,
            start_col: 0,
            end_col: 0,
            signature: Some(format!("{}({}): {}", h.name, h.params, h.return_type)),
            doc_comment: None,
            scope_path: None,
            parent_index: Some(parent_file_idx),
        })
        .collect()
}

fn synthesize_all() -> Vec<ParsedFile> {
    // -------------------------------------------------------------------------
    // File 0: Eloquent Model — forwards all static Builder methods
    //
    // PHP Eloquent Model uses `__callStatic` to proxy static calls to the
    // Builder, e.g. `File::whereIn(...)`, `User::findOrFail(...)`.  Project
    // Model subclasses inherit from the vendor `Model` class (qname "Model" in
    // the index).  We synthesize a stub with qname "Model.*" so the chain-walker
    // inheritance walk can find these methods when it reaches the parent class.
    // -------------------------------------------------------------------------
    let model_symbols = synth_class(
        "Model",
        "Model",
        BUILDER_METHODS,
        0,
    );
    let model_file = ParsedFile {
        path: "ext:laravel-stubs:eloquent/ModelForwarded.php".to_string(),
        language: "php".to_string(),
        content_hash: format!("laravel-stubs-model-{}", BUILDER_METHODS.len()),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        symbols: model_symbols,
        refs: Vec::new(),
        routes: Vec::new(),
        db_sets: Vec::new(),
        symbol_origin_languages: Vec::new(),
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: Vec::new(),
        content: None,
        has_errors: false,
        flow: crate::types::FlowMeta::default(),
        connection_points: Vec::new(),
        demand_contributions: Vec::new(),
    };

    // -------------------------------------------------------------------------
    // File 1: Eloquent Builder
    // -------------------------------------------------------------------------
    let builder_symbols = synth_class(
        "Builder",
        "Illuminate.Database.Eloquent.Builder",
        BUILDER_METHODS,
        0,
    );
    let builder_file = ParsedFile {
        path: "ext:laravel-stubs:eloquent/Builder.php".to_string(),
        language: "php".to_string(),
        content_hash: format!("laravel-stubs-builder-{}", BUILDER_METHODS.len()),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        symbols: builder_symbols,
        refs: Vec::new(),
        routes: Vec::new(),
        db_sets: Vec::new(),
        symbol_origin_languages: Vec::new(),
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: Vec::new(),
        content: None,
        has_errors: false,
        flow: crate::types::FlowMeta::default(),
        connection_points: Vec::new(),
        demand_contributions: Vec::new(),
    };

    // -------------------------------------------------------------------------
    // File 2: Collection
    // -------------------------------------------------------------------------
    let collection_symbols = synth_class(
        "Collection",
        "Illuminate.Support.Collection",
        COLLECTION_METHODS,
        0,
    );
    let collection_file = ParsedFile {
        path: "ext:laravel-stubs:support/Collection.php".to_string(),
        language: "php".to_string(),
        content_hash: format!("laravel-stubs-collection-{}", COLLECTION_METHODS.len()),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        symbols: collection_symbols,
        refs: Vec::new(),
        routes: Vec::new(),
        db_sets: Vec::new(),
        symbol_origin_languages: Vec::new(),
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: Vec::new(),
        content: None,
        has_errors: false,
        flow: crate::types::FlowMeta::default(),
        connection_points: Vec::new(),
        demand_contributions: Vec::new(),
    };

    // -------------------------------------------------------------------------
    // File 3: Global helpers
    // -------------------------------------------------------------------------
    let global_symbols = synth_globals(0);
    let globals_file = ParsedFile {
        path: "ext:laravel-stubs:helpers.php".to_string(),
        language: "php".to_string(),
        content_hash: format!("laravel-stubs-helpers-{}", GLOBAL_HELPERS.len()),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        symbols: global_symbols,
        refs: Vec::new(),
        routes: Vec::new(),
        db_sets: Vec::new(),
        symbol_origin_languages: Vec::new(),
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: Vec::new(),
        content: None,
        has_errors: false,
        flow: crate::types::FlowMeta::default(),
        connection_points: Vec::new(),
        demand_contributions: Vec::new(),
    };

    // -------------------------------------------------------------------------
    // File 4: Short-name aliases for the chain walker
    //
    // The PHP chain walker resolves `Builder` to
    // `Illuminate.Database.Eloquent.Builder` via `external_type_qname()` when
    // Composer has the vendor tree indexed.  When it hasn't, or during the
    // short-name lookup phase, also index both classes under their bare names
    // so `return_type_name("Builder.whereIn")` resolves directly.
    // -------------------------------------------------------------------------
    let builder_alias = synth_class("Builder", "Builder", BUILDER_METHODS, 0);
    let builder_alias_file = ParsedFile {
        path: "ext:laravel-stubs:eloquent/BuilderAlias.php".to_string(),
        language: "php".to_string(),
        content_hash: format!("laravel-stubs-builder-alias-{}", BUILDER_METHODS.len()),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        symbols: builder_alias,
        refs: Vec::new(),
        routes: Vec::new(),
        db_sets: Vec::new(),
        symbol_origin_languages: Vec::new(),
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: Vec::new(),
        content: None,
        has_errors: false,
        flow: crate::types::FlowMeta::default(),
        connection_points: Vec::new(),
        demand_contributions: Vec::new(),
    };

    let collection_alias = synth_class("Collection", "Collection", COLLECTION_METHODS, 0);
    let collection_alias_file = ParsedFile {
        path: "ext:laravel-stubs:support/CollectionAlias.php".to_string(),
        language: "php".to_string(),
        content_hash: format!("laravel-stubs-collection-alias-{}", COLLECTION_METHODS.len()),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        symbols: collection_alias,
        refs: Vec::new(),
        routes: Vec::new(),
        db_sets: Vec::new(),
        symbol_origin_languages: Vec::new(),
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: Vec::new(),
        content: None,
        has_errors: false,
        flow: crate::types::FlowMeta::default(),
        connection_points: Vec::new(),
        demand_contributions: Vec::new(),
    };

    vec![
        model_file,
        builder_file,
        collection_file,
        globals_file,
        builder_alias_file,
        collection_alias_file,
    ]
}

// =============================================================================
// Synthetic dep root (no on-disk path)
// =============================================================================

fn synthetic_dep_root() -> ExternalDepRoot {
    ExternalDepRoot {
        module_path: "laravel-stubs".to_string(),
        version: String::new(),
        root: std::path::PathBuf::from("ext:laravel-stubs"),
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    }
}

// =============================================================================
// Ecosystem impl
// =============================================================================

pub struct LaravelStubsEcosystem;

impl Ecosystem for LaravelStubsEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Stdlib }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::LanguagePresent("php")
    }

    fn locate_roots(&self, _ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        vec![synthetic_dep_root()]
    }

    fn walk_root(&self, _dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        Vec::new()
    }

    fn uses_demand_driven_parse(&self) -> bool { true }

    fn parse_metadata_only(&self, _dep: &ExternalDepRoot) -> Option<Vec<ParsedFile>> {
        Some(synthesize_all())
    }
}

impl ExternalSourceLocator for LaravelStubsEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }

    fn locate_roots(&self, _project_root: &Path) -> Vec<ExternalDepRoot> {
        vec![synthetic_dep_root()]
    }

    fn parse_metadata_only(&self, _project_root: &Path) -> Option<Vec<ParsedFile>> {
        Some(synthesize_all())
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn all_symbols() -> Vec<ExtractedSymbol> {
        synthesize_all()
            .into_iter()
            .flat_map(|pf| pf.symbols)
            .collect()
    }

    #[test]
    fn builder_where_in_present() {
        let syms = all_symbols();
        assert!(
            syms.iter()
                .any(|s| s.qualified_name == "Illuminate.Database.Eloquent.Builder.whereIn"),
            "Illuminate.Database.Eloquent.Builder.whereIn must be synthesized"
        );
    }

    #[test]
    fn builder_short_name_alias_present() {
        let syms = all_symbols();
        assert!(
            syms.iter().any(|s| s.qualified_name == "Builder.whereIn"),
            "Builder.whereIn (short-name alias) must be synthesized"
        );
    }

    #[test]
    fn builder_fluent_methods_return_builder() {
        let syms = all_symbols();
        let fluent_methods = [
            "Builder.whereIn",
            "Builder.withCount",
            "Builder.where",
            "Builder.with",
            "Builder.orderBy",
        ];
        for qname in fluent_methods {
            let sym = syms.iter().find(|s| s.qualified_name == qname)
                .unwrap_or_else(|| panic!("{qname} must be synthesized"));
            let sig = sym.signature.as_deref().unwrap_or("");
            assert!(
                sig.ends_with("): Builder"),
                "{qname} signature must end with '): Builder', got: {sig:?}"
            );
        }
    }

    #[test]
    fn builder_terminal_method_get_returns_collection() {
        let syms = all_symbols();
        let sym = syms.iter()
            .find(|s| s.qualified_name == "Builder.get")
            .expect("Builder.get must be synthesized");
        let sig = sym.signature.as_deref().unwrap_or("");
        assert!(
            sig.ends_with("): Collection"),
            "Builder.get signature must end with '): Collection', got: {sig:?}"
        );
    }

    #[test]
    fn collection_methods_present() {
        let syms = all_symbols();
        let expected = [
            "Illuminate.Support.Collection.filter",
            "Illuminate.Support.Collection.map",
            "Illuminate.Support.Collection.pluck",
            "Collection.filter",
            "Collection.map",
        ];
        for qname in expected {
            assert!(
                syms.iter().any(|s| s.qualified_name == qname),
                "{qname} must be synthesized"
            );
        }
    }

    #[test]
    fn global_helpers_present() {
        let syms = all_symbols();
        let helpers = ["trans", "__", "config", "route", "put", "collect"];
        for name in helpers {
            assert!(
                syms.iter().any(|s| s.qualified_name == name && s.kind == SymbolKind::Function),
                "global helper `{name}` must be synthesized as Function"
            );
        }
    }

    #[test]
    fn global_helpers_have_return_type_in_signature() {
        let syms = all_symbols();
        let trans = syms.iter()
            .find(|s| s.qualified_name == "trans")
            .expect("trans helper must be synthesized");
        let sig = trans.signature.as_deref().unwrap_or("");
        assert!(
            sig.contains("): "),
            "trans signature must contain '): ' for return type parsing, got: {sig:?}"
        );
    }

    #[test]
    fn virtual_paths_follow_convention() {
        let files = synthesize_all();
        for pf in &files {
            assert!(
                pf.path.starts_with("ext:laravel-stubs:"),
                "virtual path must start with ext:laravel-stubs: — got {}",
                pf.path
            );
        }
    }

    #[test]
    fn synthesize_all_produces_six_files() {
        let files = synthesize_all();
        assert_eq!(
            files.len(),
            6,
            "expected 6 synthetic files (ModelForwarded, Builder, Collection, helpers, Builder alias, Collection alias), got {}",
            files.len()
        );
    }

    #[test]
    fn symbol_count_reasonable() {
        let syms = all_symbols();
        assert!(
            syms.len() >= 200,
            "expected >= 200 synthetic symbols, got {}",
            syms.len()
        );
    }

    // -------------------------------------------------------------------------
    // Model forwarded stubs — cover Eloquent static method forwarding
    // (e.g. `File::whereIn(...)`, `User::findOrFail(...)`)
    // -------------------------------------------------------------------------

    #[test]
    fn model_forwarded_where_in_present() {
        let syms = all_symbols();
        assert!(
            syms.iter().any(|s| s.qualified_name == "Model.whereIn"),
            "Model.whereIn must be synthesized for static Eloquent call resolution"
        );
    }

    #[test]
    fn model_forwarded_find_or_fail_present() {
        let syms = all_symbols();
        assert!(
            syms.iter().any(|s| s.qualified_name == "Model.findOrFail"),
            "Model.findOrFail must be synthesized for static Eloquent call resolution"
        );
    }

    #[test]
    fn model_forwarded_with_count_present() {
        let syms = all_symbols();
        assert!(
            syms.iter().any(|s| s.qualified_name == "Model.withCount"),
            "Model.withCount must be synthesized for static Eloquent call resolution"
        );
    }

    #[test]
    fn model_forwarded_file_has_correct_virtual_path() {
        let files = synthesize_all();
        assert!(
            files.iter().any(|f| f.path == "ext:laravel-stubs:eloquent/ModelForwarded.php"),
            "ModelForwarded file must have the ext:laravel-stubs: prefix"
        );
    }
}
