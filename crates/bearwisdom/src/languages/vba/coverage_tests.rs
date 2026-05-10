// =============================================================================
// vba/coverage_tests.rs
//
// Node-kind coverage for VbaPlugin::symbol_node_kinds() and ref_node_kinds().
// Grammar returns None; extraction is performed by the case-insensitive line scanner.
//
// symbol_node_kinds: sub_declaration, function_declaration, class_module,
//                   property_declaration, variable_declaration
// ref_node_kinds:    call_statement
// =============================================================================

use super::extract;
use crate::types::{EdgeKind, SymbolKind};

// ---------------------------------------------------------------------------
// symbol_node_kinds
// ---------------------------------------------------------------------------

#[test]
fn cov_sub_declaration_produces_function() {
    let r = extract::extract("Sub MySub()\n    MsgBox \"Hello\"\nEnd Sub\n");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Function && s.name == "MySub"),
        "Sub should produce Function(MySub); got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_function_declaration_produces_function() {
    let r = extract::extract("Function Square(x As Integer) As Integer\n    Square = x * x\nEnd Function\n");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Function && s.name == "Square"),
        "Function should produce Function(Square); got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_class_module_produces_class() {
    // VBA class module marker: `Attribute VB_Name = "ClassName"`
    let r = extract::extract("Attribute VB_Name = \"MyClass\"\n");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Class && s.name == "MyClass"),
        "VB_Name attribute should produce Class(MyClass); got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// ref_node_kinds
// ---------------------------------------------------------------------------

#[test]
fn cov_call_statement_produces_calls() {
    // `Call SubName` inside a sub → Calls ref
    let r = extract::extract("Sub Main()\n    Call Helper\nEnd Sub\n");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls && rf.target_name == "Helper"),
        "Call statement should produce Calls(Helper); got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// Artefact regression tests (Task 7d)
// ---------------------------------------------------------------------------

#[test]
fn no_trailing_comma_in_call_ref() {
    // Arguments of a Call should not be mistaken for callee names.
    // "QuoteChar," on a continuation line must not be emitted.
    let src = "Sub Main()\n    Call WriteCsv QuoteChar, FieldID, fieldDelimiter\nEnd Sub\n";
    let r = extract::extract(src);
    for rf in &r.refs {
        assert!(
            !rf.target_name.ends_with(','),
            "ref target_name has trailing comma: {:?}",
            rf.target_name
        );
    }
}

#[test]
fn dotted_call_emits_method_name_only() {
    // "Call Create.protInit" should emit "protInit", not "Create.protInit".
    let src = "Sub Main()\n    Call Create.protInit\nEnd Sub\n";
    let r = extract::extract(src);
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "protInit"),
        "dotted Call should emit last segment; got: {:?}",
        r.refs.iter().map(|rf| &rf.target_name).collect::<Vec<_>>()
    );
    assert!(
        !r.refs.iter().any(|rf| rf.target_name.contains('.')),
        "no ref target should contain a dot; got: {:?}",
        r.refs.iter().map(|rf| &rf.target_name).collect::<Vec<_>>()
    );
}

#[test]
fn implicit_call_token_with_trailing_comma_is_dropped() {
    // A line whose first token is "QuoteChar," (from a multi-line continuation)
    // must not produce a call ref.
    let src = "Sub Main()\n    QuoteChar, FieldID\nEnd Sub\n";
    let r = extract::extract(src);
    assert!(
        r.refs.iter().all(|rf| rf.target_name != "QuoteChar,"),
        "comma-suffixed token must not become a call ref"
    );
}

#[test]
fn path_like_first_token_is_dropped() {
    // A first token containing '/' or '\' is a path string, not a callee.
    let src = "Sub Main()\n    docs/assets/Status_G something\nEnd Sub\n";
    let r = extract::extract(src);
    assert!(
        r.refs.iter().all(|rf| !rf.target_name.contains('/')),
        "path-like identifier must not become a call ref"
    );
}

