//! Backend for panel listing and file operations. Dispatches by PanelLocation (Fs vs archives)
//! so the same panel logic works on disk, ZIP, and tar.gz.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use super::copy_ops;
use super::file_ops::{
    apply_archive_unix_mode, apply_sort_mode, archive_entry_listing_fields, FileInfo,
    FileOperations, SortMode,
};
use super::location::{archive_format_for_path, ArchiveFormat, PanelLocation};
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use tar::{Archive, Builder, EntryType, Header};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

#[cfg(unix)]
fn zip_unix_perm_bits_from_path(path: &Path) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path)
        .ok()
        .map(|m| m.permissions().mode() & 0o777)
}

/// Zip writer options with Unix permission bits from `path` (files and directories).
fn zip_options_for_path(
    path: &Path,
    method: CompressionMethod,
) -> SimpleFileOptions {
    let base = SimpleFileOptions::default().compression_method(method);
    #[cfg(unix)]
    if let Some(bits) = zip_unix_perm_bits_from_path(path) {
        return base.unix_permissions(bits);
    }
    #[cfg(not(unix))]
    let _ = path;
    base
}

/// Zip writer options preserving mode from an existing central-directory entry.
fn zip_options_from_unix_mode(
    unix_mode: Option<u32>,
    method: CompressionMethod,
) -> SimpleFileOptions {
    let base = SimpleFileOptions::default().compression_method(method);
    #[cfg(unix)]
    if let Some(m) = unix_mode {
        return base.unix_permissions(m & 0o777);
    }
    #[cfg(not(unix))]
    let _ = unix_mode;
    base
}

fn zip_options_for_fs_under(
    source: &PanelLocation,
    path_under_panel: &str,
    method: CompressionMethod,
) -> SimpleFileOptions {
    match source {
        PanelLocation::Fs(p) => {
            let path = FileOperations::join_path(
                p,
                path_under_panel.trim_start_matches('/'),
            );
            zip_options_for_path(&path, method)
        }
        PanelLocation::Archive { .. } => SimpleFileOptions::default().compression_method(method),
    }
}

fn tar_mode_from_path(path: &Path) -> Option<u32> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path)
            .ok()
            .map(|m| m.permissions().mode() & 0o7777)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        None
    }
}

/// List contents of a location (directory or zip virtual directory).
/// dirs_first: when true, directories appear before files.
pub fn list(
    loc: &PanelLocation,
    show_hidden: bool,
    sort_mode: SortMode,
    dirs_first: bool,
) -> io::Result<Vec<FileInfo>> {
    match loc {
        PanelLocation::Fs(p) => {
            FileOperations::read_directory(p, show_hidden, sort_mode, dirs_first)
        }
        PanelLocation::Archive {
            format: ArchiveFormat::Zip,
            archive,
            path_inside,
        } => zip_list(
            archive,
            path_inside,
            show_hidden,
            sort_mode,
            dirs_first,
        ),
        PanelLocation::Archive {
            format: ArchiveFormat::TarGz,
            archive,
            path_inside,
        } => tar_gz_list(
            archive,
            path_inside,
            show_hidden,
            sort_mode,
            dirs_first,
        ),
    }
}

/// Read full contents of a file at the given location.
pub fn read_file(
    loc: &PanelLocation,
    name: &str,
) -> io::Result<Vec<u8>> {
    match loc {
        PanelLocation::Fs(p) => fs::read(FileOperations::join_path(p, name)),
        PanelLocation::Archive {
            format: ArchiveFormat::Zip,
            archive,
            path_inside,
        } => zip_read_file(archive, path_inside, name),
        PanelLocation::Archive {
            format: ArchiveFormat::TarGz,
            archive,
            path_inside,
        } => tar_gz_read_file(archive, path_inside, name),
    }
}

/// Whether editing (F4) is supported at this location. True for Fs and Zip.
pub fn supports_edit(_loc: &PanelLocation) -> bool {
    true
}

/// Whether mkdir (F7) is supported. True for Fs and Zip.
pub fn supports_mkdir(_loc: &PanelLocation) -> bool {
    true
}

/// Whether creating a new file (Ctrl+N) is supported. True for Fs and Zip.
pub fn supports_new_file(_loc: &PanelLocation) -> bool {
    true
}

/// Check if a file or directory with the given name already exists at the location.
pub fn entry_exists(
    loc: &PanelLocation,
    name: &str,
) -> io::Result<bool> {
    let name_clean = name.trim_end_matches('/');
    if name_clean.is_empty() || name_clean == ".." {
        return Ok(false);
    }
    match loc {
        PanelLocation::Fs(p) => Ok(FileOperations::join_path(p, name_clean).exists()),
        PanelLocation::Archive { .. } => {
            let files = list(loc, true, SortMode::NameAsc, true)?;
            Ok(files.iter().any(|f| f.name == name_clean))
        }
    }
}

/// Create directory. Only valid when supports_mkdir(loc). For Zip, adds a directory entry to the archive.
pub fn mkdir(
    loc: &PanelLocation,
    name: &str,
) -> io::Result<()> {
    match loc {
        PanelLocation::Fs(p) => fs::create_dir(FileOperations::join_path(p, name)),
        PanelLocation::Archive {
            format: ArchiveFormat::Zip,
            archive,
            path_inside,
        } => zip_mkdir(archive, path_inside, name),
        PanelLocation::Archive {
            format: ArchiveFormat::TarGz,
            archive,
            path_inside,
        } => tar_gz_mkdir(archive, path_inside, name),
    }
}

/// Write full contents of a file at the given location. For Zip, rewrites the archive with this entry replaced or added.
pub fn write_file(
    loc: &PanelLocation,
    name: &str,
    content: &[u8],
) -> io::Result<()> {
    match loc {
        PanelLocation::Fs(p) => fs::write(
            FileOperations::join_path(p, name),
            content,
        ),
        PanelLocation::Archive {
            format: ArchiveFormat::Zip,
            archive,
            path_inside,
        } => zip_write_file(archive, path_inside, name, content),
        PanelLocation::Archive {
            format: ArchiveFormat::TarGz,
            archive,
            path_inside,
        } => tar_gz_write_file(archive, path_inside, name, content),
    }
}

/// Create a zip archive containing the given items at the current location. Only valid for Fs.
/// Originals are not modified. archive_name is the file name (e.g. "archive.zip").
#[allow(dead_code)]
pub fn create_archive(
    loc: &PanelLocation,
    items: &[(String, bool)],
    archive_name: &str,
) -> io::Result<()> {
    create_archive_with_progress(
        loc,
        items,
        archive_name,
        &mut |_, _, _| {},
        None,
    )
}

