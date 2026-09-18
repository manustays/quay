//! Shared internals of the Quay hook helper.
//!
//! The helper is a binary, but the app needs the same process-identity lookup it
//! writes: whatever records a `(pid, startedAt)` pair and whatever later checks that
//! pair must agree exactly on what it means. Duplicating the libproc FFI in both
//! crates is how those two quietly drift apart.
//!
//! A library dependency only — the app must not gain a second `[[bin]]`, which the
//! universal build cannot lipo (see this crate's Cargo.toml).

pub mod proc_info;
