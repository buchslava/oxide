//! String truncation and human-readable byte counts (no terminal dependencies).

/// How to truncate a string that exceeds the max width.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TruncateMode {
    /// Keep the start, add ellipsis at end: "some long…"
    PrefixEllipsis,
    /// Keep the end, add ellipsis at start: "…long path"
    SuffixEllipsis,
    /// Keep start and end with ~ in the middle (MC-style path): "first~last"
    CompactMiddle,
}

/// Truncate string to max_width chars according to mode. Returns the full string if it fits.
pub fn truncate_str(
    s: &str,
    max_width: usize,
    mode: TruncateMode,
) -> String {
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    if n <= max_width {
        return s.to_string();
    }
    match mode {
        TruncateMode::PrefixEllipsis => {
            if max_width < 2 {
                return s.to_string();
            }
            format!(
                "{}…",
                chars
                    .iter()
                    .take(max_width.saturating_sub(1))
                    .collect::<String>()
            )
        }
        TruncateMode::SuffixEllipsis => {
            if max_width < 2 {
                return s.to_string();
            }
            let take = max_width.saturating_sub(1);
            let start = n.saturating_sub(take);
            format!("…{}", chars.iter().skip(start).collect::<String>())
        }
        TruncateMode::CompactMiddle => {
            if max_width < 2 {
                return s.to_string();
            }
            let half = (max_width - 1) / 2;
            let suffix_len = (max_width - 1).saturating_sub(half);
            let start: String = chars.iter().take(half).collect();
            let end: String = chars
                .iter()
                .rev()
                .take(suffix_len)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            format!("{}~{}", start, end)
        }
    }
}

/// Format byte count with fixed-width unit so "B" aligns under "B" in "KB".
/// Units: "  B" (bytes), " KB", " MB", " GB" — all 3 chars.
pub fn format_byte_size(bytes: u64) -> String {
    const KB: u64 = 1_000;
    const MB: u64 = 1_000_000;
    const GB: u64 = 1_000_000_000;
    const W: usize = 5;
    if bytes < KB {
        format!("{:>w$}  B", bytes, w = W)
    } else if bytes < MB {
        let whole = bytes / KB;
        let frac = (bytes % KB) * 10 / KB;
        let num = if frac == 0 {
            format!("{}", whole)
        } else {
            format!("{}.{}", whole, frac)
        };
        format!("{:>w$} KB", num, w = W)
    } else if bytes < GB {
        let whole = bytes / MB;
        let frac = (bytes % MB) * 10 / MB;
        let num = if frac == 0 {
            format!("{}", whole)
        } else {
            format!("{}.{}", whole, frac)
        };
        format!("{:>w$} MB", num, w = W)
    } else {
        let whole = bytes / GB;
        let frac = (bytes % GB) * 10 / GB;
        let num = if frac == 0 {
            format!("{}", whole)
        } else {
            format!("{}.{}", whole, frac)
        };
        format!("{:>w$} GB", num, w = W)
    }
}
