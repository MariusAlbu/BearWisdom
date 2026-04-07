// =============================================================================
// clojure/primitives.rs — Clojure primitive and built-in types
// =============================================================================

/// Primitive and built-in type/function names for Clojure.
pub(crate) const PRIMITIVES: &[&str] = &[
    // special forms / core macros
    "def", "defn", "defn-", "defmacro", "defmulti", "defmethod",
    "defprotocol", "defrecord", "deftype", "defstruct", "defonce",
    "fn", "let", "do", "if", "when", "when-not", "when-let",
    "when-first", "when-some", "if-let", "if-not", "if-some",
    "cond", "condp", "case", "loop", "recur",
    "for", "doseq", "dotimes", "while",
    "try", "catch", "finally", "throw", "assert", "comment",
    "quote", "unquote", "deref",
    // atoms / refs / agents
    "swap!", "reset!", "atom", "ref", "agent", "send", "send-off",
    "dosync", "alter", "commute", "ensure",
    "future", "promise", "deliver", "realized?",
    // namespaces
    "ns", "require", "use", "import", "refer", "in-ns",
    // comparison / logic
    "=", "not=", "==", "identical?", "nil?", "true?", "false?", "some?",
    "not", "and", "or",
    // higher-order / functional
    "comp", "complement", "partial", "juxt", "apply",
    "->", "->>", "as->", "cond->", "cond->>", "some->", "some->>",
    "identity", "constantly", "fnil", "memoize", "trampoline",
    // arithmetic
    "+", "-", "*", "/", "<", ">", "<=", ">=",
    "inc", "dec", "max", "min", "rem", "quot", "mod", "abs",
    "rand", "rand-int", "rand-nth",
    // strings
    "str", "subs", "format", "name", "keyword", "symbol", "namespace",
    "pr", "prn", "print", "println", "pr-str", "prn-str", "print-str",
    "println-str", "with-out-str", "read-string",
    // I/O
    "slurp", "spit",
    // sequence operations
    "count", "empty?", "not-empty", "seq",
    "first", "second", "last", "rest", "next",
    "nth", "get", "get-in", "assoc", "assoc-in", "dissoc",
    "update", "update-in", "select-keys", "find",
    "keys", "vals", "merge", "merge-with", "into",
    "conj", "cons", "concat", "flatten", "distinct", "dedupe",
    "sort", "sort-by", "reverse", "shuffle",
    "interleave", "interpose", "partition", "partition-all", "partition-by",
    "group-by", "frequencies",
    "map", "mapv", "mapcat", "map-indexed",
    "filter", "filterv", "remove", "keep", "keep-indexed",
    "reduce", "reduce-kv", "reductions",
    "take", "take-while", "take-nth", "take-last",
    "drop", "drop-while", "drop-last",
    "split-at", "split-with",
    "every?", "some", "not-every?", "not-any?",
    "empty", "range", "repeat", "repeatedly", "iterate", "cycle",
    "lazy-seq", "lazy-cat", "doall", "dorun",
    // collection constructors
    "vec", "vector", "vector-of", "subvec",
    "list", "list*",
    "set", "hash-set", "sorted-set", "sorted-set-by",
    "hash-map", "sorted-map", "sorted-map-by", "zipmap",
    "bean", "contains?",
    // type predicates
    "boolean", "number?", "integer?", "float?", "string?", "keyword?",
    "symbol?", "map?", "vector?", "set?", "list?", "seq?", "coll?",
    "fn?", "ifn?", "associative?", "sequential?", "counted?",
    "reversible?", "sorted?",
    "class", "type", "instance?", "isa?", "cast",
    // numeric coercions
    "num", "int", "long", "float", "double", "short", "byte", "char",
    "bigint", "bigdec", "rationalize",
    // bitwise
    "bit-and", "bit-or", "bit-xor", "bit-not", "bit-shift-left", "bit-shift-right",
    // var / binding
    "bound?", "resolve", "var", "var?", "binding", "with-bindings",
    "alter-var-root", "set!",
    // namespace introspection
    "the-ns", "all-ns", "find-ns", "ns-name", "ns-publics", "ns-imports",
    "ns-interns", "ns-refers", "ns-map", "ns-resolve",
    // metadata
    "meta", "with-meta", "vary-meta", "alter-meta!", "reset-meta!",
    "tagged-literal",
    // regex
    "re-find", "re-matches", "re-seq", "re-pattern",
    // clojure.string namespace
    "clojure.string/join", "clojure.string/split", "clojure.string/replace",
    "clojure.string/trim", "clojure.string/lower-case", "clojure.string/upper-case",
    "clojure.string/blank?", "clojure.string/includes?",
    "clojure.string/starts-with?", "clojure.string/ends-with?",
    // anonymous fn shorthand
    "%", "%1", "%2", "%3", "%&",
];