/// Like create_archive but calls progress(current_1based, total, current_item_path) for each top-level item.
/// If cancel is Some and load(Relaxed) becomes true, stops and returns Err.
pub fn create_archive_with_progress(
    loc: &PanelLocation,
    items: &[(String, bool)],
    archive_name: &str,
    progress: &mut impl FnMut(usize, usize, &str),
    cancel: Option<&std::sync::atomic::AtomicBool>,
) -> io::Result<()> {
    let base_dir = match loc {
        PanelLocation::Fs(p) => p.as_path(),
        PanelLocation::Archive { .. } => {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Cannot create archive inside an archive",
            ));
        }
    };
    let lower = archive_name.to_lowercase();
    if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
        return create_tar_gz_archive_with_progress(loc, items, archive_name, progress, cancel);
    }
    let archive_path = FileOperations::join_path(base_dir, archive_name);
    let file = fs::File::create(&archive_path)?;
    let mut writer = ZipWriter::new(file);
    let zip_method = CompressionMethod::Deflated;

    let total = items.len();
    for (idx, (name, is_dir)) in items.iter().enumerate() {
        if let Some(c) = cancel {
            if c.load(Ordering::Relaxed) {
                drop(writer);
                let _ = fs::remove_file(&archive_path);
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "Archive cancelled",
                ));
            }
        }
        let current_path = FileOperations::join_path(base_dir, name);
        let path_display = current_path.to_string_lossy().to_string();
        progress(idx + 1, total, &path_display);

        let full_path = FileOperations::join_path(base_dir, name);
        let name_clean = name.trim_end_matches('/');
        if *is_dir {
            for entry in walkdir::WalkDir::new(&full_path)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                if let Some(c) = cancel {
                    if c.load(Ordering::Relaxed) {
                        drop(writer);
                        let _ = fs::remove_file(&archive_path);
                        return Err(io::Error::new(
                            io::ErrorKind::Interrupted,
                            "Archive cancelled",
                        ));
                    }
                }
                let path = entry.path();
                let relative = path
                    .strip_prefix(&full_path)
                    .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "strip_prefix"))?;
                let name_in_zip: String = format!(
                    "{}/{}",
                    name_clean.replace('\\', "/"),
                    relative.to_string_lossy().replace('\\', "/")
                );
                if path.is_dir() {
                    writer.add_directory(
                        &name_in_zip,
                        zip_options_for_path(path, zip_method),
                    )?;
                } else {
                    writer.start_file(
                        &name_in_zip,
                        zip_options_for_path(path, zip_method),
                    )?;
                    io::copy(&mut fs::File::open(path)?, &mut writer)?;
                }
            }
        } else {
            writer.start_file(
                name_clean,
                zip_options_for_path(&full_path, zip_method),
            )?;
            io::copy(
                &mut fs::File::open(&full_path)?,
                &mut writer,
            )?;
        }
    }

    writer.finish()?;
    Ok(())
}

/// Create a `.tar.gz` / `.tgz` archive (same selection model as ZIP).
fn create_tar_gz_archive_with_progress(
    loc: &PanelLocation,
    items: &[(String, bool)],
    archive_name: &str,
    progress: &mut impl FnMut(usize, usize, &str),
    cancel: Option<&std::sync::atomic::AtomicBool>,
) -> io::Result<()> {
    let base_dir = match loc {
        PanelLocation::Fs(p) => p.as_path(),
        PanelLocation::Archive { .. } => {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Cannot create archive inside an archive",
            ));
        }
    };
    let archive_path = FileOperations::join_path(base_dir, archive_name);
    let file = fs::File::create(&archive_path)?;
    let enc = GzEncoder::new(file, Compression::default());
    let mut builder = Builder::new(enc);

    let total = items.len();

    for (idx, (name, is_dir)) in items.iter().enumerate() {
        if let Some(c) = cancel {
            if c.load(Ordering::Relaxed) {
                drop(builder);
                let _ = fs::remove_file(&archive_path);
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "Archive cancelled",
                ));
            }
        }
        let current_path = FileOperations::join_path(base_dir, name);
        let path_display = current_path.to_string_lossy().to_string();
        progress(idx + 1, total, &path_display);

        let full_path = FileOperations::join_path(base_dir, name);
        let name_clean = name.trim_end_matches('/');
        if *is_dir {
            for entry in walkdir::WalkDir::new(&full_path)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                if let Some(c) = cancel {
                    if c.load(Ordering::Relaxed) {
                        drop(builder);
                        let _ = fs::remove_file(&archive_path);
                        return Err(io::Error::new(
                            io::ErrorKind::Interrupted,
                            "Archive cancelled",
                        ));
                    }
                }
                let path = entry.path();
                let relative = path
                    .strip_prefix(&full_path)
                    .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "strip_prefix"))?;
                let name_in_archive: String = format!(
                    "{}/{}",
                    name_clean.replace('\\', "/"),
                    relative.to_string_lossy().replace('\\', "/")
                );
                if path.is_dir() {
                    let mut header = Header::new_gnu();
                    let dir_path = format!(
                        "{}/",
                        name_in_archive.trim_end_matches('/')
                    );
                    header
                        .set_path(&dir_path)
                        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
                    header.set_entry_type(EntryType::Directory);
                    header.set_size(0);
                    #[cfg(unix)]
                    let dir_mode = tar_mode_from_path(path).unwrap_or(0o755);
                    #[cfg(not(unix))]
                    let dir_mode = 0o755_u32;
                    header.set_mode(dir_mode);
                    header.set_cksum();
                    builder
                        .append(&header, &[][..])
                        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
                } else {
                    builder
                        .append_path_with_name(path, name_in_archive.as_str())
                        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
                }
            }
        } else {
            builder
                .append_path_with_name(&full_path, name_clean)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        }
    }

    builder
        .finish()
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    Ok(())
}

