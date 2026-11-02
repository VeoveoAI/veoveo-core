//! The build-script side: a `skills/` directory to the table a crate embeds.
//!
//! A crate's `build.rs` calls `embed_skills("skills")` and the crate includes
//! `concat!(env!("OUT_DIR"), "/veoveo_skills.rs")`:
//!
//! ```no_run
//! veoveo_skills::build::embed_skills("skills").expect("the skills directory");
//! ```
use std::{
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};
use veoveo_types::Sha256Digest;

use crate::{MAX_BYTES, MAX_DESCRIPTION, MAX_FILES, SKILL_MD, valid_skill_name};

/// The file a build writes under `OUT_DIR`, which the embedding crate includes.
pub const TABLE: &str = "veoveo_skills.rs";

/// Why a skills directory cannot be embedded.
#[derive(Debug, thiserror::Error)]
pub enum SkillsError {
    #[error("reading {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{0} is not a skill directory: skills/ holds only skill directories")]
    NotADirectory(PathBuf),
    #[error("{0} is a symbolic link; a skill holds only regular files and directories")]
    Link(PathBuf),
    #[error(
        "{0:?} is not a skill name: 1 to 64 of a-z, 0-9 and -, without a leading, trailing or doubled -"
    )]
    Name(String),
    #[error("{0:?} is not a UTF-8 path")]
    Path(PathBuf),
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

/// Embed the skills under `dir`, relative to the crate's manifest directory, as the table
/// `{OUT_DIR}/veoveo_skills.rs`, and have Cargo run the build again whenever anything
/// under `dir` changes. A crate without the directory embeds no skills. Files and
/// directories whose names start with `.` are left out.
pub fn embed_skills(dir: impl AsRef<Path>) -> Result<(), SkillsError> {
    let manifest = std::env::var_os("CARGO_MANIFEST_DIR")
        .ok_or(SkillsError::Environment("CARGO_MANIFEST_DIR"))?;
    let out = std::env::var_os("OUT_DIR").ok_or(SkillsError::Environment("OUT_DIR"))?;
    let dir = Path::new(&manifest).join(dir);
    // Cargo scans a directory it is told to watch for any change beneath it.
    println!("cargo:rerun-if-changed={}", dir.display());
    let table = render(&dir)?;
    fs::write(Path::new(&out).join(TABLE), table).map_err(|source| SkillsError::Read {
        path: Path::new(&out).join(TABLE),
        source,
    })
}

/// The table of the skills under `dir`: a Rust expression of type
/// `&'static [veoveo_skills::EmbeddedSkill]`, skills in name order and files in path
/// order, each file's bytes included by its absolute path.
pub fn render(dir: &Path) -> Result<String, SkillsError> {
    let mut skills = Vec::new();
    if dir.exists() {
        for entry in entries(dir)? {
            let name = utf8_name(&entry)?;
            if !metadata(&entry)?.is_dir() {
                return Err(SkillsError::NotADirectory(entry));
            }
            skills.push(skill(&entry, name)?);
        }
    }
    skills.sort_by(|a, b| a.name.cmp(&b.name));
    let mut table = String::from("&[\n");
    for skill in &skills {
        let _ = writeln!(
            table,
            "    ::veoveo_skills::EmbeddedSkill {{\n        name: {:?},\n        frontmatter: {:?},\n        files: &[",
            skill.name, skill.frontmatter
        );
        for file in &skill.files {
            let _ = writeln!(
                table,
                "            ::veoveo_skills::EmbeddedFile {{ path: {:?}, bytes: include_bytes!({:?}), digest: {:?}, size: {} }},",
                file.path, file.absolute, file.digest, file.size
            );
        }
        table.push_str("        ],\n    },\n");
    }
    table.push(']');
    Ok(table)
}

struct Skill {
    name: String,
    frontmatter: String,
    files: Vec<File>,
}

struct File {
    path: String,
    absolute: String,
    digest: String,
    size: u64,
}

fn skill(dir: &Path, name: String) -> Result<Skill, SkillsError> {
    if !valid_skill_name(&name) {
        return Err(SkillsError::Name(name));
    }
    let mut files = Vec::new();
    collect(dir, "", &mut files)?;
    files.sort_by(|a, b| a.path.cmp(&b.path));
    if files.len() > MAX_FILES {
        return Err(SkillsError::TooManyFiles {
            skill: name,
            files: files.len(),
        });
    }
    let bytes = files.iter().map(|file| file.size).sum::<u64>();
    if bytes > MAX_BYTES {
        return Err(SkillsError::TooLarge { skill: name, bytes });
    }
    if !files.iter().any(|file| file.path == SKILL_MD) {
        return Err(SkillsError::MissingSkillMd(name));
    }
    let skill_md = read(&dir.join(SKILL_MD))?;
    let frontmatter = frontmatter(&name, &skill_md)?;
    Ok(Skill {
        name,
        frontmatter,
        files,
    })
}

