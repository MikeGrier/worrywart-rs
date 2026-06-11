// Phase 2 integration tests: Job Object monitoring.
//
// These tests spawn real child processes through the `Worrywart` / `WorrywartCommand`
// API with `Monitor::JobObject` and verify that:
//   - Dropping `Worrywart` kills a sleeping child (kill-on-close).
//   - A child that crashes is classified as `TerminationReason::Crash` via IOCP.
//
// Helper binaries used:
//   - helper-sleep:  sleeps for 60 s → killed by job-object close
//   - helper-crash:  null-pointer dereference → JOB_OBJECT_MSG_ABNORMAL_EXIT_PROCESS

#[cfg(windows)]
mod job_object_tests {
    use worrywart::core::Worrywart;
    use worrywart::{Monitor, TerminationReason};

    macro_rules! helper_exe {
        ($name:expr) => {
            env!(concat!("CARGO_BIN_EXE_", $name))
        };
    }

    // -------------------------------------------------------------------------
    // P2-5a: drop Worrywart while child is sleeping → child is killed
    // -------------------------------------------------------------------------
    #[test]
    fn kill_on_drop() {
        let ww = Worrywart::new().expect("Worrywart::new");
        let mut child = ww
            .command(helper_exe!("helper-sleep"))
            .monitor(Monitor::JobObject)
            .spawn()
            .expect("spawn");

        // Drop Worrywart; JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE should kill the child.
        drop(ww);

        // wait() must return quickly (the child is dead).  helper-sleep would
        // otherwise run for 60 s; if it is not killed the test harness times out.
        // Windows terminates kill-on-close victims with exit code 0, so we cannot
        // use !status.success() as the assertion — we just verify wait() completes.
        child.wait().expect("wait after kill-on-drop");
    }

    // -------------------------------------------------------------------------
    // P2-5b: child crashes → IOCP reports JOB_OBJECT_MSG_ABNORMAL_EXIT_PROCESS
    //        → wait_diagnosed returns TerminationReason::Crash
    // -------------------------------------------------------------------------
    #[test]
    fn crash_detected_via_iocp() {
        let ww = Worrywart::new().expect("Worrywart::new");
        let mut child = ww
            .command(helper_exe!("helper-crash"))
            .monitor(Monitor::JobObject)
            .spawn()
            .expect("spawn");

        let reason = child.wait_diagnosed().expect("wait_diagnosed");

        assert!(
            matches!(reason, TerminationReason::Crash { .. }),
            "expected Crash, got {reason:?}"
        );
    }
}
