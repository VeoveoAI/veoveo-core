//! The names, repositories and revisions an installation pins.
use veoveo_types::{HexCase, HexGrammar, IdFailure, IdMetadata, IdProfile, IdProfileSpec};

use crate::ComponentLockError;

/// The longest component name, GitHub's limit for a repository name.
pub const MAX_COMPONENT_NAME: usize = 100;

/// A component's repository name: `veoveo-` followed by lowercase words of
/// letters and digits joined by hyphens, starting with a letter.
#[veoveo_types::id(text(ComponentNames))]
pub struct ComponentName(String);

/// The GitHub repository that holds a component's source:
/// `https://github.com/<owner>/<component name>`.
#[veoveo_types::id(text(ComponentRepositories))]
pub struct ComponentRepository(String);

impl ComponentRepository {
    /// The GitHub account that owns the repository, such as `VeoveoAI`.
    pub fn owner(&self) -> &str {
        repository_path(self.as_str())
            .expect("an admitted repository URL names an owner")
            .0
    }

    /// The repository's name, which names the component it holds.
    pub fn name(&self) -> ComponentName {
        ComponentName::parse(
            repository_path(self.as_str())
                .expect("an admitted repository URL names a repository")
                .1,
        )
        .expect("an admitted repository URL names a component")
    }
}

/// A full Git commit ID: 40 lowercase hexadecimal digits of a SHA-1 object name, never
/// the all-zero ID Git uses for no object.
#[veoveo_types::id(hex(GitRevisions))]
pub struct GitRevision(String);

const GITHUB: &str = "https://github.com/";

const NAME_PATTERN: &str = "veoveo-[a-z][a-z0-9]*(-[a-z0-9]+)*";

fn component_name(value: &str) -> Option<()> {
    if value.len() > MAX_COMPONENT_NAME {
        return None;
    }
    let name = value.strip_prefix("veoveo-")?;
    let mut words = name.split('-');
    let first = words.next()?;
    let lowercase_word = |word: &str| {
        !word.is_empty()
            && word
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
    };
    let starts_with_letter = first.bytes().next().is_some_and(|b| b.is_ascii_lowercase());
    (starts_with_letter && lowercase_word(first) && words.all(lowercase_word)).then_some(())
}

/// The owner and repository segments of a GitHub repository URL.
fn repository_path(value: &str) -> Option<(&str, &str)> {
    let (owner, repository) = value.strip_prefix(GITHUB)?.split_once('/')?;
    // GitHub accounts are 1 to 39 letters, digits and single inner hyphens.
    let owner_admitted = (1..=39).contains(&owner.len())
        && owner
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        && !owner.starts_with('-')
        && !owner.ends_with('-')
        && !owner.contains("--");
    (owner_admitted && component_name(repository).is_some()).then_some((owner, repository))
}

#[doc(hidden)]
pub struct ComponentNames;
impl IdProfile for ComponentNames {
    type Error = ComponentLockError;
    const PROFILE: IdProfileSpec<Self::Error> = IdProfileSpec::text(|value, _| {
        component_name(value)
            .map(|_| ())
            .ok_or_else(|| ComponentLockError::Name(value.to_owned()))
    })
    .owner_schema(name_schema, false);
}

fn name_schema(_: &mut schemars::SchemaGenerator, _: IdMetadata) -> schemars::Schema {
    schemars::json_schema!({
        "type": "string",
        "pattern": format!("^{NAME_PATTERN}$"),
        "maxLength": MAX_COMPONENT_NAME,
    })
}

#[doc(hidden)]
pub struct ComponentRepositories;
impl IdProfile for ComponentRepositories {
    type Error = ComponentLockError;
    const PROFILE: IdProfileSpec<Self::Error> = IdProfileSpec::text(|value, _| {
        repository_path(value)
            .map(|_| ())
            .ok_or_else(|| ComponentLockError::Repository(value.to_owned()))
    })
    .owner_schema(repository_schema, false);
}

fn repository_schema(_: &mut schemars::SchemaGenerator, _: IdMetadata) -> schemars::Schema {
    schemars::json_schema!({
        "type": "string",
        "format": "uri",
        "pattern": format!(
            "^https://github\\.com/[A-Za-z0-9](?:[A-Za-z0-9-]{{0,37}}[A-Za-z0-9])?/{NAME_PATTERN}$"
        ),
    })
}

#[doc(hidden)]
pub struct GitRevisions;
impl IdProfile for GitRevisions {
    type Error = ComponentLockError;
    const PROFILE: IdProfileSpec<Self::Error> = IdProfileSpec::hex(
        HexGrammar {
            length: 40,
            case: HexCase::Lower,
            nonzero: true,
        },
        |value, _, _: IdFailure| ComponentLockError::Revision(value.to_owned()),
    )
    .owner_schema(revision_schema, false);
}

fn revision_schema(_: &mut schemars::SchemaGenerator, _: IdMetadata) -> schemars::Schema {
    schemars::json_schema!({
        "type": "string",
        "pattern": "^[0-9a-f]{40}$",
        "not": { "const": "0".repeat(40) },
    })
}
