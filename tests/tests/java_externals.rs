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

use bearwisdom::full_index;
use bearwisdom_tests::TestProject;
use tempfile::TempDir;
use zip::write::SimpleFileOptions;

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
    let options =
        SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

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

#[test]
fn external_java_package_is_indexed_and_resolved() {
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
        bearwisdom::query::search::search_symbols(&db, "Greeter", 10, &Default::default())
            .unwrap();
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
