//! Member access through a composite type alias.
//!
//! A union carries a member when every arm that HAS a member surface carries
//! it. An arm spelled with an absence atom (`null` / `undefined`) has none, so
//! it neither supplies the member nor vetoes it — `Widget | null` answers
//! `spin` with `Widget`'s declaration. An arm that does declare members and
//! lacks the one being reached still makes the access invalid. An intersection
//! is additive: any arm may supply the member.
//!
//! The accesses are written unguarded on purpose: the resolver binds a
//! reference to the declaration it names, and the answer must not depend on
//! whether a narrowing guard or an optional chain preceded the access.

use bearwisdom::full_index;
use bearwisdom_tests::TestProject;
use tempfile::TempDir;

/// Classes and interfaces plus the four composite aliases over them. Each
/// consumer file below reaches one member through one alias.
fn seed_composites() -> TestProject {
    let project = TestProject {
        dir: TempDir::new().unwrap(),
    };
    project.add_file("package.json", r#"{"name":"union-members"}"#);
    project.add_file(
        "tsconfig.json",
        r#"{"compilerOptions":{"strict":true},"include":["src"]}"#,
    );
    project.add_file(
        "src/shapes.ts",
        concat!(
            "export class Circle {\n    area(): number {\n        return 1;\n    }\n}\n\n",
            "export class Square {\n    area(): number {\n        return 2;\n    }\n}\n\n",
            "export class Widget {\n    spin(): void {}\n}\n\n",
            "export interface Gadget {\n    rattle(): void;\n}\n\n",
            "export type Shape = Circle | Square;\n",
            "export type Maybe = Widget | null;\n",
            "export type Both = Circle & Gadget;\n",
            "export type Mixed = Circle | Widget;\n",
        ),
    );
    // Every arm declares `area` — the union carries it.
    project.add_file(
        "src/every_arm.ts",
        "import type { Shape } from './shapes';\n\nexport function measure(s: Shape): number {\n    return s.area();\n}\n",
    );
    // One arm is an absence atom — the other arm answers.
    project.add_file(
        "src/absent_arm.ts",
        "import type { Maybe } from './shapes';\n\nexport function start(m: Maybe): void {\n    m.spin();\n}\n",
    );
    // An intersection is additive: the member lives on the second arm only.
    project.add_file(
        "src/intersection_arm.ts",
        "import type { Both } from './shapes';\n\nexport function shake(b: Both): void {\n    b.rattle();\n}\n",
    );
    // `Widget` has members and declares no `area`, so the union does not carry it.
    project.add_file(
        "src/missing_arm.ts",
        "import type { Mixed } from './shapes';\n\nexport function size(x: Mixed): number {\n    return x.area();\n}\n",
    );
    project
}

fn index(project: &TestProject) -> bearwisdom::Database {
    let mut db = TestProject::in_memory_db();
    full_index(&mut db, project.path(), None, None, None).unwrap();
    db
}

/// Every call edge leaving `file`, as `(target name, target qualified name,
/// declaring file)`.
fn calls_from(db: &bearwisdom::Database, file: &str) -> Vec<(String, String, String)> {
    let mut stmt = db
        .prepare(
            "SELECT t.name, t.qualified_name, tf.path FROM edges e
             JOIN symbols s ON s.id = e.source_id
             JOIN files sf ON sf.id = s.file_id
             JOIN symbols t ON t.id = e.target_id
             JOIN files tf ON tf.id = t.file_id
             WHERE sf.path = ?1 AND e.kind = 'calls'
             ORDER BY t.name, t.qualified_name",
        )
        .unwrap();
    stmt.query_map([file], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// Undrained unresolved refs left by `file`, as `(target name, cause kind)`.
fn unresolved_in(db: &bearwisdom::Database, file: &str) -> Vec<(String, String)> {
    let mut stmt = db
        .prepare(
            "SELECT u.target_name, COALESCE(u.cause_kind, '') FROM unresolved_refs u
             JOIN symbols s ON s.id = u.source_id
             JOIN files f ON f.id = s.file_id
             WHERE f.path = ?1 AND u.drained = 0 AND u.kind <> 'imports'
             ORDER BY u.target_name",
        )
        .unwrap();
    stmt.query_map([file], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

#[test]
fn a_union_whose_every_arm_declares_the_member_binds_it() {
    let db = index(&seed_composites());
    let calls = calls_from(&db, "src/every_arm.ts");
    // A ref carries one target id, so the edge names the arm the walk settled
    // on; what this asserts is that the union does not decline.
    assert!(
        calls
            .iter()
            .any(|(name, qname, file)| name == "area"
                && file == "src/shapes.ts"
                && (qname == "Circle.area" || qname == "Square.area")),
        "`s.area()` binds to an arm's declaration: {calls:?}"
    );
    assert!(
        unresolved_in(&db, "src/every_arm.ts")
            .iter()
            .all(|(name, _)| name != "area"),
        "nothing is left unresolved for `area`"
    );
}

#[test]
fn a_union_with_an_absent_arm_binds_on_the_arm_that_has_members() {
    let db = index(&seed_composites());
    let calls = calls_from(&db, "src/absent_arm.ts");
    assert!(
        calls
            .iter()
            .any(|(name, qname, file)| name == "spin"
                && qname == "Widget.spin"
                && file == "src/shapes.ts"),
        "`Widget | null` answers `spin` with `Widget.spin`: {calls:?}"
    );
}

#[test]
fn an_intersection_binds_a_member_declared_on_a_later_arm() {
    let db = index(&seed_composites());
    let calls = calls_from(&db, "src/intersection_arm.ts");
    assert!(
        calls
            .iter()
            .any(|(name, qname, file)| name == "rattle"
                && qname == "Gadget.rattle"
                && file == "src/shapes.ts"),
        "`Circle & Gadget` is additive, so `rattle` comes from `Gadget`: {calls:?}"
    );
}

#[test]
fn a_union_arm_with_members_that_lacks_the_member_declines_the_access() {
    let db = index(&seed_composites());
    let calls = calls_from(&db, "src/missing_arm.ts");
    assert!(
        !calls.iter().any(|(name, _, _)| name == "area"),
        "`Widget` declares members and no `area`, so the union does not carry it: {calls:?}"
    );
    let left = unresolved_in(&db, "src/missing_arm.ts");
    // The receiver is bound to the alias declaration, which carries no members
    // of its own — `member_missing` — or, when no declaration was bound to the
    // head, the alias shape itself is what could not answer — `alias_opaque`.
    assert!(
        left.iter()
            .any(|(name, cause)| name == "area"
                && (cause == "member_missing" || cause == "alias_opaque")),
        "the declined access is recorded against the alias: {left:?}"
    );
}
