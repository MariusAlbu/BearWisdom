//! A receiver head spelled as a simple name binds the declaration its file
//! IMPORTS, not an equally-named class from another package.
//!
//! Seeds one `-sources.jar` carrying two `Widget` classes in sibling packages,
//! each declaring the same method. The consumer imports exactly one of them and
//! calls that method through a parameter, so the only evidence separating the
//! two homonyms is the import line. The chain walker must bind the imported
//! declaration's id — with neither bound, the member lookup misses and no edge
//! is written at all.
//!
//! No Maven CLI or JDK required — the jar is built in-memory with the zip crate.

use std::fs;
use std::io::Write;
use std::sync::Mutex;

use bearwisdom::full_index;
use bearwisdom_tests::TestProject;
use tempfile::TempDir;
use zip::write::SimpleFileOptions;

/// `BEARWISDOM_JAVA_MAVEN_REPO` is process-global; the set→index→restore window
/// is held under this lock. Poison is recovered so a panicking test still
/// leaves a usable lock.
static ENV_LOCK: Mutex<()> = Mutex::new(());

/// A `.m2` nested under a fresh TempDir, so the derived sources-extraction
/// cache is per-test rather than the shared system-temp one. The jar declares
/// `com.fakeext.alpha.Widget.spin()` and `com.fakeext.beta.Widget.spin()`.
fn seed_homonym_repo() -> TempDir {
    let anchor = TempDir::new().unwrap();
    let artifact_dir = anchor
        .path()
        .join("m2")
        .join("com")
        .join("fakeext")
        .join("widgets")
        .join("3.0.0");
    fs::create_dir_all(&artifact_dir).unwrap();

    let jar_file = fs::File::create(artifact_dir.join("widgets-3.0.0-sources.jar")).unwrap();
    let mut zip = zip::ZipWriter::new(jar_file);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    zip.start_file("com/fakeext/alpha/Widget.java", options)
        .unwrap();
    zip.write_all(
        br#"package com.fakeext.alpha;

public class Widget {
    public String spin() { return "alpha"; }
}
"#,
    )
    .unwrap();

    zip.start_file("com/fakeext/beta/Widget.java", options)
        .unwrap();
    zip.write_all(
        br#"package com.fakeext.beta;

public class Widget {
    public String spin() { return "beta"; }
}
"#,
    )
    .unwrap();

    zip.finish().unwrap();
    anchor
}

/// A consumer importing ONLY `com.fakeext.alpha.Widget` and calling `spin()`
/// through a parameter of that type.
fn seed_consumer() -> TestProject {
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
            <artifactId>widgets</artifactId>
            <version>3.0.0</version>
        </dependency>
    </dependencies>
</project>
"#,
    );
    project.add_file(
        "src/main/java/example/consumer/App.java",
        r#"package example.consumer;

import com.fakeext.alpha.Widget;

public class App {
    public void run(Widget widget) {
        widget.spin();
    }
}
"#,
    );
    project
}

#[test]
fn an_imported_head_binds_the_imported_homonym() {
    let _env = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let anchor = seed_homonym_repo();
    let m2 = anchor.path().join("m2");
    let project = seed_consumer();

    let prior = std::env::var_os("BEARWISDOM_JAVA_MAVEN_REPO");
    // SAFETY: process-global. This test owns the variable for its duration and
    // restores it before returning.
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

    // Both homonyms must be indexed, or the pick has nothing to disambiguate.
    let widgets: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbols
             WHERE name = 'Widget' AND kind = 'class' AND origin = 'external'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        widgets >= 2,
        "both Widget homonyms must be indexed for the pick to be a pick ({widgets})"
    );

    let alpha_edges: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM edges e
             JOIN symbols s ON s.id = e.target_id
             JOIN files f ON f.id = s.file_id
             WHERE s.name = 'spin' AND f.path LIKE '%alpha%'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        alpha_edges >= 1,
        "widget.spin() did not bind the imported com.fakeext.alpha.Widget"
    );

    let beta_edges: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM edges e
             JOIN symbols s ON s.id = e.target_id
             JOIN files f ON f.id = s.file_id
             WHERE s.name = 'spin' AND f.path LIKE '%beta%'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        beta_edges, 0,
        "the un-imported homonym must not receive the call"
    );
}
