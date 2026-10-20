//! `au3-rs`: Rust ports of Audacity 4 `au3` C++ library code.
//!
//! The crate is built as a `staticlib` that the C++ `au3-*` libraries link
//! against; every ported item has a `#[no_mangle] extern "C"` wrapper declared
//! in `au3/rust/include/`.

#![deny(unsafe_op_in_unsafe_fn)]

/// Mathematical utilities and DSP building blocks.
pub mod math;
