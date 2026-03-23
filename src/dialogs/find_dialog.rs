//! Ctrl+F Find file dialog (MC-style). Parameter form (start dir, file pattern, content pattern,
//! options), then results list. File pattern uses wildcards (*, ?) or regex per F9 Settings.
//! In wildcard mode, `|` separates alternative globs (e.g. `a*|b?`). **Ignore pattern** uses the same
//! rules but is matched against the path relative to the start directory (exclude matching paths).
//! Optional content search.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;

use crate::core::find::run_find_search;
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::{
    layout::{Margin, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
    Frame,
};

use crate::app::state::AppState;
use crate::browser::clipboard;
use crate::browser::panel::PanelOperations;
use crate::ui::styles::{
    DIALOG_INPUT_BG_FOCUSED, DIALOG_INPUT_BG_UNFOCUSED, DIALOG_INPUT_SELECTION_BG,
};
use crate::ui::text_input::{self, TextInputState};

pub use crate::core::find::{build_display_rows, FindDisplayRow, FindMessage, FindResult};

/// Phase of the Find file dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindDialogPhase {
    /// Parameter form: start dir, pattern, content, options.
    Parameter,
    /// Search running; results accumulating.
    Searching,
    /// Search done; showing results list.
    Results,
}

/// State for Ctrl+F Find file dialog.
#[derive(Debug)]
pub struct FindDialogState {
    pub phase: FindDialogPhase,
    pub start_dir_input: TextInputState,
    pub file_pattern_input: TextInputState,
    /// Same wildcard/regex semantics as file pattern; matched against relative path to exclude hits.
    pub ignore_pattern_input: TextInputState,
    pub content_pattern_input: TextInputState,
    pub recursive: bool,
    pub file_case_sens: bool,
    pub content_case_sens: bool,
    pub skip_hidden: bool,
    pub results: Vec<FindResult>,
    pub selected_index: usize,
    pub scroll_offset: usize,
    pub status_message: String,
    /// Current directory being searched (shown during search; cleared when done).
    pub search_current_dir: String,
    /// Visible list rows (set from dialog inner height in draw). Used for scroll math.
    pub visible_list_rows: usize,
    /// Focus in parameter form: 0=start_dir, 1=file_pattern, 2=ignore_pattern, 3=content, 4–7 options, 8=Find, 9=Cancel.
    pub focus: usize,
    /// Search params moved into Arc during search; restored to inputs when Done (avoids cloning strings).
    pub search_start_dir: Option<Arc<str>>,
    pub search_file_pattern: Option<Arc<str>>,
    pub search_ignore_pattern: Option<Arc<str>>,
    pub search_content_pattern: Option<Arc<str>>,
}

/// Open Ctrl+F Find file dialog with start dir from active panel.
pub fn open(app: &mut AppState) {
    use crate::app::state::Focus;
    let start_dir = app.active_panel_ref().get_current_dir();
    app.find_dialog = Some(FindDialogState {
        phase: FindDialogPhase::Parameter,
        start_dir_input: TextInputState::new(start_dir.to_string()),
        file_pattern_input: TextInputState::new(app.last_file_name_pattern.clone()),
        ignore_pattern_input: TextInputState::new(String::new()),
        content_pattern_input: TextInputState::new(String::new()),
        recursive: true,
        file_case_sens: false,
        content_case_sens: false,
        skip_hidden: true,
        results: Vec::new(),
        selected_index: 0,
        scroll_offset: 0,
        status_message: String::new(),
        search_current_dir: String::new(),
        visible_list_rows: 18,
        focus: 0,
        search_start_dir: None,
        search_file_pattern: None,
        search_ignore_pattern: None,
        search_content_pattern: None,
    });
    app.find_search_rx = None;
    app.focus = Focus::FindDialog;
}

/// Close the Find file dialog and return focus to panel.
/// If a search was in progress, signals it to stop so the background thread exits.
pub fn close(app: &mut AppState) {
    use crate::app::state::Focus;
    if let Some(cancel_flag) = app.find_search_cancel.take() {
        cancel_flag.store(true, Ordering::Relaxed);
    }
    if let Some(d) = app.find_dialog.take() {
        app.set_last_file_name_pattern(&d.file_pattern_input.text);
    }
    app.find_search_rx = None;
    app.focus = Focus::Panel;
}

