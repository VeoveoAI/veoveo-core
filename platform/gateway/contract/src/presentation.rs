//! Gateway transport, owned HTTP route and health vocabularies.
#[derive(Clone, Copy, Debug, Eq, PartialEq, veoveo_types::Vocabulary)]
pub enum GatewayServerHealthState {
    #[vocabulary(rename = "healthy")]
    Healthy,
    #[vocabulary(rename = "degraded")]
    Degraded,
    #[vocabulary(rename = "offline")]
    Offline,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, veoveo_types::Vocabulary)]
pub enum UpstreamTransport {
    #[vocabulary(rename = "streamable_http")]
    StreamableHttp,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, veoveo_types::Vocabulary)]
pub enum OwnedRoutePurpose {
    #[vocabulary(rename = "webhook")]
    Webhook,
    #[vocabulary(rename = "artifact_bytes")]
    ArtifactBytes,
    #[vocabulary(rename = "provider_fetchable_files")]
    ProviderFetchableFiles,
    #[vocabulary(rename = "health")]
    Health,
}

/// The authenticated administrator's registered-server health snapshot.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServerHealthReport {
    pub servers: Vec<ServerHealthEntry>,
}

/// State and timestamp are null until the server's first probe after startup.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServerHealthEntry {
    pub server: veoveo_types::ServerSlug,
    pub state: Option<GatewayServerHealthState>,
    pub checked_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_snapshot_preserves_null_probe() {
        let wire = serde_json::json!({
            "servers": [{"server":"frames", "state":null, "checkedAt":null}]
        });
        let report: ServerHealthReport = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(report.servers[0].state, None);
        assert_eq!(serde_json::to_value(report).unwrap(), wire);
    }

    #[test]
    fn health_snapshot_rejects_unknown_fields_and_states() {
        for wire in [
            serde_json::json!({"servers":[], "extra":true}),
            serde_json::json!({"servers":[{"server":"frames", "state":"unknown", "checkedAt":null}]}),
            serde_json::json!({"servers":[], "moduleBindings":[]}),
        ] {
            assert!(serde_json::from_value::<ServerHealthReport>(wire).is_err());
        }
    }
}
