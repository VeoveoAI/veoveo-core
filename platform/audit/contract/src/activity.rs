//! The outcome and reason vocabularies every audit record carries. What happened is
//! the record's owner-declared class and detail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, veoveo_types::Vocabulary)]
#[cfg_attr(feature = "postgres", vocabulary(postgres))]
pub enum AuditOutcome {
    Allowed,
    Denied,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, veoveo_types::Vocabulary)]
pub enum AuditReason {
    Accepted,
    PolicyDenied,
    TenantMismatch,
    InsufficientClearance,
    InsufficientAccess,
    Unauthenticated,
    InvalidCredential,
    ExpiredCredential,
    Replay,
    Revoked,
    InvalidRequest,
    NotFound,
    Conflict,
    Unavailable,
    QuotaExceeded,
    Cancelled,
    TimedOut,
    UpstreamFailure,
    InternalFailure,
    UnknownServer,
    UnknownResource,
    UnknownTask,
    UnknownPrincipal,
    UnknownScope,
    UnknownDataLabel,
    UnknownTenant,
    UnknownTokenIssuer,
    MissingPrincipal,
    MissingTenant,
    MissingGroup,
    MissingRole,
    MissingScope,
    MissingDataLabel,
    MissingPrincipalAssurance,
    TokenAudienceMismatch,
    TokenNotYetValid,
}

#[cfg(test)]
mod vocabulary_baseline;