/// Start the find search in a background thread; app.find_dialog must be Some.
pub fn start_search(app: &mut AppState) {
    let (
        start_dir,
        file_pattern,
        ignore_pattern,
        content_pattern,
        recursive,
        file_case_sens,
        content_case_sens,
        skip_hidden,
    ) = {
        let Some(dialog) = app.find_dialog.as_mut() else {
            return;
        };
        dialog.phase = FindDialogPhase::Searching;
        dialog.results.clear();
        dialog.status_message = "Searching...".to_string();
        dialog.search_current_dir.clear();
        dialog.selected_index = 0;
        dialog.scroll_offset = 0;

        // Move strings into Arc<str> (no clone); pass Arc::clone to thread (cheap). Restore from stored Arc when Done.
        let start_dir = Arc::from(std::mem::take(&mut dialog.start_dir_input.text));
        let file_pattern = Arc::from(std::mem::take(&mut dialog.file_pattern_input.text));
        let ignore_pattern = Arc::from(std::mem::take(&mut dialog.ignore_pattern_input.text));
        let content_pattern = Arc::from(std::mem::take(&mut dialog.content_pattern_input.text));
        dialog.search_start_dir = Some(Arc::clone(&start_dir));
        dialog.search_file_pattern = Some(Arc::clone(&file_pattern));
        dialog.search_ignore_pattern = Some(Arc::clone(&ignore_pattern));
        dialog.search_content_pattern = Some(Arc::clone(&content_pattern));

        let recursive = dialog.recursive;
        let file_case_sens = dialog.file_case_sens;
        let content_case_sens = dialog.content_case_sens;
        let skip_hidden = dialog.skip_hidden;
        (
            start_dir,
            file_pattern,
            ignore_pattern,
            content_pattern,
            recursive,
            file_case_sens,
            content_case_sens,
            skip_hidden,
        )
    };

    app.set_last_file_name_pattern(file_pattern.as_ref());
    let file_pattern_regex = app.persisted_settings.file_pattern_uses_regex();

    let cancel = Arc::new(AtomicBool::new(false));
    app.find_search_cancel = Some(Arc::clone(&cancel));

    let (tx, rx) = mpsc::channel();
    run_find_search(
        start_dir,
        file_pattern,
        ignore_pattern,
        content_pattern,
        recursive,
        file_case_sens,
        file_pattern_regex,
        content_case_sens,
        skip_hidden,
        cancel,
        tx,
    );
    app.find_search_rx = Some(rx);
}

/// Poll the search channel and append to find_dialog results; switch to Results when Done.
pub fn poll_search(app: &mut AppState) {
    let rx = match app.find_search_rx.as_ref() {
        Some(r) => r,
        None => return,
    };
    while let Ok(msg) = rx.try_recv() {
        match msg {
            FindMessage::Match(path, line) => {
                if let Some(ref mut find_dialog) = app.find_dialog {
                    find_dialog.results.push(FindResult { path, line });
                }
            }
            FindMessage::CurrentDir(s) => {
                if let Some(ref mut find_dialog) = app.find_dialog {
                    find_dialog.search_current_dir = s;
                }
            }
            FindMessage::Done => {
                if let Some(ref mut find_dialog) = app.find_dialog {
                    find_dialog.phase = FindDialogPhase::Results;
                    let match_count = find_dialog.results.len();
                    find_dialog.status_message =
                        format!("Search complete. {} match(es).", match_count);
                    find_dialog.search_current_dir.clear();
                    // Restore search params from Arc into input fields (one copy per field when done).
                    if let Some(arc) = find_dialog.search_start_dir.take() {
                        find_dialog.start_dir_input.text = arc.to_string();
                        find_dialog.start_dir_input.cursor =
                            find_dialog.start_dir_input.text.chars().count();
                        find_dialog.start_dir_input.anchor = None;
                    }
                    if let Some(arc) = find_dialog.search_file_pattern.take() {
                        find_dialog.file_pattern_input.text = arc.to_string();
                        find_dialog.file_pattern_input.cursor =
                            find_dialog.file_pattern_input.text.chars().count();
                        find_dialog.file_pattern_input.anchor = None;
                    }
                    if let Some(arc) = find_dialog.search_ignore_pattern.take() {
                        find_dialog.ignore_pattern_input.text = arc.to_string();
                        find_dialog.ignore_pattern_input.cursor =
                            find_dialog.ignore_pattern_input.text.chars().count();
                        find_dialog.ignore_pattern_input.anchor = None;
                    }
                    if let Some(arc) = find_dialog.search_content_pattern.take() {
                        find_dialog.content_pattern_input.text = arc.to_string();
                        find_dialog.content_pattern_input.cursor =
                            find_dialog.content_pattern_input.text.chars().count();
                        find_dialog.content_pattern_input.anchor = None;
                    }
                }
                app.find_search_rx = None;
                app.find_search_cancel = None;
                return;
            }
        }
    }
}

