//! Shared test fixtures for repositories that consume `veoveo-core`. Core's own tests
//! include the same files by path, so each fixture has one source.

#[cfg(feature = "store")]
pub mod store;

#[cfg(feature = "database-conformance")]
pub mod database;
