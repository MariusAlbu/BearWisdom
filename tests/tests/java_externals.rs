//! Integration test for Java externals via Maven local repository.
//!
//! Seeds a fake `~/.m2/repository` with a single `-sources.jar` (zip file
//! assembled in memory), points `BEARWISDOM_JAVA_MAVEN_REPO` at it, indexes
//! a tiny Maven project whose pom.xml depends on that artifact, and asserts
//! the externals pipeline end-to-end:
//!
//!   1. Sources jar is discovered + extracted to the cache dir
//!   2. External `.java` files land with origin='external'
//!   3. Internal queries still see internal symbols only
//!   4. User-code refs resolve into edges → external symbols (loop closure)
//!
//! No Maven CLI or JDK required — the jar is built in-memory with the zip
//! crate.

use std::fs;
use std::io::Write;
use std::sync::Mutex;

use bearwisdom::full_index;
use bearwisdom_tests::TestProject;
use tempfile::TempDir;
use zip::write::SimpleFileOptions;

/// `BEARWISDOM_JAVA_MAVEN_REPO` is process-global; tests that set it must not
/// run concurrently. Held across each test's full set→index→restore window.
/// Poison is recovered so a panicking test still leaves a usable lock.
static ENV_LOCK: Mutex<()> = Mutex::new(());

/// Build an in-memory `-sources.jar` with one Java file at
/// `com/fakeext/greeter/Greeter.java`, write it into the synthetic Maven
/// layout, and return the repo root.
fn seed_fake_maven_repo() -> TempDir {
    let repo = TempDir::new().unwrap();
    let artifact_dir = repo
        .path()
        .join("com")
        .join("fakeext")
        .join("greeter")
        .join("1.0.0");
    fs::create_dir_all(&artifact_dir).unwrap();

    let jar_path = artifact_dir.join("greeter-1.0.0-sources.jar");
    let jar_file = fs::File::create(&jar_path).unwrap();
    let mut zip = zip::ZipWriter::new(jar_file);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    zip.start_file("com/fakeext/greeter/Greeter.java", options)
        .unwrap();
    zip.write_all(
        br#"package com.fakeext.greeter;

/** A trivial greeting helper. */
public class Greeter {
    private final String prefix;

    public Greeter(String prefix) {
        this.prefix = prefix;
    }

    /** Build a greeting for {@code name}. */
    public String greet(String name) {
        return prefix + " " + name;
    }
}
"#,
    )
    .unwrap();

    // A second class to make the symbol count assertion meaningful.
    zip.start_file("com/fakeext/greeter/Formatter.java", options)
        .unwrap();
    zip.write_all(
        br#"package com.fakeext.greeter;

public interface Formatter {
    String format(String input);
}
"#,
    )
    .unwrap();

    zip.finish().unwrap();

    repo
}

/// Build a tiny Maven project whose `pom.xml` depends on the fake artifact.
fn seed_consumer_project() -> TestProject {
    let project = TestProject {
        dir: TempDir::new().unwrap(),
    };

    project.add_file(
        "pom.xml",
        r#"<?xml version="1.0" encoding="UTF-8"?>
<project>
    <modelVersion>4.0.0</modelVersion>
    <groupId>example.consumer</groupId>
    <artifactId>consumer</artifactId>
    <version>0.0.1-SNAPSHOT</version>
    <dependencies>
        <dependency>
            <groupId>com.fakeext</groupId>
            <artifactId>greeter</artifactId>
            <version>1.0.0</version>
        </dependency>
    </dependencies>
</project>
"#,
    );

    project.add_file(
        "src/main/java/example/consumer/App.java",
        r#"package example.consumer;

import com.fakeext.greeter.Greeter;

public class App {
    public static void main(String[] args) {
        Greeter g = new Greeter("Hello,");
        System.out.println(g.greet("world"));
    }
}
"#,
    );

    project
}

