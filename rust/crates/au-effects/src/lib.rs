// SPDX-License-Identifier: GPL-3.0-or-later
// Part of the Audacity Rust port. Derived from Audacity (c) Audacity Team, GPLv3.
//! Audacity built-in effects, generators and analyzers, realtime stacks and presets.
// approx_constant: literals like 0.7071 are copied from Audacity on purpose (bit-exact parity).
#![allow(clippy::too_many_arguments, clippy::needless_range_loop, clippy::approx_constant)]

mod basic;
pub mod core;
pub mod factory_presets;
pub mod fx;
pub mod offline;
pub mod registry;
pub mod stack;

pub use basic::*;
pub use crate::core::{BuiltinEffect, EffectContext, EffectError, EffectKind, OutLabel, ParamDef, ParamType};
