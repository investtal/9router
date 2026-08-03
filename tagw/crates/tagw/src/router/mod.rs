//! Account routing: round-robin selection and fail-over helpers.

pub mod account;

pub use account::{AccountRef, AccountRouter, MAX_FAILOVER_ATTEMPTS};
