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
//   4. Eloquent relation class hierarchy (HasMany, HasOne, BelongsTo, etc.)
//      with Inherits edges so the chain walker reaches Builder methods.
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
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, ParsedFile, SymbolKind, Visibility};
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
    terminal("increment",      "int",        "$column, $amount = 1, $extra = []"),
    terminal("decrement",      "int",        "$column, $amount = 1, $extra = []"),
];

// ---------------------------------------------------------------------------
// Relation hierarchy method surfaces
//
// The inheritance chain is:
//   Relation (base) — inherits from Builder alias
//     HasOneOrMany extends Relation
//       HasMany extends HasOneOrMany
//       HasOne  extends HasOneOrMany
//     BelongsTo extends Relation
//     BelongsToMany extends Relation
//     MorphOneOrMany extends HasOneOrMany
//       MorphMany extends MorphOneOrMany
//       MorphOne  extends MorphOneOrMany
//     MorphTo extends Relation
//     MorphToMany extends BelongsToMany
//
// All relation classes inherit from Relation which in turn inherits Builder,
// so the inheritance walk naturally reaches Builder.* after 2-3 hops.
// Additionally each relation class also directly exposes the most common
// terminal methods (get/first/create/update) so single-hop chains resolve
// without walking the whole hierarchy.
// ---------------------------------------------------------------------------

/// Methods on Relation base (forwarded Builder terminals + relation-specific).
static RELATION_METHODS: &[FluentMethod] = &[
    // Terminal forwarding — callers do ->labels()->get() / ->first()
    terminal("get",             "Collection", "$columns = ['*']"),
    terminal("first",           "mixed",      "$columns = ['*']"),
    terminal("firstOrFail",     "mixed",      "$columns = ['*']"),
    terminal("find",            "mixed",      "$id, $columns = ['*']"),
    terminal("findOrFail",      "mixed",      "$id, $columns = ['*']"),
    terminal("findMany",        "Collection", "$ids, $columns = ['*']"),
    terminal("create",          "mixed",      "$attributes = []"),
    terminal("forceCreate",     "mixed",      "$attributes = []"),
    terminal("firstOrCreate",   "mixed",      "$attributes, $values = []"),
    terminal("firstOrNew",      "mixed",      "$attributes, $values = []"),
    terminal("updateOrCreate",  "mixed",      "$attributes, $values = []"),
    terminal("update",          "int",        "$values"),
    terminal("delete",          "mixed",      ""),
    terminal("count",           "int",        "$columns = '*'"),
    terminal("exists",          "bool",       ""),
    terminal("paginate",        "mixed",      "$perPage = null, $columns = ['*']"),
    terminal("pluck",           "Collection", "$column, $key = null"),
    terminal("value",           "mixed",      "$column"),
    terminal("sum",             "mixed",      "$column"),
    terminal("min",             "mixed",      "$column"),
    terminal("max",             "mixed",      "$column"),
    terminal("avg",             "mixed",      "$column"),
    terminal("increment",       "int",        "$column, $amount = 1, $extra = []"),
    terminal("decrement",       "int",        "$column, $amount = 1, $extra = []"),
    // Fluent forwarding — allows ->labels()->where(...)->get()
    fluent("where",             "$column, $operator = null, $value = null, $boolean = 'and'"),
    fluent("orWhere",           "$column, $operator = null, $value = null"),
    fluent("whereIn",           "$column, $values, $boolean = 'and', $not = false"),
    fluent("whereNotIn",        "$column, $values, $boolean = 'and'"),
    fluent("whereNull",         "$column, $boolean = 'and', $not = false"),
    fluent("whereNotNull",      "$column, $boolean = 'and'"),
    fluent("whereBetween",      "$column, $values, $boolean = 'and', $not = false"),
    fluent("whereHas",          "$relation, $callback = null, $operator = '>=', $count = 1"),
    fluent("with",              "$relations, $callback = null"),
    fluent("withCount",         "$relations"),
    fluent("withSum",           "$relation, $column"),
    fluent("withAvg",           "$relation, $column"),
    fluent("withMax",           "$relation, $column"),
    fluent("withMin",           "$relation, $column"),
    fluent("withExists",        "$relation"),
    fluent("orderBy",           "$column, $direction = 'asc'"),
    fluent("orderByDesc",       "$column"),
    fluent("latest",            "$column = 'created_at'"),
    fluent("oldest",            "$column = 'created_at'"),
    fluent("select",            "$columns = ['*']"),
    fluent("limit",             "$value"),
    fluent("take",              "$value"),
    fluent("skip",              "$value"),
    fluent("offset",            "$value"),
    fluent("lockForUpdate",     ""),
    fluent("withTrashed",       ""),
    fluent("onlyTrashed",       ""),
    fluent("withoutTrashed",    ""),
    // Relation-specific
    fluent("getResults",        ""),
    fluent("addConstraints",    ""),
    fluent("addEagerConstraints","$models"),
    fluent("getRelationExistenceQuery","$query, $parentQuery, $columns = ['*']"),
];

