// Helper binary: triggers an access violation (null pointer dereference).
fn main() {
    unsafe {
        let ptr: *mut u32 = std::ptr::null_mut();
        ptr.write(0xDEAD_BEEF);
    }
}
