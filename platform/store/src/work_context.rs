//! Work Contexts as the active control-plane revision defines them.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::types::Json;

use crate::{PlatformStore, StoreError, TenantId, WorkContextId};

/// Whether a grant or owner names a principal or a group.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, veoveo_types::Vocabulary)]
#[vocabulary(postgres)]
pub enum GrantSubjectKind {
    #[vocabulary(rename = "principal")]
    Principal,
    #[vocabulary(rename = "group")]
    Group,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, veoveo_types::Vocabulary)]
#[vocabulary(postgres)]
pub enum GrantPermission {
    #[vocabulary(rename = "read")]
    Read,
    #[vocabulary(rename = "write")]
    Write,
    #[vocabulary(rename = "admin")]
    Admin,
}

/// Membership levels in ascending order of authority.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, PartialOrd, Ord, veoveo_types::Vocabulary)]
#[vocabulary(postgres)]
pub enum WorkContextMembershipLevel {
    #[vocabulary(rename = "viewer")]
    Viewer,
    #[vocabulary(rename = "contributor")]
    Contributor,
    #[vocabulary(rename = "custodian")]
    Custodian,
    #[vocabulary(rename = "owner")]
    Owner,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkContextInitialGrantRecord {
    pub subject_kind: GrantSubjectKind,
    pub subject_key: String,
    pub permission: GrantPermission,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkContextOutputPolicyRecord {
    pub owner_kind: GrantSubjectKind,
    pub owner_key: String,
    pub initial_grants: Vec<WorkContextInitialGrantRecord>,
    pub classification: Option<String>,
    pub data_labels: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkContextMembershipRuleRecord {
    pub level: WorkContextMembershipLevel,
    pub principals: Vec<String>,
    pub groups: Vec<String>,
    pub roles: Vec<String>,
    pub oauth_clients: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkContextRecord {
    pub id: WorkContextId,
    pub tenant: TenantId,
    pub context_key: String,
    pub title: String,
    pub policy_revision: String,
    pub output_policy: WorkContextOutputPolicyRecord,
    pub memberships: Vec<WorkContextMembershipRuleRecord>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl WorkContextRecord {
    /// The highest level a rule naming the principal grants.
    pub fn membership_for_principal(
        &self,
        principal_key: &str,
    ) -> Option<WorkContextMembershipLevel> {
        self.memberships
            .iter()
            .filter(|rule| rule.principals.iter().any(|value| value == principal_key))
            .map(|rule| rule.level)
            .max()
    }

    /// The highest level a rule naming the OAuth client grants.
    pub fn membership_for_oauth_client(
        &self,
        oauth_client: &str,
    ) -> Option<WorkContextMembershipLevel> {
        self.memberships
            .iter()
            .filter(|rule| rule.oauth_clients.iter().any(|value| value == oauth_client))
            .map(|rule| rule.level)
            .max()
    }
}

impl PlatformStore {
    pub async fn work_context_by_key(
        &self,
        tenant: TenantId,
        context_key: &str,
    ) -> Result<Option<WorkContextRecord>, StoreError> {
        let row = sqlx::query_file!(
            "src/work_context/work_context_by_key.sql",
            tenant as TenantId,
            context_key,
        )
        .fetch_optional(self.pool())
        .await?;
        Ok(row.map(|row| WorkContextRecord {
            id: row.id,
            tenant: row.tenant,
            context_key: row.context_key,
            title: row.title,
            policy_revision: row.policy_revision,
            output_policy: WorkContextOutputPolicyRecord {
                owner_kind: row.owner_kind,
                owner_key: row.owner_key,
                initial_grants: row.initial_grants.0,
                classification: row.classification,
                data_labels: row.data_labels,
            },
            memberships: row.memberships.0,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn membership_takes_the_highest_matching_level() {
        let rule = |level, principals: &[&str], clients: &[&str]| WorkContextMembershipRuleRecord {
            level,
            principals: principals.iter().map(|value| (*value).to_owned()).collect(),
            groups: Vec::new(),
            roles: Vec::new(),
            oauth_clients: clients.iter().map(|value| (*value).to_owned()).collect(),
        };
        let context = WorkContextRecord {
            id: WorkContextId::from_uuid(uuid::Uuid::nil()),
            tenant: TenantId::from_uuid(uuid::Uuid::nil()),
            context_key: "development".into(),
            title: "Development".into(),
            policy_revision: "1".into(),
            output_policy: WorkContextOutputPolicyRecord {
                owner_kind: GrantSubjectKind::Group,
                owner_key: "builders".into(),
                initial_grants: Vec::new(),
                classification: None,
                data_labels: Vec::new(),
            },
            memberships: vec![
                rule(WorkContextMembershipLevel::Custodian, &["alice"], &[]),
                rule(
                    WorkContextMembershipLevel::Viewer,
                    &["alice", "bob"],
                    &["console"],
                ),
                rule(WorkContextMembershipLevel::Contributor, &[], &["console"]),
            ],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        assert_eq!(
            context.membership_for_principal("alice"),
            Some(WorkContextMembershipLevel::Custodian)
        );
        assert_eq!(
            context.membership_for_principal("bob"),
            Some(WorkContextMembershipLevel::Viewer)
        );
        assert_eq!(context.membership_for_principal("carol"), None);
        assert_eq!(
            context.membership_for_oauth_client("console"),
            Some(WorkContextMembershipLevel::Contributor)
        );
    }
}