/// Copy items from source location to a filesystem target dir (F5). Handles both Fs->Fs and Zip->Fs.
/// When `dest_names` is `Some`, each `items[i]` is written using `dest_names[i]`; otherwise the source basename is kept.
pub fn copy_items_to_fs(
    source: &PanelLocation,
    items: &[(String, bool)],
    target_dir: &Path,
    dest_names: Option<&[String]>,
) -> io::Result<()> {
    match source {
        PanelLocation::Fs(p) => {
            for (i, (name, is_dir)) in items.iter().enumerate() {
                let dst = dest_names
                    .and_then(|d| d.get(i))
                    .map(|s| s.as_str())
                    .unwrap_or(name.as_str());
                copy_ops::copy_item_as(
                    p.as_path(),
                    target_dir,
                    name,
                    dst,
                    *is_dir,
                )?;
            }
            Ok(())
        }
        PanelLocation::Archive {
            format: ArchiveFormat::Zip,
            archive,
            path_inside,
        } => zip_extract_items(
            archive,
            path_inside,
            items,
            target_dir,
            false,
            dest_names,
        ),
        PanelLocation::Archive {
            format: ArchiveFormat::TarGz,
            archive,
            path_inside,
        } => tar_gz_extract_items(
            archive,
            path_inside,
            items,
            target_dir,
            dest_names,
        ),
    }
}

/// Move items from source to filesystem target (F6): copy (optionally under renamed basenames) then delete from source.
pub fn move_items_to_fs(
    source: &PanelLocation,
    items: &[(String, bool)],
    target_dir: &Path,
    dest_names: Option<&[String]>,
) -> io::Result<()> {
    copy_items_to_fs(source, items, target_dir, dest_names)?;
    delete_items(source, items, false)
}

/// Copy items from any source location into an existing ZIP at the given path.
/// target_archive/path_inside is the virtual directory inside the archive where items are added.
/// Handles Fs->Zip and Zip->Zip (including directories recursively).
/// When `dest_roots` is `Some`, each top-level entry inside `path_inside` uses `dest_roots[i]` instead of the source name.
pub fn copy_items_into_archive(
    source: &PanelLocation,
    items: &[(String, bool)],
    target_archive: &Path,
    path_inside: &str,
    dest_roots: Option<&[String]>,
) -> io::Result<()> {
    match archive_format_for_path(target_archive) {
        Some(ArchiveFormat::Zip) => zip_add_items(
            target_archive,
            path_inside,
            source,
            items,
            dest_roots,
        ),
        Some(ArchiveFormat::TarGz) => tar_gz_add_items(
            target_archive,
            path_inside,
            source,
            items,
            dest_roots,
        ),
        None => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "unsupported archive extension (use .zip, .tar.gz, or .tgz)",
        )),
    }
}

/// Move items from source into an existing ZIP: copy then delete from source.
pub fn move_items_into_archive(
    source: &PanelLocation,
    items: &[(String, bool)],
    target_archive: &Path,
    path_inside: &str,
) -> io::Result<()> {
    copy_items_into_archive(
        source,
        items,
        target_archive,
        path_inside,
        None,
    )?;
    delete_items(source, items, false)
}

/// Delete items at location (F8). For Zip, removes entries from the archive (`use_trash` ignored).
pub fn delete_items(
    loc: &PanelLocation,
    items: &[(String, bool)],
    use_trash: bool,
) -> io::Result<()> {
    match loc {
        PanelLocation::Fs(p) => {
            for (name, is_dir) in items {
                copy_ops::delete_item(p, name, *is_dir, use_trash)?;
            }
            Ok(())
        }
        PanelLocation::Archive {
            format: ArchiveFormat::Zip,
            archive,
            path_inside,
        } => zip_remove_items(archive, path_inside, items),
        PanelLocation::Archive {
            format: ArchiveFormat::TarGz,
            archive,
            path_inside,
        } => tar_gz_remove_items(archive, path_inside, items),
    }
}

/// Join path for display; for Fs same as FileOperations::join_path; for Zip we build virtual path.
pub fn join_path_display(
    loc: &PanelLocation,
    name: &str,
) -> String {
    match loc {
        PanelLocation::Fs(p) => FileOperations::join_path(p, name)
            .to_string_lossy()
            .to_string(),
        PanelLocation::Archive {
            archive,
            path_inside,
            ..
        } => {
            let prefix = path_inside.trim_end_matches('/');
            let full = if prefix.is_empty() {
                name.to_string()
            } else {
                format!(
                    "{}/{}",
                    prefix,
                    name.trim_end_matches('/')
                )
            };
            format!("{}/{}", archive.to_string_lossy(), full)
        }
    }
}

// --- Zip implementation ---

fn archive_child_path(
    prefix: &str,
    name: &str,
) -> String {
    let name = name.trim_end_matches('/');
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{}/{}", prefix, name)
    }
}

fn zip_list(
    archive_path: &Path,
    path_inside: &str,
    show_hidden: bool,
    sort_mode: SortMode,
    dirs_first: bool,
) -> io::Result<Vec<FileInfo>> {
    let prefix = path_inside.trim_end_matches('/');
    let prefix_with_slash = if prefix.is_empty() {
        String::new()
    } else {
        format!("{}/", prefix)
    };

    let file = fs::File::open(archive_path)?;
    let mut archive =
        ZipArchive::new(file).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    let mut map: HashMap<String, FileInfo> = HashMap::with_capacity(archive.len());

    for i in 0..archive.len() {
        let entry = archive
            .by_index(i)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
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
        // Store name without trailing slash; is_dir already marks directories.
        let display = display_name.to_string();
        let size = entry.size();
        // ZIP DateTime conversion APIs differ across versions/features; keep mtime empty for now.
        let mtime = None;
        let is_symlink = entry.is_symlink();
        let listing_mode = if is_dir && !entry.is_dir() && !name.ends_with('/') {
            None
        } else {
            entry.unix_mode()
        };
        let (permissions, is_executable) =
            archive_entry_listing_fields(listing_mode, is_dir, is_symlink);
        map.insert(
            key,
            FileInfo::with_metadata(
                display,
                is_dir,
                is_executable,
                is_symlink,
                size,
                mtime,
                permissions,
                String::new(),
                String::new(),
            ),
        );
    }

    let mut files: Vec<FileInfo> = map.into_values().collect();
    if !prefix.is_empty() || archive_path.parent().is_some() {
        let mut parent = FileInfo::new("..".to_string(), true, false);
        #[cfg(unix)]
        {
            parent.permissions = "drwxr-xr-x".to_string();
        }
        files.push(parent);
    }
    apply_sort_mode(&mut files, sort_mode, dirs_first);
    Ok(files)
}

