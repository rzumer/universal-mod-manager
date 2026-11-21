//! Win32 pieces: job objects, resuming a suspended process, listing processes, Ctrl+C.

use std::ffi::c_void;
use std::io;
use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicUsize, Ordering};
use windows_sys::Win32::Foundation::{CloseHandle, BOOL, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Console::{SetConsoleCtrlHandler, CTRL_BREAK_EVENT, CTRL_C_EVENT};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, Thread32First, Thread32Next,
    PROCESSENTRY32W, TH32CS_SNAPPROCESS, TH32CS_SNAPTHREAD, THREADENTRY32,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectBasicAccountingInformation,
    QueryInformationJobObject, JOBOBJECT_BASIC_ACCOUNTING_INFORMATION,
};
use windows_sys::Win32::System::Threading::{OpenThread, ResumeThread, THREAD_SUSPEND_RESUME};

pub use windows_sys::Win32::System::Threading::CREATE_SUSPENDED;

/// Owned handle, closed on drop.
struct Handle(HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

pub struct Job(Handle);

impl Job {
    pub fn new() -> io::Result<Job> {
        // No KILL_ON_JOB_CLOSE: if the launcher dies, the game keeps running and recovery
        // reverts once it has exited.
        let h = unsafe { CreateJobObjectW(null(), null()) };
        if h.is_null() {
            return Err(io::Error::last_os_error());
        }
        Ok(Job(Handle(h)))
    }

    pub fn assign(&self, process: HANDLE) -> io::Result<()> {
        if unsafe { AssignProcessToJobObject(self.0 .0, process) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    pub fn active_processes(&self) -> io::Result<u32> {
        let mut info: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = unsafe { zeroed() };
        let ok = unsafe {
            QueryInformationJobObject(
                self.0 .0,
                JobObjectBasicAccountingInformation,
                &mut info as *mut _ as *mut c_void,
                size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                null_mut(),
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(info.ActiveProcesses)
    }
}

fn snapshot(flags: u32) -> io::Result<Handle> {
    let h = unsafe { CreateToolhelp32Snapshot(flags, 0) };
    if h == INVALID_HANDLE_VALUE || h.is_null() {
        return Err(io::Error::last_os_error());
    }
    Ok(Handle(h))
}

/// Resumes every thread of a process created with `CREATE_SUSPENDED` (it has exactly one).
pub fn resume_process(pid: u32) -> io::Result<()> {
    let snap = snapshot(TH32CS_SNAPTHREAD)?;
    let mut entry: THREADENTRY32 = unsafe { zeroed() };
    entry.dwSize = size_of::<THREADENTRY32>() as u32;
    let mut resumed = 0;
    let mut more = unsafe { Thread32First(snap.0, &mut entry) } != 0;
    while more {
        if entry.th32OwnerProcessID == pid {
            let t = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID) };
            if t.is_null() {
                return Err(io::Error::last_os_error());
            }
            let t = Handle(t);
            if unsafe { ResumeThread(t.0) } == u32::MAX {
                return Err(io::Error::last_os_error());
            }
            resumed += 1;
        }
        more = unsafe { Thread32Next(snap.0, &mut entry) } != 0;
    }
    if resumed == 0 {
        return Err(io::Error::other(format!(
            "no threads found for process {pid}"
        )));
    }
    Ok(())
}

pub fn process_names() -> io::Result<Vec<String>> {
    let snap = snapshot(TH32CS_SNAPPROCESS)?;
    let mut entry: PROCESSENTRY32W = unsafe { zeroed() };
    entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
    let mut out = Vec::new();
    let mut more = unsafe { Process32FirstW(snap.0, &mut entry) } != 0;
    while more {
        let len = entry
            .szExeFile
            .iter()
            .position(|c| *c == 0)
            .unwrap_or(entry.szExeFile.len());
        out.push(String::from_utf16_lossy(&entry.szExeFile[..len]));
        more = unsafe { Process32NextW(snap.0, &mut entry) } != 0;
    }
    Ok(out)
}

static INTERRUPTS: AtomicUsize = AtomicUsize::new(0);

unsafe extern "system" fn on_ctrl(kind: u32) -> BOOL {
    if kind == CTRL_C_EVENT || kind == CTRL_BREAK_EVENT {
        INTERRUPTS.fetch_add(1, Ordering::SeqCst);
        1
    } else {
        0
    }
}

pub fn catch_interrupts() -> &'static AtomicUsize {
    unsafe { SetConsoleCtrlHandler(Some(on_ctrl), 1) };
    &INTERRUPTS
}
