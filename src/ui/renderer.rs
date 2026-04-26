use crate::app::state::{
    AppState, ArchiveProgress, CopyErrorState, CopyProgress, Focus, Operation,
};
use crate::browser::diff_viewer::{
    self, clear_folder_compare_if_stale, FolderCompareState, FolderDiffTag,
};
use crate::browser::editor;
use crate::browser::panel::{Panel, PanelOperations, ViewMode};
use crate::browser::viewer;
use crate::core::disk_space::disk_space_summary;
use crate::core::file_ops::FileInfo;
use crate::core::location::archive_format_for_filename;
use crate::core::panel_backend;
use crate::core::text_format::{format_byte_size, truncate_str, TruncateMode};
use crate::dialogs::{
    actions_dialog, archive_dialog, error_detail_dialog, find_dialog, mkdir_dialog, new_file_dialog,
    panel_overlay, pattern_select_dialog, rename_attr, settings_dialog, size_info_dialog,
};
use crate::ui::dialog_layout::{self, paint_modal_dim_layer, DEFAULT_PAD_H};
use crate::ui::menu_bar_key;
use crate::ui::styles;
use crate::ui::text_input;
use crate::ui::theme::{DialogPalette, UiPalette};
use crate::ui::toast::{self, TimedToast};
use ratatui::{
    layout::{Alignment, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Gauge, Paragraph},
    Frame,
};

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
    truncate_str(
        path,
        max_width,
        TruncateMode::CompactMiddle,
    )
}

/// Last path segment of the active panel directory (filesystem or display path).
fn current_dir_basename(dir_display: &str) -> String {
    use std::path::Path;
    let d = dir_display.trim_end_matches('/');
    if d.is_empty() {
        return "/".to_string();
    }
    Path::new(d)
        .file_name()
        .and_then(|s| s.to_str())
        .map(str::to_string)
        .unwrap_or_else(|| d.to_string())
}

/// Sh-style `$`, zsh `%`, fish `>`, or `#` when the session is effectively root (Oxide or subshell PTY).
fn shell_prompt_sigil(app: &AppState) -> &'static str {
    #[cfg(unix)]
    {
        if app.chrome_shows_root_session() {
            return "#";
        }
    }
    let shell = std::env::var("SHELL").unwrap_or_default();
    let s = shell.to_ascii_lowercase();
    if s.contains("fish") {
        ">"
    } else if s.contains("zsh") {
        "%"
    } else {
        "$"
    }
}

/// Panel command-line prefix: current (active panel) path + shell-style prompt character.
/// Width is capped so the typed command remains visible on narrow terminals.
fn format_command_prompt(
    app: &AppState,
    max_cols: usize,
) -> String {
    // Reserve room for `" {sigil} "` (sigil is one ASCII char).
    const MIN_TAIL: usize = 4;
    let max_cols = max_cols.max(MIN_TAIL);
    let cwd = app.get_current_dir();
    let path_part = current_dir_basename(cwd);
    let sigil = shell_prompt_sigil(app);
    let tail = format!(" {} ", sigil);
    let tail_len = tail.chars().count();
    let budget = max_cols.saturating_sub(tail_len).max(1);
    let path_compact = compact_path(&path_part, budget);
    format!("{}{}", path_compact, tail)
}

/// Column offset for byte index `end` in `s` (UTF-8 safe; wide chars count as one column).
fn utf8_prefix_display_cols(
    s: &str,
    end_byte: usize,
) -> u16 {
    s.get(..end_byte.min(s.len()))
        .map(|p| p.chars().count())
        .unwrap_or(0) as u16
}

/// Format mtime as "Feb 13 2024 20:05" (month, day, year, time with zero-padded minutes).
fn format_mtime(t: &std::time::SystemTime) -> String {
    use chrono::{DateTime, Timelike, Utc};
    let datetime: DateTime<Utc> = (*t).into();
    let date = datetime.format("%b %e %Y").to_string();
    let time = format!(
        "{:02}:{:02}",
        datetime.hour(),
        datetime.minute()
    );
    format!("{} {}", date, time)
}

/// Name and optional size label for bottom bar (no path).
fn bottom_bar_filename_parts(file: Option<&FileInfo>) -> (String, Option<String>) {
    match file {
        None => (String::new(), None),
        Some(f) => {
            let name = f.name.trim_end_matches('/').to_string();
            let sz = size_display(f);
            if sz.is_empty() {
                (name, None)
            } else {
                (name, Some(sz))
            }
        }
    }
}