fn zip_read_file(
    archive_path: &Path,
    path_inside: &str,
    name: &str,
) -> io::Result<Vec<u8>> {
    let prefix = path_inside.trim_end_matches('/');
    let full_name = if prefix.is_empty() {
        name.trim_start_matches('/').to_string()
    } else {
        format!(
            "{}/{}",
            prefix,
            name.trim_start_matches('/').trim_end_matches('/')
        )
    };

    let file = fs::File::open(archive_path)?;
    let mut archive =
        ZipArchive::new(file).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    if let Ok(mut entry) = archive.by_name(&full_name) {
        if !entry.is_dir() {
            let mut buf = Vec::with_capacity(entry.size() as usize);
            io::copy(&mut entry, &mut buf)?;
            return Ok(buf);
        }
    }

    let file = fs::File::open(archive_path)?;
    let mut arch2 =
        ZipArchive::new(file).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let mut entry = arch2
        .by_name(&format!("{}/", full_name))
        .map_err(|e| io::Error::new(io::ErrorKind::NotFound, e))?;
    if entry.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Is a directory",
        ));
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
    dest_names: Option<&[String]>,
) -> io::Result<()> {
    let prefix = path_inside.trim_end_matches('/');

    let file = fs::File::open(archive_path)?;
    let mut archive =
        ZipArchive::new(file).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    struct DirSpec {
        entry_path: String,
        entry_path_slash: String,
        dest_root: String,
    }
    let mut dirs = Vec::new();

    for (i, (name, is_dir)) in items.iter().enumerate() {
        let name_clean = name.trim_end_matches('/');
        let dest_root = dest_names
            .and_then(|d| d.get(i))
            .map(|s| s.trim_end_matches('/'))
            .unwrap_or(name_clean)
            .to_string();
        let entry_path = archive_child_path(prefix, name_clean);
        if *is_dir {
            let entry_path_slash = format!("{}/", entry_path.trim_end_matches('/'));
            dirs.push(DirSpec {
                entry_path,
                entry_path_slash,
                dest_root,
            });
            continue;
        }
        if let Ok(mut entry) = archive.by_name(&entry_path) {
            let mode = entry.unix_mode();
            let dest = target_dir.join(&dest_root);
            let mut out = fs::File::create(&dest)?;
            io::copy(&mut entry, &mut out)?;
            drop(out);
            let _ = apply_archive_unix_mode(&dest, mode);
        }
    }

    if !dirs.is_empty() {
        for i in 0..archive.len() {
            let mut entry = archive
                .by_index(i)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            let ename = entry.name().to_string();
            let Some(d) = dirs.iter().find(|d| {
                ename == d.entry_path
                    || ename == d.entry_path_slash
                    || ename.starts_with(&d.entry_path_slash)
            }) else {
                continue;
            };
            let rel = ename[d.entry_path.len().min(ename.len())..].trim_start_matches('/');
            let is_entry_dir = ename.ends_with('/');
            let dest = target_dir
                .join(&d.dest_root)
                .join(rel.trim_end_matches('/'));
            let mode = entry.unix_mode();
            if is_entry_dir {
                fs::create_dir_all(&dest).ok();
                let _ = apply_archive_unix_mode(&dest, mode);
            } else {
                if let Some(parent) = dest.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                let mut data = Vec::new();
                if io::copy(&mut entry, &mut data).is_ok() {
                    if fs::write(&dest, data).is_ok() {
                        let _ = apply_archive_unix_mode(&dest, mode);
                    }
                }
            }
        }
        for d in &dirs {
            fs::create_dir_all(target_dir.join(&d.dest_root))?;
        }
    }
    Ok(())
}

/// List all entries under a directory at the given location (recursive). Returns (relative_path, is_dir) with forward slashes.
fn list_all_under(
    loc: &PanelLocation,
    dir_name: &str,
) -> io::Result<Vec<(String, bool)>> {
    match loc {
        PanelLocation::Fs(p) => {
            let dir_path = FileOperations::join_path(p, dir_name);
            let mut out = Vec::new();
            for entry in walkdir::WalkDir::new(&dir_path)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                let path = entry.path();
                let relative = path
                    .strip_prefix(&dir_path)
                    .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "strip_prefix"))?;
                let relative_str = relative.to_string_lossy().replace('\\', "/");
                if relative_str.is_empty() {
                    continue;
                }
                out.push((relative_str, path.is_dir()));
            }
            Ok(out)
        }
        PanelLocation::Archive {
            format: ArchiveFormat::Zip,
            archive,
            path_inside,
        } => {
            let prefix_trim = path_inside.trim_end_matches('/');
            let dir_trim = dir_name.trim_end_matches('/');
            let prefix = if prefix_trim.is_empty() {
                format!("{}/", dir_trim)
            } else {
                format!("{}/{}/", prefix_trim, dir_trim)
            };
            let file = fs::File::open(archive)?;
            let mut arch =
                ZipArchive::new(file).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            let mut out = Vec::with_capacity(arch.len());
            for i in 0..arch.len() {
                let entry = arch
                    .by_index(i)
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
                let name = entry.name().to_string();
                if !name.starts_with(&prefix) || name == prefix {
                    continue;
                }
                let relative = name[prefix.len()..].trim_end_matches('/').to_string();
                if relative.is_empty() {
                    continue;
                }
                out.push((relative, name.ends_with('/')));
            }
            Ok(out)
        }
        PanelLocation::Archive {
            format: ArchiveFormat::TarGz,
            archive,
            path_inside,
        } => tar_gz_list_all_under(archive, path_inside, dir_name),
    }
}

