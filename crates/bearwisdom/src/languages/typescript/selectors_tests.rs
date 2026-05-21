//! Tests for the Angular selector extractor.

use super::extract_component_selectors;
use crate::types::{ExtractedSymbol, SymbolKind, Visibility};

fn fake_class(name: &str, qname: &str) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: qname.to_string(),
        kind: SymbolKind::Class,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index: None,
        byte_offset: 0,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    }
}

#[test]
fn at_directive_decorator_captures_selector() {
    let src = r#"
@Directive({ selector: '[appHighlight]' })
export class HighlightDirective {}
"#;
    let symbols = vec![fake_class("HighlightDirective", "HighlightDirective")];
    let pairs = extract_component_selectors(src, &symbols);
    assert!(
        pairs.iter().any(|(s, c)| s == "appHighlight" && c == "HighlightDirective"),
        "expected appHighlight -> HighlightDirective in {pairs:?}"
    );
}

#[test]
fn ng_declare_directive_call_captures_selector() {
    let src = "
class HighlightDirective {}
HighlightDirective.\u{0275}dir = i0.\u{0275}\u{0275}ngDeclareDirective({
    minVersion: \"14.0.0\",
    type: HighlightDirective,
    selector: \"[cElementRef]\",
    standalone: true,
});
";
    let symbols = vec![fake_class("HighlightDirective", "HighlightDirective")];
    let pairs = extract_component_selectors(src, &symbols);
    assert!(
        pairs.iter().any(|(s, c)| s == "cElementRef" && c == "HighlightDirective"),
        "expected cElementRef -> HighlightDirective in {pairs:?}"
    );
}

#[test]
fn ng_declare_component_call_captures_selector() {
    let src = "
class ContainerComponent {}
ContainerComponent.\u{0275}cmp = i0.\u{0275}\u{0275}ngDeclareComponent({
    type: ContainerComponent,
    selector: \"c-container\",
});
";
    let symbols = vec![fake_class("ContainerComponent", "ContainerComponent")];
    let pairs = extract_component_selectors(src, &symbols);
    assert!(
        pairs.iter().any(|(s, c)| s == "c-container" && c == "ContainerComponent"),
        "expected c-container -> ContainerComponent in {pairs:?}"
    );
}

#[test]
fn dts_directive_declaration_static_field_captures_selector() {
    let src = "
declare class ElementRefDirective {
    elementRef: any;
    static \u{0275}fac: any;
    static \u{0275}dir: _angular_core.\u{0275}\u{0275}DirectiveDeclaration<ElementRefDirective, \"[cElementRef]\", [\"cElementRef\"], {}, {}, never, never, true, never>;
}
";
    let symbols = vec![fake_class("ElementRefDirective", "ElementRefDirective")];
    let pairs = extract_component_selectors(src, &symbols);
    assert!(
        pairs.iter().any(|(s, c)| s == "cElementRef" && c == "ElementRefDirective"),
        "expected cElementRef -> ElementRefDirective in {pairs:?}"
    );
}

#[test]
fn dts_component_declaration_static_field_captures_selector() {
    let src = "
declare class AccordionComponent {
    static \u{0275}cmp: _angular_core.\u{0275}\u{0275}ComponentDeclaration<AccordionComponent, \"c-accordion\", [\"cAccordionItem\"], {}, {}, never, [\"*\"], true, never>;
}
";
    let symbols = vec![fake_class("AccordionComponent", "AccordionComponent")];
    let pairs = extract_component_selectors(src, &symbols);
    assert!(
        pairs.iter().any(|(s, c)| s == "c-accordion" && c == "AccordionComponent"),
        "expected c-accordion -> AccordionComponent in {pairs:?}"
    );
}
