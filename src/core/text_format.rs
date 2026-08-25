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
    /// Keep start and end with … in the middle: "first…last"
    MiddleEllipsis,
}

/// Truncate string to max_width chars according to mode. Returns the full string if it fits.
pub fn truncate_str(
    s: &str,
    max_width: usize,
    mode: TruncateMode,
) -> String {
    let char_count = s.chars().count();
    if char_count <= max_width {
        return s.to_string();
    }
    let chars: Vec<char> = s.chars().collect();
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
        TruncateMode::MiddleEllipsis => {
            if max_width <= 3 {
                return "…".to_string();
            }
            let keep = max_width - 1;
            let half = keep / 2;
            let start: String = chars.iter().take(half).collect();
            let end: String = chars
                .iter()
                .rev()
                .take(keep - half)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            format!("{}…{}", start, end)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{truncate_str, wrap_line, TruncateMode};

    #[test]
    fn middle_ellipsis_fits_unchanged() {
        assert_eq!(
            truncate_str(
                "short.png",
                20,
                TruncateMode::MiddleEllipsis
            ),
            "short.png"
        );
    }

    #[test]
    fn middle_ellipsis_long_name() {
        let s = "vacation_beach_sunset_final_v2.png";
        let t = truncate_str(s, 24, TruncateMode::MiddleEllipsis);
        assert!(t.chars().count() <= 24);
        assert!(t.contains('…'));
        assert!(t.starts_with("vacation"));
        assert!(t.ends_with(".png"));
    }

    #[test]
    fn middle_ellipsis_tiny_width() {
        assert_eq!(
            truncate_str("abcdef", 3, TruncateMode::MiddleEllipsis),
            "…"
        );
    }

    #[test]
    fn wrap_line_splits_on_char_width() {
        assert_eq!(wrap_line("abcd", 2), vec!["ab", "cd"]);
        assert_eq!(wrap_line("ab", 10), vec!["ab"]);
        assert_eq!(wrap_line("", 4), vec![""]);
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

/// Split `line` into chunks of at most `width` Unicode scalar values.
pub fn wrap_line(
    line: &str,
    width: usize,
) -> Vec<String> {
    if width == 0 {
        return vec![line.to_string()];
    }
    let char_count = line.chars().count();
    if char_count <= width {
        return vec![line.to_string()];
    }
    let mut out = Vec::with_capacity(char_count.div_ceil(width));
    let mut rest = line;
    while !rest.is_empty() {
        let mut chars = 0;
        let mut split_at = rest.len();
        for (i, _) in rest.char_indices() {
            if chars == width {
                split_at = i;
                break;
            }
            chars += 1;
        }
        if split_at == rest.len() {
            out.push(rest.to_string());
            break;
        }
        let (chunk, next) = rest.split_at(split_at);
        out.push(chunk.to_string());
        rest = next;
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}
