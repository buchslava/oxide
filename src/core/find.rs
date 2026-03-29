//! Find-file logic: glob matching, optional regex matching, result types, grouped display rows, background directory search.

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;

use regex::{Regex, RegexBuilder};
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

/// Prepared file-name filter for many files (Find thread, panel mark/unmark). Empty pattern matches all.
///
/// In **wildcard** mode, `|` separates alternative glob patterns (e.g. `a*|b?` matches if either
/// part matches). In **regex** mode, `|` is part of the regex (alternation), not a splitter.
#[derive(Debug)]
pub enum PreparedFilePattern {
    /// Match every non-skipped file name.
    All,
    /// Invalid regex (non-empty pattern in regex mode): match nothing.
    None,
    Wildcard {
        /// One or more shell-style globs; a name matches if any pattern matches (`|` alternation).
        patterns: Vec<String>,
        case_sensitive: bool,
    },
    Regex(Regex),
}

impl PreparedFilePattern {
    /// `regex_mode`: from F9 Settings (`file_pattern_mode`); when false, use `glob_match` semantics.
    pub fn new(
        raw: &str,
        case_sensitive: bool,
        regex_mode: bool,
    ) -> Self {
        let pat = raw.trim();
        if regex_mode {
            if pat.is_empty() {
                return Self::All;
            }
            let mut b = RegexBuilder::new(pat);
            if !case_sensitive {
                b.case_insensitive(true);
            }
            return match b.build() {
                Ok(re) => Self::Regex(re),
                Err(_) => Self::None,
            };
        }
        if pat.is_empty() {
            return Self::All;
        }
        let patterns = if pat.contains('|') {
            pat.split('|')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect::<Vec<_>>()
        } else {
            vec![pat.to_string()]
        };
        if patterns.is_empty() {
            return Self::None;
        }
        Self::Wildcard {
            patterns,
            case_sensitive,
        }
    }

    /// Match against a single path component (file base name), without trailing `/`.
    pub fn matches(
        &self,
        name: &str,
    ) -> bool {
        let name = name.trim_end_matches('/');
        match self {
            Self::All => true,
            Self::None => false,
            Self::Wildcard {
                patterns,
                case_sensitive,
            } => patterns
                .iter()
                .any(|p| glob_match(p, name, *case_sensitive)),
            Self::Regex(re) => re.is_match(name),
        }
    }
}

/// Path relative to the search root (forward slashes), for matching **Ignore pattern** against
/// directory segments (e.g. `*node_modules*` excludes zips under any `node_modules`).
fn relative_path_for_ignore(
    start_canon: &Path,
    file_path: &Path,
) -> String {
    let path_can = fs::canonicalize(file_path).unwrap_or_else(|_| file_path.to_path_buf());
    path_can
        .strip_prefix(start_canon)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| path_can.to_string_lossy().replace('\\', "/"))
}

/// Spawn a thread that walks `start_dir`, matches file names with wildcards or regex (see F9 Settings;
/// wildcard mode: `|` = alternative globs),
/// optionally scans content. Sends [`FindMessage`] values on `tx`. Stops when `cancel` is set.
///
/// **Ignore pattern** (same wildcard/regex rules as file pattern): when non-empty, tested against the
/// file path relative to the search root; if it matches, the file is skipped (not listed).
pub fn run_find_search(
    start_dir: Arc<str>,
    file_pattern: Arc<str>,
    ignore_pattern: Arc<str>,
    content_pattern: Arc<str>,
    recursive: bool,
    file_case_sens: bool,
    file_pattern_regex: bool,
    content_case_sens: bool,
    skip_hidden: bool,
    cancel: Arc<AtomicBool>,
    tx: mpsc::Sender<FindMessage>,
) {
    thread::spawn(move || {
        let start_dir = start_dir.trim();
        let file_pattern_trim = file_pattern.trim();
        let ignore_pattern_trim = ignore_pattern.trim();
        let content_pattern = content_pattern.trim();
        let name_matcher =
            PreparedFilePattern::new(file_pattern_trim, file_case_sens, file_pattern_regex);
        let ignore_matcher = if ignore_pattern_trim.is_empty() {
            None
        } else {
            Some(PreparedFilePattern::new(
                ignore_pattern_trim,
                file_case_sens,
                file_pattern_regex,
            ))
        };
        let start = PathBuf::from(start_dir);
        if !start.is_dir() {
            let _ = tx.send(FindMessage::Done);
            return;
        }
        let start_canon = fs::canonicalize(&start).unwrap_or_else(|_| start.clone());
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
            if !name_matcher.matches(name) {
                continue;
            }
            if let Some(ref ign) = ignore_matcher {
                let rel = relative_path_for_ignore(&start_canon, &path);
                if ign.matches(&rel) {
                    continue;
                }
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

#[cfg(test)]
mod prepared_pattern_tests {
    use super::PreparedFilePattern;

    #[test]
    fn wildcard_pipe_matches_any_segment() {
        let p = PreparedFilePattern::new("Screenshot*|d*zip|file*", true, false);
        assert!(p.matches("Screenshot12.png"));
        assert!(p.matches("daily.zip"));
        assert!(p.matches("file.txt"));
        assert!(!p.matches("other.txt"));
    }

    #[test]
    fn wildcard_pipe_trims_segments() {
        let p = PreparedFilePattern::new(" a* | b* ", true, false);
        assert!(p.matches("ax"));
        assert!(p.matches("by"));
    }

    #[test]
    fn wildcard_only_pipes_or_empty_segments_match_nothing() {
        let p = PreparedFilePattern::new("||", true, false);
        assert!(!p.matches("x"));
    }

    #[test]
    fn regex_mode_pipe_is_not_split() {
        let p = PreparedFilePattern::new("a|b", true, true);
        assert!(p.matches("a"));
        assert!(p.matches("b"));
        assert!(!p.matches("c"));
    }

    #[test]
    fn single_wildcard_without_pipe_unchanged() {
        let p = PreparedFilePattern::new("*.txt", true, false);
        assert!(p.matches("foo.txt"));
        assert!(!p.matches("foo.zip"));
    }

    #[test]
    fn ignore_pattern_matches_relative_path_string() {
        let p = PreparedFilePattern::new("*node_modules*", true, false);
        assert!(p.matches("foo/node_modules/bar/file.zip"));
        assert!(!p.matches("foo/other/file.zip"));
    }
}
