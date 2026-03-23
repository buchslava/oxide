//! OS trash integration: macOS Finder Trash, Linux Freedesktop `~/.local/share/Trash` (via `trash` crate).

use std::io;
use std::path::Path;

/// True when this build/OS supports moving deleted files into the system trash (not permanent delete).
pub fn trash_available() -> bool {
    #[cfg(target_os = "macos")]
    {
        true
    }
    #[cfg(target_os = "linux")]
    {
        linux_trash_available()
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        false
    }
}

#[cfg(target_os = "linux")]
fn linux_trash_available() -> bool {
    use std::fs;

    let Some(base) = dirs::data_local_dir().or_else(|| dirs::home_dir().map(|h| h.join(".local/share")))
    else {
        return false;
    };
    if !base.exists() {
        if fs::create_dir_all(&base).is_err() {
            return false;
        }
    }
    nix::unistd::access(&base, nix::unistd::AccessFlags::W_OK).is_ok()
}

/// Move `path` to the OS trash. File or directory.
pub fn move_to_trash(path: &Path) -> io::Result<()> {
    trash::delete(path).map_err(|e| io::Error::new(io::ErrorKind::Other, e))
}
