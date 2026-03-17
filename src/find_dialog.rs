//! Ctrl+F Find file dialog (MC-style). Parameter form (start dir, file pattern, content pattern,
//! options), then results list. Shell-style wildcards (*, ?). Optional content search.

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use walkdir::WalkDir;

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::{
    layout::{Margin, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
    Frame,
};

use crate::app_state::AppState;
use crate::panel::PanelOperations;
use crate::styles::{DIALOG_INPUT_BG_FOCUSED, DIALOG_INPUT_BG_UNFOCUSED};
use crate::text_input::TextInputState;

/// One result from Find file: path and optional line number (when content search matched).
#[derive(Debug, Clone)]
pub struct FindResult {
    pub path: PathBuf,
    pub line: Option<u64>,
}

/// Message from the find search background thread.
#[derive(Debug)]
pub enum FindMessage {
    Match(PathBuf, Option<u64>),
    /// Current directory being traversed (empty when search is done).
    CurrentDir(String),
    Done,
}

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
    /// Focus in parameter form: 0=start_dir, 1=file_pattern, 2=content, 3=recursive, 4=file_case, 5=content_case, 6=skip_hidden, 7=Find, 8=Cancel.
    pub focus: usize,
    /// Search params moved into Arc during search; restored to inputs when Done (avoids cloning strings).
    pub search_start_dir: Option<Arc<str>>,
    pub search_file_pattern: Option<Arc<str>>,
    pub search_content_pattern: Option<Arc<str>>,
}

/// Open Ctrl+F Find file dialog with start dir from active panel.
pub fn open(app: &mut AppState) {
    use crate::app_state::Focus;
    let start_dir = app.active_panel_ref().get_current_dir();
    app.find_dialog = Some(FindDialogState {
        phase: FindDialogPhase::Parameter,
        start_dir_input: TextInputState::new(start_dir.to_string()),
        file_pattern_input: TextInputState::new(String::new()),
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
        search_content_pattern: None,
    });
    app.find_search_rx = None;
    app.focus = Focus::FindDialog;
}

/// Close the Find file dialog and return focus to panel.
/// If a search was in progress, signals it to stop so the background thread exits.
pub fn close(app: &mut AppState) {
    use crate::app_state::Focus;
    if let Some(c) = app.find_search_cancel.take() {
        c.store(true, Ordering::Relaxed);
    }
    app.find_dialog = None;
    app.find_search_rx = None;
    app.focus = Focus::Panel;
}

/// One row in the grouped find results list: either a folder (header) or a file.
#[derive(Debug, Clone)]
pub enum FindDisplayRow {
    Folder(PathBuf),
    File(FindResult),
}

/// Build display list grouped by parent directory: for each directory (in order of first occurrence),
/// one Folder row then one File row per match in that directory.
pub fn build_display_rows(results: &[FindResult]) -> Vec<FindDisplayRow> {
    let mut groups: Vec<(PathBuf, Vec<FindResult>)> = Vec::new();
    for r in results {
        let parent = r
            .path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(PathBuf::new);
        if groups.last().map(|(p, _)| p != &parent).unwrap_or(true) {
            groups.push((parent, Vec::new()));
        }
        if let Some((_, files)) = groups.last_mut() {
            files.push(r.clone());
        }
    }
    let mut rows = Vec::new();
    for (dir, files) in groups {
        rows.push(FindDisplayRow::Folder(dir));
        for r in files {
            rows.push(FindDisplayRow::File(r));
        }
    }
    rows
}

/// Shell-style glob match: * = any sequence, ? = one character. Empty pattern matches all.
pub fn glob_match(pattern: &str, name: &str, case_sensitive: bool) -> bool {
    let (p, n) = if case_sensitive {
        (pattern.as_bytes().to_vec(), name.as_bytes().to_vec())
    } else {
        (
            pattern.to_lowercase().into_bytes(),
            name.to_lowercase().into_bytes(),
        )
    };
    let (p, n) = (p.as_slice(), n.as_slice());
    fn match_at(p: &[u8], n: &[u8]) -> bool {
        let mut pi = 0;
        let mut ni = 0;
        while pi < p.len() {
            match p[pi] {
                b'*' => {
                    pi += 1;
                    if pi == p.len() {
                        return true;
                    }
                    while ni <= n.len() {
                        if match_at(&p[pi..], &n[ni..]) {
                            return true;
                        }
                        ni += 1;
                    }
                    return false;
                }
                b'?' => {
                    if ni < n.len() {
                        pi += 1;
                        ni += 1;
                    } else {
                        return false;
                    }
                }
                c => {
                    if ni < n.len() && n[ni] == c {
                        pi += 1;
                        ni += 1;
                    } else {
                        return false;
                    }
                }
            }
        }
        ni == n.len()
    }
    if p.is_empty() {
        return true;
    }
    match_at(p, n)
}

