// =============================================================================
// powershell/primitives.rs — PowerShell primitive and built-in types
// =============================================================================

/// Primitive and built-in type/function names for PowerShell.
pub(crate) const PRIMITIVES: &[&str] = &[
    // output cmdlets
    "Write-Host", "Write-Output", "Write-Error", "Write-Warning",
    "Write-Verbose", "Write-Debug", "Write-Information", "Write-Progress",
    "Read-Host",
    // content cmdlets
    "Get-Content", "Set-Content", "Add-Content", "Clear-Content",
    "Out-File", "Out-Null", "Out-String", "Out-GridView",
    // item cmdlets
    "Get-Item", "Set-Item", "New-Item", "Remove-Item",
    "Copy-Item", "Move-Item", "Rename-Item",
    "Test-Path", "Resolve-Path", "Split-Path", "Join-Path",
    "Get-ChildItem", "Get-Location", "Set-Location",
    "Push-Location", "Pop-Location",
    // variable cmdlets
    "Get-Variable", "Set-Variable", "New-Variable",
    "Remove-Variable", "Clear-Variable",
    // process / service cmdlets
    "Get-Process", "Stop-Process", "Start-Process", "Wait-Process",
    "Get-Service", "Start-Service", "Stop-Service", "Restart-Service",
    // introspection
    "Get-Command", "Get-Help", "Get-Module",
    "Import-Module", "Remove-Module", "Get-Member",
    // object pipeline cmdlets
    "Select-Object", "Where-Object", "ForEach-Object",
    "Sort-Object", "Group-Object", "Measure-Object",
    "Compare-Object", "Tee-Object",
    // formatting
    "Format-List", "Format-Table", "Format-Wide", "Format-Custom",
    // conversion / serialisation
    "ConvertTo-Json", "ConvertFrom-Json",
    "ConvertTo-Csv", "ConvertFrom-Csv",
    "ConvertTo-Xml", "ConvertTo-Html",
    "Export-Csv", "Import-Csv",
    // remote / expression
    "Invoke-Command", "Invoke-Expression",
    "Invoke-RestMethod", "Invoke-WebRequest",
    // object creation
    "New-Object", "Add-Member", "Add-Type",
    // date / random
    "Get-Date", "New-TimeSpan", "Get-Random", "Get-Unique",
    // WMI / CIM
    "Get-WmiObject", "Get-CimInstance", "New-CimSession",
    // event log
    "Get-EventLog", "Get-WinEvent",
    // credential
    "Get-Credential",
    // jobs
    "Start-Job", "Get-Job", "Receive-Job", "Wait-Job", "Remove-Job",
    // misc
    "Start-Sleep", "Get-ComputerName",
    "Set-StrictMode", "Set-ExecutionPolicy", "Get-ExecutionPolicy",
    "Enter-PSSession", "Exit-PSSession", "New-PSSession", "Remove-PSSession",
    "Export-ModuleMember",
    // language keywords
    "Try", "Catch", "Finally", "Throw", "Trap",
    "Begin", "Process", "End", "Clean",
    "Param", "DynamicParam",
    "Return", "Break", "Continue", "Exit",
    "Switch", "If", "ElseIf", "Else",
    "For", "ForEach", "While", "Do", "Until",
    "Class", "Enum", "Using",
    // Pester test framework globals
    "Describe", "Context", "It",
    "BeforeAll", "AfterAll", "BeforeEach", "AfterEach",
    "Should", "Mock", "Assert-MockCalled", "InModuleScope",
    "New-MockObject", "Set-ItResult", "BeforeDiscovery",
    // Pester assertion operators (used as -Be, -BeTrue, etc.)
    "-Be", "-BeExactly", "-BeGreaterThan", "-BeLessThan",
    "-BeIn", "-BeOfType", "-BeTrue", "-BeFalse", "-BeNullOrEmpty",
    "-Contain", "-Exist", "-FileContentMatch",
    "-HaveCount", "-HaveParameter",
    "-Match", "-MatchExactly", "-Throw", "-Not", "-BeNull",
];
