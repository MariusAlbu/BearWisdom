// =============================================================================
// c_lang/predicates.rs — C/C++ builtin and helper predicates
// =============================================================================

use crate::types::EdgeKind;

/// Check that the edge kind is compatible with the symbol kind.
pub(super) fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::Calls => matches!(
            sym_kind,
            "method" | "function" | "constructor" | "test" | "property"
        ),
        EdgeKind::Inherits => matches!(sym_kind, "class" | "struct"),
        EdgeKind::Implements => matches!(sym_kind, "class" | "interface"),
        EdgeKind::TypeRef => matches!(
            sym_kind,
            "class" | "struct" | "interface" | "enum" | "type_alias" | "namespace"
        ),
        EdgeKind::Instantiates => matches!(sym_kind, "class" | "struct"),
        _ => true,
    }
}

/// Always-external C/C++ namespace roots (std, boost, test frameworks).
const ALWAYS_EXTERNAL_NAMESPACES: &[&str] = &["std", "boost", "gtest", "gmock", "catch2"];

/// Check whether a C/C++ namespace is external.
pub(super) fn is_external_c_namespace(ns: &str) -> bool {
    // Strip leading `::` (global scope qualifier).
    let ns = ns.strip_prefix("::").unwrap_or(ns);
    let root = ns.split("::").next().unwrap_or(ns);
    for prefix in ALWAYS_EXTERNAL_NAMESPACES {
        if root == *prefix {
            return true;
        }
    }
    false
}

/// C standard library header names (used to classify `#include <header>`).
const C_STDLIB_HEADERS: &[&str] = &[
    "stdio", "stdlib", "string", "math", "assert", "ctype", "errno",
    "float", "limits", "locale", "setjmp", "signal", "stdarg", "stddef",
    "stdint", "inttypes", "time", "wchar", "wctype", "stdbool", "complex",
    "tgmath", "fenv", "iso646", "threads", "uchar",
    // C++ standard headers (common subset)
    "algorithm", "array", "bitset", "chrono", "codecvt", "complex",
    "condition_variable", "deque", "exception", "filesystem", "forward_list",
    "fstream", "functional", "future", "initializer_list", "iomanip", "ios",
    "iosfwd", "iostream", "istream", "iterator", "limits", "list", "locale",
    "map", "memory", "mutex", "new", "numeric", "optional", "ostream",
    "queue", "random", "ratio", "regex", "set", "shared_mutex", "sstream",
    "stack", "stdexcept", "streambuf", "string", "string_view", "system_error",
    "thread", "tuple", "type_traits", "typeindex", "typeinfo", "unordered_map",
    "unordered_set", "utility", "valarray", "variant", "vector",
    // POSIX headers
    "unistd", "fcntl", "sys/types", "sys/stat", "sys/socket", "netinet/in",
    "arpa/inet", "netdb", "dirent", "pthread", "semaphore",
];

/// Check whether a `#include` path is a system/stdlib header.
pub(super) fn is_system_header(path: &str) -> bool {
    // System headers use angle brackets — the extractor typically marks these
    // differently, but we check by stripped name as a fallback.
    let base = path
        .trim_matches(|c| c == '<' || c == '>' || c == '"')
        .split('/')
        .last()
        .unwrap_or(path);
    // Strip `.h` suffix.
    let base = base.strip_suffix(".h").unwrap_or(base);
    for &h in C_STDLIB_HEADERS {
        if base == h {
            return true;
        }
    }
    false
}

/// Compiler-intrinsic names that have no source-level definition.
///
/// `__builtin_*` is the GCC/Clang convention for compiler magic (atomic ops,
/// type introspection, vector intrinsics, type IDs like `__builtin_va_list`).
/// `__clang_*`, `__sync_*`, `__atomic_*` are the analogous Clang and legacy
/// GCC families. These appear in source as Calls and TypeRefs but never
/// resolve to a definition — emit-time filter keeps `unresolved_refs` honest.
pub(super) fn is_c_compiler_intrinsic(name: &str) -> bool {
    name.starts_with("__builtin_")
        || name.starts_with("__clang_")
        || name.starts_with("__sync_")
        || name.starts_with("__atomic_")
}

