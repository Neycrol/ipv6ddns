//! Validation utilities for ipv6ddns (re-exports).
//!
//! The implementations live in the [`textops`] workspace crate so they can be
//! compiled at `opt-level = 3` independently of the binary's release profile
//! (see root Cargo.toml). This shim keeps the historical import paths stable.

pub use textops::{is_valid_ipv6, validate_record_name};
