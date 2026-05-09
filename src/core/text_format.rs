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
    let char_count = chars.len();
    if char_count <= max_width {
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
            let start = char_count.saturating_sub(take);
            format!(
                "…{}",
                chars.iter().skip(start).collect::<String>()
            )
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

/// Integer with ASCII thousands separators (e.g. `36700395` → `"36,700,395"`).
pub fn format_u64_with_commas(n: u64) -> String {
    let s = n.to_string();
    let len = s.len();
    let mut out = String::with_capacity(len + len / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (len - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Capacity for disk/volume labels: decimal SI (KB–PB), one fractional digit when non-zero.
/// Example: `1_234_000_000_000` → `"1.2 TB"`; small values use `B` with thousands separators.
pub fn format_disk_bytes(bytes: u64) -> String {
    const KB: u64 = 1_000;
    const MB: u64 = KB * 1_000;
    const GB: u64 = MB * 1_000;
    const TB: u64 = GB * 1_000;
    const PB: u64 = TB * 1_000;

    fn scaled(
        bytes: u64,
        unit: u64,
        suffix: &str,
    ) -> String {
        let whole = bytes / unit;
        let rem = bytes % unit;
        let frac = ((rem * 10) / unit) as u32;
        let whole_s = format_u64_with_commas(whole);
        if frac == 0 {
            format!("{} {}", whole_s, suffix)
        } else {
            format!("{}.{} {}", whole_s, frac, suffix)
        }
    }

    if bytes < KB {
        format!("{} B", format_u64_with_commas(bytes))
    } else if bytes < MB {
        scaled(bytes, KB, "KB")
    } else if bytes < GB {
        scaled(bytes, MB, "MB")
    } else if bytes < TB {
        scaled(bytes, GB, "GB")
    } else if bytes < PB {
        scaled(bytes, TB, "TB")
    } else {
        scaled(bytes, PB, "PB")
    }
}
