//! Task runtime behavior against a fresh PostgreSQL database per test.

#[allow(dead_code, reason = "each test binary uses part of the shared fixture")]
#[path = "../../../../testing/fixtures/store.rs"]
mod store;

mod claims;
mod inputs;
mod lifecycle;
mod notifications;
mod owner;
mod recovery;
mod steps;
mod subscriptions;
mod support;
