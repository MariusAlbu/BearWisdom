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
/// Derivation contract for the compiler-magic procedure/function names below:
/// each candidate is a `Function`/`Procedure` declared in FPC's
/// `rtl/inc/system.fpd` — the fpdoc phony-declaration stub documenting
/// routines the compiler substitutes inline and never declares in real
/// source. A candidate is drained only when it also has zero real
/// `function`/`procedure` declarations anywhere in the RTL directories the
/// `freepascal_runtime` ecosystem walker indexes (`inc/`, `objpas/`,
/// `packages/*`, and the host RTL target tree — see
/// `ecosystem::freepascal_runtime::discover_freepascal_roots`). Two source
/// shapes do not count as a real declaration: `.fpd` files themselves
/// (fpdoc stubs, never compiled) and RTL files disabled from the build
/// (`inc/lstrings.pp` is commented out at `inc/makefile.inc:22` and its
/// `Length`/`SetLength`/`Copy`/`Str`/`Val` overloads never compile under any
/// target).
///
/// Names kept off the drain list because a real declaration exists
/// elsewhere in that scan: `FreeAndNil`, `Length` (`TPointF.Length` in
/// `objpas/types.pp`), `SetLength` (`TStringBuilder.SetLength`), `Char`,
/// `Byte`, `Word`, `Boolean`, `Double`, `Extended`, `ShortString`,
/// `SmallInt`, `HRESULT`, `PLongInt`, `PSmallInt`, `PInt64`, `Sqrt`, and the
/// `Read`/`Write`/`Copy`/`Delete`/`Insert`/`Move`/`Pos`/`Round`/`New`-shaped
/// family Pascal projects routinely reuse as class method or property names
/// (`TStream.Read`, `TStream.Write`, `TFPList.Delete`, ...). The ladder runs
/// `builtin_skip` before any project-symbol lookup rung, so keeping a name
/// with even one real declaration off this list is what lets that
/// declaration — RTL-hosted or project-local — bind through the ladder's
/// normal lookup rungs instead of being pre-emptively drained; the
/// drain-audit gate (`bw quality-check`) re-checks the drained set against
/// the index and flags any name that later gains one.
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
    // ── Compiler magic — control flow / memory / string, zero real
    //    declarations anywhere in the RTL tree the ecosystem walker indexes ─
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
    // ── system.fpd-derived intrinsics, verified zero RTL declarations (see
    //    the derivation contract above) ──────────────────────────────────────
    "Addr",
    "Assert",
    "Concat",
    "Continue",
    "Exit",
    "High",
    "Inc",
    "Include",
    "Exclude",
    "Low",
    "Ofs",
    "ReadLn",
    "WriteLn",
    "Seg",
    "Str",
    "Val",
    "UnPack",
    "Default",
    "TypeInfo",
    "GetTypeKind",
    "Fail",
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
