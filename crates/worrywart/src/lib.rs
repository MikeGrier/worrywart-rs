// Copyright (c) 2026 Michael Grier

//! A library that worries, maybe too much, about child processes.
//!
//! # Overview
//!
//! `worrywart` launches and monitors child processes with enough diagnostic
//! fidelity to distinguish clean exits, crashes, fast-fails, and external
//! kills (e.g. Windows Defender ASR).
//!
//! # Usage
//!
//! For callers migrating from `tokio::process`, replace your import and
//! everything continues to work.  Add `.monitor(Monitor::DebugApi)` to the
//! builder to enable diagnosis; call `.wait_diagnosed()` instead of
//! `.wait()` to get a [`TerminationReason`].
//!
//! Refer to the [tokio::process documentation](https://docs.rs/tokio/latest/tokio/process/index.html).
//! All standard builder and child methods work identically.  The only
//! additions are `.monitor()` on the builder and `.wait_diagnosed()` on the
//! child.

pub mod compat;
pub mod core;
mod monitor;
#[cfg(windows)]
pub(crate) mod pump;
mod termination;

pub use monitor::Monitor;
pub use termination::TerminationReason;

// Re-export compat types at the crate root so callers can write
// `use worrywart::{Command, Child}` as a drop-in for `tokio::process`.
pub use compat::{Child, Command};

// Re-export core types for callers who want the correct ownership model.
#[cfg(windows)]
pub use core::{Worrywart, WorrywartChild, WorrywartCommand};
