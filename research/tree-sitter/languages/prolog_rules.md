# Prolog Extraction Rules

## Classification

- **Type**: logic programming language
- **Paradigms**: logic programming, pattern matching, unification, constraint solving
- **Extractor**: clause-aware line scanner (no tree-sitter grammar)
- **Dialect coverage**: ISO Prolog, SWI-Prolog (module system, `use_module`, `ensure_loaded`, PlUnit)

The extractor is a dedicated line scanner, not a tree-sitter grammar. It accumulates source lines
until it sees a clause-terminating `.` and then dispatches on whether the clause is a directive,
rule, or fact.

---

## Symbol Extraction

### Predicates — `SymbolKind::Function`

Both facts and rules are extracted as `Function` symbols. The name is always in **functor/arity**
notation.

**Facts** — a clause with no `:-` neck:

```prolog
animal(dog).          % → Function "animal/1"
animal(cat).          % → Function "animal/1"  (duplicate clauses all recorded)
connected.            % → Function "connected/0"  (zero-arity fact)
```

**Rules** — a clause with a `:-` neck:

```prolog
foo(X) :- bar(X).              % → Function "foo/1"
grandparent(X, Z) :-
    parent(X, Y),
    parent(Y, Z).              % → Function "grandparent/2"  (multi-line clause)
```

Arity is computed by counting top-level comma-separated arguments at depth 0 inside the opening
paren of the head. Nested parens and brackets do not advance the arity counter.

**Symbol fields**:
- `name` = `qualified_name` = `"functor/arity"` string
- `kind` = `Function`
- `visibility` = `Public` (all predicates)
- `signature` = the raw head text (e.g. `"grandparent(X, Z)"`)
- `start_line` / `end_line` = line where the clause starts
- `scope_path`, `doc_comment`, `parent_index` = `None`

Variables (tokens starting with uppercase or `_`) are rejected as functor names. Pure operator
heads (starting with non-alphanumeric, non-underscore, non-quote) are also rejected.

---

### Modules — `SymbolKind::Namespace`

Directives of the form `:- module(Name, Exports).` emit a `Namespace` symbol for the module name:

```prolog
:- module(mymod, [pred/1, pred/2]).   % → Namespace "mymod"
```

Only the first argument is captured; the export list is ignored.

---

## Edge Extraction

### Predicate calls — `EdgeKind::Calls`

Goals in rule bodies generate `Calls` edges. The body is split at top-level `,` and `;`
(conjunction and disjunction), depth-tracking parens and brackets to avoid splitting inside nested
goals. Each resulting token is parsed for its functor name.

```prolog
parent(X, Y) :- mother(X, Y).
%                ^^^^^^ → Calls "mother"

grandparent(X, Z) :- parent(X, Y), parent(Y, Z).
%                    ^^^^^^            ^^^^^^   → two Calls "parent" edges
```

Filtering applied before emitting a `Calls` edge:
- **Variable goals** — functor starts with uppercase or `_`: skipped (meta-call via variable)
- **Builtins** — see builtin filter below: skipped
- **Empty tokens**: skipped

`source_symbol_index` points to the most recently emitted `Function` symbol (the predicate whose
body is being processed). `module` and `chain` are `None`.

---

### Module imports — `EdgeKind::Imports`

Two directive forms produce `Imports` edges:

**`use_module`**:
```prolog
:- use_module(library(lists)).     % → Imports, target_name = "lists", module = "lists"
:- use_module(library(apply)).     % → Imports, target_name = "apply"
:- use_module('lib/utils').        % → Imports, target_name = "lib/utils"
:- use_module("helpers").          % → Imports, target_name = "helpers"
```

**`ensure_loaded`**:
```prolog
:- ensure_loaded(library(lists)).  % → Imports, target_name = "lists"
:- ensure_loaded('utils/helpers'). % → Imports, target_name = "utils/helpers"
```

For both directives: if the argument is `library(X)`, the target is the inner atom `X`. Otherwise
the argument is used as-is after stripping surrounding single or double quotes.

`source_symbol_index` is the index of the most recently emitted symbol at the time the directive
is processed. `chain` is `None`.

---

## Comment and Whitespace Handling

- **Line comments** (`%`): stripped before processing. The stripper checks that `%` is not inside
  a quoted atom (odd quote count before `%` → inside atom, not a comment).
