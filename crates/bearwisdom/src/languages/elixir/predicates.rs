// =============================================================================
// elixir/predicates.rs — Elixir builtin and helper predicates
// =============================================================================

use crate::types::EdgeKind;

/// Check that the edge kind is compatible with the symbol kind.
pub(super) fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::Calls => matches!(
            sym_kind,
            "method" | "function" | "constructor" | "test" | "property" | "module"
        ),
        EdgeKind::Inherits => matches!(sym_kind, "class" | "module"),
        EdgeKind::Implements => matches!(sym_kind, "class" | "module" | "interface"),
        EdgeKind::TypeRef => matches!(
            sym_kind,
            "class" | "module" | "interface" | "enum" | "type_alias" | "namespace"
        ),
        EdgeKind::Instantiates => matches!(sym_kind, "class" | "module"),
        _ => true,
    }
}

/// Always-external Elixir/Erlang/OTP module roots.
///
/// Elixir module names are dot-separated CamelCase atoms starting with a
/// capital letter. We check the root segment.
const ALWAYS_EXTERNAL: &[&str] = &[
    // Erlang/OTP (bare atoms, lowercase)
    "erlang",
    "lists",
    "maps",
    "string",
    "io",
    "file",
    "timer",
    "ets",
    "dets",
    "mnesia",
    "gen_server",
    "gen_event",
    "gen_statem",
    "supervisor",
    "application",
    "code",
    "crypto",
    "os",
    "net_kernel",
    "node",
    "rpc",
    "proc_lib",
    "sys",
    // Elixir stdlib
    "Elixir",
    "Kernel",
    "IO",
    "Enum",
    "Map",
    "MapSet",
    "List",
    "String",
    "Integer",
    "Float",
    "Atom",
    "Tuple",
    "Process",
    "Port",
    "Node",
    "File",
    "Path",
    "System",
    "Code",
    "Macro",
    "Module",
    "Agent",
    "Task",
    "GenServer",
    "GenEvent",
    "GenStateMachine",
    "Supervisor",
    "Application",
    "Registry",
    "DynamicSupervisor",
    "PartitionSupervisor",
    "Stream",
    "Range",
    "Regex",
    "URI",
    "DateTime",
    "Date",
    "Time",
    "NaiveDateTime",
    "Calendar",
    "Duration",
    "Keyword",
    "Access",
    "Bitwise",
    "Base",
    "Protocol",
    "Behaviour",
    "Inspect",
    "Collectable",
    "Enumerable",
    "OptionParser",
    "StringIO",
    "Version",
    "Config",
    "Function",
    "Record",
    "Set",
    "Dict",
    "HashDict",
    "HashSet",
    // Elixir exception types
    "ArgumentError",
    "ArithmeticError",
    "BadArityError",
    "BadBooleanError",
    "BadFunctionError",
    "BadMapError",
    "BadStructError",
    "CaseClauseError",
    "CompileError",
    "CondClauseError",
    "ErlangError",
    "FunctionClauseError",
    "KeyError",
    "MatchError",
    "RuntimeError",
    "SyntaxError",
    "SystemLimitError",
    "TokenMissingError",
    "TryClauseError",
    "UndefinedFunctionError",
    "WithClauseError",
    "UnicodeConversionError",
    // Testing
    "ExUnit",
    "Mix",
    // Hex packages
    "Phoenix",
    "Ecto",
    "Plug",
    "Tesla",
    "Jason",
    "Logger",
    "Poison",
    "Swoosh",
    "Oban",
    "Broadway",
    "Commanded",
    "Absinthe",
    "Ash",
    "Surface",
    "LiveView",
    "Finch",
    "Req",
    "Mint",
    "Bandit",
    "Cowboy",
    "Hackney",
    "HTTPoison",
    "HTTPotion",
    "Postgrex",
    "MyXQL",
    "Redix",
    "Cachex",
    "ConCache",
    "NimbleCSV",
    "NimbleParsec",
    "NimbleTOTP",
    "NimbleOptions",
    "NimblePool",
    "Floki",
    "Mox",
    "Bypass",
    "ExMachina",
    "Faker",
    "Credo",
    "Dialyxir",
    "ExDoc",
    "Gettext",
    "Timex",
    "Tzdata",
    "Decimal",
    "Money",
    "Bamboo",
    "Hammer",
    "Guardian",
    "Pow",
    "Comeonin",
    "Bcrypt",
    "Argon2",
    "Pbkdf2",
    "ExAws",
    "Sentry",
    "OpenApiSpex",
    "PromEx",
    "Telemetry",
    "TelemetryMetrics",
    "OpenTelemetry",
    "RefInspector",
    "UAInspector",
    "Kaffy",
    "LazyHTML",
];

