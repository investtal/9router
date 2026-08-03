//! Account routing: round-robin selection and fail-over helpers.

pub mod account;
pub mod model_route;

pub use account::{AccountRef, AccountRouter, MAX_FAILOVER_ATTEMPTS};
pub use model_route::{resolve_openai_pool_key, type_pool_key};
