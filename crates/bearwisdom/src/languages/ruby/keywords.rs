// =============================================================================
// ruby/keywords.rs — Ruby primitive types and language-intrinsic names
// =============================================================================

/// Language-intrinsic Ruby names: core types, Kernel methods, and method names
/// that aren't reliably indexed as stdlib symbols by tree-sitter parses of
/// the Ruby corpus.
///
/// Per `indexer/keywords.rs`: "Runtime globals and stdlib identifiers come
/// from indexed stdlib ecosystems registered in `EcosystemRegistry`". Rails /
/// ActiveRecord / RSpec / Capybara DSL names DO NOT belong here — they are
/// gem-provided third-party symbols indexed by `ecosystem/rubygems.rs` when
/// the project's Gemfile declares them.
pub(crate) const KEYWORDS: &[&str] = &[
    // High-ambiguity Ruby core types — kept as short-circuits
    "Array", "String", "Object", "Hash", "Kernel", "Module", "Date", "Time",
    "DateTime",
    // Types not reliably indexed (borderline or C-implemented)
    "BasicObject", "OpenStruct", "BigDecimal",
    "Thread", "Mutex", "Fiber", "Enumerator", "SortedSet", "SizedQueue",
    "Ractor", "StopIteration",
    // Exceptions — the root names aren't always indexed as classes
    "RuntimeError", "TypeError", "NoMethodError", "IOError", "KeyError",
    "IndexError", "Errno", "SystemCallError", "RegexpError",
    "SecurityError", "ScriptError", "SignalException", "Interrupt",
    "SystemExit", "ZeroDivisionError", "StandardError",
    "ConcurrentModificationException",
    // Ruby builtins (methods/keywords not reliably indexed as stdlib symbols)
    "puts", "print", "p", "pp", "warn", "raise", "fail",
    "require", "require_relative", "load", "autoload",
    "attr_accessor", "attr_reader", "attr_writer",
    "include", "extend", "prepend", "using",
    "public", "private", "protected",
    "define_method", "method_missing", "respond_to_missing?",
    "send", "public_send", "instance_variable_get", "instance_variable_set",
    "class_eval", "module_eval", "instance_eval", "instance_exec",
    "freeze", "frozen?", "dup", "clone", "taint", "untaint",
    "nil?", "is_a?", "kind_of?", "instance_of?", "respond_to?",
    "equal?", "eql?", "hash", "inspect", "to_s", "to_i", "to_f", "to_a", "to_h",
    "map", "select", "reject", "find", "detect", "collect", "each",
    "each_with_index", "each_with_object", "inject", "reduce",
    "flat_map", "compact", "compact_map", "zip", "sort_by", "group_by",
    "min_by", "max_by", "count", "sum", "any?", "all?", "none?", "empty?",
    "first", "last", "take", "drop", "reverse", "uniq", "flatten",
    "push", "pop", "shift", "unshift", "append", "delete",
    "include?", "index", "rindex", "length", "size",
    "split", "join", "strip", "lstrip", "rstrip", "chomp", "chop",
    "upcase", "downcase", "capitalize", "swapcase", "gsub", "gsub!", "sub", "sub!",
    "match", "match?", "scan", "tr", "squeeze", "replace", "encode", "force_encoding",
    "start_with?", "end_with?",
    "to_sym", "to_str", "to_proc", "to_json", "to_yaml",
    "tap", "then", "yield_self",
    "open", "read", "write", "close", "each_line", "readlines",
    "sleep", "exit", "abort", "at_exit", "trap",
    "lambda", "proc", "block_given?", "yield",
    // Generic type parameters
    "T", "U", "K", "V",
];