/// Build an ISOLATED fake Maven repo: the `.m2` is nested under a fresh
/// TempDir so the derived `<parent>/bearwisdom-sources-cache` is per-test, not
/// the shared system-temp cache that pollutes results across runs. Returns the
/// anchor TempDir; the repo is at `<anchor>/m2`. The data jar holds
/// `Repository.findOne(): Entity` and `Entity.getEmail(): String`.
fn seed_isolated_chain_repo() -> TempDir {
    let anchor = TempDir::new().unwrap();
    let artifact_dir = anchor
        .path()
        .join("m2")
        .join("com")
        .join("fakeext")
        .join("data")
        .join("2.0.0");
    fs::create_dir_all(&artifact_dir).unwrap();

    let jar_path = artifact_dir.join("data-2.0.0-sources.jar");
    let jar_file = fs::File::create(&jar_path).unwrap();
    let mut zip = zip::ZipWriter::new(jar_file);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    zip.start_file("com/fakeext/data/Repository.java", options)
        .unwrap();
    zip.write_all(
        br#"package com.fakeext.data;

public class Repository {
    public Entity findOne() { return null; }
}
"#,
    )
    .unwrap();

    zip.start_file("com/fakeext/data/Entity.java", options)
        .unwrap();
    zip.write_all(
        br#"package com.fakeext.data;

public class Entity {
    public String getEmail() { return ""; }
}
"#,
    )
    .unwrap();

    zip.finish().unwrap();
    anchor
}

/// Consumer that chains through `Repository.findOne()` WITHOUT naming/importing
/// the return type `Entity` — the fluent-API / junit-assertion shape. The only
/// `getEmail` reference is the second hop, so the assertion can't false-pass on
/// a direct `Entity` chain.
fn seed_no_import_chain_consumer() -> TestProject {
    let project = TestProject {
        dir: TempDir::new().unwrap(),
    };
    project.add_file(
        "pom.xml",
        r#"<?xml version="1.0" encoding="UTF-8"?>
<project>
    <modelVersion>4.0.0</modelVersion>
    <groupId>example.consumer</groupId>
    <artifactId>consumer</artifactId>
    <version>0.0.1-SNAPSHOT</version>
    <dependencies>
        <dependency>
            <groupId>com.fakeext</groupId>
            <artifactId>data</artifactId>
            <version>2.0.0</version>
        </dependency>
    </dependencies>
</project>
"#,
    );
    project.add_file(
        "src/main/java/example/consumer/App.java",
        r#"package example.consumer;

import com.fakeext.data.Repository;

public class App {
    public void run(Repository repo) {
        repo.findOne().getEmail();
    }
}
"#,
    );
    project
}

/// Control (no externals): the SAME chain shape with all classes INTERNAL.
/// Isolates "chain rooted on a method parameter" from external hydration.
///
/// A chain rooted directly on a method PARAMETER (`repo.findOne().getEmail()`)
/// type-walks. The parameter roots the chain through its enclosing method's
/// structural children (`members_by_parent` keyed by `parent_index`), and its
/// declared type resolves in the method's scope — so resolution holds even
/// though the parameter's own qname is package-less.
#[test]
fn internal_java_param_rooted_chain_resolves() {
    let project = TestProject {
        dir: TempDir::new().unwrap(),
    };
    project.add_file(
        "src/main/java/app/Repository.java",
        "package app;\npublic class Repository {\n    public Entity findOne() { return null; }\n}\n",
    );
    project.add_file(
        "src/main/java/app/Entity.java",
        "package app;\npublic class Entity {\n    public String getEmail() { return \"\"; }\n}\n",
    );
    project.add_file(
        "src/main/java/app/App.java",
        "package app;\npublic class App {\n    public void run(Repository repo) {\n        repo.findOne().getEmail();\n    }\n}\n",
    );

    let mut db = TestProject::in_memory_db();
    let _ = full_index(&mut db, project.path(), None, None, None).unwrap();

    let getemail_edge: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM edges e
             JOIN symbols s ON s.id = e.target_id
             WHERE s.name = 'getEmail'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        getemail_edge >= 1,
        "internal param-rooted chain repo.findOne().getEmail() did not resolve"
    );
}

