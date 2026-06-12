<!-- Copyright (c) 2026 Michael Grier -->
# worrywart

A library that worries, maybe too much, about child processes

## Example

The core API (Windows only) provides full process-exit diagnostics:

```rust,no_run
#[cfg(windows)]
fn main() -> std::io::Result<()> {
    use worrywart::core::{Worrywart, WorrywartCommand};
    use worrywart::Monitor;

    let worrywart = Worrywart::new()?;
    let mut child = WorrywartCommand::new(&worrywart, "my-program")
        .arg("--flag")
        .monitor(Monitor::DebugApi)
        .spawn()?;

    let reason = child.wait_diagnosed()?;
    println!("{reason:?}");
    Ok(())
}
```

The compat layer (`worrywart::Command`) is a cross-platform drop-in for
`tokio::process::Command`.  Monitoring is not yet active in the compat layer
— `wait_diagnosed()` always returns `TerminationReason::Unknown` there.

## License

MIT
