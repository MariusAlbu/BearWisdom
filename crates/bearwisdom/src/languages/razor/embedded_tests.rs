use super::embedded::detect_regions;
use super::embedded_mask::mask_razor_expressions_in_script;
use crate::types::EmbeddedOrigin;

#[test]
fn at_brace_code_block() {
    let src = "<h1>Hello</h1>\n@{ var x = 1; var y = 2; }\n<p>done</p>";
    let regions = detect_regions(src);
    assert_eq!(regions.len(), 1);
    assert_eq!(regions[0].language_id, "csharp");
    assert_eq!(regions[0].origin, EmbeddedOrigin::RazorCode);
    assert!(regions[0].text.contains("var x = 1; var y = 2;"));
    assert!(regions[0].text.contains("class __RazorBody"));
    assert_eq!(regions[0].strip_scope_prefix.as_deref(), Some("__RazorBody"));
}

#[test]
fn at_brace_with_nested_braces() {
    let src = "@{ var o = new { A = 1, B = new { C = 2 } }; }";
    let regions = detect_regions(src);
    assert_eq!(regions.len(), 1);
    assert!(regions[0].text.contains("new { A = 1"));
    assert!(regions[0].text.contains("C = 2"));
}

#[test]
fn code_and_functions_blocks() {
    let src = "@code { int Count { get; set; } }\n@functions { void Do() {} }";
    let regions = detect_regions(src);
    assert_eq!(regions.len(), 2);
    assert!(regions[0].text.contains("Count"));
    assert!(regions[1].text.contains("Do"));
}

#[test]
fn at_paren_inline_expression() {
    let src = "<p>Name: @(user.Name)</p>";
    let regions = detect_regions(src);
    assert_eq!(regions.len(), 1);
    assert_eq!(regions[0].language_id, "csharp");
    assert!(regions[0].text.contains("user.Name"));
}

#[test]
fn script_block_default_is_javascript() {
    let src = "<script>function f() {}</script>";
    let regions = detect_regions(src);
    assert_eq!(regions.len(), 1);
    assert_eq!(regions[0].language_id, "javascript");
}

#[test]
fn script_block_with_lang_ts_is_typescript() {
    let src = "<script lang=\"ts\">const x: number = 1;</script>";
    let regions = detect_regions(src);
    assert_eq!(regions.len(), 1);
    assert_eq!(regions[0].language_id, "typescript");
}

#[test]
fn razor_expressions_in_script_are_masked() {
    // Server-side Razor identifiers inside <script> blocks must not leak
    // into the JS symbol graph as ghost refs.
    let src = r#"<script>
window.A = "@Config["X.Y"]";
window.B = [@Html.Raw(string.Join(",", xs))];
var c = @(total + 1);
var d = @@literal;
</script>"#;
    let regions = detect_regions(src);
    assert_eq!(regions.len(), 1);
    let text = &regions[0].text;
    // None of the Razor identifiers should survive.
    for ghost in &["Html", "Config", "Raw", "string.Join", "total"] {
        assert!(
            !text.contains(ghost),
            "razor identifier '{ghost}' leaked into masked script body: {text}"
        );
    }
    // The `@@` escape collapses to spaces so the literal `@` doesn't
    // re-trigger the JS extractor either.
    assert!(!text.contains("@@"), "@@ escape should be masked: {text}");
    // Line count must be preserved for source-map accuracy — the
    // masking replaces bytes in place, keeping newlines intact.
    let raw_content = &src["<script>".len()..src.len() - "</script>".len()];
    assert_eq!(
        text.matches('\n').count(),
        raw_content.matches('\n').count(),
        "masking must preserve newline count"
    );
    assert_eq!(text.len(), raw_content.len(), "masking must preserve length");
}

#[test]
fn mask_preserves_non_razor_script_content() {
    let input = "var x = 1;\nfunction go() { return x; }\n";
    let masked = mask_razor_expressions_in_script(input);
    assert_eq!(masked, input, "no-op on pure JS");
}

#[test]
fn mask_handles_unterminated_razor_expression() {
    // Should not panic on a truncated Razor expression.
    let input = "var x = @Html.Ra";
    let _ = mask_razor_expressions_in_script(input); // must not panic
}

#[test]
fn razor_comment_is_skipped() {
    let src = "@* @{ nested } *@\n@{ var x = 1; }";
    let regions = detect_regions(src);
    assert_eq!(regions.len(), 1);
    assert!(regions[0].text.contains("var x = 1"));
}

#[test]
fn at_at_escape_is_ignored() {
    let src = "user@@example.com\n@{ var y = 2; }";
    let regions = detect_regions(src);
    assert_eq!(regions.len(), 1);
    assert!(regions[0].text.contains("var y = 2"));
}

#[test]
fn strings_inside_block_do_not_terminate_early() {
    let src = "@{ var s = \"}a{\"; var t = 1; }";
    let regions = detect_regions(src);
    assert_eq!(regions.len(), 1);
    assert!(regions[0].text.contains("var t = 1"));
}

// -----------------------------------------------------------------
// Directives
// -----------------------------------------------------------------

#[test]
fn model_directive_surfaces_type_as_field() {
    let src = "@model MyApp.Models.Product\n<h1>x</h1>";
    let regions = detect_regions(src);
    assert_eq!(regions.len(), 1);
    assert_eq!(regions[0].language_id, "csharp");
    assert!(regions[0].text.contains("MyApp.Models.Product __razor_model"));
    assert!(regions[0].text.contains("class __RazorBody"));
}