/// Additional methods on HasOneOrMany (save-based helpers, etc.).
static HAS_ONE_OR_MANY_EXTRA: &[FluentMethod] = &[
    terminal("save",            "mixed",      "$model"),
    terminal("saveMany",        "Collection", "$models"),
    terminal("saveQuietly",     "mixed",      "$model"),
    terminal("create",          "mixed",      "$attributes = []"),
    terminal("createMany",      "Collection", "$records"),
    terminal("createQuietly",   "mixed",      "$attributes = []"),
    fluent("chaperone",         ""),
    fluent("withoutChaperone",  ""),
];

/// Additional methods on BelongsToMany (pivot helpers).
static BELONGS_TO_MANY_EXTRA: &[FluentMethod] = &[
    fluent("wherePivot",        "$column, $operator = null, $value = null, $boolean = 'and'"),
    fluent("orWherePivot",      "$column, $operator = null, $value = null"),
    fluent("wherePivotIn",      "$column, $values, $boolean = 'and', $not = false"),
    fluent("orWherePivotIn",    "$column, $values"),
    fluent("wherePivotNotIn",   "$column, $values, $boolean = 'and'"),
    fluent("wherePivotBetween", "$column, $values, $boolean = 'and', $not = false"),
    fluent("wherePivotNull",    "$column, $boolean = 'and', $not = false"),
    fluent("wherePivotNotNull", "$column, $boolean = 'and'"),
    fluent("withPivot",         "$columns"),
    fluent("withTimestamps",    "$createdAt = 'created_at', $updatedAt = 'updated_at'"),
    fluent("as",                "$accessor"),
    fluent("orderByPivot",      "$column, $direction = 'asc'"),
    fluent("orderByPivotDesc",  "$column"),
    terminal("attach",          "void",  "$id, $attributes = [], $touch = true"),
    terminal("detach",          "mixed", "$ids = null, $touch = true"),
    terminal("sync",            "array", "$ids, $detaching = true"),
    terminal("syncWithoutDetaching","array","$ids"),
    terminal("toggle",          "array", "$ids, $touch = true"),
    terminal("updateExistingPivot","int","$id, $attributes, $touch = true"),
];

/// Additional methods on BelongsTo (associate/dissociate helpers).
static BELONGS_TO_EXTRA: &[FluentMethod] = &[
    terminal("associate",       "mixed", "$model"),
    terminal("dissociate",      "mixed", ""),
    terminal("getChild",        "mixed", ""),
    terminal("getForeignKeyName","string",""),
    terminal("getOwnerKeyName", "string",""),
];

// ---------------------------------------------------------------------------
// Model relation builder methods (used in Model body: $this->hasMany(...))
//
// These are the methods called INSIDE the model to define a relation, e.g.:
//   public function labels(): HasMany { return $this->hasMany(Label::class); }
// The return types point at the relation short names so the chain walker can
// later find HasMany.withCount etc. via the relation class hierarchy.
// ---------------------------------------------------------------------------

