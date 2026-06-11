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

mod termination;

pub use termination::TerminationReason;
