// Helper binary: sleeps for 60 seconds.
// Used by Phase 2 tests to verify that dropping Worrywart kills the child.
fn main() {
    std::thread::sleep(std::time::Duration::from_secs(60));
}
