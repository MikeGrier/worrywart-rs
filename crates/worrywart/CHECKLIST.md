# worrywart — Implementation Checklist

See [DESIGN-NOTES.md](DESIGN-NOTES.md) for the full design and milestone
descriptions that produced these items.

Commit message format: `Completed item: <id>: <full item text>`
Check the item off (`- [x]`) in the same commit as the work.
One item, one commit. Tests must pass before committing.

---

## Phase 0 — Foundation

- [x] P0-1: Add `tokio` (with `process` feature) and `windows-sys` dependencies to `crates/worrywart/Cargo.toml`
- [x] P0-2: Define `TerminationReason` enum with variants `CleanExit`, `Crash`, `FastFail`, `ExternalKill`, `Unknown`
- [x] P0-3: Define `Monitor` enum with variants `DebugApi`, `JobObject`, `Sentinel`
- [x] P0-4: Define core types `Worrywart`, `WorrywartCommand`, `WorrywartChild` as stubs (`todo!()` bodies; all methods compile but are unimplemented)
- [x] P0-5: Define tokio-compat types `Command` and `Child` that delegate to `tokio::process`; `wait_diagnosed()` returns `TerminationReason::Unknown`
- [x] P0-6: Add integration test: spawn a real child process via the compat `Command`, call `wait()`, assert exit status is success

---

## Phase 1 — Technique 2: Debug API

- [x] P1-1: Implement debug pump thread skeleton in `Worrywart`: spawn a single OS thread on first `Monitor::DebugApi` spawn, driven by a `std::sync::mpsc` request/response channel
- [x] P1-2: Implement cross-thread spawn handoff: `WorrywartCommand::spawn()` posts a creation request to the pump thread and receives process/thread handles back via response channel
- [x] P1-3: Implement `WaitForDebugEvent` loop in the pump thread with full event dispatch: `CREATE_PROCESS`, `EXCEPTION`, `EXIT_THREAD`, `EXIT_PROCESS`, `LOAD_DLL` (close image handle), `OUTPUT_DEBUG_STRING`, `RIP_EVENT`
- [x] P1-4: Implement exception correlation → `TerminationReason`: second-chance exception before `EXIT_PROCESS` → `Crash`; fast-fail codes (`0xC0000409`, `0x40000015`) → `FastFail`; exit with no exception → `Unknown`
- [x] P1-5: Set process affinity mask during the loader breakpoint window (initial `EXCEPTION_BREAKPOINT` before continuing child), when `.affinity_mask()` is set on the builder
- [x] P1-6: Add integration tests: child that returns from `main` → `Unknown`; child that calls `abort()` → `Crash`; child that calls `__fastfail` → `FastFail`

---

## Phase 2 — Technique 1: Job Object + IOCP

- [x] P2-1: Create a Job Object in `Worrywart::new()` and set `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` by default; close it on `Drop`
- [x] P2-2: Use `PROC_THREAD_ATTRIBUTE_JOB_LIST` to assign the child to the job atomically at creation (no post-creation `AssignProcessToJobObject`)
- [x] P2-3: Implement IOCP listener thread: associate an IOCP with the job, dispatch `JOB_OBJECT_MSG_EXIT_PROCESS` and `JOB_OBJECT_MSG_ABNORMAL_EXIT_PROCESS`
- [x] P2-4: Wire `Monitor::JobObject` end-to-end in `WorrywartCommand`; IOCP crash signal upgrades `Unknown` → `Crash` when Debug API is not active
- [x] P2-5: Add integration tests: drop `Worrywart` while child is sleeping → child is killed; child that crashes → `JOB_OBJECT_MSG_ABNORMAL_EXIT_PROCESS` observed

---

## Phase 3 — Technique 3: Sentinel

- [x] P3-1: Implement sentinel named pipe on the monitor side: create the pipe in `WorrywartCommand::spawn()`, inherit the write end to the child, read sentinel message asynchronously
- [x] P3-2: Create the `worrywart-client` crate in `crates/worrywart-client/` with `worrywart_init()` and `worrywart_notify_exit()` (Rust API only at this stage)
- [x] P3-3: Wire sentinel signal into `TerminationReason` classification: sentinel received before `EXIT_PROCESS` → `CleanExit`; no sentinel + no exception → `ExternalKill`
- [x] P3-4: Add integration tests: cooperative child calls `worrywart_notify_exit()` → `CleanExit`; `TerminateProcess` from test harness → `ExternalKill`

---

## Phase 4 — 0.1.0 Release

- [x] P5-1: Add `tracing` integration: pump thread and IOCP thread emit structured `trace!`/`debug!`/`warn!` events for all state transitions
- [x] P5-2: Audit tokio-compat `Command`/`Child` surface against `tokio::process` API; document any intentional gaps in rustdoc
- [x] P5-3: Write rustdoc for all public items in `worrywart` and `worrywart-client`; `cargo_doc` must produce zero warnings
- [ ] P5-4: `cargo_publish --dry-run` passes cleanly for both `worrywart` and `worrywart-client`
- [ ] P5-5: Tag `v0.1.0` and publish both crates to crates.io
---

## Phase 5 — C API (post-1.0)

A complete, standalone C implementation of the worrywart monitor and client —
not a binding to the Rust code.  Written in C, built with CMake (or a simple
`cl`/`clang-cl` build script), targeting Windows only.

- [ ] P5-1: Implement `worrywart_client` in C: `worrywart_client.h` + `worrywart_client.c` — `worrywart_init()`, `worrywart_notify_exit()`, `worrywart_notify_exit_reason()`; no Rust dependency
- [ ] P5-2: Implement `worrywart_monitor` in C: Job Object creation with kill-on-close, `PROC_THREAD_ATTRIBUTE_JOB_LIST` spawn, IOCP listener thread, exit classification — mirrors Phase 2 Rust behaviour
- [ ] P5-3: Implement Debug API monitoring in C: `WaitForDebugEvent` pump, exception correlation → crash/fastfail/unknown — mirrors Phase 1 Rust behaviour
- [ ] P5-4: Implement sentinel named-pipe support in C: monitor side creates pipe, client side writes sentinel before exit — mirrors Phase 3 Rust behaviour
- [ ] P5-5: Add a CMake (or nmake) build for the C library and a smoke test that exercises all three techniques end-to-end