/// Truncate `name` so `name + gap + size` fits within `max_width` characters.
fn truncate_bottom_bar_name(
    name: &str,
    suffix_chars: usize,
    max_width: usize,
) -> String {
    let name_max = max_width.saturating_sub(suffix_chars);
    let n = name.chars().count();
    if n <= name_max {
        return name.to_string();
    }
    if name_max <= 1 {
        return "…".to_string();
    }
    format!(
        "{}…",
        name.chars()
            .take(name_max.saturating_sub(1))
            .collect::<String>()
    )
}

/// Bottom label: filename left, file size flush right in `size_style` (within `max_width` chars per panel).
fn bottom_bar_file_line_padded(
    file: Option<&FileInfo>,
    max_width: usize,
    name_style: Style,
    size_style: Style,
) -> Line<'static> {
    const GAP_MIN: usize = 2;
    let (name, sz_opt) = bottom_bar_filename_parts(file);
    let spans: Vec<Span<'static>> = match sz_opt {
        None => {
            let display = if name.is_empty() {
                String::new()
            } else {
                truncate_bottom_bar_name(&name, 0, max_width)
            };
            let used = display.chars().count();
            let pad = max_width.saturating_sub(used);
            let mut v = vec![Span::styled(display, name_style)];
            if pad > 0 {
                v.push(Span::styled(" ".repeat(pad), name_style));
            }
            v
        }
        Some(sz) => {
            let max_sz = if name.is_empty() {
                max_width
            } else {
                max_width.saturating_sub(GAP_MIN + 1)
            };
            let sz_display = if sz.chars().count() <= max_sz {
                sz
            } else {
                truncate_bottom_bar_name(&sz, 0, max_sz.max(1))
            };
            let sz_len = sz_display.chars().count();
            let name_budget = max_width.saturating_sub(sz_len + GAP_MIN);
            let name_display = if name.is_empty() || name_budget == 0 {
                String::new()
            } else {
                truncate_bottom_bar_name(&name, 0, name_budget)
            };
            let name_len = name_display.chars().count();
            let pad_len = max_width.saturating_sub(name_len + sz_len);
            vec![
                Span::styled(name_display, name_style),
                Span::styled(" ".repeat(pad_len), name_style),
                Span::styled(sz_display, size_style),
            ]
        }
    };
    Line::from(spans)
}

/// Size display: nothing for dirs (including ".."), human-readable for files.
fn size_display(file: &FileInfo) -> String {
    if file.is_dir {
        String::new()
    } else {
        format_byte_size(file.size)
    }
}

fn is_archive_file(file: &FileInfo) -> bool {
    if file.is_dir {
        return false;
    }
    archive_format_for_filename(file.name.trim_end_matches('/')).is_some()
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
        format!(
            "{}…",
            full.chars().take(w).collect::<String>()
        )
    }
}

impl Renderer {
    /// True when a modal dialog or progress overlay is drawn on top of the panel view (F1 Actions–style dim layer).
    fn modal_dim_backdrop_active(app: &AppState) -> bool {
        app.copy_progress.is_some()
            || app.archive_progress.is_some()
            || app.folder_compare_pending.is_some()
            || app.copy_overwrite_dialog.is_some()
            || app.copy_error_dialog.is_some()
            || app.operation_confirm_pending.is_some()
            || app.mkdir_dialog.is_some()
            || app.pattern_select_dialog.is_some()
            || app.archive_dialog.is_some()
            || app.new_file_dialog.is_some()
            || app.new_file_error.is_some()
            || app.rename_attr_dialog.is_some()
            || app.actions_dialog.is_some()
            || app.error_detail.is_some()
            || app.settings_dialog.is_some()
            || app.find_dialog.is_some()
            || app.left_panel_settings_overlay.is_some()
            || app.right_panel_settings_overlay.is_some()
    }

