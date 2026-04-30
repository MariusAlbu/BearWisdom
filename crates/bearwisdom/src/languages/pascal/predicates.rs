// =============================================================================
// pascal/predicates.rs — Pascal/Delphi builtin and helper predicates
// =============================================================================

use crate::types::EdgeKind;

/// Check that the edge kind is compatible with the symbol kind.
pub(super) fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::Calls => matches!(
            sym_kind,
            "method" | "function" | "constructor" | "test" | "class"
        ),
        EdgeKind::Inherits => matches!(sym_kind, "class"),
        EdgeKind::Implements => matches!(sym_kind, "class" | "interface"),
        EdgeKind::TypeRef => matches!(
            sym_kind,
            "class" | "interface" | "enum" | "type_alias" | "function" | "variable"
        ),
        EdgeKind::Instantiates => matches!(sym_kind, "class" | "function"),
        _ => true,
    }
}

/// Pascal/Delphi RTL procedures, functions, and predefined identifiers.
///
/// Pascal is case-insensitive: `Boolean`, `boolean`, and `BOOLEAN` all
/// resolve to the same builtin. The predicate normalizes the input to
/// lowercase and matches against the lowercase form of every name.
/// Without this, GTK/GLib bindings (which use lowercase `boolean`,
/// `integer`, `inc`, `ord`, `assigned`) bypassed the builtin path and
/// dragged the resolution rate down.
pub(super) fn is_pascal_builtin(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    matches!(
        n.as_str(),
        "writeln"
            | "write"
            | "readln"
            | "read"
            | "inc"
            | "dec"
            | "ord"
            | "chr"
            | "length"
            | "setlength"
            | "high"
            | "low"
            | "assigned"
            | "freeandnil"
            | "inttostr"
            | "strtoint"
            | "floattostr"
            | "strtofloat"
            | "format"
            | "pos"
            | "copy"
            | "delete"
            | "insert"
            | "trim"
            | "trimleft"
            | "trimright"
            | "uppercase"
            | "lowercase"
            | "comparestr"
            | "comparetext"
            | "fileexists"
            | "directoryexists"
            | "extractfilename"
            | "extractfilepath"
            | "expandfilename"
            | "includetrailingpathdelimiter"
            | "abs"
            | "sqr"
            | "sqrt"
            | "round"
            | "trunc"
            | "ceil"
            | "floor"
            | "random"
            | "randomize"
            | "now"
            | "date"
            | "time"
            | "formatdatetime"
            | "encodedate"
            | "decodedate"
            | "showmessage"
            | "messagedlg"
            | "inputbox"
            | "application"
            | "screen"
            | "sender"
            | "self"
            | "result"
            | "true"
            | "false"
            | "nil"
            | "integer"
            | "string"
            | "boolean"
            | "real"
            | "double"
            | "char"
            | "byte"
            | "word"
            | "cardinal"
            | "longint"
            | "int64"
            | "tobject"
            | "tcomponent"
            | "tform"
            | "tlist"
            | "tstringlist"
    )
}
