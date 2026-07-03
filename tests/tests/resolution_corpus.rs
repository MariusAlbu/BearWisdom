//! Resolution corpus — a deterministic, per-fix resolution harness.
//!
//! Each construct below exercises ONE resolution pattern end-to-end (parse →
//! extract → flow → resolve) against SEEDED stub externals: a stub TypeScript
//! `lib.es5.d.ts` (so `string`/`array` receivers box to `String`/`Array`) and a
//! stub `@base-ui/react` package (a same-name external to probe foreign-pick).
//!
//! Because the fixture is tiny, in-memory, and its externals are pinned, indexing
//! is fully deterministic — a fix's effect shows up as a single assertion flipping
//! rather than a fraction of a percent buried under the reindex non-determinism
//! that swamps whole-project rate comparisons. Add a pattern per fix: one source
//! construct + one assertion.

use std::fs;

use bearwisdom::full_index;
use bearwisdom::Database;
use bearwisdom_tests::TestProject;
use rusqlite::params;
use tempfile::TempDir;

/// Seed a stub TypeScript standard library (`lib.es5.d.ts`) carrying just the
/// prototype members the corpus references. `BEARWISDOM_TS_LIB_DIR` points the
/// `ts-lib-dom` locator at it, so `String`/`Array`/`Number` index as
/// `ext:ts:__ts_lib__/…` deterministically without a real TypeScript install.
fn seed_ts_lib() -> TempDir {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("lib.es5.d.ts"),
        r#"interface String {
    replace(searchValue: string, replaceValue: string): string;
    toLowerCase(): string;
    toUpperCase(): string;
    trim(): string;
    split(separator: string): string[];
}
interface Array<T> {
    map<U>(callbackfn: (value: T) => U): U[];
    filter(predicate: (value: T) => boolean): T[];
    find(predicate: (value: T) => boolean): T | undefined;
}
interface Number {
    toFixed(fractionDigits?: number): string;
}
interface Boolean {}
"#,
    )
    .unwrap();
    dir
}

/// Count resolved `calls` edges for the callee `callee` in the file ending
/// `file_suffix`, whose resolved target qname matches `target_like` (SQL LIKE).
fn count_resolved_to(db: &Database, file_suffix: &str, callee: &str, target_like: &str) -> i64 {
    db.query_row(
        "SELECT COUNT(*) FROM edges e
         JOIN symbols s ON e.source_id = s.id
         JOIN files f   ON s.file_id   = f.id
         JOIN symbols t ON e.target_id = t.id
         WHERE f.path LIKE ?1 AND e.kind = 'calls' AND t.name = ?2
           AND t.qualified_name LIKE ?3",
        params![format!("%{file_suffix}"), callee, target_like],
        |r| r.get(0),
    )
    .unwrap()
}

/// Count resolved `calls` edges for `callee` in the file ending `file_suffix`,
/// regardless of the target's qname (for "resolves at all" checks where the
/// target is an anonymous object-literal member with no stable qname).
fn count_resolved(db: &Database, file_suffix: &str, callee: &str) -> i64 {
    db.query_row(
        "SELECT COUNT(*) FROM edges e
         JOIN symbols s ON e.source_id = s.id
         JOIN files f   ON s.file_id   = f.id
         JOIN symbols t ON e.target_id = t.id
         WHERE f.path LIKE ?1 AND e.kind = 'calls' AND t.name = ?2",
        params![format!("%{file_suffix}"), callee],
        |r| r.get(0),
    )
    .unwrap()
}

/// Count resolved `calls` edges for `callee` in the file ending `file_suffix`
/// whose resolved target's SYMBOL KIND matches `kind` — distinguishes
/// "resolved to a real callable" from "resolved to the binding's own
/// declaration" for a candidate probe.
fn count_resolved_kind(db: &Database, file_suffix: &str, callee: &str, kind: &str) -> i64 {
    db.query_row(
        "SELECT COUNT(*) FROM edges e
         JOIN symbols s ON e.source_id = s.id
         JOIN files f   ON s.file_id   = f.id
         JOIN symbols t ON e.target_id = t.id
         WHERE f.path LIKE ?1 AND e.kind = 'calls' AND t.name = ?2 AND t.kind = ?3",
        params![format!("%{file_suffix}"), callee, kind],
        |r| r.get(0),
    )
    .unwrap()
}

