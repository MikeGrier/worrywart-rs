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
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::sync::{Arc, Mutex, mpsc};

use crate::iocp::Iocp;
use crate::pump::{Pump, SpawnRequest, SpawnResponse, TerminationResult};
use crate::{Monitor, TerminationReason};

// ---------------------------------------------------------------------------
// RAII wrapper for the Job Object handle.
// ---------------------------------------------------------------------------

struct JobHandle(windows_sys::Win32::Foundation::HANDLE);

// SAFETY: HANDLE is *mut c_void; kernel handles are safe to transfer between
// threads in the same process.
unsafe impl Send for JobHandle {}
unsafe impl Sync for JobHandle {}

impl Drop for JobHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { windows_sys::Win32::Foundation::CloseHandle(self.0) };
        }
    }
}

impl JobHandle {
    fn raw(&self) -> windows_sys::Win32::Foundation::HANDLE {
        self.0
    }
}

/// The root monitor/owner instance.
///
/// Holds the Job Object, the IOCP listener thread (when
/// [`Monitor::JobObject`] is selected), and the debug pump thread (when
/// [`Monitor::DebugApi`] is selected).
///
/// Dropping this value kills all child processes owned by the job
/// (via `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`) once the last
/// [`WorrywartCommand`] derived from it is also dropped.
#[cfg(windows)]
pub struct Worrywart {
    /// Lazily initialised pump thread.  `None` until first `DebugApi` spawn.
    pump: Arc<Mutex<Option<Pump>>>,
    /// Job Object handle.  All children are assigned to this job at creation.
    job: Arc<JobHandle>,
    /// Lazily initialised IOCP listener.  `None` until first `JobObject` spawn.
    iocp: Arc<Mutex<Option<Iocp>>>,
}

