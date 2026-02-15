use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

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

#[derive(Debug, Clone)]
pub struct FileInfo {
    pub name: String,
    pub is_dir: bool,
    pub is_executable: bool,
}

impl FileInfo {
    pub fn new(name: String, is_dir: bool, is_executable: bool) -> Self {
        Self { name, is_dir, is_executable }
    }

    pub fn is_parent_dir(&self) -> bool {
        self.name == ".."
    }
}

pub struct FileOperations;

impl FileOperations {
    pub fn read_directory<P: AsRef<Path>>(path: P) -> io::Result<Vec<FileInfo>> {
        let mut files = Vec::new();
        let path_ref = path.as_ref();

        // Add parent directory entry if not at root
        if path_ref.parent().is_some() {
            files.push(FileInfo::new("..".to_string(), true, false));
        }

        // Read current directory
        let entries = fs::read_dir(path_ref)?;
        for entry in entries {
            let entry = entry?;
            let file_name = entry.file_name().to_string_lossy().to_string();
            let metadata = entry.metadata()?;
            let is_dir = metadata.is_dir();
            let is_executable = is_executable(&metadata);

            files.push(FileInfo::new(file_name, is_dir, is_executable));
        }

        // Sort files: directories first, then files alphabetically
        files.sort_by(|a, b| {
            match (a.is_dir, b.is_dir) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => a.name.cmp(&b.name),
            }
        });

        Ok(files)
    }

    pub fn join_path<P: AsRef<Path>>(base: P, name: &str) -> PathBuf {
        let name_clean = name.trim_start_matches('/').trim_end_matches('/');
        base.as_ref().join(name_clean)
    }

    pub fn path_exists<P: AsRef<Path>>(path: P) -> bool {
        path.as_ref().exists()
    }

    pub fn is_directory<P: AsRef<Path>>(path: P) -> bool {
        path.as_ref().is_dir()
    }
}
