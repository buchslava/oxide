//! Free/total disk space for a path (used by the panel bottom bar).

/// Format disk space for `path` as "12G / 466G (2%)". Unix only; otherwise "-".
pub fn disk_space_summary(path: &str) -> String {
    #[cfg(unix)]
    {
        use nix::sys::statvfs::statvfs;
        if let Ok(st) = statvfs(path) {
            let bsize = st.block_size();
            let total = (st.blocks() as u64).saturating_mul(bsize);
            let free = (st.blocks_free() as u64).saturating_mul(bsize);
            let used = total.saturating_sub(free);
            let total_gb = total / (1024 * 1024 * 1024);
            let used_gb = used / (1024 * 1024 * 1024);
            let pct = if total > 0 { (used * 100) / total } else { 0 };
            return format!("{}G / {}G ({}%)", used_gb, total_gb, pct);
        }
    }
    #[allow(unused_variables)]
    let _ = path;
    "-".to_string()
}
