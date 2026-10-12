use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Conformance report format.
pub const CONFORMANCE_REPORT_FORMAT: &str = "https://veoveo.ai/formats/mcp-conformance-report";

/// The conformance report format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[schemars(transform = report_format_naming)]
pub enum ConformanceReportFormat {
    #[serde(rename = "https://veoveo.ai/formats/mcp-conformance-report")]
    McpConformanceReport,
}

/// Outcome of one applicable requirement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Passed,
    Failed,
    Skipped,
    /// Required observations or traversal were unavailable.
    Incomplete,
}

/// One stable conformance requirement result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CheckResult {
    pub requirement_id: String,
    pub status: CheckStatus,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<Value>,
}

/// Server identity observed during MCP initialization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObservedImplementation {
    pub name: String,
    pub version: String,
    pub protocol_version: String,
}

/// Machine-readable result from one hosted-server certification run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConformanceReport {
    pub format: ConformanceReportFormat,
    pub profile_id: String,
    pub contract: veoveo_mcp_contract::docs::ContractId,
    pub started_at: DateTime<Utc>,
    pub completed_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub implementation: Option<ObservedImplementation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_capabilities: Option<Value>,
    pub checks: Vec<CheckResult>,
}

impl ConformanceReport {
    pub fn passed(&self) -> bool {
        self.checks
            .iter()
            .all(|check| !matches!(check.status, CheckStatus::Failed | CheckStatus::Incomplete))
    }
}

fn report_format_naming(schema: &mut schemars::Schema) {
    let profile = veoveo_types::ScalarNaming::owner(module_path!(), "conformance-report-format")
        .expect("static owner naming declaration");
    *schema = veoveo_types::scalar_schema(schema.clone(), profile)
        .expect("owned report format is scalar");
}
