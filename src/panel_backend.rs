//! Backend for panel listing and file operations. Dispatches by PanelLocation (Fs vs Zip)
//! so the same panel logic works for disk and archives.

use std::fs;
use std::io::{self, Write};
use std::path::Path;

use crate::file_ops::{apply_sort_mode, FileInfo, FileOperations};
use crate::location::PanelLocation;
use crate::copy_ops;

/// List contents of a location (directory or zip virtual directory).
/// sort_mode: name_asc, name_desc, size_asc, size_desc, mtime_asc, mtime_desc.
/// dirs_first: when true, directories appear before files.
pub fn list(loc: &PanelLocation, show_hidden: bool, sort_mode: &str, dirs_first: bool) -> io::Result<Vec<FileInfo>> {
    match loc {
        PanelLocation::Fs(p) => FileOperations::read_directory(p, show_hidden, sort_mode, dirs_first),
        PanelLocation::Zip { archive, path_inside } => zip_list(archive, path_inside, show_hidden, sort_mode, dirs_first),
    }
}

/// Read full contents of a file at the given location.
pub fn read_file(loc: &PanelLocation, name: &str) -> io::Result<Vec<u8>> {
    match loc {
        PanelLocation::Fs(p) => fs::read(FileOperations::join_path(p, name)),
        PanelLocation::Zip { archive, path_inside } => zip_read_file(archive, path_inside, name),
    }
}

/// Whether editing (F4) is supported at this location. False inside ZIP.
pub fn supports_edit(loc: &PanelLocation) -> bool {
    loc.is_fs()
}

/// Whether mkdir (F7) is supported. False inside ZIP.
pub fn supports_mkdir(loc: &PanelLocation) -> bool {
    loc.is_fs()
}

/// Create directory. Only valid when supports_mkdir(loc).
pub fn mkdir(loc: &PanelLocation, name: &str) -> io::Result<()> {
    match loc {
        PanelLocation::Fs(p) => fs::create_dir(FileOperations::join_path(p, name)),
        PanelLocation::Zip { .. } => Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Cannot create directory inside ZIP",
        )),
    }
}

/// Copy items from source location to a filesystem target dir (F5). Handles both Fs->Fs and Zip->Fs.
pub fn copy_items_to_fs(
    source: &PanelLocation,
    items: &[(String, bool)],
    target_dir: &Path,
) -> io::Result<()> {
    match source {
        PanelLocation::Fs(p) => {
            for (name, is_dir) in items {
                copy_ops::copy_item(p.as_path(), target_dir, name, *is_dir)?;
            }
            Ok(())
        }
        PanelLocation::Zip { archive, path_inside } => {
            zip_extract_items(archive, path_inside, items, target_dir, false)
        }
    }
}

/// Move items from source to filesystem target (F6): copy then delete from source.
pub fn move_items_to_fs(
    source: &PanelLocation,
    items: &[(String, bool)],
    target_dir: &Path,
) -> io::Result<()> {
    copy_items_to_fs(source, items, target_dir)?;
    delete_items(source, items)
}

/// Delete items at location (F8). For Zip, removes entries from the archive.
pub fn delete_items(loc: &PanelLocation, items: &[(String, bool)]) -> io::Result<()> {
    match loc {
        PanelLocation::Fs(p) => {
            for (name, is_dir) in items {
                copy_ops::delete_item(p, name, *is_dir)?;
            }
            Ok(())
        }
        PanelLocation::Zip { archive, path_inside } => zip_remove_items(archive, path_inside, items),
    }
}

/// Join path for display; for Fs same as FileOperations::join_path; for Zip we build virtual path.
pub fn join_path_display(loc: &PanelLocation, name: &str) -> String {
    match loc {
        PanelLocation::Fs(p) => FileOperations::join_path(p, name).to_string_lossy().to_string(),
        PanelLocation::Zip { archive, path_inside } => {
            let prefix = path_inside.trim_end_matches('/');
            let full = if prefix.is_empty() {
                name.to_string()
            } else {
                format!("{}/{}", prefix, name.trim_end_matches('/'))
            };
            format!("{}/{}", archive.to_string_lossy(), full)
        }
    }
}

