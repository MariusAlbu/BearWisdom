// =============================================================================
// ecosystem/powershell_cmdlet_types_tests.rs — Tests for cmdlet → .NET type table
// =============================================================================

use super::{cmdlet_result_module_tag, cmdlet_return_type, is_known_cmdlet};

// ---------------------------------------------------------------------------
// cmdlet_return_type — table coverage
// ---------------------------------------------------------------------------

#[test]
fn test_get_date_maps_to_datetime() {
    assert_eq!(
        cmdlet_return_type("Get-Date"),
        Some("System.DateTime"),
    );
}

#[test]
fn test_get_childitem_maps_to_filesysteminfo() {
    assert_eq!(
        cmdlet_return_type("Get-ChildItem"),
        Some("System.IO.FileSystemInfo"),
    );
}

#[test]
fn test_get_process_maps_to_process() {
    assert_eq!(
        cmdlet_return_type("Get-Process"),
        Some("System.Diagnostics.Process"),
    );
}

#[test]
fn test_get_service_maps_to_servicecontroller() {
    assert_eq!(
        cmdlet_return_type("Get-Service"),
        Some("System.ServiceProcess.ServiceController"),
    );
}

#[test]
fn test_get_content_maps_to_string() {
    assert_eq!(
        cmdlet_return_type("Get-Content"),
        Some("System.String"),
    );
}

#[test]
fn test_get_member_maps_to_psmemberinfo() {
    assert_eq!(
        cmdlet_return_type("Get-Member"),
        Some("System.Management.Automation.PSMemberInfo"),
    );
}

#[test]
fn test_get_wmiobject_maps_to_managementobject() {
    assert_eq!(
        cmdlet_return_type("Get-WmiObject"),
        Some("System.Management.ManagementObject"),
    );
}

#[test]
fn test_get_ciminstance_maps_to_managementobject() {
    assert_eq!(
        cmdlet_return_type("Get-CimInstance"),
        Some("System.Management.ManagementObject"),
    );
}

// ---------------------------------------------------------------------------
// Case-insensitivity
// ---------------------------------------------------------------------------

#[test]
fn test_case_insensitive_uppercase() {
    // All-caps input
    assert_eq!(cmdlet_return_type("GET-DATE"), Some("System.DateTime"));
}

#[test]
fn test_case_insensitive_lowercase() {
    assert_eq!(cmdlet_return_type("get-date"), Some("System.DateTime"));
}

#[test]
fn test_case_insensitive_mixed() {
    assert_eq!(cmdlet_return_type("Get-Process"), cmdlet_return_type("GET-PROCESS"));
}

// ---------------------------------------------------------------------------
// Unknown cmdlets
// ---------------------------------------------------------------------------

#[test]
fn test_unknown_cmdlet_returns_none() {
    assert_eq!(cmdlet_return_type("Invoke-WebRequest"), None);
    assert_eq!(cmdlet_return_type("Write-Host"), None);
    assert_eq!(cmdlet_return_type("Unknown-Cmdlet"), None);
    assert_eq!(cmdlet_return_type(""), None);
}

// ---------------------------------------------------------------------------
// is_known_cmdlet
// ---------------------------------------------------------------------------

#[test]
fn test_is_known_cmdlet_true_for_mapped() {
    assert!(is_known_cmdlet("Get-Date"));
    assert!(is_known_cmdlet("Get-ChildItem"));
    assert!(is_known_cmdlet("Get-Service"));
}

#[test]
fn test_is_known_cmdlet_false_for_unknown() {
    assert!(!is_known_cmdlet("Invoke-Command"));
    assert!(!is_known_cmdlet("Write-Output"));
}

// ---------------------------------------------------------------------------
// cmdlet_result_module_tag
// ---------------------------------------------------------------------------

#[test]
fn test_module_tag_get_date() {
    assert_eq!(cmdlet_result_module_tag("Get-Date"), "__cmdlet_get_date");
}

#[test]
fn test_module_tag_get_childitem() {
    assert_eq!(cmdlet_result_module_tag("Get-ChildItem"), "__cmdlet_get_childitem");
}

#[test]
fn test_module_tag_lowercases_and_replaces_dash() {
    assert_eq!(
        cmdlet_result_module_tag("Get-Process"),
        "__cmdlet_get_process",
    );
}

#[test]
fn test_module_tag_idempotent_on_lowercase() {
    // Already lowercase with underscore — same result
    assert_eq!(
        cmdlet_result_module_tag("get-process"),
        "__cmdlet_get_process",
    );
}
