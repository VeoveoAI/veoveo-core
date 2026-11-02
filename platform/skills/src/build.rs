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

use crate::{SkillsError, load};

/// The file a build writes under `OUT_DIR`, which the embedding crate includes.
pub const TABLE: &str = "veoveo_skills.rs";

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
        for (file, absolute) in skill.files.iter().zip(&skill.absolute) {
            let _ = writeln!(
                table,
                "            ::veoveo_skills::EmbeddedFile {{ path: {:?}, bytes: include_bytes!({:?}), digest: {:?}, size: {} }},",
                file.path,
                absolute,
                file.digest.to_string(),
                file.size
            );
        }
        table.push_str("        ],\n    },\n");
    }
    table.push(']');
    Ok(table)
}

/// A skill read from its directory: the loaded skill, its frontmatter as JSON text, and
/// each file's absolute path in the loaded skill's path order.
struct Skill {
    name: String,
    frontmatter: String,
    files: Vec<load::LoadedFile>,
    absolute: Vec<String>,
}

fn skill(dir: &Path, name: String) -> Result<Skill, SkillsError> {
    if !crate::valid_skill_name(&name) {
        return Err(SkillsError::Name(name));
    }
    let mut files = Vec::new();
    collect(dir, "", &mut files)?;
    let absolute = files
        .iter()
        .map(|(path, absolute, _)| (path.clone(), absolute.clone()))
        .collect::<std::collections::BTreeMap<_, _>>();
    let loaded = load::skill(
        &name,
        files.into_iter().map(|(path, _, bytes)| (path, bytes)),
    )?;
    let absolute = loaded
        .files
        .iter()
        .map(|file| absolute[&file.path].clone())
        .collect();
    Ok(Skill {
        name: loaded.name,
        frontmatter: serde_json::Value::Object(loaded.frontmatter).to_string(),
        files: loaded.files,
        absolute,
    })
}

/// Every file under `dir`, as its path relative to the skill's directory, its absolute
/// path and its bytes.
fn collect(
    dir: &Path,
    prefix: &str,
    files: &mut Vec<(String, String, Vec<u8>)>,
) -> Result<(), SkillsError> {
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
            files.push((path, absolute, bytes));
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
