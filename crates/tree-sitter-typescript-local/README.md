# Pinned TypeScript provider grammar

This crate extends `tree-sitter-typescript` 0.23.2, using its pinned JavaScript
grammar 0.23.1 and tree-sitter CLI 0.25.10 (ABI 14). The extension is in
`common/extend-grammar.js`; no user source text is rewritten.

It repairs bare `global {}` augmentation syntax, `keyof readonly` type operands,
generic import-qualified types, and the predefined `bigint` type keyword. Existing named nodes and Rust constants are
preserved. The shared workspace dependency routes indexing and highlighting to
these same parsers. Parsing a construct does not establish declaration legality,
module visibility, type compatibility or successful semantic binding.

## Regeneration

From this directory, with Node and npm installed:

```sh
npm ci --ignore-scripts --no-audit --no-fund
npm rebuild tree-sitter-cli
npm run generate
```

Use an explicit writable `--cache` directory for npm in restricted environments.
The rebuild downloads only the pinned CLI; native Node grammar bindings are not
needed. `generate.mjs` checks dependency versions, generates both parsers, and
copies the pinned upstream scanners, queries and license notices. Commit the
generated `typescript/src`, `tsx/src`, `common/scanner.h`, query and license files.
Regular Cargo builds need neither npm nor network access.

## Verification

```sh
cargo test --offline -p tree-sitter-typescript-local --lib
cargo test --offline -p bearwisdom --lib provider_grammar_
node verify-compiler-shapes.mjs <path-to-typescript-5.9.3/lib/typescript.js>
node verify-module-scopes.mjs <path-to-typescript-5.9.3/lib/typescript.js>
```

`lib_tests.rs` checks structural ownership, source anchors, existing queries,
both dialects, invalid syntax and incremental/fresh parsing. Production lexical
tests retain the incomplete-provider barrier for unsupported module binding.
The optional compiler verifier independently checks 14 syntax/AST ownership
cases. It is developer-only evidence, not a production compiler dependency or
a symbol-binding accuracy measurement.

The shared engine atomic-type oracle additionally checks 49 intrinsic/literal
values against TypeScript 5.9.3; its Rust tests exercise both dialects. The
`bigint` keyword must be a predefined type while same-spelled value bindings
remain identifiers and the distinct `BigInt` wrapper remains a named type.

`verify-module-scopes.mjs` checks the shared lexical scope fixtures against the
pinned compiler: seven sources, eleven module export sets and two source-addressed
variable targets. The Rust binder consumes the same export fixtures. This is
diagnostic semantic evidence, not representative recall or a production compiler
dependency; configured ambient-provider selection is still a separate engine task.

## Upstream provenance

- TypeScript grammar/scanner/query inputs: https://github.com/tree-sitter/tree-sitter-typescript/tree/v0.23.2
- JavaScript grammar input: https://github.com/tree-sitter/tree-sitter-javascript/tree/v0.23.1
- Generator: https://github.com/tree-sitter/tree-sitter/releases/tag/v0.25.10
- npm archive integrity and all transitive versions: `package-lock.json`.
- Upstream MIT notices: `LICENSE` and `LICENSE.javascript`.
