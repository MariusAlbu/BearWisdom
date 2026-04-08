/// clojure.core functions, special forms, and commonly-used macros that are
/// always external to any project.  These are emitted as `sym_name` call refs
/// by the extractor but will never resolve to a project-defined symbol, so the
/// resolution engine should classify them as external rather than unresolved.
pub(crate) const EXTERNALS: &[&str] = &[
    // -----------------------------------------------------------------------
    // Special forms (compiler built-ins, not functions)
    // -----------------------------------------------------------------------
    "def", "do", "if", "let", "fn", "loop", "recur", "quote", "var",
    "defn", "defmacro", "defn-",
    "new", ".", "set!", "throw", "try", "catch", "finally",
    "monitor-enter", "monitor-exit",
    // -----------------------------------------------------------------------
    // Core definition macros
    // -----------------------------------------------------------------------
    "defmulti", "defmethod", "defprotocol", "defrecord", "deftype",
    "defstruct", "defonce", "declare",
    // -----------------------------------------------------------------------
    // Flow control macros
    // -----------------------------------------------------------------------
    "when", "when-not", "when-let", "when-first", "when-some",
    "if-let", "if-not", "if-some",
    "cond", "condp", "cond->", "cond->>", "case",
    "and", "or", "not",
    "do", "doto",
    "->", "->>", "as->", "some->", "some->>",
    // -----------------------------------------------------------------------
    // Binding / local forms
    // -----------------------------------------------------------------------
    "let*", "letfn", "binding", "with-bindings", "with-local-vars",
    "with-redefs", "with-redefs-fn",
    // -----------------------------------------------------------------------
    // I/O
    // -----------------------------------------------------------------------
    "println", "print", "prn", "pr", "newline",
    "pr-str", "prn-str", "print-str", "println-str",
    "read", "read-string", "read-line",
    "slurp", "spit",
    "with-open", "with-in-str", "with-out-str",
    "pprint",
    // -----------------------------------------------------------------------
    // String
    // -----------------------------------------------------------------------
    "str", "format", "subs",
    "name", "namespace", "keyword", "symbol", "gensym",
    // -----------------------------------------------------------------------
    // Type / identity predicates
    // -----------------------------------------------------------------------
    "nil?", "true?", "false?", "some?",
    "empty?", "seq?", "map?", "vector?", "set?", "list?",
    "string?", "number?", "integer?", "float?", "ratio?",
    "keyword?", "symbol?", "fn?", "var?",
    "ifn?", "coll?", "associative?", "sequential?", "counted?",
    "reversible?", "sorted?", "chunked-seq?", "delay?",
    "future?", "realized?", "reduced?",
    "instance?", "identical?", "class", "type",
    // -----------------------------------------------------------------------
    // Equality / comparison
    // -----------------------------------------------------------------------
    "=", "not=", "==",
    "<", ">", "<=", ">=",
    "compare", "max", "min",
    // -----------------------------------------------------------------------
    // Arithmetic
    // -----------------------------------------------------------------------
    "+", "-", "*", "/", "quot", "rem", "mod",
    "inc", "dec", "abs", "neg?", "pos?", "zero?",
    "even?", "odd?",
    "bit-and", "bit-or", "bit-xor", "bit-not", "bit-shift-left", "bit-shift-right",
    "unsigned-bit-shift-right",
    "rand", "rand-int",
    // -----------------------------------------------------------------------
    // Collection construction
    // -----------------------------------------------------------------------
    "list", "vector", "hash-map", "sorted-map", "sorted-map-by",
    "array-map", "hash-set", "sorted-set", "sorted-set-by",
    "vec", "set",
    // -----------------------------------------------------------------------
    // Collection core
    // -----------------------------------------------------------------------
    "count", "first", "second", "rest", "next", "last", "butlast", "ffirst",
    "fnext", "nfirst", "nnext", "nthnext", "nthrest",
    "cons", "conj", "concat", "into", "empty",
    "nth", "peek", "pop", "get", "get-in",
    "assoc", "assoc-in", "dissoc", "update", "update-in",
    "select-keys", "keys", "vals", "find", "contains?",
    "merge", "merge-with", "zipmap",
    "seq", "rseq", "subseq", "rsubseq",
    // -----------------------------------------------------------------------
    // Higher-order / functional
    // -----------------------------------------------------------------------
    "map", "mapv", "map-indexed", "mapcat", "keep", "keep-indexed",
    "filter", "filterv", "remove",
    "reduce", "reduce-kv", "reductions",
    "apply", "partial", "comp", "complement", "identity", "constantly",
    "juxt", "fnil",
    "some", "every?", "not-every?", "not-any?",
    "any?",
    // -----------------------------------------------------------------------
    // Sequence operations
    // -----------------------------------------------------------------------
    "take", "take-last", "take-nth", "take-while",
    "drop", "drop-last", "drop-while",
    "split-at", "split-with",
    "partition", "partition-all", "partition-by",
    "group-by", "sort", "sort-by", "reverse",
    "flatten", "distinct", "dedupe",
    "interleave", "interpose", "intersperse",
    "frequencies", "tally",
    "range", "repeat", "repeatedly", "iterate", "cycle",
    "lazy-seq", "lazy-cat", "chunk", "chunk-cons",
    "doall", "dorun", "doseq", "for",
    "run!",
    // -----------------------------------------------------------------------
    // Transducers
    // -----------------------------------------------------------------------
    "transduce", "eduction", "sequence", "completing", "xform",
    // -----------------------------------------------------------------------
    // Metadata
    // -----------------------------------------------------------------------
    "meta", "with-meta", "vary-meta", "alter-meta!", "reset-meta!",
    // -----------------------------------------------------------------------
    // Concurrency primitives
    // -----------------------------------------------------------------------
    "atom", "deref", "@",
    "reset!", "swap!", "swap-vals!", "compare-and-set!",
    "ref", "ref-set", "dosync", "alter", "commute", "ensure",
    "agent", "agent-errors", "clear-agent-errors", "set-agent-send-executor!",
    "send", "send-off", "send-via", "await", "await-for", "shutdown-agents",
    "promise", "deliver",
    "future", "future-call", "future-cancel", "future-cancelled?", "future-done?",
    "delay", "force",
    "pmap", "pcalls", "pvalues",
    // -----------------------------------------------------------------------
    // Exceptions / error handling
    // -----------------------------------------------------------------------
    "ex-info", "ex-data", "ex-message", "ex-cause",
    "error-handler", "set-error-handler!", "set-error-mode!",
    // -----------------------------------------------------------------------
    // Namespaces / vars
    // -----------------------------------------------------------------------
    "ns", "in-ns", "ns-name", "ns-map", "ns-publics", "ns-interns",
    "ns-refers", "ns-imports", "ns-aliases", "ns-resolve",
    "resolve", "find-var", "intern", "require", "use", "import",
    "refer", "refer-clojure",
    // -----------------------------------------------------------------------
    // Regex
    // -----------------------------------------------------------------------
    "re-find", "re-matches", "re-seq", "re-pattern", "re-groups", "re-matcher",
    // -----------------------------------------------------------------------
    // Reflection / evaluation
    // -----------------------------------------------------------------------
    "eval", "load", "load-file", "load-reader", "load-string",
    "macroexpand", "macroexpand-1",
    "memoize", "trampoline",
    // -----------------------------------------------------------------------
    // Multimethods
    // -----------------------------------------------------------------------
    "dispatch-fn", "get-method", "methods", "prefer-method",
    "prefers", "remove-method",
    // -----------------------------------------------------------------------
    // Java interop helpers
    // -----------------------------------------------------------------------
    "bean", "bases", "supers", "ancestors", "descendants",
    "cast",
    // -----------------------------------------------------------------------
    // Common single-letter / underscore locals the extractor emits as refs
    // -----------------------------------------------------------------------
    "_", "k", "v", "m", "n", "x", "e", "f", "s",
    // -----------------------------------------------------------------------
    // clojure.test macros (very commonly used at top level)
    // -----------------------------------------------------------------------
    "deftest", "is", "are", "testing", "use-fixtures",
    "run-tests", "run-all-tests", "test-vars",
    // -----------------------------------------------------------------------
    // clojure.spec.alpha
    // -----------------------------------------------------------------------
    "s/def", "s/fdef", "s/valid?", "s/conform", "s/explain",
    "s/explain-data", "s/assert",
];
