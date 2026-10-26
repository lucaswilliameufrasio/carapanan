//! Official Sign in with ChatGPT and OpenAI Responses API integration.
//!
//! This crate is separate from `carapana-core`: it owns network, OAuth and local
//! credential concerns. It never executes model-requested tools.

mod account;
mod auth;
mod credential_store;
mod provider;

pub use account::{AccountSummary, ChatGptAccount};
pub use auth::{AuthError, authenticate, enable_plan_usage, load_fresh_account};
pub use credential_store::{CredentialStore, CredentialStoreError};
pub use provider::{ChatGptModel, ChatGptProvider};
