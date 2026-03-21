use crate::app_state::{AppState, Focus, Operation};
use crate::core::disk_space::disk_space_summary;
use crate::core::file_ops::FileInfo;
use crate::core::panel_backend;
use crate::core::text_format::{format_byte_size, truncate_str, TruncateMode};
use crate::dialog_layout::{self, DEFAULT_PAD_H};
use crate::editor;
use crate::panel::{Panel, PanelOperations, ViewMode};
use crate::styles;
use crate::styles::{DIALOG_ACCENT, DIALOG_BG, DIALOG_FOCUS};
use crate::viewer;
use ratatui::{
    layout::{Alignment, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Gauge, Paragraph},
    Frame,
};

/// Dark background for main content area (panels, command line). Ensures consistent look across terminals.
const MAIN_DARK_BG: Color = Color::Rgb(30, 30, 35);

pub struct Renderer;

/// Truncate string to max_width chars with trailing ellipsis. Returns full string if it fits.
pub fn truncate_str_ellipsis(
    s: &str,
    max_width: usize,
) -> String {
    truncate_str(s, max_width, TruncateMode::PrefixEllipsis)
}

/// Shorten path to max_width chars like MC: "first_half~last_half" (one ~ in the middle).
fn compact_path(
    path: &str,
    max_width: usize,
) -> String {
    truncate_str(path, max_width, TruncateMode::CompactMiddle)
}

/// Format mtime as "Feb 13 2024 20:05" (month, day, year, time with zero-padded minutes).
fn format_mtime(t: &std::time::SystemTime) -> String {
    use chrono::{DateTime, Timelike, Utc};
    let datetime: DateTime<Utc> = (*t).into();
    let date = datetime.format("%b %e %Y").to_string();
    let time = format!("{:02}:{:02}", datetime.hour(), datetime.minute());
    format!("{} {}", date, time)
}

/// Filename only for bottom bar display (no path).
fn filename_for_bottom_bar(file: Option<&FileInfo>) -> String {
    match file {
        None => String::new(),
        Some(f) => f.name.trim_end_matches('/').to_string(),
    }
}

/// Size display: nothing for dirs (including ".."), human-readable for files.
fn size_display(file: &FileInfo) -> String {
    if file.is_dir {
        String::new()
    } else {
        format_byte_size(file.size)
    }
}

fn is_zip_file(file: &FileInfo) -> bool {
    !file.is_dir
        && file
            .name
            .trim_end_matches('/')
            .to_ascii_lowercase()
            .ends_with(".zip")
}

