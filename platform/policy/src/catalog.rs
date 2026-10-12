use std::{collections::BTreeMap, sync::Arc};
use veoveo_mcp_contract::{
    DataLabelDefinition, GatewayControlPlane, GatewayControlPlaneError, GatewayProfile,
    GatewayProfileId, InstallationAuthorization, PolicySet, ServerSlug, TenantDefinition,
};
use veoveo_mcp_registration_extension::AuthorizationDeclaration;
use veoveo_types::{DataLabelId, PolicyVersion, TenantId};

/// Implementations expose one validated, immutable control-plane revision and one
/// registration snapshot. Lookups must never combine fields from different revisions.
pub trait PolicyCatalogView {
    fn registry(&self) -> &veoveo_gateway_contract::CatalogRegistry;
    fn sections(&self) -> &veoveo_gateway_contract::AdmittedCatalogSections;
    fn profile(&self, id: &GatewayProfileId) -> Option<&GatewayProfile>;
    fn policy(&self, id: &PolicyVersion) -> Option<&PolicySet>;
    fn data_label(&self, id: &DataLabelId) -> Option<&DataLabelDefinition>;
    fn tenant(&self, id: &TenantId) -> Option<&TenantDefinition>;
    fn installation_authorization(&self) -> &InstallationAuthorization;
    /// The checked authorization declaration of a currently registered server.
    fn server_authorization(&self, server: &ServerSlug) -> Option<&AuthorizationDeclaration>;
}

/// A validated shared snapshot for services that do not need gateway transports.
#[derive(Clone)]
pub struct PolicyCatalog {
    control_plane: Arc<GatewayControlPlane>,
    registry: veoveo_gateway_contract::CatalogRegistry,
    sections: veoveo_gateway_contract::AdmittedCatalogSections,
    profiles: BTreeMap<GatewayProfileId, usize>,
    policies: BTreeMap<PolicyVersion, usize>,
    labels: BTreeMap<DataLabelId, usize>,
    tenants: BTreeMap<TenantId, usize>,
    servers: BTreeMap<ServerSlug, Arc<AuthorizationDeclaration>>,
}
impl PolicyCatalog {
    pub fn new(
        control_plane: GatewayControlPlane,
        registry: veoveo_gateway_contract::CatalogRegistry,
    ) -> Result<Self, GatewayControlPlaneError> {
        let sections = control_plane.validate(&registry)?;
        Ok(Self {
            registry,
            sections,
            profiles: control_plane
                .profiles
                .iter()
                .enumerate()
                .map(|(i, p)| (p.id.clone(), i))
                .collect(),
            policies: control_plane
                .policies
                .iter()
                .enumerate()
                .map(|(i, p)| (p.version.clone(), i))
                .collect(),
            labels: control_plane
                .data_labels
                .iter()
                .enumerate()
                .map(|(i, p)| (p.id.clone(), i))
                .collect(),
            tenants: control_plane
                .tenants
                .iter()
                .enumerate()
                .map(|(i, p)| (p.id.clone(), i))
                .collect(),
            servers: BTreeMap::new(),
            control_plane: Arc::new(control_plane),
        })
    }
    pub fn registry(&self) -> &veoveo_gateway_contract::CatalogRegistry {
        &self.registry
    }
    pub fn sections(&self) -> &veoveo_gateway_contract::AdmittedCatalogSections {
        &self.sections
    }
    pub fn control_plane(&self) -> &GatewayControlPlane {
        &self.control_plane
    }
    /// Add one registered server's declaration. The caller has already checked it
    /// against the namespace pinned to the server's key.
    pub fn with_server(
        mut self,
        server: ServerSlug,
        declaration: AuthorizationDeclaration,
    ) -> Self {
        self.servers.insert(server, Arc::new(declaration));
        self
    }
}
impl PolicyCatalogView for PolicyCatalog {
    fn registry(&self) -> &veoveo_gateway_contract::CatalogRegistry {
        &self.registry
    }
    fn sections(&self) -> &veoveo_gateway_contract::AdmittedCatalogSections {
        &self.sections
    }
    fn profile(&self, id: &GatewayProfileId) -> Option<&GatewayProfile> {
        self.profiles
            .get(id)
            .map(|i| &self.control_plane.profiles[*i])
    }
    fn policy(&self, id: &PolicyVersion) -> Option<&PolicySet> {
        self.policies
            .get(id)
            .map(|i| &self.control_plane.policies[*i])
    }
    fn data_label(&self, id: &DataLabelId) -> Option<&DataLabelDefinition> {
        self.labels
            .get(id)
            .map(|i| &self.control_plane.data_labels[*i])
    }
    fn tenant(&self, id: &TenantId) -> Option<&TenantDefinition> {
        self.tenants
            .get(id)
            .map(|i| &self.control_plane.tenants[*i])
    }
    fn installation_authorization(&self) -> &InstallationAuthorization {
        &self.control_plane.authorization
    }
    fn server_authorization(&self, server: &ServerSlug) -> Option<&AuthorizationDeclaration> {
        self.servers.get(server).map(Arc::as_ref)
    }
}
