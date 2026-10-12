//! Identity persistence declaration; the composition root supplies its lane execution host.

use veoveo_modules::*;

pub const CURRENT_SCHEMA: &str = concat!(
    include_str!("identity/migrations/0000_current.surql"),
    include_str!("identity/migrations/0000_admission_apis.surql"),
    include_str!("identity/migrations/principal_summaries.surql"),
    include_str!("identity/migrations/search_enabled_users.surql"),
    include_str!("identity/migrations/enabled_user.surql"),
    include_str!("identity/migrations/identity_labels.surql"),
    include_str!("identity/migrations/tenant_current.surql"),
    include_str!("identity/migrations/principal_current.surql"),
    include_str!("identity/migrations/tenant_matches.surql"),
    include_str!("identity/migrations/oauth_client_exists.surql"),
    include_str!("identity/migrations/service_principal_available.surql"),
    include_str!("identity/migrations/context_snapshot_matches.surql"),
    include_str!("identity/migrations/create_service_principal.surql"),
    include_str!("identity/migrations/tenant_context_keys.surql")
);

fn actor_admitted_api() -> Result<KernelSqlApi, DeclarationError> {
    KernelSqlApi::new(
        FunctionName::new("fn::kernel::identity::actor_admitted")?,
        MigrationVersion::new(0),
        SqlSignature::new(
            vec![
                SqlParameter::new("tenant", SqlType::Record(TableName::new("tenant")?))?,
                SqlParameter::new("context", SqlType::Record(TableName::new("work_context")?))?,
                SqlParameter::new("principal", SqlType::Record(TableName::new("principal")?))?,
                SqlParameter::new("context_digest", SqlType::String)?,
                SqlParameter::new("require_human", SqlType::Bool)?,
            ],
            SqlType::Bool,
        )?,
        SqlReadProfile::new(vec![
            TableName::new("tenant")?,
            TableName::new("work_context")?,
            TableName::new("principal")?,
        ])?,
        include_str!("identity/migrations/actor_admitted.surql"),
    )
}

fn identity_enabled_api() -> Result<KernelSqlApi, DeclarationError> {
    KernelSqlApi::new(
        FunctionName::new("fn::kernel::identity::identity_enabled")?,
        MigrationVersion::new(0),
        SqlSignature::new(
            vec![
                SqlParameter::new("tenant", SqlType::Record(TableName::new("tenant")?))?,
                SqlParameter::new("context", SqlType::Record(TableName::new("work_context")?))?,
                SqlParameter::new("principal", SqlType::Record(TableName::new("principal")?))?,
            ],
            SqlType::Bool,
        )?,
        SqlReadProfile::new(vec![
            TableName::new("tenant")?,
            TableName::new("work_context")?,
            TableName::new("principal")?,
        ])?,
        include_str!("identity/migrations/identity_enabled.surql"),
    )
}

fn principal_summaries_api() -> Result<KernelSqlApi, DeclarationError> {
    KernelSqlApi::new(
        FunctionName::new("fn::kernel::identity::principal_summaries")?,
        MigrationVersion::new(0),
        SqlSignature::new(
            vec![
                SqlParameter::new("tenant", SqlType::Record(TableName::new("tenant")?))?,
                SqlParameter::new(
                    "principals",
                    SqlType::Array(Box::new(SqlType::Record(TableName::new("principal")?))),
                )?,
            ],
            SqlType::Array(Box::new(SqlType::Object)),
        )?,
        SqlReadProfile::new(vec![TableName::new("principal")?])?,
        include_str!("identity/migrations/principal_summaries.surql"),
    )
}

fn search_enabled_users_api() -> Result<KernelSqlApi, DeclarationError> {
    KernelSqlApi::new(
        FunctionName::new("fn::kernel::identity::search_enabled_users")?,
        MigrationVersion::new(0),
        SqlSignature::new(
            vec![
                SqlParameter::new("tenant", SqlType::Record(TableName::new("tenant")?))?,
                SqlParameter::new("search", SqlType::String)?,
            ],
            SqlType::Array(Box::new(SqlType::Object)),
        )?,
        SqlReadProfile::new(vec![TableName::new("principal")?])?,
        include_str!("identity/migrations/search_enabled_users.surql"),
    )
}

fn enabled_user_api() -> Result<KernelSqlApi, DeclarationError> {
    KernelSqlApi::new(
        FunctionName::new("fn::kernel::identity::enabled_user")?,
        MigrationVersion::new(0),
        SqlSignature::new(
            vec![
                SqlParameter::new("tenant", SqlType::Record(TableName::new("tenant")?))?,
                SqlParameter::new("principal", SqlType::Record(TableName::new("principal")?))?,
            ],
            SqlType::Bool,
        )?,
        SqlReadProfile::new(vec![TableName::new("principal")?])?,
        include_str!("identity/migrations/enabled_user.surql"),
    )
}

