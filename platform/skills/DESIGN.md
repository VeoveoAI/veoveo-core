# Embedded Skills

`veoveo-skills` turns a crate's `skills/` directory into a table the crate embeds, so a
server serves its skills as bytes fixed at build time, with digests and frontmatter
every host reads the same way. It is protocol-independent: `veoveo-hub-sdk` projects the
table onto the MCP Skills extension (SEP-2640).

## Standards

| Standard | Use |
|---|---|
| [Agent Skills](https://agentskills.io/specification) | A skill is a directory named by the skill, holding `SKILL.md`, whose YAML frontmatter names the skill and describes it; names are 1 to 64 of `a`–`z`, `0`–`9` and `-`, without a leading, trailing or doubled `-`, and descriptions 1 to 1024 characters |
| MCP Skills extension (SEP-2640) | Every file of a skill is listed with `sha256:{hex}` and its size; at most 512 files and 16 MiB per skill |
| YAML 1.2, through `yaml_serde` | The frontmatter, read at build time into a JSON object with every field the author wrote |

## Build

A crate's build script calls `build::embed_skills("skills")` (feature `build`, used only
as a build-dependency). It reads every directory under `skills/` as one skill and
writes `{OUT_DIR}/veoveo_skills.rs`, a `&'static [EmbeddedSkill]` expression the crate
includes: skills in name order, files in path order, each file's bytes included by its
absolute path with its `Sha256Digest` and size, and each skill's frontmatter as JSON
text. It prints `cargo:rerun-if-changed` for the directory, which Cargo scans for any
change beneath it, so an added, removed or edited file rebuilds the table. A crate
without the directory embeds no skills.

The build refuses a file directly under `skills/`, a symbolic link, a path that is not
UTF-8, a skill name outside the format, a skill without `SKILL.md`, frontmatter that does
not start the file between `---` lines or is not a mapping of JSON values, a `name` that
is not the directory's, a missing or empty description, more than 512 files or more than
16 MiB. Names starting with `.` are left out. The build reports which skill and why, so
a server cannot ship a skill a host would refuse.

## Runtime

`EmbeddedSkill` holds the name, the frontmatter JSON and the files; `EmbeddedFile` the
path relative to the skill's directory, `/`-separated, the bytes, the digest and the size.
Nothing is read or hashed at run time.

## Tests

`tests/embed.rs` renders directories it writes: order, the frontmatter as JSON, a known
SHA-256 vector and size, the missing directory and each refusal, the 512-file limit
among them.
