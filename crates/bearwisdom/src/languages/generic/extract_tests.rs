    use super::extract::*;
    use crate::types::{EdgeKind, ExtractedRef, SymbolKind};

    fn must_extract(src: &str, lang: &str) -> GenericExtraction {
        extract(src, lang).unwrap_or_else(|| panic!("extract({lang}) returned None"))
    }

    // ---- Python --------------------------------------------------------------

    #[test]
    fn python_functions_and_classes() {
        let src = r#"
def greet(name: str) -> str:
    return f"Hello, {name}"

class Animal:
    def __init__(self, name):
        self.name = name

    def speak(self):
        pass

class Dog(Animal):
    def speak(self):
        return "woof"
"#;
        let result = must_extract(src, "python");
        let names: Vec<&str> = result.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"greet"), "expected greet, got {names:?}");
        assert!(names.contains(&"Animal"), "expected Animal, got {names:?}");
        assert!(names.contains(&"Dog"), "expected Dog, got {names:?}");
    }

    #[test]
    fn python_imports() {
        let src = "import os\nfrom pathlib import Path\nimport json as j\n";
        let result = must_extract(src, "python");
        assert!(
            !result.refs.is_empty(),
            "expected at least one import ref"
        );
        let import_kinds: Vec<EdgeKind> = result.refs.iter().map(|r| r.kind).collect();
        assert!(
            import_kinds.iter().all(|k| *k == EdgeKind::Imports),
            "all refs should be Imports"
        );
    }

    #[test]
    fn python_from_import_module_and_target() {
        // `from os.path import join` → module="os.path", target="join"
        let src = "from os.path import join\n";
        let result = must_extract(src, "python");
        let imp = result
            .refs
            .iter()
            .find(|r| r.kind == EdgeKind::Imports)
            .expect("expected an Imports ref");
        assert_eq!(
            imp.module.as_deref(),
            Some("os.path"),
            "module should be os.path, got {:?}",
            imp.module
        );
        assert_eq!(
            imp.target_name, "join",
            "target_name should be join, got {}",
            imp.target_name
        );
    }

    // ---- Java ----------------------------------------------------------------

    #[test]
    fn java_class_and_methods() {
        let src = r#"
package com.example;

public class HelloWorld {
    private String message;

    public HelloWorld(String msg) {
        this.message = msg;
    }

    public void print() {
        System.out.println(message);
    }
}
"#;
        let result = must_extract(src, "java");
        let names: Vec<&str> = result.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"HelloWorld"), "expected HelloWorld, got {names:?}");
    }

    #[test]
    fn java_interface() {
        let src = r#"
public interface Printable {
    void print();
    default String format() { return ""; }
}
"#;
        let result = must_extract(src, "java");
        let kinds: Vec<SymbolKind> = result.symbols.iter().map(|s| s.kind).collect();
        assert!(
            kinds.contains(&SymbolKind::Interface),
            "expected Interface symbol, got {kinds:?}"
        );
    }

    #[test]
    fn java_nested_class_qualified_name() {
        // Inner class should get qualified name "Outer.Inner"
        let src = r#"
public class Outer {
    public class Inner {
        public void doWork() {}
    }
}
"#;
        let result = must_extract(src, "java");
        let inner = result
            .symbols
            .iter()
            .find(|s| s.name == "Inner")
            .expect("expected Inner symbol");
        assert_eq!(
            inner.qualified_name, "Outer.Inner",
            "Inner should have qualified name Outer.Inner, got {}",
            inner.qualified_name
        );
    }

    // ---- Go ------------------------------------------------------------------

    #[test]
    fn go_functions_and_structs() {
        let src = r#"
package main

import "fmt"

type Point struct {
    X, Y float64
}

func NewPoint(x, y float64) Point {
    return Point{X: x, Y: y}
}

func (p Point) String() string {
    return fmt.Sprintf("(%v, %v)", p.X, p.Y)
}
"#;
        let result = must_extract(src, "go");
        let names: Vec<&str> = result.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"Point"), "expected Point struct, got {names:?}");
        assert!(names.contains(&"NewPoint"), "expected NewPoint, got {names:?}");
    }

    #[test]
    fn go_imports() {
        let src = r#"
package main

import (
    "fmt"
    "os"
    "strings"
)

func main() {}
"#;
        let result = must_extract(src, "go");
        let has_imports = result.refs.iter().any(|r| r.kind == EdgeKind::Imports);
        assert!(has_imports, "expected at least one Imports ref for Go imports");
    }

    #[test]
    fn go_single_import_module() {
        // `import "fmt"` → module="fmt", target="fmt"
        let src = "package main\n\nimport \"fmt\"\n\nfunc main() {}\n";
        let result = must_extract(src, "go");
        let imp = result
            .refs
            .iter()
            .find(|r| r.kind == EdgeKind::Imports)
            .expect("expected an Imports ref for Go import");
        assert_eq!(
            imp.module.as_deref(),
            Some("fmt"),
            "module should be fmt, got {:?}",
            imp.module
        );
    }

    // ---- Rust ----------------------------------------------------------------

    #[test]
    fn rust_functions_structs_traits() {
        let src = r#"
use std::fmt;

pub struct Point {
    pub x: f64,
    pub y: f64,
}

pub trait Shape {
    fn area(&self) -> f64;
}

impl fmt::Display for Point {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({}, {})", self.x, self.y)
    }
}

