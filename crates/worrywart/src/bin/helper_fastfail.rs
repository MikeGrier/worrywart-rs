// Helper binary: deterministically triggers __fastfail (the `int 29h`
// instruction) with FAST_FAIL_FATAL_APP_EXIT, raising the non-continuable
// STATUS_STACK_BUFFER_OVERRUN (0xC000_0409) as a *second-chance* exception so
// the Debug API tests observe a stable TerminationReason::FastFail code.
//
// std::process::abort() also routes through __fastfail on today's MSVC CRT,
// but that is a CRT implementation detail rather than a stable contract — and
// the debug pump only correlates *second-chance* exceptions.  Issuing `int
// 29h` directly removes the CRT from the picture and guarantees both the exact
// exception code and that it arrives as the second-chance event the pump
// records.  (A plain RaiseException would arrive first-chance and be ignored.)
#[cfg(all(windows, any(target_arch = "x86_64", target_arch = "x86")))]
fn main() {
    use std::arch::asm;

    // FAST_FAIL_FATAL_APP_EXIT is passed in ECX; `int 29h` then raises the
    // non-continuable STATUS_STACK_BUFFER_OVERRUN second-chance exception.
    const FAST_FAIL_FATAL_APP_EXIT: u32 = 7;
    unsafe {
        asm!("int 29h", in("ecx") FAST_FAIL_FATAL_APP_EXIT, options(noreturn));
    }
}

// Fallback for non-x86 Windows targets (and non-Windows): abort() routes
// through __fastfail on MSVC and aborts elsewhere.
#[cfg(not(all(windows, any(target_arch = "x86_64", target_arch = "x86"))))]
fn main() {
    std::process::abort();
}