/// Count unresolved `calls` refs for `callee` in the file ending `file_suffix`.
fn count_unresolved(db: &Database, file_suffix: &str, callee: &str) -> i64 {
    db.query_row(
        "SELECT COUNT(*) FROM unresolved_refs u
         JOIN symbols s ON u.source_id = s.id
         JOIN files f   ON s.file_id   = f.id
         WHERE f.path LIKE ?1 AND u.kind = 'calls' AND u.target_name = ?2",
        params![format!("%{file_suffix}"), callee],
        |r| r.get(0),
    )
    .unwrap()
}

#[test]
fn resolution_corpus() {
    let ts_lib = seed_ts_lib();

    let project = TestProject {
        dir: TempDir::new().unwrap(),
    };

    // tsconfig activates `ts-lib-dom` (lib contains DOM) and declares the `@/`
    // path alias so alias-scoped imports resolve.
    project.add_file(
        "tsconfig.json",
        r#"{
  "compilerOptions": {
    "lib": ["DOM", "ES2015"],
    "baseUrl": ".",
    "paths": { "@/*": ["src/*"] }
  }
}
"#,
    );
    project.add_file(
        "package.json",
        r#"{
  "name": "resolution-corpus",
  "version": "0.0.1",
  "dependencies": { "@base-ui/react": "^1.0.0", "vitest": "^1.0.0" }
}
"#,
    );

    // A same-name EXTERNAL: `@base-ui/react` also exports `toast`. It carries a
    // `dismiss` member — the foreign target the resolver must NOT borrow for an
    // internally-imported `toast`.
    project.add_file(
        "node_modules/@base-ui/react/package.json",
        r#"{"name":"@base-ui/react","version":"1.0.0","types":"index.d.ts"}"#,
    );
    project.add_file(
        "node_modules/@base-ui/react/index.d.ts",
        r#"export interface BaseToast {
    dismiss(): void;
}
export declare const toast: BaseToast;
"#,
    );

    // --- pattern: string-typed PARAM receiver → String member -----------------
    // `text: string` must seed the flow cache so `.replace`/`.toLowerCase` box to
    // the `String` prototype. Regresses to an unrelated same-name (Array) without
    // parameter flow-seeding.
    project.add_file(
        "src/string_param.ts",
        r#"export function kebab(text: string): string {
    return text.replace("a", "b").toLowerCase();
}
"#,
    );

    // --- pattern: local class PARAM receiver → its own method -----------------
    project.add_file(
        "src/local_param.ts",
        r#"class User {
    getName(): string {
        return "";
    }
}

export function greet(u: User): string {
    return u.getName();
}
"#,
    );

    // --- pattern: internally-imported name must not borrow an external same-name
    // The internal `toast` is a value whose `Object.assign(...)` type is not
    // captured; `toast.dismiss()` must NOT bind to `@base-ui/react`'s `dismiss`.
    project.add_file(
        "src/lib/toast.ts",
        r#"function message(): void {}

const toast = Object.assign(message, {
    dismiss(): void {},
});

export { toast };
"#,
    );
    project.add_file(
        "src/notify.ts",
        r#"import { toast } from "@/lib/toast";

export function notify(): void {
    toast.dismiss();
}
"#,
    );
    // A SEPARATE file imports `toast` from `@base-ui/react`, so the reachability
    // walker materializes the external `@base-ui/react.toast` — the same-name
    // competitor `notify.ts` must not borrow. Without this the foreign-pick check
    // is vacuous (no external `toast` is indexed at all).
    project.add_file(
        "src/widget.ts",
        r#"import { toast } from "@base-ui/react";

export function widget(): void {
    toast.dismiss();
}
"#,
    );

    // --- pattern: object-literal function return, member call on the local ----
    // A function that RETURNS an object literal; a consumer calls a member on the
    // inferred local. (createScopedLogger pattern.)
    project.add_file(
        "src/obj_return.ts",
        r#"function makeLogger() {
    return {
        info(msg: string): void {},
        error(msg: string): void {},
    };
}

export function useLogger(): void {
    const log = makeLogger();
    log.info("hi");
}
"#,
    );
    // --- pattern: `type X = ReturnType<typeof f>` where f returns an object literal
    project.add_file(
        "src/return_type.ts",
        r#"function make() {
    return {
        go(): void {},
    };
}

type Handle = ReturnType<typeof make>;

