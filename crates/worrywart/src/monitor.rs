// Copyright (c) 2026 Michael Grier

/// Selects which monitoring technique(s) to apply to a child process.
///
/// Pass one or more variants to `.monitor()` on the builder.  Techniques
/// are not mutually exclusive; combining them yields richer classification.
///
/// This enum is `#[non_exhaustive]` because additional techniques (e.g. ETW)
/// may be added in future releases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Monitor {
    /// **Technique 2 — Debug API.**
    ///
    /// Launches the child as a debuggee (`CREATE_PROCESS_DEBUG_EVENT`).
    /// Provides the richest classification: crash exception codes, fast-fail
    /// codes, and crash addresses.
    ///
    /// **Side effect:** `IsDebuggerPresent()` returns `true` in the child.
    /// Opt in explicitly only when the child is known to tolerate this.
    DebugApi,

    /// **Technique 1 — Job Object + IOCP.**
    ///
    /// Assigns the child to a Windows Job Object with an associated I/O
    /// Completion Port.  Distinguishes crash vs. non-crash without making
    /// the child a debuggee.  Cannot distinguish `CleanExit` from
    /// `ExternalKill` without also using [`Sentinel`].
    ///
    /// [`Sentinel`]: Monitor::Sentinel
    JobObject,

    /// **Technique 3 — In-Process Sentinel.**
    ///
    /// Passes an inheritable anonymous-pipe write handle to the child (its
    /// value is communicated via the `WORRYWART_SENTINEL_HANDLE` environment
    /// variable).  Before exiting intentionally, the child writes a small
    /// sentinel message to that handle; the monitor uses its presence to
    /// resolve the `CleanExit` vs. `ExternalKill` ambiguity definitively.
    ///
    /// A future C client library (`worrywart-client`) will provide a helper
    /// for emitting this message; today the child writes it directly (see the
    /// Phase 3 integration tests).
    Sentinel,
}
