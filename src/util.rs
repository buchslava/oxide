//! Shared utilities: panel dimensions, path truncation.

use crossterm::terminal::size;

/// Compute visible panel height (rows) for layout and scroll. Terminal height minus
/// command line, menu bar, frame borders, and bottom bar.
pub(crate) fn compute_panel_height() -> usize {
    size()
        .map(|(_, h)| (h as usize).saturating_sub(5).max(1))
        .unwrap_or(18)
}

/// How to truncate a string that exceeds the max width.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TruncateMode {
    /// Keep the start, add ellipsis at end: "some long…"
    PrefixEllipsis,
    /// Keep the end, add ellipsis at start: "…long path"
    SuffixEllipsis,
    /// Keep start and end with ~ in the middle (MC-style path): "first~last"
    CompactMiddle,
}

/// Truncate string to max_width chars according to mode. Returns the full string if it fits.
pub(crate) fn truncate_str(s: &str, max_width: usize, mode: TruncateMode) -> String {
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
                chars.iter().take(max_width.saturating_sub(1)).collect::<String>()
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
