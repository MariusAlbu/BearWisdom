// =============================================================================
// typescript/ambient_modules_tests — ambient declarations vs augmentations
// =============================================================================

use super::{collect_declared_modules, is_module_file};

fn parse(src: &str) -> tree_sitter::Tree {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
        .unwrap();
    parser.parse(src, None).unwrap()
}

#[test]
fn a_script_file_declares_the_modules_it_names() {
    let src = "declare module 'virtual:pwa' { export const x: number; }\ndeclare module 'my-shim';\ndeclare module Foo { }\nmodule 'bare' { }\n";
    let tree = parse(src);
    assert!(!is_module_file(tree.root_node()));
    assert_eq!(
        collect_declared_modules(tree.root_node(), src.as_bytes()),
        vec!["virtual:pwa", "my-shim", "bare"]
    );
}

#[test]
fn a_module_file_only_augments_and_declares_nothing() {
    for src in [
        "import type { A } from './a';\ndeclare module 'react' { interface ReactNode { extra?: boolean } }\n",
        "export type Server = 'edge';\ndeclare module 'react' { }\n",
        "export = React;\ndeclare module 'react/jsx-runtime' { }\n",
    ] {
        let tree = parse(src);
        assert!(is_module_file(tree.root_node()), "{src}");
        assert!(
            collect_declared_modules(tree.root_node(), src.as_bytes()).is_empty(),
            "{src}"
        );
    }
}
