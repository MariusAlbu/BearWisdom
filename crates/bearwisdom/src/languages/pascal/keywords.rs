// =============================================================================
// pascal/keywords.rs — Pascal/Delphi/FPC primitive and built-in types
// =============================================================================

/// Primitive and built-in type/function names for Pascal/Delphi/FPC.
pub(crate) const KEYWORDS: &[&str] = &[
    // integer types
    "Integer", "LongInt", "ShortInt", "SmallInt", "Cardinal",
    "Word", "Byte", "Int64", "QWord",
    // float types
    "Real", "Double", "Extended", "Single", "Currency", "Comp",
    // other primitives
    "Boolean", "Char", "String", "AnsiString", "WideString",
    "Pointer", "PChar", "PAnsiChar", "PWideChar",
    "Variant", "OleVariant",
    // interfaces
    "IInterface", "IUnknown", "GUID", "HRESULT",
    // VCL/LCL components
    "TObject", "TComponent", "TForm", "TPanel", "TButton", "TLabel",
    "TEdit", "TMemo", "TListBox", "TComboBox", "TStringList",
    "TStream", "TFileStream", "TMemoryStream", "TStringStream",
    "TThread", "TList", "TDictionary", "TQueue", "TStack",
    // GTK/C FFI types
    "gboolean", "gint", "guint", "gchar", "Pgchar", "gpointer",
    "PPGError",
    // OpenGL types
    "GLenum", "GLuint", "GLint", "GLsizei", "GLfloat", "GLdouble", "GLvoid",
    "TGLenum", "TGLint", "TGLuint", "TGLfloat",
    // I/O
    "WriteLn", "ReadLn", "Write", "Read",
    // built-in procedures / functions
    "Inc", "Dec", "High", "Low", "Length", "SetLength",
    "Copy", "Concat", "Pos", "Delete", "Insert",
    "Trim", "IntToStr", "StrToInt", "FloatToStr", "StrToFloat",
    "Format", "Assigned", "FreeAndNil",
    "New", "Dispose", "GetMem", "FreeMem", "ReallocMem",
    "Move", "FillChar", "CompareMem",
    "SizeOf", "TypeOf",
    "Ord", "Chr", "Pred", "Succ",
    "Abs", "Round", "Trunc", "Frac", "Int",
    "Sqr", "Sqrt", "Sin", "Cos", "Ln", "Exp",
    "Random", "Randomize",
    "Assert", "Halt", "Exit", "Break", "Continue",
    // keywords used as references
    "Result", "Self", "inherited", "nil", "True", "False",
    // From former builtin_type_names:
    "UInt64", "LongWord", "ByteBool", "WordBool", "LongBool",
    "WideChar", "AnsiChar", "UnicodeString", "ShortString", "TClass",
];
