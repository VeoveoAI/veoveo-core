//! Shared test fixtures for repositories that consume `veoveo-core`. Core's own tests
//! include the same files by path, so each fixture has one source.

#[cfg(feature = "store")]
#[path = "../../store.rs"]
pub mod store;

#[cfg(feature = "tool-inputs")]
#[path = "../../tool_inputs.rs"]
pub mod tool_inputs;
