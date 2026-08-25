//! Panel location: either a filesystem path or a path inside a supported archive (ZIP, tar.gz).
//! Used to make panel logic generic over "disk" and "archive" backends.

use std::path::{Path, PathBuf};

/// Supported archive container on disk.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ArchiveFormat {
    Zip,
    TarGz,
}

/// Current panel location: filesystem directory or virtual directory inside an archive.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum PanelLocation {
    /// A directory on disk.
    Fs(PathBuf),
    /// Path inside an archive. `path_inside` uses forward slashes, no leading slash.
    Archive {
        format: ArchiveFormat,
        archive: PathBuf,
        path_inside: String,
    },
}

/// Detect archive format from a file name (e.g. path's file name).
pub fn archive_format_for_path(path: &Path) -> Option<ArchiveFormat> {
    path.file_name()
        .and_then(|s| s.to_str())
        .and_then(archive_format_for_filename)
}

/// Whether `name` refers to an archive file we can open as a panel (e.g. `.zip`, `.tar.gz`).
pub fn archive_format_for_filename(name: &str) -> Option<ArchiveFormat> {
    if ascii_suffix_eq(name, ".zip") {
        Some(ArchiveFormat::Zip)
    } else if ascii_suffix_eq(name, ".tar.gz") || ascii_suffix_eq(name, ".tgz") {
        Some(ArchiveFormat::TarGz)
    } else {
        None
    }
}

fn ascii_suffix_eq(
    name: &str,
    suffix: &str,
) -> bool {
    let n = name.len();
    let s = suffix.len();
    n >= s && name.is_char_boundary(n - s) && name[n - s..].eq_ignore_ascii_case(suffix)
}

impl PanelLocation {
    pub fn fs<P: AsRef<Path>>(path: P) -> Self {
        PanelLocation::Fs(path.as_ref().to_path_buf())
    }

    /// Display string for the panel header (e.g. "/home/user" or "/path/to/arch.zip/subdir").
    pub fn display_string(&self) -> String {
        match self {
            PanelLocation::Fs(p) => p.to_string_lossy().to_string(),
            PanelLocation::Archive {
                archive,
                path_inside,
                ..
            } => {
                let archive_display = archive.to_string_lossy();
                if path_inside.is_empty() {
                    archive_display.to_string()
                } else {
                    format!(
                        "{}/{}",
                        archive_display,
                        path_inside.trim_end_matches('/')
                    )
                }
            }
        }
    }

    /// True if this is a filesystem location (used for process cwd sync when autosave is off, subshell).
    pub fn is_fs(&self) -> bool {
        matches!(self, PanelLocation::Fs(_))
    }

    /// Parent location: for Fs the parent path; for Archive either parent dir inside archive or Fs(archive dir).
    pub fn parent(&self) -> Option<PanelLocation> {
        match self {
            PanelLocation::Fs(p) => p.parent().map(PanelLocation::fs),
            PanelLocation::Archive {
                format,
                archive,
                path_inside,
            } => {
                let trimmed = path_inside.trim_end_matches('/');
                if trimmed.is_empty() {
                    archive.parent().map(PanelLocation::fs)
                } else {
                    let parent_inside = trimmed
                        .rsplit_once('/')
                        .map(|(p, _)| p.to_string())
                        .unwrap_or_default();
                    Some(PanelLocation::Archive {
                        format: *format,
                        archive: archive.clone(),
                        path_inside: parent_inside,
                    })
                }
            }
        }
    }

    /// Enter a child: directory or (when in Fs) an archive file. Returns new location or None if not enterable.
    pub fn enter(
        &self,
        name: &str,
        is_dir: bool,
    ) -> Option<PanelLocation> {
        let name = name.trim_end_matches('/');
        if name.is_empty() {
            return None;
        }
        match self {
            PanelLocation::Fs(p) => {
                let child = p.join(name);
                if is_dir {
                    Some(PanelLocation::Fs(child))
                } else if let Some(format) = archive_format_for_filename(name) {
                    Some(PanelLocation::Archive {
                        format,
                        archive: child,
                        path_inside: String::new(),
                    })
                } else {
                    None
                }
            }
            PanelLocation::Archive {
                format,
                archive,
                path_inside,
            } => {
                if !is_dir {
                    return None;
                }
                let prefix = path_inside.trim_end_matches('/');
                let new_inside = if prefix.is_empty() {
                    name.to_string()
                } else {
                    format!("{}/{}", prefix, name)
                };
                Some(PanelLocation::Archive {
                    format: *format,
                    archive: archive.clone(),
                    path_inside: new_inside,
                })
            }
        }
    }

    /// Path for process cwd when this panel is active; only valid when is_fs().
    pub fn as_fs_path(&self) -> Option<&Path> {
        match self {
            PanelLocation::Fs(p) => Some(p.as_path()),
            PanelLocation::Archive { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{archive_format_for_filename, ArchiveFormat};

    #[test]
    fn archive_format_suffix_is_case_insensitive_without_alloc() {
        assert_eq!(
            archive_format_for_filename("a.ZIP"),
            Some(ArchiveFormat::Zip)
        );
        assert_eq!(
            archive_format_for_filename("b.Tar.Gz"),
            Some(ArchiveFormat::TarGz)
        );
        assert_eq!(
            archive_format_for_filename("c.TGZ"),
            Some(ArchiveFormat::TarGz)
        );
        assert_eq!(
            archive_format_for_filename("readme.md"),
            None
        );
    }
}