/// Truncate file display to fit column width (chars). Prevents wrapping/uglification in double-column view.
/// Prefix: "/" for folders, "@" for symlinks, "*" for executables, " " for regular files (as in screenshot).
fn truncate_for_width(
    file: &FileInfo,
    max_width: usize,
) -> String {
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
    pub fn draw_ui(
        f: &mut Frame,
        app: &mut AppState,
    ) {
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
        if let Some(ref progress) = app.archive_progress {
            Self::draw_archive_progress(f, progress);
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
        if app.archive_dialog.is_some() {
            crate::archive_dialog::draw(f, app);
        }
        if app.new_file_dialog.is_some() {
            crate::new_file_dialog::draw(f, app);
        }
        if app.new_file_error.is_some() {
            Self::draw_new_file_error_dialog(f, app);
        }
        if app.rename_attr_dialog.is_some() {
            crate::rename_attr::draw(f, app);
        }
        if app.help_dialog {
            crate::help_dialog::draw(f, app);
        }
        if app.settings_dialog.is_some() {
            crate::settings_dialog::draw(f, app);
        }
        if app.find_dialog.is_some() {
            crate::find_dialog::draw(f, app);
        }
        if app.left_panel_settings_overlay.is_some() || app.right_panel_settings_overlay.is_some() {
            crate::panel_overlay::draw(f, app);
        }
    }

    /// Operation confirmation dialog (Copy/Move/Delete): operation alert, Yes/No buttons.
    /// Tab switches focus. Y/y=Yes, N/n/Esc=No. Mouse/touchpad friendly. Y and N highlighted in orange.
    /// For Copy/Move: shows "From" and "To" paths.
    fn draw_operation_confirm_dialog(
        f: &mut Frame,
        app: &AppState,
    ) {
        let (op, params) = match app.operation_confirm_pending.as_ref() {
            Some(x) => x,
            None => return,
        };
        let count = params.items.len();
        let (title, message) = match op {
            Operation::Copy => (
                " Copy ",
                format!(
                    "Copy {} {}?",
                    count,
                    if count == 1 { "file" } else { "files" }
                ),
            ),
            Operation::Move => (
                " Move ",
                format!(
                    "Move {} {}?",
                    count,
                    if count == 1 { "file" } else { "files" }
                ),
            ),
            Operation::Delete => (
                " Delete ",
                format!(
                    "Delete {} {}?",
                    count,
                    if count == 1 { "file" } else { "files" }
                ),
            ),
        };
        let show_paths = matches!(op, Operation::Copy | Operation::Move);
        let area = f.area();
        let h = if show_paths { 11 } else { 8 };
        let rect = dialog_layout::centered_dialog_rect(area, 60, h);
        let content = dialog_layout::dialog_content_rect(rect, DEFAULT_PAD_H);
        let fill_style = Style::default().bg(DIALOG_BG).fg(Color::White);
        f.render_widget(Clear, rect);
        let block = Block::default()
            .borders(Borders::ALL)
            .title(title)
            .style(fill_style.fg(DIALOG_FOCUS));
        f.render_widget(block, rect);
        let max_msg_w = content.width as usize;
        let mut row = content.y;
        if show_paths {
            let path_w = max_msg_w.saturating_sub(2);
            let from_str = compact_path(params.source_dir.trim_end_matches('/'), path_w);
            let to_str = compact_path(params.target_dir.trim_end_matches('/'), path_w);
            let from_line = format!("From: {}", from_str);
            let to_line = format!("To:   {}", to_str);
            f.render_widget(
                Paragraph::new(from_line).style(fill_style),
                Rect {
                    x: content.x,
                    y: row,
                    width: content.width,
                    height: 1,
                },
            );
            row += 1;
            f.render_widget(
                Paragraph::new(to_line).style(fill_style),
                Rect {
                    x: content.x,
                    y: row,
                    width: content.width,
                    height: 1,
                },
            );
            row += 2; // blank before message
        }
        let msg_display = truncate_str_ellipsis(&message, max_msg_w);
        let msg_para = Paragraph::new(msg_display.as_str())
            .style(fill_style)
            .alignment(Alignment::Center);
        let msg_y = if show_paths {
            row
        } else {
            content.y + (content.height.saturating_sub(3)) / 2
        };
        f.render_widget(
            msg_para,
            Rect {
                x: content.x,
                y: msg_y,
                width: content.width,
                height: 1,
            },
        );
        // Buttons row: Yes and No only, Y and N in orange. Tab switches focus.
        const YES_W: u16 = 10;
        const NO_W: u16 = 10;
        let btn_y = content.y + content.height.saturating_sub(2);
        let (yes_rect, no_rect) = dialog_layout::two_button_rects(content, btn_y, YES_W, NO_W, 4);
        let focus_yes = app.operation_confirm_focus_yes;
        let yes_btn = Line::from(vec![
            Span::raw("  "),
            Span::styled("Y", DIALOG_ACCENT),
            Span::raw("es"),
            Span::raw("  "),
        ]);
        let no_btn = Line::from(vec![
            Span::raw("  "),
            Span::styled("N", DIALOG_ACCENT),
            Span::raw("o"),
            Span::raw("  "),
        ]);
        let yes_style = if focus_yes {
            Style::default().bg(DIALOG_FOCUS).fg(Color::Black)
        } else {
            fill_style
        };
        let no_style = if focus_yes {
            fill_style
        } else {
            Style::default().bg(DIALOG_FOCUS).fg(Color::Black)
        };
        f.render_widget(Paragraph::new(yes_btn).style(yes_style), yes_rect);
        f.render_widget(Paragraph::new(no_btn).style(no_style), no_rect);
    }

    /// Return (dialog_rect, yes_button_rect, no_button_rect) for operation confirm hit-testing.
    /// show_paths: true for Copy/Move (taller dialog).
    pub fn operation_confirm_button_rects(
        area: Rect,
        show_paths: bool,
    ) -> Option<(Rect, Rect, Rect)> {
        let h = if show_paths { 11 } else { 8 };
        let rect = dialog_layout::centered_dialog_rect(area, 60, h);
        let content = dialog_layout::dialog_content_rect(rect, DEFAULT_PAD_H);
        let btn_y = content.y + content.height.saturating_sub(2);
        // Mouse-friendly: 50/50 split, height 2
        let btn_half_w = content.width / 2;
        let yes_rect = Rect {
            x: content.x,
            y: btn_y,
            width: btn_half_w,
            height: 2,
        };
        let no_rect = Rect {
            x: content.x + btn_half_w,
            y: btn_y,
            width: content.width.saturating_sub(btn_half_w),
            height: 2,
        };
        Some((rect, yes_rect, no_rect))
    }

    /// Return (rect, content) for overwrite dialog hit-testing. Option rows at content.y+2..content.y+7.
    pub fn overwrite_dialog_layout(area: Rect) -> (Rect, Rect) {
        let rect = dialog_layout::centered_dialog_rect(area, 54, 12);
        let content = dialog_layout::dialog_content_rect(rect, DEFAULT_PAD_H);
        (rect, content)
    }

    /// Return (rect, content) for error dialog hit-testing. Option rows at content.y+2..content.y+5.
    pub fn error_dialog_layout(area: Rect) -> (Rect, Rect) {
        let rect = dialog_layout::centered_dialog_rect(area, 52, 10);
        let content = dialog_layout::dialog_content_rect(rect, DEFAULT_PAD_H);
        (rect, content)
    }

    /// Draw a list of numbered options (e.g. "1. Skip", "2. Cancel"). Option rows start at content.y + start_row.
    fn draw_numbered_options(
        f: &mut Frame,
        content: Rect,
        options: &[(u8, &str)],
        focus_index: usize,
        fill_style: Style,
        start_row: u16,
    ) {
        let focus = focus_index.min(options.len().saturating_sub(1));
        for (i, (num, label)) in options.iter().enumerate() {
            let num_s = num.to_string();
            let line = Line::from(vec![
                Span::styled(num_s.as_str(), DIALOG_ACCENT),
                Span::raw(format!(". {}", label)),
            ]);
            let opt_rect = Rect {
                x: content.x,
                y: content.y + start_row + i as u16,
                width: content.width,
                height: 1,
            };
            let style = if i == focus {
                Style::default().bg(DIALOG_FOCUS).fg(Color::Black)
            } else {
                fill_style
            };
            f.render_widget(Paragraph::new(line).style(style), opt_rect);
        }
    }

    /// Copy/move error dialog: grey style like confirm, keys 1–3, Tab/Enter. Keys differ from Yes/No.
    fn draw_copy_error_dialog(
        f: &mut Frame,
        app: &AppState,
        err: &crate::app_state::CopyErrorState,
    ) {
        let area = f.area();
        let rect = dialog_layout::centered_dialog_rect(area, 52, 10);
        let content = dialog_layout::dialog_content_rect(rect, DEFAULT_PAD_H);
        let fill_style = Style::default().bg(DIALOG_BG).fg(Color::White);
        f.render_widget(Clear, rect);
        let title = match err.operation {
            Operation::Copy => " Copy error ",
            Operation::Move => " Move error ",
            Operation::Delete => " Delete error ",
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .title(title)
            .style(fill_style.fg(DIALOG_FOCUS));
        f.render_widget(block, rect);
        let max_msg_w = content.width as usize;
        let msg_display = truncate_str_ellipsis(&err.message, max_msg_w);
        f.render_widget(
            Paragraph::new(msg_display.as_str())
                .style(fill_style)
                .alignment(Alignment::Center),
            Rect {
                x: content.x,
                y: content.y,
                width: content.width,
                height: 1,
            },
        );
        let opts: [(u8, &str); 3] = [
            (1, "Skip this file"),
            (2, "Cancel the whole operation"),
            (3, "Ignore all and continue"),
        ];
        Self::draw_numbered_options(
            f,
            content,
            &opts,
            app.copy_error_focus.min(2),
            fill_style,
            2,
        );
    }

    /// New file error dialog (e.g. file already exists): grey style, message and 1. OK. Enter/Esc or click OK closes.
    fn draw_new_file_error_dialog(
        f: &mut Frame,
        app: &AppState,
    ) {
        let area = f.area();
        let rect = dialog_layout::centered_dialog_rect(area, 52, 8);
        let content = dialog_layout::dialog_content_rect(rect, DEFAULT_PAD_H);
        let fill_style = Style::default().bg(DIALOG_BG).fg(Color::White);
        f.render_widget(Clear, rect);
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Error ")
            .style(fill_style.fg(DIALOG_FOCUS));
        f.render_widget(block, rect);
        let msg = app.new_file_error.as_deref().unwrap_or("");
        let msg_display = truncate_str_ellipsis(msg, content.width as usize);
        f.render_widget(
            Paragraph::new(msg_display)
                .style(fill_style)
                .alignment(Alignment::Center),
            Rect {
                x: content.x,
                y: content.y,
                width: content.width,
                height: 1,
            },
        );
        let ok_opts: [(u8, &str); 1] = [(1, "OK")];
        Self::draw_numbered_options(f, content, &ok_opts, 0, fill_style, 4);
    }

    /// Return OK button rect for new file error dialog hit-testing (row at content.y + 4).
    pub fn new_file_error_ok_rect(area: Rect) -> Option<Rect> {
        let rect = dialog_layout::centered_dialog_rect(area, 52, 8);
        let content = dialog_layout::dialog_content_rect(rect, DEFAULT_PAD_H);
        const OK_W: u16 = 6;
        let ok_x = content.x + content.width.saturating_sub(OK_W) / 2;
        let ok_y = content.y + 4;
        Some(Rect {
            x: ok_x,
            y: ok_y,
            width: OK_W,
            height: 1,
        })
    }

    /// File exists overwrite dialog: grey style like confirm, keys 1–5, Tab/Enter. Keys differ from Yes/No.
    fn draw_copy_overwrite_dialog(
        f: &mut Frame,
        app: &AppState,
        filename: &str,
    ) {
        let area = f.area();
        let rect = dialog_layout::centered_dialog_rect(area, 54, 12);
        let content = dialog_layout::dialog_content_rect(rect, DEFAULT_PAD_H);
        let fill_style = Style::default().bg(DIALOG_BG).fg(Color::White);
        f.render_widget(Clear, rect);
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" File exists ")
            .style(fill_style.fg(DIALOG_FOCUS));
        f.render_widget(block, rect);
        let max_name_w = (content.width as usize).saturating_sub(2);
        let name_only = std::path::Path::new(filename)
            .file_name()
            .and_then(|p| p.to_str())
            .unwrap_or(filename);
        let name_display = truncate_str_ellipsis(name_only, max_name_w);
        f.render_widget(
            Paragraph::new(name_display.as_str())
                .style(fill_style)
                .alignment(Alignment::Center),
            Rect {
                x: content.x,
                y: content.y,
                width: content.width,
                height: 1,
            },
        );
        let opts: [(u8, &str); 5] = [
            (1, "Rewrite this file"),
            (2, "Rewrite all files"),
            (3, "Skip this file"),
            (4, "Skip all existing files"),
            (5, "Cancel the whole operation"),
        ];
        Self::draw_numbered_options(
            f,
            content,
            &opts,
            app.copy_overwrite_focus.min(4),
            fill_style,
            2,
        );
    }

    /// MC-style copy progress overlay: navy blue background, wider, centered content, small margins. ESC: Cancel.
    fn draw_copy_progress(
        f: &mut Frame,
        progress: &crate::app_state::CopyProgress,
    ) {
        let area = f.area();
        let inner_width = 76usize;
        const PAD_H: u16 = 2;
        let w = (inner_width as u16 + 2 + PAD_H * 2).min(area.width.saturating_sub(4));
        let h = if matches!(progress.operation, Operation::Copy | Operation::Move) {
            12
        } else {
            10
        };
        let x = area.x + (area.width.saturating_sub(w)) / 2;
        let y = area.y + (area.height.saturating_sub(h)) / 2;
        let rect = Rect {
            x,
            y,
            width: w,
            height: h,
        };
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
        let inner = rect.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });
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
        f.render_widget(
            src_label,
            Rect {
                x: content.x,
                y: content.y + row,
                width: content.width,
                height: 1,
            },
        );
        row += 1;
        let path_display = compact_path(&progress.current_path, max_path_width.max(10));
        let path_para = Paragraph::new(path_display)
            .style(fill_style.fg(Color::White))
            .alignment(Alignment::Center);
        f.render_widget(
            path_para,
            Rect {
                x: content.x,
                y: content.y + row,
                width: content.width,
                height: 1,
            },
        );
        row += 1;
        if matches!(progress.operation, Operation::Copy | Operation::Move) {
            let tgt_label = Paragraph::new("Target")
                .style(fill_style.fg(Color::Yellow))
                .alignment(Alignment::Center);
            f.render_widget(
                tgt_label,
                Rect {
                    x: content.x,
                    y: content.y + row,
                    width: content.width,
                    height: 1,
                },
            );
            row += 1;
            let target_display = compact_path(&progress.target_path, max_path_width.max(10));
            let target_para = Paragraph::new(target_display)
                .style(fill_style.fg(Color::White))
                .alignment(Alignment::Center);
            f.render_widget(
                target_para,
                Rect {
                    x: content.x,
                    y: content.y + row,
                    width: content.width,
                    height: 1,
                },
            );
            row += 1;
        }
        let ratio = if progress.total > 0 {
            (progress.current as f64) / (progress.total as f64).max(1.0)
        } else {
            0.0
        };
        let gauge = Gauge::default()
            .gauge_style(Style::default().fg(Color::Cyan))
            .ratio(ratio)
            .label(format!("{} / {}", progress.current, progress.total));
        f.render_widget(
            gauge,
            Rect {
                x: content.x,
                y: content.y + row,
                width: content.width,
                height: 1,
            },
        );
        row += 1;
        let cancel_hint = Paragraph::new("ESC: Cancel")
            .style(fill_style.fg(Color::DarkGray))
            .alignment(Alignment::Center);
        f.render_widget(
            cancel_hint,
            Rect {
                x: content.x,
                y: content.y + row,
                width: content.width,
                height: 1,
            },
        );
    }

    /// Archive progress overlay (Ctrl+A): same style as copy progress — Source, Target, gauge. ESC: Cancel.
    fn draw_archive_progress(
        f: &mut Frame,
        progress: &crate::app_state::ArchiveProgress,
    ) {
        let area = f.area();
        let inner_width = 76usize;
        const PAD_H: u16 = 2;
        let w = (inner_width as u16 + 2 + PAD_H * 2).min(area.width.saturating_sub(4));
        let h = 12u16;
        let x = area.x + (area.width.saturating_sub(w)) / 2;
        let y = area.y + (area.height.saturating_sub(h)) / 2;
        let rect = Rect {
            x,
            y,
            width: w,
            height: h,
        };
        let progress_bg = Color::Rgb(25, 40, 60);
        let fill_style = Style::default().bg(progress_bg);

        f.render_widget(Clear, rect);
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Archive ")
            .style(fill_style.fg(Color::Cyan));
        f.render_widget(block, rect);
        let inner = rect.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });
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
        f.render_widget(
            src_label,
            Rect {
                x: content.x,
                y: content.y + row,
                width: content.width,
                height: 1,
            },
        );
        row += 1;
        let path_display = compact_path(&progress.current_path, max_path_width.max(10));
        f.render_widget(
            Paragraph::new(path_display)
                .style(fill_style.fg(Color::White))
                .alignment(Alignment::Center),
            Rect {
                x: content.x,
                y: content.y + row,
                width: content.width,
                height: 1,
            },
        );
        row += 1;
        let tgt_label = Paragraph::new("Target")
            .style(fill_style.fg(Color::Yellow))
            .alignment(Alignment::Center);
        f.render_widget(
            tgt_label,
            Rect {
                x: content.x,
                y: content.y + row,
                width: content.width,
                height: 1,
            },
        );
        row += 1;
        let target_display = compact_path(&progress.target_path, max_path_width.max(10));
        f.render_widget(
            Paragraph::new(target_display)
                .style(fill_style.fg(Color::White))
                .alignment(Alignment::Center),
            Rect {
                x: content.x,
                y: content.y + row,
                width: content.width,
                height: 1,
            },
        );
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
        f.render_widget(
            gauge,
            Rect {
                x: content.x,
                y: content.y + row,
                width: content.width,
                height: 1,
            },
        );
        row += 1;
        let cancel_hint = Paragraph::new("ESC: Cancel")
            .style(fill_style.fg(Color::DarkGray))
            .alignment(Alignment::Center);
        f.render_widget(
            cancel_hint,
            Rect {
                x: content.x,
                y: content.y + row,
                width: content.width,
                height: 1,
            },
        );
    }

    /// Menu bar items (label, F-key number). Used for drawing and hit test. Bottom row.
    pub fn menu_bar_items() -> Vec<(&'static str, u16)> {
        vec![
            ("1 Help", 1),
            ("2 File", 2),
            ("3 View", 3),
            ("4 Edit", 4),
            ("5 Copy", 5),
            ("6 Move", 6),
            ("7 Folder", 7),
            ("8 Delete", 8),
            ("9 Settings", 9),
            ("10 Quit", 10),
        ]
    }

    fn is_menu_action_available(
        app: &AppState,
        key: u16,
    ) -> bool {
        // While Find file dialog is open, no menu actions are available.
        if app.find_dialog.is_some() {
            return false;
        }
        // In command prompt mode only F10 (Quit) is available.
        if app.focus == Focus::CommandLine {
            return key == 10;
        }
        match key {
            3 => app
                .active_panel_ref()
                .get_selected_file()
                .map_or(false, |f| !f.is_dir && !f.is_parent_dir()),
            4 => {
                panel_backend::supports_edit(&app.get_current_location())
                    && app
                        .active_panel_ref()
                        .get_selected_file()
                        .map_or(false, |f| !f.is_dir && !f.is_parent_dir())
            }
            5 | 6 => {
                let source = app.get_current_dir();
                let target = app.get_opposite_panel_dir();
                let (items, ..) = app
                    .active_panel_ref()
                    .get_names_to_copy_with_restore_neighbors();
                source != target && !items.is_empty()
            }
            7 => panel_backend::supports_mkdir(&app.get_current_location()),
            8 => {
                let (items, ..) = app
                    .active_panel_ref()
                    .get_names_to_copy_with_restore_neighbors();
                !items.is_empty()
            }
            _ => true,
        }
    }

    fn draw_menu_bar(
        f: &mut Frame,
        area: Rect,
        app: &AppState,
    ) {
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
        let unavailable_style = Style::default().fg(Color::DarkGray).bg(dark_bg);
        for (i, (label, key)) in items.iter().enumerate() {
            let slot_start = area.x + (i as u16) * slot_w;
            let unavailable = !Self::is_menu_action_available(app, *key);
            let display_label = truncate_str_ellipsis(label, slot_w as usize);
            let label_len = display_label.chars().count() as u16;
            let x = slot_start + (slot_w.saturating_sub(label_len)) / 2;
            let digits_end = display_label
                .char_indices()
                .find(|(_, c)| !c.is_ascii_digit())
                .map(|(i, _)| i)
                .unwrap_or(display_label.len());
            let (num_part, rest) = display_label.split_at(digits_end);
            let line = Line::from(vec![
                Span::styled(
                    num_part,
                    if unavailable {
                        unavailable_style
                    } else {
                        num_style
                    },
                ),
                Span::styled(
                    rest,
                    if unavailable {
                        unavailable_style
                    } else {
                        label_style
                    },
                ),
            ]);
            let rect = Rect {
                x,
                y: area.y,
                width: label_len.min(slot_w),
                height: 1,
            };
            f.render_widget(Paragraph::new(line), rect);
        }
    }

    fn draw_panels_view(
        f: &mut Frame,
        app: &mut AppState,
    ) {
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
        let frame_block = Block::default()
            .borders(Borders::LEFT | Borders::RIGHT | Borders::BOTTOM)
            .border_style(Style::default().fg(Color::White).bg(MAIN_DARK_BG))
            .style(Style::default().bg(MAIN_DARK_BG));
        let inner = frame_block.inner(frame_rect);
        f.render_widget(frame_block, frame_rect);

        let left_w = inner.width / 2;
        let right_w = inner.width.saturating_sub(left_w).saturating_sub(1);
        let sep_x = inner.x + left_w;

        // Row 0: path (without filename) at top-left of each panel
        let path_style = Style::default().fg(Color::Rgb(255, 180, 80));
        let left_path = app.left_panel().get_current_dir();
        let right_path = app.right_panel().get_current_dir();
        let left_path_display = compact_path(left_path.trim_end_matches('/'), left_w as usize);
        let right_path_display = compact_path(right_path.trim_end_matches('/'), right_w as usize);
        f.render_widget(
            Paragraph::new(left_path_display).style(path_style),
            Rect {
                x: inner.x,
                y: inner.y,
                width: left_w,
                height: 1,
            },
        );
        f.render_widget(
            Paragraph::new(right_path_display).style(path_style),
            Rect {
                x: sep_x + 1,
                y: inner.y,
                width: right_w,
                height: 1,
            },
        );

        let panel_content_height = inner.height.saturating_sub(2); // 1 for path row, 1 for bottom bar
        let left_panel = Rect {
            x: inner.x,
            y: inner.y + 1,
            width: left_w,
            height: panel_content_height,
        };
        let right_panel = Rect {
            x: sep_x + 1,
            y: inner.y + 1,
            width: right_w,
            height: panel_content_height,
        };
        app.left_panel_rect = Some(left_panel);
        app.right_panel_rect = Some(right_panel);
        let bottom_file_rect = Rect {
            x: inner.x,
            y: inner.y + 1 + panel_content_height,
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
        Self::draw_single_panel(
            f,
            app.left_panel_mut(),
            left_panel,
            "Left Panel",
            active_panel == 0,
        );
        Self::draw_single_panel(
            f,
            app.right_panel_mut(),
            right_panel,
            "Right Panel",
            active_panel == 1,
        );
        // Vertical separator │ from path row through bottom bar
        let sep_style = Style::default().bg(MAIN_DARK_BG).fg(Color::White);
        for row in inner.y..(inner.y + panel_content_height + 2) {
            f.render_widget(
                Paragraph::new("│").style(sep_style),
                Rect {
                    x: sep_x,
                    y: row,
                    width: 1,
                    height: 1,
                },
            );
        }
        Self::draw_bottom_file_bar(f, app, bottom_file_rect, sep_x);
        Self::draw_command_line(f, app, command_rect);
        Self::draw_menu_bar(f, menu_rect, app);
    }

    /// Bottom bar: filename (without path) per panel; size info (Ctrl+G) replaces active panel's filename; disk space only on Ctrl+G.
    fn draw_bottom_file_bar(
        f: &mut Frame,
        app: &AppState,
        area: Rect,
        sep_x: u16,
    ) {
        let bar_style = Style::default().bg(MAIN_DARK_BG).fg(Color::White);
        let left_half_w = (sep_x.saturating_sub(area.x)) as usize;
        let right_total_w = area.width.saturating_sub((sep_x - area.x) + 1) as usize;

        let show_disk = app.size_info_dialog.is_some();
        const DISK_W: usize = 20; // "12G / 466G (2%)"
        let right_content_w = if show_disk && right_total_w > DISK_W {
            right_total_w.saturating_sub(DISK_W)
        } else {
            right_total_w
        };

        let size_info_line = crate::size_info_dialog::format_bottom_bar_line(app);
        let active = app.active_panel();

        let left_text = if active == 0 {
            size_info_line
                .clone()
                .unwrap_or_else(|| filename_for_bottom_bar(app.left_panel().get_selected_file()))
        } else {
            filename_for_bottom_bar(app.left_panel().get_selected_file())
        };
        let right_text = if active == 1 {
            size_info_line
                .clone()
                .unwrap_or_else(|| filename_for_bottom_bar(app.right_panel().get_selected_file()))
        } else {
            filename_for_bottom_bar(app.right_panel().get_selected_file())
        };

        let filename_color = Color::Rgb(255, 180, 80); // orange, matches path
        let left_style = if active == 0 && size_info_line.is_some() {
            bar_style.fg(Color::Green)
        } else {
            bar_style.fg(filename_color)
        };
        let right_style = if active == 1 && size_info_line.is_some() {
            bar_style.fg(Color::Green)
        } else {
            bar_style.fg(filename_color)
        };

        let left_trunc: String = left_text.chars().take(left_half_w).collect();
        let left_pad = left_half_w.saturating_sub(left_trunc.chars().count());
        f.render_widget(
            Paragraph::new(format!("{}{}", left_trunc, " ".repeat(left_pad))).style(left_style),
            Rect {
                x: area.x,
                y: area.y,
                width: left_half_w as u16,
                height: 1,
            },
        );
        f.render_widget(
            Paragraph::new("│").style(bar_style),
            Rect {
                x: sep_x,
                y: area.y,
                width: 1,
                height: 1,
            },
        );

        let right_trunc: String = right_text.chars().take(right_content_w).collect();
        let right_pad = right_content_w.saturating_sub(right_trunc.chars().count());
        let right_display = format!("{}{}", right_trunc, " ".repeat(right_pad));
        f.render_widget(
            Paragraph::new(right_display).style(right_style),
            Rect {
                x: sep_x + 1,
                y: area.y,
                width: right_content_w as u16,
                height: 1,
            },
        );

        if show_disk && right_total_w > right_content_w {
            let disk = disk_space_summary(app.get_current_dir());
            let disk_str: String = disk.chars().take(DISK_W).collect();
            let disk_x = sep_x + 1 + right_content_w as u16;
            let disk_rect = Rect {
                x: disk_x,
                y: area.y,
                width: (right_total_w - right_content_w) as u16,
                height: 1,
            };
            f.render_widget(Paragraph::new(disk_str).style(bar_style), disk_rect);
        }
    }

    fn draw_command_line(
        f: &mut Frame,
        app: &AppState,
        area: Rect,
    ) {
        let prompt = "$ ";
        let line = format!("{}{}", prompt, app.command_line);
        let is_focused = app.focus == Focus::CommandLine;
        let base = Style::default().bg(MAIN_DARK_BG);
        let style = base.fg(Color::White);
        let p = Paragraph::new(line.clone()).style(style);
        f.render_widget(p, area);
        if is_focused {
            let cursor_x =
                (prompt.len() + app.command_line_cursor.min(app.command_line.len())) as u16;
            if cursor_x < area.width {
                f.set_cursor_position((area.x + cursor_x, area.y));
            }
        }
    }

    fn draw_single_panel(
        f: &mut Frame,
        panel: &mut Panel,
        area: Rect,
        _title: &str,
        is_active_panel: bool,
    ) {
        match panel.get_view_mode() {
            ViewMode::SingleColumn => {
                Self::draw_single_column_view(f, panel, area, is_active_panel)
            }
            ViewMode::DoubleColumn => {
                Self::draw_double_column_view(f, panel, area, is_active_panel)
            }
        }
    }

    fn draw_single_column_view(
        f: &mut Frame,
        panel: &mut Panel,
        area: Rect,
        is_active_panel: bool,
    ) {
        // One-column view: no header row; data rows have name + size + mtime (like two-column: no redundant left padding).
        // Mark: only "> " when marked (no leading spaces when unmarked, to match two-column).
        const SIZE_W: u16 = 12;
        const MTIME_W: u16 = 17; // "Feb 13 2024 20:05"
        const GAP: u16 = 1;
        const SPACE_BETWEEN_SIZE_MTIME: u16 = 1;
        let name_w = area
            .width
            .saturating_sub(SIZE_W + GAP + MTIME_W + SPACE_BETWEEN_SIZE_MTIME)
            .max(10) as usize;

        let panel_height = (area.height as usize).max(1);
        panel.update_scroll_offset(panel_height);

        let files = panel.get_files();
        let scroll = panel
            .get_scroll_offset()
            .min(files.len().saturating_sub(1).max(0));

        let base = Style::default().bg(MAIN_DARK_BG);

        for (i, file) in files.iter().skip(scroll).take(panel_height).enumerate() {
            let actual_index = i + scroll;
            let is_selected = is_active_panel && actual_index == panel.get_selected_index();
            let is_marked = panel.is_marked(actual_index);

            let mark_cell = if is_marked { "> " } else { "" };
            let name_display = truncate_for_width(file, name_w);
            let size_str = size_display(file);
            let mtime_str = file
                .mtime
                .as_ref()
                .map(format_mtime)
                .unwrap_or_else(String::new);
            let size_pad = format!("{:>1$}", size_str, SIZE_W as usize);
            let mtime_pad = format!("{:>17}", mtime_str); // "Feb 13 2024 20:05" = 17 chars

            let (name_style, mark_style) = if is_selected {
                let sel = Style::default().fg(Color::White).bg(Color::Blue);
                (sel, sel)
            } else if file.is_dir {
                let dir = base.fg(Color::Cyan).add_modifier(Modifier::BOLD);
                (dir, base)
            } else if file.is_executable {
                (base.fg(Color::Green), base)
            } else if is_zip_file(file) {
                (base.fg(Color::Rgb(160, 120, 255)), base)
            } else if file.is_symlink {
                (base.fg(Color::Magenta), base)
            } else {
                (base.fg(Color::White), base)
            };

            let pad_len = (name_w + GAP as usize)
                .saturating_sub(name_display.chars().count())
                .saturating_sub(mark_cell.len());
            let pad_after_name = " ".repeat(pad_len);

            let spans = vec![
                Span::styled(mark_cell, if is_selected { mark_style } else { base }),
                Span::styled(name_display, name_style),
                Span::styled(pad_after_name, if is_selected { mark_style } else { base }),
                Span::styled(
                    size_pad.as_str(),
                    if is_selected {
                        mark_style
                    } else {
                        base.fg(Color::White)
                    },
                ),
                Span::raw(" "),
                Span::styled(
                    mtime_pad.as_str(),
                    if is_selected {
                        mark_style
                    } else {
                        base.fg(Color::White)
                    },
                ),
            ];

            let row = Line::from(spans);
            let row_rect = Rect {
                x: area.x,
                y: area.y + i as u16,
                width: area.width,
                height: 1,
            };
            f.render_widget(Paragraph::new(row), row_rect);
        }
    }

    fn draw_double_column_view(
        f: &mut Frame,
        panel: &mut Panel,
        area: Rect,
        is_active_panel: bool,
    ) {
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
        let visible_files: Vec<_> = files.iter().skip(scroll).take(files_per_page).collect();
        let (left_files, right_files) =
            visible_files.split_at(visible_files.len().min(files_per_column));

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
            let line = styles::create_file_line_from_display(
                &display,
                file.is_dir,
                file.is_symlink,
                file.is_executable,
                is_zip_file(file),
                is_selected,
                is_marked,
            );
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
            let line = styles::create_file_line_from_display(
                &display,
                file.is_dir,
                file.is_symlink,
                file.is_executable,
                is_zip_file(file),
                is_selected,
                is_marked,
            );
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
