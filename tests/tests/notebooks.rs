//! Integration tests for E5 — Notebook family (.ipynb, .Rmd, .qmd, .dib).
//!
//! Verifies end-to-end through the indexer:
//!
//!   1. `.dib` files produce C# + F# + PowerShell symbols from their
//!      respective cells, attributed to the dib file.
//!   2. `.Rmd` chunks with `{r}` and `{python}` produce R + Python
//!      symbols.
//!   3. `.qmd` chunks produce Python symbols (Quarto reuses the same
//!      host-scan + chunk dispatch logic as RMarkdown).
//!   4. `.ipynb` Python cells produce Python symbols with correct
//!      `origin_language = 'python'` attribution.
//!   5. Jupyter magic lines (`!pip install`, `%timeit`) do not break
//!      the Python parser — a symbol defined after a magic line still
//!      surfaces.

use std::fs;

use bearwisdom::full_index;
use bearwisdom_tests::TestProject;
use tempfile::TempDir;

#[test]
fn dib_cells_produce_csharp_and_fsharp_symbols() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::write(
        root.join("demo.dib"),
        "#!csharp\npublic class CsClass { public int X; }\n\n#!fsharp\nlet addOne x = x + 1\n",
    )
    .unwrap();

    let mut db = TestProject::in_memory_db();
    full_index(&mut db, root, None, None, None).expect("index failed");

    let lang: String = db
        .query_row(
            "SELECT language FROM files WHERE path LIKE '%demo.dib'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(lang, "polyglot_nb");

    let cs_syms: Vec<String> = db
        .prepare(
            "SELECT s.name FROM symbols s
             JOIN files f ON f.id = s.file_id
             WHERE f.path LIKE '%demo.dib'
               AND s.origin_language = 'csharp'",
        )
        .unwrap()
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .flatten()
        .collect();
    assert!(
        cs_syms.iter().any(|n| n == "CsClass"),
        "expected CsClass from C# cell in .dib, got {cs_syms:?}"
    );

    let fs_syms: Vec<String> = db
        .prepare(
            "SELECT s.name FROM symbols s
             JOIN files f ON f.id = s.file_id
             WHERE f.path LIKE '%demo.dib'
               AND s.origin_language = 'fsharp'",
        )
        .unwrap()
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .flatten()
        .collect();
    assert!(
        fs_syms.iter().any(|n| n == "addOne"),
        "expected addOne from F# cell in .dib, got {fs_syms:?}"
    );
}

#[test]
fn rmd_r_chunk_produces_r_symbols() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::write(
        root.join("report.Rmd"),
        r#"---
title: "Report"
---

# Analysis

```{r setup, echo=FALSE}
compute_mean <- function(xs) mean(xs)
```

```{python}
def pyhelper(xs):
    return sum(xs) / len(xs)
```
"#,
    )
    .unwrap();

    let mut db = TestProject::in_memory_db();
    full_index(&mut db, root, None, None, None).expect("index failed");

    let lang: String = db
        .query_row(
            "SELECT language FROM files WHERE path LIKE '%report.Rmd'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(lang, "rmarkdown");

    let r_syms: Vec<String> = db
        .prepare(
            "SELECT s.name FROM symbols s
             JOIN files f ON f.id = s.file_id
             WHERE f.path LIKE '%report.Rmd'
               AND s.origin_language = 'r'",
        )
        .unwrap()
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .flatten()
        .collect();
    assert!(
        r_syms.iter().any(|n| n == "compute_mean"),
        "expected R symbol compute_mean from {{r}} chunk, got {r_syms:?}"
    );

    let py_syms: Vec<String> = db
        .prepare(
            "SELECT s.name FROM symbols s
             JOIN files f ON f.id = s.file_id
             WHERE f.path LIKE '%report.Rmd'
               AND s.origin_language = 'python'",
        )
        .unwrap()
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .flatten()
        .collect();
    assert!(
        py_syms.iter().any(|n| n == "pyhelper"),
        "expected Python symbol pyhelper from {{python}} chunk, got {py_syms:?}"
    );
}

#[test]
fn qmd_python_chunk_produces_python_symbols() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::write(
        root.join("doc.qmd"),
        r#"---
title: "Doc"
---

# Section

```{python}
def quarto_func(x):
    return x * 2
```
"#,
    )
    .unwrap();

    let mut db = TestProject::in_memory_db();
    full_index(&mut db, root, None, None, None).expect("index failed");

    let lang: String = db
        .query_row(
            "SELECT language FROM files WHERE path LIKE '%doc.qmd'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(lang, "quarto");

    let py_syms: Vec<String> = db
        .prepare(
            "SELECT s.name FROM symbols s
             JOIN files f ON f.id = s.file_id
             WHERE f.path LIKE '%doc.qmd'
               AND s.origin_language = 'python'",
        )
        .unwrap()
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .flatten()
        .collect();
    assert!(
        py_syms.iter().any(|n| n == "quarto_func"),
        "expected Python quarto_func from .qmd {{python}} chunk, got {py_syms:?}"
    );
}

#[test]
fn ipynb_code_cells_produce_python_symbols() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    let notebook = r##"{
 "cells": [
  {"cell_type": "markdown", "source": "# Analysis", "metadata": {}},
  {"cell_type": "code", "source": "def compute(x):\n    return x * 2\n", "metadata": {}}
 ],
 "metadata": {"kernelspec": {"language": "python", "name": "python3"}},
 "nbformat": 4,
 "nbformat_minor": 5
}
"##;
    fs::write(root.join("analysis.ipynb"), notebook).unwrap();

    let mut db = TestProject::in_memory_db();
    full_index(&mut db, root, None, None, None).expect("index failed");

    let lang: String = db
        .query_row(
            "SELECT language FROM files WHERE path LIKE '%analysis.ipynb'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(lang, "jupyter");

    let py_syms: Vec<String> = db
        .prepare(
            "SELECT s.name FROM symbols s
             JOIN files f ON f.id = s.file_id
             WHERE f.path LIKE '%analysis.ipynb'
               AND s.origin_language = 'python'",
        )
        .unwrap()
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .flatten()
        .collect();
    assert!(
        py_syms.iter().any(|n| n == "compute"),
        "expected Python 'compute' from .ipynb code cell, got {py_syms:?}"
    );
}

#[test]
fn ipynb_magic_lines_do_not_break_python_parser() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    let notebook = r##"{
 "cells": [
  {"cell_type": "code", "source": "!pip install numpy\n%timeit do_thing()\ndef my_func(x):\n    return x\n", "metadata": {}}
 ],
 "metadata": {"kernelspec": {"language": "python"}},
 "nbformat": 4
}
"##;
    fs::write(root.join("magic.ipynb"), notebook).unwrap();

    let mut db = TestProject::in_memory_db();
    full_index(&mut db, root, None, None, None).expect("index failed");

    let my_func: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbols s
             JOIN files f ON f.id = s.file_id
             WHERE f.path LIKE '%magic.ipynb'
               AND s.origin_language = 'python'
               AND s.name = 'my_func'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        my_func, 1,
        "expected 'my_func' defined AFTER magic lines to still be extracted"
    );
}
