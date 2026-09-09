//! Diagnostic probes for the September 2026 architecture review.
//! Copy to tests/tests/audit_engine_probe.rs to run with:
//! cargo test -p bearwisdom-tests --test audit_engine_probe -- --nocapture --test-threads=1
//! All six tests failed against 32da1ea3; these assert intended behavior.
use bearwisdom::{full_index, Database};
use bearwisdom_tests::TestProject;

fn project(files: &[(&str, &str)]) -> (TestProject, Database) {
    let p = TestProject { dir: tempfile::TempDir::new().unwrap() };
    for (path, body) in files { p.add_file(path, body); }
    let mut db = TestProject::in_memory_db();
    full_index(&mut db, p.path(), None, None, None).unwrap();
    (p, db)
}

fn calls(db: &Database) -> Vec<(String, String, String, i64)> {
    db.prepare("SELECT f.path, s.qualified_name, t.qualified_name, e.source_line FROM edges e JOIN symbols s ON s.id=e.source_id JOIN symbols t ON t.id=e.target_id JOIN files f ON f.id=s.file_id WHERE e.kind='calls' AND f.origin='internal' ORDER BY f.path,e.source_line,t.qualified_name").unwrap()
        .query_map([], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap().map(Result::unwrap).collect()
}

#[test]
fn audit_scope_collision() {
    let (_, db) = project(&[("main.ts", "class Alpha { save(): void {} }\nclass Beta { save(): void {} }\nfunction first(value: Alpha) { value.save(); }\nfunction second(value: Beta) { value.save(); }\n")]);
    println!("AUDIT scope_collision {:?}", calls(&db));
    let first = calls(&db).into_iter().find(|r|r.1=="first").unwrap();
    assert_eq!(first.2,"Alpha.save");
}

#[test]
fn audit_branch_narrowing() {
    let (_, db) = project(&[("main.ts", "class Alpha { onlyAlpha(): void {} }\nclass Beta { onlyBeta(): void {} }\nfunction run(value: Alpha | Beta) {\n if (value instanceof Alpha) { value.onlyAlpha(); }\n}\n")]);
    println!("AUDIT branch_narrowing {:?}", calls(&db));
    assert!(calls(&db).iter().any(|r|r.2=="Alpha.onlyAlpha"));
}

#[test]
fn audit_occurrences_and_cache() {
    let (_, mut db) = project(&[("main.py", "def helper():\n    return 1\ndef caller():\n    helper(); helper()\n    helper()\n    helper()\n")]);
    let count: i64 = db.query_row("SELECT COUNT(*) FROM ref_resolutions r JOIN symbols t ON t.id=r.target_id WHERE t.name='helper' AND r.kind='calls'", [], |r|r.get(0)).unwrap();
    let raw = bearwisdom::query::references::find_references(&db,"helper",0).unwrap();
    db.query_cache=Some(std::sync::Arc::new(bearwisdom::query::cache::QueryCache::new(10)));
    let small=bearwisdom::query::references::find_references(&db,"helper",1).unwrap();
    let all=bearwisdom::query::references::find_references(&db,"helper",0).unwrap();
    println!("AUDIT occurrences ref_sites={count} uncached={} limited={} subsequent_unlimited={}",raw.len(),small.len(),all.len());
    assert_eq!(all.len(),4);
}

#[test]
fn audit_module_identity() {
    let (_, db) = project(&[("a.ts", "export interface Config { onlyA(): void; }\n"),("b.ts", "export interface Config { onlyB(): void; }\n")]);
    let rows: Vec<(i64,String,String)> = db.prepare("SELECT s.id,s.qualified_name,f.path FROM symbols s JOIN files f ON f.id=s.file_id WHERE s.name='Config'").unwrap().query_map([], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap().map(Result::unwrap).collect();
    println!("AUDIT module_identity {rows:?}");
    assert_eq!(rows.len(),2);
}

#[test]
fn audit_incremental_return_type() {
    let api_a="export class Alpha { save(): void {} }\nexport class Beta { save(): void {} }\nexport function make(): Alpha { return new Alpha(); }\n";
    let api_b="export class Alpha { save(): void {} }\nexport class Beta { save(): void {} }\nexport function make(): Beta { return new Beta(); }\n";
    let (p, mut db) = project(&[("api.ts", api_a),("consumer.ts", "import { make } from './api';\nexport function run() { const x = make(); x.save(); }\n")]);
    let before=calls(&db);
    p.add_file("api.ts",api_b);
    let stats=bearwisdom::indexer::incremental::incremental_index(&mut db,p.path(),None).unwrap();
    let after=calls(&db);
    let mut fresh=TestProject::in_memory_db();
    full_index(&mut fresh,p.path(),None,None,None).unwrap();
    let rebuilt=calls(&fresh);
    println!("AUDIT incremental stats={stats:?} before={before:?} after={after:?} fresh={rebuilt:?}");
    assert_eq!(after,rebuilt);
}

#[test]
fn audit_overload_selection() {
    let (_, db) = project(&[("main.ts", "class Alpha { save(): void {} }\nclass Beta { save(): void {} }\ndeclare function make(x: string): Alpha;\ndeclare function make(x: number): Beta;\nfunction run() { make(123).save(); }\n")]);
    println!("AUDIT overload {:?}",calls(&db));
    assert!(calls(&db).iter().any(|r|r.2=="Beta.save"));
}
