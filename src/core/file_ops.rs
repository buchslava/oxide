use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, PermissionsExt};

#[cfg(unix)]
fn uid_to_owner(uid: u32) -> String {
    unsafe {
        let pw = libc::getpwuid(uid);
        if pw.is_null() {
            return uid.to_string();
        }
        let name = (*pw).pw_name;
        if name.is_null() {
            return uid.to_string();
        }
        std::ffi::CStr::from_ptr(name)
            .to_string_lossy()
            .into_owned()
    }
}

#[cfg(unix)]
fn gid_to_group(gid: u32) -> String {
    unsafe {
        let gr = libc::getgrgid(gid);
        if gr.is_null() {
            return gid.to_string();
        }
        let name = (*gr).gr_name;
        if name.is_null() {
            return gid.to_string();
        }
        std::ffi::CStr::from_ptr(name)
            .to_string_lossy()
            .into_owned()
    }
}

fn is_executable(metadata: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        false
    }
}

#[cfg(unix)]
fn format_permissions(mode: u32) -> String {
    let kind = if (mode & 0o170000) == 0o040000 {
        'd'
    } else if (mode & 0o170000) == 0o120000 {
        'l'
    } else {
        '-'
    };
    let r = if mode & 0o400 != 0 { 'r' } else { '-' };
    let w = if mode & 0o200 != 0 { 'w' } else { '-' };
    let x = if mode & 0o100 != 0 { 'x' } else { '-' };
    let r2 = if mode & 0o40 != 0 { 'r' } else { '-' };
    let w2 = if mode & 0o20 != 0 { 'w' } else { '-' };
    let x2 = if mode & 0o10 != 0 { 'x' } else { '-' };
    let r3 = if mode & 0o4 != 0 { 'r' } else { '-' };
    let w3 = if mode & 0o2 != 0 { 'w' } else { '-' };
    let x3 = if mode & 0o1 != 0 { 'x' } else { '-' };
    format!(
        "{}{}{}{}{}{}{}{}{}{}",
        kind, r, w, x, r2, w2, x2, r3, w3, x3
    )
}

#[cfg(not(unix))]
fn format_permissions(_mode: u32) -> String {
    "----------".to_string()
}

/// Combine permission bits with a file kind when archives store only `0o777` (common in tar).
#[cfg(unix)]
fn mode_bits_for_permission_string(raw: u32, is_dir: bool, is_symlink: bool) -> u32 {
    if (raw & 0o170000) != 0 {
        raw
    } else {
        let perm = raw & 0o7777;
        let kind = if is_symlink {
            0o120000
        } else if is_dir {
            0o040000
        } else {
            0o100000
        };
        kind | perm
    }
}