// --- Zip implementation ---

fn zip_list(archive_path: &Path, path_inside: &str, show_hidden: bool, sort_mode: &str, dirs_first: bool) -> io::Result<Vec<FileInfo>> {
    let prefix = path_inside.trim_end_matches('/');
    let prefix_with_slash = if prefix.is_empty() {
        String::new()
    } else {
        format!("{}/", prefix)
    };

    let file = fs::File::open(archive_path)?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    let mut map: std::collections::HashMap<String, FileInfo> = std::collections::HashMap::new();

    for i in 0..archive.len() {
        let entry = archive.by_index(i).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let name = entry.name().to_string();
        let name_clean = name.trim_end_matches('/');

        if name_clean.is_empty() {
            continue;
        }
        let (display_name, is_dir) = if prefix.is_empty() {
            if name.contains('/') {
                let first = name.split('/').next().unwrap_or("");
                if first.is_empty() {
                    continue;
                }
                (first, true)
            } else {
                (name_clean, name.ends_with('/'))
            }
        } else {
            if !name.starts_with(&prefix_with_slash) {
                continue;
            }
            let rest = name[prefix_with_slash.len()..].trim_end_matches('/');
            if rest.is_empty() {
                continue;
            }
            if rest.contains('/') {
                let first = rest.split('/').next().unwrap_or("");
                (first, true)
            } else {
                (rest, name.ends_with('/'))
            }
        };
        if !show_hidden && display_name.starts_with('.') {
            continue;
        }
        let key = display_name.to_string();
        if map.contains_key(&key) {
            continue;
        }
        let display = if is_dir && !display_name.ends_with('/') {
            format!("{}/", display_name)
        } else {
            display_name.to_string()
        };
        let size = entry.size();
        // ZIP DateTime conversion APIs differ across versions/features; keep mtime empty for now.
        let mtime = None;
        map.insert(key, FileInfo::with_metadata(
            display,
            is_dir,
            false,
            false,
            size,
            mtime,
            "----------".to_string(),
            String::new(),
            String::new(),
        ));
    }

    let mut files: Vec<FileInfo> = map.into_values().collect();
    if !prefix.is_empty() || archive_path.parent().is_some() {
        files.push(FileInfo::new("..".to_string(), true, false));
    }
    apply_sort_mode(&mut files, sort_mode, dirs_first);
    Ok(files)
}

fn zip_read_file(archive_path: &Path, path_inside: &str, name: &str) -> io::Result<Vec<u8>> {
    let prefix = path_inside.trim_end_matches('/');
    let full_name = if prefix.is_empty() {
        name.trim_start_matches('/').to_string()
    } else {
        format!("{}/{}", prefix, name.trim_start_matches('/').trim_end_matches('/'))
    };

    let file = fs::File::open(archive_path)?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    if let Ok(mut entry) = archive.by_name(&full_name) {
        if !entry.is_dir() {
            let mut buf = Vec::with_capacity(entry.size() as usize);
            io::copy(&mut entry, &mut buf)?;
            return Ok(buf);
        }
    }

    let file = fs::File::open(archive_path)?;
    let mut arch2 = zip::ZipArchive::new(file).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let mut entry = arch2
        .by_name(&format!("{}/", full_name))
        .map_err(|e| io::Error::new(io::ErrorKind::NotFound, e))?;
    if entry.is_dir() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "Is a directory"));
    }
    let mut buf = Vec::with_capacity(entry.size() as usize);
    io::copy(&mut entry, &mut buf)?;
    Ok(buf)
}

