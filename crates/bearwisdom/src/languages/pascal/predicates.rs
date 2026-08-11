// =============================================================================
// pascal/predicates.rs — Pascal/Delphi builtin and helper predicates
// =============================================================================

use crate::types::EdgeKind;

/// Compiler-supplied type names and compiler-magic procedures/functions that
/// are never project symbols, matched case-insensitively (Pascal identifiers
/// fold case). Feeds `LanguageProfile::builtin_skip`, which the resolve
/// ladder consults on the ref's raw `target_name` — case folding is done here
/// because the ladder does not normalize the string before calling this
/// predicate.
///
/// A type-cast (`Integer(x)`, `Pointer(p)`) and an ordinary function call
/// share the same `exprCall` grammar node; the Pascal extractor cannot tell
/// them apart without a symbol table, so both are emitted as `Calls` refs
/// (see `refs::extract_call`). Distinguishing "this name is a builtin type,
/// not a missing project function" is therefore resolve-time work, which is
/// exactly what `builtin_skip` does.
///
/// Deliberately excludes any name with at least one internal declaration in
/// the Pascal reference corpus (case-insensitively): `FreeAndNil`, `Inc`,
/// `High`, `Low`, `Length`, `SetLength`, `Assert`, `Char`, `Byte`, `LongInt`,
/// `LongWord`, `WriteLn`, `ReadLn`, `Exit`, `Continue`, `Concat`, `Sqrt`,
/// `Double`, `Extended`, `Boolean`, `ShortString`, `SmallInt`, `HRESULT`,
/// `PLongInt`, `PSmallInt`, `PInt64`, `Word`, and the full family of
/// `Read`/`Write`/`Copy`/`Delete`/`Insert`/`Move`/`Pos`/`Round`/`New`-shaped
/// names that Pascal projects routinely reuse as class method or property
/// names (`TStream.Read`, `TStream.Write`, `TList.Delete`, ...). The ladder
/// runs `builtin_skip` before any project-symbol lookup rung, so draining a
/// name with even one real declaration would silently blind the resolver to
/// that declaration everywhere in the corpus.
///
/// `TObject` is likewise excluded: it is not a compiler-magic identifier but
/// a real `System`-unit class, and the FPC RTL's `system.pp` is walked as an
/// external file yet extracts zero symbols in this corpus (a parse/extractor
/// gap upstream of this predicate) — `TObject` belongs behind the ladder's
/// external lookup once that gap closes, not behind a drain.
pub(super) fn is_pascal_builtin_cast_or_intrinsic(name: &str) -> bool {
    PASCAL_BUILTIN_SKIP.iter().any(|b| name.eq_ignore_ascii_case(b))
}

const PASCAL_BUILTIN_SKIP: &[&str] = &[
    // ── Compiler magic — control flow / memory / string, none shadowed by a
    //    project declaration anywhere in the Pascal reference corpus ────────
    "Dec",
    "SizeOf",
    "TypeOf",
    "Ord",
    "Chr",
    "Pred",
    "Succ",
    "Halt",
    "Break",
    "Dispose",
    "ReallocMem",
    "FillChar",
    "CompareMem",
    "LoadResString",
    "Sqr",
    "Cos",
    "Ln",
    "Randomize",
    "Assigned",
    "Odd",
    // ── Compiler primitive integer types ────────────────────────────────────
    "ShortInt",
    "Integer",
    "Cardinal",
    "NativeInt",
    "NativeUInt",
    "IntPtr",
    "UIntPtr",
    "ValSInt",
    "ValUInt",
    "CodePtrInt",
    "CodePtrUInt",
    "ALUSInt",
    "ALUUInt",
    // ── Compiler primitive float types ──────────────────────────────────────
    "Single",
    "Real",
    "Currency",
    // ── Compiler primitive boolean types ────────────────────────────────────
    "ByteBool",
    "WordBool",
    "LongBool",
    "QWordBool",
    // ── Compiler primitive character / string types ─────────────────────────
    "WideChar",
    "AnsiChar",
    "UnicodeChar",
    "AnsiString",
    "WideString",
    "UTF8String",
    "String",
    // ── Compiler primitive pointer and variant types ────────────────────────
    "Pointer",
    "PChar",
    "PAnsiChar",
    "PWideChar",
    "PPChar",
    "PNativeInt",
    "PNativeUInt",
    "Variant",
    "OleVariant",
    "IInterface",
    "TGUID",
    "PGUID",
];

/// Check that the edge kind is compatible with the symbol kind.
///
/// The Pascal extractor emits `Calls` for all reference sites — both actual
/// procedure/function calls and typeref nodes (type annotations, variable
/// declarations, etc.) — so `Calls` must accept any addressable symbol kind.
pub(super) fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        // Accept all symbol kinds: the extractor uses Calls for both call
        // expressions and typeref nodes, covering functions, types, enums,
        // records, variables, and properties.
        EdgeKind::Calls => !matches!(sym_kind, "namespace" | "module" | "package"),
        EdgeKind::Inherits => matches!(sym_kind, "class" | "interface"),
        EdgeKind::Implements => matches!(sym_kind, "class" | "interface"),
        EdgeKind::TypeRef => matches!(
            sym_kind,
            "class" | "interface" | "enum" | "type_alias" | "function" | "variable" | "struct"
        ),
        EdgeKind::Instantiates => matches!(sym_kind, "class" | "function"),
        _ => true,
    }
}

#[cfg(test)]
#[path = "predicates_tests.rs"]
mod tests;
