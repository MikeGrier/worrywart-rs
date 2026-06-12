// Copyright (c) 2026 Michael Grier

#![deny(missing_docs)]

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
//! ## Cross-platform drop-in (compat layer)
//!
//! For callers migrating from `tokio::process`, replace your import and
//! everything continues to work identically.  [`Command`] and [`Child`]
//! mirror `tokio::process` exactly.
//!
//! **Current limitation:** monitoring is not yet implemented in the compat
//! layer.  [`Command::monitor`] stores the requested technique but does not
//! apply it, and [`Child::wait_diagnosed`] always returns
//! [`TerminationReason::Unknown`].  Use the [`core`] types below for actual
//! diagnostics.
//!
//! ## Windows diagnostic API (core layer)
//!
//! On Windows, use [`core::WorrywartCommand`] to spawn children with full
//! monitoring.  Add `.monitor(Monitor::DebugApi)` (or `Monitor::JobObject` /
//! `Monitor::Sentinel`) to the builder and call `.wait_diagnosed()` on the
//! returned [`core::WorrywartChild`] to get a classified [`TerminationReason`].

pub mod compat;
#[cfg(windows)]
pub mod core;
#[cfg(windows)]
pub(crate) mod iocp;
mod monitor;
#[cfg(windows)]
pub(crate) mod pump;
#[cfg(windows)]
pub(crate) mod sentinel;
mod termination;

pub use monitor::Monitor;
pub use termination::TerminationReason;

// Re-export compat types at the crate root so callers can write
// `use worrywart::{Command, Child}` as a drop-in for `tokio::process`.
pub use compat::{Child, Command};

// Re-export core types for callers who want the correct ownership model.
#[cfg(windows)]
pub use core::{Worrywart, WorrywartChild, WorrywartCommand};
