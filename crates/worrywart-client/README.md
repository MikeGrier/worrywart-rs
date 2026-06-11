# worrywart-client

Client-side sentinel support for the [`worrywart`](https://crates.io/crates/worrywart)
child-process monitor.

Child processes that are monitored via `Monitor::Sentinel` should call
[`notify_exit`] immediately before intentionally exiting.  The worrywart
monitor uses this signal to classify the exit as `CleanExit` rather than
`ExternalKill`.

On non-Windows platforms the functions are no-ops and always return `false`.

## Usage

```toml
[dependencies]
worrywart-client = "0.1"
```

```rust
fn main() {
    // ... do work ...
    worrywart_client::notify_exit(0);
    std::process::exit(0);
}
```

## License

MIT — see [LICENSE](../../LICENSE).