/// Add items from source into an existing archive at path_inside. Overwrites existing entries with the same name.
/// When `dest_roots` is set (same length as `items`), new archive paths use those names instead of the source names.
fn zip_add_items(
    archive_path: &Path,
    path_inside: &str,
    source: &PanelLocation,
    items: &[(String, bool)],
    dest_roots: Option<&[String]>,
) -> io::Result<()> {
    let target_prefix = path_inside.trim_end_matches('/');
    let target_prefix_slash = if target_prefix.is_empty() {
        String::new()
    } else {
        format!("{}/", target_prefix)
    };

    let mut to_add: HashSet<String> = HashSet::new();
    for (i, (name, is_dir)) in items.iter().enumerate() {
        let name_clean = name.trim_end_matches('/');
        let dst_root = dest_roots
            .and_then(|d| d.get(i))
            .map(|s| s.trim_end_matches('/'))
            .unwrap_or(name_clean);
        if *is_dir {
            if let Ok(entries) = list_all_under(source, name_clean) {
                if target_prefix_slash.is_empty() {
                    to_add.insert(format!("{}/", dst_root));
                } else {
                    to_add.insert(format!(
                        "{}{}/",
                        target_prefix_slash, dst_root
                    ));
                }
                for (rel, rel_is_dir) in entries {
                    let zip_name = if target_prefix_slash.is_empty() {
                        format!("{}/{}", dst_root, rel)
                    } else {
                        format!(
                            "{}{}/{}",
                            target_prefix_slash, dst_root, rel
                        )
                    };
                    let zip_name = if rel_is_dir {
                        format!("{}/", zip_name.trim_end_matches('/'))
                    } else {
                        zip_name
                    };
                    to_add.insert(zip_name);
                }
            }
        } else {
            let zip_name = if target_prefix_slash.is_empty() {
                dst_root.to_string()
            } else {
                format!("{}{}", target_prefix_slash, dst_root)
            };
            to_add.insert(zip_name);
        }
    }

    let file = fs::File::open(archive_path)?;
    let mut archive =
        ZipArchive::new(file).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let out_path = archive_path.with_extension("zip.tmp");
    let out_file = fs::File::create(&out_path)?;
    let mut writer = ZipWriter::new(out_file);
    let zip_method = CompressionMethod::Deflated;

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let name = entry.name().to_string();
        if to_add.contains(&name) || to_add.contains(name.trim_end_matches('/')) {
            continue;
        }
        if entry.is_dir() {
            continue;
        }
        let unix_mode = entry.unix_mode();
        let mut data = Vec::new();
        io::copy(&mut entry, &mut data)?;
        drop(entry);
        let copy_opts = zip_options_from_unix_mode(unix_mode, CompressionMethod::Stored);
        writer.start_file(name, copy_opts)?;
        writer.write_all(&data)?;
    }

    for (i, (name, is_dir)) in items.iter().enumerate() {
        let name_clean = name.trim_end_matches('/');
        let dst_root = dest_roots
            .and_then(|d| d.get(i))
            .map(|s| s.trim_end_matches('/'))
            .unwrap_or(name_clean);
        if *is_dir {
            let entries = list_all_under(source, name_clean)?;
            let _dir_zip_prefix = if target_prefix_slash.is_empty() {
                format!("{}/", dst_root)
            } else {
                format!("{}{}/", target_prefix_slash, dst_root)
            };
            for (rel, rel_is_dir) in entries {
                let zip_name = if target_prefix_slash.is_empty() {
                    format!("{}/{}", dst_root, rel)
                } else {
                    format!(
                        "{}{}/{}",
                        target_prefix_slash, dst_root, rel
                    )
                };
                if rel_is_dir {
                    let dir_entry = format!("{}/", zip_name.trim_end_matches('/'));
                    let fs_under = format!(
                        "{}/{}",
                        name_clean,
                        rel.trim_end_matches('/')
                    );
                    writer.add_directory(
                        &dir_entry,
                        zip_options_for_fs_under(source, &fs_under, zip_method),
                    )?;
                } else {
                    let rel_path = format!("{}/{}", name_clean, rel);
                    let data = read_file(source, &rel_path)?;
                    writer.start_file(
                        &zip_name,
                        zip_options_for_fs_under(source, &rel_path, zip_method),
                    )?;
                    writer.write_all(&data)?;
                }
            }
        } else {
            let data = read_file(source, name_clean)?;
            let zip_name = if target_prefix_slash.is_empty() {
                dst_root.to_string()
            } else {
                format!("{}{}", target_prefix_slash, dst_root)
            };
            writer.start_file(
                &zip_name,
                zip_options_for_fs_under(source, name_clean, zip_method),
            )?;
            writer.write_all(&data)?;
        }
    }

    writer.finish()?;
    fs::rename(out_path, archive_path)?;
    Ok(())
}

