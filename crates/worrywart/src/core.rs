// Copyright (c) 2026 Michael Grier

//! Core worrywart types — the correct ownership model.
//!
//! A [`Worrywart`] instance owns a Windows Job Object (Phase 2) and the
//! debug pump thread (when [`Monitor::DebugApi`] is selected).  Child
//! processes are assigned to the job atomically at creation and monitored
//! via the pump.
//!
//! These types are Windows-only.  For cross-platform use, see [`crate::compat`].

use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::sync::{mpsc, Arc, Mutex};

use crate::pump::{Pump, SpawnRequest, SpawnResponse, TerminationResult};
use crate::{Monitor, TerminationReason};

/// The root monitor/owner instance.
///
/// Holds the debug pump thread (when [`Monitor::DebugApi`] is selected).
/// Job Object ownership is added in Phase 2.
///
/// Dropping this value shuts down the pump thread.
#[cfg(windows)]
pub struct Worrywart {
    /// Lazily initialised pump thread.  `None` until first `DebugApi` spawn.
    pump: Arc<Mutex<Option<Pump>>>,
}

#[cfg(windows)]
impl Worrywart {
    /// Creates a new `Worrywart` instance.
    pub fn new() -> std::io::Result<Self> {
        Ok(Worrywart {
            pump: Arc::new(Mutex::new(None)),
        })
    }

    /// Returns a builder for spawning a monitored child process.
    pub fn command<S: AsRef<OsStr>>(&self, program: S) -> WorrywartCommand {
        WorrywartCommand::new_with_pump(program.as_ref(), Arc::clone(&self.pump))
    }
}

#[cfg(windows)]
impl Default for Worrywart {
    fn default() -> Self {
        Self::new().expect("Worrywart::new failed")
    }
}

/// Builder for spawning a child process under a [`Worrywart`] instance.
///
/// Mirrors `tokio::process::Command` in all standard methods, extended with
/// `.monitor()` and `.affinity_mask()`.
#[cfg(windows)]
pub struct WorrywartCommand {
    program: OsString,
    args: Vec<OsString>,
    envs: Vec<(OsString, OsString)>,
    env_clear: bool,
    current_dir: Option<OsString>,
    monitors: Vec<Monitor>,
    affinity_mask: usize,
    pump: Arc<Mutex<Option<Pump>>>,
}

#[cfg(windows)]
impl WorrywartCommand {
    fn new_with_pump(program: &OsStr, pump: Arc<Mutex<Option<Pump>>>) -> Self {
        WorrywartCommand {
            program: program.to_owned(),
            args: Vec::new(),
            envs: Vec::new(),
            env_clear: false,
            current_dir: None,
            monitors: Vec::new(),
            affinity_mask: 0,
            pump,
        }
    }

    /// Appends a single argument.
    pub fn arg(mut self, arg: impl AsRef<OsStr>) -> Self {
        self.args.push(arg.as_ref().to_owned());
        self
    }

    /// Appends multiple arguments.
    pub fn args(mut self, args: impl IntoIterator<Item = impl AsRef<OsStr>>) -> Self {
        self.args
            .extend(args.into_iter().map(|a| a.as_ref().to_owned()));
        self
    }

    /// Sets or overrides an environment variable.
    pub fn env(mut self, key: impl AsRef<OsStr>, val: impl AsRef<OsStr>) -> Self {
        self.envs
            .push((key.as_ref().to_owned(), val.as_ref().to_owned()));
        self
    }

    /// Removes an environment variable.
    pub fn env_remove(mut self, key: impl AsRef<OsStr>) -> Self {
        self.envs.retain(|(k, _)| k != key.as_ref());
        self
    }

    /// Clears the child's environment entirely.
    pub fn env_clear(mut self) -> Self {
        self.env_clear = true;
        self.envs.clear();
        self
    }

    /// Sets the child's working directory.
    pub fn current_dir(mut self, dir: impl AsRef<Path>) -> Self {
        self.current_dir = Some(dir.as_ref().as_os_str().to_owned());
        self
    }

    /// Enables a monitoring technique for this child.
    pub fn monitor(mut self, technique: Monitor) -> Self {
        if !self.monitors.contains(&technique) {
            self.monitors.push(technique);
        }
        self
    }

    /// Sets the process affinity mask (applied during the loader breakpoint
    /// window when `Monitor::DebugApi` is active).
    pub fn affinity_mask(mut self, mask: usize) -> Self {
        self.affinity_mask = mask;
        self
    }

    /// Spawns the child process.
    pub fn spawn(self) -> std::io::Result<WorrywartChild> {
        let use_debug_api = self.monitors.contains(&Monitor::DebugApi);

        if use_debug_api {
            self.spawn_debug()
        } else {
            self.spawn_plain()
        }
    }

