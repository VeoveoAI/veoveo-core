//! PostgreSQL names the store derives or admits: a registered server's schema and roles,
//! notification channels and singleton locks.
use std::{borrow::Cow, fmt};

use veoveo_types::ServerSlug;

use crate::StoreError;

/// Schemas the kernel owns. No server may use one as its schema.
pub const KERNEL_SCHEMAS: [&str; 5] = ["kernel", "identity", "gateway", "tasks", "audit"];

/// Schemas PostgreSQL creates in every database.
const POSTGRES_SCHEMAS: [&str; 2] = ["public", "information_schema"];

/// The longest server name, which leaves room for a login suffix within PostgreSQL's
/// 63-byte identifiers.
pub const MAX_SERVER_NAME: usize = 48;

/// The names a registered server owns: the schema named after its slug with each `-`
/// written as `_`, the group role `server_<schema>` that owns the schema, and the login
/// roles `<schema>_<suffix>` created for its registrations. `kernel.server_schema`
/// applies the same rules in SQL.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ServerSchema {
    server: ServerSlug,
    schema: String,
}

impl ServerSchema {
    pub fn new(server: &ServerSlug) -> Result<Self, StoreError> {
        check_server_name(server)?;
        let schema = server.as_str().replace('-', "_");
        if schema.starts_with("pg_")
            || KERNEL_SCHEMAS.contains(&schema.as_str())
            || POSTGRES_SCHEMAS.contains(&schema.as_str())
        {
            return Err(StoreError::invalid_name(
                "server name",
                server.as_str(),
                "the schema is reserved for the kernel or PostgreSQL",
            ));
        }
        Ok(Self {
            server: server.clone(),
            schema,
        })
    }

    pub fn server(&self) -> &ServerSlug {
        &self.server
    }

    pub fn schema(&self) -> &str {
        &self.schema
    }

    /// The group role that owns the schema.
    pub fn group_role(&self) -> String {
        format!("server_{}", self.schema)
    }

    /// The channel on which the schema's tables notify changes.
    pub fn channel(&self) -> Channel {
        Channel(Cow::Owned(self.schema.clone()))
    }

    /// The table that records the server's applied migrations.
    pub fn migrations_table(&self) -> String {
        format!("{}._sqlx_migrations", self.schema)
    }

    /// The login role `<schema>_<suffix>` for one registration.
    pub fn login(&self, suffix: &str) -> Result<LoginName, StoreError> {
        if suffix.is_empty()
            || !suffix
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        {
            return Err(StoreError::invalid_name(
                "login suffix",
                suffix,
                "lowercase letters, digits or underscores",
            ));
        }
        LoginName::parse(format!("{}_{suffix}", self.schema))
    }
}

/// Instances of kernel components and registered servers name their owner with a slug
/// that has the server name's shape; kernel components may use reserved names.
pub(crate) fn check_server_name(server: &ServerSlug) -> Result<(), StoreError> {
    let name = server.as_str();
    let mut bytes = name.bytes();
    let shaped = bytes.next().is_some_and(|b| b.is_ascii_lowercase())
        && bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && name.len() <= MAX_SERVER_NAME;
    if shaped {
        Ok(())
    } else {
        Err(StoreError::invalid_name(
            "server name",
            name,
            "a lowercase letter, then at most 47 lowercase letters, digits or hyphens",
        ))
    }
}

/// An unquoted PostgreSQL identifier: a lowercase letter or underscore, then up to 62
/// lowercase letters, digits or underscores.
fn check_identifier(kind: &'static str, value: &str) -> Result<(), StoreError> {
    let mut bytes = value.bytes();
    let admitted = bytes
        .next()
        .is_some_and(|b| b.is_ascii_lowercase() || b == b'_')
        && bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        && value.len() <= 63;
    if admitted {
        Ok(())
    } else {
        Err(StoreError::invalid_name(
            kind,
            value,
            "a lowercase letter or underscore, then up to 62 lowercase letters, digits or underscores",
        ))
    }
}

/// A login role name.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LoginName(String);

