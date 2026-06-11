// Copyright (c) 2026 Michael Grier

//! Debug pump thread — the single OS thread that owns all `CreateProcess`
//! calls made with `DEBUG_PROCESS` and drives the `WaitForDebugEvent` loop.
//!
//! Architecture (from DESIGN-NOTES.md):
//!
//! - One pump thread per `Pump` instance.
//! - Callers send a `SpawnRequest` over `tx`; the pump calls `CreateProcess`,
//!   then replies with a `SpawnResponse` (or error) over the per-request
//!   channel before re-entering `WaitForDebugEvent`.
//! - When a child exits the pump sends the classified `TerminationReason`
//!   back over the per-child `exit_tx` stored in `ChildEntry`.
//! - The thread exits when the `Pump` is dropped and no more children are
//!   tracked (the request sender is closed).

use std::collections::HashMap;
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::sync::mpsc;

use windows_sys::Win32::Foundation::{CloseHandle, FALSE, HANDLE, INVALID_HANDLE_VALUE, TRUE};
use windows_sys::Win32::System::Diagnostics::Debug::{
    ContinueDebugEvent, WaitForDebugEvent, CREATE_PROCESS_DEBUG_EVENT, CREATE_THREAD_DEBUG_EVENT,
    DEBUG_EVENT, EXCEPTION_DEBUG_EVENT, EXIT_PROCESS_DEBUG_EVENT, EXIT_THREAD_DEBUG_EVENT,
    LOAD_DLL_DEBUG_EVENT, OUTPUT_DEBUG_STRING_EVENT, RIP_EVENT, UNLOAD_DLL_DEBUG_EVENT,
};
use windows_sys::Win32::System::Threading::{
    CreateProcessW, DeleteProcThreadAttributeList, InitializeProcThreadAttributeList,
    CREATE_UNICODE_ENVIRONMENT, DEBUG_PROCESS, EXTENDED_STARTUPINFO_PRESENT,
    PROCESS_CREATION_FLAGS, PROCESS_INFORMATION, STARTF_USESTDHANDLES, STARTUPINFOEXW,
};

// ContinueDebugEvent status codes (NTSTATUS - i32 in windows-sys 0.59)
const DBG_CONTINUE: i32 = 0x00010002_u32 as i32;
const DBG_EXCEPTION_NOT_HANDLED: i32 = 0x80010001_u32 as i32;

// Exception codes (named constants — no bare integers in logic code)
// ExceptionCode in EXCEPTION_RECORD is i32 (NTSTATUS) in windows-sys 0.59.
mod exception_code {
    /// Access violation.
    pub const ACCESS_VIOLATION: i32 = 0xC000_0005_u32 as i32;
    /// Stack buffer overrun / __fastfail fast path.
    pub const STACK_BUFFER_OVERRUN: i32 = 0xC000_0409_u32 as i32;
    /// Fatal application exit / RtlFailFast.
    pub const FATAL_APP_EXIT: i32 = 0x4000_0015_i32;
    /// Debugger breakpoint (loader breakpoint at process attach).
    pub const BREAKPOINT: i32 = 0x8000_0003_u32 as i32;
    /// Single step.
    pub const SINGLE_STEP: i32 = 0x8000_0004_u32 as i32;
}

// Exception flags
mod exception_flag {
    /// This is a second-chance (unhandled) exception.
    pub const NON_CONTINUABLE: u32 = 0x0000_0001;
    pub const SECOND_CHANCE: u32 = 0x0000_0000; // first-chance is 0 in dwFirstChance
}

/// The full set of information the caller needs to send to the pump to
/// launch a child process.
pub struct SpawnRequest {
    /// Null-terminated UTF-16 application name (or None to derive from command line).
    pub application_name: Option<Vec<u16>>,
    /// Null-terminated UTF-16 command line.
    pub command_line: Vec<u16>,
    /// Null-terminated UTF-16 environment block (or None to inherit).
    pub environment: Option<Vec<u16>>,
    /// Null-terminated UTF-16 current directory (or None to inherit).
    pub current_directory: Option<Vec<u16>>,
    /// Requested process affinity mask (0 = don't set).
    pub affinity_mask: usize,
    /// Channel on which to send the `TerminationReason` when the child exits.
    pub exit_tx: mpsc::SyncSender<TerminationResult>,
    /// STARTUPINFOW fields — stdio handles.
    pub stdin_handle: HANDLE,
    pub stdout_handle: HANDLE,
    pub stderr_handle: HANDLE,
    pub use_stdio_handles: bool,
}

