use super::is_pascal_builtin_cast_or_intrinsic;

#[test]
fn drains_builtin_cast_type_names() {
    assert!(is_pascal_builtin_cast_or_intrinsic("Integer"));
    assert!(is_pascal_builtin_cast_or_intrinsic("Single"));
    assert!(is_pascal_builtin_cast_or_intrinsic("Pointer"));
    assert!(is_pascal_builtin_cast_or_intrinsic("String"));
    assert!(is_pascal_builtin_cast_or_intrinsic("Cardinal"));
}

#[test]
fn drains_compiler_magic_procedures_with_no_project_declaration() {
    assert!(is_pascal_builtin_cast_or_intrinsic("Dec"));
    assert!(is_pascal_builtin_cast_or_intrinsic("SizeOf"));
    assert!(is_pascal_builtin_cast_or_intrinsic("Ord"));
    assert!(is_pascal_builtin_cast_or_intrinsic("Chr"));
    assert!(is_pascal_builtin_cast_or_intrinsic("Assigned"));
}

#[test]
fn drains_system_fpd_intrinsics_with_zero_rtl_declarations() {
    // Every name below is a Function/Procedure listed in FPC's
    // rtl/inc/system.fpd fpdoc stub and has zero real declarations in the
    // RTL directories the freepascal_runtime ecosystem walker indexes.
    assert!(is_pascal_builtin_cast_or_intrinsic("Inc"));
    assert!(is_pascal_builtin_cast_or_intrinsic("High"));
    assert!(is_pascal_builtin_cast_or_intrinsic("Low"));
    assert!(is_pascal_builtin_cast_or_intrinsic("Addr"));
    assert!(is_pascal_builtin_cast_or_intrinsic("Assert"));
    assert!(is_pascal_builtin_cast_or_intrinsic("Concat"));
    assert!(is_pascal_builtin_cast_or_intrinsic("Continue"));
    assert!(is_pascal_builtin_cast_or_intrinsic("Exit"));
    assert!(is_pascal_builtin_cast_or_intrinsic("Include"));
    assert!(is_pascal_builtin_cast_or_intrinsic("Exclude"));
    assert!(is_pascal_builtin_cast_or_intrinsic("Ofs"));
    assert!(is_pascal_builtin_cast_or_intrinsic("Seg"));
    assert!(is_pascal_builtin_cast_or_intrinsic("ReadLn"));
    assert!(is_pascal_builtin_cast_or_intrinsic("WriteLn"));
    assert!(is_pascal_builtin_cast_or_intrinsic("Str"));
    assert!(is_pascal_builtin_cast_or_intrinsic("Val"));
    assert!(is_pascal_builtin_cast_or_intrinsic("UnPack"));
    assert!(is_pascal_builtin_cast_or_intrinsic("Default"));
    assert!(is_pascal_builtin_cast_or_intrinsic("TypeInfo"));
    assert!(is_pascal_builtin_cast_or_intrinsic("GetTypeKind"));
    assert!(is_pascal_builtin_cast_or_intrinsic("Fail"));
}

#[test]
fn drain_is_case_insensitive() {
    // Pascal identifiers fold case; the ladder passes the ref's raw source
    // text (unnormalized) into this predicate, so it must fold internally.
    assert!(is_pascal_builtin_cast_or_intrinsic("integer"));
    assert!(is_pascal_builtin_cast_or_intrinsic("INTEGER"));
    assert!(is_pascal_builtin_cast_or_intrinsic("single"));
    assert!(is_pascal_builtin_cast_or_intrinsic("pOiNtEr"));
    assert!(is_pascal_builtin_cast_or_intrinsic("inc"));
    assert!(is_pascal_builtin_cast_or_intrinsic("INC"));
    assert!(is_pascal_builtin_cast_or_intrinsic("hIgH"));
}

#[test]
fn declines_names_with_a_real_declaration_so_that_declaration_can_bind() {
    // BuiltinSkipRule runs before any project-symbol lookup rung
    // (engine/rules/mod.rs: rung #2, right after ModuleSkipRule) and is a
    // pure name match — it cannot tell an RTL declaration from a
    // project-local one. Keeping a name off the drain list is therefore the
    // only thing that lets ANY declaration under that name — RTL-hosted
    // (TPointF.Length) or project-local (a user's own `function Length`) —
    // reach the ladder's normal lookup rungs instead of being drained
    // before it is ever looked up.
    assert!(!is_pascal_builtin_cast_or_intrinsic("FreeAndNil"));
    assert!(!is_pascal_builtin_cast_or_intrinsic("Length")); // TPointF.Length, objpas/types.pp
    assert!(!is_pascal_builtin_cast_or_intrinsic("SetLength")); // TStringBuilder.SetLength
    assert!(!is_pascal_builtin_cast_or_intrinsic("Abs")); // systemh.inc, [internproc]
    assert!(!is_pascal_builtin_cast_or_intrinsic("GetMem")); // systemh.inc, [internproc]
    assert!(!is_pascal_builtin_cast_or_intrinsic("Char"));
    assert!(!is_pascal_builtin_cast_or_intrinsic("Byte"));
    assert!(!is_pascal_builtin_cast_or_intrinsic("Word"));
    assert!(!is_pascal_builtin_cast_or_intrinsic("Boolean"));
    assert!(!is_pascal_builtin_cast_or_intrinsic("boolean"));
    assert!(!is_pascal_builtin_cast_or_intrinsic("Sqrt"));
    assert!(!is_pascal_builtin_cast_or_intrinsic("Double"));
    assert!(!is_pascal_builtin_cast_or_intrinsic("Extended"));
    assert!(!is_pascal_builtin_cast_or_intrinsic("ShortString"));
    assert!(!is_pascal_builtin_cast_or_intrinsic("SmallInt"));
    assert!(!is_pascal_builtin_cast_or_intrinsic("HRESULT"));
    assert!(!is_pascal_builtin_cast_or_intrinsic("PLongInt"));
    assert!(!is_pascal_builtin_cast_or_intrinsic("PSmallInt"));
    assert!(!is_pascal_builtin_cast_or_intrinsic("PInt64"));
    // Common OOP method/property names that happen to match a compiler
    // magic procedure name when called unqualified.
    assert!(!is_pascal_builtin_cast_or_intrinsic("Read"));
    assert!(!is_pascal_builtin_cast_or_intrinsic("Write"));
    assert!(!is_pascal_builtin_cast_or_intrinsic("Copy"));
    assert!(!is_pascal_builtin_cast_or_intrinsic("Delete"));
    assert!(!is_pascal_builtin_cast_or_intrinsic("Insert"));
    assert!(!is_pascal_builtin_cast_or_intrinsic("Move"));
    assert!(!is_pascal_builtin_cast_or_intrinsic("Pos"));
    assert!(!is_pascal_builtin_cast_or_intrinsic("Round"));
    assert!(!is_pascal_builtin_cast_or_intrinsic("New"));
}

#[test]
fn declines_rtl_class_not_a_compiler_intrinsic() {
    // TObject is a real System-unit class, not compiler magic; draining it
    // would hide the extraction gap in the RTL's system.pp instead of
    // surfacing it.
    assert!(!is_pascal_builtin_cast_or_intrinsic("TObject"));
}