/// Run find in a background thread; send matches and Done on tx.
/// Uses walkdir for robust traversal (handles special dir names like "!!!").
/// Takes Arc<str> so the caller can Arc::clone (cheap) instead of cloning the string.
/// If cancel.load(Ordering::Relaxed) becomes true, stops and sends Done.
fn run_search(
    start_dir: Arc<str>,
    file_pattern: Arc<str>,
    content_pattern: Arc<str>,
    recursive: bool,
    file_case_sens: bool,
    content_case_sens: bool,
    skip_hidden: bool,
    cancel: Arc<AtomicBool>,
    tx: mpsc::Sender<FindMessage>,
) {
    thread::spawn(move || {
        let start_dir = start_dir.trim();
        let file_pattern = file_pattern.trim();
        let content_pattern = content_pattern.trim();
        let start = PathBuf::from(start_dir);
        if !start.is_dir() {
            let _ = tx.send(FindMessage::Done);
            return;
        }
        let content_empty = content_pattern.is_empty();
        let mut current_dir_sent: Option<PathBuf> = None;

        let walker = WalkDir::new(&start)
            .min_depth(1)
            .max_depth(if recursive { usize::MAX } else { 1 })
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| {
                let name = e.file_name().to_string_lossy();
                if skip_hidden && name.starts_with('.') {
                    return false;
                }
                true
            })
            .filter_map(|e| e.ok());

        for entry in walker {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            let path = entry.path().to_path_buf();
            if let Some(parent) = path.parent() {
                if current_dir_sent.as_ref().map(PathBuf::as_path) != Some(parent) {
                    current_dir_sent = Some(parent.to_path_buf());
                    let _ = tx.send(FindMessage::CurrentDir(parent.display().to_string()));
                }
            }
            if !entry.file_type().is_file() {
                continue;
            }
            let name_cow = entry.file_name().to_string_lossy();
            let name = name_cow.as_ref();
            if !glob_match(file_pattern, name, file_case_sens) {
                continue;
            }
            if content_empty {
                let _ = tx.send(FindMessage::Match(path, None));
            } else if let Ok(file) = fs::File::open(&path) {
                let needle = if content_case_sens {
                    content_pattern.to_string()
                } else {
                    content_pattern.to_lowercase()
                };
                let reader = BufReader::new(file);
                for (line_no, line) in reader.lines().enumerate() {
                    if cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    if let Ok(ref ln) = line {
                        let hay = if content_case_sens {
                            ln.clone()
                        } else {
                            ln.to_lowercase()
                        };
                        if hay.contains(&needle) {
                            let _ = tx.send(FindMessage::Match(
                                path.clone(),
                                Some((line_no + 1) as u64),
                            ));
                            break;
                        }
                    }
                }
            }
        }
        let _ = tx.send(FindMessage::Done);
    });
}