/// Template parameter names and patterns that should be classified as external
/// (they're not real symbols in the index, just formal type parameters).
pub(super) fn is_template_param(name: &str) -> bool {
    // Synthetic token emitted for template_argument_list coverage nodes.
    if name == "<template_args>" {
        return true;
    }
    // C++ expression keywords that can leak into type_identifier-shaped CST
    // positions through error recovery or unusual grammar paths. None of
    // these is ever a type; emitting them as TypeRef pollutes
    // unresolved_refs (e.g. `connect(this, SIGNAL(...))` was leaking
    // `this` as a type_ref via the chain segment for `this_expression`).
    if matches!(name, "this" | "nullptr" | "true" | "false") {
        return true;
    }
    // Single uppercase letter: T, U, V, K, N, E, etc.
    if name.len() == 1 && name.chars().next().map(|c| c.is_uppercase()).unwrap_or(false) {
        return true;
    }
    // Names ending in "Type" are almost always template type parameters
    // (e.g. BasicJsonType, CharType, IteratorType, KeyType, ValueType).
    if name.ends_with("Type") && name.len() > 4 {
        return true;
    }
    // Leading-underscore + uppercase is the C++ standard library implementation
    // convention for template parameter names — `_Range`, `_Pred`, `_Proj`,
    // `_T1`, `_Up`, `_ExecutionPolicy`, `_ForwardIterator`, etc. The C++
    // standard reserves the `_<uppercase>` namespace for implementations,
    // so user code can't legitimately use these as type names. libc++ headers
    // are full of these as type-parameter declarations; emitting them as
    // TypeRefs inflates unresolved counts (5K+ on zig-compiler-fresh's
    // vendored libc++).
    if let Some(rest) = name.strip_prefix('_') {
        if rest.chars().next().map(|c| c.is_uppercase()).unwrap_or(false) {
            return true;
        }
    }
    // Common multi-character template param names.
    matches!(
        name,
        "Args"
            | "Func"
            | "Allocator"
            | "Iterator"
            | "Container"
            | "Predicate"
            | "Compare"
            | "Hash"
            | "KeyEqual"
            | "Traits"
            | "CharT"
            | "Tp"
            | "Up"
            | "Vp"
            | "Ts"
            | "Us"
            | "Vs"
    )
}

/// C/C++ primitive types and language built-in tokens. Used by the extractor
/// to filter `int`, `char`, `void`, `size_t`, etc. from being emitted as
/// TypeRefs — emitting one ref per primitive-type usage explodes the refs
/// table on any C/C++ project. Stdlib *types* (FILE, jmp_buf, std::string,
/// boost::asio::io_context, ...) come in as real symbols via posix_headers
/// / msvc_sdk / qt_runtime walkers and resolve through the index.
pub(super) fn is_c_primitive_type(name: &str) -> bool {
    matches!(
        name,
        // Core language types
        "void" | "char" | "short" | "int" | "long" | "signed" | "unsigned"
        | "float" | "double" | "bool" | "_Bool"
        | "wchar_t" | "char8_t" | "char16_t" | "char32_t"
        | "auto" | "decltype" | "nullptr_t"
        // <stdint.h> fixed-width integers — emitted into every translation unit
        | "int8_t" | "int16_t" | "int32_t" | "int64_t"
        | "uint8_t" | "uint16_t" | "uint32_t" | "uint64_t"
        | "intmax_t" | "uintmax_t" | "intptr_t" | "uintptr_t"
        // <stddef.h> universal typedefs
        | "size_t" | "ssize_t" | "ptrdiff_t" | "max_align_t" | "byte"
        // POSIX universal typedefs (appear in nearly every TU touching syscalls)
        | "off_t" | "mode_t" | "pid_t" | "uid_t" | "gid_t" | "time_t"
        // Language constants
        | "true" | "false" | "nullptr" | "NULL"
    )
}