/// Handle key when Find dialog is open.
pub fn handle_key(
    app: &mut AppState,
    code: KeyCode,
    modifiers: KeyModifiers,
) -> Option<crate::app::events::AppAction> {
    use crate::app::events::AppAction;
    let phase = app
        .find_dialog
        .as_ref()
        .map(|find_dialog| find_dialog.phase)?;
    match phase {
        FindDialogPhase::Parameter => handle_key_parameter(app, code, modifiers),
        FindDialogPhase::Searching | FindDialogPhase::Results => {
            // ESC while searching: stop the search but keep the dialog open with results so far.
            if phase == FindDialogPhase::Searching && code == KeyCode::Esc {
                if let Some(ref mut find_dialog) = app.find_dialog {
                    find_dialog.phase = FindDialogPhase::Results;
                    let match_count = find_dialog.results.len();
                    find_dialog.status_message =
                        format!("Stopped. {} match(es).", match_count);
                    find_dialog.search_current_dir.clear();
                }
                app.find_search_rx = None;
                return Some(AppAction::Continue);
            }
            // ESC when search completed (Results): close dialog. Other keys: list navigation, Enter/F3.
            handle_key_results(app, code, modifiers)
        }
    }
}

fn handle_key_parameter(
    app: &mut AppState,
    code: KeyCode,
    modifiers: KeyModifiers,
) -> Option<crate::app::events::AppAction> {
    use crate::app::events::AppAction;
    let dialog = app.find_dialog.as_mut()?;
    if modifiers.contains(KeyModifiers::CONTROL) {
        if code == KeyCode::Char('a') && dialog.focus <= 3 {
            let input = match dialog.focus {
                0 => &mut dialog.start_dir_input,
                1 => &mut dialog.file_pattern_input,
                2 => &mut dialog.ignore_pattern_input,
                3 => &mut dialog.content_pattern_input,
                _ => return Some(AppAction::Continue),
            };
            if !input.text.is_empty() {
                *input = std::mem::take(input).select_all();
            }
            return Some(AppAction::Continue);
        }
        if code == KeyCode::Char('v') && dialog.focus <= 3 {
            let input = match dialog.focus {
                0 => &mut dialog.start_dir_input,
                1 => &mut dialog.file_pattern_input,
                2 => &mut dialog.ignore_pattern_input,
                3 => &mut dialog.content_pattern_input,
                _ => return Some(AppAction::Continue),
            };
            if let Some(s) = clipboard::get() {
                *input = std::mem::take(input).insert_str(&s);
            }
            return Some(AppAction::Continue);
        }
        if code == KeyCode::Char('c') {
            if dialog.focus <= 3 {
                let text = match dialog.focus {
                    0 => dialog
                        .start_dir_input
                        .get_selected_text()
                        .unwrap_or_else(|| dialog.start_dir_input.text.clone()),
                    1 => dialog
                        .file_pattern_input
                        .get_selected_text()
                        .unwrap_or_else(|| dialog.file_pattern_input.text.clone()),
                    2 => dialog
                        .ignore_pattern_input
                        .get_selected_text()
                        .unwrap_or_else(|| dialog.ignore_pattern_input.text.clone()),
                    3 => dialog
                        .content_pattern_input
                        .get_selected_text()
                        .unwrap_or_else(|| dialog.content_pattern_input.text.clone()),
                    _ => String::new(),
                };
                if !text.is_empty() {
                    clipboard::set(&text);
                }
                return Some(AppAction::Continue);
            }
            close(app);
            return Some(AppAction::FindClose);
        }
    }
    match code {
        KeyCode::Esc => return Some(AppAction::FindClose),
        KeyCode::Tab | KeyCode::Down => {
            dialog.focus = (dialog.focus + 1) % 10;
            return Some(AppAction::Continue);
        }
        KeyCode::BackTab | KeyCode::Up => {
            dialog.focus = (dialog.focus + 9) % 10;
            return Some(AppAction::Continue);
        }
        KeyCode::Enter => {
            if dialog.focus == 9 {
                return Some(AppAction::FindClose);
            }
            // Enter from any other widget (inputs 0–3, options 4–7, Find 8) starts the search
            return Some(AppAction::FindStartSearch);
        }
        KeyCode::Char(' ') => {
            if dialog.focus >= 4 && dialog.focus <= 7 {
                match dialog.focus {
                    4 => dialog.recursive = !dialog.recursive,
                    5 => dialog.file_case_sens = !dialog.file_case_sens,
                    6 => dialog.content_case_sens = !dialog.content_case_sens,
                    7 => dialog.skip_hidden = !dialog.skip_hidden,
                    _ => {}
                }
                return Some(AppAction::Continue);
            }
        }
        KeyCode::Char(c) => {
            if c.is_ascii() && !c.is_control() && dialog.focus <= 3 {
                let input = match dialog.focus {
                    0 => &mut dialog.start_dir_input,
                    1 => &mut dialog.file_pattern_input,
                    2 => &mut dialog.ignore_pattern_input,
                    3 => &mut dialog.content_pattern_input,
                    _ => return Some(AppAction::Continue),
                };
                *input = std::mem::take(input).insert_char(c);
            }
        }
        KeyCode::Backspace => {
            if dialog.focus <= 3 {
                let input = match dialog.focus {
                    0 => &mut dialog.start_dir_input,
                    1 => &mut dialog.file_pattern_input,
                    2 => &mut dialog.ignore_pattern_input,
                    3 => &mut dialog.content_pattern_input,
                    _ => return Some(AppAction::Continue),
                };
                *input = std::mem::take(input).backspace();
            }
        }
        KeyCode::Left => {
            if dialog.focus <= 3 {
                let shift = modifiers.contains(KeyModifiers::SHIFT);
                let input = match dialog.focus {
                    0 => &mut dialog.start_dir_input,
                    1 => &mut dialog.file_pattern_input,
                    2 => &mut dialog.ignore_pattern_input,
                    3 => &mut dialog.content_pattern_input,
                    _ => return Some(AppAction::Continue),
                };
                *input = std::mem::take(input).move_left(shift);
            }
        }
        KeyCode::Right => {
            if dialog.focus <= 3 {
                let shift = modifiers.contains(KeyModifiers::SHIFT);
                let input = match dialog.focus {
                    0 => &mut dialog.start_dir_input,
                    1 => &mut dialog.file_pattern_input,
                    2 => &mut dialog.ignore_pattern_input,
                    3 => &mut dialog.content_pattern_input,
                    _ => return Some(AppAction::Continue),
                };
                *input = std::mem::take(input).move_right(shift);
            }
        }
        KeyCode::Home => {
            if dialog.focus <= 3 {
                let shift = modifiers.contains(KeyModifiers::SHIFT);
                let input = match dialog.focus {
                    0 => &mut dialog.start_dir_input,
                    1 => &mut dialog.file_pattern_input,
                    2 => &mut dialog.ignore_pattern_input,
                    3 => &mut dialog.content_pattern_input,
                    _ => return Some(AppAction::Continue),
                };
                *input = std::mem::take(input).move_home(shift);
            }
        }
        KeyCode::End => {
            if dialog.focus <= 3 {
                let shift = modifiers.contains(KeyModifiers::SHIFT);
                let input = match dialog.focus {
                    0 => &mut dialog.start_dir_input,
                    1 => &mut dialog.file_pattern_input,
                    2 => &mut dialog.ignore_pattern_input,
                    3 => &mut dialog.content_pattern_input,
                    _ => return Some(AppAction::Continue),
                };
                *input = std::mem::take(input).move_end(shift);
            }
        }
        _ => {}
    }
    Some(AppAction::Continue)
}

