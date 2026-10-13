//! Installation documents that hold no server's internals. The component lock pins the
//! repository and Git revision of every component an installation runs.
mod component;
mod lock;

pub use component::{
    ComponentKind, ComponentName, ComponentNames, ComponentRepositories, ComponentRepository,
    GitRevision, GitRevisions, MAX_COMPONENT_NAME,
};
pub use lock::{COMPONENT_LOCK_FORMAT, ComponentLock, ComponentLockFormat, LockedComponent};

/// Why a component lock or one of its values is rejected. The diagnostic names the
/// rejected text, which an installation publishes.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ComponentLockError {
    #[error(
        "invalid component name {0:?}: veoveo-<kind>-<name> of at most 100 bytes, with kind service, mcp, runtime, app or showcase and a name of lowercase words joined by hyphens"
    )]
    Name(String),
    #[error("invalid component repository {0:?}: https://github.com/<owner>/<component name>")]
    Repository(String),
    #[error("invalid revision {0:?}: a full Git commit ID of 40 lowercase hexadecimal digits")]
    Revision(String),
    #[error("component {name} names repository {repository}, whose name differs")]
    RepositoryName {
        name: ComponentName,
        repository: ComponentRepository,
    },
    #[error("component {0} is locked more than once")]
    DuplicateComponent(ComponentName),
    #[error("a component lock names at least one component")]
    Empty,
}
