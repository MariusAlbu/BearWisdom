// =============================================================================
// zig/predicates.rs — Zig builtin and helper predicates
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

/// Zig builtin functions and the std namespace always in scope.
pub(super) fn is_zig_builtin(name: &str) -> bool {
    matches!(
        name,
        // --- Type cast / conversion builtins ---
        "@import"
            | "@intCast"
            | "@floatCast"
            | "@as"
            | "@bitCast"
            | "@ptrCast"
            | "@alignCast"
            | "@truncate"
            | "@enumFromInt"
            | "@intFromEnum"
            | "@intFromBool"
            | "@intFromFloat"
            | "@intFromPtr"
            | "@floatFromInt"
            | "@ptrFromInt"
            // --- Size / alignment / type introspection ---
            | "@sizeOf"
            | "@alignOf"
            | "@bitSizeOf"
            | "@offsetOf"
            | "@typeInfo"
            | "@typeName"
            | "@TypeOf"
            | "@Type"
            | "@tagName"
            | "@hasField"
            | "@hasDecl"
            // --- Field / pointer builtins ---
            | "@fieldParentPtr"
            | "@field"
            // --- Error / compile builtins ---
            | "@errorName"
            | "@errorReturnTrace"
            | "@returnAddress"
            | "@frameAddress"
            | "@panic"
            | "@compileError"
            | "@compileLog"
            | "@breakpoint"
            // --- Source location ---
            | "@src"
            // --- Embedding / C interop ---
            | "@embedFile"
            | "@cImport"
            | "@cInclude"
            | "@cDefine"
            | "@cUndef"
            | "@extern"
            // --- Math builtins ---
            | "@min"
            | "@max"
            | "@abs"
            | "@sqrt"
            | "@sin"
            | "@cos"
            | "@exp"
            | "@exp2"
            | "@log"
            | "@log2"
            | "@log10"
            | "@floor"
            | "@ceil"
            | "@round"
            | "@trunc"
            | "@mod"
            | "@rem"
            | "@divFloor"
            | "@divTrunc"
            | "@divExact"
            // --- Overflow arithmetic ---
            | "@addWithOverflow"
            | "@subWithOverflow"
            | "@mulWithOverflow"
            | "@shlWithOverflow"
            | "@shlExact"
            | "@shrExact"
            // --- Memory builtins ---
            | "@memset"
            | "@memcpy"
            | "@memmove"
            // --- Atomics ---
            | "@atomicLoad"
            | "@atomicStore"
            | "@atomicRmw"
            | "@cmpxchgWeak"
            | "@cmpxchgStrong"
            | "@fence"
            | "@prefetch"
            // --- SIMD / vector ---
            | "@Vector"
            | "@splat"
            | "@reduce"
            | "@shuffle"
            | "@select"
            // --- Comptime ---
            | "@inComptime"
            | "@setEvalBranchQuota"
            | "@setFloatMode"
            | "@setRuntimeSafety"
            | "@setAlignStack"
            // --- Frame / async ---
            | "@Frame"
            | "@frame"
            | "@asyncCall"
            | "@wasmMemorySize"
            | "@wasmMemoryGrow"
            // --- Miscellaneous ---
            | "@call"
            | "@constCast"
            | "@volatileCast"
            | "@workItemId"
            | "@workGroupId"
            | "@workGroupSize"
            // std namespace root — also matched by externals
            | "std"
            | "builtin"
    )
}