    /// MC-style: panels + status + command line; or viewer (F3) or editor (F4) with optional confirm dialog.
    pub fn draw_ui(
        f: &mut Frame,
        app: &mut AppState,
    ) {
        if app.diff_viewer_screen.is_some() {
            diff_viewer::draw(f, app);
            return;
        }
        if app.viewer_screen.is_some() {
            viewer::draw(f, app);
            return;
        }
        if app.editor_screen.is_some() {
            editor::draw(f, app);
            return;
        }
        Self::draw_panels_view(f, app);
        if Self::modal_dim_backdrop_active(app) {
            paint_modal_dim_layer(f, &app.ui_palette);
        }
        if let Some(ref progress) = app.copy_progress {
            Self::draw_copy_progress(f, progress, &app.ui_palette);
        }
        if let Some(ref progress) = app.archive_progress {
            Self::draw_archive_progress(f, progress, &app.ui_palette);
        }
        if app.folder_compare_pending.is_some() {
            Self::draw_folder_compare_pending(f, &app.ui_palette);
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
            mkdir_dialog::draw(f, app);
        }
        if app.pattern_select_dialog.is_some() {
            pattern_select_dialog::draw(f, app);
        }
        if app.archive_dialog.is_some() {
            archive_dialog::draw(f, app);
        }
        if app.new_file_dialog.is_some() {
            new_file_dialog::draw(f, app);
        }
        if app.new_file_error.is_some() {
            Self::draw_new_file_error_dialog(f, app);
        }
        if app.rename_attr_dialog.is_some() {
            rename_attr::draw(f, app);
        }
        if app.actions_dialog.is_some() {
            actions_dialog::draw(f, app);
        }
        if app.error_detail.is_some() {
            error_detail_dialog::draw(f, app);
        }
        if app.settings_dialog.is_some() {
            settings_dialog::draw(f, app);
        }
        if app.find_dialog.is_some() {
            find_dialog::draw(f, app);
        }
        if app.left_panel_settings_overlay.is_some() || app.right_panel_settings_overlay.is_some() {
            panel_overlay::draw(f, app);
        }
        // Bottom-left timed toast (e.g. Ctrl+X C save layout) — same pattern as editor save.
        if app.viewer_screen.is_none() && app.editor_screen.is_none() {
            TimedToast::clear_if_expired(&mut app.timed_toast);
            if let Some(ref t) = app.timed_toast {
                toast::draw_timed_bottom_left(f, f.area(), &app.ui_palette, t);
            }
        }
    }

