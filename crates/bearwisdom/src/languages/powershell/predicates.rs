// =============================================================================
// powershell/predicates.rs — PowerShell builtin and helper predicates
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
        // Output
        "Write-Host"
            | "Write-Output"
            | "Write-Verbose"
            | "Write-Warning"
            | "Write-Error"
            | "Write-Debug"
            | "Read-Host"
            | "Out-Null"
            | "Out-File"
            | "Out-String"
            // Item / path
            | "Get-Item"
            | "Set-Item"
            | "New-Item"
            | "Remove-Item"
            | "Copy-Item"
            | "Move-Item"
            | "Rename-Item"
            | "Test-Path"
            | "Join-Path"
            | "Split-Path"
            | "Resolve-Path"
            // Content
            | "Get-Content"
            | "Set-Content"
            | "Add-Content"
            // Registry / item property
            | "Get-ItemProperty"
            | "Set-ItemProperty"
            | "Remove-ItemProperty"
            | "New-ItemProperty"
            // Service management
            | "Get-Service"
            | "Set-Service"
            | "Start-Service"
            | "Stop-Service"
            | "Restart-Service"
            // Process management
            | "Get-Process"
            | "Start-Process"
            | "Stop-Process"
            // Pipeline / object manipulation
            | "ForEach-Object"
            | "Where-Object"
            | "Select-Object"
            | "Sort-Object"
            | "Group-Object"
            | "Measure-Object"
            | "Compare-Object"
            | "New-Object"
            | "Add-Member"
            | "Get-Member"
            // Directory
            | "Get-ChildItem"
            | "Set-Location"
            | "Get-Location"
            // Network / web
            | "Invoke-Command"
            | "Invoke-WebRequest"
            | "Invoke-RestMethod"
            // WMI / CIM
            | "Get-WmiObject"
            | "Get-CimInstance"
            // Serialisation
            | "ConvertTo-Json"
            | "ConvertFrom-Json"
            | "ConvertTo-Html"
            | "Export-Csv"
            | "Import-Csv"
            // Date / misc
            | "Get-Date"
            | "Add-Type"
            // Module / discovery
            | "Import-Module"
            | "Get-Command"
            | "Get-Help"
            | "Get-Module"
            // Environment control
            | "Set-StrictMode"
            | "Set-ExecutionPolicy"
    )
}