fn zip_extract_items(
    archive_path: &Path,
    path_inside: &str,
    items: &[(String, bool)],
    target_dir: &Path,
    _remove_after: bool,
) -> io::Result<()> {
    let prefix = path_inside.trim_end_matches('/');

    let file = fs::File::open(archive_path)?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    for (name, is_dir) in items {
        let name_clean = name.trim_end_matches('/');
        let entry_path = if prefix.is_empty() {
            name_clean.to_string()
        } else {
            format!("{}/{}", prefix, name_clean)
        };

        if *is_dir {
            let dir_prefix = format!("{}/", entry_path.trim_end_matches('/'));
            for i in 0..archive.len() {
                let mut entry = archive.by_index(i).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
                let ename = entry.name().to_string();
                if ename == entry_path || ename == format!("{}/", entry_path) || ename.starts_with(&dir_prefix) {
                    let rel = ename[entry_path.len().min(ename.len())..].trim_start_matches('/');
                    let is_entry_dir = ename.ends_with('/');
                    let dest = target_dir.join(name_clean).join(rel.trim_end_matches('/'));
                    if is_entry_dir {
                        fs::create_dir_all(&dest).ok();
                    } else {
                        if let Some(parent) = dest.parent() {
                            let _ = fs::create_dir_all(parent);
                        }
                        let mut data = Vec::new();
                        if io::copy(&mut entry, &mut data).is_ok() {
                            let _ = fs::write(&dest, data);
                        }
                    }
                }
            }
            fs::create_dir_all(target_dir.join(name_clean))?;
        } else {
            let full_name = if prefix.is_empty() {
                name_clean.to_string()
            } else {
                format!("{}/{}", prefix, name_clean)
            };
            if let Ok(mut entry) = archive.by_name(&full_name) {
                let dest = target_dir.join(name_clean);
                let mut out = fs::File::create(&dest)?;
                io::copy(&mut entry, &mut out)?;
            }
        }
    }
    Ok(())
}

/// Rewrite archive without the given items (used for F8 delete and F6 move).
fn zip_remove_items(
    archive_path: &Path,
    path_inside: &str,
    items: &[(String, bool)],
) -> io::Result<()> {
    let prefix = path_inside.trim_end_matches('/');

    let to_remove: std::collections::HashSet<String> = items
        .iter()
        .map(|(name, is_dir)| {
            let n = name.trim_end_matches('/');
            if *is_dir {
                if prefix.is_empty() {
                    format!("{}/", n)
                } else {
                    format!("{}/{}", prefix, n)
                }
            } else {
                if prefix.is_empty() {
                    n.to_string()
                } else {
                    format!("{}/{}", prefix, n)
                }
            }
        })
        .chain(items.iter().map(|(name, is_dir)| {
            let n = name.trim_end_matches('/');
            if *is_dir {
                if prefix.is_empty() {
                    n.to_string()
                } else {
                    format!("{}/{}", prefix, n)
                }
            } else {
                String::new()
            }
        }))
        .filter(|s| !s.is_empty())
        .collect();

    let should_remove = |name: &str| -> bool {
        let name_clean = name.trim_end_matches('/');
        for rm in &to_remove {
            let rm_clean = rm.trim_end_matches('/');
            if name_clean == rm_clean || name_clean.starts_with(&format!("{}/", rm_clean)) {
                return true;
            }
        }
        false
    };

    let file = fs::File::open(archive_path)?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    let out_path = archive_path.with_extension("zip.tmp");
    let out_file = fs::File::create(&out_path)?;
    let mut writer = zip::ZipWriter::new(out_file);
    let options = zip::write::SimpleFileOptions::default();

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let name = entry.name().to_string();
        if should_remove(&name) {
            continue;
        }
        if entry.is_dir() {
            continue; // directory entries are optional; we'll create dirs from file paths
        }
        let mut data = Vec::new();
        io::copy(&mut entry, &mut data)?;
        drop(entry);
        let opts = options.compression_method(zip::CompressionMethod::Stored);
        writer.start_file(name, opts)?;
        writer.write_all(&data)?;
    }

    writer.finish()?;
    fs::rename(out_path, archive_path)?;
    Ok(())
}