    /// Operation confirmation dialog (Copy/Move/Delete): operation alert, Yes/No buttons.
    /// Tab switches focus. Y/y=Yes, N/n/Esc=No. Mouse/touchpad friendly. Y and N highlighted in orange.
    /// For Copy/Move: shows "From" and "To" paths.
    fn draw_operation_confirm_dialog(
        f: &mut Frame,
        app: &AppState,
    ) {
        let d = &app.ui_palette.dialog;
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
        let fill_style = d.fill_style();
        f.render_widget(Clear, rect);
        let block = Block::default()
            .borders(Borders::ALL)
            .title(title)
            .style(fill_style.fg(d.border));
        f.render_widget(block, rect);
        let max_msg_w = content.width as usize;
        let mut row = content.y;
        if show_paths {
            let path_w = max_msg_w.saturating_sub(2);
            let from_str = compact_path(
                params.source_dir.trim_end_matches('/'),
                path_w,
            );
            let to_str = compact_path(
                params.target_dir.trim_end_matches('/'),
                path_w,
            );
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
            Span::styled("Y", Style::default().fg(d.accent)),
            Span::raw("es"),
            Span::raw("  "),
        ]);
        let no_btn = Line::from(vec![
            Span::raw("  "),
            Span::styled("N", Style::default().fg(d.accent)),
            Span::raw("o"),
            Span::raw("  "),
        ]);
        let yes_style = if focus_yes {
            d.focus_row_style()
        } else {
            fill_style
        };
        let no_style = if focus_yes {
            fill_style
        } else {
            d.focus_row_style()
        };
        f.render_widget(
            Paragraph::new(yes_btn).style(yes_style),
            yes_rect,
        );
        f.render_widget(
            Paragraph::new(no_btn).style(no_style),
            no_rect,
        );
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
        dialog: &DialogPalette,
        fill_style: Style,
        start_row: u16,
    ) {
        let focus = focus_index.min(options.len().saturating_sub(1));
        for (i, (num, label)) in options.iter().enumerate() {
            let num_s = num.to_string();
            let line = Line::from(vec![
                Span::styled(
                    num_s.as_str(),
                    Style::default().fg(dialog.accent),
                ),
                Span::raw(format!(". {}", label)),
            ]);
            let opt_rect = Rect {
                x: content.x,
                y: content.y + start_row + i as u16,
                width: content.width,
                height: 1,
            };
            let style = if i == focus {
                dialog.focus_row_style()
            } else {
                fill_style
            };
            f.render_widget(
                Paragraph::new(line).style(style),
                opt_rect,
            );
        }
    }

    /// Copy/move error dialog: grey style like confirm, keys 1–3, Tab/Enter. Keys differ from Yes/No.
    fn draw_copy_error_dialog(
        f: &mut Frame,
        app: &AppState,
        err: &CopyErrorState,
    ) {
        let d = &app.ui_palette.dialog;
        let area = f.area();
        let rect = dialog_layout::centered_dialog_rect(area, 52, 10);
        let content = dialog_layout::dialog_content_rect(rect, DEFAULT_PAD_H);
        let fill_style = d.fill_style();
        f.render_widget(Clear, rect);
        let title = match err.operation {
            Operation::Copy => " Copy error ",
            Operation::Move => " Move error ",
            Operation::Delete => " Delete error ",
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .title(title)
            .style(fill_style.fg(d.border));
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
            d,
            fill_style,
            2,
        );
    }

    /// New file error dialog (e.g. file already exists): grey style, message and 1. OK. Enter/Esc or click OK closes.
    fn draw_new_file_error_dialog(
        f: &mut Frame,
        app: &AppState,
    ) {
        let d = &app.ui_palette.dialog;
        let area = f.area();
        let rect = Self::new_file_error_dialog_rect(area);
        let content = dialog_layout::dialog_content_rect(rect, DEFAULT_PAD_H);
        let fill_style = d.fill_style();
        f.render_widget(Clear, rect);
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Error ")
            .style(fill_style.fg(d.border));
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
        Self::draw_numbered_options(f, content, &ok_opts, 0, d, fill_style, 4);
    }

    /// Outer rect for new file error dialog (must match [`Self::draw_new_file_error_dialog`]).
    pub fn new_file_error_dialog_rect(area: Rect) -> Rect {
        dialog_layout::centered_dialog_rect(area, 52, 8)
    }

    /// Return OK button rect for new file error dialog hit-testing (row at content.y + 4).
    pub fn new_file_error_ok_rect(area: Rect) -> Option<Rect> {
        let rect = Self::new_file_error_dialog_rect(area);
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
        let d = &app.ui_palette.dialog;
        let area = f.area();
        let rect = dialog_layout::centered_dialog_rect(area, 54, 12);
        let content = dialog_layout::dialog_content_rect(rect, DEFAULT_PAD_H);
        let fill_style = d.fill_style();
        f.render_widget(Clear, rect);
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" File exists ")
            .style(fill_style.fg(d.border));
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
            d,
            fill_style,
            2,
        );
    }

    /// MC-style copy progress overlay: navy blue background, wider, centered content, small margins. ESC: Cancel.
    fn draw_copy_progress(
        f: &mut Frame,
        progress: &CopyProgress,
        palette: &UiPalette,
    ) {
        let area = f.area();
        let inner_width = 76usize;
        const PAD_H: u16 = 2;
        let w = (inner_width as u16 + 2 + PAD_H * 2).min(area.width.saturating_sub(4));
        let h = if matches!(
            progress.operation,
            Operation::Copy | Operation::Move
        ) {
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
        let pr = &palette.progress;
        let fill_style = Style::default().bg(pr.background);

        f.render_widget(Clear, rect);
        let title = match progress.operation {
            Operation::Copy => " Copy ",
            Operation::Move => " Move ",
            Operation::Delete => " Delete ",
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .title(title)
            .style(fill_style.fg(pr.border));
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
                Paragraph::new(space_line.as_str()).style(Style::default().bg(pr.background)),
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
            .style(fill_style.fg(pr.section_label))
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
        let path_display = compact_path(
            &progress.current_path,
            max_path_width.max(10),
        );
        let path_para = Paragraph::new(path_display)
            .style(fill_style.fg(pr.path_text))
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
        if matches!(
            progress.operation,
            Operation::Copy | Operation::Move
        ) {
            let tgt_label = Paragraph::new("Target")
                .style(fill_style.fg(pr.section_label))
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
            let target_display = compact_path(
                &progress.target_path,
                max_path_width.max(10),
            );
            let target_para = Paragraph::new(target_display)
                .style(fill_style.fg(pr.path_text))
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
            .gauge_style(Style::default().fg(pr.gauge))
            .ratio(ratio)
            .label(format!(
                "{} / {}",
                progress.current, progress.total
            ));
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
            .style(fill_style.fg(pr.hint))
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
        progress: &ArchiveProgress,
        palette: &UiPalette,
    ) {
        let pr = &palette.progress;
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
        let fill_style = Style::default().bg(pr.background);

        f.render_widget(Clear, rect);
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Archive ")
            .style(fill_style.fg(pr.border));
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
                Paragraph::new(space_line.as_str()).style(Style::default().bg(pr.background)),
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
            .style(fill_style.fg(pr.section_label))
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
        let path_display = compact_path(
            &progress.current_path,
            max_path_width.max(10),
        );
        f.render_widget(
            Paragraph::new(path_display)
                .style(fill_style.fg(pr.path_text))
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
            .style(fill_style.fg(pr.section_label))
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
        let target_display = compact_path(
            &progress.target_path,
            max_path_width.max(10),
        );
        f.render_widget(
            Paragraph::new(target_display)
                .style(fill_style.fg(pr.path_text))
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
            .gauge_style(Style::default().fg(pr.gauge))
            .ratio(ratio)
            .label(format!(
                "{} / {}",
                progress.current, progress.total
            ));
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
            .style(fill_style.fg(pr.hint))
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

    /// Ctrl+X D with no marks: panel listings are being compared in the background (Esc cancels).
    fn draw_folder_compare_pending(
        f: &mut Frame,
        palette: &UiPalette,
    ) {
        let pr = &palette.progress;
        let area = f.area();
        let w = 54u16.min(area.width.saturating_sub(4)).max(40);
        let h = 5u16;
        let x = area.x + (area.width.saturating_sub(w)) / 2;
        let y = area.y + (area.height.saturating_sub(h)) / 2;
        let rect = Rect {
            x,
            y,
            width: w,
            height: h,
        };
        let fill_style = Style::default().bg(pr.background);
        f.render_widget(Clear, rect);
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Compare folders ")
            .style(fill_style.fg(pr.border));
        f.render_widget(block, rect);
        let inner = rect.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });
        let msg = Paragraph::new("Reading files to compare listings…")
            .style(fill_style.fg(pr.path_text))
            .alignment(Alignment::Center);
        f.render_widget(
            msg,
            Rect {
                x: inner.x,
                y: inner.y,
                width: inner.width,
                height: 1,
            },
        );
        let esc = Paragraph::new("ESC: Cancel")
            .style(fill_style.fg(pr.hint))
            .alignment(Alignment::Center);
        f.render_widget(
            esc,
            Rect {
                x: inner.x,
                y: inner.y + 2,
                width: inner.width,
                height: 1,
            },
        );
    }

    /// Menu bar items (label, F-key number). Used for drawing and hit test. Bottom row.
    pub fn menu_bar_items() -> Vec<(&'static str, u16)> {
        vec![
            ("1 Actions", menu_bar_key::ACTIONS),
            ("2 File", menu_bar_key::FILE),
            ("3 View", menu_bar_key::VIEW),
            ("4 Edit", menu_bar_key::EDIT),
            ("5 Copy", menu_bar_key::COPY),
            ("6 Move", menu_bar_key::MOVE),
            ("7 Folder", menu_bar_key::FOLDER),
            ("8 Delete", menu_bar_key::DELETE),
            ("9 Settings", menu_bar_key::SETTINGS),
            ("10 Quit", menu_bar_key::QUIT),
        ]
    }

    pub(crate) fn is_menu_action_available(
        app: &AppState,
        key: u16,
    ) -> bool {
        // While Find file dialog is open, no menu actions are available.
        if app.find_dialog.is_some() {
            return false;
        }
        // In command prompt mode only F10 (Quit) is available.
        if app.focus == Focus::CommandLine {
            return key == menu_bar_key::QUIT;
        }
        match key {
            menu_bar_key::FILE => app
                .active_panel_ref()
                .get_selected_file()
                .map_or(false, |f| !f.is_parent_dir()),
            menu_bar_key::VIEW => app
                .active_panel_ref()
                .get_selected_file()
                .map_or(false, |f| !f.is_dir && !f.is_parent_dir()),
            menu_bar_key::EDIT => {
                panel_backend::supports_edit(&app.get_current_location())
                    && app
                        .active_panel_ref()
                        .get_selected_file()
                        .map_or(false, |f| !f.is_dir && !f.is_parent_dir())
            }
            menu_bar_key::COPY => {
                let (items, ..) = app
                    .active_panel_ref()
                    .get_names_to_copy_with_restore_neighbors();
                !items.is_empty()
            }
            menu_bar_key::MOVE => {
                let source = app.get_current_dir();
                let target = app.get_opposite_panel_dir();
                let (items, ..) = app
                    .active_panel_ref()
                    .get_names_to_copy_with_restore_neighbors();
                source != target && !items.is_empty()
            }
            menu_bar_key::FOLDER => panel_backend::supports_mkdir(&app.get_current_location()),
            menu_bar_key::DELETE => {
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
        let c = &app.ui_palette.chrome;
        let (menu_bg, menu_hotkey, menu_label, menu_unavailable) =
            if app.chrome_shows_root_session() {
                // High-contrast “danger” strip so root sessions are obvious regardless of theme.
                (
                    Color::Rgb(110, 0, 0),
                    Color::Rgb(255, 235, 160),
                    Color::Rgb(255, 220, 220),
                    Color::Rgb(130, 70, 70),
                )
            } else {
                (
                    c.menu_overlay_bg,
                    c.menu_hotkey,
                    c.menu_label,
                    c.menu_unavailable,
                )
            };
        f.render_widget(
            Paragraph::new(" ".repeat(area.width as usize)).style(Style::default().bg(menu_bg)),
            area,
        );
        let items = Self::menu_bar_items();
        let menu_item_count = items.len() as u16;
        if menu_item_count == 0 {
            return;
        }
        let slot_w = area.width / menu_item_count;
        let num_style = Style::default().fg(menu_hotkey).bg(menu_bg);
        let label_style = Style::default().fg(menu_label).bg(menu_bg);
        let unavailable_style = Style::default().fg(menu_unavailable).bg(menu_bg);
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
        clear_folder_compare_if_stale(app);
        let area = f.area();
        let c = app.ui_palette.chrome;
        let main_bg = c.main_background;
        f.render_widget(
            Block::default().style(Style::default().bg(main_bg)),
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
            .border_style(Style::default().fg(c.panel_border_fg).bg(c.panel_border_bg))
            .style(Style::default().bg(main_bg));
        let inner = frame_block.inner(frame_rect);
        f.render_widget(frame_block, frame_rect);

        let left_w = inner.width / 2;
        let right_w = inner.width.saturating_sub(left_w).saturating_sub(1);
        let sep_x = inner.x + left_w;

        // Row 0: path (without filename) at top-left of each panel
        let path_style = Style::default().fg(c.bottom_bar_path);
        let left_path = app.left_panel().get_current_dir();
        let right_path = app.right_panel().get_current_dir();
        let left_path_display = compact_path(
            left_path.trim_end_matches('/'),
            left_w as usize,
        );
        let right_path_display = compact_path(
            right_path.trim_end_matches('/'),
            right_w as usize,
        );
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
        let palette = app.ui_palette;
        // Clone avoids overlapping `&mut AppState` (panels) with `folder_compare` borrows; size is O(entries).
        let folder_compare = app.folder_compare.clone();
        Self::draw_single_panel(
            f,
            app.left_panel_mut(),
            left_panel,
            "Left Panel",
            active_panel == 0,
            &palette,
            true,
            folder_compare.as_ref(),
        );
        Self::draw_single_panel(
            f,
            app.right_panel_mut(),
            right_panel,
            "Right Panel",
            active_panel == 1,
            &palette,
            false,
            folder_compare.as_ref(),
        );
        // Vertical separator │ from path row through bottom bar
        let sep_style = Style::default().bg(main_bg).fg(c.column_separator);
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

    /// Bottom bar: filename left, file size right (second color) per panel; size info (Ctrl+X then S) replaces active side with green size summary; disk space only while that banner is open.
    fn draw_bottom_file_bar(
        f: &mut Frame,
        app: &AppState,
        area: Rect,
        sep_x: u16,
    ) {
        let c = &app.ui_palette.chrome;
        let bar_style = Style::default().bg(c.main_background).fg(c.command_line_fg);
        let left_half_w = (sep_x.saturating_sub(area.x)) as usize;
        let right_total_w = area.width.saturating_sub((sep_x - area.x) + 1) as usize;

        let show_disk = app.size_info_dialog.is_some();
        const DISK_W: usize = 20; // "12G / 466G (2%)"
        let right_content_w = if show_disk && right_total_w > DISK_W {
            right_total_w.saturating_sub(DISK_W)
        } else {
            right_total_w
        };

        let size_info_line = size_info_dialog::format_bottom_bar_line(app);
        let active = app.active_panel();

        let name_style_bar = Style::default().bg(c.main_background).fg(c.bottom_bar_path);
        let size_style_bar = Style::default().bg(c.main_background).fg(c.bottom_bar_size);

        if active == 0 && size_info_line.is_some() {
            let left_text = size_info_line.as_ref().unwrap().clone();
            let left_trunc: String = left_text.chars().take(left_half_w).collect();
            let left_pad = left_half_w.saturating_sub(left_trunc.chars().count());
            f.render_widget(
                Paragraph::new(format!(
                    "{}{}",
                    left_trunc,
                    " ".repeat(left_pad)
                ))
                .style(bar_style.fg(c.bottom_bar_success)),
                Rect {
                    x: area.x,
                    y: area.y,
                    width: left_half_w as u16,
                    height: 1,
                },
            );
        } else {
            let line = bottom_bar_file_line_padded(
                app.left_panel().get_selected_file(),
                left_half_w,
                name_style_bar,
                size_style_bar,
            );
            f.render_widget(
                Paragraph::new(line),
                Rect {
                    x: area.x,
                    y: area.y,
                    width: left_half_w as u16,
                    height: 1,
                },
            );
        }
        f.render_widget(
            Paragraph::new("│").style(bar_style),
            Rect {
                x: sep_x,
                y: area.y,
                width: 1,
                height: 1,
            },
        );

        if active == 1 && size_info_line.is_some() {
            let right_text = size_info_line.as_ref().unwrap().clone();
            let right_trunc: String = right_text.chars().take(right_content_w).collect();
            let right_pad = right_content_w.saturating_sub(right_trunc.chars().count());
            let right_display = format!("{}{}", right_trunc, " ".repeat(right_pad));
            f.render_widget(
                Paragraph::new(right_display).style(bar_style.fg(c.bottom_bar_success)),
                Rect {
                    x: sep_x + 1,
                    y: area.y,
                    width: right_content_w as u16,
                    height: 1,
                },
            );
        } else {
            let line = bottom_bar_file_line_padded(
                app.right_panel().get_selected_file(),
                right_content_w,
                name_style_bar,
                size_style_bar,
            );
            f.render_widget(
                Paragraph::new(line),
                Rect {
                    x: sep_x + 1,
                    y: area.y,
                    width: right_content_w as u16,
                    height: 1,
                },
            );
        }

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
            f.render_widget(
                Paragraph::new(disk_str).style(bar_style),
                disk_rect,
            );
        }
    }

    fn draw_command_line(
        f: &mut Frame,
        app: &AppState,
        area: Rect,
    ) {
        let c = &app.ui_palette.chrome;
        let w = area.width as usize;
        let prompt = format_command_prompt(app, w);
        let line_str = format!("{}{}", prompt, app.command_line);
        let is_focused = app.focus == Focus::CommandLine;
        let base = Style::default().bg(c.main_background);
        let prompt_fg = if is_focused {
            c.command_prompt_active_fg
        } else {
            c.command_prompt_inactive_fg
        };
        let prompt_style = base.fg(prompt_fg);
        let cmd_style = base.fg(c.command_line_fg);
        let byte_pos = prompt.len() + app.command_line_cursor.min(app.command_line.len());
        let cursor_col = utf8_prefix_display_cols(&line_str, byte_pos) as usize;
        let display_offset = if is_focused {
            text_input::horizontal_display_offset(cursor_col, w)
        } else {
            0
        };
        let chars: Vec<char> = line_str.chars().collect();
        let len = chars.len();
        let start = display_offset.min(len);
        let end = (display_offset + w).min(len);
        let prompt_chars = prompt.chars().count();
        let mut spans = Vec::new();
        let mut i = start;
        while i < end {
            let use_prompt = i < prompt_chars;
            let style = if use_prompt { prompt_style } else { cmd_style };
            let mut j = i + 1;
            while j < end {
                let np = j < prompt_chars;
                if np != use_prompt {
                    break;
                }
                j += 1;
            }
            spans.push(Span::styled(
                chars[i..j].iter().collect::<String>(),
                style,
            ));
            i = j;
        }
        let pad = w.saturating_sub(end - start);
        if pad > 0 {
            spans.push(Span::styled(" ".repeat(pad), cmd_style));
        }
        f.render_widget(Paragraph::new(Line::from(spans)), area);
        if is_focused {
            let col_in_view = cursor_col.saturating_sub(display_offset);
            let max_col = w.saturating_sub(1);
            let col = col_in_view.min(max_col);
            f.set_cursor_position((area.x + col as u16, area.y));
        }
    }

    fn draw_single_panel(
        f: &mut Frame,
        panel: &mut Panel,
        area: Rect,
        _title: &str,
        is_active_panel: bool,
        palette: &UiPalette,
        is_left_panel: bool,
        folder_compare: Option<&FolderCompareState>,
    ) {
        match panel.get_view_mode() {
            ViewMode::SingleColumn => Self::draw_single_column_view(
                f,
                panel,
                area,
                is_active_panel,
                palette,
                is_left_panel,
                folder_compare,
            ),
            ViewMode::DoubleColumn => Self::draw_double_column_view(
                f,
                panel,
                area,
                is_active_panel,
                palette,
                is_left_panel,
                folder_compare,
            ),
        }
    }

    fn draw_single_column_view(
        f: &mut Frame,
        panel: &mut Panel,
        area: Rect,
        is_active_panel: bool,
        palette: &UiPalette,
        is_left_panel: bool,
        folder_compare: Option<&FolderCompareState>,
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

        let list = &palette.panel_list;
        let base = Style::default().bg(palette.chrome.main_background);

        for (i, file) in files.iter().skip(scroll).take(panel_height).enumerate() {
            let actual_index = i + scroll;
            let is_selected = is_active_panel && actual_index == panel.get_selected_index();
            let is_marked = panel.is_marked(actual_index);
            let folder_tag = folder_compare.and_then(|fc| fc.tag_for_entry(is_left_panel, file));

            let mark_cell = if is_marked {
                "> "
            } else {
                match folder_tag {
                    Some(FolderDiffTag::ContentDiff) => "C ",
                    Some(FolderDiffTag::SizeDiff) => "S ",
                    Some(FolderDiffTag::AbsentOnOther) => "X ",
                    None => "",
                }
            };
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
                let sel = Style::default().fg(list.selected_fg).bg(list.selected_bg);
                (sel, sel)
            } else if file.is_hidden_dotfile() {
                if file.is_dir {
                    (
                        base.fg(list.hidden_fg).add_modifier(Modifier::BOLD),
                        base,
                    )
                } else {
                    (base.fg(list.hidden_fg), base)
                }
            } else if file.is_dir {
                let dir = base.fg(list.directory_fg).add_modifier(Modifier::BOLD);
                (dir, base)
            } else if is_archive_file(file) {
                (base.fg(list.zip_fg), base)
            } else if file.is_executable {
                (base.fg(list.executable_fg), base)
            } else if file.is_symlink {
                (base.fg(list.symlink_fg), base)
            } else {
                (base.fg(list.file_fg), base)
            };

            let pad_len = (name_w + GAP as usize)
                .saturating_sub(name_display.chars().count())
                .saturating_sub(mark_cell.len());
            let pad_after_name = " ".repeat(pad_len);

            let mark_cell_style = if mark_cell.is_empty() {
                if is_selected {
                    mark_style
                } else {
                    base
                }
            } else {
                Style::default().fg(list.marked_prefix).bg(if is_selected {
                    list.selected_bg
                } else {
                    palette.chrome.main_background
                })
            };

            let spans = vec![
                Span::styled(mark_cell, mark_cell_style),
                Span::styled(name_display, name_style),
                Span::styled(
                    pad_after_name,
                    if is_selected { mark_style } else { base },
                ),
                Span::styled(
                    size_pad.as_str(),
                    if is_selected {
                        mark_style
                    } else if file.is_hidden_dotfile() {
                        base.fg(list.hidden_fg)
                    } else {
                        base.fg(list.file_fg)
                    },
                ),
                Span::raw(" "),
                Span::styled(
                    mtime_pad.as_str(),
                    if is_selected {
                        mark_style
                    } else if file.is_hidden_dotfile() {
                        base.fg(list.hidden_fg)
                    } else {
                        base.fg(list.file_fg)
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
        palette: &UiPalette,
        is_left_panel: bool,
        folder_compare: Option<&FolderCompareState>,
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
            let folder_tag = folder_compare.and_then(|fc| fc.tag_for_entry(is_left_panel, file));
            let display = truncate_for_width(file, max_left_w);
            let line = styles::create_file_line_from_display(
                &display,
                &palette.panel_list,
                file.is_dir,
                file.is_symlink,
                file.is_executable,
                is_archive_file(file),
                is_selected,
                is_marked,
                folder_tag,
                file.is_hidden_dotfile(),
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
            let folder_tag = folder_compare.and_then(|fc| fc.tag_for_entry(is_left_panel, file));
            let display = truncate_for_width(file, max_right_w);
            let line = styles::create_file_line_from_display(
                &display,
                &palette.panel_list,
                file.is_dir,
                file.is_symlink,
                file.is_executable,
                is_archive_file(file),
                is_selected,
                is_marked,
                folder_tag,
                file.is_hidden_dotfile(),
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
                Paragraph::new("│").style(Style::default().fg(palette.chrome.column_separator)),
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