impl LoginName {
    pub fn parse(value: impl Into<String>) -> Result<Self, StoreError> {
        let value = value.into();
        check_identifier("login", &value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for LoginName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// The longest channel name, PostgreSQL's identifier limit.
const MAX_CHANNEL: usize = 63;

/// A notification channel. Each schema notifies on the channel named after it. A table
/// whose trigger names a qualifier column notifies on `<schema>.<qualifier>` instead,
/// such as `tasks.time` for the Tasks of server `time`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Channel(Cow<'static, str>);

impl Channel {
    pub const IDENTITY: Self = Self(Cow::Borrowed("identity"));
    pub const GATEWAY: Self = Self(Cow::Borrowed("gateway"));
    pub const TASKS: Self = Self(Cow::Borrowed("tasks"));
    pub const AUDIT: Self = Self(Cow::Borrowed("audit"));

    /// Admit a schema channel, an unquoted identifier, or a qualified channel whose
    /// qualifier holds lowercase letters, digits, hyphens or underscores. A channel is at
    /// most 63 bytes.
    pub fn parse(value: impl Into<Cow<'static, str>>) -> Result<Self, StoreError> {
        let value = value.into();
        let (schema, qualifier) = match value.split_once('.') {
            Some((schema, qualifier)) => (schema, Some(qualifier)),
            None => (value.as_ref(), None),
        };
        let invalid = |reason| Err(StoreError::invalid_name("channel", value.as_ref(), reason));
        if check_identifier("channel", schema).is_err() {
            return invalid(
                "a schema name: a lowercase letter or underscore, then lowercase letters, digits or underscores",
            );
        }
        if qualifier.is_some_and(|qualifier| {
            qualifier.is_empty()
                || !qualifier
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
        }) {
            return invalid("a qualifier of lowercase letters, digits, hyphens or underscores");
        }
        if value.len() > MAX_CHANNEL {
            return invalid("at most 63 bytes");
        }
        Ok(Self(value))
    }

    /// The channel `<schema>.<qualifier>` on which a trigger of this schema that names a
    /// qualifier column notifies.
    pub fn qualified(&self, qualifier: &str) -> Result<Self, StoreError> {
        if self.qualifier().is_some() {
            return Err(StoreError::invalid_name(
                "channel",
                format!("{}.{qualifier}", self.0),
                "a qualified channel takes no second qualifier",
            ));
        }
        Self::parse(format!("{}.{qualifier}", self.0))
    }

    /// The schema whose tables notify on this channel.
    pub fn schema(&self) -> &str {
        self.0.split_once('.').map_or(&self.0, |(schema, _)| schema)
    }

    /// The qualifier of a qualified channel.
    pub fn qualifier(&self) -> Option<&str> {
        self.0.split_once('.').map(|(_, qualifier)| qualifier)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Channel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// The name of a role exactly one replica performs, such as `audit.sealer`. It starts
/// with its owner's slug so that owners cannot contend for each other's locks.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SingletonName(Cow<'static, str>);

impl SingletonName {
    pub fn parse(value: impl Into<Cow<'static, str>>) -> Result<Self, StoreError> {
        let value = value.into();
        let admitted = !value.is_empty()
            && value.len() <= 128
            && value.bytes().all(|b| {
                b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'_' | b'.')
            });
        if !admitted {
            return Err(StoreError::invalid_name(
                "singleton",
                value,
                "1 to 128 lowercase letters, digits, hyphens, underscores or dots",
            ));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SingletonName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slug(value: &str) -> ServerSlug {
        ServerSlug::parse(value).unwrap()
    }

    #[test]
    fn server_names_map_to_schema_role_and_logins() {
        let schema = ServerSchema::new(&slug("time-zones")).unwrap();
        assert_eq!(schema.schema(), "time_zones");
        assert_eq!(schema.group_role(), "server_time_zones");
        assert_eq!(schema.channel().as_str(), "time_zones");
        assert_eq!(schema.migrations_table(), "time_zones._sqlx_migrations");
        assert_eq!(schema.login("0b6f").unwrap().as_str(), "time_zones_0b6f");
        assert!(schema.login("").is_err());
        assert!(schema.login("Upper").is_err());
        assert!(schema.login(&"a".repeat(60)).is_err());
    }

    #[test]
    fn reserved_and_misshapen_server_names_are_rejected() {
        for name in [
            "kernel",
            "identity",
            "gateway",
            "tasks",
            "audit",
            "public",
            "information-schema",
            "pg-catalog",
            "1time",
            "time_zones",
            "-time",
        ] {
            assert!(ServerSchema::new(&slug(name)).is_err(), "{name}");
        }
        assert!(ServerSchema::new(&slug(&"t".repeat(MAX_SERVER_NAME))).is_ok());
        assert!(ServerSchema::new(&slug(&"t".repeat(MAX_SERVER_NAME + 1))).is_err());
        assert!(check_server_name(&slug("gateway")).is_ok());
    }

    #[test]
    fn channels_and_singletons_are_admitted_once() {
        assert_eq!(Channel::parse("time_zones").unwrap().as_str(), "time_zones");
        assert!(Channel::parse("Time").is_err());
        assert!(Channel::parse("a-b").is_err());
        let tasks = Channel::TASKS.qualified("time-zones").unwrap();
        assert_eq!(tasks.as_str(), "tasks.time-zones");
        assert_eq!(
            (tasks.schema(), tasks.qualifier()),
            ("tasks", Some("time-zones"))
        );
        assert_eq!(Channel::parse("tasks.time-zones").unwrap(), tasks);
        assert_eq!(
            (Channel::TASKS.schema(), Channel::TASKS.qualifier()),
            ("tasks", None)
        );
        let longest = Channel::TASKS
            .qualified(&"t".repeat(MAX_SERVER_NAME))
            .unwrap();
        assert_eq!(longest.as_str().len(), 54);
        for rejected in [
            "tasks.",
            ".time",
            "tasks.Time",
            "tasks.time.zones",
            "tasks.time zones",
            "Tasks.time",
        ] {
            assert!(Channel::parse(rejected).is_err(), "{rejected}");
        }
        assert!(Channel::TASKS.qualified(&"t".repeat(58)).is_err());
        assert!(Channel::TASKS.qualified(&"t".repeat(57)).is_ok());
        assert!(tasks.qualified("again").is_err());
        assert!(Channel::TASKS.qualified("").is_err());
        assert!(SingletonName::parse("audit.sealer").is_ok());
        assert!(SingletonName::parse("").is_err());
        assert!(SingletonName::parse("Audit").is_err());
    }
}