fn identity_labels_api() -> Result<KernelSqlApi, DeclarationError> {
    KernelSqlApi::new(
        FunctionName::new("fn::kernel::identity::identity_labels")?,
        MigrationVersion::new(0),
        SqlSignature::new(
            vec![
                SqlParameter::new("tenant", SqlType::Record(TableName::new("tenant")?))?,
                SqlParameter::new("context", SqlType::Record(TableName::new("work_context")?))?,
                SqlParameter::new("principal", SqlType::Record(TableName::new("principal")?))?,
            ],
            SqlType::Option(Box::new(SqlType::Object)),
        )?,
        SqlReadProfile::new(vec![
            TableName::new("tenant")?,
            TableName::new("work_context")?,
            TableName::new("principal")?,
        ])?,
        include_str!("identity/migrations/identity_labels.surql"),
    )
}

fn tenant_current_api() -> Result<KernelSqlApi, DeclarationError> {
    KernelSqlApi::new(
        FunctionName::new("fn::kernel::identity::tenant_current")?,
        MigrationVersion::new(0),
        SqlSignature::new(
            vec![
                SqlParameter::new("enterprise", SqlType::Record(TableName::new("enterprise")?))?,
                SqlParameter::new("tenant", SqlType::Record(TableName::new("tenant")?))?,
            ],
            SqlType::Option(Box::new(SqlType::Object)),
        )?,
        SqlReadProfile::new(vec![
            TableName::new("enterprise")?,
            TableName::new("tenant")?,
        ])?,
        include_str!("identity/migrations/tenant_current.surql"),
    )
}

fn principal_current_api() -> Result<KernelSqlApi, DeclarationError> {
    KernelSqlApi::new(
        FunctionName::new("fn::kernel::identity::principal_current")?,
        MigrationVersion::new(0),
        SqlSignature::new(
            vec![
                SqlParameter::new("tenant", SqlType::Record(TableName::new("tenant")?))?,
                SqlParameter::new("principal", SqlType::Record(TableName::new("principal")?))?,
            ],
            SqlType::Option(Box::new(SqlType::Object)),
        )?,
        SqlReadProfile::new(vec![TableName::new("principal")?])?,
        include_str!("identity/migrations/principal_current.surql"),
    )
}

fn tenant_matches_api() -> Result<KernelSqlApi, DeclarationError> {
    KernelSqlApi::new(
        FunctionName::new("fn::kernel::identity::tenant_matches")?,
        MigrationVersion::new(0),
        SqlSignature::new(
            vec![
                SqlParameter::new("enterprise", SqlType::Record(TableName::new("enterprise")?))?,
                SqlParameter::new("tenant", SqlType::Record(TableName::new("tenant")?))?,
                SqlParameter::new("slug", SqlType::String)?,
            ],
            SqlType::Bool,
        )?,
        SqlReadProfile::new(vec![
            TableName::new("enterprise")?,
            TableName::new("tenant")?,
        ])?,
        include_str!("identity/migrations/tenant_matches.surql"),
    )
}

fn oauth_client_exists_api() -> Result<KernelSqlApi, DeclarationError> {
    let api = KernelSqlApi::new(
        FunctionName::new("fn::kernel::identity::oauth_client_exists")?,
        MigrationVersion::new(0),
        SqlSignature::new(
            vec![SqlParameter::new("client", SqlType::String)?],
            SqlType::Bool,
        )?,
        SqlReadProfile::new(vec![TableName::new("oauth_client")?])?,
        include_str!("identity/migrations/oauth_client_exists.surql"),
    )?;
    Ok(api)
}

fn service_principal_available_api() -> Result<KernelSqlApi, DeclarationError> {
    let api = KernelSqlApi::new(
        FunctionName::new("fn::kernel::identity::service_principal_available")?,
        MigrationVersion::new(0),
        SqlSignature::new(
            vec![
                SqlParameter::new("principal", SqlType::Record(TableName::new("principal")?))?,
                SqlParameter::new("client", SqlType::String)?,
            ],
            SqlType::Bool,
        )?,
        SqlReadProfile::new(vec![
            TableName::new("principal")?,
            TableName::new("oauth_client")?,
        ])?,
        include_str!("identity/migrations/service_principal_available.surql"),
    )?;
    Ok(api)
}

fn context_snapshot_matches_api() -> Result<KernelSqlApi, DeclarationError> {
    let api = KernelSqlApi::new(
        FunctionName::new("fn::kernel::identity::context_snapshot_matches")?,
        MigrationVersion::new(0),
        SqlSignature::new(
            vec![
                SqlParameter::new("tenant", SqlType::Record(TableName::new("tenant")?))?,
                SqlParameter::new("context", SqlType::Record(TableName::new("work_context")?))?,
                SqlParameter::new("digest", SqlType::String)?,
            ],
            SqlType::Bool,
        )?,
        SqlReadProfile::new(vec![TableName::new("work_context")?])?,
        include_str!("identity/migrations/context_snapshot_matches.surql"),
    )?;
    Ok(api)
}

