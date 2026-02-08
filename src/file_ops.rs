use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct FileInfo {
    pub name: String,
    pub is_dir: bool,
}

impl FileInfo {
    pub fn new(name: String, is_dir: bool) -> Self {
        Self { name, is_dir }
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
            files.push(FileInfo::new("..".to_string(), true));
        }

        // Read current directory
        let entries = fs::read_dir(path_ref)?;
        for entry in entries {
            let entry = entry?;
            let file_name = entry.file_name().to_string_lossy().to_string();
            let metadata = entry.metadata()?;
            let is_dir = metadata.is_dir();

            files.push(FileInfo::new(file_name, is_dir));
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
        let name_clean = name.trim_end_matches('/');
        base.as_ref().join(name_clean)
    }

    pub fn path_exists<P: AsRef<Path>>(path: P) -> bool {
        path.as_ref().exists()
    }

    pub fn is_directory<P: AsRef<Path>>(path: P) -> bool {
        path.as_ref().is_dir()
    }
}