    fn spawn_debug(self) -> std::io::Result<WorrywartChild> {
        use crate::pump::to_wide_null;

        // Ensure the pump thread exists.
        let mut guard = self.pump.lock().unwrap();
        if guard.is_none() {
            *guard = Some(Pump::start());
        }
        let pump = guard.as_ref().unwrap();

        let command_line = build_command_line(&self.program, &self.args);

        let environment = if self.env_clear && self.envs.is_empty() {
            Some(vec![0u16])
        } else {
            None
        };

        let current_directory = self.current_dir.as_deref().map(to_wide_null);

        let (exit_tx, exit_rx) = mpsc::sync_channel::<TerminationResult>(1);

        let req = SpawnRequest {
            application_name: None,
            command_line,
            environment,
            current_directory,
            affinity_mask: self.affinity_mask,
            exit_tx,
            stdin_handle: windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE,
            stdout_handle: windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE,
            stderr_handle: windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE,
            use_stdio_handles: false,
        };

        let SpawnResponse {
            pid,
            process_handle,
            thread_handle,
        } = pump.spawn_child(req)?;

        drop(guard);

        Ok(WorrywartChild {
            pid,
            process_handle,
            thread_handle,
            exit_rx: Some(exit_rx),
            cached_reason: None,
        })
    }

    fn spawn_plain(self) -> std::io::Result<WorrywartChild> {
        let mut cmd = std::process::Command::new(&self.program);
        cmd.args(&self.args);
        if self.env_clear {
            cmd.env_clear();
        }
        for (k, v) in &self.envs {
            cmd.env(k, v);
        }
        if let Some(dir) = &self.current_dir {
            cmd.current_dir(dir);
        }
        let child = cmd.spawn()?;
        let pid = child.id();
        Ok(WorrywartChild::from_std(child, pid))
    }
}

fn build_command_line(program: &OsStr, args: &[OsString]) -> Vec<u16> {
    let mut line = String::new();
    quote_arg(&mut line, &program.to_string_lossy());
    for arg in args {
        line.push(' ');
        quote_arg(&mut line, &arg.to_string_lossy());
    }
    line.encode_utf16().chain(std::iter::once(0)).collect()
}

fn quote_arg(out: &mut String, arg: &str) {
    let needs_quote =
        arg.is_empty() || arg.contains(' ') || arg.contains('"') || arg.contains('\t');
    if !needs_quote {
        out.push_str(arg);
        return;
    }
    out.push('"');
    let mut backslashes = 0usize;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                for _ in 0..backslashes {
                    out.push_str("\\\\");
                }
                backslashes = 0;
                out.push_str("\\\"");
            }
            _ => {
                for _ in 0..backslashes {
                    out.push('\\');
                }
                backslashes = 0;
                out.push(c);
            }
        }
    }
    for _ in 0..backslashes {
        out.push_str("\\\\");
    }
    out.push('"');
}

/// Handle to a running child process spawned by [`WorrywartCommand`].
#[cfg(windows)]
pub struct WorrywartChild {
    pid: u32,
    process_handle: windows_sys::Win32::Foundation::HANDLE,
    thread_handle: windows_sys::Win32::Foundation::HANDLE,
    exit_rx: Option<mpsc::Receiver<TerminationResult>>,
    cached_reason: Option<CachedReason>,
}

/// A cheaply-clonable snapshot of a `TerminationReason`.
#[cfg(windows)]
enum CachedReason {
    Unknown(u32),
    Crash { code: u32, address: u64 },
    FastFail(u32),
    ExternalKill(u32),
    CleanExit(u32),
}

#[cfg(windows)]
impl CachedReason {
    fn from_reason(r: &TerminationReason) -> Self {
        match r {
            TerminationReason::Unknown(s) => CachedReason::Unknown(exit_code(s)),
            TerminationReason::Crash { code, address } => CachedReason::Crash {
                code: *code,
                address: *address,
            },
            TerminationReason::FastFail(c) => CachedReason::FastFail(*c),
            TerminationReason::ExternalKill(s) => CachedReason::ExternalKill(exit_code(s)),
            TerminationReason::CleanExit(s) => CachedReason::CleanExit(exit_code(s)),
        }
    }

    fn into_reason(self) -> TerminationReason {
        match self {
            CachedReason::Unknown(c) => TerminationReason::Unknown(make_exit_status(c)),
            CachedReason::Crash { code, address } => TerminationReason::Crash { code, address },
            CachedReason::FastFail(c) => TerminationReason::FastFail(c),
            CachedReason::ExternalKill(c) => TerminationReason::ExternalKill(make_exit_status(c)),
            CachedReason::CleanExit(c) => TerminationReason::CleanExit(make_exit_status(c)),
        }
    }
}