pub type TerminationResult = Result<crate::TerminationReason, std::io::Error>;

// SAFETY: HANDLE is a Win32 *mut c_void, but Windows guarantees it is safe
// to transfer kernel handles between threads in the same process.
unsafe impl Send for SpawnRequest {}

/// Reply from the pump after (or instead of) `CreateProcess`.
pub struct SpawnResponse {
    pub pid: u32,
    pub process_handle: HANDLE,
    pub thread_handle: HANDLE,
}

// SAFETY: same HANDLE-transfer rationale as for SpawnRequest.
unsafe impl Send for SpawnResponse {}

/// Internal per-child state tracked by the pump.
struct ChildEntry {
    pid: u32,
    process_handle: HANDLE,
    thread_handle: HANDLE,
    /// Last recorded second-chance exception, if any.
    pending_exception: Option<PendingException>,
    /// Whether we have passed the initial loader breakpoint.
    past_loader_bp: bool,
    /// Requested affinity mask (0 = don't set).
    affinity_mask: usize,
    /// Channel to notify the waiter when the child exits.
    exit_tx: mpsc::SyncSender<TerminationResult>,
}

struct PendingException {
    /// Raw NTSTATUS code from `EXCEPTION_RECORD.ExceptionCode` (i32 in windows-sys).
    code: i32,
    address: u64,
    is_fast_fail: bool,
}

enum PumpMessage {
    Spawn(
        SpawnRequest,
        mpsc::SyncSender<Result<SpawnResponse, std::io::Error>>,
    ),
    Shutdown,
}

/// Handle to the debug pump thread.  Drop to request shutdown.
pub struct Pump {
    tx: mpsc::Sender<PumpMessage>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Pump {
    /// Spawns the pump OS thread.
    pub fn start() -> Self {
        let (tx, rx) = mpsc::channel::<PumpMessage>();
        let thread = std::thread::Builder::new()
            .name("worrywart-debug-pump".into())
            .spawn(move || pump_loop(rx))
            .expect("failed to spawn debug pump thread");
        Pump {
            tx,
            thread: Some(thread),
        }
    }

    /// Sends a spawn request to the pump and waits for the response.
    pub fn spawn_child(&self, req: SpawnRequest) -> Result<SpawnResponse, std::io::Error> {
        let (resp_tx, resp_rx) = mpsc::sync_channel(1);
        self.tx
            .send(PumpMessage::Spawn(req, resp_tx))
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::BrokenPipe, "pump thread gone"))?;
        resp_rx
            .recv()
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::BrokenPipe, "pump thread gone"))?
    }
}

