// =============================================================================
// pascal/builtins.rs — Pascal/Delphi builtin and helper predicates
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
pub(super) fn is_pascal_builtin(name: &str) -> bool {
    matches!(
        name,
        "WriteLn"
            | "Write"
            | "ReadLn"
            | "Read"
            | "Inc"
            | "Dec"
            | "Ord"
            | "Chr"
            | "Length"
            | "SetLength"
            | "High"
            | "Low"
            | "Assigned"
            | "FreeAndNil"
            | "IntToStr"
            | "StrToInt"
            | "FloatToStr"
            | "StrToFloat"
            | "Format"
            | "Pos"
            | "Copy"
            | "Delete"
            | "Insert"
            | "Trim"
            | "TrimLeft"
            | "TrimRight"
            | "UpperCase"
            | "LowerCase"
            | "CompareStr"
            | "CompareText"
            | "FileExists"
            | "DirectoryExists"
            | "ExtractFileName"
            | "ExtractFilePath"
            | "ExpandFileName"
            | "IncludeTrailingPathDelimiter"
            | "Abs"
            | "Sqr"
            | "Sqrt"
            | "Round"
            | "Trunc"
            | "Ceil"
            | "Floor"
            | "Random"
            | "Randomize"
            | "Now"
            | "Date"
            | "Time"
            | "FormatDateTime"
            | "EncodeDate"
            | "DecodeDate"
            | "ShowMessage"
            | "MessageDlg"
            | "InputBox"
            | "Application"
            | "Screen"
            | "Sender"
            | "Self"
            | "Result"
            | "True"
            | "False"
            | "nil"
            | "Integer"
            | "String"
            | "Boolean"
            | "Real"
            | "Double"
            | "Char"
            | "Byte"
            | "Word"
            | "Cardinal"
            | "Longint"
            | "Int64"
            | "TObject"
            | "TComponent"
            | "TForm"
            | "TList"
            | "TStringList"
    )
}
