//! Copy, move, and delete files/directories. Used by F5 Copy, F6 Move, F8 Delete (MC-style).

use std::fs;
use std::io;
use std::path::Path;

use super::file_ops::FileOperations;
use crate::core::trash_delete::move_to_trash;

/// EXDEV: cross-device link not permitted (rename across filesystems).
#[cfg(unix)]
const EXDEV: i32 = 18;
#[cfg(not(unix))]
const EXDEV: i32 = -1;

/// Copy a single file from src to dst. Overwrites if dst exists.
pub fn copy_file<P: AsRef<Path>>(
    src: P,
    dst: P,
) -> io::Result<u64> {
    fs::copy(src.as_ref(), dst.as_ref())
}

/// Copy a directory recursively from src to dst. Creates dst if needed.
pub fn copy_dir_recursive<P: AsRef<Path>>(
    src: P,
    dst: P,
) -> io::Result<()> {
    let src = src.as_ref();
    let dst = dst.as_ref();
    fs::create_dir_all(dst)?;
    #[cfg(unix)]
    {
        let perm = fs::metadata(src)?.permissions();
        fs::set_permissions(dst, perm)?;
    }
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let name = entry.file_name();
        let src_path = src.join(&name);
        let dst_path = dst.join(&name);
        if ty.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

/// Copy from `source_dir/src_name` to `target_dir/dst_name` (file or directory tree).
/// Use the same string for `src_name` and `dst_name` when the basename is unchanged.
pub fn copy_item_as<P: AsRef<Path>>(
    source_dir: P,
    target_dir: P,
    src_name: &str,
    dst_name: &str,
    is_dir: bool,
) -> io::Result<()> {
    let src = FileOperations::join_path(source_dir, src_name.trim_end_matches('/'));
    let dst = FileOperations::join_path(target_dir, dst_name.trim_end_matches('/'));
    if is_dir {
        copy_dir_recursive(&src, &dst)
    } else {
        copy_file(&src, &dst).map(|_| ())
    }
}

/// Move from `source_dir/src_name` to `target_dir/dst_name`.
/// Uses rename when possible; on EXDEV (cross-filesystem) copies then removes source.
pub fn move_item_as<P: AsRef<Path>>(
    source_dir: P,
    target_dir: P,
    src_name: &str,
    dst_name: &str,
    is_dir: bool,
) -> io::Result<()> {
    let src = FileOperations::join_path(
        &source_dir,
        src_name.trim_end_matches('/'),
    );
    let dst = FileOperations::join_path(
        &target_dir,
        dst_name.trim_end_matches('/'),
    );
    match fs::rename(&src, &dst) {
        Ok(()) => Ok(()),
        Err(e) if e.raw_os_error() == Some(EXDEV) => {
            copy_item_as(
                &source_dir,
                &target_dir,
                src_name,
                dst_name,
                is_dir,
            )?;
            if is_dir {
                fs::remove_dir_all(&src)
            } else {
                fs::remove_file(&src)
            }
        }
        Err(e) => Err(e),
    }
}

/// Delete one item (file or directory) at source_dir/name.
/// is_dir: true = remove directory and contents recursively.
/// When `use_trash` is true, moves to the OS trash instead of unlinking (see F9 Safe delete).
pub fn delete_item<P: AsRef<Path>>(
    source_dir: P,
    name: &str,
    is_dir: bool,
    use_trash: bool,
) -> io::Result<()> {
    let path = FileOperations::join_path(source_dir, name);
    if use_trash {
        return move_to_trash(&path);
    }
    if is_dir {
        fs::remove_dir_all(&path)
    } else {
        fs::remove_file(&path)
    }
}
