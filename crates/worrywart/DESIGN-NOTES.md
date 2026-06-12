# worrywart — Design Notes

> **Scope:** These notes cover the Windows implementation.  Non-Windows
> support will be addressed in a later revision.  "Windows" is the implicit
> context throughout; it is not repeated on every paragraph.

## Problem Statement

The LLVM toolchain produces binaries that Windows Defender's
Attack Surface Reduction (ASR) rules can misclassify as malicious due to
high file I/O velocity.  The relevant ASR rules are primarily:

- **"Use advanced protection against ransomware"**
  (GUID `c1db55ab-c21a-4637-bb3f-a12568109d35`) — triggers on processes with
  high file I/O velocity.
- **"Block executable files from running unless they meet a prevalence,
  age, or trusted-list criterion"** — catches newly built binaries.

The approved remediation path is to get an exemption from the Defender team
once the program is well-established and published, but that requires the
program to already have a track record — a chicken-and-egg problem.

One mitigation is to move the high-I/O work to a child service process.
That raises a new diagnostic problem: **when the child terminates
unexpectedly, it is unclear whether it exited on its own, crashed, was
killed by Defender, or was killed by some other agent.**

`worrywart` is a library for launching and monitoring child processes on
Windows with enough diagnostic fidelity to distinguish these cases.

Primary target consumer: the text-server process in `MikeGrier/tpu-rs`,
which is an LLVM-built process that encounters this problem.

---

## Termination Classification Goals

Given a monitored child process, worrywart should be able to classify its
termination into (at minimum) the following buckets:

| Class | Description |
|---|---|
| `CleanExit(code)` | Process called `ExitProcess` / returned from `main` voluntarily. |
| `Crash(exception)` | Process terminated due to an unhandled exception (SEH). |
| `FastFail(code)` | Process called `__fastfail` / `RtlFailFast` (`0xC0000409`, `0x40000015`). |
| `ExternalKill(code)` | Process was terminated by an outside agent (e.g. Defender). |
| `Unknown(code)` | Termination observed but cause cannot be determined. |

The distinction between `CleanExit` and `ExternalKill` is the hardest
problem: at the Win32 level, `ExitProcess(N)` called from inside the process
is indistinguishable from `TerminateProcess(handle, N)` called from outside.
Solving this precisely requires either child cooperation or ETW kernel events.

---

## Monitoring Techniques — A Suite

Three complementary techniques are in scope.  They are not mutually
exclusive; the library should allow callers to combine them.

### Technique 1: Job Object + IOCP (no child code, no debugger)

Assign the child to a Windows Job Object with an associated I/O Completion
Port.  The kernel posts notifications to the IOCP when process events occur:

- `JOB_OBJECT_MSG_EXIT_PROCESS` (7) — exited without an unhandled exception.
- `JOB_OBJECT_MSG_ABNORMAL_EXIT_PROCESS` (8) — exited due to an unhandled
  exception.

**Strengths:**
- Requires no modification to the child.
- Child does not see `IsDebuggerPresent() = true`.
- Distinguishes crash vs non-crash without being a debugger.
- Survives scenarios where the child itself spawns sub-children (job
  inheritance).

**Limitations:**
- Cannot distinguish `CleanExit` from `ExternalKill` — both produce
  `JOB_OBJECT_MSG_EXIT_PROCESS`.
- Cannot capture exception codes or addresses.
- If the child is already assigned to a job that does not allow breakaway,
  assigning it to a new job fails (less common on Windows 8+ with nested
  jobs, but still a consideration).

### Technique 2: Debug API (no child code; child sees debugger present)

Launch the child as a debuggee (or attach with `DebugActiveProcess`).
Process `WaitForDebugEvent` in a dedicated thread:

- `EXCEPTION_DEBUG_EVENT` — capture exception code, address, first/second
  chance.  Correlate with subsequent `EXIT_PROCESS_DEBUG_EVENT`.
- `EXIT_PROCESS_DEBUG_EVENT` — capture exit code.
- Correlation rule: second-chance exception followed immediately by exit →
  `Crash`.  Fast-fail exception codes → `FastFail`.  Exit without prior
  exception → `CleanExit | ExternalKill` (ambiguous without sentinel).

