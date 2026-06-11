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
            TerminationReason::Crash { .. } => {
                // Crash exits have a non-zero code; we return the status
                // captured at EXIT_PROCESS time.  Callers that need the
                // exception code should match on the variant directly.
                todo!("Phase 1: return captured exit status for Crash")
            }
            TerminationReason::FastFail(_) => {
                todo!("Phase 1: return captured exit status for FastFail")
            }
            TerminationReason::ExternalKill(s) => *s,
            TerminationReason::Unknown(s) => *s,
        }
    }
}