fn handle_key_results(
    app: &mut AppState,
    code: KeyCode,
    _modifiers: KeyModifiers,
) -> Option<crate::app::events::AppAction> {
    use crate::app::events::AppAction;
    let dialog = app.find_dialog.as_mut()?;
    let display_rows = build_display_rows(&dialog.results);
    let len = display_rows.len();
    if len > 0 && dialog.selected_index >= len {
        dialog.selected_index = len - 1;
    }
    match code {
        KeyCode::Esc => return Some(AppAction::FindClose),
        KeyCode::Enter => return Some(AppAction::FindChdir),
        KeyCode::F(3) => {
            if len > 0 {
                if let Some(FindDisplayRow::File(_)) = display_rows.get(dialog.selected_index) {
                    return Some(AppAction::FindView);
                }
            }
        }
        KeyCode::F(4) => {
            if len > 0 {
                if let Some(FindDisplayRow::File(_)) = display_rows.get(dialog.selected_index) {
                    return Some(AppAction::FindEdit);
                }
            }
        }
        KeyCode::Up => {
            if len > 0 {
                dialog.selected_index = dialog.selected_index.saturating_sub(1);
                if dialog.selected_index < dialog.scroll_offset {
                    dialog.scroll_offset = dialog.selected_index;
                }
            }
        }
        KeyCode::Down => {
            if len > 0 {
                dialog.selected_index = (dialog.selected_index + 1).min(len - 1);
                let max_visible = dialog.visible_list_rows.max(1);
                if dialog.selected_index >= dialog.scroll_offset + max_visible {
                    dialog.scroll_offset = dialog.selected_index - max_visible + 1;
                }
            }
        }
        KeyCode::PageUp => {
            if len > 0 {
                let visible_rows = dialog.visible_list_rows.max(1);
                dialog.selected_index = dialog
                    .selected_index
                    .saturating_sub(visible_rows)
                    .max(0);
                if dialog.selected_index < dialog.scroll_offset {
                    dialog.scroll_offset = dialog.selected_index;
                }
            }
        }
        KeyCode::PageDown => {
            if len > 0 {
                let visible_rows = dialog.visible_list_rows.max(1);
                dialog.selected_index = (dialog.selected_index + visible_rows).min(len - 1);
                if dialog.selected_index >= dialog.scroll_offset + visible_rows {
                    dialog.scroll_offset = dialog.selected_index - visible_rows + 1;
                }
            }
        }
        KeyCode::Home => {
            dialog.selected_index = 0;
            dialog.scroll_offset = 0;
        }
        KeyCode::End => {
            if len > 0 {
                dialog.selected_index = len - 1;
                dialog.scroll_offset = len.saturating_sub(dialog.visible_list_rows.max(1));
            }
        }
        _ => {}
    }
    Some(AppAction::Continue)
}