/// Check whether an Elixir module alias is external (stdlib, OTP, or hex package).
pub(super) fn is_external_elixir_module(module: &str) -> bool {
    // The root segment of the module (before the first `.`).
    let root = module.split('.').next().unwrap_or(module);
    for &ext in ALWAYS_EXTERNAL {
        if root == ext {
            return true;
        }
    }
    false
}

/// Check whether `name` is commonly injected by an Ecto-schema-style
/// `__using__` macro pattern.
///
/// Many Phoenix/Ecto projects define a `Schema` module (e.g. `MyApp.Schema`,
/// `Changelog.Schema`) with a `defmacro __using__` that injects a standard
/// set of query-builder helpers via `quote do`. BearWisdom can't expand these
/// macros, so the injected function bodies are never seen as top-level symbols.
///
/// This predicate recognises the conventional helper names injected by such
/// macros. It fires when the file has `use <Anything>.Schema` (or similar
/// project-internal schema wrappers) AND the name is in the expected set.
pub(super) fn is_schema_using_injected(name: &str) -> bool {
    matches!(
        name,
        // Common Ecto query helper names injected by project Schema modules
        "newest_first"
            | "newest_last"
            | "by_position"
            | "any?"
            | "with_ids"
            | "newer_than"
            | "older_than"
            | "hashid"
            | "decode"
            // Ecto.Query wrappers often re-exported by schema modules
            | "paginate"
            | "published"
            | "unpublished"
            | "published_after"
            | "published_before"
            | "search_by"
    )
}

/// Returns true if `module` looks like a project-internal Schema wrapper
/// (e.g. `Changelog.Schema`, `MyApp.Schemas.Base`, `AppWeb.Schema`).
pub(super) fn is_internal_schema_module(module: &str) -> bool {
    let last = module.split('.').last().unwrap_or(module);
    last == "Schema" || last == "Schemas" || last == "BaseSchema" || last == "ModelHelpers"
}

/// Check whether `name` is commonly injected by a project-internal
/// `<AppWeb>` controller wrapper module (the Phoenix 1.5+ `use AppWeb, :controller`
/// pattern, where `AppWeb.controller/0` returns a `quote do` block that
/// defines shared helpers for all controllers in the project).
///
/// These names appear in controllers that do `use ChangelogWeb, :controller`
/// (or `use MyAppWeb, :controller`) and call functions defined in the web
/// module's `quote do` block — invisible to the extractor.
pub(super) fn is_web_controller_injected(name: &str) -> bool {
    matches!(
        name,
        // Commonly defined in AppWeb.controller quote blocks
        "redirect_next"
            | "replace_next_edit_path"
            | "log_request"
            | "send_to_sentry"
            | "current_user_or_nil"
            | "require_user"
            | "is_admin?"
            | "is_editor?"
            | "is_host?"
    )
}

/// Returns true if `module` looks like a project-internal `<AppWeb>` wrapper
/// module (e.g. `ChangelogWeb`, `MyAppWeb`, `HelloWeb`).
pub(super) fn is_internal_web_module(module: &str) -> bool {
    // Single-segment CamelCase module ending in "Web" — the conventional
    // Phoenix project web module name.
    let last = module.split('.').last().unwrap_or(module);
    last.ends_with("Web") && !last.is_empty()
}

