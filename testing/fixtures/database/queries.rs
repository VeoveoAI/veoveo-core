//! The queries a check covers: sqlx's offline query metadata, attributed to the `.sql`
//! files that `query_file!` compiled, and inline statements.
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    path::{Path, PathBuf},
};

use serde::Deserialize;
use sha2::{Digest, Sha256};

pub(super) struct Query {
    /// The query's `.sql` file relative to its workspace, or its label.
    pub subject: String,
    pub text: String,
}

/// One `query-<hash>.json` file of a sqlx offline directory.
#[derive(Deserialize)]
struct OfflineQuery {
    query: String,
    hash: String,
}

/// The queries of every offline directory, each attributed to the `.sql` file under the
/// directory's workspace whose SHA-256 is the query's hash, and the inline ones. With
/// `sources`, only queries whose file lies under one of them are kept.
pub(super) fn load(
    directories: &[PathBuf],
    sources: &[PathBuf],
    inline: &[(String, String)],
) -> Result<Vec<Query>, String> {
    let sources: Vec<PathBuf> = sources
        .iter()
        .map(|source| canonical(source))
        .collect::<Result<_, _>>()?;
    let mut queries = BTreeMap::new();
    for directory in directories {
        let directory = canonical(directory)?;
        let workspace = directory
            .parent()
            .ok_or_else(|| format!("{} has no parent workspace", directory.display()))?
            .to_owned();
        let mut files = HashMap::new();
        sql_files(&workspace, &mut files)?;
        files.values_mut().for_each(|paths| paths.sort());
        let entries = fs::read_dir(&directory)
            .map_err(|error| format!("reading {}: {error}", directory.display()))?;
        for entry in entries {
            let path = entry
                .map_err(|error| format!("reading {}: {error}", directory.display()))?
                .path();
            let named = path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("query-") && name.ends_with(".json"));
            if !named {
                continue;
            }
            let bytes =
                fs::read(&path).map_err(|error| format!("reading {}: {error}", path.display()))?;
            let offline: OfflineQuery = serde_json::from_slice(&bytes)
                .map_err(|error| format!("decoding {}: {error}", path.display()))?;
            let candidates = files
                .get(&offline.hash)
                .map(Vec::as_slice)
                .unwrap_or_default();
            let file = if sources.is_empty() {
                candidates.first()
            } else {
                let Some(file) = candidates
                    .iter()
                    .find(|file| sources.iter().any(|source| file.starts_with(source)))
                else {
                    continue;
                };
                Some(file)
            };
            let subject = match file {
                Some(file) => file
                    .strip_prefix(&workspace)
                    .unwrap_or(file)
                    .display()
                    .to_string(),
                None => format!(
                    "{} (query-{})",
                    first_line(&offline.query),
                    &offline.hash[..12.min(offline.hash.len())]
                ),
            };
            queries.insert(
                subject.clone(),
                Query {
                    subject,
                    text: offline.query,
                },
            );
        }
    }
    for (label, text) in inline {
        queries.insert(
            label.clone(),
            Query {
                subject: label.clone(),
                text: text.clone(),
            },
        );
    }
    Ok(queries.into_values().collect())
}

fn canonical(path: &Path) -> Result<PathBuf, String> {
    path.canonicalize()
        .map_err(|error| format!("{}: {error}", path.display()))
}

/// Every `.sql` file under `directory` by its SHA-256, skipping build output and hidden
/// directories. Files with the same contents share a hash.
fn sql_files(directory: &Path, files: &mut HashMap<String, Vec<PathBuf>>) -> Result<(), String> {
    let entries = fs::read_dir(directory)
        .map_err(|error| format!("reading {}: {error}", directory.display()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            if !name.starts_with('.') && name != "target" && name != "node_modules" {
                sql_files(&path, files)?;
            }
        } else if kind.is_file() && name.ends_with(".sql") {
            let bytes =
                fs::read(&path).map_err(|error| format!("reading {}: {error}", path.display()))?;
            files
                .entry(hex::encode(Sha256::digest(&bytes)))
                .or_default()
                .push(path);
        }
    }
    Ok(())
}

fn first_line(query: &str) -> String {
    let line = query
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with("--"))
        .unwrap_or_default();
    match line.char_indices().nth(60) {
        Some((end, _)) => format!("{}…", &line[..end]),
        None => line.to_owned(),
    }
}
