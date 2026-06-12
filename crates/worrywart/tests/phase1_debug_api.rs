// Phase 1 integration tests: Debug API monitoring.
//
// These tests spawn real child processes through the `Worrywart` / `WorrywartCommand`
// API with `Monitor::DebugApi` and verify that the pump thread correctly classifies
// each exit.
//
// Helper binaries used (built as [[bin]] targets in Cargo.toml):
//   - helper-exit0: exits cleanly → TerminationReason::Unknown(0)
//   - helper-crash: null-pointer dereference → TerminationReason::Crash { code: 0xC000_0005, .. }
//   - helper-fastfail: RaiseException(0xC000_0409, NONCONTINUABLE) → TerminationReason::FastFail

#[cfg(windows)]
mod debug_api_tests {
    use worrywart::core::{Worrywart, WorrywartChild};
    use worrywart::{Monitor, TerminationReason};

    /// Path to the named helper binary built by Cargo.
    macro_rules! helper_exe {
        ($name:expr) => {
            env!(concat!("CARGO_BIN_EXE_", $name))
        };
    }

    fn spawn_with_debug(program: &str) -> (Worrywart, WorrywartChild) {
        let ww = Worrywart::new().expect("Worrywart::new");
        let child = ww
            .command(program)
            .monitor(Monitor::DebugApi)
            .spawn()
            .expect("spawn");
        (ww, child)
    }

    // -------------------------------------------------------------------------
    // P1-6a: child that returns from main → TerminationReason::Unknown(exit_code=0)
    // -------------------------------------------------------------------------
    #[test]
    fn exit0_produces_unknown() {
        let (_ww, mut child) = spawn_with_debug(helper_exe!("helper-exit0"));
        let reason = child.wait_diagnosed().expect("wait_diagnosed");
        match reason {
            TerminationReason::Unknown(status) => {
                assert_eq!(
                    status.code(),
                    Some(0),
                    "expected exit code 0, got {status:?}"
                );
            }
            other => panic!("expected Unknown, got {other:?}"),
        }
    }

    // -------------------------------------------------------------------------
    // P1-6b: child that crashes (null-pointer write) → TerminationReason::Crash
    // Expected exception code: 0xC000_0005 (STATUS_ACCESS_VIOLATION)
    // -------------------------------------------------------------------------
    #[test]
    fn crash_produces_crash_reason() {
        let (_ww, mut child) = spawn_with_debug(helper_exe!("helper-crash"));
        let reason = child.wait_diagnosed().expect("wait_diagnosed");
        match reason {
            TerminationReason::Crash { code, .. } => {
                assert_eq!(
                    code, 0xC000_0005,
                    "expected STATUS_ACCESS_VIOLATION (0xC0000005), got {code:#010x}"
                );
            }
            other => panic!("expected Crash, got {other:?}"),
        }
    }

    // -------------------------------------------------------------------------
    // P1-6c: child that fast-fails → TerminationReason::FastFail
    // Expected exception code: 0xC000_0409 (STATUS_STACK_BUFFER_OVERRUN)
    // -------------------------------------------------------------------------
    #[test]
    fn fastfail_produces_fastfail_reason() {
        let (_ww, mut child) = spawn_with_debug(helper_exe!("helper-fastfail"));
        let reason = child.wait_diagnosed().expect("wait_diagnosed");
        match reason {
            TerminationReason::FastFail(code) => {
                assert_eq!(
                    code, 0xC000_0409,
                    "expected STATUS_STACK_BUFFER_OVERRUN (0xC0000409), got {code:#010x}"
                );
            }
            other => panic!("expected FastFail, got {other:?}"),
        }
    }
}
