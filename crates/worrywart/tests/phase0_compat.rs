// Copyright (c) 2026 Michael Grier

//! Phase 0 integration tests — compat layer end-to-end.
//!
//! These tests verify that `worrywart::Command` and `worrywart::Child`
//! function correctly as a drop-in for `tokio::process` before any
//! monitoring instrumentation is implemented.

use worrywart::Command;

/// Spawn a no-op process via the compat `Command`, wait for it to exit,
/// and assert that the exit status indicates success.
#[tokio::test]
async fn compat_spawn_and_wait_success() {
    let mut cmd = if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.args(["/c", "exit 0"]);
        c
    } else {
        Command::new("true")
    };

    let mut child = cmd.spawn().expect("failed to spawn child");
    let status = child.wait().await.expect("failed to wait");
    assert!(
        status.success(),
        "expected success exit status, got {status}"
    );
}

/// `wait_diagnosed()` returns `TerminationReason::Unknown` in Phase 0
/// because no monitoring techniques are implemented yet.
#[tokio::test]
async fn compat_wait_diagnosed_returns_unknown() {
    let mut cmd = if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.args(["/c", "exit 0"]);
        c
    } else {
        Command::new("true")
    };

    let mut child = cmd.spawn().expect("failed to spawn child");
    let reason = child
        .wait_diagnosed()
        .await
        .expect("failed to wait_diagnosed");

    assert!(
        matches!(reason, worrywart::TerminationReason::Unknown(_)),
        "expected Unknown in Phase 0, got {reason:?}"
    );
}