**Strengths:**
- Rich diagnostic data: exception codes, addresses, first vs second chance.
- No child-side code needed.
- Can classify `FastFail` with high confidence.

**Limitations:**
- `IsDebuggerPresent()` returns `true` in the child.  Programs that check
  this (anti-tamper code, some CRT assertions, some test frameworks) will
  behave differently.
- Only one debugger can be attached at a time.  If the caller itself wants
  to be a debugger, this technique is unavailable.
- `ExternalKill` vs `CleanExit` ambiguity remains without the sentinel.

### Technique 3: In-Process Sentinel (requires child cooperation)

The child calls a small API before intentional exit.  The monitor sees the
signal before the exit event and classifies any exit that lacks the signal
as `ExternalKill` (or `Crash`, if combined with Technique 1/2).

Signal transport candidates:
- **Named pipe** — child writes a sentinel message; monitor reads it.  Simple,
  low-overhead, works cross-process.
- **Named kernel event** — child sets a named event just before
  `ExitProcess`.  Monitor checks whether it was set before or after exit.
- **Shared memory flag** — similar to event but slightly more flexible.

Named pipe is preferred: it naturally carries a payload (e.g. an exit
reason string) and the monitor can detect pipe closure (child crash) vs
graceful write+close.

**Strengths:**
- Closes the `CleanExit` vs `ExternalKill` ambiguity definitively.
- Can carry structured diagnostic data from the child (e.g. last known state,
  custom exit reason).

**Limitations:**
- Requires code in the child.
- Needs both a Rust API and a C API (see below).

---

## Client-Side API (Sentinel Support)

When Technique 3 is used, the child process needs to call into worrywart.
Two delivery vehicles are needed:

### Rust crate: `worrywart-client`

A separate, minimal crate (not a feature of `worrywart`).  Rationale for
separation:

- The monitor (`worrywart`) is a heavy-ish Windows-API library.  Child
  processes should not pay that cost just to emit a signal.
- `worrywart-client` has no dependencies beyond `windows-sys` or raw FFI.
- Separate versioning: the client API is stable and rarely changes; the
  monitor evolves more frequently.
- Cleaner C FFI surface (see below) when isolated.

### C API: `worrywart_client.h` + static/dynamic lib

Some child processes are not Rust (or are Rust but link to C code that
initialises first).  A plain-C API is required:

```c
// Call once during process startup to connect to the monitor.
int worrywart_init(void);

// Call immediately before intentional ExitProcess / return from main.
void worrywart_notify_exit(int exit_code);

// Optional: report a structured reason string.
void worrywart_notify_exit_reason(int exit_code, const char* reason);
```

The C API should be generated from the Rust implementation via `cbindgen`
or hand-maintained as a thin shim — TBD.

---

## Architecture: Two-Layer API

### Decision: Correct Core API + tokio Compatibility Layer on Top

The codebase is structured in two explicit layers.

**Layer 1 — worrywart core (`worrywart::core` or just internal types)**

This is the *correct* ownership model.  A `Worrywart` instance owns a Job
Object.  Child processes are assigned to that job atomically at creation via
`PROC_THREAD_ATTRIBUTE_JOB_LIST`.  `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` is
set by default — when the `Worrywart` is dropped, all children die.  This is
proper resource ownership: the job handle *is* the lifetime of the process
group.

The core types are:
- `Worrywart` — the monitor/owner instance.  Holds the job handle, the debug
  pump thread (if `Monitor::DebugApi` is selected), and the IOCP thread (if
  `Monitor::JobObject` is selected).
- `WorrywartCommand` — builder for spawning a monitored child.
- `WorrywartChild` — handle to a running child.  `wait()` and
  `wait_diagnosed()` live here.  Drop kills the process (via job-close
  semantics if applicable) or orphans it, controlled by an explicit option.

tpu-rs and other callers that want the full API use these types directly.

**Layer 2 — tokio compatibility surface (`worrywart::Command`, `worrywart::Child`)**

