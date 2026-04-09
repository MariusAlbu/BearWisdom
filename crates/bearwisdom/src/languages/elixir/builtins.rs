// =============================================================================
// elixir/builtins.rs — Elixir builtin and helper predicates
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
