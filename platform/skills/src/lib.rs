//! Skills a crate embeds at build time, as the Agent Skills format and the MCP Skills
//! extension (SEP-2640) describe them: each skill is a directory whose `SKILL.md` starts
//! with YAML frontmatter naming it, and every file of it is listed with its SHA-256 and
//! size. A build script turns a `skills/` directory into a table with
//! [`build::embed_skills`]; the crate includes that table and serves it unchanged, so every
//! host reads the same bytes, digests and frontmatter. Protocol-independent: the MCP
//! projection belongs to `veoveo-hub-sdk`.
#[cfg(feature = "build")]
pub mod build;

use veoveo_types::Sha256Digest;

/// The file that names a skill and starts with its frontmatter.
pub const SKILL_MD: &str = "SKILL.md";
/// The most files one skill holds.
pub const MAX_FILES: usize = 512;
/// The most bytes one skill's files hold together: 16 MiB.
pub const MAX_BYTES: u64 = 16 << 20;
/// The longest skill name.
pub const MAX_NAME: usize = 64;
/// The longest description.
pub const MAX_DESCRIPTION: usize = 1024;

/// One embedded skill: its name, which is its directory's, its frontmatter as JSON and its
/// files in path order, `SKILL.md` among them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmbeddedSkill {
    pub name: &'static str,
    /// The frontmatter of `SKILL.md`, every field the author wrote, as a JSON object.
    pub frontmatter: &'static str,
    pub files: &'static [EmbeddedFile],
}

/// One file of a skill: its path relative to the skill's directory, `/`-separated, its
/// bytes, their SHA-256 and their length.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmbeddedFile {
    pub path: &'static str,
    pub bytes: &'static [u8],
    pub digest: &'static str,
    pub size: u64,
}

impl EmbeddedFile {
    /// The file's SHA-256, `sha256:` and 64 lowercase hexadecimal digits.
    pub fn digest(&self) -> Sha256Digest {
        Sha256Digest::parse(self.digest).expect("an embedded file's digest is canonical")
    }
}

impl EmbeddedSkill {
    /// The skill's `SKILL.md`.
    pub fn skill_md(&self) -> &'static EmbeddedFile {
        self.files
            .iter()
            .find(|file| file.path == SKILL_MD)
            .expect("an embedded skill holds its SKILL.md")
    }

    /// The file at `path`, relative to the skill's directory.
    pub fn file(&self, path: &str) -> Option<&'static EmbeddedFile> {
        self.files.iter().find(|file| file.path == path)
    }
}

/// Whether `name` is a skill name: 1 to 64 of `a`–`z`, `0`–`9` and `-`, neither starting
/// nor ending with `-` and without `--`, as the Agent Skills format requires.
pub fn valid_skill_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !name.starts_with('-')
        && !name.ends_with('-')
        && !name.contains("--")
}

#[cfg(test)]
mod tests {
    use super::valid_skill_name;

    #[test]
    fn skill_names_follow_the_agent_skills_format() {
        for name in ["watch", "pdf-processing", "a", "v2-tools", &"a".repeat(64)] {
            assert!(valid_skill_name(name), "{name}");
        }
        for name in [
            "",
            "Watch",
            "-watch",
            "watch-",
            "wa--tch",
            "wa_tch",
            "wa tch",
            "ünï",
            &"a".repeat(65),
        ] {
            assert!(!valid_skill_name(name), "{name}");
        }
    }
}
