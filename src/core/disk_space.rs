//! Free/total disk space for a path (used by the Size result dialog).

use crate::core::text_format::format_disk_bytes;

/// Format disk space for `path` as `"12.5 GB / 466 GB (2%)"` (scaled SI units). Unix only; otherwise "-".
pub fn disk_space_summary(path: &str) -> String {
    #[cfg(unix)]
    {
        use nix::sys::statvfs::statvfs;
        if let Ok(st) = statvfs(path) {
            // `blocks()` / `blocks_free()` are measured in **fragment_size** (`f_frsize`) units, not
            // `block_size()` (`f_bsize`). Using `f_bsize` here inflates totals (e.g. ~256× on APFS).
            let fr = st.fragment_size() as u64;
            if fr == 0 {
                return "-".to_string();
            }
            let total = (st.blocks() as u64).saturating_mul(fr);
            let free = (st.blocks_free() as u64).saturating_mul(fr);
            let used = total.saturating_sub(free);
            let pct = if total > 0 { (used * 100) / total } else { 0 };
            return format!(
                "{} / {} ({}%)",
                format_disk_bytes(used),
                format_disk_bytes(total),
                pct
            );
        }
    }
    #[allow(unused_variables)]
    let _ = path;
    "-".to_string()
}