impl Drop for Pump {
    fn drop(&mut self) {
        let _ = self.tx.send(PumpMessage::Shutdown);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// The main pump loop.  Must run on the OS thread that calls `CreateProcess`.
fn pump_loop(rx: mpsc::Receiver<PumpMessage>) {
    // Map from PID → ChildEntry for all active debuggees.
    let mut children: HashMap<u32, ChildEntry> = HashMap::new();

    loop {
        // When no children are active, block until a message arrives.
        // When children are active, non-blocking check so we can call
        // WaitForDebugEvent promptly.
        let msg = if children.is_empty() {
            rx.recv().ok()
        } else {
            rx.try_recv().ok()
        };
        if let Some(msg) = msg {
            match msg {
                PumpMessage::Spawn(req, resp_tx) => {
                    let result = create_debugged_child(&req);
                    match result {
                        Ok((pi, past_loader)) => {
                            let entry = ChildEntry {
                                pid: pi.dwProcessId,
                                process_handle: pi.hProcess,
                                thread_handle: pi.hThread,
                                pending_exception: None,
                                past_loader_bp: past_loader,
                                affinity_mask: req.affinity_mask,
                                exit_tx: req.exit_tx,
                            };
                            children.insert(pi.dwProcessId, entry);
                            let _ = resp_tx.send(Ok(SpawnResponse {
                                pid: pi.dwProcessId,
                                process_handle: pi.hProcess,
                                thread_handle: pi.hThread,
                            }));
                        }
                        Err(e) => {
                            let _ = resp_tx.send(Err(e));
                        }
                    }
                }
                PumpMessage::Shutdown => {
                    // Drain remaining EXIT_PROCESS events then return.
                    // For now: just return; children will be killed when
                    // their handles are closed.
                    return;
                }
            }
        }

        if children.is_empty() {
            continue;
        }

        // Wait for a debug event (10 ms timeout so we can check for new
        // spawn requests while children are active).
        let mut event: DEBUG_EVENT = unsafe { std::mem::zeroed() };
        let got_event = unsafe { WaitForDebugEvent(&mut event, 10) };

        if got_event == FALSE {
            // Timeout or error — loop back.
            continue;
        }

        let pid = event.dwProcessId;
        let tid = event.dwThreadId;

        let continue_status = dispatch_event(&mut children, &event);

        unsafe {
            ContinueDebugEvent(pid, tid, continue_status);
        }

        // Remove child if it exited.
        // The exit_tx notification is sent inside dispatch_event.
    }
}

/// Create the child process with DEBUG_PROCESS set.
fn create_debugged_child(req: &SpawnRequest) -> std::io::Result<(PROCESS_INFORMATION, bool)> {
    let mut pi: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };

    // Build STARTUPINFOEXW with an empty attribute list (no job list yet —
    // that's Phase 2).  We still need EXTENDED_STARTUPINFO_PRESENT.
    const ATTR_COUNT: usize = 0;
    let mut attr_list_size: usize = 0;

    // Query required size for the attribute list.
    unsafe {
        InitializeProcThreadAttributeList(
            std::ptr::null_mut(),
            ATTR_COUNT as u32,
            0,
            &mut attr_list_size,
        );
    }

    let mut attr_list_buf: Vec<u8> = vec![0u8; attr_list_size];
    let attr_list_ptr = attr_list_buf.as_mut_ptr() as *mut _;

    let init_ok = unsafe {
        InitializeProcThreadAttributeList(attr_list_ptr, ATTR_COUNT as u32, 0, &mut attr_list_size)
    };
    if init_ok == FALSE {
        return Err(std::io::Error::last_os_error());
    }

    let mut si: STARTUPINFOEXW = unsafe { std::mem::zeroed() };
    si.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
    si.lpAttributeList = attr_list_ptr;

    if req.use_stdio_handles {
        si.StartupInfo.dwFlags |= STARTF_USESTDHANDLES;
        si.StartupInfo.hStdInput = req.stdin_handle;
        si.StartupInfo.hStdOutput = req.stdout_handle;
        si.StartupInfo.hStdError = req.stderr_handle;
    }

    let creation_flags: PROCESS_CREATION_FLAGS =
        DEBUG_PROCESS | EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT;

    let app_name_ptr = req
        .application_name
        .as_deref()
        .map(|s| s.as_ptr())
        .unwrap_or(std::ptr::null());

    let env_ptr = req
        .environment
        .as_deref()
        .map(|e| e.as_ptr() as *const _)
        .unwrap_or(std::ptr::null());

    let dir_ptr = req
        .current_directory
        .as_deref()
        .map(|d| d.as_ptr())
        .unwrap_or(std::ptr::null());

    let ok = unsafe {
        CreateProcessW(
            app_name_ptr,
            req.command_line.as_ptr() as *mut _,
            std::ptr::null(), // process security attributes
            std::ptr::null(), // thread security attributes
            TRUE,             // inherit handles
            creation_flags,
            env_ptr,
            dir_ptr,
            &si.StartupInfo,
            &mut pi,
        )
    };

    unsafe { DeleteProcThreadAttributeList(attr_list_ptr) };

    if ok == FALSE {
        return Err(std::io::Error::last_os_error());
    }

    Ok((pi, false))
}

/// Dispatch one debug event and return the `ContinueDebugEvent` status code.
fn dispatch_event(children: &mut HashMap<u32, ChildEntry>, event: &DEBUG_EVENT) -> i32 {
    let pid = event.dwProcessId;

    match event.dwDebugEventCode {
        CREATE_PROCESS_DEBUG_EVENT => {
            // The image file handle must be closed to avoid handle leaks.
            let h = unsafe { event.u.CreateProcessInfo.hFile };
            if h != INVALID_HANDLE_VALUE && !h.is_null() {
                unsafe { CloseHandle(h) };
            }
            // The initial loader breakpoint hasn't fired yet.
            DBG_CONTINUE
        }

        EXCEPTION_DEBUG_EVENT => {
            let info = unsafe { &event.u.Exception };
            let record = &info.ExceptionRecord;
            let code = record.ExceptionCode;
            let address = record.ExceptionAddress as u64;
            let first_chance = info.dwFirstChance != 0;

            if let Some(child) = children.get_mut(&pid) {
                if !child.past_loader_bp && code == exception_code::BREAKPOINT {
                    // Initial loader breakpoint — mark passed and optionally
                    // set affinity before continuing.
                    child.past_loader_bp = true;
                    if child.affinity_mask != 0 {
                        unsafe {
                            windows_sys::Win32::System::Threading::SetProcessAffinityMask(
                                child.process_handle,
                                child.affinity_mask,
                            )
                        };
                    }
                    return DBG_CONTINUE;
                }

                if !first_chance {
                    // Second-chance exception — record for correlation with EXIT_PROCESS.
                    let is_fast_fail = code == exception_code::STACK_BUFFER_OVERRUN
                        || code == exception_code::FATAL_APP_EXIT;
                    child.pending_exception = Some(PendingException {
                        code,
                        address,
                        is_fast_fail,
                    });
                    return DBG_EXCEPTION_NOT_HANDLED;
                }
            }

            // First-chance: let the process handle it.
            DBG_EXCEPTION_NOT_HANDLED
        }

        EXIT_PROCESS_DEBUG_EVENT => {
            if let Some(child) = children.remove(&pid) {
                let exit_code = unsafe { event.u.ExitProcess.dwExitCode };
                let reason = classify_exit(child.pending_exception, exit_code);
                let _ = child.exit_tx.send(Ok(reason));
                // Close our copies of the process/thread handles.
                unsafe {
                    CloseHandle(child.process_handle);
                    CloseHandle(child.thread_handle);
                }
            }
            DBG_CONTINUE
        }

        EXIT_THREAD_DEBUG_EVENT | CREATE_THREAD_DEBUG_EVENT | UNLOAD_DLL_DEBUG_EVENT => {
            DBG_CONTINUE
        }

        LOAD_DLL_DEBUG_EVENT => {
            // Must close the image file handle to avoid handle leaks.
            let h = unsafe { event.u.LoadDll.hFile };
            if h != INVALID_HANDLE_VALUE && !h.is_null() {
                unsafe { CloseHandle(h) };
            }
            DBG_CONTINUE
        }

        OUTPUT_DEBUG_STRING_EVENT => DBG_CONTINUE,

        RIP_EVENT => {
            // System integrity error — treat as ExternalKill.
            if let Some(child) = children.remove(&pid) {
                let status = make_exit_status_pub(1);
                let _ = child
                    .exit_tx
                    .send(Ok(crate::TerminationReason::ExternalKill(status)));
                unsafe {
                    CloseHandle(child.process_handle);
                    CloseHandle(child.thread_handle);
                }
            }
            DBG_CONTINUE
        }

        _ => DBG_CONTINUE,
    }
}

/// Classify a process exit into a `TerminationReason`.
fn classify_exit(pending: Option<PendingException>, exit_code: u32) -> crate::TerminationReason {
    match pending {
        Some(exc) if exc.is_fast_fail => crate::TerminationReason::FastFail(exc.code as u32),
        Some(exc) => crate::TerminationReason::Crash {
            code: exc.code as u32,
            address: exc.address,
        },
        None => {
            // No exception observed — could be CleanExit or ExternalKill.
            // Phase 3 (sentinel) will resolve this; for now return Unknown.
            crate::TerminationReason::Unknown(make_exit_status_pub(exit_code))
        }
    }
}

/// Construct a `std::process::ExitStatus` from a raw Win32 exit code.
pub fn make_exit_status_pub(code: u32) -> std::process::ExitStatus {
    use std::os::windows::process::ExitStatusExt;
    std::process::ExitStatus::from_raw(code)
}

/// Convert an `OsStr` to a null-terminated UTF-16 `Vec<u16>`.
pub fn to_wide_null(s: &OsStr) -> Vec<u16> {
    s.encode_wide().chain(std::iter::once(0)).collect()
}
