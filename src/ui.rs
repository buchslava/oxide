use ratatui::{
    layout::{Alignment, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Gauge, Paragraph},
    Frame,
};
use crate::app_state::{AppState, Focus, Operation};
use crate::editor;
use crate::file_ops::FileInfo;
use crate::viewer;
use crate::panel::{Panel, PanelOperations, ViewMode};
use crate::styles;

/// Dark background for main content area (panels, command line). Ensures consistent look across terminals.
const MAIN_DARK_BG: Color = Color::Rgb(30, 30, 35);

/// Format disk space for current path as "12G / 466G (2%)". Unix only; otherwise "-".
fn disk_space_string(path: &str) -> String {
    #[cfg(unix)]
    {
        use nix::sys::statvfs::statvfs;
        if let Ok(st) = statvfs(path) {
            let bsize = st.block_size() as u64;
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

pub struct Renderer;

/// Truncate string to max_width chars with trailing ellipsis. Returns full string if it fits.
pub fn truncate_str_ellipsis(s: &str, max_width: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max_width || max_width < 2 {
        return s.to_string();
    }
    format!("{}…", chars.iter().take(max_width.saturating_sub(1)).collect::<String>())
}

/// Shorten path to max_width chars like MC: "first_half~last_half" (one ~ in the middle).
fn compact_path(path: &str, max_width: usize) -> String {
    let chars: Vec<char> = path.chars().collect();
    let n = chars.len();
    if n <= max_width || max_width < 2 {
        return path.to_string();
    }
    let half = (max_width - 1) / 2;
    let suffix_len = (max_width - 1).saturating_sub(half);
    let start: String = chars.iter().take(half).collect();
    let end: String = chars.iter().rev().take(suffix_len).collect::<Vec<_>>().into_iter().rev().collect();
    format!("{}~{}", start, end)
}

/// Format byte count with fixed-width unit so "B" aligns under "B" in "KB".
/// Units: "  B" (bytes), " KB", " MB", " GB" — all 3 chars.
pub fn format_size(bytes: u64) -> String {
    const KB: u64 = 1_000;
    const MB: u64 = 1_000_000;
    const GB: u64 = 1_000_000_000;
    const NUM_W: usize = 5;
    if bytes < KB {
        format!("{:>NUM_W$}  B", bytes)
    } else if bytes < MB {
        let whole = bytes / KB;
        let frac = (bytes % KB) * 10 / KB;
        let num = if frac == 0 {
            format!("{}", whole)
        } else {
            format!("{}.{}", whole, frac)
        };
        format!("{:>NUM_W$} KB", num)
    } else if bytes < GB {
        let whole = bytes / MB;
        let frac = (bytes % MB) * 10 / MB;
        let num = if frac == 0 {
            format!("{}", whole)
        } else {
            format!("{}.{}", whole, frac)
        };
        format!("{:>NUM_W$} MB", num)
    } else {
        let whole = bytes / GB;
        let frac = (bytes % GB) * 10 / GB;
        let num = if frac == 0 {
            format!("{}", whole)
        } else {
            format!("{}.{}", whole, frac)
        };
        format!("{:>NUM_W$} GB", num)
    }
}

/// Format mtime as "Feb 13 20:05" (month name, day, time).
fn format_mtime(t: &std::time::SystemTime) -> String {
    use chrono::{DateTime, Utc};
    let datetime: DateTime<Utc> = (*t).into();
    datetime.format("%b %e %H:%M").to_string()
}

/// Size display: "UP--DIR" for ".." (parent), nothing for other dirs, human-readable for files.
fn size_display(file: &FileInfo) -> String {
    if file.is_parent_dir() {
        "UP--DIR".to_string()
    } else if file.is_dir {
        String::new()
    } else {
        format_size(file.size)
    }
}

/// Truncate file display to fit column width (chars). Prevents wrapping/uglification in double-column view.
/// Prefix: "/" for folders, "@" for symlinks, "*" for executables, " " for regular files (as in screenshot).
fn truncate_for_width(file: &FileInfo, max_width: usize) -> String {
    if file.is_parent_dir() {
        return "..".to_string();
    }
    let full = if file.is_dir {
        format!("/{}", file.name)
    } else if file.is_symlink {
        format!("@{}", file.name)
    } else if file.is_executable {
        format!("*{}", file.name)
    } else {
        format!(" {}", file.name)
    };
    let w = max_width.saturating_sub(1); // leave room for "…"
    if full.chars().count() <= max_width {
        full
    } else {
        format!("{}…", full.chars().take(w).collect::<String>())
    }
}

impl Renderer {
    /// MC-style: panels + status + command line; or viewer (F3) or editor (F4) with optional confirm dialog.
    pub fn draw_ui(f: &mut Frame, app: &mut AppState) {
        if app.viewer_screen.is_some() {
            viewer::draw(f, app);
            return;
        }
        if app.editor_screen.is_some() {
            editor::draw(f, app);
            return;
        }
        Self::draw_panels_view(f, app);
        if let Some(ref progress) = app.copy_progress {
            Self::draw_copy_progress(f, progress);
        }
        if let Some(ref filename) = app.copy_overwrite_dialog {
            Self::draw_copy_overwrite_dialog(f, app, filename);
        }
        if let Some(ref err) = app.copy_error_dialog {
            Self::draw_copy_error_dialog(f, app, err);
        }
        if app.operation_confirm_pending.is_some() {
            Self::draw_operation_confirm_dialog(f, app);
        }
        if app.mkdir_dialog.is_some() {
            crate::mkdir_dialog::draw(f, app);
        }
        if app.rename_attr_dialog.is_some() {
            crate::rename_attr::draw(f, app);
        }
        if app.size_info_dialog.is_some() {
            crate::size_info_dialog::draw(f, app);
        }
        if app.settings_dialog.is_some() {
            crate::settings_dialog::draw(f, app);
        }
    }

    /// Operation confirmation dialog (Copy/Move/Delete): operation alert, Yes/No buttons.
    /// Tab switches focus. Y/y=Yes, N/n/Esc=No. Mouse/touchpad friendly. Y and N highlighted in orange.
    fn draw_operation_confirm_dialog(f: &mut Frame, app: &AppState) {
        let (op, params) = match app.operation_confirm_pending.as_ref() {
            Some(x) => x,
            None => return,
        };
        let count = params.items.len();
        let (title, message) = match op {
            Operation::Copy => (" Copy ", format!("Copy {} {}?", count, if count == 1 { "file" } else { "files" })),
            Operation::Move => (" Move ", format!("Move {} {}?", count, if count == 1 { "file" } else { "files" })),
            Operation::Delete => (" Delete ", format!("Delete {} {}?", count, if count == 1 { "file" } else { "files" })),
        };
        let area = f.area();
        let max_w = 46u16;
        let w = max_w.min(area.width.saturating_sub(4));
        let h = 8u16;
        let x = area.x + (area.width.saturating_sub(w)) / 2;
        let y = area.y + (area.height.saturating_sub(h)) / 2;
        let rect = Rect { x, y, width: w, height: h };
        let menu_bg = Color::Rgb(60, 60, 60);
        let fill_style = Style::default().bg(menu_bg).fg(Color::White);
        let orange = Color::Rgb(255, 180, 80);
        f.render_widget(Clear, rect);
        let block = Block::default()
            .borders(Borders::ALL)
            .title(title)
            .style(fill_style.fg(Color::Cyan));
        f.render_widget(block, rect);
        let inner = rect.inner(Margin { horizontal: 1, vertical: 1 });
        const PAD_H: u16 = 2;
        let content = Rect {
            x: inner.x + PAD_H,
            y: inner.y,
            width: inner.width.saturating_sub(PAD_H * 2),
            height: inner.height,
        };
        let max_msg_w = content.width as usize;
        let msg_display = truncate_str_ellipsis(&message, max_msg_w);
        // Centered message
        let msg_para = Paragraph::new(msg_display.as_str())
            .style(fill_style)
            .alignment(Alignment::Center);
        let msg_y = content.y + (content.height.saturating_sub(3)) / 2;
        f.render_widget(msg_para, Rect {
            x: content.x,
            y: msg_y,
            width: content.width,
            height: 1,
        });
        // Buttons row: Yes and No only, Y and N in orange. Tab switches focus. Horizontal padding.
        const YES_W: u16 = 10;
        const NO_W: u16 = 10;
        let btn_y = content.y + content.height.saturating_sub(2);
        let btn_gap = 4u16;
        let total_btns = YES_W + NO_W + btn_gap;
        let btn_start_x = content.x + content.width.saturating_sub(total_btns) / 2;
        let yes_rect = Rect {
            x: btn_start_x,
            y: btn_y,
            width: YES_W,
            height: 1,
        };
        let no_rect = Rect {
            x: btn_start_x + YES_W + btn_gap,
            y: btn_y,
            width: NO_W,
            height: 1,
        };
        let focus_yes = app.operation_confirm_focus_yes;
        let yes_btn = Line::from(vec![
            Span::raw("  "),
            Span::styled("Y", orange),
            Span::raw("es"),
            Span::raw("  "),
        ]);
        let no_btn = Line::from(vec![
            Span::raw("  "),
            Span::styled("N", orange),
            Span::raw("o"),
            Span::raw("  "),
        ]);
        let yes_style = if focus_yes {
            Style::default().bg(Color::Cyan).fg(Color::Black)
        } else {
            fill_style
        };
        let no_style = if focus_yes {
            fill_style
        } else {
            Style::default().bg(Color::Cyan).fg(Color::Black)
        };
        f.render_widget(Paragraph::new(yes_btn).style(yes_style), yes_rect);
        f.render_widget(Paragraph::new(no_btn).style(no_style), no_rect);
    }

    /// Return (dialog_rect, yes_button_rect, no_button_rect) for operation confirm hit-testing.
    pub fn operation_confirm_button_rects(area: Rect) -> Option<(Rect, Rect, Rect)> {
        let max_w = 46u16;
        let w = max_w.min(area.width.saturating_sub(4));
        let h = 8u16;
        let x = area.x + (area.width.saturating_sub(w)) / 2;
        let y = area.y + (area.height.saturating_sub(h)) / 2;
        let rect = Rect { x, y, width: w, height: h };
        let inner = rect.inner(Margin { horizontal: 1, vertical: 1 });
        const PAD_H: u16 = 2;
        let content = Rect {
            x: inner.x + PAD_H,
            y: inner.y,
            width: inner.width.saturating_sub(PAD_H * 2),
            height: inner.height,
        };
        const YES_W: u16 = 10;
        const NO_W: u16 = 10;
        let btn_gap = 4u16;
        let total_btns = YES_W + NO_W + btn_gap;
        let btn_start_x = content.x + content.width.saturating_sub(total_btns) / 2;
        let btn_y = content.y + content.height.saturating_sub(2);
        let yes_rect = Rect {
            x: btn_start_x,
            y: btn_y,
            width: YES_W,
            height: 1,
        };
        let no_rect = Rect {
            x: btn_start_x + YES_W + btn_gap,
            y: btn_y,
            width: NO_W,
            height: 1,
        };
        Some((rect, yes_rect, no_rect))
    }

    /// Return (rect, content) for overwrite dialog hit-testing. Option rows at content.y+2..content.y+7.
    pub fn overwrite_dialog_layout(area: Rect) -> (Rect, Rect) {
        let max_w = 54u16;
        let w = max_w.min(area.width.saturating_sub(4));
        let h = 12u16;
        let x = area.x + (area.width.saturating_sub(w)) / 2;
        let y = area.y + (area.height.saturating_sub(h)) / 2;
        let rect = Rect { x, y, width: w, height: h };
        let inner = rect.inner(Margin { horizontal: 1, vertical: 1 });
        const PAD_H: u16 = 2;
        let content = Rect {
            x: inner.x + PAD_H,
            y: inner.y,
            width: inner.width.saturating_sub(PAD_H * 2),
            height: inner.height,
        };
        (rect, content)
    }

    /// Return (rect, content) for error dialog hit-testing. Option rows at content.y+2..content.y+5.
    pub fn error_dialog_layout(area: Rect) -> (Rect, Rect) {
        let max_w = 52u16;
        let w = max_w.min(area.width.saturating_sub(4));
        let h = 10u16;
        let x = area.x + (area.width.saturating_sub(w)) / 2;
        let y = area.y + (area.height.saturating_sub(h)) / 2;
        let rect = Rect { x, y, width: w, height: h };
        let inner = rect.inner(Margin { horizontal: 1, vertical: 1 });
        const PAD_H: u16 = 2;
        let content = Rect {
            x: inner.x + PAD_H,
            y: inner.y,
            width: inner.width.saturating_sub(PAD_H * 2),
            height: inner.height,
        };
        (rect, content)
    }

    /// Copy/move error dialog: grey style like confirm, keys 1–3, Tab/Enter. Keys differ from Yes/No.
    fn draw_copy_error_dialog(f: &mut Frame, app: &AppState, err: &crate::app_state::CopyErrorState) {
        let area = f.area();
        let max_w = 52u16;
        let w = max_w.min(area.width.saturating_sub(4));
        let h = 10u16;
        let x = area.x + (area.width.saturating_sub(w)) / 2;
        let y = area.y + (area.height.saturating_sub(h)) / 2;
        let rect = Rect { x, y, width: w, height: h };
        let menu_bg = Color::Rgb(60, 60, 60);
        let fill_style = Style::default().bg(menu_bg).fg(Color::White);
        let orange = Color::Rgb(255, 180, 80);
        f.render_widget(Clear, rect);
        let title = match err.operation {
            Operation::Copy => " Copy error ",
            Operation::Move => " Move error ",
            Operation::Delete => " Delete error ",
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .title(title)
            .style(fill_style.fg(Color::Cyan));
        f.render_widget(block, rect);
        let inner = rect.inner(Margin { horizontal: 1, vertical: 1 });
        const PAD_H: u16 = 2;
        let content = Rect {
            x: inner.x + PAD_H,
            y: inner.y,
            width: inner.width.saturating_sub(PAD_H * 2),
            height: inner.height,
        };
        let max_msg_w = content.width as usize;
        let msg_display = truncate_str_ellipsis(&err.message, max_msg_w);
        let msg_para = Paragraph::new(msg_display.as_str())
            .style(fill_style)
            .alignment(Alignment::Center);
        f.render_widget(msg_para, Rect {
            x: content.x,
            y: content.y,
            width: content.width,
            height: 1,
        });
        let opts: [(u8, &str); 3] = [
            (1, "Skip this file"),
            (2, "Cancel the whole operation"),
            (3, "Ignore all and continue"),
        ];
        let focus = app.copy_error_focus.min(2);
        for (i, (num, label)) in opts.iter().enumerate() {
            let num_s = num.to_string();
            let line = Line::from(vec![
                Span::styled(num_s.as_str(), orange),
                Span::raw(format!(". {}", label)),
            ]);
            let opt_rect = Rect {
                x: content.x,
                y: content.y + 2 + i as u16,
                width: content.width,
                height: 1,
            };
            let style = if i == focus {
                Style::default().bg(Color::Cyan).fg(Color::Black)
            } else {
                fill_style
            };
            f.render_widget(Paragraph::new(line).style(style), opt_rect);
        }
    }

    /// File exists overwrite dialog: grey style like confirm, keys 1–5, Tab/Enter. Keys differ from Yes/No.
    fn draw_copy_overwrite_dialog(f: &mut Frame, app: &AppState, filename: &str) {
        let area = f.area();
        let max_w = 54u16;
        let w = max_w.min(area.width.saturating_sub(4));
        let h = 12u16;
        let x = area.x + (area.width.saturating_sub(w)) / 2;
        let y = area.y + (area.height.saturating_sub(h)) / 2;
        let rect = Rect { x, y, width: w, height: h };
        let menu_bg = Color::Rgb(60, 60, 60);
        let fill_style = Style::default().bg(menu_bg).fg(Color::White);
        let orange = Color::Rgb(255, 180, 80);
        f.render_widget(Clear, rect);
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" File exists ")
            .style(fill_style.fg(Color::Cyan));
        f.render_widget(block, rect);
        let inner = rect.inner(Margin { horizontal: 1, vertical: 1 });
        const PAD_H: u16 = 2;
        let content = Rect {
            x: inner.x + PAD_H,
            y: inner.y,
            width: inner.width.saturating_sub(PAD_H * 2),
            height: inner.height,
        };
        let max_name_w = (content.width as usize).saturating_sub(2);
        let name_only = std::path::Path::new(filename)
            .file_name()
            .and_then(|p| p.to_str())
            .unwrap_or(filename);
        let name_display = truncate_str_ellipsis(name_only, max_name_w);
        let msg_para = Paragraph::new(name_display.as_str())
            .style(fill_style)
            .alignment(Alignment::Center);
        f.render_widget(msg_para, Rect {
            x: content.x,
            y: content.y,
            width: content.width,
            height: 1,
        });
        let opts: [(u8, &str); 5] = [
            (1, "Rewrite this file"),
            (2, "Rewrite all files"),
            (3, "Skip this file"),
            (4, "Skip all existing files"),
            (5, "Cancel the whole operation"),
        ];
        let focus = app.copy_overwrite_focus.min(4);
        for (i, (num, label)) in opts.iter().enumerate() {
            let num_s = num.to_string();
            let line = Line::from(vec![
                Span::styled(num_s.as_str(), orange),
                Span::raw(format!(". {}", label)),
            ]);
            let opt_rect = Rect {
                x: content.x,
                y: content.y + 2 + i as u16,
                width: content.width,
                height: 1,
            };
            let style = if i == focus {
                Style::default().bg(Color::Cyan).fg(Color::Black)
            } else {
                fill_style
            };
            f.render_widget(Paragraph::new(line).style(style), opt_rect);
        }
    }

    /// MC-style copy progress overlay: navy blue background, wider, centered content, small margins. ESC: Cancel.
    fn draw_copy_progress(f: &mut Frame, progress: &crate::app_state::CopyProgress) {
        let area = f.area();
        let inner_width = 76usize;
        const PAD_H: u16 = 2;
        let w = (inner_width as u16 + 2 + PAD_H * 2).min(area.width.saturating_sub(4));
        let h = 10u16;
        let x = area.x + (area.width.saturating_sub(w)) / 2;
        let y = area.y + (area.height.saturating_sub(h)) / 2;
        let rect = Rect { x, y, width: w, height: h };
        let progress_bg = Color::Rgb(25, 40, 60);
        let fill_style = Style::default().bg(progress_bg);

        f.render_widget(Clear, rect);
        let title = match progress.operation {
            Operation::Copy => " Copy ",
            Operation::Move => " Move ",
            Operation::Delete => " Delete ",
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .title(title)
            .style(fill_style.fg(Color::Cyan));
        f.render_widget(block, rect);
        let inner = rect.inner(Margin { horizontal: 1, vertical: 1 });
        let content = Rect {
            x: inner.x + PAD_H,
            y: inner.y,
            width: inner.width.saturating_sub(PAD_H * 2),
            height: inner.height,
        };
        let max_path_width = content.width as usize;

        let space_line = " ".repeat(content.width as usize);
        for r in 0..content.height {
            f.render_widget(
                Paragraph::new(space_line.as_str()).style(Style::default().bg(progress_bg)),
                Rect {
                    x: content.x,
                    y: content.y + r,
                    width: content.width,
                    height: 1,
                },
            );
        }

        let mut row = 0u16;
        let src_label = Paragraph::new("Source")
            .style(fill_style.fg(Color::Yellow))
            .alignment(Alignment::Center);
        f.render_widget(src_label, Rect {
            x: content.x,
            y: content.y + row,
            width: content.width,
            height: 1,
        });
        row += 1;
        let path_display = compact_path(&progress.current_path, max_path_width.max(10));
        let path_para = Paragraph::new(path_display)
            .style(fill_style.fg(Color::White))
            .alignment(Alignment::Center);
        f.render_widget(path_para, Rect {
            x: content.x,
            y: content.y + row,
            width: content.width,
            height: 1,
        });
        row += 1;
        let ratio = if progress.total > 0 {
            (progress.current as f64) / (progress.total as f64).max(1.0)
        } else {
            0.0
        };
        let gauge = Gauge::default()
            .gauge_style(Style::default().fg(Color::Cyan))
            .ratio(ratio)
            .label(format!("{} / {}", progress.current, progress.total));
        f.render_widget(gauge, Rect {
            x: content.x,
            y: content.y + row,
            width: content.width,
            height: 1,
        });
        row += 1;
        let cancel_hint = Paragraph::new("ESC: Cancel")
            .style(fill_style.fg(Color::DarkGray))
            .alignment(Alignment::Center);
        f.render_widget(cancel_hint, Rect {
            x: content.x,
            y: content.y + row,
            width: content.width,
            height: 1,
        });
    }

    /// Menu bar items (label, F-key number). Used for drawing and hit test. Bottom row.
    pub fn menu_bar_items() -> Vec<(&'static str, u16)> {
        vec![
            ("1 Settings", 1),
            ("2 File", 2),
            ("3 View", 3),
            ("4 Edit", 4),
            ("5 Copy", 5),
            ("6 Move", 6),
            ("7 Folder", 7),
            ("8 Delete", 8),
            ("9 Size", 9),
            ("10 Quit", 10),
        ]
    }

    fn draw_menu_bar(f: &mut Frame, area: Rect) {
        let dark_bg = Color::Rgb(60, 60, 60);
        f.render_widget(
            Paragraph::new(" ".repeat(area.width as usize)).style(Style::default().bg(dark_bg)),
            area,
        );
        let items = Self::menu_bar_items();
        let n = items.len() as u16;
        if n == 0 {
            return;
        }
        let slot_w = area.width / n;
        let num_style = Style::default().fg(Color::Rgb(255, 180, 80)).bg(dark_bg);
        let label_style = Style::default().fg(Color::Rgb(180, 180, 180)).bg(dark_bg);
        for (i, (label, _)) in items.iter().enumerate() {
            let slot_start = area.x + (i as u16) * slot_w;
            let label_len = label.chars().count() as u16;
            let x = slot_start + (slot_w.saturating_sub(label_len)) / 2;
            let digits_end = label.char_indices().find(|(_, c)| !c.is_ascii_digit()).map(|(i, _)| i).unwrap_or(label.len());
            let (num_part, rest) = label.split_at(digits_end);
            let line = Line::from(vec![
                Span::styled(num_part, num_style),
                Span::styled(rest, label_style),
            ]);
            let rect = Rect { x, y: area.y, width: label_len.min(slot_w), height: 1 };
            f.render_widget(Paragraph::new(line), rect);
        }
    }

    fn draw_panels_view(f: &mut Frame, app: &mut AppState) {
        let area = f.area();
        f.render_widget(
            Block::default().style(Style::default().bg(MAIN_DARK_BG)),
            area,
        );
        // Rows 0..height-2: frame + panels. Row height-2: command line. Row height-1: menu bar (footer).
        let content_height = area.height.saturating_sub(2);
        let frame_rect = Rect {
            x: area.x,
            y: area.y,
            width: area.width,
            height: content_height,
        };
        let path = app.get_current_dir();
        let title_w = (frame_rect.width.saturating_sub(2)) as usize;
        let title_line = if let Some(file) = app.active_panel_ref().get_selected_file() {
            if !file.is_parent_dir() {
                let combined = format!("{}/{}", path.trim_end_matches('/'), file.name.trim_start_matches('/'));
                Line::from(Span::styled(
                    compact_path(&combined, title_w.max(1)),
                    Style::default().fg(Color::Rgb(255, 180, 80)),
                ))
            } else {
                Line::from(Span::styled(
                    compact_path(path, title_w.max(1)),
                    Style::default().fg(Color::Rgb(255, 180, 80)),
                ))
            }
        } else {
            Line::from(Span::styled(
                compact_path(path, title_w.max(1)),
                Style::default().fg(Color::Rgb(255, 180, 80)),
            ))
        };
        let frame_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::White).bg(MAIN_DARK_BG))
            .style(Style::default().bg(MAIN_DARK_BG))
            .title(title_line);
        let inner = frame_block.inner(frame_rect);
        f.render_widget(frame_block, frame_rect);

        let panel_content_height = inner.height.saturating_sub(1); // bottom bar 1
        let left_w = inner.width / 2;
        let right_w = inner.width.saturating_sub(left_w).saturating_sub(1);
        let sep_x = inner.x + left_w;

        let left_panel = Rect {
            x: inner.x,
            y: inner.y,
            width: left_w,
            height: panel_content_height,
        };
        let right_panel = Rect {
            x: sep_x + 1,
            y: inner.y,
            width: right_w,
            height: panel_content_height,
        };
        let bottom_file_rect = Rect {
            x: inner.x,
            y: inner.y + panel_content_height,
            width: inner.width,
            height: 1,
        };
        let command_rect = Rect {
            x: area.x,
            y: area.y + content_height,
            width: area.width,
            height: 1,
        };
        let menu_rect = Rect {
            x: area.x,
            y: area.y + content_height + 1,
            width: area.width,
            height: 1,
        };

        let active_panel = app.active_panel();
        Self::draw_single_panel(f, app.left_panel_mut(), left_panel, "Left Panel", active_panel == 0);
        Self::draw_single_panel(f, app.right_panel_mut(), right_panel, "Right Panel", active_panel == 1);
        // Vertical separator │ from top of content to bottom bar
        let sep_style = Style::default().bg(MAIN_DARK_BG).fg(Color::White);
        for row in inner.y..(inner.y + panel_content_height + 1) {
            f.render_widget(
                Paragraph::new("│").style(sep_style),
                Rect { x: sep_x, y: row, width: 1, height: 1 },
            );
        }
        Self::draw_bottom_file_bar(f, app, bottom_file_rect, sep_x);
        Self::draw_command_line(f, app, command_rect);
        Self::draw_menu_bar(f, menu_rect);
    }

    /// Bottom bar (MC-style): vertical │ at split; left = file attributes, right = disk space.
    fn draw_bottom_file_bar(f: &mut Frame, app: &AppState, area: Rect, sep_x: u16) {
        let bar_style = Style::default().bg(MAIN_DARK_BG).fg(Color::White);
        let file_opt = if app.active_panel() == 0 {
            app.left_panel().get_selected_file()
        } else {
            app.right_panel().get_selected_file()
        };
        let (size, permissions) = if let Some(file) = file_opt {
            (
                file.size,
                if file.permissions.is_empty() {
                    "----------"
                } else {
                    file.permissions.as_str()
                },
            )
        } else {
            (0u64, "----------")
        };
        let size_str = format_size(size);
        let left_text = if permissions.is_empty() || permissions == "----------" {
            format!(" {} - ", size_str)
        } else {
            format!(" {} {}", size_str, permissions)
        };
        let left_w = (sep_x.saturating_sub(area.x)) as usize;
        let left_line = if left_text.chars().count() > left_w {
            left_text.chars().take(left_w).collect::<String>()
        } else {
            format!("{}{}", left_text, " ".repeat(left_w.saturating_sub(left_text.chars().count())))
        };
        let left_rect = Rect { x: area.x, y: area.y, width: left_w as u16, height: 1 };
        f.render_widget(Paragraph::new(left_line).style(bar_style), left_rect);
        f.render_widget(Paragraph::new("│").style(bar_style), Rect { x: sep_x, y: area.y, width: 1, height: 1 });
        let right_w = area.width.saturating_sub((sep_x - area.x) + 1);
        let disk = disk_space_string(app.get_current_dir());
        let disk_len = disk.chars().count().min(right_w as usize);
        let right_pad = (right_w as usize).saturating_sub(disk_len);
        let right_line = format!("{}{}", " ".repeat(right_pad), disk);
        let right_rect = Rect { x: sep_x + 1, y: area.y, width: right_w, height: 1 };
        f.render_widget(Paragraph::new(right_line).style(bar_style), right_rect);
    }

    fn draw_command_line(f: &mut Frame, app: &AppState, area: Rect) {
        let prompt = "$ ";
        let line = format!("{}{}", prompt, app.command_line);
        let is_focused = app.focus == Focus::CommandLine;
        let base = Style::default().bg(MAIN_DARK_BG);
        let style = if is_focused {
            base.fg(Color::Yellow)
        } else {
            base.fg(Color::DarkGray)
        };
        let p = Paragraph::new(line.clone()).style(style);
        f.render_widget(p, area);
        if is_focused {
            let cursor_x = (prompt.len() + app.command_line_cursor.min(app.command_line.len())) as u16;
            if cursor_x < area.width {
                f.set_cursor_position((area.x + cursor_x, area.y));
            }
        }
    }

    fn draw_single_panel(f: &mut Frame, panel: &mut Panel, area: Rect, _title: &str, is_active_panel: bool) {
        match panel.get_view_mode() {
            ViewMode::SingleColumn => Self::draw_single_column_view(f, panel, area, is_active_panel),
            ViewMode::DoubleColumn => Self::draw_double_column_view(f, panel, area, is_active_panel),
        }
    }

    fn draw_single_column_view(f: &mut Frame, panel: &mut Panel, area: Rect, is_active_panel: bool) {
        const IN_W: u16 = 2;
        const SIZE_W: u16 = 12;
        const MTIME_W: u16 = 12;
        const GAP: u16 = 1;

        let name_w = area.width.saturating_sub(IN_W + GAP + SIZE_W + GAP + MTIME_W).max(10);
        let total_content_w = IN_W + GAP + name_w + GAP + SIZE_W + GAP + MTIME_W;

        let panel_height = (area.height as usize).max(1);
        let data_rows = panel_height.saturating_sub(1).max(0); // reserve 1 for header
        panel.update_scroll_offset(data_rows);

        let files = panel.get_files();
        let scroll = panel.get_scroll_offset().min(files.len().saturating_sub(1).max(0));

        let base = Style::default().bg(MAIN_DARK_BG);
        // Header row: "in", "Name", "Size", "Modify time"
        let header = Line::from(vec![
            Span::styled("in", base.fg(Color::DarkGray)),
            Span::raw(" "),
            Span::styled("Name", base.fg(Color::DarkGray)),
            Span::raw(" ".repeat((name_w + 1 + SIZE_W + 1) as usize)),
            Span::styled("Size", base.fg(Color::DarkGray)),
            Span::raw(" "),
            Span::styled("Modify time", base.fg(Color::DarkGray)),
        ]);
        let header_rect = Rect {
            x: area.x,
            y: area.y,
            width: area.width,
            height: 1,
        };
        f.render_widget(Paragraph::new(header), header_rect);

        for (i, file) in files.iter().skip(scroll).take(data_rows).enumerate() {
            let actual_index = i + scroll;
            let is_selected = is_active_panel && actual_index == panel.get_selected_index();
            let is_marked = panel.is_marked(actual_index);

            let in_cell = if is_marked { "> " } else { "  " };
            let name_display = truncate_for_width(file, name_w as usize);
            let size_str = size_display(file);
            let mtime_str = file
                .mtime
                .as_ref()
                .map(format_mtime)
                .unwrap_or_else(String::new);
            let size_pad = format!("{:>1$}", size_str, SIZE_W as usize);
            let mtime_pad = format!("{:>12}", mtime_str);

            let (name_style, in_style) = if is_selected {
                let sel = Style::default().fg(Color::White).bg(Color::Blue);
                (sel, sel)
            } else if file.is_dir {
                let dir = base.fg(Color::Cyan).add_modifier(Modifier::BOLD);
                (dir, base)
            } else if file.is_executable {
                (base.fg(Color::Green), base)
            } else if file.is_symlink {
                (base.fg(Color::Magenta), base)
            } else {
                (base.fg(Color::White), base)
            };

            let pad_after_name = " ".repeat(
                (name_w as usize)
                    .saturating_sub(name_display.chars().count())
                    .saturating_add(GAP as usize),
            );

            let spans = vec![
                Span::styled(in_cell, if is_selected { in_style } else { base }),
                Span::styled(name_display, name_style),
                Span::styled(pad_after_name, if is_selected { in_style } else { base }),
                Span::styled(size_pad.as_str(), if is_selected { in_style } else { base.fg(Color::White) }),
                Span::raw(" "),
                Span::styled(mtime_pad.as_str(), if is_selected { in_style } else { base.fg(Color::White) }),
            ];

            let row = Line::from(spans);
            let row_rect = Rect {
                x: area.x,
                y: area.y + 1 + i as u16,
                width: area.width.min(total_content_w),
                height: 1,
            };
            f.render_widget(Paragraph::new(row), row_rect);
        }
    }

    fn draw_double_column_view(f: &mut Frame, panel: &mut Panel, area: Rect, is_active_panel: bool) {
        let panel_height = (area.height as usize).max(1);
        let files_per_column = panel_height;
        let files_per_page = files_per_column * 2;

        panel.update_scroll_offset_double_column(panel_height);

        let col_w = (area.width / 2).max(1);
        let left_col = Rect {
            x: area.x,
            y: area.y,
            width: col_w,
            height: area.height,
        };
        let right_col = Rect {
            x: area.x + col_w + 1,
            y: area.y,
            width: area.width.saturating_sub(col_w + 1).max(1),
            height: area.height,
        };
        let vertical_line_x = area.x + col_w;

        let files = panel.get_files();
        let max_scroll = files.len().saturating_sub(files_per_page).max(0);
        let scroll = panel.get_scroll_offset().min(max_scroll);
        let visible_files: Vec<_> = files
            .iter()
            .skip(scroll)
            .take(files_per_page)
            .collect();
        let (left_files, right_files) = visible_files.split_at(visible_files.len().min(files_per_column));

        let max_left_w = (left_col.width as usize).max(1);
        let max_right_w = (right_col.width as usize).max(1);

        for (i, file) in left_files.iter().enumerate() {
            if i >= panel_height {
                break;
            }
            let actual_index = i + scroll;
            let is_selected = is_active_panel && actual_index == panel.get_selected_index();
            let is_marked = panel.is_marked(actual_index);
            let display = truncate_for_width(file, max_left_w);
            let line = styles::create_file_line_from_display(&display, file.is_dir, file.is_symlink, file.is_executable, is_selected, is_marked);
            let line_area = Rect {
                x: left_col.x,
                y: left_col.y + i as u16,
                width: left_col.width,
                height: 1,
            };
            f.render_widget(Paragraph::new(line), line_area);
        }
        for (i, file) in right_files.iter().enumerate() {
            if i >= panel_height {
                break;
            }
            let actual_index = i + left_files.len() + scroll;
            let is_selected = is_active_panel && actual_index == panel.get_selected_index();
            let is_marked = panel.is_marked(actual_index);
            let display = truncate_for_width(file, max_right_w);
            let line = styles::create_file_line_from_display(&display, file.is_dir, file.is_symlink, file.is_executable, is_selected, is_marked);
            let line_area = Rect {
                x: right_col.x,
                y: right_col.y + i as u16,
                width: right_col.width,
                height: 1,
            };
            f.render_widget(Paragraph::new(line), line_area);
        }

        for y in left_col.y..left_col.y + left_col.height {
            f.render_widget(
                Paragraph::new("│").style(Style::default().fg(Color::White)),
                Rect {
                    x: vertical_line_x,
                    y,
                    width: 1,
                    height: 1,
                },
            );
        }
    }
}