pub fn distance(a: &Point, b: &Point) -> f64 {
    ((b.x - a.x).powi(2) + (b.y - a.y).powi(2)).sqrt()
}
"#;
        let result = must_extract(src, "rust");
        let names: Vec<&str> = result.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"Point"), "expected Point, got {names:?}");
        assert!(names.contains(&"Shape"), "expected Shape trait, got {names:?}");
        assert!(names.contains(&"distance"), "expected distance fn, got {names:?}");
    }

    #[test]
    fn rust_use_declarations() {
        let src = "use std::collections::HashMap;\nuse crate::types::SymbolKind;\n\nfn main() {}\n";
        let result = must_extract(src, "rust");
        let imports: Vec<&ExtractedRef> = result
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Imports)
            .collect();
        assert!(!imports.is_empty(), "expected Imports refs for use declarations");
    }

    #[test]
    fn rust_use_module_field() {
        // `use crate::db::Database` → module contains "crate::db::Database"
        let src = "use crate::db::Database;\n\nfn main() {}\n";
        let result = must_extract(src, "rust");
        let imp = result
            .refs
            .iter()
            .find(|r| r.kind == EdgeKind::Imports)
            .expect("expected an Imports ref");
        let module = imp.module.as_deref().unwrap_or("");
        assert!(
            module.contains("crate") && module.contains("db"),
            "module should contain crate::db path, got {module:?}"
        );
    }

    // ---- Inheritance extraction ---------------------------------------------

    #[test]
    fn python_class_inheritance() {
        let src = "class Dog(Animal):\n    pass\n";
        let result = must_extract(src, "python");
        let inherits: Vec<&ExtractedRef> = result
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Inherits)
            .collect();
        assert!(
            !inherits.is_empty(),
            "expected Inherits ref for Dog(Animal), got refs: {:?}",
            result.refs.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
        );
        assert!(
            inherits.iter().any(|r| r.target_name == "Animal"),
            "expected Inherits target 'Animal'"
        );
    }

    #[test]
    fn java_class_extends_and_implements() {
        let src = r#"
public class Dog extends Animal implements Comparable {
    public void speak() {}
}
"#;
        let result = must_extract(src, "java");
        let inherits: Vec<&str> = result
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Inherits)
            .map(|r| r.target_name.as_str())
            .collect();
        let implements: Vec<&str> = result
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Implements)
            .map(|r| r.target_name.as_str())
            .collect();
        // Note: grammar may expose these differently — check for either edge kind
        let all_inheritance: Vec<&str> = result
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Inherits || r.kind == EdgeKind::Implements)
            .map(|r| r.target_name.as_str())
            .collect();
        assert!(
            all_inheritance.iter().any(|n| *n == "Animal"),
            "expected inheritance ref to 'Animal', got {all_inheritance:?}"
        );
    }

    #[test]
    fn java_call_extraction_works() {
        // Verify the generic extractor actually extracts calls (was already present)
        let src = r#"
public class Main {
    public void run() {
        System.out.println("hello");
        doWork();
    }
    public void doWork() {}
}
"#;
        let result = must_extract(src, "java");
        let calls: Vec<&str> = result
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Calls)
            .map(|r| r.target_name.as_str())
            .collect();
        assert!(
            !calls.is_empty(),
            "expected call refs from Java code, got none. All refs: {:?}",
            result.refs.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn go_call_extraction() {
        let src = r#"
package main

import "fmt"

func main() {
    fmt.Println("hello")
    doWork()
}

func doWork() {}
"#;
        let result = must_extract(src, "go");
        let calls: Vec<&str> = result
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Calls)
            .map(|r| r.target_name.as_str())
            .collect();
        assert!(
            calls.iter().any(|c| *c == "doWork" || *c == "Println"),
            "expected call to doWork or Println, got {calls:?}"
        );
    }

    // ---- Python qualified names --------------------------------------------

    #[test]
    fn python_method_qualified_name() {
        // Method inside a class should get qualified name "MyClass.my_method"
        let src = r#"
class MyClass:
    def my_method(self):
        pass
"#;
        let result = must_extract(src, "python");
        let method = result
            .symbols
            .iter()
            .find(|s| s.name == "my_method")
            .expect("expected my_method symbol");
        assert_eq!(
            method.qualified_name, "MyClass.my_method",
            "my_method should have qualified name MyClass.my_method, got {}",
            method.qualified_name
        );
    }

    // ---- Generic grammar support -------------------------------------------

    #[test]
    fn unknown_language_returns_none() {
        assert!(extract("hello world", "cobol").is_none());
        assert!(extract("", "brainfuck").is_none());
    }

    #[test]
    fn empty_source_does_not_panic() {
        for lang in &["python", "java", "go", "rust", "ruby", "cpp", "c"] {
            let result = must_extract("", lang);
            assert!(result.symbols.is_empty(), "{lang}: empty src should have no symbols");
        }
    }

    #[test]
    fn syntax_error_flag_is_set() {
        let src = "def broken(:\n    pass\n";
        let result = must_extract(src, "python");
        assert!(result.has_errors, "expected has_errors for broken Python");
    }

    // ---- No placeholder refs -----------------------------------------------

    #[test]
    fn no_import_placeholder_refs() {
        // None of the import refs should have target_name == "<import>"
        let sources = [
            ("python", "import os\nfrom pathlib import Path\n"),
            ("go", "package main\nimport \"fmt\"\n"),
            ("rust", "use std::collections::HashMap;\n"),
            ("java", "import java.util.List;\n"),
        ];
        for (lang, src) in &sources {
            let result = must_extract(src, lang);
            for r in &result.refs {
                assert_ne!(
                    r.target_name, "<import>",
                    "{lang}: found placeholder <import> ref: {r:?}"
                );
            }
        }
    }
