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
    /// Passes a named pipe handle to the child.  The child calls
    /// `worrywart_notify_exit()` (from the `worrywart-client` crate) before
    /// intentional exit.  Resolves the `CleanExit` vs. `ExternalKill`
    /// ambiguity definitively.
    ///
    /// Requires the child to link against `worrywart-client`.
    Sentinel,
}
