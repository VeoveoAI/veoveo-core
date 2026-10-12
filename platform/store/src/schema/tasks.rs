//! Tasks persistence declaration; the composition root supplies its lane execution host.

use veoveo_modules::*;

pub const SELECTION_V1: &str = include_str!("tasks/migrations/0000_selection.surql");

fn selection_api() -> Result<KernelSqlApi, DeclarationError> {
    fn record(table: &str) -> Result<SqlType, DeclarationError> {
        Ok(SqlType::Record(TableName::new(table)?))
    }
    fn optional(kind: SqlType) -> SqlType {
        SqlType::Option(Box::new(kind))
    }
    fn array(kind: SqlType) -> SqlType {
        SqlType::Array(Box::new(kind))
    }
    let parameters = vec![
        SqlParameter::new("task", record("task")?)?,
        SqlParameter::new("server", record("mcp_server")?)?,
        SqlParameter::new("tenant", record("tenant")?)?,
        SqlParameter::new("owner", record("principal")?)?,
        SqlParameter::new("profile", record("profile")?)?,
        SqlParameter::new("principal_key", SqlType::String)?,
        SqlParameter::new("profile_key", SqlType::String)?,
        SqlParameter::new("tenant_key", optional(SqlType::String))?,
        SqlParameter::new("labels", array(SqlType::String))?,
        SqlParameter::new("task_types", optional(array(SqlType::String)))?,
        SqlParameter::new("work_context", optional(record("work_context")?))?,
        SqlParameter::new("work_context_key", optional(SqlType::String))?,
        SqlParameter::new("authority_tenant", optional(SqlType::String))?,
    ];
    KernelSqlApi::new(
        FunctionName::new("fn::kernel::tasks::selection")?,
        MigrationVersion::new(0),
        SqlSignature::new(parameters, optional(SqlType::Object))?,
        SqlReadProfile::new(vec![TableName::new("task")?])?,
        SELECTION_V1,
    )
}

pub const LIFECYCLE_V1: &str = include_str!("tasks/migrations/0000_lifecycle.surql");

fn lifecycle_api() -> Result<KernelSqlApi, DeclarationError> {
    KernelSqlApi::new(
        FunctionName::new("fn::kernel::tasks::lifecycle")?,
        MigrationVersion::new(0),
        SqlSignature::new(
            vec![
                SqlParameter::new("task", SqlType::Record(TableName::new("task")?))?,
                SqlParameter::new("server", SqlType::Record(TableName::new("mcp_server")?))?,
                SqlParameter::new("tenant", SqlType::Record(TableName::new("tenant")?))?,
                SqlParameter::new("task_types", SqlType::Array(Box::new(SqlType::String)))?,
                SqlParameter::new(
                    "expected_result",
                    SqlType::Option(Box::new(SqlType::Object)),
                )?,
            ],
            SqlType::Option(Box::new(SqlType::Object)),
        )?,
        SqlReadProfile::new(vec![TableName::new("task")?])?,
        LIFECYCLE_V1,
    )
}

pub const PROVIDER_OBSERVATION_V1: &str =
    include_str!("tasks/migrations/0000_provider_observation.surql");
fn provider_observation_api() -> Result<KernelSqlApi, DeclarationError> {
    KernelSqlApi::new(
        FunctionName::new("fn::kernel::tasks::provider_observation")?,
        MigrationVersion::new(0),
        SqlSignature::new(
            vec![
                SqlParameter::new("job", SqlType::Record(TableName::new("provider_job")?))?,
                SqlParameter::new("task", SqlType::Record(TableName::new("task")?))?,
                SqlParameter::new("server", SqlType::Record(TableName::new("mcp_server")?))?,
                SqlParameter::new("tenant", SqlType::Record(TableName::new("tenant")?))?,
                SqlParameter::new("task_types", SqlType::Array(Box::new(SqlType::String)))?,
                SqlParameter::new("provider", SqlType::String)?,
            ],
            SqlType::Option(Box::new(SqlType::Object)),
        )?,
        SqlReadProfile::new(vec![
            TableName::new("provider_job")?,
            TableName::new("provider_event")?,
            TableName::new("task")?,
        ])?,
        PROVIDER_OBSERVATION_V1,
    )
}

pub const OWNER_CONTEXT_MATCHES_V1: &str =
    include_str!("tasks/migrations/0000_owner_context_matches.surql");
fn owner_context_matches_api() -> Result<KernelSqlApi, DeclarationError> {
    KernelSqlApi::new(
        FunctionName::new("fn::kernel::tasks::owner_context_matches")?,
        MigrationVersion::new(0),
        SqlSignature::new(
            vec![
                SqlParameter::new("task", SqlType::Record(TableName::new("task")?))?,
                SqlParameter::new("expected_owner_context", SqlType::Object)?,
            ],
            SqlType::Bool,
        )?,
        SqlReadProfile::new(vec![TableName::new("task")?])?,
        OWNER_CONTEXT_MATCHES_V1,
    )
}

