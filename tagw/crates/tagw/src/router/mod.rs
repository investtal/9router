//! Account routing: round-robin selection and fail-over helpers.

pub mod account;
pub mod model_route;

pub use account::{AccountRef, AccountRouter, MAX_FAILOVER_ATTEMPTS};
pub use model_route::{
    parse_model, resolve_openai_pool_key, rewrite_body_model, type_pool_key, ParsedModel,
};
