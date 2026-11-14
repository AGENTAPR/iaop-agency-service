#![forbid(unsafe_code)]
//! Infrastructure layer for Agency Service.
//! Adapters: configuration, PostgreSQL persistence (org store,
//! idempotency, outbox publisher), JWT/JWKS validation, event transport.

pub mod config;
pub mod idempotency;
pub mod jwt;
pub mod messaging;
pub mod org_store;
pub mod outbox;
pub mod persistence;
pub mod security;
pub mod transport;
