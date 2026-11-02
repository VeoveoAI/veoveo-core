//! A skills directory to the embedded table: names, frontmatter, digests, order and the
//! refusals, on directories each test writes under the system's temporary directory.
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

use veoveo_skills::{SkillsError, build::render};

/// A fresh directory for one test, removed when dropped.
struct Dir(PathBuf);

impl Dir {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "veoveo-skills-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn write(&self, relative: &str, text: &str) -> &Self {
        let path = self.0.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
        self
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

const WATCH: &str = "---\nname: watch\ndescription: Watch a served branch's builds.\nmetadata:\n  audience: agents\n---\n# Watch\n";

#[test]
fn a_skills_directory_becomes_a_table_in_name_and_path_order() {
    let dir = Dir::new();
    dir.write("watch/SKILL.md", WATCH)
        .write("watch/references/logs.md", "Logs.\n")
        .write("watch/.DS_Store", "ignored")
        .write(
            "audit/SKILL.md",
            "---\nname: audit\ndescription: Read the audit log.\n---\n",
        );
    let table = render(dir.path()).unwrap();
    // Skills by name, files by path, every file with its digest and size.
    let audit = table.find("name: \"audit\"").unwrap();
    let watch = table.find("name: \"watch\"").unwrap();
    assert!(audit < watch, "{table}");
    let skill_md = watch + table[watch..].find("path: \"SKILL.md\", bytes").unwrap();
    let references = table.find("path: \"references/logs.md\"").unwrap();
    assert!(skill_md < references, "{table}");
    assert!(!table.contains("DS_Store"), "{table}");
    // The frontmatter is JSON with every field the author wrote.
    let frontmatter = table
        .split("frontmatter: ")
        .nth(2)
        .unwrap()
        .split(",\n")
        .next()
        .unwrap();
    let json: String = serde_json::from_str(frontmatter).unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(
        value,
        serde_json::json!({
            "name": "watch",
            "description": "Watch a served branch's builds.",
            "metadata": {"audience": "agents"}
        })
    );
}

#[test]
fn digests_and_sizes_are_those_of_the_files_bytes() {
    let dir = Dir::new();
    dir.write("watch/SKILL.md", WATCH)
        .write("watch/abc.txt", "abc");
    let table = render(dir.path()).unwrap();
    // FIPS 180-2's SHA-256 test vector for "abc".
    assert!(
        table.contains(
            "digest: \"sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad\", size: 3 }"
        ),
        "{table}"
    );
    assert!(
        table.contains(&format!("size: {} }}", WATCH.len())),
        "{table}"
    );
}

#[test]
fn a_missing_directory_embeds_no_skills() {
    let dir = Dir::new();
    assert_eq!(render(&dir.path().join("skills")).unwrap(), "&[\n]");
}

#[test]
fn malformed_skills_are_refused() {
    let cases: Vec<(&str, Vec<(&str, &str)>)> = vec![
        ("NotADirectory", vec![("README.md", "x")]),
        ("Name", vec![("Watch/SKILL.md", WATCH)]),
        ("MissingSkillMd", vec![("watch/notes.md", "x")]),
        ("NoFrontmatter", vec![("watch/SKILL.md", "# Watch\n")]),
        (
            "NoFrontmatter",
            vec![("watch/SKILL.md", "---\nname: watch\n")],
        ),
        (
            "Frontmatter",
            vec![("watch/SKILL.md", "---\n- a\n- b\n---\n")],
        ),
        (
            "NameMismatch",
            vec![("watch/SKILL.md", "---\nname: other\ndescription: d\n---\n")],
        ),
        (
            "Description",
            vec![(
                "watch/SKILL.md",
                "---\nname: watch\ndescription: \"\"\n---\n",
            )],
        ),
    ];
    for (expected, files) in cases {
        let dir = Dir::new();
        for (path, text) in &files {
            dir.write(path, text);
        }
        let error = render(dir.path()).unwrap_err();
        let kind = format!("{error:?}");
        assert!(kind.starts_with(expected), "{expected}: {error}");
    }
}

#[test]
fn a_skill_holds_at_most_512_files() {
    let dir = Dir::new();
    dir.write("watch/SKILL.md", WATCH);
    for index in 0..512 {
        dir.write(&format!("watch/f/{index}.md"), "x");
    }
    assert!(matches!(
        render(dir.path()),
        Err(SkillsError::TooManyFiles { files: 513, .. })
    ));
}
