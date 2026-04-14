//! Blade directive taxonomy.
//!
//! Blade has dozens of directives but only a handful matter for the
//! code-graph: structural ones that name templates, sections, or
//! components. The rest (`@if`, `@foreach`, `@auth`, `@guest`, …) are
//! control flow that doesn't yield queryable graph nodes — they're
//! skipped by the host extractor.

use crate::types::SymbolKind;

/// Directives that yield a NAMED symbol on the host file. Each maps to
/// the `SymbolKind` we record for it. Symbol name is the first string
/// argument (e.g. `@section("content")` → name `"content"`).
pub static DEFINING_DIRECTIVES: &[(&str, SymbolKind)] = &[
    ("section",   SymbolKind::Method),    // a content slot definition
    ("push",      SymbolKind::Method),    // append to a stack
    ("prepend",   SymbolKind::Method),    // prepend to a stack
    ("stack",     SymbolKind::Field),     // declare a stack outlet
    ("component", SymbolKind::Class),     // anonymous component definition
    ("slot",      SymbolKind::Field),     // named slot inside a component
    ("hasSection",SymbolKind::Method),    // section query — still names a section
    ("yield",     SymbolKind::Field),     // outlet for a section
];

/// Directives that emit a REFERENCE to an external template by name.
/// Each is recorded as an `Imports` edge so the resolver can match it
/// against template symbols defined elsewhere.
pub static REFERENCING_DIRECTIVES: &[&str] = &[
    "extends",
    "include",
    "includeIf",
    "includeWhen",
    "includeUnless",
    "includeFirst",
    "each",
    "use",
];
