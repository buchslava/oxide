//! Find-file logic: glob matching, result types, grouped display rows, background directory search.

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;

use walkdir::WalkDir;

/// One result from Find file: path and optional line number (when content search matched).
#[derive(Debug, Clone)]
pub struct FindResult {
    pub path: PathBuf,
    pub line: Option<u64>,
}

/// Message from the find search background thread.
#[derive(Debug)]
pub enum FindMessage {
    Match(PathBuf, Option<u64>),
    /// Current directory being traversed (empty when search is done).
    CurrentDir(String),
    Done,
}

/// One row in the grouped find results list: either a folder (header) or a file.
#[derive(Debug, Clone)]
pub enum FindDisplayRow {
    Folder(PathBuf),
    File(FindResult),
}

/// Build display list grouped by parent directory: for each directory (in order of first occurrence),
/// one Folder row then one File row per match in that directory.
pub fn build_display_rows(results: &[FindResult]) -> Vec<FindDisplayRow> {
    let mut groups: Vec<(PathBuf, Vec<FindResult>)> = Vec::new();
    for r in results {
        let parent = r
            .path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(PathBuf::new);
        if groups.last().map(|(p, _)| p != &parent).unwrap_or(true) {
            groups.push((parent, Vec::new()));
        }
        if let Some((_, files)) = groups.last_mut() {
            files.push(r.clone());
        }
    }
    let mut rows = Vec::new();
    for (dir, files) in groups {
        rows.push(FindDisplayRow::Folder(dir));
        for r in files {
            rows.push(FindDisplayRow::File(r));
        }
    }
    rows
}

/// Shell-style glob match: * = any sequence, ? = one character. Empty pattern matches all.
pub fn glob_match(
    pattern: &str,
    name: &str,
    case_sensitive: bool,
) -> bool {
    let (p, n) = if case_sensitive {
        (pattern.as_bytes().to_vec(), name.as_bytes().to_vec())
    } else {
        (
            pattern.to_lowercase().into_bytes(),
            name.to_lowercase().into_bytes(),
        )
    };
    let (p, n) = (p.as_slice(), n.as_slice());
    fn match_at(
        p: &[u8],
        n: &[u8],
    ) -> bool {
        let mut pi = 0;
        let mut ni = 0;
        while pi < p.len() {
            match p[pi] {
                b'*' => {
                    pi += 1;
                    if pi == p.len() {
                        return true;
                    }
                    while ni <= n.len() {
                        if match_at(&p[pi..], &n[ni..]) {
                            return true;
                        }
                        ni += 1;
                    }
                    return false;
                }
                b'?' => {
                    if ni < n.len() {
                        pi += 1;
                        ni += 1;
                    } else {
                        return false;
                    }
                }
                c => {
                    if ni < n.len() && n[ni] == c {
                        pi += 1;
                        ni += 1;
                    } else {
                        return false;
                    }
                }
            }
        }
        ni == n.len()
    }
    if p.is_empty() {
        return true;
    }
    match_at(p, n)
}

/// Spawn a thread that walks `start_dir`, matches files with `glob_match`, optionally scans content.
/// Sends [`FindMessage`] values on `tx`. Stops when `cancel` is set.
pub fn run_find_search(
    start_dir: Arc<str>,
    file_pattern: Arc<str>,
    content_pattern: Arc<str>,
    recursive: bool,
    file_case_sens: bool,
    content_case_sens: bool,
    skip_hidden: bool,
    cancel: Arc<AtomicBool>,
    tx: mpsc::Sender<FindMessage>,
) {
    thread::spawn(move || {
        let start_dir = start_dir.trim();
        let file_pattern = file_pattern.trim();
        let content_pattern = content_pattern.trim();
        let start = PathBuf::from(start_dir);
        if !start.is_dir() {
            let _ = tx.send(FindMessage::Done);
            return;
        }
        let content_empty = content_pattern.is_empty();
        let mut current_dir_sent: Option<PathBuf> = None;

        let walker = WalkDir::new(&start)
            .min_depth(1)
            .max_depth(if recursive { usize::MAX } else { 1 })
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| {
                let name = e.file_name().to_string_lossy();
                if skip_hidden && name.starts_with('.') {
                    return false;
                }
                true
            })
            .filter_map(|e| e.ok());

        for entry in walker {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            let path = entry.path().to_path_buf();
            if let Some(parent) = path.parent() {
                if current_dir_sent.as_deref() != Some(parent) {
                    current_dir_sent = Some(parent.to_path_buf());
                    let _ = tx.send(FindMessage::CurrentDir(parent.display().to_string()));
                }
            }
            if !entry.file_type().is_file() {
                continue;
            }
            let name_cow = entry.file_name().to_string_lossy();
            let name = name_cow.as_ref();
            if !glob_match(file_pattern, name, file_case_sens) {
                continue;
            }
            if content_empty {
                let _ = tx.send(FindMessage::Match(path, None));
            } else if let Ok(file) = fs::File::open(&path) {
                let needle = if content_case_sens {
                    content_pattern.to_string()
                } else {
                    content_pattern.to_lowercase()
                };
                let reader = BufReader::new(file);
                for (line_no, line) in reader.lines().enumerate() {
                    if cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    if let Ok(ref ln) = line {
                        let hay = if content_case_sens {
                            ln.clone()
                        } else {
                            ln.to_lowercase()
                        };
                        if hay.contains(&needle) {
                            let _ = tx
                                .send(FindMessage::Match(path.clone(), Some((line_no + 1) as u64)));
                            break;
                        }
                    }
                }
            }
        }
        let _ = tx.send(FindMessage::Done);
    });
}