A thin wrapper over Layer 1 that exactly mirrors the `tokio::process` API
including its defaults:
- Drop of `Child` without `wait()` → orphan (matches tokio default).
- `kill_on_drop(true)` → explicit kill on drop (matches tokio's opt-in).
- No `.monitor()` calls → no job object, no debug pump; pure tokio behaviour.

This layer exists so that the import-swap case is zero-friction.  It is
implemented *on top of* the core API rather than alongside it, so the correct
model is not compromised to fit tokio's model.

```
┌─────────────────────────────────────────────────────┐
│  tokio compat  worrywart::Command / worrywart::Child │
│  (tokio defaults, orphan-on-drop)                    │
├─────────────────────────────────────────────────────┤
│  worrywart core  Worrywart / WorrywartCommand /      │
│                  WorrywartChild                      │
│  (correct ownership: job owns children, kill-on-     │
│   close by default, explicit lifetime control)       │
├─────────────────────────────────────────────────────┤
│  Win32  CreateProcess · Job Objects · Debug API      │
│         IOCP · PROC_THREAD_ATTRIBUTE_JOB_LIST        │
└─────────────────────────────────────────────────────┘
```

If tokio's model is wrong for a use case, the caller reaches down to the core
layer.  Nothing in the compat layer prevents this.

---

## API Design Decisions

### Decision: Builder Pattern, Not Raw Win32 Signature

The primary API is a builder, not a mirror of `CreateProcessW`'s 10-parameter
signature.  Rationale: process creation has too many optional parameters for a
flat function to be ergonomic.  `std::process::Command` established this
convention for good reason.

The builder's `.spawn()` implementation uses `CreateProcessW` with
`STARTUPINFOEXW` and process-thread attributes internally, so callers get the
full power of the extended API without spelling it out.  Escape hatch:
`.raw_attribute(key, value)` for any `PROC_THREAD_ATTRIBUTE_*` not yet
exposed as a builder method.

```rust
let child = Command::new("my_service.exe")
    .arg("--port=8080")
    .monitor(Monitor::DebugApi)     // Technique 2 — MVP
    .monitor(Monitor::Sentinel)     // Technique 3 — complement
    .spawn()?;

let reason: TerminationReason = child.wait_diagnosed()?;
```

### Decision: Technique 2 (Debug API) Is the MVP

The Debug API provides the richest classification:
- Unhandled exception → `Crash(exception_code, address)`
- Fast-fail codes → `FastFail(code)`
- Exit without exception → `CleanExit | ExternalKill` (ambiguous without sentinel)

Job Object + IOCP (Technique 1) is a complement, not a prerequisite.
Sentinel (Technique 3) is what closes the `CleanExit`/`ExternalKill` gap.
All three should eventually be available; Technique 2 ships first.

### Decision: Primary Positioning — Drop-in for `tokio::process`

The elevator pitch is: **"use `worrywart::Command` anywhere you use
`tokio::process::Command`, and your process monitoring becomes diagnostic."**

The 99% case — callers that only care about exit codes — should work without
any changes beyond swapping the import.  The 1% case — callers that want to
know *why* the process went away — gets `wait_diagnosed()`.  Both cases
should feel native; neither should feel like an afterthought.

Concretely, the types `worrywart::Command` and `worrywart::Child` mirror
`tokio::process::Command` and `tokio::process::Child` in all commonly-used
methods.  Documentation for `worrywart` should say:

> "Refer to the [tokio::process documentation](https://docs.rs/tokio/latest/tokio/process/index.html).
> All standard methods work identically.  The only additions are
> `.monitor()` on the builder and `.wait_diagnosed()` on the child."

### Decision: `wait()` + `wait_diagnosed()` — tokio Compatibility via Option B

`wait()` returns `Result<ExitStatus>` — identical to tokio.  Existing code
that only cares about exit codes compiles unchanged with a crate swap.

`wait_diagnosed()` returns `Result<TerminationReason>` — the richer result.
Internally, `wait()` drives the full diagnosis machinery and caches the reason;
`wait_diagnosed()` returns the cached result.  A caller cannot call both
sequentially on the child handle — `wait()` consumes it — but they also do
not need to.  Callers that want diagnosis use `wait_diagnosed()` exclusively.

`TerminationReason` carries an `ExitStatus` internally, so `.exit_status()`
is available for callers migrating from tokio who want the richer type.

```rust
// Existing tokio code — no change needed after swapping the import:
let status: ExitStatus = child.wait().await?;

// Code that wants diagnosis:
let reason: TerminationReason = child.wait_diagnosed().await?;
match reason {
    TerminationReason::CleanExit(status)       => { /* ... */ }
    TerminationReason::Crash { code, address } => { /* ... */ }
    TerminationReason::FastFail(code)          => { /* ... */ }
    TerminationReason::ExternalKill(status)    => { /* ... */ }
    TerminationReason::Unknown(status)         => { /* ... */ }
}
```

### Decision: worrywart Always Launches (Primary); Attach Is Secondary

The primary API launches the process.  This is mandatory for Technique 2:
`WaitForDebugEvent` only receives events from processes created with
`DEBUG_PROCESS` on the *same thread*, or attached via `DebugActiveProcess`
(which has a race window if the process is already running).

Attaching to an existing PID will be a secondary API (`Command::attach(pid)`)
supporting Technique 1 and Technique 3 only.

---

## Debug Pump Thread Architecture

This is the central implementation complexity of the Debug API technique.

### One Shared Thread Per `Worrywart` Instance

A single OS thread ("the debug pump") services *all* debugged child processes
owned by a `Worrywart` instance.  `WaitForDebugEvent` returns events from all
processes the calling thread created with `DEBUG_PROCESS` — the event payload
carries `dwProcessId` so the pump knows which child fired.

For a service managing N worker processes: **1 pump thread total**, not N.
The pump thread is created lazily on first `spawn()` and lives until the
`Worrywart` instance is dropped.

### Spawn Must Cross to the Pump Thread

`WaitForDebugEvent` only sees processes created on *that specific thread*.
Therefore `Command::spawn()` cannot call `CreateProcess` on the caller's
thread (which may be a tokio thread pool thread).  The sequence is:

1. Caller calls `child = command.spawn()` on their thread.
2. worrywart serialises the full creation request (command line, all
   attributes, handles) into a message and sends it to the pump thread via a
   channel.
3. The pump thread receives the message, calls `CreateProcess(DEBUG_PROCESS,
   ...)`, then immediately re-enters `WaitForDebugEvent`.
4. The pump thread sends the resulting handles (process, thread, PID) back
   to the caller via a response channel.
5. `spawn()` returns the `Child` handle to the caller.

This cross-thread handoff is latency-neutral for long-running service
processes.  It is the unavoidable cost of the debug API's thread affinity
constraint.

### Per-Process Attributes Carried Across the Channel

The spawn message must carry all of the following (builder fields → Win32):

| Builder method | Win32 mechanism |
|---|---|
| `.arg()` / `.args()` | `lpCommandLine` |
| `.env()` / `.env_clear()` | `lpEnvironment` |
| `.current_dir()` | `lpCurrentDirectory` |
| `.stdin/stdout/stderr()` | `STARTUPINFOW.hStd*` + `STARTF_USESTDHANDLES` |
| `.with_token(HANDLE)` | `CreateProcessAsUserW` instead of `CreateProcessW` |
| `.with_job(HANDLE)` | `PROC_THREAD_ATTRIBUTE_JOB_LIST` (atomic, no race) |
| `.inherit_handles(list)` | `PROC_THREAD_ATTRIBUTE_HANDLE_LIST` |
| `.parent_process(HANDLE)` | `PROC_THREAD_ATTRIBUTE_PARENT_PROCESS` |
| `.creation_flags(u32)` | `dwCreationFlags` |
| `.raw_attribute(key, val)` | `UpdateProcThreadAttribute` escape hatch |
| `.affinity_mask(usize)` | `SetProcessAffinityMask` (post-creation; see note) |

**Token propagation note:** If the caller holds an impersonation token on
their thread, the pump thread will not have it.  The caller must explicitly
pass the token handle via `.with_token(handle)`.  The pump thread calls
`ImpersonateLoggedOnUser` / `CreateProcessAsUserW` / `RevertToSelf` as an
atomic unit while processing the spawn message.

**Affinity note:** `SetProcessAffinityMask` cannot be set atomically at
creation time — there is no `PROC_THREAD_ATTRIBUTE_*` for it.  However, when
the debug API is active, the child's main thread is suspended at the initial
loader breakpoint (`EXCEPTION_DEBUG_EVENT` with `EXCEPTION_BREAKPOINT` at
process attach).  The pump thread sets affinity during this window, before
continuing the child.  This is effectively atomic from the child's
perspective.

**Job object note:** `PROC_THREAD_ATTRIBUTE_JOB_LIST` assigns the job at
creation time with no race window.  This is strictly better than calling
`AssignProcessToJobObject` after `CreateProcess`.  Requires Windows 8+
(safe to assume for this library's scope).

### Debug Event Handling in the Pump

The pump thread runs a loop:

```
loop {
    WaitForDebugEvent(&event, INFINITE)
    match event.dwDebugEventCode {
        CREATE_PROCESS_DEBUG_EVENT  => register child, store handles
        EXCEPTION_DEBUG_EVENT       => record exception; classify first/second chance
        EXIT_THREAD_DEBUG_EVENT     => bookkeeping
        EXIT_PROCESS_DEBUG_EVENT    => correlate with recorded exceptions → TerminationReason
                                       notify waiting caller via per-child channel
        LOAD_DLL_DEBUG_EVENT        => close the image file handle (required); optionally record
        UNLOAD_DLL_DEBUG_EVENT      => bookkeeping
        OUTPUT_DEBUG_STRING_EVENT   => optionally forward to tracing
        RIP_EVENT                   => system error; treat as ExternalKill
        _                           => continue
    }
    ContinueDebugEvent(pid, tid, DBG_CONTINUE | DBG_EXCEPTION_NOT_HANDLED)
}
```

Key correlation rules:
- Second-chance `EXCEPTION_DEBUG_EVENT` immediately before
  `EXIT_PROCESS_DEBUG_EVENT` → `Crash { code, address }`
- Exception code `0xC0000409` (`STATUS_STACK_BUFFER_OVERRUN` / `__fastfail`)
  or `0x40000015` (`STATUS_FATAL_APP_EXIT`) → `FastFail(code)`
- `EXIT_PROCESS_DEBUG_EVENT` with no preceding exception
  AND no sentinel received → `CleanExit | ExternalKill` → `Unknown` without
  Technique 3, or `ExternalKill` with Technique 3

The initial `EXCEPTION_BREAKPOINT` at process attach (loader breakpoint) must
be continued with `DBG_CONTINUE` and not recorded as a user exception.

---

## Open Design Questions

1. **Technique selection API shape**
   Builder `.monitor(Monitor::DebugApi)` enum approach vs. separate
   builder types (`DebuggedCommand`, `JobCommand`).  Enum is simpler for
   combining techniques; separate types give stronger compile-time guarantees
   that conflicting options cannot be set.  TBD.

2. **`wait_diagnosed()` and async**
   The debug pump is a blocking OS thread.  `wait_diagnosed()` in an async
   context should be a `spawn_blocking` wrapper or should post a waker to
   the pump loop.  The cleaner option (waker) requires the pump loop to
   manage a table of wakers alongside the process table.  TBD.

3. **`worrywart-client` C API delivery**
   Static lib (`.lib`) + header, `cdylib`, or `cbindgen`-generated header.
   TBD once the Rust sentinel API is stable.

4. **ETW as a future Technique 4**
   `Microsoft-Windows-Kernel-Process` `ProcessStop` events carry the PID of
   the terminating caller — the only way to definitively identify Defender
   without child cooperation.  Requires an ETW session and elevated trust.
   Out of scope for v0.1; noted here for future consideration.

---

## Implementation Milestones

Phases are ordered by dependency, not by importance.  No version numbers are
assigned to intermediate phases — only the final phase targets a crates.io
release (0.1.0).

### Phase 0 — Foundation

Goal: the full public API surface compiles, CI is green, nothing real is
implemented yet.

- Define all public types: `TerminationReason`, `Monitor`, `Worrywart`,
  `WorrywartCommand`, `WorrywartChild`, and the tokio-compat
  `Command`/`Child` wrappers.
- All methods are stubs (`todo!()` or delegating to `tokio::process`).
- tokio-compat layer delegates directly to `tokio::process` so that the
  import-swap case works end-to-end (no monitoring, but correct behaviour).
- `wait_diagnosed()` returns `TerminationReason::Unknown(status)`.
- Integration test: spawn a child, call `wait()`, assert exit status.
  Passes on CI (Linux and Windows).
- `cargo_check`, `cargo_clippy`, `cargo_fmt_check` all clean.

### Phase 1 — Technique 2: Debug API

Goal: `TerminationReason` has real values for crash and fast-fail.

- Implement the debug pump thread: single OS thread per `Worrywart` instance.
- Implement the cross-thread spawn handoff via channel.
- Implement the `WaitForDebugEvent` loop with the full event dispatch table.
- Implement exception correlation → `TerminationReason` classification.
- `Monitor::DebugApi` is functional end-to-end.
- Integration tests:
  - Child that returns from `main` → `Unknown` (no sentinel yet).
  - Child that calls `abort()` → `Crash`.
  - Child that calls `__fastfail` → `FastFail`.
- Process affinity set during loader breakpoint window.

### Phase 2 — Technique 1: Job Object + IOCP

Goal: the core ownership model is fully realised.

- Create the Job Object in `Worrywart::new()`.
- Set `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` by default.
- Use `PROC_THREAD_ATTRIBUTE_JOB_LIST` to assign atomically at creation.
- Implement the IOCP listener thread.
- `Monitor::JobObject` is functional.  Crash vs. clean-exit classification
  from IOCP messages used to complement Debug API results.
- Integration tests:
  - Drop `Worrywart` while child is running → child is killed.
  - Child crash detected via `JOB_OBJECT_MSG_ABNORMAL_EXIT_PROCESS`.

### Phase 3 — Technique 3: Sentinel

Goal: `CleanExit` vs. `ExternalKill` is fully resolved for cooperative
children.

- Implement the sentinel named pipe: monitor creates it, passes the handle
  to the child at spawn.
- Implement the `worrywart-client` crate with `worrywart_notify_exit()`.
- Integration between sentinel signal and `TerminationReason` classification:
  sentinel received → `CleanExit`; no sentinel + no exception → `ExternalKill`.
- Integration tests:
  - Cooperative child calls `worrywart_notify_exit()` → `CleanExit`.
  - `TerminateProcess` from test harness → `ExternalKill`.

### Phase 4 — C API

Goal: non-Rust children can use the sentinel.

- Generate `worrywart_client.h` via `cbindgen` from the `worrywart-client`
  crate.
- Build a static lib target.
- Smoke test: a small C program that calls `worrywart_init()` and
  `worrywart_notify_exit()`, linked against the static lib.

### Phase 5 — 0.1.0 Release

Goal: publishable, documented, production-ready.

- `tracing` integration: pump thread and IOCP thread emit structured events.
- Audit tokio-compat surface for parity gaps; fill or document them.
- Full rustdoc coverage on all public items.
- `cargo_publish --dry-run` passes cleanly.
- Tag and publish 0.1.0.

---

## Known Constraints

- **Non-Windows support:** TBD in a later revision.
- **All diagnostics are opt-in.**  With no `.monitor()` calls,
  `worrywart` behaves like plain `tokio::process` even on Windows —
  no debug pump thread, no job object, no sentinel pipe.
- **Debug API is opt-in** because `IsDebuggerPresent()` returns `true` in
  the child.  Callers who want Job Object + Sentinel without the debugger
  side-effect can do so.
- The immediate consumer (`tpu-rs` text server) is a Rust process that we
  control.  Technique 2 + Technique 3 together give full classification for
  this case.
- The library should also be useful for processes we do *not* control
  (Technique 1 and/or 2 without sentinel), making it generally applicable.
