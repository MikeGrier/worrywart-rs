// Helper binary: calls std::process::abort(), which on Windows calls __fastfail
// (FAST_FAIL_FATAL_APP_EXIT=3), raising STATUS_STACK_BUFFER_OVERRUN (0xC000_0409).
// The debug pump should classify this as FastFail.
fn main() {
    std::process::abort();
}
