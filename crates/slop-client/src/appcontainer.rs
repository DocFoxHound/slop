//! Sandbox mode on Windows (DESIGN §6, SECURITY §6.2): the app runs in an AppContainer, the box
//! Windows gives Store apps. A process in one can only open files and folders whose permissions
//! name its container (or all app packages, as Windows' own files do), so it can't see the
//! player's documents, other apps or Slop. Slop gives its container read access to the app's
//! files and mods, and write access to its save and cache folders; a mods folder inside the
//! saves gets a deny-write entry on top. The network is a capability: given unless the player
//! blocked it.
//!
//! The Slop app starts a copy of itself as the launcher (`--slop-sandbox <spec>`, so the
//! command still looks like any other to the caller), which sets this up, starts the app in a
//! job object that ends it if the launcher goes, waits, and exits with the app's exit code.

use std::{ffi::OsStr, io, os::windows::ffi::OsStrExt, path::Path, ptr};

use windows_sys::Win32::{
    Foundation::{CloseHandle, ERROR_SUCCESS, HANDLE, LocalFree},
    Security::{
        ACL,
        Authorization::{
            DENY_ACCESS, EXPLICIT_ACCESS_W, GRANT_ACCESS, GetExplicitEntriesFromAclW,
            GetNamedSecurityInfoW, NO_MULTIPLE_TRUSTEE, SE_FILE_OBJECT, SetEntriesInAclW,
            SetNamedSecurityInfoW, TRUSTEE_IS_SID, TRUSTEE_IS_UNKNOWN, TRUSTEE_W,
        },
        CreateWellKnownSid, DACL_SECURITY_INFORMATION, EqualSid, FreeSid,
        Isolation::{
            CreateAppContainerProfile, DeleteAppContainerProfile,
            DeriveAppContainerSidFromAppContainerName,
        },
        PSID, SECURITY_CAPABILITIES, SECURITY_MAX_SID_SIZE, SID_AND_ATTRIBUTES,
        SUB_CONTAINERS_AND_OBJECTS_INHERIT, WinCapabilityInternetClientServerSid,
        WinCapabilityInternetClientSid, WinCapabilityPrivateNetworkClientServerSid,
    },
    System::{
        JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOB_OBJECT_UILIMIT_DESKTOP, JOB_OBJECT_UILIMIT_EXITWINDOWS,
            JOBOBJECT_BASIC_UI_RESTRICTIONS, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JobObjectBasicUIRestrictions, JobObjectExtendedLimitInformation,
            SetInformationJobObject,
        },
        SystemServices::SE_GROUP_ENABLED,
        Threading::{
            CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, CreateProcessW,
            DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess,
            INFINITE, InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST,
            PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES, PROCESS_INFORMATION, ResumeThread,
            STARTUPINFOEXW, TerminateProcess, UpdateProcThreadAttribute, WaitForSingleObject,
        },
    },
};

use crate::sandbox::Spec;

/// Read and run: list folders, read files and their attributes, run programs.
const READ: u32 = 0x0012_00A9;
/// Read, write and delete, but not delete-child, so a folder's deny entries hold.
const MODIFY: u32 = 0x0013_01BF;
/// Write data, append, write attributes, delete, delete children, change permissions or owner.
const NO_WRITE: u32 = 0x2 | 0x4 | 0x10 | 0x100 | 0x40 | 0x1_0000 | 0x4_0000 | 0x8_0000;
/// HRESULT_FROM_WIN32(ERROR_ALREADY_EXISTS).
const ALREADY_EXISTS: i32 = 0x8007_00B7_u32 as i32;

fn wide(s: impl AsRef<OsStr>) -> Vec<u16> {
    s.as_ref().encode_wide().chain([0]).collect()
}

fn hr(what: &str, code: i32) -> io::Error {
    io::Error::other(format!("{what} failed (0x{:08x})", code as u32))
}

/// The app's container SID. Freed when dropped.
struct ContainerSid(PSID);

impl Drop for ContainerSid {
    fn drop(&mut self) {
        unsafe { FreeSid(self.0) };
    }
}

/// Creates the app's container, or finds the one made before.
fn container(name: &str) -> io::Result<ContainerSid> {
    let (wname, display) = (wide(name), wide(format!("Slop sandbox: {name}")));
    let mut sid: PSID = ptr::null_mut();
    let res = unsafe {
        CreateAppContainerProfile(
            wname.as_ptr(),
            display.as_ptr(),
            display.as_ptr(),
            ptr::null(),
            0,
            &mut sid,
        )
    };
    if res == ALREADY_EXISTS {
        let res = unsafe { DeriveAppContainerSidFromAppContainerName(wname.as_ptr(), &mut sid) };
        if res != 0 {
            return Err(hr("finding the sandbox", res));
        }
    } else if res != 0 {
        return Err(hr("making the sandbox", res));
    }
    Ok(ContainerSid(sid))
}

