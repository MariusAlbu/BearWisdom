// =============================================================================
// powershell/builtins.rs — PowerShell builtin and helper predicates
// =============================================================================

use crate::types::EdgeKind;

/// Check that the edge kind is compatible with the symbol kind.
pub(super) fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::Calls => matches!(sym_kind, "method" | "function" | "constructor" | "test" | "class"),
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

/// PowerShell built-in cmdlets that are never in the project index.
pub(super) fn is_powershell_builtin(name: &str) -> bool {
    matches!(
        name,
        "Write-Host"
            | "Write-Output"
            | "Get-Item"
            | "Set-Item"
            | "Get-Content"
            | "Set-Content"
            | "ForEach-Object"
            | "Where-Object"
            | "Select-Object"
            | "Sort-Object"
            | "Group-Object"
            | "Measure-Object"
            | "New-Object"
            | "Add-Member"
            | "Get-Process"
            | "Start-Process"
            | "Get-Service"
            | "Invoke-Command"
            | "Invoke-WebRequest"
            | "ConvertTo-Json"
            | "ConvertFrom-Json"
            | "Out-File"
            | "Test-Path"
            | "Join-Path"
            | "Split-Path"
            | "Resolve-Path"
    )
}