#[test]
fn inject_directive_surfaces_type_and_name() {
    let src = "@inject IUserService UserSvc\n<h1>x</h1>";
    let regions = detect_regions(src);
    assert_eq!(regions.len(), 1);
    assert!(regions[0].text.contains("IUserService UserSvc"));
}

#[test]
fn using_directive_emits_using_statement() {
    let src = "@using Microsoft.Extensions.Logging\n<h1>x</h1>";
    let regions = detect_regions(src);
    assert_eq!(regions.len(), 1);
    assert!(regions[0].text.contains("using Microsoft.Extensions.Logging;"));
    assert!(regions[0].text.contains("class __RazorBody"));
}

#[test]
fn inherits_directive_becomes_base_type() {
    let src = "@inherits RazorPageBase<UserViewModel>\n";
    let regions = detect_regions(src);
    assert_eq!(regions.len(), 1);
    assert!(regions[0].text.contains(": RazorPageBase<UserViewModel>"));
}

#[test]
fn implements_directive_becomes_interface_list() {
    let src = "@implements IDisposable\n";
    let regions = detect_regions(src);
    assert_eq!(regions.len(), 1);
    assert!(regions[0].text.contains(": IDisposable"));
}

#[test]
fn namespace_directive_wraps_in_namespace() {
    let src = "@namespace Acme.Web.Views\n";
    let regions = detect_regions(src);
    assert_eq!(regions.len(), 1);
    assert!(regions[0].text.contains("namespace Acme.Web.Views"));
}

#[test]
fn empty_directive_payload_emits_no_region() {
    let src = "@model\n@inject\n";
    let regions = detect_regions(src);
    assert!(regions.is_empty());
}

#[test]
fn directive_trailing_semicolon_is_stripped() {
    let src = "@using Foo.Bar;\n";
    let regions = detect_regions(src);
    assert_eq!(regions.len(), 1);
    // Exactly one semicolon after the using payload (not two).
    let count = regions[0].text.matches("using Foo.Bar;").count();
    assert_eq!(count, 1);
}

// -----------------------------------------------------------------
// Control flow
// -----------------------------------------------------------------

#[test]
fn if_control_flow_produces_method_body() {
    let src = "@if (user.IsAdmin) { <p>Hi @user.Name</p> }";
    let regions = detect_regions(src);
    assert_eq!(regions.len(), 1);
    assert_eq!(regions[0].language_id, "csharp");
    assert!(regions[0].text.contains("if (user.IsAdmin)"));
    assert!(regions[0].text.contains("void __M()"));
}

#[test]
fn foreach_control_flow_matched() {
    let src = "@foreach (var item in Model.Items) { <li>@item.Name</li> }";
    let regions = detect_regions(src);
    assert_eq!(regions.len(), 1);
    assert!(regions[0].text.contains("foreach (var item in Model.Items)"));
}

#[test]
fn using_with_parens_is_control_flow_not_directive() {
    // `@using (var ctx = new Context()) { ... }` is a disposable
    // using-statement, not a namespace import.
    let src = "@using (var ctx = new Db()) { <p>ok</p> }";
    let regions = detect_regions(src);
    assert_eq!(regions.len(), 1);
    assert!(regions[0].text.contains("using (var ctx = new Db())"));
    // Must be wrapped as method body (has `void __M()`), not as a
    // using-directive compilation unit.
    assert!(regions[0].text.contains("void __M()"));
}

#[test]
fn using_without_parens_is_directive() {
    let src = "@using Microsoft.Extensions.Logging\n<h1>x</h1>";
    let regions = detect_regions(src);
    assert_eq!(regions.len(), 1);
    // Namespace-directive shape (no method body wrapper).
    assert!(!regions[0].text.contains("void __M()"));
}

#[test]
fn switch_and_while_and_for_control_flow() {
    let src = "@switch (x) { case 1: break; }\n@while (true) { }\n@for (int i=0;i<10;i++) { }";
    let regions = detect_regions(src);
    assert_eq!(regions.len(), 3);
    assert!(regions.iter().all(|r| r.language_id == "csharp"));
    assert!(regions[0].text.contains("switch (x)"));
    assert!(regions[1].text.contains("while (true)"));
    assert!(regions[2].text.contains("for (int i=0;i<10;i++)"));
}

// -----------------------------------------------------------------
// Misc
// -----------------------------------------------------------------

#[test]
fn multiple_constructs_coexist() {
    let src = "@model Foo\n@{ var a = 1; }\n@if (a > 0) { <p>yes</p> }\n<script>alert('hi');</script>";
    let regions = detect_regions(src);
    assert_eq!(regions.len(), 4);
    assert_eq!(
        regions.iter().filter(|r| r.language_id == "csharp").count(),
        3
    );
    assert_eq!(
        regions.iter().filter(|r| r.language_id == "javascript").count(),
        1
    );
}

#[test]
fn no_regions_in_plain_html() {
    let src = "<html><body><h1>Hello</h1></body></html>";
    assert!(detect_regions(src).is_empty());
}

#[test]
fn unterminated_block_does_not_loop_forever() {
    let src = "@{ var x = 1; // missing close";
    let _ = detect_regions(src);
}