fn exit_code(s: &std::process::ExitStatus) -> u32 {
    s.code().unwrap_or(1) as u32
}

fn make_exit_status(code: u32) -> std::process::ExitStatus {
    use std::os::windows::process::ExitStatusExt;
    std::process::ExitStatus::from_raw(code)
}

#[cfg(windows)]
impl WorrywartChild {
    fn from_std(child: std::process::Child, pid: u32) -> Self {
        // Drop the std::process::Child — we've already captured its PID.
        // The process keeps running; we hold no handle in plain mode.
        // (Phase 2 will hold a proper HANDLE via OpenProcess.)
        drop(child);
        WorrywartChild {
            pid,
            process_handle: windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE,
            thread_handle: windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE,
            exit_rx: None,
            cached_reason: None,
        }
    }

    /// Returns the OS process identifier.
    pub fn id(&self) -> Option<u32> {
        if self.pid == 0 {
            None
        } else {
            Some(self.pid)
        }
    }

    /// Waits for the child to exit and returns its exit status.
    pub fn wait(&mut self) -> std::io::Result<std::process::ExitStatus> {
        let reason = self.wait_diagnosed()?;
        Ok(reason_to_exit_status(reason))
    }

    /// Waits for the child to exit and returns the classified [`TerminationReason`].
    pub fn wait_diagnosed(&mut self) -> std::io::Result<TerminationReason> {
        if let Some(cached) = self.cached_reason.take() {
            return Ok(cached.into_reason());
        }

        let reason = if let Some(ref rx) = self.exit_rx {
            rx.recv().map_err(|_| {
                std::io::Error::new(std::io::ErrorKind::BrokenPipe, "pump thread gone")
            })??
        } else {
            self.wait_plain()?
        };

        // Re-cache for a second call.
        self.cached_reason = Some(CachedReason::from_reason(&reason));
        Ok(reason)
    }

    fn wait_plain(&self) -> std::io::Result<TerminationReason> {
        use windows_sys::Win32::Foundation::{FALSE, WAIT_OBJECT_0};
        use windows_sys::Win32::System::Threading::{
            GetExitCodeProcess, OpenProcess, WaitForSingleObject, INFINITE,
            PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
        };

        let handle = unsafe {
            OpenProcess(
                PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION,
                FALSE,
                self.pid,
            )
        };
        if handle.is_null() {
            return Err(std::io::Error::last_os_error());
        }
        let wait = unsafe { WaitForSingleObject(handle, INFINITE) };
        if wait != WAIT_OBJECT_0 {
            unsafe { windows_sys::Win32::Foundation::CloseHandle(handle) };
            return Err(std::io::Error::last_os_error());
        }
        let mut code: u32 = 0;
        let ok = unsafe { GetExitCodeProcess(handle, &mut code) };
        unsafe { windows_sys::Win32::Foundation::CloseHandle(handle) };
        if ok == FALSE {
            return Err(std::io::Error::last_os_error());
        }
        Ok(TerminationReason::Unknown(make_exit_status(code)))
    }

    /// Terminates the child process.
    pub fn kill(&mut self) -> std::io::Result<()> {
        use windows_sys::Win32::Foundation::FALSE;
        use windows_sys::Win32::System::Threading::TerminateProcess;
        if self.process_handle == windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE {
            return Err(std::io::Error::other(
                "no process handle available for kill",
            ));
        }
        let ok = unsafe { TerminateProcess(self.process_handle, 1) };
        if ok == FALSE {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

fn reason_to_exit_status(reason: TerminationReason) -> std::process::ExitStatus {
    match reason {
        TerminationReason::Unknown(s) => s,
        TerminationReason::CleanExit(s) => s,
        TerminationReason::ExternalKill(s) => s,
        TerminationReason::Crash { code, .. } => make_exit_status(code),
        TerminationReason::FastFail(code) => make_exit_status(code),
    }
}

#[cfg(windows)]
impl Drop for WorrywartChild {
    fn drop(&mut self) {
        use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
        if self.process_handle != INVALID_HANDLE_VALUE && !self.process_handle.is_null() {
            unsafe { CloseHandle(self.process_handle) };
            self.process_handle = INVALID_HANDLE_VALUE;
        }
        if self.thread_handle != INVALID_HANDLE_VALUE && !self.thread_handle.is_null() {
            unsafe { CloseHandle(self.thread_handle) };
            self.thread_handle = INVALID_HANDLE_VALUE;
        }
    }
}