- **Block comments** (`/* ... */`): tracked with an `in_block_comment` flag across source lines.
  Inline block comments (open and close on same line) are stripped in place. Multi-line block
  comments skip all intermediate lines until `*/` is found.
- **Multi-line clauses**: the scanner accumulates partial lines into a buffer until it finds a
  trailing `.`. The `.` detector is a simple `trimmed.ends_with('.')` check (no quote-escape
  awareness for the terminator itself).

---

## Builtin Filter

The following predicates are **not** emitted as `Calls` edges (suppressed in both the inline
`is_prolog_builtin` in `extract.rs` and the richer list in `builtins.rs`):

**Control**: `true`, `false`, `fail`, `halt`

**I/O**: `write`, `writeln`, `writef`, `read`, `nl`, `tab`, `read_term`, `write_term`, `format`

**Type checks**: `atom`, `number`, `integer`, `float`, `compound`, `is_list`, `var`, `nonvar`,
`atomic`, `callable`, `ground`, `string`

**Atom/string manipulation**: `atom_chars`, `atom_length`, `atom_concat`, `atom_string`,
`atom_codes`, `atom_to_term`, `atom_to_number`, `number_chars`, `number_codes`, `char_code`,
`sub_atom`, `term_to_atom`, `term_string`, `string_concat`, `string_codes`, `string_chars`,
`split_string`, `string_to_atom`, `format_atom`, `atomic_list_concat`, `concat_atom`

**Term inspection**: `functor`, `arg`, `copy_term`, `numbervars`, `number_vars`

**Database**: `assert`, `retract`, `asserta`, `assertz`, `retractall`, `abolish`

**Aggregation/search**: `findall`, `bagof`, `setof`, `aggregate_all`, `forall`, `between`,
`once`, `ignore`, `call`

**Arithmetic**: `succ`, `plus`, `is`, `mod`, `rem`, `abs`, `sign`, `min`, `max`, `truncate`,
`round`, `ceiling`, `floor`, `sqrt`, `sin`, `cos`, `tan`, `exp`, `log`, `random`, `succ_or_zero`,
`max_list`, `min_list`, `sum_list`, `sumlist`

**List predicates**: `length`, `append`, `member`, `memberchk`, `nth0`, `nth1`, `last`, `msort`,
`sort`, `predsort`, `permutation`, `flatten`, `subtract`, `intersection`, `union`, `select`,
`selectchk`, `maplist`, `include`, `exclude`, `foldl`, `numlist`

**Operators used as goals**: `!`, `->`, `\+`, `not`, `=`, `\=`, `==`, `\==`, `<`, `>`, `>=`,
`=<`, `=:=`, `=\=`

---

## Not Extracted

The following Prolog constructs are **not** currently extracted:

| Construct | Example | Reason |
|---|---|---|
| DCG rules | `s --> np, vp.` | `-->` neck not handled by `find_neck` |
| Module-qualified calls | `lists:member(X, L)` | `:` operator not parsed |
| `meta_predicate` declarations | `:- meta_predicate foo(+, :).` | Not a handled directive |
| `use_module/2` (selective import) | `:- use_module(library(lists), [member/2]).` | Arg parsing stops after first arg |
| Dynamic/discontiguous declarations | `:- dynamic foo/2.` | No symbol or edge emitted |
| `load_files/1` | `:- load_files('file').` | Not a handled directive |
| PlUnit test blocks | `:- begin_tests(suite).` | Not a handled directive |
| Operator definitions | `:- op(700, xfx, ===).` | Not a handled directive |
| Quoted atom functors | `'My Pred'(X) :- …` | Single-quoted functors are stripped of quotes but otherwise handled normally |
| Export list in `module/2` | `[pred/1, pred/2]` | Export list is ignored; only module name captured |

---

## Test Detection

PlUnit is not currently detected. There is no test-suite-aware extraction — no
`begin_tests`/`end_tests` block handling and no `test(Name)` predicate special-casing. Test
predicates defined within a PlUnit block would be extracted as ordinary `Function` symbols if they
match the fact/rule pattern, but `begin_tests` and `end_tests` directives are silently dropped.

---

## Extractor Location

`crates/bearwisdom/src/languages/prolog/extract.rs` — 461 lines, clause-aware line scanner.
`crates/bearwisdom/src/languages/prolog/builtins.rs` — `is_prolog_builtin` and `kind_compatible`.
`crates/bearwisdom/src/languages/prolog/coverage_tests.rs` — 8 coverage tests.
