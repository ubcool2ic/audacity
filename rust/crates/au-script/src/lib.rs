//! Audacity scripting commands and macros.
pub mod cmd;
pub mod macros;
pub use cmd::{execute, parse_command, ScriptHost, Selection};
