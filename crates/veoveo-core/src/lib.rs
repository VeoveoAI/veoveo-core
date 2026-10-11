//! Placeholder crate that gives the repository's gates something to check until the
//! component's MCP server code arrives.

/// The component name, taken from the crate name.
pub fn component() -> &'static str {
    env!("CARGO_PKG_NAME")
}

#[cfg(test)]
mod tests {
    #[test]
    fn component_is_the_crate_name() {
        assert_eq!(super::component(), "veoveo-core");
    }
}
