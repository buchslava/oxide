//! Linux: move files into the user’s Freedesktop home trash (`XDG_DATA_HOME/Trash` or
//! `~/.local/share/Trash`) without calling `libc::getmntent` / mount enumeration.
//!
//! The `trash` crate’s Linux backend uses `getmntent`, which is not reliably safe under
//! concurrency and has been linked to crashes on some Linux builds (including musl/static).
//! This module reimplements the **home trash** branch of the Freedesktop spec: same on-disk
//! layout as GNOME/KDE for the usual case where deleted files live on the same filesystem as
//! `$HOME`. Cross-mountpoint deletes fall back to copy+remove (same idea as `trash`).

use std::borrow::Cow;
use std::ffi::{OsStr, OsString};
use std::fs::{self, OpenOptions};
use std::io::{self, ErrorKind, Write};
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf};

use chrono::Local;
use urlencoding::encode_binary;

type FsError = (PathBuf, io::Error);

fn io_other(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::Other, msg.into())
}

/// `Path=` value for `.trashinfo` (`file:///…`).
fn file_uri_from_absolute_path(abs: &Path) -> io::Result<String> {
    let mut out = String::from("file://");
    for component in abs.components() {
        match component {
            Component::Normal(part) => {
                out.push_str(&encode_binary(part.as_bytes()).to_string());
            }
            _ => {
                let s = component.as_os_str().to_str().ok_or_else(|| {
                    io_other("non-UTF-8 path component in trash URI")
                })?;
                out.push_str(s);
            }
        }
    }
    Ok(out)
}

fn try_creating_placeholders(
    src: &Path,
    dst: &Path,
) -> Result<(), FsError> {
    let metadata = src.symlink_metadata().map_err(|e| (src.to_path_buf(), e))?;
    if metadata.is_dir() {
        fs::create_dir(dst).map_err(|e| (dst.to_path_buf(), e))?;
    } else {
        OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(dst)
            .map_err(|e| (dst.to_path_buf(), e))?;
    }
    Ok(())
}

fn copy_dir_all(
    src: &Path,
    dst: &Path,
) -> Result<(), FsError> {
    fs::create_dir_all(dst).map_err(|e| (dst.to_path_buf(), e))?;
    for entry in fs::read_dir(src).map_err(|e| (src.to_path_buf(), e))? {
        let entry = entry.map_err(|e| (src.to_path_buf(), e))?;
        let file_type = entry.file_type().map_err(|e| (entry.path(), e))?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_all(&src_path, &dst_path)?;
        } else if file_type.is_symlink() {
            let target = fs::read_link(&src_path).map_err(|e| (src_path.clone(), e))?;
            std::os::unix::fs::symlink(&target, &dst_path).map_err(|e| (dst_path.clone(), e))?;
        } else {
            fs::copy(&src_path, &dst_path).map_err(|e| (src_path.clone(), e))?;
        }
    }
    Ok(())
}

fn move_items_no_replace(
    src: &Path,
    dst: &Path,
) -> Result<(), FsError> {
    try_creating_placeholders(src, dst)?;
    match fs::rename(src, dst) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == ErrorKind::CrossesDevices => {
            if src.is_dir() {
                copy_dir_all(src, dst)?;
            } else {
                fs::copy(src, dst).map_err(|e| (src.to_path_buf(), e))?;
            }
            if src.is_dir() {
                fs::remove_dir_all(src).map_err(|e| (src.to_path_buf(), e))?;
            } else {
                fs::remove_file(src).map_err(|e| (src.to_path_buf(), e))?;
            }
            Ok(())
        }
        Err(e) => Err((src.to_path_buf(), e)),
    }
}

fn move_into_home_trash_inner(
    src: &Path,
    trash_folder: &Path,
) -> Result<(), FsError> {
    let files_folder = trash_folder.join("files");
    let info_folder = trash_folder.join("info");
    fs::create_dir_all(&files_folder).map_err(|e| (files_folder.clone(), e))?;
    fs::create_dir_all(&info_folder).map_err(|e| (info_folder.clone(), e))?;

    let filename = src.file_name().ok_or_else(|| {
        (
            src.to_path_buf(),
            io_other("path has no file name"),
        )
    })?;

    let mut appendage = 0usize;
    loop {
        appendage += 1;
        let in_trash_name: Cow<'_, OsStr> = if appendage > 1 {
            let mut trash_name = filename.to_os_string();
            trash_name.push(format!(".{appendage}"));
            trash_name.into()
        } else {
            filename.into()
        };

        let mut info_name = OsString::with_capacity(in_trash_name.len() + 10);
        info_name.push(&*in_trash_name);
        info_name.push(".trashinfo");
        let info_file_path = info_folder.join(&info_name);

        let info_result = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&info_file_path);

        match info_result {
            Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
            Err(error) => return Err((info_file_path, error)),
            Ok(mut file) => {
                let absolute_uri =
                    file_uri_from_absolute_path(src).map_err(|e| (info_file_path.clone(), e))?;
                writeln!(file, "[Trash Info]")
                    .and_then(|_| writeln!(file, "Path={absolute_uri}"))
                    .and_then(|_| {
                        writeln!(
                            file,
                            "DeletionDate={}",
                            Local::now().format("%Y-%m-%dT%H:%M:%S")
                        )
                    })
                    .map_err(|e| (info_file_path.clone(), e))?;
            }
        }

        let dst_path = files_folder.join(Path::new(&*in_trash_name));
        match move_items_no_replace(src, &dst_path) {
            Ok(()) => break,
            Err((_, error)) if error.kind() == ErrorKind::AlreadyExists => {
                let _ = fs::remove_file(&info_file_path);
                continue;
            }
            Err((path, error)) => {
                let _ = fs::remove_file(&info_file_path);
                return Err((path, error));
            }
        }
    }
    Ok(())
}

/// Move `path` into the user’s Freedesktop **home** trash (no `getmntent`).
pub fn move_to_home_trash(path: &Path) -> io::Result<()> {
    let Some(data_local) = dirs::data_local_dir() else {
        return Err(io_other(
            "no XDG data directory (dirs::data_local_dir)",
        ));
    };
    let trash_folder = data_local.join("Trash");
    let abs = fs::canonicalize(path)?;
    move_into_home_trash_inner(&abs, &trash_folder).map_err(|(p, e)| {
        io::Error::new(
            e.kind(),
            format!("trash move at {}: {e}", p.display()),
        )
    })
}