pub const INPUT_MATCHES_V1: &str = include_str!("tasks/migrations/0000_input_matches.surql");
fn input_matches_api() -> Result<KernelSqlApi, DeclarationError> {
    KernelSqlApi::new(
        FunctionName::new("fn::kernel::tasks::input_matches")?,
        MigrationVersion::new(0),
        SqlSignature::new(
            vec![
                SqlParameter::new("task", SqlType::Record(TableName::new("task")?))?,
                SqlParameter::new("expected_input", SqlType::Object)?,
            ],
            SqlType::Bool,
        )?,
        SqlReadProfile::new(vec![TableName::new("task")?])?,
        INPUT_MATCHES_V1,
    )
}

pub const EXISTS_V1: &str = include_str!("tasks/migrations/0000_exists.surql");
fn exists_api() -> Result<KernelSqlApi, DeclarationError> {
    KernelSqlApi::new(
        FunctionName::new("fn::kernel::tasks::exists")?,
        MigrationVersion::new(0),
        SqlSignature::new(
            vec![SqlParameter::new(
                "task",
                SqlType::Record(TableName::new("task")?),
            )?],
            SqlType::Bool,
        )?,
        SqlReadProfile::new(vec![TableName::new("task")?])?,
        EXISTS_V1,
    )
}

pub const CURRENT_SCHEMA: &str = concat!(
    include_str!("tasks/migrations/0000_current.surql"),
    include_str!("tasks/migrations/0000_admission_apis.surql"),
    include_str!("tasks/migrations/0000_lifecycle.surql"),
    include_str!("tasks/migrations/0000_input_matches.surql"),
    include_str!("tasks/migrations/0000_exists.surql"),
    include_str!("tasks/migrations/0000_provider_observation.surql"),
    include_str!("tasks/migrations/0000_owner_context_matches.surql")
);

fn release_retention_api() -> Result<KernelSqlApi, DeclarationError> {
    KernelSqlApi::new(
        FunctionName::new("fn::kernel::tasks::release_retention")?,
        MigrationVersion::new(0),
        SqlSignature::new(
            vec![
                SqlParameter::new("task", SqlType::Record(TableName::new("task")?))?,
                SqlParameter::new(
                    "expected_server",
                    SqlType::Record(TableName::new("mcp_server")?),
                )?,
                SqlParameter::new("pin", SqlType::String)?,
            ],
            SqlType::Bool,
        )?,
        SqlReadProfile::new(vec![TableName::new("task")?])?,
        include_str!("tasks/migrations/0000_admission_apis.surql"),
    )
    .map(|api| {
        api.with_effects(SqlEffectProfile::OwnedUpdate(
            SqlUpdateProfile::new(
                TableName::new("task").expect("static Task table"),
                vec![SqlFieldName::new("retention_pins").expect("static retention field")],
            )
            .expect("static Task update profile"),
        ))
    })
}

/// Declare target ownership and dependencies without applying the mixed Store catalog.
pub fn module_setup(execution: LaneExecution) -> Result<ModuleSetup, DeclarationError> {
    ModuleSetup::builder(ModuleName::new("tasks")?, ModuleLayer::Kernel)
        .ownership(vec![
            OwnershipClaim::Function(FunctionName::new("fn::kernel::tasks::input_matches")?),
            OwnershipClaim::Function(FunctionName::new("fn::kernel::tasks::exists")?),
            OwnershipClaim::Function(FunctionName::new(
                "fn::kernel::tasks::provider_observation",
            )?),
            OwnershipClaim::Function(FunctionName::new("fn::kernel::tasks::release_retention")?),
            OwnershipClaim::Function(FunctionName::new("fn::kernel::tasks::lifecycle")?),
            OwnershipClaim::Function(FunctionName::new(
                "fn::kernel::tasks::owner_context_matches",
            )?),
            OwnershipClaim::Table(TableName::new("task")?),
            OwnershipClaim::Function(FunctionName::new("fn::kernel::tasks::selection")?),
            OwnershipClaim::Table(TableName::new("task_input")?),
            OwnershipClaim::Table(TableName::new("task_idempotency")?),
            OwnershipClaim::Table(TableName::new("task_produced_artifact")?),
            OwnershipClaim::Table(TableName::new("task_used_artifact")?),
            OwnershipClaim::Table(TableName::new("provider_job")?),
            OwnershipClaim::Table(TableName::new("provider_event")?),
            OwnershipClaim::Table(TableName::new("domain_usage")?),
        ])
        .lane(MigrationLane::new(vec![Migration::new(
            MigrationVersion::new(0),
            MigrationName::new("selection")?,
            concat!(
                include_str!("tasks/migrations/0000_current.surql"),
                include_str!("tasks/migrations/0000_selection.surql"),
                include_str!("tasks/migrations/0000_admission_apis.surql"),
                include_str!("tasks/migrations/0000_lifecycle.surql"),
                include_str!("tasks/migrations/0000_input_matches.surql"),
                include_str!("tasks/migrations/0000_exists.surql"),
                include_str!("tasks/migrations/0000_provider_observation.surql"),
                include_str!("tasks/migrations/0000_owner_context_matches.surql")
            ),
        )?])?)
        .sql_apis(vec![
            selection_api()?,
            lifecycle_api()?,
            provider_observation_api()?,
            owner_context_matches_api()?,
            input_matches_api()?,
            exists_api()?,
            release_retention_api()?,
        ])
        .execution(execution)
        .requires(vec![LaneRequirement::Satisfied(ModuleName::new(
            "artifacts",
        )?)])
        .build()
}
