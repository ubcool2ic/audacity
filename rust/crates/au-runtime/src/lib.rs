// SPDX-License-Identifier: GPL-3.0-or-later
// Part of the Audacity Rust port.
//! Runtime services: a rayon work-stealing pool for offline work (effect
//! render/export, import decode, waveform summaries, spectrograms), bounded
//! job queues with cancellation and progress, panic isolation, and local
//! crash reports with a recent-actions ring log.
//!
//! The real-time audio callback (`au-io`) never touches anything in this
//! crate: it does not lock, wait on, or submit to the pool. All results are
//! handed to the UI thread through channels it polls.

pub mod compute;
pub mod crash;
pub mod pool;

pub use pool::{CancelToken, JobError, JobHandle, Progress, Runtime};
