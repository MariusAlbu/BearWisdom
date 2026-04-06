// =============================================================================
// vba/builtins.rs — VBA builtin and helper predicates
// =============================================================================

use crate::types::EdgeKind;

/// Check that the edge kind is compatible with the symbol kind.
pub(super) fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::Calls => matches!(sym_kind, "method" | "function" | "constructor" | "test" | "class"),
        EdgeKind::Inherits => matches!(sym_kind, "class"),
        EdgeKind::Implements => matches!(sym_kind, "class" | "interface"),
        EdgeKind::TypeRef => matches!(sym_kind, "class" | "interface" | "enum" | "type_alias" | "function" | "variable"),
        EdgeKind::Instantiates => matches!(sym_kind, "class" | "function"),
        _ => true,
    }
}

/// VBA builtin functions, statements, and constants always in scope.
pub(super) fn is_vba_builtin(name: &str) -> bool {
    matches!(
        name,
        // UI
        "MsgBox"
            | "InputBox"
            | "Debug.Print"
            | "Debug.Assert"
            // String functions
            | "Len"
            | "Mid"
            | "Left"
            | "Right"
            | "Trim"
            | "LTrim"
            | "RTrim"
            | "UCase"
            | "LCase"
            | "InStr"
            | "InStrRev"
            | "Replace"
            | "Split"
            | "Join"
            // Type conversion
            | "Val"
            | "Str"
            | "CStr"
            | "CInt"
            | "CLng"
            | "CDbl"
            | "CSng"
            | "CBool"
            | "CDate"
            | "CByte"
            | "CVar"
            | "Format"
            // Date/time
            | "Now"
            | "Date"
            | "Time"
            | "Year"
            | "Month"
            | "Day"
            | "Hour"
            | "Minute"
            | "Second"
            | "DateAdd"
            | "DateDiff"
            | "DateSerial"
            | "TimeSerial"
            | "Timer"
            // Type checks
            | "IsNull"
            | "IsEmpty"
            | "IsNumeric"
            | "IsDate"
            | "IsArray"
            | "IsObject"
            | "IsMissing"
            | "IsError"
            | "TypeName"
            | "VarType"
            // Array
            | "Array"
            | "UBound"
            | "LBound"
            | "Erase"
            | "ReDim"
            // File system
            | "Dir"
            | "Kill"
            | "FileCopy"
            | "MkDir"
            | "RmDir"
            | "ChDir"
            | "ChDrive"
            | "CurDir"
            | "FileLen"
            | "FileDateTime"
            | "FreeFile"
            | "Open"
            | "Close"
            | "Input"
            | "Print"
            | "Write"
            | "Get"
            | "Put"
            | "Seek"
            | "EOF"
            | "LOF"
            // System
            | "Shell"
            | "Environ"
            | "CreateObject"
            | "GetObject"
            | "Err"
            | "Error"
            | "On"
            | "Resume"
            | "GoTo"
            | "Exit"
            | "End"
            | "Stop"
            | "DoEvents"
            // Constants
            | "Nothing"
            | "True"
            | "False"
            | "Null"
            | "Empty"
            | "vbCrLf"
            | "vbTab"
            | "vbNewLine"
            | "vbNullString"
    )
}
