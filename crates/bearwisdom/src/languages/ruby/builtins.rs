// =============================================================================
// ruby/builtins.rs — Ruby builtin and helper predicates
// =============================================================================

use crate::indexer::project_context::ProjectContext;
use crate::types::EdgeKind;

/// Check that the edge kind is compatible with the symbol kind.
pub(super) fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::Calls => matches!(
            sym_kind,
            "method" | "function" | "constructor" | "test" | "property"
        ),
        EdgeKind::Inherits => matches!(sym_kind, "class"),
        // Ruby modules (mixins) are stored as "namespace" in the index.
        EdgeKind::Implements => matches!(sym_kind, "namespace" | "interface"),
        EdgeKind::TypeRef => matches!(
            sym_kind,
            "class" | "namespace" | "interface" | "enum" | "type_alias"
        ),
        EdgeKind::Instantiates => matches!(sym_kind, "class"),
        _ => true,
    }
}

/// Ruby stdlib module names — always external regardless of Gemfile.
const RUBY_STDLIB: &[&str] = &[
    "json",
    "net/http",
    "uri",
    "fileutils",
    "set",
    "csv",
    "yaml",
    "erb",
    "cgi",
    "digest",
    "base64",
    "open-uri",
    "socket",
    "logger",
    "optparse",
    "benchmark",
    "tempfile",
    "pathname",
    "date",
    "time",
    "pp",
    "forwardable",
    "singleton",
    "ostruct",
    "struct",
];

/// Check whether a require path refers to an external gem or stdlib.
pub(super) fn is_external_ruby_require(
    require_path: &str,
    project_ctx: Option<&ProjectContext>,
) -> bool {
    // Stdlib — always external.
    if RUBY_STDLIB.contains(&require_path) {
        return true;
    }
    // Strip ruby_gems stored in project_ctx.
    // Since ProjectContext uses external_prefixes for .NET/Java namespaces,
    // Ruby gem names are stored in ruby_gems on the context.
    // Use the generic external_prefixes check if gems were stored there.
    if let Some(ctx) = project_ctx {
        // The Ruby resolver stores gem names as external_prefixes entries
        // (root name, e.g., "rails", "devise").
        let gem_root = require_path.split('/').next().unwrap_or(require_path);
        if ctx.external_prefixes.contains(gem_root) {
            return true;
        }
    }
    false
}

/// Ruby built-in methods and kernel functions always in scope.
///
/// Covers Object, Enumerable, Array, String, Hash built-ins, Rails/ActiveSupport
/// convenience methods, and Kernel functions. Used in `infer_external_namespace`
/// to classify unresolved calls as `ruby_core` rather than leaving them unknown.
pub(super) fn is_ruby_builtin(name: &str) -> bool {
    let root = name.split('.').next().unwrap_or(name);
    matches!(
        root,
        // Kernel / top-level functions
        "puts"
            | "print"
            | "p"
            | "pp"
            | "raise"
            | "require"
            | "require_relative"
            | "sleep"
            | "rand"
            | "exit"
            | "abort"
            | "lambda"
            | "proc"
            | "block_given?"
            | "yield"
            // Class-definition helpers (always available at class scope)
            | "include"
            | "extend"
            | "attr_reader"
            | "attr_writer"
            | "attr_accessor"
            | "define_method"
            // Object methods (on every Ruby object)
            | "nil?"
            | "is_a?"
            | "respond_to?"
            | "send"
            | "class"
            | "freeze"
            | "frozen?"
            | "dup"
            | "clone"
            | "to_s"
            | "to_i"
            | "to_f"
            | "to_a"
            | "to_h"
            | "inspect"
            | "hash"
            | "equal?"
            // Enumerable methods (mixed into Array, Hash, Range, etc.)
            | "each"
            | "map"
            | "select"
            | "reject"
            | "find"
            | "detect"
            | "collect"
            | "reduce"
            | "inject"
            | "any?"
            | "all?"
            | "none?"
            | "count"
            | "min"
            | "max"
            | "sort"
            | "sort_by"
            | "group_by"
            | "flat_map"
            | "zip"
            | "first"
            | "last"
            | "take"
            | "drop"
            | "each_with_object"
            | "each_with_index"
            // Array methods
            | "push"
            | "pop"
            | "shift"
            | "unshift"
            | "flatten"
            | "compact"
            | "uniq"
            | "reverse"
            | "join"
            | "length"
            | "size"
            | "empty?"
            | "include?"
            | "index"
            | "sample"
            | "shuffle"
            // String methods
            | "strip"
            | "chomp"
            | "chop"
            | "gsub"
            | "sub"
            | "split"
            | "upcase"
            | "downcase"
            | "capitalize"
            | "start_with?"
            | "end_with?"
            | "match?"
            | "scan"
            | "encode"
            | "bytes"
            | "chars"
            | "lines"
            // Hash methods
            | "keys"
            | "values"
            | "merge"
            | "merge!"
            | "fetch"
            | "delete"
            | "has_key?"
            | "has_value?"
            | "each_pair"
            | "transform_keys"
            | "transform_values"
            | "slice"
            | "except"
            // Rails/ActiveSupport convenience methods
            | "present?"
            | "blank?"
            | "presence"
            | "try"
            | "in?"
            // Top-level constants always available
            | "Array"
            | "Integer"
            | "Float"
            | "String"
            | "Hash"
            | "Kernel"
            | "Object"
            | "BasicObject"
            | "Module"
            | "Class"
            | "Comparable"
            | "Enumerable"
            | "Enumerator"
            | "nil"
            | "true"
            | "false"
            | "self"
            // Core Ruby classes
            | "NilClass"
            | "TrueClass"
            | "FalseClass"
            | "Numeric"
            | "Symbol"
            | "Range"
            | "Regexp"
            | "Proc"
            | "Method"
            | "UnboundMethod"
            | "IO"
            | "File"
            | "Dir"
            | "Exception"
            | "StandardError"
            | "RuntimeError"
            | "TypeError"
            | "ArgumentError"
            | "NameError"
            | "NoMethodError"
            | "IndexError"
            | "KeyError"
            | "StopIteration"
            | "NotImplementedError"
            | "SystemExit"
            | "SignalException"
            | "Interrupt"
            | "ScriptError"
            | "LoadError"
            | "SyntaxError"
            | "Math"
            | "Encoding"
            | "Fiber"
            | "Thread"
            | "GC"
            | "ObjectSpace"
            | "Struct"
            | "Data"
            | "Complex"
            | "Rational"
            | "STDOUT"
            | "STDERR"
            | "STDIN"
            | "ARGV"
            | "ENV"
            | "RUBY_VERSION"
            | "RUBY_PLATFORM"
            // Rails / ActiveRecord / ActiveSupport framework constants
            | "ActiveRecord"
            | "ActiveSupport"
            | "ActionController"
            | "ActionView"
            | "ActionMailer"
            | "ActionCable"
            | "ActiveJob"
            | "ActiveStorage"
            | "ActiveModel"
            | "ApplicationRecord"
            | "ApplicationController"
            | "ApplicationHelper"
            | "ApplicationMailer"
            | "ApplicationJob"
            | "Base"
            | "Migration"
            | "Schema"
            | "Concern"
            | "Railtie"
            | "Engine"
            | "Record"
            | "Connection"
            | "Logger"
            | "Middleware"
            | "Rack"
            | "Bundler"
            | "Gemfile"
            | "Rails"
            | "Minitest"
            | "RSpec"
    )
}
