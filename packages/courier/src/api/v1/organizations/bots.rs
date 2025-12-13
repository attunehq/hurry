//! Organization bots endpoints.
//!
//! Bots are organization-scoped accounts without GitHub identity, used for CI
//! systems and automation. To revoke a bot, disable its account using the
//! account management endpoints.

pub mod create;
pub mod list;
