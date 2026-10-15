//! Task runtime behavior against a fresh PostgreSQL database per test.

#[allow(dead_code, reason = "each test binary uses part of the shared fixture")]
#[path = "../../../../testing/fixtures/src/store.rs"]
mod store;

mod claims;
mod inputs;
mod lifecycle;
mod notifications;
mod operations;
mod owner;
mod reconnect;
mod recovery;
mod steps;
mod subscriptions;
mod support;
