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
  "dependencies": { "@base-ui/react": "^1.0.0" }
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

    // Each row: (label, pass, detail). Printed as a table so a fix's effect is a
    // single line flipping ✅/❌.
    let string_param_replace = count_resolved_to(&db, "string_param.ts", "replace", "%String%");
    let string_param_lower = count_resolved_to(&db, "string_param.ts", "toLowerCase", "%String%");
    let local_param_getname = count_resolved_to(&db, "local_param.ts", "getName", "%User%");
    let notify_dismiss_external = count_resolved_to(&db, "notify.ts", "dismiss", "%base-ui%");
    let notify_dismiss_unresolved = count_unresolved(&db, "notify.ts", "dismiss");

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