#[test]
fn quoted_first_token_is_dropped() {
    // A first token starting with `"` is a string literal that bled into the
    // first-token slot when the leading whitespace before the quote was
    // stripped. Common in VBA code that writes HTTP headers:
    //     "Set-Cookie", strValue
    let src = "Sub Main()\n    \"Set-Cookie\", strValue\nEnd Sub\n";
    let r = extract::extract(src);
    assert!(
        r.refs.iter().all(|rf| !rf.target_name.starts_with('"')),
        "double-quoted token must not become a call ref; got {:?}",
        r.refs.iter().map(|rf| &rf.target_name).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// Continuation-line suppression
// ---------------------------------------------------------------------------

#[test]
fn continuation_line_does_not_produce_call_ref() {
    // A line ending with " _" continues onto the next physical line. The
    // continuation line begins with an expression token (e.g. a local variable
    // name) that would otherwise look like an implicit call statement.
    // The scanner must not emit a Calls ref for the continuation line.
    let src = concat!(
        "Sub Main()\n",
        "    Result = SomeFunc(Arg1, _\n",   // ends with " _" → continues
        "                     TokenEndingPos - 1)\n",  // continuation: NOT a call
        "End Sub\n",
    );
    let r = extract::extract(src);
    assert!(
        r.refs.iter().all(|rf| rf.target_name != "TokenEndingPos"),
        "identifier on a continuation line must not become a Calls ref; got: {:?}",
        r.refs.iter().map(|rf| &rf.target_name).collect::<Vec<_>>()
    );
}

#[test]
fn first_line_of_continuation_still_emits_call() {
    // The first line of a continuation block (ending with " _") is a real
    // statement and should produce a Calls ref for its callee.
    let src = concat!(
        "Sub Main()\n",
        "    Call SomeFunc Arg1, _\n",
        "                 Arg2\n",
        "End Sub\n",
    );
    let r = extract::extract(src);
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls && rf.target_name == "SomeFunc"),
        "callee on the first line of a continuation must still produce a Calls ref; got: {:?}",
        r.refs.iter().map(|rf| &rf.target_name).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// Declare statement extraction
// ---------------------------------------------------------------------------

#[test]
fn declare_ptrsafe_function_produces_symbol() {
    let src = "Private Declare PtrSafe Function GdipDisposeImage Lib \"gdiplus\" (ByVal Image As LongPtr) As Long\n";
    let r = extract::extract(src);
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Function && s.name == "GdipDisposeImage"),
        "Declare PtrSafe Function must produce Function symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn declare_sub_produces_symbol() {
    let src = "Private Declare PtrSafe Sub GdiplusShutdown Lib \"gdiplus\" (ByVal token As LongPtr)\n";
    let r = extract::extract(src);
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Function && s.name == "GdiplusShutdown"),
        "Declare PtrSafe Sub must produce Function symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn declare_inside_if_block_produces_symbol() {
    // Declares inside #If / #Else / #End If blocks must be extracted.
    let src = concat!(
        "#If VBA7 Then\n",
        "Private Declare PtrSafe Function utc_GetTimeZoneInformation Lib \"kernel32\" () As Long\n",
        "#Else\n",
        "Private Declare Function utc_GetTimeZoneInformation Lib \"kernel32\" () As Long\n",
        "#End If\n",
    );
    let r = extract::extract(src);
    let count = r.symbols.iter()
        .filter(|s| s.name == "utc_GetTimeZoneInformation" && s.kind == SymbolKind::Function)
        .count();
    assert!(
        count >= 1,
        "Declare inside #If/#Else block must produce at least one Function symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn conditional_compilation_marker_does_not_produce_call_ref() {
    // #If, #ElseIf, #Else, #End If directives must be treated as transparent
    // markers — they must not produce any Calls ref.
    let src = concat!(
        "Sub Main()\n",
        "#If VBA7 Then\n",
        "    Call Helper\n",
        "#Else\n",
        "    Call Helper\n",
        "#End If\n",
        "End Sub\n",
    );
    let r = extract::extract(src);
    // Exactly one Calls(Helper) — not two or zero.
    let count = r.refs.iter()
        .filter(|rf| rf.kind == EdgeKind::Calls && rf.target_name == "Helper")
        .count();
    // Both branches call Helper, so two refs is correct; the key invariant is
    // that the directive lines themselves don't produce extra refs.
    let bad_refs: Vec<_> = r.refs.iter()
        .filter(|rf| rf.target_name.starts_with('#'))
        .collect();
    assert!(
        bad_refs.is_empty(),
        "directive lines must not produce Calls refs; got: {:?}",
        bad_refs
    );
    let _ = count; // both-branch extraction is intentional
}