fn create_service_principal_api() -> Result<KernelSqlApi, DeclarationError> {
    let api = KernelSqlApi::new(
        FunctionName::new("fn::kernel::identity::create_service_principal")?,
        MigrationVersion::new(0),
        SqlSignature::new(
            vec![
                SqlParameter::new("tenant", SqlType::Record(TableName::new("tenant")?))?,
                SqlParameter::new("principal", SqlType::Record(TableName::new("principal")?))?,
                SqlParameter::new("issuer", SqlType::String)?,
                SqlParameter::new("subject", SqlType::String)?,
                SqlParameter::new("name", SqlType::String)?,
            ],
            SqlType::Object,
        )?,
        SqlReadProfile::new(vec![TableName::new("principal")?])?,
        include_str!("identity/migrations/create_service_principal.surql"),
    )?;
    Ok(
        api.with_effects(SqlEffectProfile::OwnedCreate(SqlCreateProfile::new(
            TableName::new("principal")?,
            vec![
                SqlFieldName::new("tenant")?,
                SqlFieldName::new("kind")?,
                SqlFieldName::new("issuer")?,
                SqlFieldName::new("subject")?,
                SqlFieldName::new("display_name")?,
                SqlFieldName::new("claims_hash")?,
                SqlFieldName::new("enabled")?,
            ],
        )?)),
    )
}

fn tenant_context_keys_api() -> Result<KernelSqlApi, DeclarationError> {
    let api = KernelSqlApi::new(
        FunctionName::new("fn::kernel::identity::tenant_context_keys")?,
        MigrationVersion::new(0),
        SqlSignature::new(
            vec![
                SqlParameter::new("tenant", SqlType::Record(TableName::new("tenant")?))?,
                SqlParameter::new("context", SqlType::Record(TableName::new("work_context")?))?,
            ],
            SqlType::Option(Box::new(SqlType::Object)),
        )?,
        SqlReadProfile::new(vec![
            TableName::new("tenant")?,
            TableName::new("work_context")?,
        ])?,
        include_str!("identity/migrations/tenant_context_keys.surql"),
    )?;
    Ok(api)
}

/// Declare target ownership and dependencies without applying the mixed Store catalog.
pub fn module_setup(execution: LaneExecution) -> Result<ModuleSetup, DeclarationError> {
    ModuleSetup::builder(ModuleName::new("identity")?, ModuleLayer::Kernel)
        .ownership(vec![
            OwnershipClaim::Function(FunctionName::new(
                "fn::kernel::identity::oauth_client_exists",
            )?),
            OwnershipClaim::Function(FunctionName::new(
                "fn::kernel::identity::service_principal_available",
            )?),
            OwnershipClaim::Function(FunctionName::new(
                "fn::kernel::identity::context_snapshot_matches",
            )?),
            OwnershipClaim::Function(FunctionName::new(
                "fn::kernel::identity::create_service_principal",
            )?),
            OwnershipClaim::Function(FunctionName::new(
                "fn::kernel::identity::tenant_context_keys",
            )?),
            OwnershipClaim::Function(FunctionName::new("fn::kernel::identity::tenant_matches")?),
            OwnershipClaim::Function(FunctionName::new("fn::kernel::identity::tenant_current")?),
            OwnershipClaim::Function(FunctionName::new(
                "fn::kernel::identity::principal_current",
            )?),
            OwnershipClaim::Function(FunctionName::new(
                "fn::kernel::identity::principal_summaries",
            )?),
            OwnershipClaim::Function(FunctionName::new(
                "fn::kernel::identity::search_enabled_users",
            )?),
            OwnershipClaim::Function(FunctionName::new("fn::kernel::identity::enabled_user")?),
            OwnershipClaim::Function(FunctionName::new("fn::kernel::identity::identity_labels")?),
            OwnershipClaim::Function(FunctionName::new("fn::kernel::identity::identity_enabled")?),
            OwnershipClaim::Function(FunctionName::new("fn::kernel::identity::actor_admitted")?),
            OwnershipClaim::Table(TableName::new("tenant")?),
            OwnershipClaim::Table(TableName::new("enterprise")?),
            OwnershipClaim::Table(TableName::new("principal")?),
            OwnershipClaim::Table(TableName::new("principal_group")?),
            OwnershipClaim::Table(TableName::new("work_context")?),
            OwnershipClaim::Table(TableName::new("oauth_client")?),
        ])
        .lane(MigrationLane::new(vec![veoveo_modules::Migration::new(
            veoveo_modules::MigrationVersion::new(0),
            veoveo_modules::MigrationName::new("current")?,
            CURRENT_SCHEMA,
        )?])?)
        .sql_apis(vec![
            oauth_client_exists_api()?,
            service_principal_available_api()?,
            context_snapshot_matches_api()?,
            create_service_principal_api()?,
            tenant_context_keys_api()?,
            tenant_matches_api()?,
            tenant_current_api()?,
            principal_current_api()?,
            actor_admitted_api()?,
            identity_enabled_api()?,
            principal_summaries_api()?,
            search_enabled_users_api()?,
            enabled_user_api()?,
            identity_labels_api()?,
        ])
        .execution(execution)
        .requires(vec![LaneRequirement::Satisfied(ModuleName::new("store")?)])
        .build()
}
