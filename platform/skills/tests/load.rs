//! A skill from its files in memory: the frontmatter, digests, order and the refusals a
//! host applies to skills it takes from its configuration.
use veoveo_skills::{SkillsError, load::skill};

const WATCH: &str = "---\nname: watch\ndescription: Watch a served branch's builds.\nmetadata:\n  audience: agents\n---\n# Watch\n";

/// A skill's files: each path and its text.
type Files<'a> = Vec<(&'a str, &'a str)>;

fn files(entries: &[(&str, &str)]) -> Vec<(String, Vec<u8>)> {
    entries
        .iter()
        .map(|(path, text)| ((*path).to_owned(), text.as_bytes().to_vec()))
        .collect()
}

#[test]
fn a_skills_files_load_in_path_order_with_digests_and_frontmatter() {
    let loaded = skill(
        "watch",
        files(&[
            ("references/logs.md", "Logs.\n"),
            ("abc.txt", "abc"),
            ("SKILL.md", WATCH),
        ]),
    )
    .unwrap();
    assert_eq!(
        loaded
            .files
            .iter()
            .map(|file| (file.path.as_str(), file.size))
            .collect::<Vec<_>>(),
        [
            ("SKILL.md", WATCH.len() as u64),
            ("abc.txt", 3),
            ("references/logs.md", 6)
        ]
    );
    // FIPS 180-2's SHA-256 test vector for "abc".
    assert_eq!(
        loaded.files[1].digest.to_string(),
        "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(
        serde_json::Value::Object(loaded.frontmatter),
        serde_json::json!({
            "name": "watch",
            "description": "Watch a served branch's builds.",
            "metadata": {"audience": "agents"}
        })
    );
}

#[test]
fn a_skill_a_host_would_refuse_does_not_load() {
    let cases: Vec<(&str, &str, Files)> = vec![
        ("Name", "Watch", vec![("SKILL.md", WATCH)]),
        (
            "FilePath",
            "watch",
            vec![("SKILL.md", WATCH), ("/etc/passwd", "x")],
        ),
        (
            "FilePath",
            "watch",
            vec![("SKILL.md", WATCH), ("../other/SKILL.md", "x")],
        ),
        (
            "FilePath",
            "watch",
            vec![("SKILL.md", WATCH), ("a//b.md", "x")],
        ),
        (
            "FilePath",
            "watch",
            vec![("SKILL.md", WATCH), ("./b.md", "x")],
        ),
        ("FilePath", "watch", vec![("SKILL.md", WATCH), ("", "x")]),
        (
            "DuplicateFile",
            "watch",
            vec![("SKILL.md", WATCH), ("SKILL.md", WATCH)],
        ),
        ("MissingSkillMd", "watch", vec![("notes.md", "x")]),
        ("NoFrontmatter", "watch", vec![("SKILL.md", "# Watch\n")]),
        (
            "NameMismatch",
            "watch",
            vec![("SKILL.md", "---\nname: other\ndescription: d\n---\n")],
        ),
        (
            "Description",
            "watch",
            vec![("SKILL.md", "---\nname: watch\n---\n")],
        ),
    ];
    for (expected, name, entries) in cases {
        let error = skill(name, files(&entries)).unwrap_err();
        assert!(
            format!("{error:?}").starts_with(expected),
            "{expected}: {error}"
        );
    }
}

#[test]
fn a_loaded_skill_holds_at_most_16_mib() {
    let large = vec![b'x'; 16 << 20];
    let mut entries = files(&[("SKILL.md", WATCH)]);
    entries.push(("large.bin".to_owned(), large));
    assert!(matches!(
        skill("watch", entries),
        Err(SkillsError::TooLarge { .. })
    ));
}
