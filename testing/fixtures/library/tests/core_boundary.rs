//! Core stays protocol-independent: no package in its lockfile, the closure of every core
//! crate over all features and targets, is an MCP implementation, a Veoveo MCP or SDK
//! crate, or a component.

/// Package names that only a protocol layer or a component may depend on.
fn forbidden(name: &str) -> bool {
    name == "rmcp"
        || name.starts_with("rmcp-")
        || ["veoveo-mcp-", "veoveo-sdk-", "veoveo-service-"]
            .iter()
            .any(|prefix| name.starts_with(prefix))
}

#[test]
fn no_core_crate_depends_on_mcp_or_a_component() {
    let lock = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../Cargo.lock"))
        .expect("core's Cargo.lock is readable");
    let packages: Vec<&str> = lock
        .lines()
        .filter_map(|line| line.strip_prefix("name = \""))
        .filter_map(|rest| rest.strip_suffix('"'))
        .collect();
    assert!(
        packages.contains(&"veoveo-platform-store"),
        "the lockfile lists core's own crates"
    );
    let found: Vec<&str> = packages
        .into_iter()
        .filter(|name| forbidden(name))
        .collect();
    assert!(
        found.is_empty(),
        "core depends on protocol or component packages: {found:?}"
    );
}

#[test]
fn the_boundary_names_protocol_and_component_packages() {
    for name in [
        "rmcp",
        "rmcp-macros",
        "veoveo-mcp-contract",
        "veoveo-sdk-rust",
        "veoveo-service-gateway",
    ] {
        assert!(forbidden(name), "{name}");
    }
    for name in ["veoveo-types", "veoveo-platform-store", "sqlx", "rmcpx"] {
        assert!(!forbidden(name), "{name}");
    }
}
