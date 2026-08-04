//! Provider registry: API-key providers (Task 7) and OAuth (Task 8).
//!
//! OAuth connect + refresh lives in [`crate::oauth`]; both kinds feed
//! [`crate::cache::ConfigCache`] account pools.

pub mod api_key;

pub use api_key::{
    create_account, create_provider, list_providers, load_account_pools, set_account_enabled,
    set_provider_enabled, AccountPublic, ApiKeyProviderType, CreateAccountRequest,
    CreateProviderRequest, ProviderPublic,
};