/// Rewrite archive without the given items (used for F8 delete and F6 move).
fn zip_remove_items(
    archive_path: &Path,
    path_inside: &str,
    items: &[(String, bool)],
) -> io::Result<()> {
    let prefix = path_inside.trim_end_matches('/');

    let to_remove: HashSet<String> = items
        .iter()
        .map(|(name, is_dir)| {
            let trimmed_name = name.trim_end_matches('/');
            if *is_dir {
                if prefix.is_empty() {
                    format!("{}/", trimmed_name)
                } else {
                    format!("{}/{}", prefix, trimmed_name)
                }
            } else if prefix.is_empty() {
                trimmed_name.to_string()
            } else {
                format!("{}/{}", prefix, trimmed_name)
            }
        })
        .chain(items.iter().map(|(name, is_dir)| {
            let trimmed_name = name.trim_end_matches('/');
            if *is_dir {
                if prefix.is_empty() {
                    trimmed_name.to_string()
                } else {
                    format!("{}/{}", prefix, trimmed_name)
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
    let mut archive =
        ZipArchive::new(file).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    let out_path = archive_path.with_extension("zip.tmp");
    let out_file = fs::File::create(&out_path)?;
    let mut writer = ZipWriter::new(out_file);

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let name = entry.name().to_string();
        if should_remove(&name) {
            continue;
        }
        if entry.is_dir() {
            continue; // directory entries are optional; we'll create dirs from file paths
        }
        let unix_mode = entry.unix_mode();
        let mut data = Vec::new();
        io::copy(&mut entry, &mut data)?;
        drop(entry);
        let opts = zip_options_from_unix_mode(unix_mode, CompressionMethod::Stored);
        writer.start_file(name, opts)?;
        writer.write_all(&data)?;
    }

    writer.finish()?;
    fs::rename(out_path, archive_path)?;
    Ok(())
}

/// Add a directory entry to an existing archive at path_inside/name/.
fn zip_mkdir(
    archive_path: &Path,
    path_inside: &str,
    name: &str,
) -> io::Result<()> {
    let prefix = path_inside.trim_end_matches('/');
    let name_clean = name.trim_end_matches('/');
    let dir_entry = if prefix.is_empty() {
        format!("{}/", name_clean)
    } else {
        format!("{}/{}/", prefix, name_clean)
    };

    let file = fs::File::open(archive_path)?;
    let mut archive =
        ZipArchive::new(file).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let out_path = archive_path.with_extension("zip.tmp");
    let out_file = fs::File::create(&out_path)?;
    let mut writer = ZipWriter::new(out_file);
    let opts = SimpleFileOptions::default();

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let name = entry.name().to_string();
        if entry.is_dir() {
            continue;
        }
        let unix_mode = entry.unix_mode();
        let mut data = Vec::new();
        io::copy(&mut entry, &mut data)?;
        drop(entry);
        let copy_opts = zip_options_from_unix_mode(unix_mode, CompressionMethod::Stored);
        writer.start_file(name, copy_opts)?;
        writer.write_all(&data)?;
    }

    writer.add_directory(&dir_entry, opts)?;
    writer.finish()?;
    fs::rename(out_path, archive_path)?;
    Ok(())
}

/// Replace or add a single file in the archive at path_inside/name.
fn zip_write_file(
    archive_path: &Path,
    path_inside: &str,
    name: &str,
    content: &[u8],
) -> io::Result<()> {
    let prefix = path_inside.trim_end_matches('/');
    let name_clean = name.trim_end_matches('/');
    let entry_name = if prefix.is_empty() {
        name_clean.to_string()
    } else {
        format!("{}/{}", prefix, name_clean)
    };

    let file = fs::File::open(archive_path)?;
    let mut archive =
        ZipArchive::new(file).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let out_path = archive_path.with_extension("zip.tmp");
    let out_file = fs::File::create(&out_path)?;
    let mut writer = ZipWriter::new(out_file);
    let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let name = entry.name().to_string();
        if name == entry_name || name == format!("{}/", entry_name) {
            continue; // skip existing entry; we'll write new content below
        }
        if entry.is_dir() {
            continue;
        }
        let unix_mode = entry.unix_mode();
        let mut data = Vec::new();
        io::copy(&mut entry, &mut data)?;
        drop(entry);
        let copy_opts = zip_options_from_unix_mode(unix_mode, CompressionMethod::Stored);
        writer.start_file(name, copy_opts)?;
        writer.write_all(&data)?;
    }

    writer.start_file(&entry_name, opts)?;
    writer.write_all(content)?;
    writer.finish()?;
    fs::rename(out_path, archive_path)?;
    Ok(())
}

// --- tar.gz implementation ---

fn archive_temp_path(archive_path: &Path) -> PathBuf {
    let mut name = archive_path.file_name().unwrap_or_default().to_os_string();
    name.push(".tmp");
    archive_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(name)
}

fn normalize_tar_path(s: &str) -> String {
    s.replace('\\', "/").trim_start_matches('/').to_string()
}

struct TarEntry {
    path: String,
    data: Vec<u8>,
    is_dir: bool,
    /// GNU/ustar mode bits (as stored in the tar header).
    mode: u32,
}

fn tar_gz_read_all_entries(archive_path: &Path) -> io::Result<Vec<TarEntry>> {
    let file = fs::File::open(archive_path)?;
    let dec = GzDecoder::new(file);
    let mut archive = Archive::new(dec);
    let mut out = Vec::new();
    for entry in archive
        .entries()
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?
    {
        let mut entry = entry.map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let path = entry
            .path()
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let raw = normalize_tar_path(&path.to_string_lossy());
        let is_dir = entry.header().entry_type().is_dir() || raw.ends_with('/');
        let name_clean = raw.trim_end_matches('/').to_string();
        if name_clean.is_empty() {
            continue;
        }
        let mode = entry
            .header()
            .mode()
            .unwrap_or(if is_dir { 0o755 } else { 0o644 });
        if is_dir {
            out.push(TarEntry {
                path: format!("{}/", name_clean),
                data: Vec::new(),
                is_dir: true,
                mode,
            });
        } else {
            let mut data = Vec::new();
            io::copy(&mut entry, &mut data)?;
            out.push(TarEntry {
                path: name_clean,
                data,
                is_dir: false,
                mode,
            });
        }
    }
    Ok(out)
}

fn write_tar_gz_entries(
    archive_path: &Path,
    entries: &[TarEntry],
) -> io::Result<()> {
    let tmp = archive_temp_path(archive_path);
    let out = fs::File::create(&tmp)?;
    let enc = GzEncoder::new(out, Compression::default());
    let mut builder = Builder::new(enc);
    for e in entries {
        let mut header = Header::new_gnu();
        header
            .set_path(&e.path)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        if e.is_dir {
            header.set_entry_type(EntryType::Directory);
            header.set_size(0);
            header.set_mode(e.mode);
            header.set_cksum();
            builder
                .append(&header, &[][..])
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        } else {
            header.set_size(e.data.len() as u64);
            header.set_mode(e.mode);
            header.set_cksum();
            builder
                .append(&header, e.data.as_slice())
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        }
    }
    builder
        .finish()
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    fs::rename(&tmp, archive_path)?;
    Ok(())
}

fn tar_gz_list(
    archive_path: &Path,
    path_inside: &str,
    show_hidden: bool,
    sort_mode: SortMode,
    dirs_first: bool,
) -> io::Result<Vec<FileInfo>> {
    let prefix = path_inside.trim_end_matches('/');
    let prefix_with_slash = if prefix.is_empty() {
        String::new()
    } else {
        format!("{}/", prefix)
    };

    let file = fs::File::open(archive_path)?;
    let dec = GzDecoder::new(file);
    let mut archive = Archive::new(dec);

    let mut map: HashMap<String, FileInfo> = HashMap::new();

    for entry in archive
        .entries()
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?
    {
        let entry = entry.map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let path = entry
            .path()
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let name = normalize_tar_path(&path.to_string_lossy());
        let name_clean = name.trim_end_matches('/');
        if name_clean.is_empty() {
            continue;
        }
        let is_entry_dir = entry.header().entry_type().is_dir() || name.ends_with('/');
        let (display_name, is_dir) = if prefix.is_empty() {
            if name_clean.contains('/') {
                let first = name_clean.split('/').next().unwrap_or("");
                if first.is_empty() {
                    continue;
                }
                (first.to_string(), true)
            } else {
                (name_clean.to_string(), is_entry_dir)
            }
        } else {
            if !name_clean.starts_with(&prefix_with_slash) {
                continue;
            }
            let rest = name_clean[prefix_with_slash.len()..].trim_end_matches('/');
            if rest.is_empty() {
                continue;
            }
            if rest.contains('/') {
                let first = rest.split('/').next().unwrap_or("");
                (first.to_string(), true)
            } else {
                (rest.to_string(), is_entry_dir)
            }
        };
        if !show_hidden && display_name.starts_with('.') {
            continue;
        }
        let key = display_name.to_string();
        if map.contains_key(&key) {
            continue;
        }
        let size = entry.size();
        let mtime = None;
        let is_symlink = entry.header().entry_type() == EntryType::Symlink;
        let listing_mode = if is_dir && !is_entry_dir && name_clean.contains('/') {
            None
        } else {
            entry.header().mode().ok()
        };
        let (permissions, is_executable) =
            archive_entry_listing_fields(listing_mode, is_dir, is_symlink);
        map.insert(
            key,
            FileInfo::with_metadata(
                display_name,
                is_dir,
                is_executable,
                is_symlink,
                size,
                mtime,
                permissions,
                String::new(),
                String::new(),
            ),
        );
    }

    let mut files: Vec<FileInfo> = map.into_values().collect();
    if !prefix.is_empty() || archive_path.parent().is_some() {
        let mut parent = FileInfo::new("..".to_string(), true, false);
        #[cfg(unix)]
        {
            parent.permissions = "drwxr-xr-x".to_string();
        }
        files.push(parent);
    }
    apply_sort_mode(&mut files, sort_mode, dirs_first);
    Ok(files)
}

fn tar_gz_read_file(
    archive_path: &Path,
    path_inside: &str,
    name: &str,
) -> io::Result<Vec<u8>> {
    let prefix = path_inside.trim_end_matches('/');
    let full_name = if prefix.is_empty() {
        name.trim_start_matches('/').to_string()
    } else {
        format!(
            "{}/{}",
            prefix,
            name.trim_start_matches('/').trim_end_matches('/')
        )
    };

    let file = fs::File::open(archive_path)?;
    let dec = GzDecoder::new(file);
    let mut archive = Archive::new(dec);

    for entry in archive
        .entries()
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?
    {
        let mut entry = entry.map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let path = entry
            .path()
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let ename = normalize_tar_path(&path.to_string_lossy());
        let ename_clean = ename.trim_end_matches('/').to_string();
        if ename_clean == full_name
            && !entry.header().entry_type().is_dir()
            && !ename.ends_with('/')
        {
            let mut buf = Vec::with_capacity(entry.size() as usize);
            io::copy(&mut entry, &mut buf)?;
            return Ok(buf);
        }
    }

    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "path not found in tar.gz",
    ))
}