static MODEL_RELATION_METHODS: &[FluentMethod] = &[
    terminal("hasMany",         "HasMany",        "$related, $foreignKey = null, $localKey = null"),
    terminal("hasOne",          "HasOne",         "$related, $foreignKey = null, $localKey = null"),
    terminal("belongsTo",       "BelongsTo",      "$related, $foreignKey = null, $ownerKey = null, $relation = null"),
    terminal("belongsToMany",   "BelongsToMany",  "$related, $table = null, $foreignPivotKey = null, $relatedPivotKey = null"),
    terminal("morphMany",       "MorphMany",      "$related, $name, $type = null, $id = null, $localKey = null"),
    terminal("morphOne",        "MorphOne",       "$related, $name, $type = null, $id = null, $localKey = null"),
    terminal("morphTo",         "MorphTo",        "$name = null, $type = null, $id = null, $ownerKey = null"),
    terminal("morphToMany",     "MorphToMany",    "$related, $name, $table = null, $foreignPivotKey = null, $relatedPivotKey = null"),
    terminal("hasManyThrough",  "HasManyThrough", "$related, $through, $firstKey = null, $secondKey = null, $localKey = null, $secondLocalKey = null"),
    terminal("hasOneThrough",   "HasOneThrough",  "$related, $through, $firstKey = null, $secondKey = null, $localKey = null, $secondLocalKey = null"),
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
// Synthesis helpers
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

/// Build a ParsedFile for a relation class that inherits from `parent_short_name`.
///
/// The `Inherits` ref is emitted from the class symbol (index 0) so the
/// `inherits_map` builder in the engine can follow the chain:
///   HasMany → HasOneOrMany → Relation → Builder (via the Builder alias file).
///
/// `extra_methods` are merged with `RELATION_METHODS` (which all relations share).
/// Pass `&[]` for classes with no extra methods beyond the common Relation surface.
fn synth_relation_file(
    class_short: &str,
    class_qname: &str,
    extra_methods: &[FluentMethod],
    parent_short_name: &str,
) -> ParsedFile {
    // Merge RELATION_METHODS + any class-specific extras.
    let mut all_methods: Vec<&FluentMethod> = RELATION_METHODS.iter().collect();
    all_methods.extend(extra_methods.iter());

    let mut symbols = Vec::with_capacity(all_methods.len() + 1);
    // Symbol 0: the class itself.
    symbols.push(ExtractedSymbol {
        name: class_short.to_string(),
        qualified_name: class_qname.to_string(),
        kind: SymbolKind::Class,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        signature: Some(format!("class {class_short} extends {parent_short_name}")),
        doc_comment: None,
        scope_path: None,
        parent_index: None,
    });
    for m in &all_methods {
        symbols.push(ExtractedSymbol {
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
            parent_index: Some(0),
        });
    }

    // Emit the Inherits edge so the engine populates `inherits_map`.
    // source_symbol_index = 0 → the class symbol above.
    // target_name = parent short name → resolved via by_name in the engine.
    let refs = vec![ExtractedRef {
        source_symbol_index: 0,
        target_name: parent_short_name.to_string(),
        kind: EdgeKind::Inherits,
        line: 0,
        module: None,
        chain: None,
        byte_offset: 0,
            namespace_segments: Vec::new(),
}];

    ParsedFile {
        path: format!("ext:laravel-stubs:eloquent/relations/{class_short}.php"),
        language: "php".to_string(),
        content_hash: format!("laravel-stubs-relation-{class_short}-{}", all_methods.len()),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        symbols,
        refs,
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
    }
}

fn synthesize_all() -> Vec<ParsedFile> {
    // -------------------------------------------------------------------------
    // File 0: Eloquent Model — forwards all static Builder methods + relation
    // builder methods (`hasMany`, `hasOne`, etc.)
    //
    // PHP Eloquent Model uses `__callStatic` to proxy static calls to the
    // Builder, e.g. `File::whereIn(...)`, `User::findOrFail(...)`.  Project
    // Model subclasses inherit from the vendor `Model` class (qname "Model" in
    // the index).  We synthesize a stub with qname "Model.*" so the chain-walker
    // inheritance walk can find these methods when it reaches the parent class.
    //
    // Relation builder methods are also merged in so `$this->hasMany(...)` calls
    // resolve to their corresponding relation return types.
    // -------------------------------------------------------------------------
    let mut model_all_methods: Vec<&FluentMethod> = BUILDER_METHODS.iter().collect();
    model_all_methods.extend(MODEL_RELATION_METHODS.iter());
    let model_symbols = {
        let mut out = Vec::with_capacity(model_all_methods.len() + 1);
        out.push(ExtractedSymbol {
            name: "Model".to_string(),
            qualified_name: "Model".to_string(),
            kind: SymbolKind::Class,
            visibility: Some(Visibility::Public),
            start_line: 0, end_line: 0, start_col: 0, end_col: 0,
            signature: Some("class Model".to_string()),
            doc_comment: None,
            scope_path: None,
            parent_index: None,
        });
        for m in &model_all_methods {
            out.push(ExtractedSymbol {
                name: m.name.to_string(),
                qualified_name: format!("Model.{}", m.name),
                kind: SymbolKind::Method,
                visibility: Some(Visibility::Public),
                start_line: 0, end_line: 0, start_col: 0, end_col: 0,
                signature: Some(format!("{}({}): {}", m.name, m.params, m.return_type)),
                doc_comment: None,
                scope_path: Some("Model".to_string()),
                parent_index: Some(0),
            });
        }
        out
    };
    let model_file = ParsedFile {
        path: "ext:laravel-stubs:eloquent/ModelForwarded.php".to_string(),
        language: "php".to_string(),
        content_hash: format!("laravel-stubs-model-{}", model_all_methods.len()),
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

    // -------------------------------------------------------------------------
    // Files 6-18: Eloquent relation hierarchy
    //
    // Chain: HasMany / HasOne → HasOneOrMany → Relation → Builder (alias)
    //        BelongsTo → Relation → Builder (alias)
    //        BelongsToMany → Relation → Builder (alias)
    //        MorphMany / MorphOne → MorphOneOrMany → HasOneOrMany → Relation → Builder
    //        MorphTo → Relation → Builder (alias)
    //        MorphToMany → BelongsToMany → Relation → Builder (alias)
    //
    // "Relation" itself is the base; it inherits from "Builder" (the short-name
    // alias) so the chain walker's inheritance walk reaches Builder.withCount
    // etc. in ≤5 hops.
    // -------------------------------------------------------------------------

    // Relation base — inherits from Builder (the short-name alias).
    let relation_file = synth_relation_file(
        "Relation",
        "Illuminate.Database.Eloquent.Relations.Relation",
        &[],
        "Builder",
    );

    // HasOneOrMany — inherits from Relation, adds save* / create* helpers.
    let has_one_or_many_file = synth_relation_file(
        "HasOneOrMany",
        "Illuminate.Database.Eloquent.Relations.HasOneOrMany",
        HAS_ONE_OR_MANY_EXTRA,
        "Relation",
    );

    // HasMany — inherits from HasOneOrMany.
    let has_many_file = synth_relation_file(
        "HasMany",
        "Illuminate.Database.Eloquent.Relations.HasMany",
        &[],
        "HasOneOrMany",
    );

    // HasOne — inherits from HasOneOrMany.
    let has_one_file = synth_relation_file(
        "HasOne",
        "Illuminate.Database.Eloquent.Relations.HasOne",
        &[],
        "HasOneOrMany",
    );

    // BelongsTo — inherits from Relation, adds associate/dissociate.
    let belongs_to_file = synth_relation_file(
        "BelongsTo",
        "Illuminate.Database.Eloquent.Relations.BelongsTo",
        BELONGS_TO_EXTRA,
        "Relation",
    );

    // BelongsToMany — inherits from Relation, adds pivot helpers.
    let belongs_to_many_file = synth_relation_file(
        "BelongsToMany",
        "Illuminate.Database.Eloquent.Relations.BelongsToMany",
        BELONGS_TO_MANY_EXTRA,
        "Relation",
    );

    // MorphOneOrMany — inherits from HasOneOrMany.
    let morph_one_or_many_file = synth_relation_file(
        "MorphOneOrMany",
        "Illuminate.Database.Eloquent.Relations.MorphOneOrMany",
        HAS_ONE_OR_MANY_EXTRA,
        "HasOneOrMany",
    );

    // MorphMany — inherits from MorphOneOrMany.
    let morph_many_file = synth_relation_file(
        "MorphMany",
        "Illuminate.Database.Eloquent.Relations.MorphMany",
        &[],
        "MorphOneOrMany",
    );

    // MorphOne — inherits from MorphOneOrMany.
    let morph_one_file = synth_relation_file(
        "MorphOne",
        "Illuminate.Database.Eloquent.Relations.MorphOne",
        &[],
        "MorphOneOrMany",
    );

    // MorphTo — inherits from Relation (special: resolves to different model per type).
    let morph_to_file = synth_relation_file(
        "MorphTo",
        "Illuminate.Database.Eloquent.Relations.MorphTo",
        &[],
        "Relation",
    );

    // MorphToMany — inherits from BelongsToMany.
    let morph_to_many_file = synth_relation_file(
        "MorphToMany",
        "Illuminate.Database.Eloquent.Relations.MorphToMany",
        BELONGS_TO_MANY_EXTRA,
        "BelongsToMany",
    );

    // HasManyThrough / HasOneThrough — inherit from Relation.
    let has_many_through_file = synth_relation_file(
        "HasManyThrough",
        "Illuminate.Database.Eloquent.Relations.HasManyThrough",
        &[],
        "Relation",
    );

    let has_one_through_file = synth_relation_file(
        "HasOneThrough",
        "Illuminate.Database.Eloquent.Relations.HasOneThrough",
        &[],
        "Relation",
    );

    vec![
        model_file,
        builder_file,
        collection_file,
        globals_file,
        builder_alias_file,
        collection_alias_file,
        // Relations (files 6-18)
        relation_file,
        has_one_or_many_file,
        has_many_file,
        has_one_file,
        belongs_to_file,
        belongs_to_many_file,
        morph_one_or_many_file,
        morph_many_file,
        morph_one_file,
        morph_to_file,
        morph_to_many_file,
        has_many_through_file,
        has_one_through_file,
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
    fn synthesize_all_produces_expected_files() {
        let files = synthesize_all();
        // 6 original + 13 relation files (Relation, HasOneOrMany, HasMany, HasOne,
        // BelongsTo, BelongsToMany, MorphOneOrMany, MorphMany, MorphOne, MorphTo,
        // MorphToMany, HasManyThrough, HasOneThrough)
        assert_eq!(
            files.len(),
            19,
            "expected 19 synthetic files, got {}",
            files.len()
        );
    }

    #[test]
    fn symbol_count_reasonable() {
        let syms = all_symbols();
        assert!(
            syms.len() >= 400,
            "expected >= 400 synthetic symbols (including relation stubs), got {}",
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

    // -------------------------------------------------------------------------
    // Relation hierarchy tests
    // -------------------------------------------------------------------------

    #[test]
    fn model_has_relation_builder_methods() {
        let syms = all_symbols();
        let expected = [
            "Model.hasMany",
            "Model.hasOne",
            "Model.belongsTo",
            "Model.belongsToMany",
            "Model.morphMany",
            "Model.morphOne",
            "Model.morphTo",
            "Model.morphToMany",
            "Model.hasManyThrough",
            "Model.hasOneThrough",
        ];
        for qname in expected {
            assert!(
                syms.iter().any(|s| s.qualified_name == qname),
                "{qname} must be synthesized on Model"
            );
        }
    }

    #[test]
    fn model_has_many_returns_has_many_type() {
        let syms = all_symbols();
        let sym = syms.iter()
            .find(|s| s.qualified_name == "Model.hasMany")
            .expect("Model.hasMany must be synthesized");
        let sig = sym.signature.as_deref().unwrap_or("");
        assert!(
            sig.ends_with("): HasMany"),
            "Model.hasMany must declare return type HasMany, got: {sig:?}"
        );
    }

    #[test]
    fn relation_classes_present() {
        let syms = all_symbols();
        let expected_qnames = [
            "Illuminate.Database.Eloquent.Relations.Relation",
            "Illuminate.Database.Eloquent.Relations.HasOneOrMany",
            "Illuminate.Database.Eloquent.Relations.HasMany",
            "Illuminate.Database.Eloquent.Relations.HasOne",
            "Illuminate.Database.Eloquent.Relations.BelongsTo",
            "Illuminate.Database.Eloquent.Relations.BelongsToMany",
            "Illuminate.Database.Eloquent.Relations.MorphOneOrMany",
            "Illuminate.Database.Eloquent.Relations.MorphMany",
            "Illuminate.Database.Eloquent.Relations.MorphOne",
            "Illuminate.Database.Eloquent.Relations.MorphTo",
            "Illuminate.Database.Eloquent.Relations.MorphToMany",
            "Illuminate.Database.Eloquent.Relations.HasManyThrough",
            "Illuminate.Database.Eloquent.Relations.HasOneThrough",
        ];
        for qname in expected_qnames {
            assert!(
                syms.iter().any(|s| s.qualified_name == qname && s.kind == SymbolKind::Class),
                "relation class {qname} must be synthesized"
            );
        }
    }

    #[test]
    fn relation_classes_have_get_method() {
        let syms = all_symbols();
        // Every relation class should expose `get` directly (not just via inheritance)
        // so single-hop chains like `->labels()->get()` resolve without an inheritance walk.
        let relation_classes = [
            "Illuminate.Database.Eloquent.Relations.HasMany",
            "Illuminate.Database.Eloquent.Relations.HasOne",
            "Illuminate.Database.Eloquent.Relations.BelongsTo",
            "Illuminate.Database.Eloquent.Relations.BelongsToMany",
            "Illuminate.Database.Eloquent.Relations.MorphMany",
            "Illuminate.Database.Eloquent.Relations.MorphOne",
        ];
        for cls in relation_classes {
            let get_qname = format!("{cls}.get");
            assert!(
                syms.iter().any(|s| s.qualified_name == get_qname),
                "{get_qname} must be directly on the relation class"
            );
        }
    }

    #[test]
    fn has_many_exposes_with_count() {
        let syms = all_symbols();
        assert!(
            syms.iter().any(|s| s.qualified_name == "Illuminate.Database.Eloquent.Relations.HasMany.withCount"),
            "HasMany.withCount must be synthesized"
        );
    }

    #[test]
    fn belongs_to_many_exposes_where_pivot() {
        let syms = all_symbols();
        let qname = "Illuminate.Database.Eloquent.Relations.BelongsToMany.wherePivot";
        assert!(
            syms.iter().any(|s| s.qualified_name == qname),
            "{qname} must be synthesized"
        );
    }

    #[test]
    fn belongs_to_exposes_associate() {
        let syms = all_symbols();
        let qname = "Illuminate.Database.Eloquent.Relations.BelongsTo.associate";
        assert!(
            syms.iter().any(|s| s.qualified_name == qname),
            "{qname} must be synthesized"
        );
    }

    #[test]
    fn relation_files_have_inherits_refs() {
        let files = synthesize_all();
        // Every relation file (path contains /relations/) must have exactly one
        // Inherits ref pointing at the parent class.
        for pf in files.iter().filter(|f| f.path.contains("/relations/")) {
            let inherits: Vec<_> = pf.refs.iter().filter(|r| r.kind == EdgeKind::Inherits).collect();
            assert_eq!(
                inherits.len(),
                1,
                "relation file {} must have exactly 1 Inherits ref, got {}",
                pf.path,
                inherits.len()
            );
            // The Inherits ref must point at the class symbol (index 0).
            assert_eq!(
                inherits[0].source_symbol_index,
                0,
                "Inherits ref in {} must point at symbol index 0 (the class)",
                pf.path
            );
        }
    }

    #[test]
    fn relation_virtual_paths_follow_convention() {
        let files = synthesize_all();
        for pf in files.iter().filter(|f| f.path.contains("/relations/")) {
            assert!(
                pf.path.starts_with("ext:laravel-stubs:eloquent/relations/"),
                "relation file path must match convention, got: {}",
                pf.path
            );
        }
    }

    #[test]
    fn morph_to_many_has_where_pivot() {
        let syms = all_symbols();
        // MorphToMany inherits BELONGS_TO_MANY_EXTRA so wherePivot is directly on it.
        let qname = "Illuminate.Database.Eloquent.Relations.MorphToMany.wherePivot";
        assert!(
            syms.iter().any(|s| s.qualified_name == qname),
            "{qname} must be synthesized on MorphToMany"
        );
    }

    #[test]
    fn has_one_or_many_has_save_method() {
        let syms = all_symbols();
        let qname = "Illuminate.Database.Eloquent.Relations.HasOneOrMany.save";
        assert!(
            syms.iter().any(|s| s.qualified_name == qname),
            "{qname} must be synthesized"
        );
    }
}
