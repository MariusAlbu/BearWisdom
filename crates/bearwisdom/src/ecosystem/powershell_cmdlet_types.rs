// =============================================================================
// ecosystem/powershell_cmdlet_types.rs — PowerShell cmdlet → .NET return type
//
// Maps common PowerShell cmdlets to the .NET type they return, so that
// member-access refs on cmdlet results (`(Get-Date).ToString()`,
// `(Get-ChildItem).Extension`) can be routed to `dotnet-stdlib` rather than
// landing as unresolved_refs.
//
// Used by:
//   languages/powershell/extract.rs — emits sentinels for pipeline-var and
//     cmdlet-result patterns.
//   languages/powershell/resolve.rs — inherited via the sentinel mechanism;
//     no direct use here.
//
// Per feedback_no_hardcoded_library_builtins: this table belongs in
// ecosystem/, not in language predicates. It describes .NET API surface
// that happens to be the return type of PowerShell cmdlets.
// =============================================================================

/// Map a PowerShell cmdlet name (case-insensitive) to the fully-qualified .NET
/// type it returns, or `None` if the cmdlet is not in the table.
///
/// The type name is the canonical CLR name used for disambiguation; it does NOT
/// need to resolve through `dotnet_stdlib` — it is only used to decide whether
/// member accesses on cmdlet results should be classified as `dotnet-stdlib`
/// external refs.
pub fn cmdlet_return_type(cmdlet: &str) -> Option<&'static str> {
    // Case-insensitive match on the cmdlet name.
    match cmdlet.to_ascii_lowercase().as_str() {
        // Filesystem
        "get-childitem" | "get-item" | "dir" | "ls" => {
            Some("System.IO.FileSystemInfo")
        }
        "get-content" => Some("System.String"),

        // Processes
        "get-process" => Some("System.Diagnostics.Process"),

        // Services
        "get-service" => Some("System.ServiceProcess.ServiceController"),

        // WMI / CIM
        "get-wmiobject" | "get-ciminstance" => {
            Some("System.Management.ManagementObject")
        }

        // Date / time
        "get-date" => Some("System.DateTime"),

        // Reflection / members
        "get-member" => Some("System.Management.Automation.PSMemberInfo"),

        // Disk / partition
        "get-disk" => Some("Microsoft.Management.Infrastructure.CimInstance"),
        "get-partition" => Some("Microsoft.Management.Infrastructure.CimInstance"),
        "get-volume" => Some("Microsoft.Management.Infrastructure.CimInstance"),

        // Registry
        "get-itemproperty" => Some("System.Management.Automation.PSCustomObject"),

        // Jobs
        "start-job" | "get-job" => Some("System.Management.Automation.Job"),

        // Network
        "get-netadapter" => Some("Microsoft.Management.Infrastructure.CimInstance"),
        "get-nettcpconnection" => Some("Microsoft.Management.Infrastructure.CimInstance"),

        // WinUtil-specific
        "get-eventlog" => Some("System.Diagnostics.EventLogEntry"),

        _ => None,
    }
}

/// Returns `true` if `cmdlet` (case-insensitive) is in the return-type table.
pub fn is_known_cmdlet(cmdlet: &str) -> bool {
    cmdlet_return_type(cmdlet).is_some()
}

/// Synthetic module tag emitted for `(Get-Xxx).Member` chains.
///
/// The extractor emits this as the `module` field on the method/property
/// ref so the resolver can match it against the sentinel that `emit_dotnet_binding_sentinels`
/// places for the same pattern.
///
/// The tag encodes the cmdlet name in a way that is:
///   (a) unlikely to collide with a real variable name
///   (b) deterministic given the cmdlet name
pub fn cmdlet_result_module_tag(cmdlet: &str) -> String {
    format!("__cmdlet_{}", cmdlet.to_ascii_lowercase().replace('-', "_"))
}

#[cfg(test)]
#[path = "powershell_cmdlet_types_tests.rs"]
mod tests;