pub fn is_r_c_api_symbol(name: &str) -> bool {
    matches!(
        name,
        // Core SEXP types
        "SEXP"
            | "SEXPTYPE"
            | "SEXP_STRUCT"
            | "R_xlen_t"
            | "R_len_t"
            | "Rbyte"
            | "Rcomplex"
            | "Rboolean"
            | "R_varloc_t"
            | "R_pstream_t"
            | "R_outpstream_t"
            // SEXP type codes
            | "NILSXP"
            | "SYMSXP"
            | "LISTSXP"
            | "CLOSXP"
            | "ENVSXP"
            | "PROMSXP"
            | "LANGSXP"
            | "SPECIALSXP"
            | "BUILTINSXP"
            | "CHARSXP"
            | "LGLSXP"
            | "INTSXP"
            | "REALSXP"
            | "CPLXSXP"
            | "STRSXP"
            | "DOTSXP"
            | "ANYSXP"
            | "VECSXP"
            | "EXPRSXP"
            | "BCODESXP"
            | "EXTPTRSXP"
            | "WEAKREFSXP"
            | "RAWSXP"
            | "S4SXP"
            | "NEWSXP"
            | "FREESXP"
            | "FUNSXP"
            // Protect stack
            | "PROTECT"
            | "UNPROTECT"
            | "PROTECT_PTR"
            | "UNPROTECT_PTR"
            | "PROTECT_WITH_INDEX"
            | "REPROTECT"
            | "R_ProtectWithIndex"
            | "R_Reprotect"
            | "R_PreserveObject"
            | "R_ReleaseObject"
            // Allocation
            | "Rf_allocVector"
            | "Rf_allocVector3"
            | "Rf_allocMatrix"
            | "Rf_allocArray"
            | "Rf_allocList"
            | "Rf_allocSExp"
            | "Rf_coerceVector"
            | "Rf_PairToVectorList"
            | "Rf_VectorToPairList"
            | "Rf_duplicate"
            | "Rf_shallow_duplicate"
            | "Rf_lazy_duplicate"
            | "Rf_lengthgets"
            | "Rf_xlengthgets"
            // Length
            | "LENGTH"
            | "XLENGTH"
            | "LENGTH_EX"
            | "XLENGTH_EX"
            | "Rf_length"
            | "Rf_xlength"
            | "SETLENGTH"
            | "SET_XLENGTH"
            | "TRUELENGTH"
            | "SET_TRUELENGTH"
            // Type query
            | "TYPEOF"
            | "OBJECT"
            | "ALTREP"
            | "IS_S4_OBJECT"
            | "SET_S4_OBJECT"
            | "UNSET_S4_OBJECT"
            | "IS_SCALAR"
            | "Rf_isNull"
            | "Rf_isSymbol"
            | "Rf_isLogical"
            | "Rf_isReal"
            | "Rf_isComplex"
            | "Rf_isExpression"
            | "Rf_isEnvironment"
            | "Rf_isString"
            | "Rf_isObject"
            | "Rf_isNewList"
            | "Rf_isList"
            | "Rf_isNumeric"
            | "Rf_isNumber"
            | "Rf_isInteger"
            | "Rf_isPrimitive"
            | "Rf_isFactor"
            | "Rf_isFunction"
            | "Rf_isLanguage"
            | "Rf_isMatrix"
            | "Rf_isFrame"
            | "Rf_isArray"
            | "Rf_isTs"
            | "Rf_isVector"
            | "Rf_isVectorAtomic"
            | "Rf_isVectorizable"
            | "Rf_isVectorList"
            // Scalar constructors
            | "Rf_ScalarInteger"
            | "Rf_ScalarReal"
            | "Rf_ScalarLogical"
            | "Rf_ScalarComplex"
            | "Rf_ScalarString"
            | "Rf_ScalarRaw"
            // CHARSXP / string
            | "Rf_mkChar"
            | "Rf_mkCharCE"
            | "Rf_mkCharLen"
            | "Rf_mkCharLenCE"
            | "Rf_mkString"
            | "CHAR"
            | "R_CHAR"
            | "PRINTNAME"
            | "Rf_translateChar"
            | "Rf_translateCharUTF8"
            | "Rf_EncodeReal"
            | "Rf_EncodeInteger"
            | "Rf_EncodeLogical"
            | "Rf_EncodeComplex"
            // Vector element accessors
            | "INTEGER"
            | "INTEGER_RO"
            | "REAL"
            | "REAL_RO"
            | "LOGICAL"
            | "LOGICAL_RO"
            | "COMPLEX"
            | "COMPLEX_RO"
            | "RAW"
            | "RAW_RO"
            | "STRING_ELT"
            | "SET_STRING_ELT"
            | "VECTOR_ELT"
            | "SET_VECTOR_ELT"
            | "INTEGER_ELT"
            | "SET_INTEGER_ELT"
            | "REAL_ELT"
            | "SET_REAL_ELT"
            | "LOGICAL_ELT"
            | "SET_LOGICAL_ELT"
            | "COMPLEX_ELT"
            | "SET_COMPLEX_ELT"
            | "RAW_ELT"
            | "SET_RAW_ELT"
            // List / pairlist accessors
            | "CAR"
            | "CDR"
            | "CAAR"
            | "CDAR"
            | "CADR"
            | "CDDR"
            | "CADDR"
            | "CDDDR"
            | "CADDDR"
            | "CD4R"
            | "CAD4R"
            | "SETCAR"
            | "SETCDR"
            | "SETCADR"
            | "SETCADDR"
            | "SETCADDDR"
            | "SETCAD4R"
            | "TAG"
            | "SETTAG"
            // Symbol / name
            | "Rf_install"
            | "Rf_installChar"
            | "Rf_installTrChar"
            | "Rf_installS3Signature"
            | "Rf_installNoTrChar"
            | "SYMVALUE"
            | "SET_SYMVALUE"
            | "IS_ACTIVE_BINDING"
            | "BINDING_IS_LOCKED"
            | "LOCK_BINDING"
            | "UNLOCK_BINDING"
            // Evaluation
            | "Rf_eval"
            | "Rf_applyClosure"
            | "R_tryEval"
            | "R_tryEvalSilent"
            | "Rf_findFun"
            | "Rf_findVar"
            | "Rf_findVarInFrame"
            | "Rf_findVarInFrame3"
            | "Rf_defineVar"
            | "Rf_setVar"
            | "R_MakeActiveBinding"
            | "Rf_getVar"
            | "Rf_GetOption1"
            | "Rf_GetOption"
            | "Rf_GetOptionDigits"
            | "Rf_GetOptionWidth"
            // Environment
            | "R_GlobalEnv"
            | "R_BaseEnv"
            | "R_EmptyEnv"
            | "R_BaseNamespace"
            | "R_NamespaceRegistry"
            | "Rf_NewEnvironment"
            | "R_NewEnvironment"
            | "ENCLOS"
            | "FRAME"
            | "Rf_EnvironmentIsLocked"
            | "Rf_LockEnvironment"
            | "Rf_LockBinding"
            | "Rf_UnlockBinding"
            | "R_IsPackageEnv"
            | "R_IsNamespaceEnv"
            | "R_FindNamespace"
            // Attributes
            | "Rf_getAttrib"
            | "Rf_setAttrib"
            | "Rf_copyMostAttrib"
            | "Rf_copyMostAttribNoTs"
            | "Rf_inherits"
            | "Rf_GetRowNames"
            | "ATTRIB"
            | "SET_ATTRIB"
            | "R_ClassSymbol"
            | "R_DimSymbol"
            | "R_DimNamesSymbol"
            | "R_NamesSymbol"
            | "R_LevelsSymbol"
            | "R_TspSymbol"
            | "R_CommentSymbol"
            | "R_SrcrefSymbol"
            | "R_SrcfileSymbol"
            | "R_NaRmSymbol"
            | "R_DotsSymbol"
            | "R_DropSymbol"
            | "R_QuoteSymbol"
            | "R_WholeSrcrefSymbol"
            | "R_LastvalueSymbol"
            // Error / condition handling
            | "Rf_error"
            | "Rf_errorcall"
            | "Rf_warning"
            | "Rf_warningcall"
            | "Rf_warningcall_immediate"
            | "R_CheckStack"
            | "R_CheckStack2"
            | "R_CheckUserInterrupt"
            | "R_interrupts_suspended"
            | "R_interrupts_pending"
            | "R_isInterrupted"
            | "Rf_onintr"
            | "Rf_onintrNoResume"
            // NA / missing sentinels
            | "NA_INTEGER"
            | "NA_REAL"
            | "NA_LOGICAL"
            | "NA_STRING"
            | "NA_COMPLEX"
            | "R_NaInt"
            | "R_NaReal"
            | "R_NaLogical"
            | "R_NaN"
            | "R_PosInf"
            | "R_NegInf"
            | "R_IsNA"
            | "R_IsNaN"
            | "R_IsNaNorNA"
            | "R_IsFinite"
            | "R_IsInfinite"
            | "ISNA"
            | "ISNAN"
            | "ISNAREAL"
            | "R_FINITE"
            // Special SEXP constants
            | "R_NilValue"
            | "R_UnboundValue"
            | "R_MissingArg"
            | "R_CurrentExpr"
            | "R_TrueValue"
            | "R_FalseValue"
            | "R_LogicalNAValue"
            | "R_EmptyString"
            | "R_BlankString"
            | "R_BlankScalarString"
            // Memory / GC
            | "R_gc"
            | "R_gc_running"
            | "R_RegisterCFinalizer"
            | "R_RegisterCFinalizerEx"
            | "R_WeakRefKey"
            | "R_WeakRefValue"
            | "R_MakeWeakRef"
            | "R_MakeWeakRefC"
            | "R_RunWeakRefs"
            | "R_RunPendingFinalizers"
            | "vmaxget"
            | "vmaxset"
            | "R_alloc"
            | "S_alloc"
            | "S_realloc"
            // External pointer
            | "R_ExternalPtrAddr"
            | "R_ExternalPtrTag"
            | "R_ExternalPtrProtected"
            | "R_SetExternalPtrAddr"
            | "R_SetExternalPtrTag"
            | "R_SetExternalPtrProtected"
            | "R_MakeExternalPtr"
            | "R_ClearExternalPtr"
            // Call / function construction
            | "Rf_lcons"
            | "LCONS"
            | "Rf_cons"
            | "Rf_list1"
            | "Rf_list2"
            | "Rf_list3"
            | "Rf_list4"
            | "Rf_list5"
            | "Rf_list6"
            | "Rf_lang1"
            | "Rf_lang2"
            | "Rf_lang3"
            | "Rf_lang4"
            | "Rf_lang5"
            | "Rf_lang6"
            // Print
            | "Rf_PrintValue"
            | "Rf_PrintValueRec"
            | "Rf_PrintGeneral"
            // Random numbers (R_ext/Random.h)
            | "GetRNGstate"
            | "PutRNGstate"
            | "unif_rand"
            | "norm_rand"
            | "exp_rand"
            | "R_unif_index"
            // Utils (R_ext/Utils.h)
            | "R_Calloc"
            | "R_Free"
            | "R_Realloc"
            | "Rf_sort"
            | "rsort_with_index"
            | "revsort"
            | "iPsort"
            | "rPsort"
            // Rdefines.h macros
            | "NUMERIC_POINTER"
            | "INTEGER_POINTER"
            | "LOGICAL_POINTER"
            | "CHARACTER_POINTER"
            | "COMPLEX_POINTER"
            | "RAW_POINTER"
            | "NEW_NUMERIC"
            | "NEW_INTEGER"
            | "NEW_LOGICAL"
            | "NEW_CHARACTER"
            | "NEW_COMPLEX"
            | "NEW_RAW"
            | "NEW_LIST"
            | "GET_NUMERIC"
            | "GET_INTEGER"
            | "GET_LOGICAL"
            | "GET_CHARACTER"
            | "GET_COMPLEX"
            | "GET_RAW"
            | "GET_LIST"
            | "GET_LENGTH"
            | "GET_DIM"
            | "GET_NAMES"
            | "GET_CLASS"
            | "GET_LEVELS"
            | "GET_DIMNAMES"
            | "SET_LENGTH"
            | "SET_DIM"
            | "SET_NAMES"
            | "SET_CLASS"
            | "SET_LEVELS"
            | "SET_DIMNAMES"
            | "MAKE_CLASS"
            | "IS_LIST"
            | "IS_CHARACTER"
            | "IS_NUMERIC"
            | "IS_INTEGER"
            | "IS_LOGICAL"
            | "IS_COMPLEX"
            | "IS_RAW"
            // R_ext/RS.h helpers
            | "Calloc"
            | "Free"
            | "Realloc"
            | "CallocCharBuf"
            // Additional R C API symbols from r-dplyr unresolved refs
            | "DllInfo"
            | "PROTECT_INDEX"
            | "PRVALUE"
            | "R_BindingType_t"
            | "R_CallMethodDef"
            | "R_GetCCallable"
            | "R_MakeDelayedBinding"
            | "R_registerRoutines"
            | "R_removeVarFromFrame"
            | "R_useDynamicSymbols"
            | "R_Version"
            | "R_getVar"
            | "Rf_asInteger"
            | "Rf_charIsASCII"
            | "Rf_classgets"
            | "Rf_getCharCE"
            | "Rf_namesgets"
            | "SET_PRCODE"
            | "SET_PRENV"
            | "SET_PRVALUE"
            | "STRING_PTR_RO"
            | "STRING_PTR"
    )
}

