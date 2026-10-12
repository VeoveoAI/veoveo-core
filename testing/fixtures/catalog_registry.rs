//! A catalog registry with only the Gateway's kernel declarations. Servers contribute
//! their own declarations through registration, so core tests name no server.
use std::sync::OnceLock;
use veoveo_gateway_contract::{CatalogRegistry, CatalogRegistryBuilder, GatewayAction};
use veoveo_types::ExtensionName;

pub fn fresh_registry() -> CatalogRegistry {
    let mut builder = CatalogRegistryBuilder::new(
        [
            "branding",
            "identity_providers",
            "authorization_servers",
            "servers",
            "profiles",
            "tenants",
            "work_contexts",
            "policies",
            "data_labels",
            "oauth_clients",
            "oidc_clients",
            "secrets",
            "metadata",
        ]
        .into_iter()
        .map(String::from),
        [
            "gateway",
            "server",
            "tool",
            "resource",
            "resource_template",
            "prompt",
            "task",
            "platform_task",
            "artifact",
            "usage",
        ]
        .into_iter()
        .map(|name| ExtensionName::parse(name).expect("kernel extension name"))
        .collect::<Vec<_>>(),
    );
    builder
        .register_kernel::<GatewayAction>()
        .expect("kernel catalog declarations");
    builder.build().expect("kernel catalog registry")
}

#[allow(dead_code, reason = "Some consumers build a fresh registry per test")]
pub fn registry() -> CatalogRegistry {
    static REGISTRY: OnceLock<CatalogRegistry> = OnceLock::new();
    REGISTRY.get_or_init(fresh_registry).clone()
}
