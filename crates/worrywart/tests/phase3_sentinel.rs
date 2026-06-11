// Phase 3 integration tests: Sentinel named-pipe monitor.
//
// These tests spawn real child processes through the `Worrywart` / `WorrywartCommand`
// API with `Monitor::Sentinel` and verify that:
//   P3-4a: A cooperative child that calls worrywart_notify_exit() before
//           exiting is classified as `TerminationReason::CleanExit`.
//   P3-4b: A child that is terminated externally (TerminateProcess) without
//           calling worrywart_notify_exit() is classified as
//           `TerminationReason::ExternalKill`.
//
// Helper binaries used:
//   - helper-sentinel: writes the sentinel message then returns from main()
//   - helper-sleep:    sleeps for 60 s; killed via child.kill() in the test

#[cfg(windows)]
mod sentinel_tests {
    use worrywart::core::Worrywart;
    use worrywart::{Monitor, TerminationReason};

    macro_rules! helper_exe {
        ($name:expr) => {
            env!(concat!("CARGO_BIN_EXE_", $name))
        };
    }

    // -------------------------------------------------------------------------
    // P3-4a: cooperative child sends sentinel → CleanExit
    // -------------------------------------------------------------------------
    #[test]
    fn cooperative_child_clean_exit() {
        let ww = Worrywart::new().expect("Worrywart::new");
        let mut child = ww
            .command(helper_exe!("helper-sentinel"))
            .monitor(Monitor::Sentinel)
            .spawn()
            .expect("spawn");

        let reason = child.wait_diagnosed().expect("wait_diagnosed");

        assert!(
            matches!(reason, TerminationReason::CleanExit(_)),
            "expected CleanExit, got {reason:?}"
        );
    }

    // -------------------------------------------------------------------------
    // P3-4b: TerminateProcess without sentinel → ExternalKill
    // -------------------------------------------------------------------------
    #[test]
    fn external_kill_no_sentinel() {
        let ww = Worrywart::new().expect("Worrywart::new");
        let mut child = ww
            .command(helper_exe!("helper-sleep"))
            .monitor(Monitor::Sentinel)
            .spawn()
            .expect("spawn");

        // Kill externally — helper-sleep never calls worrywart_notify_exit().
        child.kill().expect("kill");

        let reason = child.wait_diagnosed().expect("wait_diagnosed");

        assert!(
            matches!(reason, TerminationReason::ExternalKill(_)),
            "expected ExternalKill, got {reason:?}"
        );
    }
}
