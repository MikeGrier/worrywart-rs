// Copyright (c) 2026 Michael Grier
//! Helper binary: writes the worrywart sentinel message then exits normally.
//!
//! Used by Phase 3 integration tests to verify that a cooperative child
//! is classified as `CleanExit`.

#[cfg(windows)]
fn main() {
    use windows_sys::Win32::Storage::FileSystem::WriteFile;

    const SENTINEL_MAGIC: [u8; 4] = *b"WORT";
    const ENV_VAR: &str = "WORRYWART_SENTINEL_HANDLE";

    if let Ok(val) = std::env::var(ENV_VAR) {
        if let Ok(n) = val.parse::<usize>() {
            if n != 0 {
                let handle = n as windows_sys::Win32::Foundation::HANDLE;
                let mut msg = [0u8; 8];
                msg[0..4].copy_from_slice(&SENTINEL_MAGIC);
                // exit code 0 encoded as LE i32
                msg[4..8].copy_from_slice(&0i32.to_le_bytes());
                let mut written: u32 = 0;
                let ok = unsafe {
                    WriteFile(
                        handle,
                        msg.as_ptr() as *const _,
                        8,
                        &mut written,
                        std::ptr::null_mut(),
                    )
                };
                let _ = ok;
                let _ = written;
            }
        }
    }
}

#[cfg(not(windows))]
fn main() {}