fn tar_gz_extract_items(
    archive_path: &Path,
    path_inside: &str,
    items: &[(String, bool)],
    target_dir: &Path,
    dest_names: Option<&[String]>,
) -> io::Result<()> {
    let prefix = path_inside.trim_end_matches('/');
    struct Spec {
        dest_root: String,
        entry_path: String,
        dir_prefix: String,
        is_dir: bool,
    }
    let specs: Vec<Spec> = items
        .iter()
        .enumerate()
        .map(|(i, (name, is_dir))| {
            let name_clean = name.trim_end_matches('/');
            let dest_root = dest_names
                .and_then(|d| d.get(i))
                .map(|s| s.trim_end_matches('/'))
                .unwrap_or(name_clean)
                .to_string();
            let entry_path = archive_child_path(prefix, name_clean);
            let dir_prefix = if *is_dir {
                format!("{}/", entry_path.trim_end_matches('/'))
            } else {
                String::new()
            };
            Spec {
                dest_root,
                entry_path,
                dir_prefix,
                is_dir: *is_dir,
            }
        })
        .collect();

    let file = fs::File::open(archive_path)?;
    let dec = GzDecoder::new(file);
    let mut archive = Archive::new(dec);
    for entry in archive
        .entries()
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?
    {
        let mut entry = entry.map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let path = entry
            .path()
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let ename = normalize_tar_path(&path.to_string_lossy());
        let ename_clean = ename.trim_end_matches('/');
        let is_header_dir = entry.header().entry_type().is_dir();
        let spec_idx = specs.iter().position(|s| {
            if s.is_dir {
                ename_clean == s.entry_path
                    || ename == s.dir_prefix
                    || ename.starts_with(&s.dir_prefix)
            } else {
                ename_clean == s.entry_path && !is_header_dir
            }
        });
        let Some(spec) = spec_idx.map(|i| &specs[i]) else {
            continue;
        };
        if spec.is_dir {
            let rel = if ename_clean == spec.entry_path {
                String::new()
            } else {
                ename[spec.entry_path.len().min(ename.len())..]
                    .trim_start_matches('/')
                    .to_string()
            };
            let is_entry_dir = is_header_dir || ename.ends_with('/');
            let dest = target_dir
                .join(&spec.dest_root)
                .join(rel.trim_end_matches('/'));
            let mode = entry.header().mode().ok();
            if is_entry_dir {
                fs::create_dir_all(&dest).ok();
                let _ = apply_archive_unix_mode(&dest, mode);
            } else {
                if let Some(parent) = dest.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                let mut data = Vec::new();
                if io::copy(&mut entry, &mut data).is_ok() {
                    if fs::write(&dest, data).is_ok() {
                        let _ = apply_archive_unix_mode(&dest, mode);
                    }
                }
            }
        } else {
            let mode = entry.header().mode().ok();
            let dest = target_dir.join(&spec.dest_root);
            let mut out = fs::File::create(&dest)?;
            io::copy(&mut entry, &mut out)?;
            drop(out);
            let _ = apply_archive_unix_mode(&dest, mode);
        }
    }
    for spec in &specs {
        if spec.is_dir {
            fs::create_dir_all(target_dir.join(&spec.dest_root))?;
        }
    }
    Ok(())
}

fn tar_gz_list_all_under(
    archive_path: &Path,
    path_inside: &str,
    dir_name: &str,
) -> io::Result<Vec<(String, bool)>> {
    let prefix_trim = path_inside.trim_end_matches('/');
    let dir_trim = dir_name.trim_end_matches('/');
    let prefix = if prefix_trim.is_empty() {
        format!("{}/", dir_trim)
    } else {
        format!("{}/{}/", prefix_trim, dir_trim)
    };
    let file = fs::File::open(archive_path)?;
    let dec = GzDecoder::new(file);
    let mut archive = Archive::new(dec);
    let mut out = Vec::new();
    for entry in archive
        .entries()
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?
    {
        let entry = entry.map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let path = entry
            .path()
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let name = normalize_tar_path(&path.to_string_lossy());
        if !name.starts_with(&prefix) || name == prefix {
            continue;
        }
        let relative = name[prefix.len()..].trim_end_matches('/').to_string();
        if relative.is_empty() {
            continue;
        }
        out.push((relative, name.ends_with('/')));
    }
    Ok(out)
}