/// Maps stored Unix mode from a zip/tar entry to list columns (`permissions`, `is_executable`).
pub fn archive_entry_listing_fields(
    unix_mode: Option<u32>,
    is_dir: bool,
    is_symlink: bool,
) -> (String, bool) {
    #[cfg(unix)]
    {
        match unix_mode {
            Some(raw) => {
                let mode = mode_bits_for_permission_string(raw, is_dir, is_symlink);
                let permissions = format_permissions(mode);
                let is_executable =
                    !is_dir && !is_symlink && (mode & 0o111) != 0;
                (permissions, is_executable)
            }
            None => ("----------".to_string(), false),
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (unix_mode, is_dir, is_symlink);
        ("----------".to_string(), false)
    }
}

/// Apply permission bits from an archive entry to a newly created path (`chmod`, Unix only).
pub fn apply_archive_unix_mode(path: &Path, unix_mode: Option<u32>) -> io::Result<()> {
    #[cfg(unix)]
    {
        let Some(raw) = unix_mode else {
            return Ok(());
        };
        let bits = raw & 0o7777;
        if bits == 0 {
            return Ok(());
        }
        fs::set_permissions(path, fs::Permissions::from_mode(bits))
    }
    #[cfg(not(unix))]
    {
        let _ = (path, unix_mode);
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct FileInfo {
    pub name: String,
    pub is_dir: bool,
    pub is_executable: bool,
    pub is_symlink: bool,
    /// File size in bytes (0 for ".." or dirs when not computed).
    pub size: u64,
    /// Modification time (None for ".." or on error).
    pub mtime: Option<std::time::SystemTime>,
    /// Unix-style permissions string, e.g. "-rwxr-xr-x".
    pub permissions: String,
    /// Owner name (Unix) or empty.
    pub owner: String,
    /// Group name (Unix) or empty.
    pub group: String,
}

impl FileInfo {
    pub fn new(
        name: String,
        is_dir: bool,
        is_executable: bool,
    ) -> Self {
        Self {
            name,
            is_dir,
            is_executable,
            is_symlink: false,
            size: 0,
            mtime: None,
            permissions: String::new(),
            owner: String::new(),
            group: String::new(),
        }
    }

    pub fn with_metadata(
        name: String,
        is_dir: bool,
        is_executable: bool,
        is_symlink: bool,
        size: u64,
        mtime: Option<std::time::SystemTime>,
        permissions: String,
        owner: String,
        group: String,
    ) -> Self {
        Self {
            name,
            is_dir,
            is_executable,
            is_symlink,
            size,
            mtime,
            permissions,
            owner,
            group,
        }
    }

    pub fn is_parent_dir(&self) -> bool {
        self.name == ".."
    }

    /// True for `..` false; name starts with `.` (dotfiles shown when hidden files are visible).
    #[inline]
    pub fn is_hidden_dotfile(&self) -> bool {
        !self.is_parent_dir() && self.name.starts_with('.')
    }
}

/// Sort mode strings used in settings.
pub const SORT_MODES: [&str; 6] = [
    "name_asc",
    "name_desc",
    "size_asc",
    "size_desc",
    "mtime_asc",
    "mtime_desc",
];

/// Next entry in [`SORT_MODES`]. Unknown `current` is treated as the first mode.
pub fn cycle_sort_mode(
    current: &str,
    forward: bool,
) -> String {
    let idx = SORT_MODES.iter().position(|s| *s == current).unwrap_or(0);
    let len = SORT_MODES.len();
    let next = if forward {
        (idx + 1) % len
    } else {
        (idx + len - 1) % len
    };
    SORT_MODES[next].to_string()
}

/// Compare two entries by sort_mode (no special ".." handling). Used for ordering within dirs or files.
fn cmp_by_sort_mode(
    a: &FileInfo,
    b: &FileInfo,
    sort_mode: &str,
) -> std::cmp::Ordering {
    match sort_mode {
        "name_asc" => a
            .name
            .trim_end_matches('/')
            .cmp(b.name.trim_end_matches('/')),
        "name_desc" => b
            .name
            .trim_end_matches('/')
            .cmp(a.name.trim_end_matches('/')),
        "size_asc" => a.size.cmp(&b.size).then_with(|| a.name.cmp(&b.name)),
        "size_desc" => b.size.cmp(&a.size).then_with(|| a.name.cmp(&b.name)),
        "mtime_asc" => {
            let ta = a.mtime.unwrap_or(std::time::UNIX_EPOCH);
            let tb = b.mtime.unwrap_or(std::time::UNIX_EPOCH);
            ta.cmp(&tb).then_with(|| a.name.cmp(&b.name))
        }
        "mtime_desc" => {
            let ta = a.mtime.unwrap_or(std::time::UNIX_EPOCH);
            let tb = b.mtime.unwrap_or(std::time::UNIX_EPOCH);
            tb.cmp(&ta).then_with(|| a.name.cmp(&b.name))
        }
        _ => a
            .name
            .trim_end_matches('/')
            .cmp(b.name.trim_end_matches('/')),
    }
}

/// Sort file list: ".." always first. If dirs_first then directories next (sorted by sort_mode), then files (sorted by sort_mode); else unified by sort_mode.
pub fn apply_sort_mode(
    files: &mut [FileInfo],
    sort_mode: &str,
    dirs_first: bool,
) {
    files.sort_by(|a, b| {
        if a.is_parent_dir() && !b.is_parent_dir() {
            return std::cmp::Ordering::Less;
        }
        if !a.is_parent_dir() && b.is_parent_dir() {
            return std::cmp::Ordering::Greater;
        }
        if dirs_first {
            match (a.is_dir, b.is_dir) {
                (true, false) => return std::cmp::Ordering::Less,
                (false, true) => return std::cmp::Ordering::Greater,
                _ => {}
            }
        }
        cmp_by_sort_mode(a, b, sort_mode)
    });
}

pub struct FileOperations;

impl FileOperations {
    /// Read directory contents. When show_hidden is false, entries starting with "." are excluded.
    /// sort_mode: name_asc, name_desc, size_asc, size_desc, mtime_asc, mtime_desc.
    /// dirs_first: when true, directories appear before files; when false, unified sort.
    pub fn read_directory<P: AsRef<Path>>(
        path: P,
        show_hidden: bool,
        sort_mode: &str,
        dirs_first: bool,
    ) -> io::Result<Vec<FileInfo>> {
        let mut files = Vec::new();
        let path_ref = path.as_ref();

        // Add parent directory entry if not at root
        if path_ref.parent().is_some() {
            let mut parent = FileInfo::new("..".to_string(), true, false);
            parent.permissions = "drwxr-xr-x".to_string();
            files.push(parent);
        }

        // Read current directory
        let entries = fs::read_dir(path_ref)?;
        for entry in entries {
            let entry = entry?;
            let file_name = entry.file_name().to_string_lossy().to_string();
            if !show_hidden && file_name.starts_with('.') {
                continue;
            }
            let metadata = entry.metadata()?;
            let file_type = entry.file_type()?;
            let is_symlink = file_type.is_symlink();
            let is_dir = metadata.is_dir();
            let is_executable = is_executable(&metadata);
            let size = metadata.len();
            let mtime = metadata.modified().ok();
            let (permissions, owner, group) = {
                #[cfg(unix)]
                {
                    let mode = metadata.permissions().mode();
                    let uid = metadata.uid();
                    let gid = metadata.gid();
                    (
                        format_permissions(mode),
                        uid_to_owner(uid),
                        gid_to_group(gid),
                    )
                }
                #[cfg(not(unix))]
                {
                    (
                        "----------".to_string(),
                        String::new(),
                        String::new(),
                    )
                }
            };

            files.push(FileInfo::with_metadata(
                file_name,
                is_dir,
                is_executable,
                is_symlink,
                size,
                mtime,
                permissions,
                owner,
                group,
            ));
        }

        apply_sort_mode(&mut files, sort_mode, dirs_first);

        Ok(files)
    }

    pub fn join_path<P: AsRef<Path>>(
        base: P,
        name: &str,
    ) -> PathBuf {
        let name_clean = name.trim_start_matches('/').trim_end_matches('/');
        base.as_ref().join(name_clean)
    }

    /// Get file mode (0o7777: suid, sgid, sticky + rwx for owner/group/other). Unix only.
    #[cfg(unix)]
    pub fn get_file_mode<P: AsRef<Path>>(path: P) -> io::Result<u32> {
        use std::os::unix::fs::PermissionsExt;
        let meta = fs::metadata(path)?;
        Ok(meta.permissions().mode() & 0o7777)
    }

    #[cfg(not(unix))]
    pub fn get_file_mode<P: AsRef<Path>>(_path: P) -> io::Result<u32> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "get_file_mode not supported",
        ))
    }

    /// Set file permissions (chmod). On Unix, preserves file type bits (0o170000) and sets 0o7777 (suid, sgid, sticky + rwx).
    #[cfg(unix)]
    pub fn set_permissions<P: AsRef<Path>>(
        path: P,
        mode_bits: u32,
    ) -> io::Result<()> {
        use std::os::unix::fs::PermissionsExt;
        let path = path.as_ref();
        let meta = fs::metadata(path)?;
        let current = meta.permissions().mode();
        let new_mode = (current & 0o170000) | (mode_bits & 0o7777);
        fs::set_permissions(path, fs::Permissions::from_mode(new_mode))
    }

    #[cfg(not(unix))]
    pub fn set_permissions<P: AsRef<Path>>(
        _path: P,
        _mode_bits: u32,
    ) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "chmod not supported",
        ))
    }

    /// Load list of user names from /etc/passwd (Unix). First field of each line.
    #[cfg(unix)]
    pub fn load_user_list() -> Vec<String> {
        let mut out = Vec::new();
        if let Ok(s) = fs::read_to_string("/etc/passwd") {
            for line in s.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                if let Some(name) = line.split(':').next() {
                    out.push(name.to_string());
                }
            }
        }
        out
    }

    #[cfg(not(unix))]
    pub fn load_user_list() -> Vec<String> {
        Vec::new()
    }

    /// Load list of group names from /etc/group (Unix). First field of each line.
    #[cfg(unix)]
    pub fn load_group_list() -> Vec<String> {
        let mut out = Vec::new();
        if let Ok(s) = fs::read_to_string("/etc/group") {
            for line in s.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                if let Some(name) = line.split(':').next() {
                    out.push(name.to_string());
                }
            }
        }
        out
    }

    #[cfg(not(unix))]
    pub fn load_group_list() -> Vec<String> {
        Vec::new()
    }

    /// Change owner and group of a file (chown). Unix only; looks up uid/gid by name.
    #[cfg(unix)]
    pub fn chown<P: AsRef<Path>>(
        path: P,
        user: &str,
        group: &str,
    ) -> io::Result<()> {
        let uid = unsafe {
            let pw = libc::getpwnam(
                std::ffi::CString::new(user)
                    .map_err(|_| {
                        io::Error::new(
                            io::ErrorKind::InvalidInput,
                            "invalid user",
                        )
                    })?
                    .as_ptr(),
            );
            if pw.is_null() {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("user '{}' not found", user),
                ));
            }
            (*pw).pw_uid
        };
        let gid = unsafe {
            let gr = libc::getgrnam(
                std::ffi::CString::new(group)
                    .map_err(|_| {
                        io::Error::new(
                            io::ErrorKind::InvalidInput,
                            "invalid group",
                        )
                    })?
                    .as_ptr(),
            );
            if gr.is_null() {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("group '{}' not found", group),
                ));
            }
            (*gr).gr_gid
        };
        let path = path.as_ref();
        let path_c =
            std::ffi::CString::new(path.as_os_str().as_bytes().to_vec()).map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "path contains null",
                )
            })?;
        if unsafe { libc::chown(path_c.as_ptr(), uid, gid) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    #[cfg(not(unix))]
    pub fn chown<P: AsRef<Path>>(
        _path: P,
        _user: &str,
        _group: &str,
    ) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "chown not supported",
        ))
    }

    /// Compute total size of a path: file size for files; recursively sum for directories.
    /// Symlinks are followed (metadata() resolves to target). Errors (e.g. permission denied) are
    /// skipped and contribute 0 to the total.
    pub fn size_of_path_recursive<P: AsRef<Path>>(path: P) -> u64 {
        let path = path.as_ref();
        let Ok(metadata) = fs::metadata(path) else {
            return 0;
        };
        if metadata.is_file() {
            return metadata.len();
        }
        if !metadata.is_dir() {
            return 0;
        }
        let Ok(entries) = fs::read_dir(path) else {
            return 0;
        };
        let mut total = 0u64;
        for entry in entries.flatten() {
            let entry_path = entry.path();
            total = total.saturating_add(Self::size_of_path_recursive(entry_path));
        }
        total
    }
}
