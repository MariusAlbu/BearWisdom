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

/// Elixir builtins: functions/macros from `Kernel` which are always in scope,
/// plus common `ExUnit.Case` macros.
pub(super) fn is_elixir_builtin(name: &str) -> bool {
    let root = name.split('.').next().unwrap_or(name);
    matches!(
        root,
        // Kernel macros / special forms always in scope
        "def"
            | "defp"
            | "defmacro"
            | "defmacrop"
            | "defmodule"
            | "defstruct"
            | "defprotocol"
            | "defimpl"
            | "defdelegate"
            | "defoverridable"
            | "defexception"
            | "defguard"
            | "defguardp"
            | "use"
            | "import"
            | "require"
            | "alias"
            | "raise"
            | "reraise"
            | "throw"
            | "catch"
            | "rescue"
            | "receive"
            | "send"
            | "spawn"
            | "spawn_link"
            | "spawn_monitor"
            | "self"
            | "super"
            | "if"
            | "unless"
            | "cond"
            | "case"
            | "with"
            | "for"
            | "try"
            | "fn"
            | "quote"
            | "unquote"
            | "unquote_splicing"
            | "and"
            | "or"
            | "not"
            | "in"
            | "is_nil"
            | "is_list"
            | "is_map"
            | "is_tuple"
            | "is_atom"
            | "is_binary"
            | "is_boolean"
            | "is_bitstring"
            | "is_float"
            | "is_function"
            | "is_integer"
            | "is_number"
            | "is_pid"
            | "is_port"
            | "is_reference"
            | "is_struct"
            | "is_exception"
            | "length"
            | "hd"
            | "tl"
            | "elem"
            | "put_elem"
            | "tuple_size"
            | "map_size"
            | "byte_size"
            | "bit_size"
            | "div"
            | "rem"
            | "abs"
            | "round"
            | "trunc"
            | "floor"
            | "ceil"
            | "max"
            | "min"
            | "apply"
            | "exit"
            | "node"
            | "make_ref"
            | "to_string"
            | "to_charlist"
            | "inspect"
            | "put_in"
            | "get_in"
            | "update_in"
            | "pop_in"
            | "get_and_update_in"
            | "struct"
            | "struct!"
            // ExUnit.Case macros
            | "assert"
            | "refute"
            | "assert_raise"
            | "assert_receive"
            | "assert_received"
            | "refute_receive"
            | "refute_received"
            | "describe"
            | "test"
            | "setup"
            | "setup_all"
            | "on_exit"
            | "flunk"
            // IO.puts shorthand (extremely common bare call)
            | "IO"
            // Logger shorthand
            | "Logger"
    )
}

/// Check whether a module name matches the Phoenix test-case wrapper pattern.
///
/// Phoenix projects conventionally define `*ConnCase`, `*ChannelCase`, and
/// `*DataCase` modules in `test/support/` that re-export `Phoenix.ConnTest`
/// helpers via `using do ... import Phoenix.ConnTest ... end`. These modules
/// are internal to the project so the external-module guard in the resolver
/// would skip them — this predicate identifies them by convention.
pub(super) fn is_phoenix_test_case_wrapper(module: &str) -> bool {
    // Match the conventional suffix patterns used in Phoenix projects.
    module.ends_with("ConnCase")
        || module.ends_with("ChannelCase")
        || module.ends_with("ControllerCase")
        || module.ends_with("ViewCase")
        || module.ends_with("LiveCase")
}

/// Check whether `name` is a symbol injected by a Phoenix test-case wrapper.
///
/// Called when the file has `use *ConnCase` (or similar), meaning Phoenix.ConnTest
/// and Plug.Conn helpers are available, plus the `Routes` alias to Router.Helpers.
pub(super) fn is_conn_case_injected(name: &str) -> bool {
    // All ConnTest helpers
    matches!(
        name,
        "html_response" | "json_response" | "text_response" | "response"
        | "redirected_to" | "get" | "post" | "put" | "patch" | "delete"
        | "recycle" | "build_conn" | "assert_error_sent"
        | "dispatch" | "bypass_through"
        | "put_req_header" | "get_flash" | "put_flash"
        | "conn" | "resp_body"
        // Plug.Conn helpers typically imported in ConnCase
        | "send_resp" | "put_resp_header" | "assign" | "fetch_session"
        | "get_session" | "put_session" | "get_resp_header"
        // `Routes` — injected as `alias MyAppWeb.Router.Helpers, as: Routes`
        // in almost every Phoenix ConnCase; it resolves to a compile-time
        // Helpers module we synthesise in phoenix_routes.rs.
        | "Routes"
    )
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

