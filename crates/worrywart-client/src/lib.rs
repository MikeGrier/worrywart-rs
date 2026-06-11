// Copyright (c) 2026 Michael Grier

//! Sentinel client for the `worrywart` child-process monitor.
//!
//! Child processes that are monitored via `Monitor::Sentinel` should call
//! [`notify_exit`] immediately before intentionally exiting.  The worrywart
//! monitor uses this signal to classify the exit as `CleanExit` rather than
//! `ExternalKill`.
//!
//! On non-Windows platforms the functions are no-ops and always return `false`.

/// Environment variable set by the worrywart monitor to pass the sentinel
/// pipe write handle to the child process.  The value is a decimal string
/// representation of the raw `HANDLE` integer.
pub const ENV_VAR: &str = "WORRYWART_SENTINEL_HANDLE";

/// Magic prefix written at the start of every sentinel message: `"WORT"`.
const SENTINEL_MAGIC: [u8; 4] = *b"WORT";

/// Notifies the worrywart monitor that this process is about to exit
/// intentionally (clean exit).
///
/// If `WORRYWART_SENTINEL_HANDLE` is not set in the environment (i.e. the
/// process was not launched by a sentinel-aware monitor), this function is a
/// no-op and returns `false`.
///
/// Returns `true` if the notification was successfully written.
pub fn notify_exit(exit_code: i32) -> bool {
    notify_exit_reason(exit_code, "")
}

/// Notifies the worrywart monitor with an optional reason string.
///
/// The `reason` parameter is reserved for future use and is not currently
/// inspected by the monitor.
///
/// Returns `true` if the notification was successfully written.
pub fn notify_exit_reason(exit_code: i32, _reason: &str) -> bool {
    #[cfg(windows)]
    {
        platform::notify(exit_code)
    }
    #[cfg(not(windows))]
    {
        let _ = exit_code;
        false
    }
}

#[cfg(windows)]
mod platform {
    use super::{ENV_VAR, SENTINEL_MAGIC};
    use windows_sys::Win32::Foundation::{FALSE, HANDLE};
    use windows_sys::Win32::Storage::FileSystem::WriteFile;

    pub fn notify(exit_code: i32) -> bool {
        let handle = match get_handle() {
            Some(h) => h,
            None => return false,
        };
        write_sentinel(handle, exit_code)
    }

    fn get_handle() -> Option<HANDLE> {
        let val = std::env::var(ENV_VAR).ok()?;
        let n: usize = val.parse().ok()?;
        if n == 0 {
            return None;
        }
        Some(n as HANDLE)
    }

    fn write_sentinel(handle: HANDLE, exit_code: i32) -> bool {
        let mut msg = [0u8; 8];
        msg[0..4].copy_from_slice(&SENTINEL_MAGIC);
        msg[4..8].copy_from_slice(&exit_code.to_le_bytes());

        let mut written: u32 = 0;
        let ok = unsafe {
            WriteFile(
                handle,
                msg.as_ptr() as *const _,
                8,
                &mut written,
                std::ptr::null_mut(),
            )
        };
        ok != FALSE && written == 8
    }
}
