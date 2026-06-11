// Copyright (c) 2026 Michael Grier

//! Core worrywart types — the correct ownership model.
//!
//! A [`Worrywart`] instance owns a Windows Job Object.  Child processes are
//! assigned to that job atomically at creation.  Dropping the [`Worrywart`]
//! kills all children (via `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` semantics).
//!
//! These types are Windows-only.  For cross-platform use, see the
//! [`crate::compat`] module.

use std::ffi::OsStr;

use crate::{Monitor, TerminationReason};

/// The root monitor/owner instance.
///
/// Holds the Job Object handle, the debug pump thread (if
/// [`Monitor::DebugApi`] is selected), and the IOCP thread (if
/// [`Monitor::JobObject`] is selected).
///
/// Dropping this value kills all child processes owned by the job
/// (via `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`).
#[cfg(windows)]
pub struct Worrywart {
    _private: (),
}

#[cfg(windows)]
impl Worrywart {
    /// Creates a new `Worrywart` instance and its associated Job Object.
    pub fn new() -> std::io::Result<Self> {
        todo!("Phase 2: create Job Object, set JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE")
    }

    /// Returns a builder for spawning a monitored child process.
    pub fn command(&self, _program: impl AsRef<OsStr>) -> WorrywartCommand {
        todo!("Phase 1/2: return WorrywartCommand bound to this Worrywart")
    }
}

/// Builder for spawning a child process under a [`Worrywart`] instance.
///
/// Mirrors the `tokio::process::Command` builder interface, extended with
/// `.monitor()` for selecting monitoring techniques.
#[cfg(windows)]
pub struct WorrywartCommand {
    _private: (),
}

#[cfg(windows)]
impl WorrywartCommand {
    /// Appends an argument to the child's command line.
    pub fn arg(self, _arg: impl AsRef<OsStr>) -> Self {
        todo!("Phase 1: store arg")
    }

    /// Appends multiple arguments to the child's command line.
    pub fn args(self, _args: impl IntoIterator<Item = impl AsRef<OsStr>>) -> Self {
        todo!("Phase 1: store args")
    }

    /// Sets or overrides an environment variable for the child.
    pub fn env(self, _key: impl AsRef<OsStr>, _val: impl AsRef<OsStr>) -> Self {
        todo!("Phase 1: store env var")
    }

    /// Removes an environment variable from the child's environment.
    pub fn env_remove(self, _key: impl AsRef<OsStr>) -> Self {
        todo!("Phase 1: store env removal")
    }

    /// Clears the child's environment entirely.
    pub fn env_clear(self) -> Self {
        todo!("Phase 1: set env_clear flag")
    }

    /// Sets the child's working directory.
    pub fn current_dir(self, _dir: impl AsRef<std::path::Path>) -> Self {
        todo!("Phase 1: store working dir")
    }

    /// Enables a monitoring technique for this child.
    ///
    /// May be called multiple times to combine techniques.
    pub fn monitor(self, _technique: Monitor) -> Self {
        todo!("Phase 1: store monitor technique")
    }

    /// Spawns the child process.
    ///
    /// When [`Monitor::DebugApi`] is active, the actual `CreateProcess` call
    /// is performed on the debug pump thread; this method blocks until the
    /// pump thread returns the process handle.
    pub async fn spawn(self) -> std::io::Result<WorrywartChild> {
        todo!("Phase 1: cross-thread spawn handoff")
    }
}

/// Handle to a running child process spawned by [`Worrywart`].
#[cfg(windows)]
pub struct WorrywartChild {
    _private: (),
}

#[cfg(windows)]
impl WorrywartChild {
    /// Waits for the child to exit and returns its exit status.
    ///
    /// Identical in signature to `tokio::process::Child::wait`.
    pub async fn wait(&mut self) -> std::io::Result<std::process::ExitStatus> {
        todo!("Phase 1: wait via debug pump / IOCP")
    }

    /// Waits for the child to exit and returns the classified
    /// [`TerminationReason`].
    ///
    /// Internally drives the same machinery as [`wait`]; the reason is
    /// cached so calling either method consumes the wait.
    ///
    /// [`wait`]: WorrywartChild::wait
    pub async fn wait_diagnosed(&mut self) -> std::io::Result<TerminationReason> {
        todo!("Phase 1: wait and classify via debug pump / IOCP")
    }

    /// Returns the OS process identifier, if available.
    pub fn id(&self) -> Option<u32> {
        todo!("Phase 1: return PID from stored handle")
    }

    /// Sends `TerminateProcess` to the child.
    pub async fn kill(&mut self) -> std::io::Result<()> {
        todo!("Phase 1: TerminateProcess")
    }
}
