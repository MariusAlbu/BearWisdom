use super::*;
use tree_sitter::{InputEdit, Language, Parser, Point, Query, Tree};

fn parser(language: LanguageFn) -> Parser {
    let mut parser = Parser::new();
    parser.set_language(&language.into()).unwrap();
    parser
}

fn clean(parser: &mut Parser, source: &str) -> Tree {
    let tree = parser.parse(source, None).unwrap();
    assert!(
        !tree.root_node().has_error(),
        "{source}\n{}",
        tree.root_node().to_sexp()
    );
    tree
}

#[test]
fn both_dialects_preserve_queries_and_valid_neighboring_forms() {
    let sources = [
        "export {}; declare global { interface Catalog { read(): string; } }",
        "declare module 'provider' { global { interface Catalog { read(): string; } } }",
        "declare namespace Env { interface Box<T> { get(): T; } }",
        "interface Box { [Symbol.iterator](): Iterator<string>; readonly [key: string]: unknown; }",
        "type Keys = { readonly [K in keyof readonly any[]]?: boolean };",
        "type Stream<T> = import('provider').nested.Stream<T>;",
        "type Ctor = typeof import('provider').Factory; type Member = import('provider').Stream['read'];",
        "const global = () => {}; global(); const o = { global() { return 1; } }; global: { break global; }",
        "const result = import('provider').then(value => value.run()); const item = factory<string>();",
    ];
    for language in [LANGUAGE_TYPESCRIPT, LANGUAGE_TSX] {
        let grammar: Language = language.into();
        for query in [HIGHLIGHTS_QUERY, LOCALS_QUERY, TAGS_QUERY] {
            Query::new(&grammar, query).unwrap();
        }
        let mut parser = parser(language);
        for source in sources {
            clean(&mut parser, source);
        }
    }
}

#[test]
fn bigint_type_keyword_does_not_become_a_user_defined_type_head() {
    for language in [LANGUAGE_TYPESCRIPT, LANGUAGE_TSX] {
        let mut parser = parser(language);
        let source =
            "type Big = bigint; const bigint = 1n; const value = { bigint }; type Named = BigInt;";
        let tree = clean(&mut parser, source);
        let root = tree.root_node();
        let atom = root
            .named_child(0)
            .unwrap()
            .child_by_field_name("value")
            .unwrap();
        assert_eq!(atom.kind(), "predefined_type");
        assert_eq!(atom.utf8_text(source.as_bytes()).unwrap(), "bigint");
        let binding = root
            .named_child(1)
            .unwrap()
            .named_child(0)
            .unwrap()
            .child_by_field_name("name")
            .unwrap();
        assert_eq!(binding.kind(), "identifier");
        let nominal = root
            .named_child(3)
            .unwrap()
            .child_by_field_name("value")
            .unwrap();
        assert_eq!(nominal.kind(), "type_identifier");
    }
}

#[test]
fn type_only_wildcard_exports_preserve_source_fields_and_incremental_edits() {
    for language in [LANGUAGE_TYPESCRIPT, LANGUAGE_TSX] {
        let mut parser = parser(language);
        for clause in ["*", "* as API"] {
            let source = format!("export type {clause} from './provider';");
            let mut tree = clean(&mut parser, &source);
            let statement = tree.root_node().named_child(0).unwrap();
            assert_eq!(statement.kind(), "export_statement");
            let provider = statement.child_by_field_name("source").unwrap();
            assert_eq!(
                provider.utf8_text(source.as_bytes()).unwrap(),
                "'./provider'"
            );
            assert_eq!(provider.start_byte(), source.find("'./provider'").unwrap());
            let mut cursor = statement.walk();
            assert!(statement
                .children(&mut cursor)
                .any(|node| node.kind() == "type"));
            drop(cursor);
            let start = source.find("provider").unwrap();
            let updated = source.replace("provider", "changed");
            tree.edit(&InputEdit {
                start_byte: start,
                old_end_byte: start + 8,
                new_end_byte: start + 7,
                start_position: Point::new(0, start),
                old_end_position: Point::new(0, start + 8),
                new_end_position: Point::new(0, start + 7),
            });
            let incremental = parser.parse(&updated, Some(&tree)).unwrap();
            let fresh = clean(&mut parser, &updated);
            assert!(!incremental.root_node().has_error());
            assert_eq!(
                incremental.root_node().to_sexp(),
                fresh.root_node().to_sexp()
            );
        }
        for invalid in [
            "export type *;",
            "export type * as API;",
            "export type * from ;",
        ] {
            assert!(
                parser.parse(invalid, None).unwrap().root_node().has_error(),
                "{invalid}"
            );
        }
    }
}

