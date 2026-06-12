// Copyright (c) 2026 Michael Grier

use std::process::ExitStatus;

/// The reason a monitored child process terminated.
///
/// Returned by [`crate::Child::wait_diagnosed`] and
/// [`crate::core::WorrywartChild::wait_diagnosed`].
///
/// This enum is `#[non_exhaustive]` because additional classification
/// techniques (e.g. ETW) may introduce new variants in future releases.
#[derive(Debug)]
#[non_exhaustive]
pub enum TerminationReason {
    /// The process exited voluntarily — called `ExitProcess` or returned
    /// from `main`.  Only available when Technique 3 (sentinel) is active;
    /// otherwise an unexceptional exit is reported as [`Unknown`].
    ///
    /// [`Unknown`]: TerminationReason::Unknown
    CleanExit(ExitStatus),

    /// The process terminated due to an unhandled SEH exception.
    /// `code` is the exception code; `address` is the faulting instruction
    /// pointer (as a 64-bit value on both 32- and 64-bit targets).
    Crash {
        /// Win32 exception code (e.g. `0xC0000005` for access violation).
        code: u32,
        /// Faulting instruction pointer.
        address: u64,
    },

    /// The process called `__fastfail` / `RtlFailFast`.
    /// Common codes: `0xC0000409` (`STATUS_STACK_BUFFER_OVERRUN`),
    /// `0x40000015` (`STATUS_FATAL_APP_EXIT`).
    FastFail(u32),

    /// The process was killed by an outside agent (e.g. Windows Defender).
    /// Only available when Technique 3 (sentinel) is active and no sentinel
    /// was received before the process exited.
    ExternalKill(ExitStatus),

    /// The process exited but the cause could not be determined — either
    /// because no monitoring techniques were active, or because the active
    /// techniques are insufficient to classify this exit.
    Unknown(ExitStatus),
}

impl TerminationReason {
    /// Returns the underlying [`ExitStatus`] regardless of the variant.
    pub fn exit_status(&self) -> ExitStatus {
        match self {
            TerminationReason::CleanExit(s) => *s,
            TerminationReason::Crash { code, .. } => {
                // Synthesise an ExitStatus from the exception code; callers
                // that need the exception address should match the variant.
                #[cfg(windows)]
                {
                    use std::os::windows::process::ExitStatusExt;
                    ExitStatus::from_raw(*code)
                }
                #[cfg(not(windows))]
                {
                    let _ = code;
                    unreachable!("Crash variant is only produced on Windows")
                }
            }
            TerminationReason::FastFail(code) => {
                #[cfg(windows)]
                {
                    use std::os::windows::process::ExitStatusExt;
                    ExitStatus::from_raw(*code)
                }
                #[cfg(not(windows))]
                {
                    let _ = code;
                    unreachable!("FastFail variant is only produced on Windows")
                }
            }
            TerminationReason::ExternalKill(s) => *s,
            TerminationReason::Unknown(s) => *s,
        }
    }
}
