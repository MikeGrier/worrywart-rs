// Copyright (c) 2026 Michael Grier

//! Sentinel anonymous-pipe support (monitor side).
//!
//! For each spawn that includes [`Monitor::Sentinel`], the monitor:
//!
//! 1. Creates an anonymous pipe with an **inheritable write end**.
//! 2. Passes the write-end handle value to the child via the
//!    [`ENV_VAR`] environment variable.
//! 3. Immediately after `CreateProcess` returns, closes its own copy of the
//!    write end so that only the child holds it.
//! 4. Runs a background thread that reads from the read end.
//!    - If the sentinel message arrives → the thread reports `true` (clean exit).
//!    - If the pipe closes without the sentinel → the thread reports `false`
//!      (external kill or crash).
//!
//! [`Monitor::Sentinel`]: crate::Monitor::Sentinel

use std::sync::mpsc;

use tracing::{debug, trace};

use windows_sys::Win32::Foundation::SetHandleInformation;
use windows_sys::Win32::Foundation::{
    CloseHandle, FALSE, HANDLE, HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE, TRUE,
};
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::Storage::FileSystem::ReadFile;
use windows_sys::Win32::System::IO::OVERLAPPED;
use windows_sys::Win32::System::Pipes::CreatePipe;

/// Environment variable used to communicate the sentinel write handle to the
/// child process.  Its value is the decimal representation of the raw
/// `HANDLE` integer.
pub const ENV_VAR: &str = "WORRYWART_SENTINEL_HANDLE";

/// Magic bytes at the start of a sentinel message: `"WORT"` (4 bytes).
const SENTINEL_MAGIC: [u8; 4] = *b"WORT";

/// Total byte length of a sentinel message: 4 magic + 4 LE exit-code.
const SENTINEL_MSG_LEN: usize = 8;

/// Monitor-side handle to an active sentinel pipe.
///
/// The write end is passed to the child via `ENV_VAR`; this struct owns
/// the write handle until the caller closes it (immediately after spawn)
/// and the read end is owned by the listener thread.
pub struct SentinelPipe {
    /// Inheritable write handle — pass its integer value via `ENV_VAR`, then
    /// call `CloseHandle` on it immediately after `CreateProcess` returns.
    pub write_handle: HANDLE,
    /// Receives `true` once the sentinel message arrived before EOF, or
    /// `false` if the pipe closed without a valid sentinel.
    pub sentinel_rx: mpsc::Receiver<bool>,
}

// SAFETY: HANDLE is *mut c_void; safe to transfer between threads on Windows.
unsafe impl Send for SentinelPipe {}

/// Creates an anonymous pipe and starts the sentinel listener thread.
///
/// The caller must:
/// 1. Add the write handle value (as decimal) to the child's environment
///    under [`ENV_VAR`].
/// 2. Spawn the child (with `bInheritHandles = TRUE`).
/// 3. Call `CloseHandle(pipe.write_handle)` immediately after spawn.
pub fn create() -> std::io::Result<SentinelPipe> {
    let mut read_handle: HANDLE = INVALID_HANDLE_VALUE;
    let mut write_handle: HANDLE = INVALID_HANDLE_VALUE;

    // Create the pipe.  The write end is inheritable so the child can use it.
    let mut sa: SECURITY_ATTRIBUTES = unsafe { std::mem::zeroed() };
    sa.nLength = std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32;
    sa.bInheritHandle = TRUE;
    sa.lpSecurityDescriptor = std::ptr::null_mut();

    let ok = unsafe { CreatePipe(&mut read_handle, &mut write_handle, &sa, 0) };
    if ok == FALSE {
        return Err(std::io::Error::last_os_error());
    }

    // The read end must NOT be inherited — the child must not hold it open,
    // or the pipe would never close when the child exits.
    let ok = unsafe { SetHandleInformation(read_handle, HANDLE_FLAG_INHERIT, 0) };
    if ok == FALSE {
        let err = std::io::Error::last_os_error();
        unsafe {
            CloseHandle(read_handle);
            CloseHandle(write_handle);
        }
        return Err(err);
    }

    let (tx, rx) = mpsc::sync_channel::<bool>(1);
    let read_raw = read_handle as usize;
    std::thread::Builder::new()
        .name("worrywart-sentinel".into())
        .spawn(move || sentinel_thread(read_raw as HANDLE, tx))
        .inspect_err(|_| unsafe {
            CloseHandle(read_raw as HANDLE);
            CloseHandle(write_handle);
        })?;

    trace!("sentinel: pipe created");
    Ok(SentinelPipe {
        write_handle,
        sentinel_rx: rx,
    })
}

fn sentinel_thread(read_handle: HANDLE, tx: mpsc::SyncSender<bool>) {
    let mut buf = [0u8; SENTINEL_MSG_LEN];
    let mut offset = 0usize;
    let mut received = false;

    loop {
        let mut bytes_read: u32 = 0;
        let ok = unsafe {
            ReadFile(
                read_handle,
                buf[offset..].as_mut_ptr() as *mut _,
                (SENTINEL_MSG_LEN - offset) as u32,
                &mut bytes_read,
                std::ptr::null_mut::<OVERLAPPED>(),
            )
        };
        if ok == FALSE || bytes_read == 0 {
            // Broken pipe (child exited/crashed) or read error.
            break;
        }
        offset += bytes_read as usize;
        if offset >= SENTINEL_MSG_LEN {
            if buf[0..4] == SENTINEL_MAGIC {
                received = true;
            }
            // Reset for any additional messages (should not normally occur).
            offset = 0;
        }
    }

    unsafe { CloseHandle(read_handle) };
    if received {
        debug!("sentinel: sentinel message received");
    } else {
        debug!("sentinel: pipe closed without sentinel");
    }
    // Ignore send error: WorrywartChild may have been dropped without waiting.
    let _ = tx.send(received);
}