/// Whether AppContainers work here: makes (or finds) Slop's test container.
pub fn check() -> io::Result<()> {
    container("slop.check").map(drop)
}

/// Removes an app's container (when the app is uninstalled). Missing is fine.
pub fn remove(name: &str) {
    let wname = wide(name);
    unsafe { DeleteAppContainerProfile(wname.as_ptr()) };
}

/// Whether the folder's own permissions already have this entry. Then what's in it inherited it
/// too, and a launch needn't walk the whole folder again. A folder Slop replaces (an update, a
/// restored save) comes without it.
fn has_entry(acl: *const ACL, sid: PSID, access: u32, mode: i32) -> bool {
    if acl.is_null() {
        return false;
    }
    let mut count = 0u32;
    let mut entries: *mut EXPLICIT_ACCESS_W = ptr::null_mut();
    if unsafe { GetExplicitEntriesFromAclW(acl, &mut count, &mut entries) } != ERROR_SUCCESS {
        return false;
    }
    let found = (0..count as usize).any(|i| {
        let e = unsafe { &*entries.add(i) };
        e.Trustee.TrusteeForm == TRUSTEE_IS_SID
            && e.grfAccessMode == mode
            && e.grfAccessPermissions & access == access
            && e.grfInheritance & SUB_CONTAINERS_AND_OBJECTS_INHERIT
                == SUB_CONTAINERS_AND_OBJECTS_INHERIT
            && unsafe { EqualSid(e.Trustee.ptstrName.cast(), sid) } != 0
    });
    unsafe { LocalFree(entries.cast()) };
    found
}

/// Adds an inheritable entry for `sid` to a folder's permissions, unless it's there.
fn allow(path: &Path, sid: PSID, access: u32, mode: i32) -> io::Result<()> {
    let wpath = wide(path);
    let mut old: *mut ACL = ptr::null_mut();
    let mut descriptor = ptr::null_mut();
    let err = unsafe {
        GetNamedSecurityInfoW(
            wpath.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            &mut old,
            ptr::null_mut(),
            &mut descriptor,
        )
    };
    if err != ERROR_SUCCESS {
        return Err(io::Error::from_raw_os_error(err as i32));
    }
    if has_entry(old, sid, access, mode) {
        unsafe { LocalFree(descriptor) };
        return Ok(());
    }
    let entry = EXPLICIT_ACCESS_W {
        grfAccessPermissions: access,
        grfAccessMode: mode,
        grfInheritance: SUB_CONTAINERS_AND_OBJECTS_INHERIT,
        Trustee: TRUSTEE_W {
            pMultipleTrustee: ptr::null_mut(),
            MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
            TrusteeForm: TRUSTEE_IS_SID,
            TrusteeType: TRUSTEE_IS_UNKNOWN,
            ptstrName: sid.cast(),
        },
    };
    let mut new: *mut ACL = ptr::null_mut();
    let err = unsafe { SetEntriesInAclW(1, &entry, old, &mut new) };
    let result = if err != ERROR_SUCCESS {
        Err(io::Error::from_raw_os_error(err as i32))
    } else {
        // Setting the folder's list also passes inheritable entries down to what's in it.
        let err = unsafe {
            SetNamedSecurityInfoW(
                wpath.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                ptr::null_mut(),
                ptr::null_mut(),
                new,
                ptr::null(),
            )
        };
        unsafe { LocalFree(new.cast()) };
        if err != ERROR_SUCCESS {
            Err(io::Error::from_raw_os_error(err as i32))
        } else {
            Ok(())
        }
    };
    unsafe { LocalFree(descriptor) };
    result
}

/// A well-known capability SID, in a buffer the caller keeps alive.
fn capability(kind: i32) -> io::Result<Vec<u8>> {
    let mut buf = vec![0u8; SECURITY_MAX_SID_SIZE as usize];
    let mut size = SECURITY_MAX_SID_SIZE;
    if unsafe { CreateWellKnownSid(kind, ptr::null_mut(), buf.as_mut_ptr().cast(), &mut size) } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(buf)
}

struct Handle(HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CloseHandle(self.0) };
        }
    }
}

