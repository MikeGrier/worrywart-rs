// Copyright (c) 2026 Michael Grier

//! tokio-compat surface — drop-in replacement for `tokio::process`.
//!
//! `Command` and `Child` mirror `tokio::process::Command` and
//! `tokio::process::Child` with identical signatures and defaults.
//! The only additions are:
//!
//! - [`Command::monitor`] — opt-in to a monitoring technique.
//! - [`Child::wait_diagnosed`] — retrieve the [`TerminationReason`].
//!
//! For callers that only care about exit codes, replace
//! `use tokio::process` with `use worrywart` and nothing else changes.

use std::ffi::OsStr;
use std::path::Path;
use std::process::{ExitStatus, Stdio};

use tokio::process as tokio_process;

use crate::{Monitor, TerminationReason};

/// A builder for spawning child processes.
///
/// Mirrors [`tokio::process::Command`] in all standard methods.
/// Use [`monitor`] to enable worrywart diagnostics.
///
/// [`monitor`]: Command::monitor
pub struct Command {
    inner: tokio_process::Command,
    /// Monitoring techniques requested by the caller.  Empty means
    /// no worrywart instrumentation — behaves identically to
    /// `tokio::process::Command`.
    _monitors: Vec<Monitor>,
}

impl Command {
    /// Constructs a new `Command` for launching the program at `program`.
    pub fn new(program: impl AsRef<OsStr>) -> Self {
        Self {
            inner: tokio_process::Command::new(program),
            _monitors: Vec::new(),
        }
    }

    /// Appends a single argument.
    pub fn arg(&mut self, arg: impl AsRef<OsStr>) -> &mut Self {
        self.inner.arg(arg);
        self
    }

    /// Appends multiple arguments.
    pub fn args(&mut self, args: impl IntoIterator<Item = impl AsRef<OsStr>>) -> &mut Self {
        self.inner.args(args);
        self
    }

    /// Sets or overrides an environment variable.
    pub fn env(&mut self, key: impl AsRef<OsStr>, val: impl AsRef<OsStr>) -> &mut Self {
        self.inner.env(key, val);
        self
    }

    /// Sets multiple environment variables from an iterator of `(key, val)` pairs.
    pub fn envs(
        &mut self,
        vars: impl IntoIterator<Item = (impl AsRef<OsStr>, impl AsRef<OsStr>)>,
    ) -> &mut Self {
        self.inner.envs(vars);
        self
    }

    /// Removes an environment variable.
    pub fn env_remove(&mut self, key: impl AsRef<OsStr>) -> &mut Self {
        self.inner.env_remove(key);
        self
    }

    /// Clears the child's entire environment.
    pub fn env_clear(&mut self) -> &mut Self {
        self.inner.env_clear();
        self
    }

    /// Sets the child's working directory.
    pub fn current_dir(&mut self, dir: impl AsRef<Path>) -> &mut Self {
        self.inner.current_dir(dir);
        self
    }

    /// Configures the child's stdin handle.
    pub fn stdin(&mut self, cfg: impl Into<Stdio>) -> &mut Self {
        self.inner.stdin(cfg);
        self
    }

    /// Configures the child's stdout handle.
    pub fn stdout(&mut self, cfg: impl Into<Stdio>) -> &mut Self {
        self.inner.stdout(cfg);
        self
    }

    /// Configures the child's stderr handle.
    pub fn stderr(&mut self, cfg: impl Into<Stdio>) -> &mut Self {
        self.inner.stderr(cfg);
        self
    }

    /// If `kill_on_drop` is true, the child process is killed when the
    /// [`Child`] handle is dropped without calling [`Child::wait`].
    ///
    /// Default: `false` (matches `tokio::process::Command`).
    ///
    /// [`Child::wait`]: Child::wait
    pub fn kill_on_drop(&mut self, kill_on_drop: bool) -> &mut Self {
        self.inner.kill_on_drop(kill_on_drop);
        self
    }

    /// Enables a monitoring technique for this child.
    ///
    /// May be called multiple times to combine techniques.  In Phase 0
    /// the technique is stored but not yet acted upon.
    pub fn monitor(&mut self, technique: Monitor) -> &mut Self {
        self._monitors.push(technique);
        self
    }

    /// Spawns the child process.
    pub fn spawn(&mut self) -> std::io::Result<Child> {
        let inner = self.inner.spawn()?;
        Ok(Child::from_tokio(inner))
    }
}

/// A handle to a running child process.
///
/// Mirrors [`tokio::process::Child`] in all standard methods.
/// Additional method: [`wait_diagnosed`].
///
/// [`wait_diagnosed`]: Child::wait_diagnosed
pub struct Child {
    /// Stdio handles, re-exposed as public fields to match
    /// `tokio::process::Child`.
    pub stdin: Option<tokio_process::ChildStdin>,
    /// Standard output handle (if captured).
    pub stdout: Option<tokio_process::ChildStdout>,
    /// Standard error handle (if captured).
    pub stderr: Option<tokio_process::ChildStderr>,
    inner: tokio_process::Child,
}

impl Child {
    fn from_tokio(mut c: tokio_process::Child) -> Self {
        Self {
            stdin: c.stdin.take(),
            stdout: c.stdout.take(),
            stderr: c.stderr.take(),
            inner: c,
        }
    }

    /// Waits for the child to exit and returns its exit status.
    ///
    /// Identical in signature to `tokio::process::Child::wait`.
    pub async fn wait(&mut self) -> std::io::Result<ExitStatus> {
        self.inner.wait().await
    }

    /// Waits for the child to exit and returns the classified
    /// [`TerminationReason`].
    ///
    /// In Phase 0 this always returns [`TerminationReason::Unknown`]
    /// because no monitoring techniques are implemented yet.
    pub async fn wait_diagnosed(&mut self) -> std::io::Result<TerminationReason> {
        let status = self.inner.wait().await?;
        Ok(TerminationReason::Unknown(status))
    }

    /// Returns the OS process identifier, if available.
    pub fn id(&self) -> Option<u32> {
        self.inner.id()
    }

    /// Sends a kill signal to the child.
    ///
    /// Identical in signature to `tokio::process::Child::kill`.
    pub async fn kill(&mut self) -> std::io::Result<()> {
        self.inner.kill().await
    }

    /// Attempts to collect the exit status if the child has already exited.
    ///
    /// Identical in signature to `tokio::process::Child::try_wait`.
    pub fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>> {
        self.inner.try_wait()
    }
}
