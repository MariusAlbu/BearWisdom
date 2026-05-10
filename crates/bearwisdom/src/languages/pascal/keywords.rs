// =============================================================================
// pascal/keywords.rs — Pascal/Delphi syntactic keywords and compiler
// intrinsics.
//
// Three categories belong here:
//
//  1. True grammar tokens — reserved words and pseudo-identifiers that the
//     tree-sitter grammar treats as structural elements (begin/end, if/then,
//     nil, Result, Self, inherited, ...).
//
//  2. Compiler primitive types — integer, boolean, pointer, and variant base
//     types that the FPC compiler supplies without any declaration in .pp/.pas/
//     .inc source files. They are wired into the compiler (INTERNPROC / compiler
//     magic) and appear only in the .fpd documentation stub used by fpdoc; the
//     ecosystem walker cannot discover them because there is no source file
//     containing their type declarations. Examples: Integer, Boolean, Byte,
//     LongInt, LongWord, OleVariant, WordBool, Char, WideChar, Pointer.
//
//  3. Compiler magic procedures and functions — operations that look like
//     ordinary identifiers in Pascal source but are handled entirely by the
//     FPC compiler. They have no declaration in any .pp/.pas/.inc source file;
//     the ecosystem walker therefore cannot discover them. Examples: WriteLn,
//     Inc, Dec, High, Low, SizeOf, SetLength, Copy, Halt, Exit, Break,
//     Continue.
//
// What does NOT belong here: library types, procedures, and classes from the
// RTL, SysUtils, Classes, VCL, or LCL. Those are declared in actual Pascal
// source files and are discovered by the freepascal_runtime ecosystem walker,
// then resolved via the symbol index.
// =============================================================================

/// True Pascal/Delphi grammar tokens and compiler intrinsics.
pub(crate) const KEYWORDS: &[&str] = &[
    // ── Program structure ────────────────────────────────────────────────────
    "program", "unit", "library", "package",
    "uses", "interface", "implementation", "initialization", "finalization",

    // ── Declarations ────────────────────────────────────────────────────────
    "var", "const", "type", "label", "threadvar", "resourcestring",
    "procedure", "function", "constructor", "destructor", "operator",
    "class", "object", "record",
    "property", "published", "public", "protected", "private", "strict",
    "abstract", "virtual", "override", "overload", "reintroduce",
    "dynamic", "message", "static", "inline", "assembler",
    "external", "forward", "stdcall", "cdecl", "pascal", "register",
    "safecall", "winapi",

    // ── Type constructors ────────────────────────────────────────────────────
    "array", "of", "set", "file", "string",
    "packed", "dispinterface",
    "generic", "specialize",

    // ── Control flow ────────────────────────────────────────────────────────
    "begin", "end",
    "if", "then", "else",
    "case",
    "while", "do",
    "repeat", "until",
    "for", "to", "downto",
    "with",
    "goto",

    // ── Exception handling ───────────────────────────────────────────────────
    "try", "except", "finally", "raise", "on",

    // ── Boolean / logic / arithmetic operators ───────────────────────────────
    "and", "or", "not", "xor", "in", "is", "as",
    "div", "mod", "shl", "shr",

    // ── Pseudo-identifiers (grammar treats as keywords) ──────────────────────
    "nil", "True", "False",
    "Self", "inherited", "Result",

    // ── Compiler-directive-adjacent keywords ─────────────────────────────────
    "out", "default", "name", "index", "read", "write",
    "stored", "nodefault",

    // ── Compiler magic — I/O ─────────────────────────────────────────────────
    // These are handled by the FPC compiler itself and have no declaration
    // in any .pp/.pas/.inc source file. The INTERNPROC / compiler-magic
    // mechanism makes them implicitly available everywhere.
    "Write", "WriteLn", "Read", "ReadLn",

    // ── Compiler magic — ordinal / memory / control ───────────────────────────
    "Inc", "Dec",
    "High", "Low",
    "SizeOf", "TypeOf",
    "Ord", "Chr", "Pred", "Succ",
    "Halt", "Exit", "Break", "Continue",
    "Assert",
    "New", "Dispose",
    "GetMem", "FreeMem", "ReallocMem",
    "Move", "FillChar", "CompareMem",

    // ── Compiler magic — string / array ──────────────────────────────────────
    "Length", "SetLength",
    "Copy", "Delete", "Insert", "Concat", "Pos",
    "SetString", "LoadResString",

    // ── Compiler magic — math ─────────────────────────────────────────────────
    "Abs", "Sqr", "Sqrt",
    "Sin", "Cos", "Ln", "Exp",
    "Round", "Trunc", "Frac", "Int",
    "Random", "Randomize",

    // ── Compiler magic — type identity ───────────────────────────────────────
    "Assigned",
    "Addr", "Ptr",
    "Swap",
    "Odd",

    // ── Compiler primitive integer types ─────────────────────────────────────
    // Wired into the FPC compiler; no corresponding declaration exists in any
    // .pp/.pas/.inc source file. The ecosystem walker cannot locate them.
    "Byte", "ShortInt", "SmallInt", "Word", "Integer", "LongInt", "Cardinal",
    "LongWord", "DWord", "QWord", "Int64", "UInt64",
    "NativeInt", "NativeUInt", "SizeInt", "SizeUInt",
    "PtrInt", "PtrUInt", "IntPtr", "UIntPtr",
    "ValSInt", "ValUInt",
    "CodePtrInt", "CodePtrUInt",
    // Platform-width aliases for integer types that alias compiler primitives.
    "ALUSInt", "ALUUInt",

    // ── Compiler primitive float types ───────────────────────────────────────
    "Single", "Double", "Extended", "Real", "Comp", "Currency",

    // ── Compiler primitive boolean types ─────────────────────────────────────
    "Boolean", "ByteBool", "WordBool", "LongBool", "QWordBool",

    // ── Compiler primitive character / string types ───────────────────────────
    "Char", "WideChar", "AnsiChar", "UnicodeChar",
    // ShortString is a compiler primitive; longer string types are RTL aliases
    // but appear in code without any uses clause, so treat them uniformly here.
    "AnsiString", "WideString", "UnicodeString", "ShortString", "UTF8String",
    "RawByteString",

    // ── Compiler primitive pointer and variant types ──────────────────────────
    "Pointer", "PChar", "PAnsiChar", "PWideChar", "PUTF8Char",
    "PByte", "PWord", "PCardinal", "PInteger", "PInt64", "PUInt64",
    "PSmallInt", "PShortInt", "PBoolean", "PPointer", "PPChar",
    "PDWord", "PLongWord", "PLongInt", "PNativeInt", "PNativeUInt",
    "Variant", "OleVariant",
    // IInterface / IUnknown are compiler-known interfaces; GUID and HRESULT are
    // their companion types that the compiler hard-codes as COM interop stubs.
    "IInterface", "IUnknown",
    "TGUID", "PGUID", "GUID", "HRESULT",
];
