//! Markdown link extraction, path resolution, and heading fragments for the F3 viewer.

use std::path::{Path, PathBuf};

use pulldown_cmark::{Event, Parser, Tag, TagEnd};

use super::viewer_markdown::is_markdown_path;

/// One inline/reference link in document order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedLink {
    pub url: String,
    pub label: String,
}

/// Result of resolving a markdown link target against a base file path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedLink {
    External {
        url: String,
    },
    SameFile {
        fragment: String,
    },
    LocalFile {
        path: PathBuf,
        fragment: Option<String>,
    },
}

fn split_url_fragment(url: &str) -> (String, Option<String>) {
    let Some(hash) = url.find('#') else {
        return (url.to_string(), None);
    };
    let path_part = url[..hash].to_string();
    let frag = url[hash + 1..].to_string();
    if frag.is_empty() && path_part.is_empty() {
        return (String::new(), None);
    }
    if frag.is_empty() {
        return (path_part, None);
    }
    (path_part, Some(frag))
}

/// GitHub-style heading slug for fragment matching.
pub fn slugify_heading(text: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for c in text.trim().to_ascii_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
            prev_dash = false;
        } else if c.is_whitespace() || c == '-' || c == '_' {
            if !out.is_empty() && !prev_dash {
                out.push('-');
                prev_dash = true;
            }
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

fn push_unique_link(
    out: &mut Vec<ExtractedLink>,
    link: ExtractedLink,
) {
    if link.url.is_empty() {
        return;
    }
    if out
        .iter()
        .any(|e| e.url == link.url && e.label == link.label)
    {
        return;
    }
    out.push(link);
}

/// Scan for bare `http(s)://…` tokens not already captured by pulldown.
fn extract_bare_urls(
    markdown: &str,
    existing: &[ExtractedLink],
) -> Vec<ExtractedLink> {
    let mut out = Vec::new();
    let mut i = 0usize;
    let chars: Vec<char> = markdown.chars().collect();
    while i < chars.len() {
        let rest: String = chars[i..].iter().collect();
        if rest.starts_with("http://") || rest.starts_with("https://") {
            let mut end = i;
            while end < chars.len() {
                let c = chars[end];
                if c.is_whitespace() || c == ')' || c == '>' || c == '"' {
                    break;
                }
                end += 1;
            }
            let url: String = chars[i..end].iter().collect();
            if !existing.iter().any(|e: &ExtractedLink| e.url == url)
                && !out.iter().any(|e: &ExtractedLink| e.url == url)
            {
                out.push(ExtractedLink {
                    label: url.clone(),
                    url,
                });
            }
            i = end;
            continue;
        }
        i += 1;
    }
    out
}

/// Extract `[label](url)`, autolinks, and bare URLs from markdown (document order).
pub fn extract_links(markdown: &str) -> Vec<ExtractedLink> {
    let mut out = Vec::new();
    let mut in_link = false;
    let mut link_url = String::new();
    let mut link_label = String::new();

    for event in Parser::new(markdown) {
        match event {
            Event::Start(Tag::Link {
                link_type: _,
                dest_url,
                ..
            }) => {
                in_link = true;
                link_url = dest_url.to_string();
                link_label.clear();
            }
            Event::End(TagEnd::Link) => {
                if in_link {
                    let label = if link_label.is_empty() {
                        link_url.clone()
                    } else {
                        link_label.clone()
                    };
                    push_unique_link(
                        &mut out,
                        ExtractedLink {
                            url: link_url.clone(),
                            label,
                        },
                    );
                }
                in_link = false;
                link_url.clear();
                link_label.clear();
            }
            Event::Text(text) if in_link => {
                link_label.push_str(&text);
            }
            Event::Code(code) if in_link => {
                link_label.push_str(&code);
            }
            _ => {}
        }
    }

    let bare = extract_bare_urls(markdown, &out);
    out.extend(bare);
    out
}

pub fn resolve_markdown_link(
    base_file: &Path,
    url: &str,
) -> ResolvedLink {
    let trimmed = url.trim();
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return ResolvedLink::External {
            url: trimmed.to_string(),
        };
    }

    let (path_part, fragment) = split_url_fragment(trimmed);

    if path_part.is_empty() {
        return ResolvedLink::SameFile {
            fragment: fragment.unwrap_or_default(),
        };
    }

    let path = Path::new(&path_part);
    let resolved = if path.is_absolute() {
        path.to_path_buf()
    } else if let Some(parent) = base_file.parent() {
        parent.join(path)
    } else {
        path.to_path_buf()
    };

    let normalized = resolved.canonicalize().unwrap_or(resolved);

    ResolvedLink::LocalFile {
        path: normalized,
        fragment,
    }
}

/// Whether a resolved local path is openable as markdown in the viewer.
pub fn local_markdown_exists(path: &Path) -> bool {
    path.is_file() && is_markdown_path(&path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;

    #[test]
    fn extract_bare_https_urls() {
        let md = "See https://example.com/foo and [x](./a.md).";
        let links = extract_links(md);
        assert!(links.iter().any(|l| l.url == "https://example.com/foo"));
        assert!(links.iter().any(|l| l.url == "./a.md"));
    }

    #[test]
    fn extract_inline_links_in_order() {
        let md = "See [one](./a.md) and [two](../b.md#x).";
        let links = extract_links(md);
        assert_eq!(links.len(), 2);
        assert_eq!(links[0].label, "one");
        assert_eq!(links[0].url, "./a.md");
        assert_eq!(links[1].label, "two");
        assert_eq!(links[1].url, "../b.md#x");
    }

    #[test]
    fn slugify_heading_github_style() {
        assert_eq!(
            slugify_heading("Hello, World!"),
            "hello-world"
        );
        assert_eq!(slugify_heading("  Foo Bar  "), "foo-bar");
    }

    #[test]
    fn split_url_fragment_parts() {
        assert_eq!(
            split_url_fragment("doc.md#section"),
            ("doc.md".into(), Some("section".into()))
        );
        assert_eq!(
            split_url_fragment("#only"),
            (String::new(), Some("only".into()))
        );
        assert_eq!(
            split_url_fragment("plain.md"),
            ("plain.md".into(), None)
        );
    }

    #[test]
    fn resolve_relative_and_external() {
        let base = PathBuf::from("/tmp/docs/readme.md");
        match resolve_markdown_link(&base, "https://example.com/x") {
            ResolvedLink::External { url } => assert_eq!(url, "https://example.com/x"),
            _ => panic!("expected external"),
        }
        match resolve_markdown_link(&base, "#anchor") {
            ResolvedLink::SameFile { fragment } => assert_eq!(fragment, "anchor"),
            _ => panic!("expected same file"),
        }
        match resolve_markdown_link(&base, "other.md") {
            ResolvedLink::LocalFile { path, fragment } => {
                assert_eq!(path, PathBuf::from("/tmp/docs/other.md"));
                assert_eq!(fragment, None);
            }
            _ => panic!("expected local"),
        }
    }

    #[test]
    fn local_markdown_exists_checks_extension() {
        let dir = std::env::temp_dir().join(format!(
            "oxide_md_test_{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let md = dir.join("note.md");
        let mut f = fs::File::create(&md).unwrap();
        writeln!(f, "# hi").unwrap();
        assert!(local_markdown_exists(&md));
        assert!(!local_markdown_exists(&dir.join("x.txt")));
        let _ = fs::remove_dir_all(&dir);
    }
}
