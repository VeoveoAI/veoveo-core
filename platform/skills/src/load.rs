//! One skill from its files in memory, as a host loads it from its configuration and as
//! the build reads it from a directory: the same rules, digests and frontmatter for both.
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use veoveo_types::Sha256Digest;

use crate::{MAX_BYTES, MAX_DESCRIPTION, MAX_FILES, SKILL_MD, SkillsError, valid_skill_name};

/// A skill's name, its frontmatter as a JSON object with every field the author wrote,
/// and its files in path order, `SKILL.md` among them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedSkill {
    pub name: String,
    pub frontmatter: Map<String, Value>,
    pub files: Vec<LoadedFile>,
}

/// One file of a skill: its path relative to the skill's directory, its bytes, their
/// SHA-256 and their length.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedFile {
    pub path: String,
    pub bytes: Vec<u8>,
    pub digest: Sha256Digest,
    pub size: u64,
}

/// Whether `path` names a file within a skill's directory: relative, `/`-separated, with
/// no empty, `.` or `..` segment.
pub fn valid_file_path(path: &str) -> bool {
    !path.is_empty()
        && path
            .split('/')
            .all(|segment| !segment.is_empty() && segment != "." && segment != "..")
}

/// The skill `name` with `files`, each a path relative to the skill's directory and its
/// bytes. It refuses what a host would refuse: a name outside the Agent Skills format, a
/// file path outside the skill or named twice, more than 512 files or 16 MiB, a missing
/// `SKILL.md`, or frontmatter that is not a YAML mapping between `---` lines naming the
/// skill and describing it.
pub fn skill(
    name: &str,
    files: impl IntoIterator<Item = (String, Vec<u8>)>,
) -> Result<LoadedSkill, SkillsError> {
    if !valid_skill_name(name) {
        return Err(SkillsError::Name(name.to_owned()));
    }
    let mut loaded = Vec::new();
    for (path, bytes) in files {
        if !valid_file_path(&path) {
            return Err(SkillsError::FilePath {
                skill: name.to_owned(),
                path,
            });
        }
        loaded.push(LoadedFile {
            digest: Sha256Digest::from_bytes(Sha256::digest(&bytes).into()),
            size: bytes.len() as u64,
            path,
            bytes,
        });
    }
    loaded.sort_by(|a, b| a.path.cmp(&b.path));
    if let Some(pair) = loaded.windows(2).find(|pair| pair[0].path == pair[1].path) {
        return Err(SkillsError::DuplicateFile {
            skill: name.to_owned(),
            path: pair[0].path.clone(),
        });
    }
    if loaded.len() > MAX_FILES {
        return Err(SkillsError::TooManyFiles {
            skill: name.to_owned(),
            files: loaded.len(),
        });
    }
    let bytes = loaded.iter().map(|file| file.size).sum::<u64>();
    if bytes > MAX_BYTES {
        return Err(SkillsError::TooLarge {
            skill: name.to_owned(),
            bytes,
        });
    }
    let skill_md = loaded
        .iter()
        .find(|file| file.path == SKILL_MD)
        .ok_or_else(|| SkillsError::MissingSkillMd(name.to_owned()))?;
    let frontmatter = frontmatter(name, &skill_md.bytes)?;
    Ok(LoadedSkill {
        name: name.to_owned(),
        frontmatter,
        files: loaded,
    })
}

/// The frontmatter of `SKILL.md` as a JSON object: the YAML between a leading `---` line
/// and the next `---` line, with a `name` equal to the skill's and a description.
fn frontmatter(skill: &str, skill_md: &[u8]) -> Result<Map<String, Value>, SkillsError> {
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
    let value: Value = yaml_serde::from_str(&yaml).map_err(|error| SkillsError::Frontmatter {
        skill: skill.to_owned(),
        reason: error.to_string(),
    })?;
    let Value::Object(fields) = value else {
        return Err(SkillsError::Frontmatter {
            skill: skill.to_owned(),
            reason: "it is not a mapping".to_owned(),
        });
    };
    match fields.get("name").and_then(Value::as_str) {
        Some(name) if name == skill => {}
        other => {
            return Err(SkillsError::NameMismatch {
                skill: skill.to_owned(),
                name: other.unwrap_or_default().to_owned(),
            });
        }
    }
    match fields.get("description").and_then(Value::as_str) {
        Some(description)
            if !description.trim().is_empty() && description.chars().count() <= MAX_DESCRIPTION => {
        }
        _ => {
            return Err(SkillsError::Description {
                skill: skill.to_owned(),
            });
        }
    }
    Ok(fields)
}