/// A job that ends the app (and anything it starts) when the launcher goes, and keeps it from
/// logging the player off or switching desktops.
fn job() -> io::Result<Handle> {
    let job = Handle(unsafe { CreateJobObjectW(ptr::null(), ptr::null()) });
    if job.0.is_null() {
        return Err(io::Error::last_os_error());
    }
    let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    let ui = JOBOBJECT_BASIC_UI_RESTRICTIONS {
        UIRestrictionsClass: JOB_OBJECT_UILIMIT_EXITWINDOWS | JOB_OBJECT_UILIMIT_DESKTOP,
    };
    unsafe {
        if SetInformationJobObject(
            job.0,
            JobObjectExtendedLimitInformation,
            (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        ) == 0
            || SetInformationJobObject(
                job.0,
                JobObjectBasicUIRestrictions,
                (&ui as *const JOBOBJECT_BASIC_UI_RESTRICTIONS).cast(),
                size_of::<JOBOBJECT_BASIC_UI_RESTRICTIONS>() as u32,
            ) == 0
        {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(job)
}

/// The launcher's work: sets up the container, starts the app in it and waits. Returns the
/// app's exit code.
pub fn run(spec: &Spec) -> io::Result<u32> {
    let sid = container(&spec.name)?;
    for dir in &spec.read {
        allow(dir, sid.0, READ, GRANT_ACCESS)?;
    }
    for dir in &spec.write {
        allow(dir, sid.0, MODIFY, GRANT_ACCESS)?;
    }
    for dir in &spec.read_only {
        allow(dir, sid.0, READ, GRANT_ACCESS)?;
        allow(dir, sid.0, NO_WRITE, DENY_ACCESS)?;
    }

    let mut caps_buf = Vec::new();
    if spec.network {
        for kind in [
            WinCapabilityInternetClientSid,
            WinCapabilityInternetClientServerSid,
            WinCapabilityPrivateNetworkClientServerSid,
        ] {
            caps_buf.push(capability(kind)?);
        }
    }
    let mut caps: Vec<SID_AND_ATTRIBUTES> = caps_buf
        .iter_mut()
        .map(|b| SID_AND_ATTRIBUTES {
            Sid: b.as_mut_ptr().cast(),
            Attributes: SE_GROUP_ENABLED as u32,
        })
        .collect();
    let security = SECURITY_CAPABILITIES {
        AppContainerSid: sid.0,
        Capabilities: if caps.is_empty() {
            ptr::null_mut()
        } else {
            caps.as_mut_ptr()
        },
        CapabilityCount: caps.len() as u32,
        Reserved: 0,
    };

    let mut size = 0usize;
    unsafe { InitializeProcThreadAttributeList(ptr::null_mut(), 1, 0, &mut size) };
    let mut list_buf = vec![0u8; size];
    let list: LPPROC_THREAD_ATTRIBUTE_LIST = list_buf.as_mut_ptr().cast();
    if unsafe { InitializeProcThreadAttributeList(list, 1, 0, &mut size) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let result = (|| {
        if unsafe {
            UpdateProcThreadAttribute(
                list,
                0,
                PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES as usize,
                (&security as *const SECURITY_CAPABILITIES).cast(),
                size_of::<SECURITY_CAPABILITIES>(),
                ptr::null_mut(),
                ptr::null(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        let mut info: STARTUPINFOEXW = unsafe { std::mem::zeroed() };
        info.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
        info.lpAttributeList = list;
        let program = wide(&spec.program);
        let mut line = wide(command_line(&spec.program, &spec.args));
        let cwd = wide(&spec.cwd);
        let mut process: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
        let job = job()?;
        // Started paused, so it's in the job before it can start anything else.
        if unsafe {
            CreateProcessW(
                program.as_ptr(),
                line.as_mut_ptr(),
                ptr::null(),
                ptr::null(),
                0,
                EXTENDED_STARTUPINFO_PRESENT | CREATE_SUSPENDED | CREATE_UNICODE_ENVIRONMENT,
                ptr::null(),
                cwd.as_ptr(),
                &info.StartupInfo,
                &mut process,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        let (proc_h, thread_h) = (Handle(process.hProcess), Handle(process.hThread));
        if unsafe { AssignProcessToJobObject(job.0, proc_h.0) } == 0 {
            let err = io::Error::last_os_error();
            unsafe { TerminateProcess(proc_h.0, 1) };
            return Err(err);
        }
        unsafe { ResumeThread(thread_h.0) };
        unsafe { WaitForSingleObject(proc_h.0, INFINITE) };
        let mut code = 0u32;
        unsafe { GetExitCodeProcess(proc_h.0, &mut code) };
        Ok(code)
    })();
    unsafe { DeleteProcThreadAttributeList(list) };
    result
}

/// Windows command-line quoting, as the C runtime reads it.
pub(crate) fn command_line(program: &Path, args: &[String]) -> String {
    std::iter::once(program.to_string_lossy().into_owned())
        .chain(args.iter().cloned())
        .map(|a| crate::sandbox::quote_windows_arg(&a))
        .collect::<Vec<_>>()
        .join(" ")
}
