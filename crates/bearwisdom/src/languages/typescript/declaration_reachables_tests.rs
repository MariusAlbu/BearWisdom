// =============================================================================
// typescript/declaration_reachables_tests — side-effect imports and NgModule
// declarations are reachable files
// =============================================================================

use super::reachables;

#[test]
fn a_declaration_file_reaches_its_relative_side_effect_imports_only() {
    let content = "import './chunks/global.d.B15mdLcR.js';\nimport 'vitest/globals';\nimport type { A } from './a';\nimport { b } from './b.js';\nexport { c } from './c';\nimport './chunks/global.d.B15mdLcR.js';\n";
    assert_eq!(
        reachables("node_modules/vitest/dist/index.d.ts", content),
        vec!["./chunks/global.d.B15mdLcR.js"],
        "named imports and re-exports are hops of their own; a bare side-effect import is a package"
    );
}

#[test]
fn a_source_file_reaches_nothing_this_way() {
    assert!(reachables("src/app.ts", "import './styles.css';\n").is_empty());
}

#[test]
fn an_ng_module_declaration_reaches_the_components_it_declares() {
    let content = "import * as i0 from '@angular/core';\nimport * as i1 from './button.component';\nimport * as i2 from './tooltip.directive';\nimport * as i3 from './shared.service';\nexport declare class UiModule {\n    static ɵmod: i0.ɵɵNgModuleDeclaration<UiModule, [typeof i1.ButtonComponent], [], []>;\n}\n";
    assert_eq!(
        reachables("node_modules/ui/lib/ui.module.d.ts", content),
        vec!["./button.component", "./tooltip.directive"]
    );
}