/// Start the find search in a background thread; app.find_dialog must be Some.
pub fn start_search(app: &mut AppState) {
    let d = match app.find_dialog.as_mut() {
        Some(d) => d,
        None => return,
    };
    d.phase = FindDialogPhase::Searching;
    d.results.clear();
    d.status_message = "Searching...".to_string();
    d.search_current_dir.clear();
    d.selected_index = 0;
    d.scroll_offset = 0;

    // Move strings into Arc<str> (no clone); pass Arc::clone to thread (cheap). Restore from stored Arc when Done.
    let start_dir = Arc::from(std::mem::take(&mut d.start_dir_input.text));
    let file_pattern = Arc::from(std::mem::take(&mut d.file_pattern_input.text));
    let content_pattern = Arc::from(std::mem::take(&mut d.content_pattern_input.text));
    d.search_start_dir = Some(Arc::clone(&start_dir));
    d.search_file_pattern = Some(Arc::clone(&file_pattern));
    d.search_content_pattern = Some(Arc::clone(&content_pattern));

    let recursive = d.recursive;
    let file_case_sens = d.file_case_sens;
    let content_case_sens = d.content_case_sens;
    let skip_hidden = d.skip_hidden;

    let cancel = Arc::new(AtomicBool::new(false));
    app.find_search_cancel = Some(Arc::clone(&cancel));

    let (tx, rx) = mpsc::channel();
    run_search(
        start_dir,
        file_pattern,
        content_pattern,
        recursive,
        file_case_sens,
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
                if let Some(ref mut d) = app.find_dialog {
                    d.results.push(FindResult { path, line });
                }
            }
            FindMessage::CurrentDir(s) => {
                if let Some(ref mut d) = app.find_dialog {
                    d.search_current_dir = s;
                }
            }
            FindMessage::Done => {
                if let Some(ref mut d) = app.find_dialog {
                    d.phase = FindDialogPhase::Results;
                    let n = d.results.len();
                    d.status_message = format!("Search complete. {} match(es).", n);
                    d.search_current_dir.clear();
                    // Restore search params from Arc into input fields (one copy per field when done).
                    if let Some(arc) = d.search_start_dir.take() {
                        d.start_dir_input.text = arc.to_string();
                        d.start_dir_input.cursor = d.start_dir_input.text.chars().count();
                    }
                    if let Some(arc) = d.search_file_pattern.take() {
                        d.file_pattern_input.text = arc.to_string();
                        d.file_pattern_input.cursor = d.file_pattern_input.text.chars().count();
                    }
                    if let Some(arc) = d.search_content_pattern.take() {
                        d.content_pattern_input.text = arc.to_string();
                        d.content_pattern_input.cursor = d.content_pattern_input.text.chars().count();
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
) -> Option<crate::events::AppAction> {
    use crate::events::AppAction;
    let phase = app.find_dialog.as_ref().map(|d| d.phase)?;
    match phase {
        FindDialogPhase::Parameter => handle_key_parameter(app, code, modifiers),
        FindDialogPhase::Searching | FindDialogPhase::Results => {
            // ESC while searching: stop the search but keep the dialog open with results so far.
            if phase == FindDialogPhase::Searching && code == KeyCode::Esc {
                if let Some(ref mut d) = app.find_dialog {
                    d.phase = FindDialogPhase::Results;
                    let n = d.results.len();
                    d.status_message = format!("Stopped. {} match(es).", n);
                    d.search_current_dir.clear();
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
) -> Option<crate::events::AppAction> {
    use crate::events::AppAction;
    let d = app.find_dialog.as_mut()?;
    if modifiers.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('c') {
        close(app);
        return Some(AppAction::FindClose);
    }
    match code {
        KeyCode::Esc => return Some(AppAction::FindClose),
        KeyCode::Tab | KeyCode::Down => {
            d.focus = (d.focus + 1) % 9;
            return Some(AppAction::Continue);
        }
        KeyCode::BackTab | KeyCode::Up => {
            d.focus = (d.focus + 8) % 9;
            return Some(AppAction::Continue);
        }
        KeyCode::Enter => {
            if d.focus == 8 {
                return Some(AppAction::FindClose);
            }
            // Enter from any other widget (inputs 0–2, options 3–6, Find 7) starts the search
            return Some(AppAction::FindStartSearch);
        }
        KeyCode::Char(' ') => {
            if d.focus >= 3 && d.focus <= 6 {
                match d.focus {
                    3 => d.recursive = !d.recursive,
                    4 => d.file_case_sens = !d.file_case_sens,
                    5 => d.content_case_sens = !d.content_case_sens,
                    6 => d.skip_hidden = !d.skip_hidden,
                    _ => {}
                }
                return Some(AppAction::Continue);
            }
        }
        KeyCode::Char(c) => {
            if c.is_ascii() && !c.is_control() && d.focus <= 2 {
                let input = match d.focus {
                    0 => &mut d.start_dir_input,
                    1 => &mut d.file_pattern_input,
                    2 => &mut d.content_pattern_input,
                    _ => return Some(AppAction::Continue),
                };
                *input = std::mem::take(input).insert_char(c);
            }
        }
        KeyCode::Backspace => {
            if d.focus <= 2 {
                let input = match d.focus {
                    0 => &mut d.start_dir_input,
                    1 => &mut d.file_pattern_input,
                    2 => &mut d.content_pattern_input,
                    _ => return Some(AppAction::Continue),
                };
                *input = std::mem::take(input).backspace();
            }
        }
        KeyCode::Left => {
            if d.focus <= 2 {
                let input = match d.focus {
                    0 => &mut d.start_dir_input,
                    1 => &mut d.file_pattern_input,
                    2 => &mut d.content_pattern_input,
                    _ => return Some(AppAction::Continue),
                };
                *input = std::mem::take(input).move_left();
            }
        }
        KeyCode::Right => {
            if d.focus <= 2 {
                let input = match d.focus {
                    0 => &mut d.start_dir_input,
                    1 => &mut d.file_pattern_input,
                    2 => &mut d.content_pattern_input,
                    _ => return Some(AppAction::Continue),
                };
                *input = std::mem::take(input).move_right();
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
) -> Option<crate::events::AppAction> {
    use crate::events::AppAction;
    let d = app.find_dialog.as_mut()?;
    let display_rows = build_display_rows(&d.results);
    let len = display_rows.len();
    if len > 0 && d.selected_index >= len {
        d.selected_index = len - 1;
    }
    match code {
        KeyCode::Esc => return Some(AppAction::FindClose),
        KeyCode::Enter => return Some(AppAction::FindChdir),
        KeyCode::F(3) => {
            if len > 0 {
                if let Some(FindDisplayRow::File(_)) = display_rows.get(d.selected_index) {
                    return Some(AppAction::FindView);
                }
            }
        }
        KeyCode::F(4) => {
            if len > 0 {
                if let Some(FindDisplayRow::File(_)) = display_rows.get(d.selected_index) {
                    return Some(AppAction::FindEdit);
                }
            }
        }
        KeyCode::Up => {
            if len > 0 {
                d.selected_index = d.selected_index.saturating_sub(1);
                if d.selected_index < d.scroll_offset {
                    d.scroll_offset = d.selected_index;
                }
            }
        }
        KeyCode::Down => {
            if len > 0 {
                d.selected_index = (d.selected_index + 1).min(len - 1);
                let max_visible = d.visible_list_rows.max(1);
                if d.selected_index >= d.scroll_offset + max_visible {
                    d.scroll_offset = d.selected_index - max_visible + 1;
                }
            }
        }
        KeyCode::PageUp => {
            if len > 0 {
                let n = d.visible_list_rows.max(1);
                d.selected_index = d.selected_index.saturating_sub(n).max(0);
                if d.selected_index < d.scroll_offset {
                    d.scroll_offset = d.selected_index;
                }
            }
        }
        KeyCode::PageDown => {
            if len > 0 {
                let n = d.visible_list_rows.max(1);
                d.selected_index = (d.selected_index + n).min(len - 1);
                if d.selected_index >= d.scroll_offset + n {
                    d.scroll_offset = d.selected_index - n + 1;
                }
            }
        }
        KeyCode::Home => {
            d.selected_index = 0;
            d.scroll_offset = 0;
        }
        KeyCode::End => {
            if len > 0 {
                d.selected_index = len - 1;
                d.scroll_offset = len.saturating_sub(d.visible_list_rows.max(1));
            }
        }
        _ => {}
    }
    Some(AppAction::Continue)
}

const FIND_DIALOG_W: u16 = 110;
const FIND_DIALOG_H_PARAM: u16 = 18;
const FIND_DIALOG_H_RESULTS: u16 = 28;
const STATUS_ROWS: u16 = 2; // status line + gap
const HINT_ROWS: u16 = 1;

/// Draw the Find file dialog (parameter form or results list).
pub fn draw(f: &mut Frame, app: &mut AppState) {
    poll_search(app);
    let d = match app.find_dialog.as_mut() {
        Some(d) => d,
        None => return,
    };
    let area = f.area();
    let (w, h) = match d.phase {
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
    let title = match d.phase {
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

    match d.phase {
        FindDialogPhase::Parameter => {
            draw_parameter_form(f, d, inner, content_w, fill_style);
        }
        FindDialogPhase::Searching => {
            draw_status_and_list(f, d, inner, content_w, fill_style, true);
        }
        FindDialogPhase::Results => {
            draw_status_and_list(f, d, inner, content_w, fill_style, false);
        }
    }
}

fn draw_parameter_form(
    f: &mut Frame,
    d: &FindDialogState,
    inner: Rect,
    content_w: u16,
    fill_style: Style,
) {
    let cx = inner.x + 1;
    let mut row = inner.y;

    const LABEL_W: u16 = 18;

    for (focus_idx, label, input) in [
        (0, "Start directory:", &d.start_dir_input),
        (1, "File pattern:", &d.file_pattern_input),
        (2, "Content pattern:", &d.content_pattern_input),
    ] {
        let value = input.text.as_str();
        let cursor_char = input.cursor_column();
        let focused = d.focus == focus_idx;
        let bg = if focused {
            DIALOG_INPUT_BG_FOCUSED
        } else {
            DIALOG_INPUT_BG_UNFOCUSED
        };
        let value_w = content_w.saturating_sub(LABEL_W);
        let value_w_usize = value_w as usize;
        let display_offset = if value_w_usize == 0 {
            0
        } else if cursor_char + 1 <= value_w_usize {
            0
        } else {
            cursor_char + 1 - value_w_usize
        };
        let displayed: String = value.chars().skip(display_offset).take(value_w_usize).collect();
        let cursor_screen = cursor_char.saturating_sub(display_offset);
        // Label and input on one line
        f.render_widget(
            Paragraph::new(label).style(fill_style),
            Rect { x: cx, y: row, width: LABEL_W, height: 1 },
        );
        f.render_widget(
            Paragraph::new(displayed.as_str()).style(Style::default().bg(bg).fg(Color::White)),
            Rect { x: cx + LABEL_W, y: row, width: value_w, height: 1 },
        );
        if focused {
            let cursor_x = cx + LABEL_W + (cursor_screen as u16).min(value_w.saturating_sub(1));
            f.set_cursor_position((cursor_x, row));
        }
        row += 1;
    }
    row += 1;

    let opts = [
        (3, "Recursive", d.recursive),
        (4, "File name case sensitive", d.file_case_sens),
        (5, "Content case sensitive", d.content_case_sens),
        (6, "Skip hidden files", d.skip_hidden),
    ];
    for (idx, label, on) in opts {
        let mark = if on { "[x]" } else { "[ ]" };
        let style = if d.focus == idx {
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
        if d.focus == idx {
            Style::default().bg(Color::Cyan).fg(Color::Black)
        } else {
            fill_style
        }
    };
    f.render_widget(
        Paragraph::new("  Find  ").style(btn_style(7)),
        Rect {
            x: cx,
            y: row,
            width: 8,
            height: 1,
        },
    );
    f.render_widget(
        Paragraph::new("  Cancel  ").style(btn_style(8)),
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
    d: &mut FindDialogState,
    inner: Rect,
    content_w: u16,
    fill_style: Style,
    searching: bool,
) {
    let cx = inner.x + 1;
    let mut row = inner.y;
    let show_hint = !searching && !d.results.is_empty();
    let list_height = inner
        .height
        .saturating_sub(STATUS_ROWS)
        .saturating_sub(if show_hint { HINT_ROWS } else { 0 })
        .max(1);
    d.visible_list_rows = list_height as usize;
    let v = d.visible_list_rows.max(1);
    let display_rows = build_display_rows(&d.results);
    let len = display_rows.len();
    if len > 0 {
        if d.selected_index >= len {
            d.selected_index = len - 1;
        }
        if d.scroll_offset + v > len {
            d.scroll_offset = len.saturating_sub(v);
        }
        if d.selected_index < d.scroll_offset {
            d.scroll_offset = d.selected_index;
        }
        if d.selected_index >= d.scroll_offset + v {
            d.scroll_offset = d.selected_index - v + 1;
        }
    }

    if searching {
        // One line: current directory (or "Scanning...") and number of found items; suffix fixed so it doesn't jump.
        let n = d.results.len();
        let suffix = format!("  {} found", n);
        let suffix_len = suffix.chars().count();
        let path_w = (content_w as usize).saturating_sub(suffix_len).max(0);
        let path_display = if d.search_current_dir.is_empty() {
            "Scanning...".to_string()
        } else {
            truncate_path(&d.search_current_dir, path_w)
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
            Paragraph::new(d.status_message.as_str()).style(fill_style),
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
        .skip(d.scroll_offset)
        .take(d.visible_list_rows)
        .enumerate()
        .map(|(i, row)| {
            let idx = d.scroll_offset + i;
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
            let style = if idx == d.selected_index {
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

fn truncate_path(s: &str, max: usize) -> String {
    crate::util::truncate_str(s, max, crate::util::TruncateMode::SuffixEllipsis)
}

#[cfg(test)]
mod tests {
    use super::glob_match;

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
