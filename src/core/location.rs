//! Panel location: either a filesystem path or a path inside a ZIP archive.
//! Used to make panel logic generic over "disk" and "archive" backends.

use std::path::{Path, PathBuf};

/// Current panel location: filesystem directory or virtual directory inside a ZIP.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum PanelLocation {
    /// A directory on disk.
    Fs(PathBuf),
    /// A path inside a ZIP archive. `path_inside` is stored with forward slashes, no leading slash.
    Zip {
        archive: PathBuf,
        path_inside: String,
    },
}

impl PanelLocation {
    pub fn fs<P: AsRef<Path>>(path: P) -> Self {
        PanelLocation::Fs(path.as_ref().to_path_buf())
    }

    /// Display string for the panel header (e.g. "/home/user" or "/path/to/arch.zip/subdir").
    pub fn display_string(&self) -> String {
        match self {
            PanelLocation::Fs(p) => p.to_string_lossy().to_string(),
            PanelLocation::Zip {
                archive,
                path_inside,
            } => {
                let archive_display = archive.to_string_lossy();
                if path_inside.is_empty() {
                    archive_display.to_string()
                } else {
                    format!("{}/{}", archive_display, path_inside.trim_end_matches('/'))
                }
            }
        }
    }

    /// True if this is a filesystem location (used for process cwd sync when autosave is off, subshell).
    pub fn is_fs(&self) -> bool {
        matches!(self, PanelLocation::Fs(_))
    }

    /// Parent location: for Fs the parent path; for Zip either parent dir inside archive or Fs(archive dir).
    pub fn parent(&self) -> Option<PanelLocation> {
        match self {
            PanelLocation::Fs(p) => p.parent().map(PanelLocation::fs),
            PanelLocation::Zip {
                archive,
                path_inside,
            } => {
                let trimmed = path_inside.trim_end_matches('/');
                if trimmed.is_empty() {
                    // Top of archive: parent is the directory containing the archive
                    archive.parent().map(PanelLocation::fs)
                } else {
                    let parent_inside = trimmed
                        .rsplit_once('/')
                        .map(|(p, _)| p.to_string())
                        .unwrap_or_default();
                    Some(PanelLocation::Zip {
                        archive: archive.clone(),
                        path_inside: parent_inside,
                    })
                }
            }
        }
    }

    /// Enter a child: directory or (when in Fs) a .zip file. Returns new location or None if not enterable.
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
                } else {
                    // Enter .zip as archive root
                    let lower = name.to_lowercase();
                    if lower.ends_with(".zip") {
                        Some(PanelLocation::Zip {
                            archive: child,
                            path_inside: String::new(),
                        })
                    } else {
                        None
                    }
                }
            }
            PanelLocation::Zip {
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
                Some(PanelLocation::Zip {
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
            PanelLocation::Zip { .. } => None,
        }
    }
}
