/// Runtime globals always external for Pascal/Delphi/FPC.
///
/// VCL/LCL/RTL identifiers that appear in project code but are never defined
/// there — they come from the RTL units (System, SysUtils, Classes, Controls,
/// etc.) and from the FPC/Delphi standard library. Types are in primitives.rs;
/// this file covers additional VCL components, event types, and RTL globals.
pub(crate) const EXTERNALS: &[&str] = &[
    // Additional VCL/LCL widget types not already in primitives
    "TFrame", "TGroupBox", "TScrollBox", "TScrollBar",
    "TCheckBox", "TRadioButton", "TRadioGroup",
    "TImage", "TBitmap", "TIcon", "TMetafile",
    "TImageList", "TActionList", "TAction",
    "TMainMenu", "TPopupMenu",
    "TToolBar", "TToolButton", "TStatusBar",
    "TTreeView", "TTreeNode", "TTreeNodes",
    "TListView", "TListItem", "TListItems", "TListColumn",
    "TPageControl", "TTabSheet", "TTabControl",
    "TSplitter", "TBevel",
    "TOpenDialog", "TSaveDialog", "TColorDialog",
    "TFontDialog", "TPrintDialog", "TFindDialog",
    "TTimer", "TApplicationEvents",
    // Additional RTL classes
    "TObjectList", "TInterfaceList",
    "TBinaryReader", "TBinaryWriter",
    "TStringBuilder",
    "TRegEx", "TMatch", "TMatchCollection", "TGroupCollection",
    "TEncoding", "THashAlgorithm",
    "TMonitor",
    // Exceptions — FPC/Delphi built-in exception classes
    "Exception", "EAbort", "EAccessViolation",
    "EConvertError", "EDivByZero", "EInvalidCast", "EInvalidOp",
    "EInvalidPointer", "EIOError", "EIntOverflow", "ERangeError",
    "EStackOverflow", "EOutOfMemory", "ESafecallException",
    "EExternal", "EExternalException",
    "EFilerError", "EReadError", "EWriteError",
    "EMathError", "EOverflow", "EUnderflow", "EZeroDivide",
    // SysUtils functions not in primitives
    "FileGetAttr", "FileSetAttr", "FileAge", "FileIsReadOnly",
    "FileSetReadOnly", "RenameFile", "DeleteFile",
    "GetCurrentDir", "SetCurrentDir",
    "CreateDir", "RemoveDir",
    "ExcludeTrailingPathDelimiter",
    "ChangeFileExt", "ExtractFileExt", "ExtractFileDrive",
    "MatchesMask",
    "IntToHex", "HexToInt",
    "BoolToStr", "StrToBool",
    "FloatToStrF", "StrToFloat", "TryStrToFloat",
    "TryStrToInt", "TryStrToInt64",
    "VarToStr", "VarIsNull", "VarIsEmpty",
    "DateToStr", "StrToDate", "TimeToStr", "StrToTime",
    "DateTimeToStr", "StrToDateTime",
    "EncodeTime", "DecodeTime", "IncDay", "IncMonth",
    "DaysBetween", "HoursBetween", "MinutesBetween",
    "CompareDate", "CompareTime", "CompareDateTime",
    // WinAPI types (Delphi/FPC Win32 targets)
    "HWND", "HINSTANCE", "HANDLE", "HDC", "HBITMAP",
    "HFONT", "HBRUSH", "HPEN", "HRGN", "HCURSOR",
    "DWORD", "UINT", "WPARAM", "LPARAM", "LRESULT",
    "BOOL", "PBOOL", "PHANDLE",
    "MSG", "POINT", "SIZE", "RECT", "PAINTSTRUCT",
    "WNDCLASS", "WNDPROC",
    // RTL global variables
    "Application", "Screen", "Printer", "Mouse", "Clipboard",
    "ExitCode", "ParamCount", "ParamStr",
    // System unit identifiers
    "IsConsole", "IsLibrary",
    "MainThreadID", "GetCurrentThreadID",
];