const FIND_DIALOG_W: u16 = 110;
const FIND_DIALOG_H_PARAM: u16 = 19;
const FIND_DIALOG_H_RESULTS: u16 = 28;
const STATUS_ROWS: u16 = 2; // status line + gap
const HINT_ROWS: u16 = 1;

/// Draw the Find file dialog (parameter form or results list).
pub fn draw(
    f: &mut Frame,
    app: &mut AppState,
) {
    poll_search(app);
    let file_pattern_regex = app.persisted_settings.file_pattern_uses_regex();
    let Some(dialog) = app.find_dialog.as_mut() else {
        return;
    };
    let area = f.area();
    let (w, h) = match dialog.phase {
        FindDialogPhase::Parameter => (FIND_DIALOG_W, FIND_DIALOG_H_PARAM),
        FindDialogPhase::Searching | FindDialogPhase::Results => {
            (FIND_DIALOG_W, FIND_DIALOG_H_RESULTS)
        }
    };
    let x = area.x + area.width.saturating_sub(w) / 2;
    let y = area.y + area.height.saturating_sub(h) / 2;
    let rect = ratatui::layout::Rect {
        x,
        y,
        width: w,
        height: h,
    };
    let grey_bg = Color::Rgb(60, 60, 60);
    let fill_style = Style::default().bg(grey_bg).fg(Color::White);
    f.render_widget(Clear, rect);
    let title = match dialog.phase {
        FindDialogPhase::Parameter => " Find file ",
        FindDialogPhase::Searching => " Find file (searching...) ",
        FindDialogPhase::Results => " Find file (results) ",
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
    let content_w = inner.width.saturating_sub(2);

    match dialog.phase {
        FindDialogPhase::Parameter => {
            draw_parameter_form(
                f,
                dialog,
                inner,
                content_w,
                fill_style,
                file_pattern_regex,
            );
        }
        FindDialogPhase::Searching => {
            draw_status_and_list(f, dialog, inner, content_w, fill_style, true);
        }
        FindDialogPhase::Results => {
            draw_status_and_list(f, dialog, inner, content_w, fill_style, false);
        }
    }
}

fn draw_parameter_form(
    f: &mut Frame,
    dialog: &FindDialogState,
    inner: Rect,
    content_w: u16,
    fill_style: Style,
    file_pattern_regex: bool,
) {
    let cx = inner.x + 1;
    let mut row = inner.y;

    let file_pattern_label = if file_pattern_regex {
        "File pattern (regex):"
    } else {
        "File pattern (*, ?):"
    };
    let ignore_pattern_label = if file_pattern_regex {
        "Ignore pattern (regex):"
    } else {
        "Ignore pattern (*, ?):"
    };
    let rows: [(usize, &str, &TextInputState); 4] = [
        (0, "Start directory:", &dialog.start_dir_input),
        (1, file_pattern_label, &dialog.file_pattern_input),
        (2, ignore_pattern_label, &dialog.ignore_pattern_input),
        (3, "Content pattern:", &dialog.content_pattern_input),
    ];
    let label_w = rows
        .iter()
        .map(|(_, l, _)| l.chars().count())
        .max()
        .unwrap_or(18) as u16;

    for (focus_idx, label, input) in rows {
        let cursor_char = input.cursor_column();
        let focused = dialog.focus == focus_idx;
        let bg = if focused {
            DIALOG_INPUT_BG_FOCUSED
        } else {
            DIALOG_INPUT_BG_UNFOCUSED
        };
        let value_w = content_w.saturating_sub(label_w);
        let value_w_usize = value_w as usize;
        let display_offset = if value_w_usize == 0 {
            0
        } else if cursor_char + 1 <= value_w_usize {
            0
        } else {
            cursor_char + 1 - value_w_usize
        };
        let cursor_screen = cursor_char.saturating_sub(display_offset);
        let base_style = Style::default().bg(bg).fg(Color::White);
        let selection_style = Style::default()
            .bg(DIALOG_INPUT_SELECTION_BG)
            .fg(Color::White);
        let line = text_input::input_line_with_selection_slice(
            input,
            display_offset,
            value_w_usize,
            base_style,
            selection_style,
        );
        // Label and input on one line
        f.render_widget(
            Paragraph::new(label).style(fill_style),
            Rect {
                x: cx,
                y: row,
                width: label_w,
                height: 1,
            },
        );
        f.render_widget(
            Paragraph::new(line),
            Rect {
                x: cx + label_w,
                y: row,
                width: value_w,
                height: 1,
            },
        );
        if focused {
            let cursor_x = cx + label_w + (cursor_screen as u16).min(value_w.saturating_sub(1));
            f.set_cursor_position((cursor_x, row));
        }
        row += 1;
    }
    row += 1;

    let opts = [
        (4, "Recursive", dialog.recursive),
        (5, "File name case sensitive", dialog.file_case_sens),
        (6, "Content case sensitive", dialog.content_case_sens),
        (7, "Skip hidden files", dialog.skip_hidden),
    ];
    for (idx, label, on) in opts {
        let mark = if on { "[x]" } else { "[ ]" };
        let style = if dialog.focus == idx {
            Style::default().bg(Color::Cyan).fg(Color::Black)
        } else {
            fill_style
        };
        f.render_widget(
            Paragraph::new(format!("{} {}", mark, label)).style(style),
            Rect {
                x: cx,
                y: row,
                width: content_w,
                height: 1,
            },
        );
        row += 1;
    }
    row += 1;

    let btn_style = |idx: usize| {
        if dialog.focus == idx {
            Style::default().bg(Color::Cyan).fg(Color::Black)
        } else {
            fill_style
        }
    };
    f.render_widget(
        Paragraph::new("  Find  ").style(btn_style(8)),
        Rect {
            x: cx,
            y: row,
            width: 8,
            height: 1,
        },
    );
    f.render_widget(
        Paragraph::new("  Cancel  ").style(btn_style(9)),
        Rect {
            x: cx + 10,
            y: row,
            width: 10,
            height: 1,
        },
    );
}

fn draw_status_and_list(
    f: &mut Frame,
    dialog: &mut FindDialogState,
    inner: Rect,
    content_w: u16,
    fill_style: Style,
    searching: bool,
) {
    let cx = inner.x + 1;
    let mut row = inner.y;
    let show_hint = !searching && !dialog.results.is_empty();
    let list_height = inner
        .height
        .saturating_sub(STATUS_ROWS)
        .saturating_sub(if show_hint { HINT_ROWS } else { 0 })
        .max(1);
    dialog.visible_list_rows = list_height as usize;
    let visible_row_count = dialog.visible_list_rows.max(1);
    let display_rows = build_display_rows(&dialog.results);
    let len = display_rows.len();
    if len > 0 {
        if dialog.selected_index >= len {
            dialog.selected_index = len - 1;
        }
        if dialog.scroll_offset + visible_row_count > len {
            dialog.scroll_offset = len.saturating_sub(visible_row_count);
        }
        if dialog.selected_index < dialog.scroll_offset {
            dialog.scroll_offset = dialog.selected_index;
        }
        if dialog.selected_index >= dialog.scroll_offset + visible_row_count {
            dialog.scroll_offset = dialog.selected_index - visible_row_count + 1;
        }
    }

    if searching {
        // One line: current directory (or "Scanning...") and number of found items; suffix fixed so it doesn't jump.
        let found_count = dialog.results.len();
        let suffix = format!("  {} found", found_count);
        let suffix_len = suffix.chars().count();
        let path_w = (content_w as usize).saturating_sub(suffix_len).max(0);
        let path_display = if dialog.search_current_dir.is_empty() {
            "Scanning...".to_string()
        } else {
            truncate_path(&dialog.search_current_dir, path_w)
        };
        let line = format!("{:<path_w$}{}", path_display, suffix);
        f.render_widget(
            Paragraph::new(line).style(fill_style),
            Rect {
                x: cx,
                y: row,
                width: content_w,
                height: 1,
            },
        );
    } else {
        // Search complete: show status only.
        f.render_widget(
            Paragraph::new(dialog.status_message.as_str()).style(fill_style),
            Rect {
                x: cx,
                y: row,
                width: content_w,
                height: 1,
            },
        );
    }
    row += STATUS_ROWS;

    let list_rect = Rect {
        x: cx,
        y: row,
        width: content_w,
        height: list_height,
    };
    let visible: Vec<ListItem> = display_rows
        .iter()
        .skip(dialog.scroll_offset)
        .take(dialog.visible_list_rows)
        .enumerate()
        .map(|(i, row)| {
            let idx = dialog.scroll_offset + i;
            let (line_str, is_folder) = match row {
                FindDisplayRow::Folder(path) => (path.display().to_string(), true),
                FindDisplayRow::File(r) => {
                    let name = r
                        .path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| r.path.display().to_string());
                    let s = match &r.line {
                        Some(l) => format!("{}:{}", name, l),
                        None => name,
                    };
                    (s, false)
                }
            };
            let style = if idx == dialog.selected_index {
                Style::default().bg(Color::Cyan).fg(Color::Black)
            } else {
                fill_style
            };
            let prefix = if is_folder { "" } else { "  " };
            let max_w = (content_w as usize).saturating_sub(prefix.len());
            let display = format!("{}{}", prefix, truncate_path(&line_str, max_w));
            ListItem::new(Line::from(Span::raw(display))).style(style)
        })
        .collect();
    let list = List::new(visible);
    f.render_widget(list, list_rect);

    if show_hint {
        row += list_rect.height + 1;
        let hint = "Enter: Chdir  F3: View  F4: Edit  Esc: Close";
        f.render_widget(
            Paragraph::new(hint).style(fill_style.fg(Color::DarkGray)),
            Rect {
                x: cx,
                y: row,
                width: content_w,
                height: 1,
            },
        );
    }
}

fn truncate_path(
    s: &str,
    max: usize,
) -> String {
    crate::core::text_format::truncate_str(
        s,
        max,
        crate::core::text_format::TruncateMode::SuffixEllipsis,
    )
}

#[cfg(test)]
mod tests {
    use crate::core::find::glob_match;

    #[test]
    fn glob_star_offline_zip_matches_offline_zip() {
        assert!(glob_match("*offline.zip", "offline.zip", true));
        assert!(glob_match("*offline.zip", "offline.zip", false));
    }

    #[test]
    fn glob_exact_and_star_prefix() {
        assert!(glob_match("offline.zip", "offline.zip", true));
        assert!(glob_match("*", "offline.zip", true));
        assert!(glob_match("*.zip", "offline.zip", true));
        assert!(glob_match("", "anything", true));
    }
}