fn tar_gz_add_items(
    archive_path: &Path,
    path_inside: &str,
    source: &PanelLocation,
    items: &[(String, bool)],
    dest_roots: Option<&[String]>,
) -> io::Result<()> {
    let target_prefix = path_inside.trim_end_matches('/');
    let target_prefix_slash = if target_prefix.is_empty() {
        String::new()
    } else {
        format!("{}/", target_prefix)
    };

    let mut to_add: HashSet<String> = HashSet::new();
    for (i, (name, is_dir)) in items.iter().enumerate() {
        let name_clean = name.trim_end_matches('/');
        let dst_root = dest_roots
            .and_then(|d| d.get(i))
            .map(|s| s.trim_end_matches('/'))
            .unwrap_or(name_clean);
        if *is_dir {
            if let Ok(entries) = list_all_under(source, name_clean) {
                if target_prefix_slash.is_empty() {
                    to_add.insert(format!("{}/", dst_root));
                } else {
                    to_add.insert(format!(
                        "{}{}/",
                        target_prefix_slash, dst_root
                    ));
                }
                for (rel, rel_is_dir) in entries {
                    let arc_name = if target_prefix_slash.is_empty() {
                        format!("{}/{}", dst_root, rel)
                    } else {
                        format!(
                            "{}{}/{}",
                            target_prefix_slash, dst_root, rel
                        )
                    };
                    let arc_name = if rel_is_dir {
                        format!("{}/", arc_name.trim_end_matches('/'))
                    } else {
                        arc_name
                    };
                    to_add.insert(arc_name);
                }
            }
        } else {
            let arc_name = if target_prefix_slash.is_empty() {
                dst_root.to_string()
            } else {
                format!("{}{}", target_prefix_slash, dst_root)
            };
            to_add.insert(arc_name);
        }
    }

    let mut entries = tar_gz_read_all_entries(archive_path)?;
    entries.retain(|e| {
        if e.is_dir {
            return false;
        }
        let name = &e.path;
        !(to_add.contains(name) || to_add.contains(name.trim_end_matches('/')))
    });

    let tar_mode_for_fs_under = |rel: &str, is_dir: bool| -> u32 {
        match source {
            PanelLocation::Fs(p) => {
                let joined = FileOperations::join_path(p, rel.trim_start_matches('/'));
                tar_mode_from_path(&joined).unwrap_or(if is_dir { 0o755 } else { 0o644 })
            }
            PanelLocation::Archive { .. } => {
                if is_dir {
                    0o755
                } else {
                    0o644
                }
            }
        }
    };

    for (i, (name, is_dir)) in items.iter().enumerate() {
        let name_clean = name.trim_end_matches('/');
        let dst_root = dest_roots
            .and_then(|d| d.get(i))
            .map(|s| s.trim_end_matches('/'))
            .unwrap_or(name_clean);
        if *is_dir {
            let sub = list_all_under(source, name_clean)?;
            let _dir_prefix = if target_prefix_slash.is_empty() {
                format!("{}/", dst_root)
            } else {
                format!("{}{}/", target_prefix_slash, dst_root)
            };
            for (rel, rel_is_dir) in sub {
                let path = if target_prefix_slash.is_empty() {
                    format!("{}/{}", dst_root, rel)
                } else {
                    format!(
                        "{}{}/{}",
                        target_prefix_slash, dst_root, rel
                    )
                };
                if rel_is_dir {
                    let fs_rel = format!(
                        "{}/{}",
                        name_clean,
                        rel.trim_end_matches('/')
                    );
                    entries.push(TarEntry {
                        path: format!("{}/", path.trim_end_matches('/')),
                        data: Vec::new(),
                        is_dir: true,
                        mode: tar_mode_for_fs_under(&fs_rel, true),
                    });
                } else {
                    let rel_path = format!("{}/{}", name_clean, rel);
                    let data = read_file(source, &rel_path)?;
                    entries.push(TarEntry {
                        path,
                        data,
                        is_dir: false,
                        mode: tar_mode_for_fs_under(&rel_path, false),
                    });
                }
            }
        } else {
            let data = read_file(source, name_clean)?;
            let path = if target_prefix_slash.is_empty() {
                dst_root.to_string()
            } else {
                format!("{}{}", target_prefix_slash, dst_root)
            };
            entries.push(TarEntry {
                path,
                data,
                is_dir: false,
                mode: tar_mode_for_fs_under(name_clean, false),
            });
        }
    }

    write_tar_gz_entries(archive_path, &entries)
}

fn tar_gz_remove_items(
    archive_path: &Path,
    path_inside: &str,
    items: &[(String, bool)],
) -> io::Result<()> {
    let prefix = path_inside.trim_end_matches('/');

    let to_remove: HashSet<String> = items
        .iter()
        .map(|(name, is_dir)| {
            let trimmed_name = name.trim_end_matches('/');
            if *is_dir {
                if prefix.is_empty() {
                    format!("{}/", trimmed_name)
                } else {
                    format!("{}/{}", prefix, trimmed_name)
                }
            } else if prefix.is_empty() {
                trimmed_name.to_string()
            } else {
                format!("{}/{}", prefix, trimmed_name)
            }
        })
        .chain(items.iter().map(|(name, is_dir)| {
            let trimmed_name = name.trim_end_matches('/');
            if *is_dir {
                if prefix.is_empty() {
                    trimmed_name.to_string()
                } else {
                    format!("{}/{}", prefix, trimmed_name)
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

    let mut entries = tar_gz_read_all_entries(archive_path)?;
    entries.retain(|e| !should_remove(&e.path));
    write_tar_gz_entries(archive_path, &entries)
}

fn tar_gz_mkdir(
    archive_path: &Path,
    path_inside: &str,
    name: &str,
) -> io::Result<()> {
    let prefix = path_inside.trim_end_matches('/');
    let name_clean = name.trim_end_matches('/');
    let dir_path = if prefix.is_empty() {
        format!("{}/", name_clean)
    } else {
        format!("{}/{}/", prefix, name_clean)
    };

    let mut entries = tar_gz_read_all_entries(archive_path)?;
    if entries
        .iter()
        .any(|e| e.path == dir_path || e.path.trim_end_matches('/') == name_clean)
    {
        return Ok(());
    }
    entries.push(TarEntry {
        path: dir_path,
        data: Vec::new(),
        is_dir: true,
        mode: 0o755,
    });
    write_tar_gz_entries(archive_path, &entries)
}

fn tar_gz_write_file(
    archive_path: &Path,
    path_inside: &str,
    name: &str,
    content: &[u8],
) -> io::Result<()> {
    let prefix = path_inside.trim_end_matches('/');
    let name_clean = name.trim_end_matches('/');
    let entry_name = if prefix.is_empty() {
        name_clean.to_string()
    } else {
        format!("{}/{}", prefix, name_clean)
    };

    let mut entries = tar_gz_read_all_entries(archive_path)?;
    entries.retain(|e| {
        let p = e.path.trim_end_matches('/');
        p != entry_name && !p.starts_with(&format!("{}/", entry_name))
    });
    entries.push(TarEntry {
        path: entry_name,
        data: content.to_vec(),
        is_dir: false,
        mode: 0o644,
    });
    write_tar_gz_entries(archive_path, &entries)
}