export function run(h: Handle): void {
    h.go();
}
"#,
    );
    // --- pattern: destructuring an object-literal-returning call's result, then
    // calling the destructured binding directly (`const { info } = f(); info(...)`).
    project.add_file(
        "src/obj_return_destructure.ts",
        r#"function makeLogger2() {
    return {
        info(msg: string): void {},
        error(msg: string): void {},
    };
}

export function useLoggerDestructured(): void {
    const { info } = makeLogger2();
    info("hi");
}
"#,
    );

    // --- pattern: a plain (non-destructured) const binding from a member-chain
    // call, then a member call on the binding (`const s = vi.spyOn(...); s.m()`).
    // A stub `vitest` external carries `spyOn`'s declared return type so the
    // binding's forward-inferred type is a real, resolvable interface.
    project.add_file(
        "node_modules/vitest/package.json",
        r#"{"name":"vitest","version":"1.0.0","types":"index.d.ts"}"#,
    );
    project.add_file(
        "node_modules/vitest/index.d.ts",
        r#"export interface SpyInstance {
    mockRestore(): void;
}
export interface VitestUtils {
    spyOn(obj: any, method: string): SpyInstance;
}
export declare const vi: VitestUtils;
"#,
    );
    project.add_file(
        "src/spy_const.ts",
        r#"import { vi } from "vitest";

export function useSpy(obj: { m(): void }): void {
    const s = vi.spyOn(obj, "m");
    s.mockRestore();
}
"#,
    );

    // --- candidate patterns (probed, not yet asserted) ------------------------
    // An optional-array param: `T[] | undefined` receiver, member via `?.`.
    project.add_file(
        "src/optional_array.ts",
        r#"export function firsts(items: string[] | undefined): string[] | undefined {
    return items?.map((x) => x.trim());
}
"#,
    );
    // Array-destructuring a tuple-returning call's result, then calling the
    // destructured binding directly (`const [count, reset] = f(); reset()`).
    project.add_file(
        "src/tuple_destructure.ts",
        r#"function makeCounter(): [number, () => void] {
    return [0, () => {}];
}

