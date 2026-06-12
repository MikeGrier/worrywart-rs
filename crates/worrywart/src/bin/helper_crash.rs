// Helper binary: triggers an access violation (null pointer dereference).
// Used only by Phase 1 integration tests to verify Crash detection.
fn main() {
    // codeql[rust/dereferenced-dangling-pointer] - Intentional null dereference; this binary exists solely to generate an access violation for test purposes.
    unsafe {
        let ptr: *mut u32 = std::ptr::null_mut();
        ptr.write(0xDEAD_BEEF);
    }
}