#[test]
fn global_augmentation_shape_and_source_coordinates_are_preserved() {
    for language in [LANGUAGE_TYPESCRIPT, LANGUAGE_TSX] {
        let mut parser = parser(language);
        for prefix in ["global", "declare global"] {
            let source =
                format!("declare module 'provider' {{ {prefix} {{ interface Catalog {{}} }} }}");
            let tree = clean(&mut parser, &source);
            let module = tree
                .root_node()
                .named_child(0)
                .unwrap()
                .named_child(0)
                .unwrap();
            let global = module
                .child_by_field_name("body")
                .unwrap()
                .named_child(0)
                .unwrap();
            assert_eq!(global.kind(), "ambient_declaration");
            assert_eq!(global.start_byte(), source.find(prefix).unwrap());
            assert_eq!(
                global.named_child_count(),
                1,
                "explicit declare must not add a nested wrapper"
            );
            assert_eq!(global.named_child(0).unwrap().kind(), "statement_block");
        }
    }
}

#[test]
fn generic_import_type_has_a_structural_head_and_argument_list() {
    for language in [LANGUAGE_TYPESCRIPT, LANGUAGE_TSX] {
        let mut parser = parser(language);
        let source = "type Pair<T, U> = import('provider').Pair<T, U>;";
        let tree = clean(&mut parser, source);
        let value = tree
            .root_node()
            .named_child(0)
            .unwrap()
            .child_by_field_name("value")
            .unwrap();
        assert_eq!(value.kind(), "generic_type");
        let head = value.child_by_field_name("name").unwrap();
        assert_eq!(head.kind(), "member_expression");
        assert_eq!(
            head.utf8_text(source.as_bytes()).unwrap(),
            "import('provider').Pair"
        );
        let args = value.child_by_field_name("type_arguments").unwrap();
        assert_eq!(args.named_child_count(), 2);
        assert_eq!(args.utf8_text(source.as_bytes()).unwrap(), "<T, U>");
    }
}

#[test]
fn import_postfixes_and_keyof_readonly_preserve_operator_ownership() {
    for language in [LANGUAGE_TYPESCRIPT, LANGUAGE_TSX] {
        let mut parser = parser(language);
        let tree = clean(&mut parser, "type Keys = keyof readonly string[] | number;");
        let value = tree
            .root_node()
            .named_child(0)
            .unwrap()
            .child_by_field_name("value")
            .unwrap();
        assert_eq!(value.kind(), "union_type", "{}", value.to_sexp());
        let keys = value.named_child(0).unwrap();
        assert_eq!(keys.kind(), "index_type_query");
        assert_eq!(keys.named_child(0).unwrap().kind(), "readonly_type");
        assert_eq!(
            keys.named_child(0).unwrap().named_child(0).unwrap().kind(),
            "array_type"
        );
        let tree = clean(
            &mut parser,
            "type Item = import('provider').Stream<string>[number][];",
        );
        let value = tree
            .root_node()
            .named_child(0)
            .unwrap()
            .child_by_field_name("value")
            .unwrap();
        assert_eq!(value.kind(), "array_type");
        let lookup = value.named_child(0).unwrap();
        assert_eq!(lookup.kind(), "lookup_type");
        assert_eq!(lookup.named_child(0).unwrap().kind(), "generic_type");
    }
}

#[test]
fn malformed_provider_syntax_is_not_silently_accepted() {
    for language in [LANGUAGE_TYPESCRIPT, LANGUAGE_TSX] {
        let mut parser = parser(language);
        for source in [
            "declare module 'provider' { global { interface Catalog { broken(:; } } }",
            "type Keys = { [K in keyof readonly ]: boolean };",
            "type Stream<T> = import('provider').Stream<T;",
        ] {
            assert!(
                parser.parse(source, None).unwrap().root_node().has_error(),
                "{source}"
            );
        }
    }
}

#[test]
fn provider_incremental_reparse_equals_fresh_tree() {
    for language in [LANGUAGE_TYPESCRIPT, LANGUAGE_TSX] {
        let mut parser = parser(language);
        let original = "declare module 'provider' { global { type Alias<T> = import('provider').Stream<T>; } }";
        let mut tree = clean(&mut parser, original);
        let start = original.find("Stream").unwrap();
        let updated = original.replace("Stream", "Channel");
        tree.edit(&InputEdit {
            start_byte: start,
            old_end_byte: start + 6,
            new_end_byte: start + 7,
            start_position: Point::new(0, start),
            old_end_position: Point::new(0, start + 6),
            new_end_position: Point::new(0, start + 7),
        });
        let incremental = parser.parse(&updated, Some(&tree)).unwrap();
        let fresh = clean(&mut parser, &updated);
        assert!(!incremental.root_node().has_error());
        assert_eq!(
            incremental.root_node().to_sexp(),
            fresh.root_node().to_sexp()
        );
        assert_eq!(incremental.root_node().range(), fresh.root_node().range());
    }
}

#[test]
fn dialect_specific_expression_syntax_remains_distinct() {
    clean(
        &mut parser(LANGUAGE_TYPESCRIPT),
        "const value = <string>input;",
    );
    clean(
        &mut parser(LANGUAGE_TSX),
        "const view = <Panel<string> title='ok'>{value}</Panel>;",
    );
    assert!(parser(LANGUAGE_TYPESCRIPT)
        .parse("const view = <Panel />;", None)
        .unwrap()
        .root_node()
        .has_error());
}