export function useCounter(): void {
    const [count, reset] = makeCounter();
    reset();
}
"#,
    );

    // Point the locators at the seeded stubs, index once, restore env.
    let prior_lib = std::env::var_os("BEARWISDOM_TS_LIB_DIR");
    let prior_nm = std::env::var_os("BEARWISDOM_TS_NODE_MODULES");
    unsafe {
        std::env::set_var("BEARWISDOM_TS_LIB_DIR", ts_lib.path());
        std::env::set_var(
            "BEARWISDOM_TS_NODE_MODULES",
            project.path().join("node_modules"),
        );
    }

    let mut db = TestProject::in_memory_db();
    let result = full_index(&mut db, project.path(), None, None, None);

    unsafe {
        match prior_lib {
            Some(v) => std::env::set_var("BEARWISDOM_TS_LIB_DIR", v),
            None => std::env::remove_var("BEARWISDOM_TS_LIB_DIR"),
        }
        match prior_nm {
            Some(v) => std::env::set_var("BEARWISDOM_TS_NODE_MODULES", v),
            None => std::env::remove_var("BEARWISDOM_TS_NODE_MODULES"),
        }
    }
    result.expect("index failed");

    // Preconditions: the stub externals MUST be indexed, else the patterns that
    // depend on them (String boxing, foreign-pick competitor) pass vacuously.
    let stub_string: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbols WHERE name='String' AND kind='interface' AND origin='external'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let external_toast: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbols WHERE name='toast' AND origin='external'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    println!("\n--- preconditions ---");
    println!("  stub String interface indexed: {stub_string}");
    println!("  external @base-ui toast indexed: {external_toast}");
    assert!(
        stub_string >= 1,
        "precondition: stub `String` interface must be indexed (BEARWISDOM_TS_LIB_DIR), else the \
         string-boxing patterns pass vacuously"
    );
    assert!(
        external_toast >= 1,
        "precondition: external `@base-ui/react.toast` must be indexed, else the foreign-pick \
         pattern has no competitor and passes vacuously"
    );

    // Candidate probes — KNOWN-RED next targets, diagnostic only (not asserted).
    // Promote a row to `checks` once fixed. Root causes (traced):
    //   optional-arr — `T[] | undefined` interns as `Union([Array, undefined])`, and
    //     union member access requires the member on EVERY arm, so the `undefined`
    //     arm kills `.map`. Needs nullish arms peeled before the union walk.
    //   tuple-destructure — `const [count, reset] = f()` (array-pattern) is
    //     structurally extracted (a Variable symbol per element plus a
    //     tuple-index TypeRef on each), but `TS_FLOW_CONFIG`'s destructure
    //     capture only matches `object_pattern`; no array-pattern arm feeds
    //     `flow_binding_destructure`, so the binding is never seeded into the
    //     per-file flow cache. `reset()` still shows a resolved edge, but to
    //     `reset`'s own local declaration (the only same-named symbol in the
    //     file) rather than a real callable — `LocalFlowHeadRule` never gets a
    //     recorded local type to work from.
    println!("\n--- candidate probes (known red) ---");
    println!(
        "  optional-arr  items?.map() resolved-to-Array={} unresolved={}",
        count_resolved_to(&db, "optional_array.ts", "map", "%Array%"),
        count_unresolved(&db, "optional_array.ts", "map")
    );
    println!(
        "  tuple-destructure  reset() resolved-to-its-own-decl={} unresolved={}",
        count_resolved_kind(&db, "tuple_destructure.ts", "reset", "variable"),
        count_unresolved(&db, "tuple_destructure.ts", "reset")
    );

    // Each row: (label, pass, detail). Printed as a table so a fix's effect is a
    // single line flipping ✅/❌.
    let string_param_replace = count_resolved_to(&db, "string_param.ts", "replace", "%String%");
    let string_param_lower = count_resolved_to(&db, "string_param.ts", "toLowerCase", "%String%");
    let local_param_getname = count_resolved_to(&db, "local_param.ts", "getName", "%User%");
    let notify_dismiss_external = count_resolved_to(&db, "notify.ts", "dismiss", "%base-ui%");
    let notify_dismiss_unresolved = count_unresolved(&db, "notify.ts", "dismiss");
    let obj_return_info = count_resolved(&db, "obj_return.ts", "info");
    let return_type_go = count_resolved(&db, "return_type.ts", "go");
    let obj_return_destructure_info =
        count_resolved_to(&db, "obj_return_destructure.ts", "info", "%$Ret%");
    let spy_const_mock_restore = count_resolved(&db, "spy_const.ts", "mockRestore");

    let checks = [
        (
            "string param  text.replace() -> String.replace",
            string_param_replace >= 1,
            format!("resolved-to-String edges = {string_param_replace}"),
        ),
        (
            "string param  text.toLowerCase() -> String.toLowerCase",
            string_param_lower >= 1,
            format!("resolved-to-String edges = {string_param_lower}"),
        ),
        (
            "local param   u.getName() -> User.getName",
            local_param_getname >= 1,
            format!("resolved-to-User edges = {local_param_getname}"),
        ),
        (
            "foreign pick  toast.dismiss() NOT -> @base-ui",
            notify_dismiss_external == 0,
            format!(
                "external-dismiss edges = {notify_dismiss_external} (unresolved = {notify_dismiss_unresolved})"
            ),
        ),
        (
            "obj return    log.info() -> makeLogger$Ret.info",
            obj_return_info >= 1,
            format!("resolved edges = {obj_return_info}"),
        ),
        (
            "return type   h.go() -> make$Ret.go (via ReturnType<typeof make>)",
            return_type_go >= 1,
            format!("resolved edges = {return_type_go}"),
        ),
        (
            "obj destructure  info(\"hi\") -> makeLogger2$Ret.info",
            obj_return_destructure_info >= 1,
            format!("resolved-to-$Ret edges = {obj_return_destructure_info}"),
        ),
        (
            "spy const     s.mockRestore() -> SpyInstance.mockRestore",
            spy_const_mock_restore >= 1,
            format!("resolved edges = {spy_const_mock_restore}"),
        ),
    ];

    println!("\n=== resolution corpus ===");
    let mut failures = Vec::new();
    for (label, pass, detail) in &checks {
        println!("  {} {label}  [{detail}]", if *pass { "✅" } else { "❌" });
        if !pass {
            failures.push(*label);
        }
    }
    println!("  {} / {} patterns resolved as expected\n", checks.len() - failures.len(), checks.len());

    assert!(
        failures.is_empty(),
        "resolution corpus regressions: {failures:?}"
    );
}
