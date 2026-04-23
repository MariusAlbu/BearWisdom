// =============================================================================
// memory_cap — Windows Job Object memory limit for the current process
//
// Binds the current process to a new Job Object with
// `JOB_OBJECT_LIMIT_PROCESS_MEMORY` set. If the process tries to commit more
// memory than the cap, Windows fails the allocation; Rust's default alloc
// error handler then aborts with a clean panic instead of letting the
// system-wide working-set swell until the desktop is unresponsive.
//
// Used as a safety harness for indexer memory regression testing: set
// `BEARWISDOM_MEMORY_CAP_MB=3072` and any pathological allocation dies at
// 3 GB instead of paging out the user's entire machine.
//
// No-op on non-Windows platforms.
// =============================================================================

/// Read `BEARWISDOM_MEMORY_CAP_MB` and install a Windows Job Object cap at
/// that many MiB. Unset or unparseable values leave the process uncapped.
pub fn install_from_env() {
    let Ok(raw) = std::env::var("BEARWISDOM_MEMORY_CAP_MB") else {
        return;
    };
    let Ok(mb) = raw.trim().parse::<u64>() else {
        eprintln!(
            "[memory_cap] ignoring BEARWISDOM_MEMORY_CAP_MB={raw:?} — not a non-negative integer"
        );
        return;
    };
    if mb == 0 {
        return;
    }
    match install_cap_mb(mb) {
        Ok(()) => eprintln!("[memory_cap] process memory capped at {mb} MiB"),
        Err(e) => eprintln!("[memory_cap] failed to install cap: {e}"),
    }
}

#[cfg(windows)]
pub fn install_cap_mb(mb: u64) -> std::io::Result<()> {
    use std::mem::size_of;
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, SetInformationJobObject,
        JobObjectExtendedLimitInformation, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_PROCESS_MEMORY,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;

    let bytes = (mb as usize).saturating_mul(1024 * 1024);

    unsafe {
        let job = CreateJobObjectW(std::ptr::null_mut(), std::ptr::null());
        if job.is_null() || job == INVALID_HANDLE_VALUE {
            return Err(std::io::Error::last_os_error());
        }

        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_PROCESS_MEMORY;
        info.ProcessMemoryLimit = bytes;

        let ok = SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const core::ffi::c_void,
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        );
        if ok == 0 {
            let e = std::io::Error::last_os_error();
            let _ = CloseHandle(job);
            return Err(e);
        }

        let ok = AssignProcessToJobObject(job, GetCurrentProcess());
        if ok == 0 {
            let e = std::io::Error::last_os_error();
            let _ = CloseHandle(job);
            return Err(e);
        }

        // Intentionally leave `job` un-closed for the process lifetime.
        // The handle is just a void* (Copy), so there's no Drop to sidestep —
        // the point is that we never call `CloseHandle(job)`. If we did,
        // and nobody else held a reference, the job would dissolve and the
        // memory limit with it.
        let _ = job;
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn install_cap_mb(_mb: u64) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "memory_cap is Windows-only",
    ))
}