/// EXT-2 end-to-end: a chain types past an external Java method because its
/// return type is hydrated from the Maven sources jar, and the never-imported
/// return-type class is pulled on demand so the second hop binds.
///
/// `repo.findOne().getEmail()`: the parameter root types to `Repository`, the
/// chain qualifies it to its imported qname `com.fakeext.data.Repository`, walks
/// `findOne` to its return type `Entity`, qualifies that same-package to
/// `com.fakeext.data.Entity`, and pulls `Entity` from the jar on the chain-miss
/// pass — `Entity` is never imported (only a return type), so the same-package
/// qualification is the only path to the right one of the several `Entity`
/// classes the JDK also publishes. Uses an isolated extraction cache (the `.m2`
/// nested under a fresh TempDir) so results aren't polluted by the shared
/// system-temp `bearwisdom-sources-cache` across runs.
#[test]
fn external_java_chain_types_past_external_method_return() {
    let _env = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let anchor = seed_isolated_chain_repo();
    let m2 = anchor.path().join("m2");
    let project = seed_no_import_chain_consumer();

    let prior = std::env::var_os("BEARWISDOM_JAVA_MAVEN_REPO");
    unsafe {
        std::env::set_var("BEARWISDOM_JAVA_MAVEN_REPO", &m2);
    }
    let mut db = TestProject::in_memory_db();
    let _ = full_index(&mut db, project.path(), None, None, None).unwrap();
    unsafe {
        match prior {
            Some(v) => std::env::set_var("BEARWISDOM_JAVA_MAVEN_REPO", v),
            None => std::env::remove_var("BEARWISDOM_JAVA_MAVEN_REPO"),
        }
    }

    let repo_indexed: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbols WHERE name = 'Repository' AND origin = 'external'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        repo_indexed >= 1,
        "Repository (imported) not pulled ({repo_indexed})"
    );

    let entity_indexed: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbols WHERE name = 'Entity' AND origin = 'external'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        entity_indexed >= 1,
        "transitive return type Entity not pulled on demand ({entity_indexed})"
    );

    let getemail_edge: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM edges e
             JOIN symbols s ON s.id = e.target_id
             WHERE s.origin = 'external' AND s.name = 'getEmail'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        getemail_edge >= 1,
        "chain did not type past the external method's return type"
    );
}

#[test]
fn external_java_package_is_indexed_and_resolved() {
    let _env = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let repo = seed_fake_maven_repo();
    let project = seed_consumer_project();

    let prior_repo = std::env::var_os("BEARWISDOM_JAVA_MAVEN_REPO");
    // SAFETY: std::env::set_var is process-global. This test owns the
    // BEARWISDOM_JAVA_MAVEN_REPO variable for its duration and restores it
    // before returning so sibling tests that read the value still see the
    // original environment.
    unsafe {
        std::env::set_var("BEARWISDOM_JAVA_MAVEN_REPO", repo.path());
    }

    let mut db = TestProject::in_memory_db();
    let stats = full_index(&mut db, project.path(), None, None, None).unwrap();

    unsafe {
        match prior_repo {
            Some(v) => std::env::set_var("BEARWISDOM_JAVA_MAVEN_REPO", v),
            None => std::env::remove_var("BEARWISDOM_JAVA_MAVEN_REPO"),
        }
    }

    // --- Assertion 1: internal stats ignore externals ---
    assert!(
        stats.file_count >= 1,
        "expected at least one internal file (App.java), got {}",
        stats.file_count
    );

    // --- Assertion 2: external files landed ---
    let external_files: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM files WHERE origin = 'external'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        external_files >= 2,
        "expected Greeter.java + Formatter.java (2 externals), got {external_files}"
    );

    // --- Assertion 3: external symbols indexed ---
    let external_symbols: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbols WHERE origin = 'external'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        external_symbols >= 2,
        "expected Greeter + Formatter classes as external symbols, got {external_symbols}"
    );

    // --- Assertion 4: internal queries skip externals ---
    let internal_symbols: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbols WHERE origin = 'internal'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        internal_symbols >= 1,
        "expected at least 1 internal symbol (App), got {internal_symbols}"
    );

    // Search must not leak externals into user-facing results.
    let search_hits =
        bearwisdom::query::search::search_symbols(&db, "Greeter", 10, &Default::default()).unwrap();
    assert!(
        search_hits
            .iter()
            .all(|s| !s.file_path.contains("ext:java:")),
        "search_symbols leaked an external symbol: {:?}",
        search_hits
            .iter()
            .map(|s| &s.qualified_name)
            .collect::<Vec<_>>()
    );
}