/// The frontmatter of `SKILL.md` as a JSON object: the YAML between a leading `---` line
/// and the next `---` line, with a `name` equal to the skill's and a description.
fn frontmatter(skill: &str, skill_md: &[u8]) -> Result<String, SkillsError> {
    let no_frontmatter = || SkillsError::NoFrontmatter {
        skill: skill.to_owned(),
    };
    let text = std::str::from_utf8(skill_md).map_err(|_| no_frontmatter())?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut lines = text.split_inclusive('\n');
    if lines.next().map(str::trim_end) != Some("---") {
        return Err(no_frontmatter());
    }
    let mut yaml = String::new();
    let mut closed = false;
    for line in lines {
        if line.trim_end() == "---" {
            closed = true;
            break;
        }
        yaml.push_str(line);
    }
    if !closed {
        return Err(no_frontmatter());
    }
    let value: serde_json::Value =
        yaml_serde::from_str(&yaml).map_err(|error| SkillsError::Frontmatter {
            skill: skill.to_owned(),
            reason: error.to_string(),
        })?;
    let serde_json::Value::Object(fields) = value else {
        return Err(SkillsError::Frontmatter {
            skill: skill.to_owned(),
            reason: "it is not a mapping".to_owned(),
        });
    };
    match fields.get("name").and_then(serde_json::Value::as_str) {
        Some(name) if name == skill => {}
        other => {
            return Err(SkillsError::NameMismatch {
                skill: skill.to_owned(),
                name: other.unwrap_or_default().to_owned(),
            });
        }
    }
    match fields
        .get("description")
        .and_then(serde_json::Value::as_str)
    {
        Some(description)
            if !description.trim().is_empty() && description.chars().count() <= MAX_DESCRIPTION => {
        }
        _ => {
            return Err(SkillsError::Description {
                skill: skill.to_owned(),
            });
        }
    }
    Ok(serde_json::Value::Object(fields).to_string())
}

fn collect(dir: &Path, prefix: &str, files: &mut Vec<File>) -> Result<(), SkillsError> {
    for entry in entries(dir)? {
        let name = utf8_name(&entry)?;
        let path = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        let metadata = metadata(&entry)?;
        if metadata.is_dir() {
            collect(&entry, &path, files)?;
        } else {
            let bytes = read(&entry)?;
            let absolute = entry
                .to_str()
                .ok_or_else(|| SkillsError::Path(entry.clone()))?
                .to_owned();
            files.push(File {
                path,
                absolute,
                digest: Sha256Digest::from_bytes(Sha256::digest(&bytes).into()).to_string(),
                size: bytes.len() as u64,
            });
        }
    }
    Ok(())
}

/// The entries of `dir` whose names do not start with `.`.
fn entries(dir: &Path) -> Result<Vec<PathBuf>, SkillsError> {
    let read = fs::read_dir(dir).map_err(|source| SkillsError::Read {
        path: dir.to_owned(),
        source,
    })?;
    let mut entries = Vec::new();
    for entry in read {
        let entry = entry.map_err(|source| SkillsError::Read {
            path: dir.to_owned(),
            source,
        })?;
        if !entry.file_name().to_string_lossy().starts_with('.') {
            entries.push(entry.path());
        }
    }
    Ok(entries)
}

/// The entry's metadata, refusing a symbolic link.
fn metadata(path: &Path) -> Result<fs::Metadata, SkillsError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| SkillsError::Read {
        path: path.to_owned(),
        source,
    })?;
    if metadata.file_type().is_symlink() {
        return Err(SkillsError::Link(path.to_owned()));
    }
    Ok(metadata)
}

fn utf8_name(path: &Path) -> Result<String, SkillsError> {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(str::to_owned)
        .ok_or_else(|| SkillsError::Path(path.to_owned()))
}

fn read(path: &Path) -> Result<Vec<u8>, SkillsError> {
    fs::read(path).map_err(|source| SkillsError::Read {
        path: path.to_owned(),
        source,
    })
}
