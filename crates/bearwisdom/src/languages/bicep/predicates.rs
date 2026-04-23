// =============================================================================
// bicep/builtins.rs — Bicep builtin and helper predicates
// =============================================================================

use crate::types::EdgeKind;

/// Check that the edge kind is compatible with the symbol kind.
pub(super) fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::Calls => matches!(sym_kind, "method" | "function" | "constructor"),
        EdgeKind::TypeRef => matches!(
            sym_kind,
            "class" | "interface" | "enum" | "type_alias" | "variable" | "function"
        ),
        EdgeKind::Instantiates => matches!(sym_kind, "class" | "function"),
        _ => true,
    }
}

/// Bicep built-in functions that are never in the project symbol index.
pub(super) fn is_bicep_builtin(name: &str) -> bool {
    matches!(
        name,
        // ── Scope / deployment functions ─────────────────────────────────────
        "resourceGroup"
            | "subscription"
            | "tenant"
            | "managementGroup"
            | "deployment"
            | "environment"
            // ── Resource functions ────────────────────────────────────────────
            | "reference"
            | "resourceId"
            | "subscriptionResourceId"
            | "tenantResourceId"
            | "extensionResourceId"
            | "list"
            | "listKeys"
            | "listSecrets"
            // ── String functions ──────────────────────────────────────────────
            | "format"
            | "concat"
            | "contains"
            | "empty"
            | "first"
            | "last"
            | "indexOf"
            | "join"
            | "length"
            | "replace"
            | "skip"
            | "split"
            | "startsWith"
            | "endsWith"
            | "toLower"
            | "toUpper"
            | "trim"
            | "uniqueString"
            | "uri"
            | "uriComponent"
            | "uriComponentToString"
            | "substring"
            | "lastIndexOf"
            | "padLeft"
            // ── Encoding functions ────────────────────────────────────────────
            | "base64"
            | "base64ToJson"
            | "base64ToString"
            // ── Type conversion functions ─────────────────────────────────────
            | "json"
            | "string"
            | "int"
            | "bool"
            // ── Array / object functions ──────────────────────────────────────
            | "array"
            | "object"
            | "null"
            | "union"
            | "intersection"
            | "min"
            | "max"
            | "range"
            | "coalesce"
            | "if"
            | "any"
            | "createArray"
            | "flatten"
            | "filter"
            | "map"
            | "sort"
            | "reduce"
            | "toObject"
            | "items"
            | "objectKeys"
            | "values"
            // ── Numeric functions ─────────────────────────────────────────────
            | "add"
            | "sub"
            | "mul"
            | "div"
            | "mod"
            // ── Date / time functions ─────────────────────────────────────────
            | "dateTimeAdd"
            | "utcNow"
            | "dateTimeToEpoch"
            | "dateTimeFromEpoch"
            // ── Unique ID / GUID functions ────────────────────────────────────
            | "newGuid"
            | "guid"
            // ── File loading functions ────────────────────────────────────────
            | "loadTextContent"
            | "loadFileAsBase64"
            | "loadJsonContent"
            | "loadYamlContent"
            | "readEnvironmentVariable"
            // ── Type check / reflection ───────────────────────────────────────
            | "getType"
            | "isObject"
            | "isArray"
            | "isString"
            | "isInt"
            | "isBool"
            // ── Namespace aliases ─────────────────────────────────────────────
            | "sys"
            | "az"
            // ── Decorators (language builtins, not library API) ───────────────
            // Used as `@description('...')`, `@minLength(3)`, etc. The ref the
            // extractor emits targets the bare name without the `@`.
            | "description"
            | "minLength"
            | "maxLength"
            | "minValue"
            | "maxValue"
            | "allowed"
            | "secure"
            | "metadata"
            | "batchSize"
            | "export"
            | "sealed"
            | "discriminator"
    )
}
