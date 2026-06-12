<!-- Copyright (c) 2026 Michael Grier -->
# worrywart

A library that worries, maybe too much, about child processes

## Example

```rust,no_run
use worrywart::{Command, Monitor};

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let mut child = Command::new("my-program")
        .arg("--flag")
        .monitor(Monitor::JobObject)
        .spawn()?;

    let reason = child.wait_diagnosed().await?;
    println!("{reason:?}");
    Ok(())
}
```

## License

MIT
