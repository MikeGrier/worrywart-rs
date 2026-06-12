// Copyright (c) 2026 Michael Grier

//! IOCP (I/O Completion Port) listener thread for Job Object monitoring.
//!
//! When a process assigned to a Job Object exits, the kernel posts a
//! completion packet to the IOCP associated with the job.  The listener
//! thread reads these packets and sends classified [`TerminationReason`]
//! values to per-child receivers.

use std::collections::HashMap;
use std::sync::mpsc;

use tracing::{debug, trace};

use windows_sys::Win32::Foundation::{CloseHandle, FALSE, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::IO::{
    CreateIoCompletionPort, GetQueuedCompletionStatus, OVERLAPPED, PostQueuedCompletionStatus,
};
use windows_sys::Win32::System::JobObjects::{
    JOBOBJECT_ASSOCIATE_COMPLETION_PORT, JobObjectAssociateCompletionPortInformation,
    SetInformationJobObject,
};
use windows_sys::Win32::System::Threading::{
    GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
};

// Not exported by windows-sys 0.59; raw values from the Windows SDK.
const JOB_OBJECT_MSG_EXIT_PROCESS: u32 = 7;
const JOB_OBJECT_MSG_ABNORMAL_EXIT_PROCESS: u32 = 8;

pub type IocpResult = Result<crate::TerminationReason, std::io::Error>;

/// Completion key used for all job-object notifications.
const JOB_KEY: usize = 1;

/// Special completion key posted to shut down the listener thread.
const SHUTDOWN_KEY: usize = 0;

enum IocpMessage {
    Register {
        pid: u32,
        exit_tx: mpsc::SyncSender<IocpResult>,
    },
}

/// Handle to the IOCP listener thread.
pub struct Iocp {
    /// Raw IOCP handle — used only to post the shutdown sentinel on drop.
    iocp_handle: HANDLE,
    tx: mpsc::Sender<IocpMessage>,
    thread: Option<std::thread::JoinHandle<()>>,
}

// SAFETY: HANDLE is *mut c_void; safe to transfer between threads on Windows.
unsafe impl Send for Iocp {}
unsafe impl Sync for Iocp {}

impl Iocp {
    /// Creates an IOCP, associates it with `job`, and starts the listener thread.
    pub fn start(job: HANDLE) -> std::io::Result<Self> {
        let iocp =
            unsafe { CreateIoCompletionPort(INVALID_HANDLE_VALUE, std::ptr::null_mut(), 0, 0) };
        if iocp.is_null() {
            return Err(std::io::Error::last_os_error());
        }

        // Associate the job with the IOCP.
        let assoc = JOBOBJECT_ASSOCIATE_COMPLETION_PORT {
            CompletionKey: JOB_KEY as *mut _,
            CompletionPort: iocp,
        };
        let ok = unsafe {
            SetInformationJobObject(
                job,
                JobObjectAssociateCompletionPortInformation,
                &assoc as *const _ as *const _,
                std::mem::size_of::<JOBOBJECT_ASSOCIATE_COMPLETION_PORT>() as u32,
            )
        };
        if ok == FALSE {
            unsafe { CloseHandle(iocp) };
            return Err(std::io::Error::last_os_error());
        }

        let (tx, rx) = mpsc::channel::<IocpMessage>();
        // Cast to usize so the closure can cross the Send bound: HANDLE is
        // *mut c_void which is !Send, but it is safe to transfer on Windows.
        let iocp_raw = iocp as usize;
        let thread = std::thread::Builder::new()
            .name("worrywart-iocp".into())
            .spawn(move || iocp_loop(iocp_raw as HANDLE, rx))
            .expect("failed to spawn IOCP listener thread");

        Ok(Iocp {
            iocp_handle: iocp,
            tx,
            thread: Some(thread),
        })
    }

    /// Registers a child PID so the IOCP thread can route its exit notification.
    pub fn register(&self, pid: u32, exit_tx: mpsc::SyncSender<IocpResult>) {
        let _ = self.tx.send(IocpMessage::Register { pid, exit_tx });
    }
}

impl Drop for Iocp {
    fn drop(&mut self) {
        // Post a shutdown sentinel to wake the IOCP thread immediately.
        unsafe {
            PostQueuedCompletionStatus(self.iocp_handle, 0, SHUTDOWN_KEY, std::ptr::null_mut());
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        // The IOCP handle is closed by the iocp_loop thread before it exits.
    }
}

fn iocp_loop(iocp: HANDLE, rx: mpsc::Receiver<IocpMessage>) {
    debug!("iocp: listener started");
    let mut waiters: HashMap<u32, mpsc::SyncSender<IocpResult>> = HashMap::new();
    // Buffer for notifications that arrive before the waiter registers.
    let mut pending: HashMap<u32, IocpResult> = HashMap::new();

    loop {
        // Drain any pending registration messages.
        while let Ok(msg) = rx.try_recv() {
            match msg {
                IocpMessage::Register { pid, exit_tx } => {
                    trace!(pid, "iocp: waiter registered");
                    if let Some(result) = pending.remove(&pid) {
                        let _ = exit_tx.send(result);
                    } else {
                        waiters.insert(pid, exit_tx);
                    }
                }
            }
        }

        let mut bytes: u32 = 0;
        let mut key: usize = 0;
        let mut overlapped: *mut OVERLAPPED = std::ptr::null_mut();

        let ok =
            unsafe { GetQueuedCompletionStatus(iocp, &mut bytes, &mut key, &mut overlapped, 10) };

        if key == SHUTDOWN_KEY {
            // Drain any already-queued notifications before exiting.
            // This matters when the job closes (killing children) just before
            // the IOCP receives the shutdown sentinel.
            drain_remaining(iocp, &mut waiters, &mut pending);
            debug!("iocp: listener shutdown");
            unsafe { CloseHandle(iocp) };
            return;
        }

        if ok == FALSE && overlapped.is_null() {
            // Timeout (WAIT_TIMEOUT) — no event; loop back.
            continue;
        }

        // key == JOB_KEY, bytes == message type, overlapped == PID as pointer.
        dispatch_notification(bytes, overlapped, &mut waiters, &mut pending);
    }
}

/// Process a single job-object notification.
///
/// Only exit-class messages (7 and 8) cause a result to be delivered; all
/// other messages (e.g. `JOB_OBJECT_MSG_NEW_PROCESS = 6`) are silently ignored
/// so that they do not prematurely trigger a pending waiter.
fn dispatch_notification(
    msg_type: u32,
    overlapped: *mut OVERLAPPED,
    waiters: &mut HashMap<u32, mpsc::SyncSender<IocpResult>>,
    pending: &mut HashMap<u32, IocpResult>,
) {
    if msg_type != JOB_OBJECT_MSG_EXIT_PROCESS && msg_type != JOB_OBJECT_MSG_ABNORMAL_EXIT_PROCESS {
        // NEW_PROCESS (6), ACTIVE_PROCESS_ZERO (4), etc. — not a termination.
        return;
    }

    let pid = overlapped as usize as u32;
    debug!(pid, msg_type, "iocp: exit notification");
    let result = classify_iocp_exit(pid, msg_type);

    if let Some(tx) = waiters.remove(&pid) {
        let _ = tx.send(result);
    } else {
        pending.insert(pid, result);
    }
}

/// Drain all already-queued IOCP notifications using a zero-millisecond timeout.
/// Called when the shutdown sentinel is received so that any exit notifications
/// posted by the kernel immediately before shutdown are not lost.
fn drain_remaining(
    iocp: HANDLE,
    waiters: &mut HashMap<u32, mpsc::SyncSender<IocpResult>>,
    pending: &mut HashMap<u32, IocpResult>,
) {
    loop {
        let mut bytes: u32 = 0;
        let mut key: usize = 0;
        let mut overlapped: *mut OVERLAPPED = std::ptr::null_mut();

        let ok =
            unsafe { GetQueuedCompletionStatus(iocp, &mut bytes, &mut key, &mut overlapped, 0) };

        // Stop on timeout (no more packets) or another shutdown sentinel.
        if ok == FALSE && overlapped.is_null() {
            break;
        }
        if key == SHUTDOWN_KEY {
            break;
        }

        dispatch_notification(bytes, overlapped, waiters, pending);
    }
}

fn classify_iocp_exit(pid: u32, msg_type: u32) -> IocpResult {
    let code = get_exit_code(pid).unwrap_or(0);

    // Use the kernel-supplied message type rather than a high-bit heuristic:
    // JOB_OBJECT_MSG_ABNORMAL_EXIT_PROCESS (8) means the process was
    // terminated abnormally; JOB_OBJECT_MSG_EXIT_PROCESS (7) is a clean exit.
    if msg_type == JOB_OBJECT_MSG_ABNORMAL_EXIT_PROCESS {
        debug!(pid, code, "iocp: classified as Crash");
        Ok(crate::TerminationReason::Crash { code, address: 0 })
    } else {
        debug!(pid, code, "iocp: classified as Unknown (pending sentinel)");
        Ok(crate::TerminationReason::Unknown(
            crate::pump::make_exit_status_pub(code),
        ))
    }
}

/// Attempts to read the exit code of a process by PID.
fn get_exit_code(pid: u32) -> Option<u32> {
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid) };
    if handle.is_null() {
        return None;
    }
    let mut code: u32 = 0;
    let ok = unsafe { GetExitCodeProcess(handle, &mut code) };
    unsafe { CloseHandle(handle) };
    if ok == FALSE { None } else { Some(code) }
}
