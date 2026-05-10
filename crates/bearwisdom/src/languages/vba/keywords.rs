// =============================================================================
// vba/keywords.rs — VBA compiler intrinsics and runtime primitives.
//
// Two categories belong here:
//
//  1. Primitive types — built into the VBA compiler. No source declaration
//     exists in any .bas/.cls/.frm file. Examples: Integer, Long, String,
//     Boolean, Variant, Date, Object.
//
//  2. VBA runtime intrinsics — functions and statements that the VBA runtime
//     supplies without any module declaration. They are wired into the VBA
//     engine (msvbvm60 / VBE7) and therefore unreachable by the ecosystem
//     walker. Examples: Len, Mid, MsgBox, CInt, IsNumeric, UBound.
//
// What does NOT belong here: functions declared in project .bas/.cls/.frm
// files or by Declare statements pointing to external DLLs. Those are
// extracted as symbols and resolved through the normal symbol index.
//
// Office Object Model names (Application, Workbook, Worksheet, Range, …) are
// included under a clearly-labelled section. They are COM type library
// definitions shipped with Microsoft Office, not VBA language intrinsics.
// A proper fix would walk Office's type libraries; absent that infrastructure,
// listing the most common names here suppresses the bulk of false unresolved
// refs on Office-hosted VBA corpora. The trade-off is documented here rather
// than hidden.
// =============================================================================

pub(crate) const KEYWORDS: &[&str] = &[
    // ── Primitive types ──────────────────────────────────────────────────────
    "Integer",
    "Long",
    "LongLong",
    "LongPtr",
    "Single",
    "Double",
    "Currency",
    "Decimal",
    "Boolean",
    "Byte",
    "Date",
    "String",
    "Object",
    "Variant",
    "Nothing",
    "Empty",
    "Null",

    // ── String intrinsics ─────────────────────────────────────────────────────
    "Len",
    "LenB",
    "Mid",
    "MidB",
    "Left",
    "LeftB",
    "Right",
    "RightB",
    "Trim",
    "LTrim",
    "RTrim",
    "LCase",
    "UCase",
    "InStr",
    "InStrB",
    "InStrRev",
    "Replace",
    "Split",
    "Join",
    "StrComp",
    "StrConv",
    "Space",
    "String",
    "Chr",
    "ChrB",
    "ChrW",
    "Asc",
    "AscB",
    "AscW",
    "Format",
    "FormatNumber",
    "FormatCurrency",
    "FormatPercent",
    "FormatDateTime",
    "StrReverse",

    // ── Type-conversion intrinsics ────────────────────────────────────────────
    "CInt",
    "CLng",
    "CLngLng",
    "CLngPtr",
    "CDbl",
    "CSng",
    "CDec",
    "CCur",
    "CStr",
    "CBool",
    "CDate",
    "CByte",
    "CVar",
    "CVErr",

    // ── Numeric / math intrinsics ─────────────────────────────────────────────
    "Int",
    "Fix",
    "Round",
    "Abs",
    "Sgn",
    "Sqr",
    "Exp",
    "Log",
    "Sin",
    "Cos",
    "Tan",
    "Atn",
    "Rnd",
    "Randomize",

    // ── Type-test intrinsics ──────────────────────────────────────────────────
    "IsNumeric",
    "IsEmpty",
    "IsNull",
    "IsArray",
    "IsDate",
    "IsObject",
    "IsError",
    "IsMissing",
    "TypeName",
    "VarType",

    // ── Array intrinsics ──────────────────────────────────────────────────────
    "Array",
    "UBound",
    "LBound",
    "Filter",

    // ── Event / flow control statements ──────────────────────────────────────
    // These are VBA language statements that appear as the first identifier
    // on a line and look like procedure calls to the heuristic scanner.
    "RaiseEvent",
    "GoSub",
    "DoEvents",
    "Static",

    // ── I/O and interaction intrinsics ────────────────────────────────────────
    // These are VBA runtime statements, not Office-specific.
    "MsgBox",
    "InputBox",
    "Shell",
    "SaveSetting",
    "DeleteSetting",
    "GetSetting",
    "GetAllSettings",
    "SendKeys",
    "Beep",

    // ── Date / time intrinsics ────────────────────────────────────────────────
    "Now",
    "Date",
    "Time",
    "Year",
    "Month",
    "Day",
    "Hour",
    "Minute",
    "Second",
    "Weekday",
    "WeekdayName",
    "MonthName",
    "DateAdd",
    "DateDiff",
    "DatePart",
    "DateSerial",
    "DateValue",
    "TimeSerial",
    "TimeValue",
    "Timer",

    // ── File / I/O intrinsics ─────────────────────────────────────────────────
    "Dir",
    "FileLen",
    "FileDateTime",
    "GetAttr",
    "SetAttr",
    "Kill",
    "FileCopy",
    "MkDir",
    "RmDir",
    "ChDir",
    "ChDrive",
    "CurDir",
    "FreeFile",
    "LOF",
    "EOF",
    "Loc",
    "Seek",

    // ── Pointer / memory intrinsics ───────────────────────────────────────────
    "VarPtr",
    "ObjPtr",
    "StrPtr",

    // ── Miscellaneous intrinsics ──────────────────────────────────────────────
    "Choose",
    "Switch",
    "IIf",
    "Environ",
    "Hex",
    "Oct",
    "Val",
    "Str",
    "Error",
    "CreateObject",
    "GetObject",
    "TypeOf",
    "Nz",

    // ── Office Object Model — common top-level objects ────────────────────────
    // These are COM type library definitions from Microsoft Office, not VBA
    // language intrinsics. A typelib walker would be the correct fix; this
    // list covers the most frequent names in Office-hosted VBA corpora.
    "Application",
    "ActiveWorkbook",
    "ActiveSheet",
    "ActiveCell",
    "ActiveWindow",
    "ActiveDocument",
    "ActivePresentation",
    "Selection",
    "ThisWorkbook",
    "Workbooks",
    "Worksheets",
    "Sheets",
    "Charts",
    "Workbook",
    "Worksheet",
    "Range",
    "Cells",
    "Rows",
    "Columns",
    "Cell",
    "Shape",
    "Shapes",
    "CommandBars",
    "CommandBar",
    "UserForm",
    "Me",
    "Err",
    "Debug",

    // ── Excel-specific constants frequently used as bare names ────────────────
    "xlUp",
    "xlDown",
    "xlToLeft",
    "xlToRight",
    "xlBitmap",
    "xlPicture",
    "xlScreen",
    "xlPrinter",
    "xlByRows",
    "xlByColumns",
    "xlValues",
    "xlFormulas",
    "xlNotes",
    "xlWhole",
    "xlPart",
    "xlAscending",
    "xlDescending",
    "xlYes",
    "xlNo",
    "xlGuess",
    "xlNone",
    "xlThin",
    "xlMedium",
    "xlThick",
    "xlContinuous",
    "xlDash",
    "xlDot",
    "xlDashDot",
    "xlDashDotDot",
    "xlSlantDashDot",
    "xlDouble",
    "xlEdgeLeft",
    "xlEdgeTop",
    "xlEdgeBottom",
    "xlEdgeRight",
    "xlInsideVertical",
    "xlInsideHorizontal",
];
