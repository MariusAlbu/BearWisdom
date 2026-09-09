# Contextual callback identity — working design

1. Keep the implementation in the core Rust engine and shared syntax ingestion.
2. Capture TS/JS callback parameter declaration spans, not semantic name keys.
3. Retain the existing name-only argument form for unmigrated language extractors.
4. Index exact declaration spans to BindingIds in the file-owned lexical graph.
5. Contextual writes take the BindingId and retain the canonical TypeId.
6. Install callback facts in the callback's execution scope, not the caller's.
7. Preserve explicit annotations and isolate same-named sibling/nested callbacks.
8. Feed resolved bare-call callee IDs through the existing generic argument machinery.
9. Check independent compiler labels before changing production behavior.
10. Verify member, bare, generic and mid-chain calls without widening name fallbacks.

This is part of F1/F2; it does not complete overload selection or CFG inference.

## Implemented evidence

The TS/JS extractor emits `CallArg::LambdaAt` with optional parameter spans,
without semantic name keys. Lexical ingestion maps each complete span to one
BindingId. FileLookup writes contextual TypeIds to that binding independently
of the caller's cursor. Explicit annotations win; conflicting contexts become
Unknown. Cache reset clears contextual inference.

The existing `Lambda` variant remains for unmigrated language extractors, but
its seeder no longer writes a formatted type string after the TypeId (that write
evicted the canonical ID). Bare calls now use the resolved callee ID with the
same argument/context machinery as member calls, even if no return type exists.

Compiler-checked fixtures cover bare, generic-member, nested and mid-chain
callbacks, annotation precedence, and repeated callee spelling. The latter
revealed both an oracle anchor collision and a real pipeline duplicate loss.
Called-identifier anchors now distinguish those occurrences; the original v1
oracle snapshot remains unchanged. See [resolution-oracle.md](resolution-oracle.md).

Remaining: lexical bare-call target selection, precise extracted declaration
IDs, indirect/contextual function values, destructuring/rest projection, sound
overload selection, CFG joins and real-project semantic benchmarking.
