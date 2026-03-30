    use super::*;

    // -----------------------------------------------------------------------
    // Package + function declarations
    // -----------------------------------------------------------------------

    #[test]
    fn package_prefix_qualifies_function() {
        let source = r#"package myapp

func Hello() string {
    return "hi"
}
"#;
        let r = extract(source);
        let sym = r.symbols.iter().find(|s| s.name == "Hello").expect("no Hello");
        assert_eq!(sym.qualified_name, "myapp.Hello");
        assert_eq!(sym.kind, SymbolKind::Function);
        assert_eq!(sym.visibility, Some(Visibility::Public));
    }

    #[test]
    fn unexported_function_is_private() {
        let source = r#"package util

func helper() {}
"#;
        let r = extract(source);
        let sym = r.symbols.iter().find(|s| s.name == "helper").expect("no helper");
        assert_eq!(sym.visibility, Some(Visibility::Private));
        assert_eq!(sym.kind, SymbolKind::Function);
    }

    // -----------------------------------------------------------------------
    // Struct with fields
    // -----------------------------------------------------------------------

    #[test]
    fn struct_with_named_fields() {
        let source = r#"package model

type User struct {
    ID   int
    Name string
}
"#;
        let r = extract(source);

        let user = r.symbols.iter().find(|s| s.name == "User").expect("no User");
        assert_eq!(user.kind, SymbolKind::Struct);
        assert_eq!(user.qualified_name, "model.User");

        let id_field = r.symbols.iter().find(|s| s.name == "ID").expect("no ID field");
        assert_eq!(id_field.kind, SymbolKind::Field);
        assert_eq!(id_field.qualified_name, "model.User.ID");

        let name_field = r.symbols.iter().find(|s| s.name == "Name").expect("no Name field");
        assert_eq!(name_field.qualified_name, "model.User.Name");
    }

    // -----------------------------------------------------------------------
    // Interface with method specs
    // -----------------------------------------------------------------------

    #[test]
    fn interface_with_method_specs() {
        let source = r#"package io

type Writer interface {
    Write(p []byte) (n int, err error)
}
"#;
        let r = extract(source);

        let iface = r.symbols.iter().find(|s| s.name == "Writer").expect("no Writer");
        assert_eq!(iface.kind, SymbolKind::Interface);

        let method = r.symbols.iter().find(|s| s.name == "Write").expect("no Write");
        assert_eq!(method.kind, SymbolKind::Method);
        assert_eq!(method.qualified_name, "io.Writer.Write");
    }

    // -----------------------------------------------------------------------
    // Method with receiver
    // -----------------------------------------------------------------------

    #[test]
    fn method_with_value_receiver_qualified_name() {
        let source = r#"package geom

type Point struct {
    X, Y float64
}

func (p Point) String() string {
    return ""
}
"#;
        let r = extract(source);
        let method = r.symbols.iter().find(|s| s.name == "String").expect("no String");
        assert_eq!(method.kind, SymbolKind::Method);
        assert_eq!(method.qualified_name, "geom.Point.String");
    }

    #[test]
    fn method_with_pointer_receiver_strips_star() {
        let source = r#"package srv

type Server struct{}

func (s *Server) HandleRequest() {}
"#;
        let r = extract(source);
        let method = r
            .symbols
            .iter()
            .find(|s| s.name == "HandleRequest")
            .expect("no HandleRequest");
        assert_eq!(method.qualified_name, "srv.Server.HandleRequest");
        assert_eq!(method.kind, SymbolKind::Method);
    }

    // -----------------------------------------------------------------------
    // Imports
    // -----------------------------------------------------------------------

    #[test]
    fn single_import_produces_imports_ref() {
        let source = r#"package main

import "fmt"
"#;
        let r = extract(source);
        let imports: Vec<_> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Imports)
            .collect();
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].target_name, "fmt");
        assert_eq!(imports[0].module.as_deref(), Some("fmt"));
    }

    #[test]
    fn grouped_imports_produce_multiple_refs() {
        let source = r#"package main

import (
    "fmt"
    "os"
    "github.com/user/repo/pkg"
)
"#;
        let r = extract(source);
        let import_names: Vec<&str> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Imports)
            .map(|r| r.target_name.as_str())
            .collect();
        assert!(import_names.contains(&"fmt"), "missing fmt: {import_names:?}");
        assert!(import_names.contains(&"os"), "missing os: {import_names:?}");
        assert!(import_names.contains(&"pkg"), "missing pkg: {import_names:?}");
    }

    #[test]
    fn import_last_segment_is_target_name() {
        let source = r#"package main

import "github.com/user/repo/mypkg"
"#;
        let r = extract(source);
        let imp = r
            .refs
            .iter()
            .find(|r| r.kind == EdgeKind::Imports)
            .expect("no import ref");
        assert_eq!(imp.target_name, "mypkg");
        assert_eq!(imp.module.as_deref(), Some("github.com/user/repo/mypkg"));
    }

    // -----------------------------------------------------------------------
    // Call expressions
    // -----------------------------------------------------------------------

    #[test]
    fn call_expressions_produce_calls_edges() {
        let source = r#"package main

func run() {
    foo()
    bar.Baz()
}
"#;
        let r = extract(source);
        let call_names: Vec<&str> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Calls)
            .map(|r| r.target_name.as_str())
            .collect();
        assert!(call_names.contains(&"foo"), "missing foo: {call_names:?}");
        assert!(call_names.contains(&"Baz"), "missing Baz: {call_names:?}");
    }

    // -----------------------------------------------------------------------
    // Composite literals
    // -----------------------------------------------------------------------

    #[test]
    fn composite_literal_produces_instantiates_edge() {
        let source = r#"package main

func build() {
    u := User{Name: "Alice"}
    _ = u
}
"#;
        let r = extract(source);
        let inst: Vec<_> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Instantiates)
            .collect();
        assert!(!inst.is_empty(), "expected at least one Instantiates ref");
        assert_eq!(inst[0].target_name, "User");
    }

    // -----------------------------------------------------------------------
    // Embedded struct fields (Inherits edge)
    // -----------------------------------------------------------------------

    #[test]
    fn embedded_struct_field_produces_inherits_edge() {
        let source = r#"package zoo

type Animal struct {
    Name string
}

type Dog struct {
    Animal
    Breed string
}
"#;
        let r = extract(source);
        let inherits: Vec<_> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Inherits)
            .collect();
        assert_eq!(inherits.len(), 1, "expected 1 Inherits ref, got {}", inherits.len());
        assert_eq!(inherits[0].target_name, "Animal");
    }

    #[test]
    fn embedded_pointer_field_strips_star() {
        let source = r#"package base

type Base struct{}

type Child struct {
    *Base
}
"#;
        let r = extract(source);
        let inherits: Vec<_> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Inherits)
            .collect();
        assert!(!inherits.is_empty(), "expected Inherits ref");
        assert_eq!(inherits[0].target_name, "Base");
    }

    // -----------------------------------------------------------------------
    // Visibility
    // -----------------------------------------------------------------------

    #[test]
    fn visibility_uppercase_public_lowercase_private() {
        let source = r#"package p

type PublicType struct{}
type privateType struct{}
"#;
        let r = extract(source);
        let pub_sym = r.symbols.iter().find(|s| s.name == "PublicType").unwrap();
        let priv_sym = r.symbols.iter().find(|s| s.name == "privateType").unwrap();
        assert_eq!(pub_sym.visibility, Some(Visibility::Public));
        assert_eq!(priv_sym.visibility, Some(Visibility::Private));
    }

    // -----------------------------------------------------------------------
    // Test function detection
    // -----------------------------------------------------------------------

    #[test]
    fn test_function_gets_test_kind() {
        let source = r#"package mytest

import "testing"

func TestConnect(t *testing.T) {
    _ = t
}

func BenchmarkRun(b *testing.B) {
    _ = b
}

func ExampleFoo() {}
"#;
        let r = extract(source);

        let tc = r.symbols.iter().find(|s| s.name == "TestConnect").unwrap();
        assert_eq!(tc.kind, SymbolKind::Test);

        let bench = r.symbols.iter().find(|s| s.name == "BenchmarkRun").unwrap();
        assert_eq!(bench.kind, SymbolKind::Test);

        let example = r.symbols.iter().find(|s| s.name == "ExampleFoo").unwrap();
        assert_eq!(example.kind, SymbolKind::Test);
    }

    // -----------------------------------------------------------------------
    // Doc comments
    // -----------------------------------------------------------------------

    #[test]
    fn doc_comment_attached_to_function() {
        let source = r#"package doc

// Hello greets the caller.
// It returns a greeting string.
func Hello() string {
    return "hi"
}
"#;
        let r = extract(source);
        let sym = r.symbols.iter().find(|s| s.name == "Hello").unwrap();
        let doc = sym.doc_comment.as_deref().unwrap_or("");
        assert!(doc.contains("Hello greets"), "doc_comment was: {doc:?}");
    }

    // -----------------------------------------------------------------------
    // Type alias
    // -----------------------------------------------------------------------

    #[test]
    fn type_alias_produces_type_alias_kind() {
        let source = r#"package alias

type MyInt int
type StringSlice = []string
"#;
        let r = extract(source);
        let my_int = r.symbols.iter().find(|s| s.name == "MyInt").unwrap();
        assert_eq!(my_int.kind, SymbolKind::TypeAlias);

        // `type StringSlice = []string` uses Go's alias syntax (=).
        // tree-sitter-go may represent this as a `type_alias` node rather than `type_spec`.
        // If extracted, it should be TypeAlias.
        if let Some(ss) = r.symbols.iter().find(|s| s.name == "StringSlice") {
            assert_eq!(ss.kind, SymbolKind::TypeAlias);
        }
    }

    // -----------------------------------------------------------------------
    // Const / var
    // -----------------------------------------------------------------------

    #[test]
    fn const_declaration_produces_variable_symbols() {
        let source = r#"package cfg

const MaxRetries = 3
const (
    DefaultTimeout = 30
    DefaultPort    = 8080
)
"#;
        let r = extract(source);
        let names: Vec<&str> = r
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Variable)
            .map(|s| s.name.as_str())
            .collect();
        assert!(names.contains(&"MaxRetries"), "missing MaxRetries: {names:?}");
        assert!(names.contains(&"DefaultTimeout"), "missing DefaultTimeout: {names:?}");
        assert!(names.contains(&"DefaultPort"), "missing DefaultPort: {names:?}");
    }

    // -----------------------------------------------------------------------
    // Error tolerance
    // -----------------------------------------------------------------------

    #[test]
    fn handles_parse_errors_gracefully() {
        let source = "package broken\n\nfunc (  {\n";
        let r = extract(source);
        // Must not panic; partial results and has_errors=true are acceptable.
        let _ = &r.symbols;
        let _ = r.has_errors;
    }

    // -----------------------------------------------------------------------
    // Struct tags
    // -----------------------------------------------------------------------

    #[test]
    fn struct_tags_stored_in_field_doc_comment() {
        let source = r#"package model

type User struct {
    Name  string `json:"name" db:"user_name" validate:"required"`
    Email string `json:"email,omitempty" db:"email"`
    Age   int    `json:"age" db:"age"`
}
"#;
        let r = extract(source);

        let name_field = r
            .symbols
            .iter()
            .find(|s| s.name == "Name" && s.kind == SymbolKind::Field)
            .expect("no Name field");
        let doc = name_field.doc_comment.as_deref().unwrap_or("");
        assert!(
            doc.contains("json=\"name\""),
            "expected json tag, got: {doc:?}"
        );
        assert!(
            doc.contains("db=\"user_name\""),
            "expected db tag, got: {doc:?}"
        );
        assert!(
            doc.contains("validate=\"required\""),
            "expected validate tag, got: {doc:?}"
        );
    }

    #[test]
    fn struct_tag_with_omitempty_option() {
        let source = r#"package model

type Response struct {
    Message string `json:"message,omitempty"`
}
"#;
        let r = extract(source);
        let field = r
            .symbols
            .iter()
            .find(|s| s.name == "Message" && s.kind == SymbolKind::Field)
            .expect("no Message field");
        let doc = field.doc_comment.as_deref().unwrap_or("");
        assert!(
            doc.contains("json=\"message,omitempty\""),
            "expected omitempty in tag value, got: {doc:?}"
        );
    }

    #[test]
    fn struct_field_without_tags_has_no_doc_comment() {
        let source = r#"package model

type Point struct {
    X float64
    Y float64
}
"#;
        let r = extract(source);
        for sym in r.symbols.iter().filter(|s| s.kind == SymbolKind::Field) {
            assert!(
                sym.doc_comment.is_none(),
                "field {} should have no doc_comment, got: {:?}",
                sym.name,
                sym.doc_comment
            );
        }
    }

    #[test]
    fn struct_tags_multiple_fields_all_tagged() {
        let source = r#"package api

type Item struct {
    ID    int    `json:"id" gorm:"primaryKey"`
    Title string `json:"title" gorm:"column:title"`
}
"#;
        let r = extract(source);

        let id_field = r
            .symbols
            .iter()
            .find(|s| s.name == "ID" && s.kind == SymbolKind::Field)
            .expect("no ID field");
        let id_doc = id_field.doc_comment.as_deref().unwrap_or("");
        assert!(id_doc.contains("gorm=\"primaryKey\""), "got: {id_doc:?}");

        let title_field = r
            .symbols
            .iter()
            .find(|s| s.name == "Title" && s.kind == SymbolKind::Field)
            .expect("no Title field");
        let title_doc = title_field.doc_comment.as_deref().unwrap_or("");
        assert!(title_doc.contains("gorm=\"column:title\""), "got: {title_doc:?}");
    }

    // -----------------------------------------------------------------------
    // defer / go statement call extraction
    // -----------------------------------------------------------------------

    #[test]
    fn defer_statement_call_is_extracted() {
        let source = r#"package server

func handleConn(conn net.Conn) {
    defer conn.Close()
    conn.Read(nil)
}
"#;
        let r = extract(source);
        let call_names: Vec<&str> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Calls)
            .map(|r| r.target_name.as_str())
            .collect();
        assert!(
            call_names.contains(&"Close"),
            "expected Close call from defer, got: {call_names:?}"
        );
    }

    #[test]
    fn go_statement_call_is_extracted() {
        let source = r#"package worker

func start(h Handler) {
    go h.Process()
}
"#;
        let r = extract(source);
        let call_names: Vec<&str> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Calls)
            .map(|r| r.target_name.as_str())
            .collect();
        assert!(
            call_names.contains(&"Process"),
            "expected Process call from go statement, got: {call_names:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Type narrowing — type assertions and type switches
    // -----------------------------------------------------------------------

    #[test]
    fn type_assertion_emits_type_ref() {
        let source = r#"package app

func handle(x interface{}) {
    if admin, ok := x.(*Admin); ok {
        _ = admin
    }
}
"#;
        let r = extract(source);
        let type_refs: Vec<_> = r.refs.iter().filter(|r| r.kind == EdgeKind::TypeRef).collect();
        assert!(
            type_refs.iter().any(|r| r.target_name == "Admin"),
            "expected TypeRef to Admin, got: {type_refs:?}"
        );
    }

    #[test]
    fn type_switch_emits_type_refs() {
        let source = r#"package app

func process(x interface{}) {
    switch v := x.(type) {
    case *Admin:
        _ = v
    case *User:
        _ = v
    }
}
"#;
        let r = extract(source);
        let type_refs: Vec<_> = r.refs.iter().filter(|r| r.kind == EdgeKind::TypeRef).collect();
        let names: Vec<&str> = type_refs.iter().map(|r| r.target_name.as_str()).collect();
        assert!(names.contains(&"Admin"), "expected TypeRef to Admin, got: {names:?}");
        assert!(names.contains(&"User"), "expected TypeRef to User, got: {names:?}");
    }


