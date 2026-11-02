//! Why a skill cannot be loaded from configuration or embedded from a directory.

/// Why a skill cannot be loaded or embedded.
#[derive(Debug, thiserror::Error)]
pub enum SkillsError {
    #[error("reading {path}: {source}")]
    Read {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    #[error("{0} is not a skill directory: skills/ holds only skill directories")]
    NotADirectory(std::path::PathBuf),
    #[error("{0} is a symbolic link; a skill holds only regular files and directories")]
    Link(std::path::PathBuf),
    #[error(
        "{0:?} is not a skill name: 1 to 64 of a-z, 0-9 and -, without a leading, trailing or doubled -"
    )]
    Name(String),
    #[error("{0:?} is not a UTF-8 path")]
    Path(std::path::PathBuf),
    #[error(
        "{path:?} of skill {skill} is not a file path: relative and /-separated, without empty, . or .. segments"
    )]
    FilePath { skill: String, path: String },
    #[error("skill {skill} holds {path:?} twice")]
    DuplicateFile { skill: String, path: String },
    #[error("skill {0} has no SKILL.md")]
    MissingSkillMd(String),
    #[error("SKILL.md of skill {skill} has no frontmatter: it starts with a --- line and ends one")]
    NoFrontmatter { skill: String },
    #[error("the frontmatter of skill {skill} is not a YAML mapping of JSON values: {reason}")]
    Frontmatter { skill: String, reason: String },
    #[error("the frontmatter of skill {skill} names it {name:?}, not its directory's name")]
    NameMismatch { skill: String, name: String },
    #[error("the frontmatter of skill {skill} has no description of 1 to 1024 characters")]
    Description { skill: String },
    #[error("skill {skill} holds {files} files; a skill holds at most 512")]
    TooManyFiles { skill: String, files: usize },
    #[error("skill {skill} holds {bytes} bytes; a skill holds at most 16 MiB")]
    TooLarge { skill: String, bytes: u64 },
    #[error("environment variable {0} is not set; embed_skills runs in a build script")]
    Environment(&'static str),
}