#[cfg(windows)]
impl Worrywart {
    /// Creates a new `Worrywart` instance and its associated Job Object.
    pub fn new() -> std::io::Result<Self> {
        use windows_sys::Win32::System::JobObjects::{
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JobObjectExtendedLimitInformation, SetInformationJobObject,
        };

        let job = unsafe {
            windows_sys::Win32::System::JobObjects::CreateJobObjectW(
                std::ptr::null(),
                std::ptr::null(),
            )
        };
        if job.is_null() {
            return Err(std::io::Error::last_os_error());
        }

        // Set JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE so that dropping this
        // Worrywart kills all child processes still in the job.
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let ok = unsafe {
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const _,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if ok == windows_sys::Win32::Foundation::FALSE {
            unsafe { windows_sys::Win32::Foundation::CloseHandle(job) };
            return Err(std::io::Error::last_os_error());
        }

        Ok(Worrywart {
            pump: Arc::new(Mutex::new(None)),
            job: Arc::new(JobHandle(job)),
            iocp: Arc::new(Mutex::new(None)),
        })
    }

    /// Returns a builder for spawning a monitored child process.
    pub fn command<S: AsRef<OsStr>>(&self, program: S) -> WorrywartCommand {
        WorrywartCommand::new_with_state(
            program.as_ref(),
            Arc::clone(&self.pump),
            Arc::clone(&self.job),
            Arc::clone(&self.iocp),
        )
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
    job: Arc<JobHandle>,
    iocp: Arc<Mutex<Option<Iocp>>>,
}

#[cfg(windows)]
impl WorrywartCommand {
    fn new_with_state(
        program: &OsStr,
        pump: Arc<Mutex<Option<Pump>>>,
        job: Arc<JobHandle>,
        iocp: Arc<Mutex<Option<Iocp>>>,
    ) -> Self {
        WorrywartCommand {
            program: program.to_owned(),
            args: Vec::new(),
            envs: Vec::new(),
            env_clear: false,
            current_dir: None,
            monitors: Vec::new(),
            affinity_mask: 0,
            pump,
            job,
            iocp,
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
    pub fn spawn(mut self) -> std::io::Result<WorrywartChild> {
        use windows_sys::Win32::Foundation::CloseHandle;

        let use_debug_api = self.monitors.contains(&Monitor::DebugApi);
        let use_job_object = self.monitors.contains(&Monitor::JobObject);
        let use_sentinel = self.monitors.contains(&Monitor::Sentinel);

        // Create sentinel pipe before spawning so the inheritable write end
        // is available when CreateProcess runs.
        let sentinel = if use_sentinel {
            let pipe = crate::sentinel::create()?;
            let handle_val: OsString = (pipe.write_handle as usize).to_string().into();
            self.envs
                .push((crate::sentinel::ENV_VAR.into(), handle_val));
            Some(pipe)
        } else {
            None
        };

        let mut child = if use_debug_api {
            // Debug pump path — also assigns to the job object.
            self.spawn_debug()?
        } else if use_job_object || use_sentinel {
            // Job-object / sentinel path — IOCP monitors exits.
            self.spawn_job_monitored()?
        } else {
            self.spawn_plain()?
        };

        if let Some(pipe) = sentinel {
            // Close the parent's copy of the write end immediately.  The
            // child now holds the only remaining copy; when the child exits
            // the pipe closes and the sentinel listener reports its result.
            unsafe { CloseHandle(pipe.write_handle) };
            child.sentinel_rx = Some(pipe.sentinel_rx);
        }

        Ok(child)
    }

    fn spawn_debug(self) -> std::io::Result<WorrywartChild> {
        use crate::pump::to_wide_null;

        // Ensure the pump thread exists.
        let mut guard = self.pump.lock().unwrap();
        if guard.is_none() {
            *guard = Some(Pump::start()?);
        }
        let pump = guard.as_ref().unwrap();

        let command_line = build_command_line(&self.program, &self.args);
        let environment = build_env_block(self.env_clear, &self.envs);
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
            // Sentinel is not used in the debug-pump path; no handles to inherit.
            inherit_handles: false,
            // Always assign to the job so kill-on-close applies.
            job_handle: self.job.raw(),
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
            sentinel_rx: None,
        })
    }

    fn spawn_job_monitored(self) -> std::io::Result<WorrywartChild> {
        use crate::pump::{JobSpawnParams, create_process_for_job, to_wide_null};
        use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};

        let command_line = build_command_line(&self.program, &self.args);
        let environment = build_env_block(self.env_clear, &self.envs);
        let current_directory = self.current_dir.as_deref().map(to_wide_null);

        // Sentinel write handle is inheritable; CreateProcess must propagate it.
        let use_sentinel = self.monitors.contains(&Monitor::Sentinel);
        let params = JobSpawnParams {
            application_name: None,
            command_line,
            environment,
            current_directory,
            job_handle: self.job.raw(),
            inherit_handles: use_sentinel,
        };

        let pi = create_process_for_job(&params)?;

        // Register the child with the IOCP listener.
        let mut iocp_guard = self.iocp.lock().unwrap();
        if iocp_guard.is_none() {
            *iocp_guard = Some(Iocp::start(self.job.raw())?);
        }
        let (exit_tx, exit_rx) = mpsc::sync_channel::<TerminationResult>(1);
        iocp_guard
            .as_ref()
            .unwrap()
            .register(pi.dwProcessId, exit_tx);
        drop(iocp_guard);

        // The thread handle is not needed for monitoring; close it.
        if !pi.hThread.is_null() && pi.hThread != INVALID_HANDLE_VALUE {
            unsafe { CloseHandle(pi.hThread) };
        }

        Ok(WorrywartChild {
            pid: pi.dwProcessId,
            process_handle: pi.hProcess,
            thread_handle: INVALID_HANDLE_VALUE,
            exit_rx: Some(exit_rx),
            cached_reason: None,
            sentinel_rx: None,
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

/// Builds a `CREATE_UNICODE_ENVIRONMENT` block for `CreateProcessW`.
///
/// Returns `None` (inherit parent environment unchanged) when `env_clear` is
/// false and `envs` is empty.  Otherwise merges the current-process
/// environment (unless `env_clear`) with the provided overrides/additions.
fn build_env_block(env_clear: bool, envs: &[(OsString, OsString)]) -> Option<Vec<u16>> {
    if !env_clear && envs.is_empty() {
        return None;
    }

    let mut pairs: Vec<(OsString, OsString)> = if env_clear {
        Vec::new()
    } else {
        std::env::vars_os().collect()
    };

    // Apply explicit overrides/additions.
    for (k, v) in envs {
        pairs.retain(|(ek, _)| ek.as_os_str() != k.as_os_str());
        pairs.push((k.clone(), v.clone()));
    }

    let mut block: Vec<u16> = Vec::new();
    for (k, v) in &pairs {
        block.extend(k.encode_wide());
        block.push(b'=' as u16);
        block.extend(v.encode_wide());
        block.push(0);
    }
    block.push(0); // double-null terminator
    Some(block)
}

/// Refines an `Unknown` exit into `CleanExit` or `ExternalKill` based on
/// whether the sentinel message was received.  Other variants are unchanged.
fn refine_with_sentinel(
    reason: TerminationReason,
    sentinel_rx: &mut Option<mpsc::Receiver<bool>>,
) -> TerminationReason {
    let rx = match sentinel_rx.take() {
        Some(rx) => rx,
        None => return reason,
    };
    let sentinel_ok = rx.recv().unwrap_or(false);
    match reason {
        TerminationReason::Unknown(status) if sentinel_ok => TerminationReason::CleanExit(status),
        TerminationReason::Unknown(status) => TerminationReason::ExternalKill(status),
        // Crash / FastFail / already classified — sentinel does not override.
        other => other,
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
    /// Receives `true` if the sentinel message arrived before pipe EOF;
    /// receives `false` if the pipe closed without a sentinel.
    /// `None` when `Monitor::Sentinel` was not requested.
    sentinel_rx: Option<mpsc::Receiver<bool>>,
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
        drop(child);
        WorrywartChild {
            pid,
            process_handle: windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE,
            thread_handle: windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE,
            exit_rx: None,
            cached_reason: None,
            sentinel_rx: None,
        }
    }

    /// Returns the OS process identifier.
    pub fn id(&self) -> Option<u32> {
        if self.pid == 0 { None } else { Some(self.pid) }
    }

    /// Waits for the child to exit and returns its exit status.
    pub fn wait(&mut self) -> std::io::Result<std::process::ExitStatus> {
        let reason = self.wait_diagnosed()?;
        Ok(reason_to_exit_status(reason))
    }

    /// Waits for the child to exit and returns the classified [`TerminationReason`].
    pub fn wait_diagnosed(&mut self) -> std::io::Result<TerminationReason> {
        if let Some(cached) = self.cached_reason.take() {
            let reason = cached.into_reason();
            // Re-cache so subsequent calls don't block on a closed channel.
            self.cached_reason = Some(CachedReason::from_reason(&reason));
            return Ok(reason);
        }

        let raw = if let Some(ref rx) = self.exit_rx {
            match rx.recv() {
                Ok(result) => result?,
                // Channel closed (pump/IOCP thread gone) — fall back to a plain
                // OS-level wait so the caller isn't left hanging.
                Err(_) => self.wait_plain()?,
            }
        } else {
            self.wait_plain()?
        };

        // Refine Unknown → CleanExit / ExternalKill using the sentinel result.
        // By the time exit_rx delivers, the child process is dead and its copy
        // of the pipe write end is closed, so sentinel_rx resolves promptly.
        let reason = refine_with_sentinel(raw, &mut self.sentinel_rx);

        // Re-cache for a second call.
        self.cached_reason = Some(CachedReason::from_reason(&reason));
        Ok(reason)
    }

    fn wait_plain(&self) -> std::io::Result<TerminationReason> {
        use windows_sys::Win32::Foundation::{FALSE, WAIT_OBJECT_0};
        use windows_sys::Win32::System::Threading::{
            GetExitCodeProcess, INFINITE, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
            PROCESS_SYNCHRONIZE, WaitForSingleObject,
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
