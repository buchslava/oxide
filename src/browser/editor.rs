//! Embedded code editor (F4): open file, edit, F2 save, F8 delete current line, ESC exit,
//! Page Up/Down, Home/End, Ctrl+X then F search (in file), unsaved-changes dialog.
//! Selection (MC-style): F3 starts or stops selection; then ←→↑↓ extend. Ctrl+C copies then clears.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent};
use ratatui::{
    layout::{Alignment, Margin, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};
use ratatui_code_editor::actions::DeleteLine;
use ratatui_code_editor::editor::Editor;
use ratatui_code_editor::selection::Selection;
use ratatui_code_editor::theme::vesper;

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

use crate::app::ctrl_x_chord;
use crate::app::events::AppAction;
use crate::app::state::{AppState, Focus};
use crate::browser::panel::PanelOperations;
use crate::core::file_ops::FileInfo;
use crate::core::file_ops::FileOperations;
use crate::core::location::PanelLocation;
use crate::core::panel_backend::{join_path_display, read_file, write_file};
use crate::core::text_format::format_byte_size;
use crate::ui::text_input;
use crate::ui::theme::{DialogPalette, ViewerPalette};
use crate::ui::toast::{self, TimedToast};
use crate::util;
use crate::util::compute_panel_height;

/// Match [`ratatui_code_editor`] render: `digits.max(5) + 2` columns for the line-number gutter.
fn editor_gutter_width(
    editor: &Editor,
    area_width: u16,
) -> u16 {
    let total_lines = editor.code_ref().len_lines().max(1);
    let digits = total_lines.to_string().len().max(5);
    let w = (digits + 2) as u16;
    w.min(area_width)
}

/// Slightly lighter than the main editor canvas so the gutter reads as a separate strip.
fn editor_gutter_background(main_bg: Color) -> Color {
    match main_bg {
        Color::Rgb(r, g, b) => Color::Rgb(
            r.saturating_add(10),
            g.saturating_add(10),
            b.saturating_add(12),
        ),
        _ => Color::Rgb(42, 42, 48),
    }
}

/// State when the embedded code editor is open (F4).
pub struct EditorScreenState {
    /// Display path (for title/lang). When editing inside Zip, this is the virtual path.
    pub file_path: String,
    /// Content when file was opened; used to detect unsaved changes.
    pub initial_content: String,
    pub editor: Editor,
    /// Last draw area for the editor (used for input/mouse). When search is open, height is reduced by 1.
    pub area: Rect,
    /// When Some, search bar is open and the string is the current query (Ctrl+X then F).
    pub search_query: Option<String>,
    /// Cursor position in the search query (0..=len). Only used when search_query is Some.
    pub search_query_cursor: usize,
    /// F3 selection mode (MC-style): when true, arrows extend selection.
    pub selection_extend_mode: bool,
    /// When editing a file inside a Zip, these are set; otherwise None (save uses file_path to fs).
    pub edit_location: Option<PanelLocation>,
    pub edit_name: Option<String>,
    /// True when the file bytes were not valid UTF-8 at open; F2 saves as `name.text` to avoid overwriting binary.
    pub opened_with_invalid_utf8: bool,
}

/// Files above this size show a warning before load; loading runs in a background thread (Esc cancels).
pub const EDITOR_LARGE_FILE_WARN_BYTES: u64 = 32 * 1024 * 1024;

/// Hard cap for the embedded editor: [`Editor::new`] runs on the UI thread and the type is not `Send`,
/// so larger files would freeze the app (Esc cannot run until decoding finishes). Refuse before/after read.
pub const EDITOR_MAX_EMBEDDED_BYTES: u64 = 128 * 1024 * 1024;

/// How to read the file when finishing a background load (same as open path).
#[derive(Debug, Clone)]
pub enum EditorOpenSpec {
    Fs { path: PathBuf },
    Archive { loc: PanelLocation, name: String },
}

/// F4 UI: warning for huge files, loading with cancel, or the actual editor.
pub enum EditorViewState {
    WarnLargeFile {
        file_path: String,
        size_bytes: u64,
        spec: EditorOpenSpec,
    },
    Loading {
        file_path: String,
        rx: mpsc::Receiver<io::Result<Vec<u8>>>,
        cancel: Arc<AtomicBool>,
        spec: EditorOpenSpec,
    },
    /// Raw bytes received from worker; [`Editor::new`] not run yet so Esc can cancel before that freeze.
    BytesLoaded {
        file_path: String,
        bytes: Vec<u8>,
        spec: EditorOpenSpec,
    },
    Ready(EditorScreenState),
}

/// User choice in the "Save changes?" dialog when exiting editor with unsaved changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorConfirmChoice {
    Save,
    Discard,
    Cancel,
}

/// Language identifier for ratatui-code-editor from file path (by extension).
fn get_lang_from_path(path: &str) -> &'static str {
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    let ext_lower: String = ext.chars().flat_map(|c| c.to_lowercase()).collect();
    match ext_lower.as_str() {
        "rs" => "rust",
        "py" => "python",
        "js" | "mjs" | "cjs" => "javascript",
        "ts" | "mts" | "cts" => "typescript",
        "go" => "go",
        "c" | "h" => "c",
        "cpp" | "cc" | "cxx" | "hpp" => "cpp",
        "html" | "htm" => "html",
        "css" => "css",
        "json" => "json",
        "toml" => "toml",
        "yaml" | "yml" => "yaml",
        "md" => "markdown",
        "sh" | "bash" => "bash",
        _ => "plain",
    }
}

fn editor_too_large_toast(app: &mut AppState) {
    app.set_timed_toast_alert(
        Duration::from_secs(8),
        format!(
            "Embedded editor limit is {} (buffer built on UI thread). Use an external editor for larger files.",
            format_byte_size(EDITOR_MAX_EMBEDDED_BYTES),
        ),
    );
}

fn editor_source_size(
    loc: &PanelLocation,
    file: &FileInfo,
) -> io::Result<u64> {
    match loc {
        PanelLocation::Fs(p) => {
            let path = FileOperations::join_path(p, &file.name);
            Ok(std::fs::metadata(&path)?.len())
        }
        PanelLocation::Archive { .. } => Ok(file.size),
    }
}

fn editor_ready_from_bytes(
    file_path: String,
    bytes: Vec<u8>,
    spec: EditorOpenSpec,
) -> EditorScreenState {
    let opened_with_invalid_utf8 = std::str::from_utf8(&bytes).is_err();
    let content = String::from_utf8_lossy(&bytes).into_owned();
    let lang = get_lang_from_path(&file_path);
    let theme = vesper();
    let editor = Editor::new(lang, &content, theme);
    let (edit_location, edit_name) = match spec {
        EditorOpenSpec::Fs { .. } => (None, None),
        EditorOpenSpec::Archive { loc, name } => (Some(loc), Some(name)),
    };
    EditorScreenState {
        file_path,
        initial_content: content,
        editor,
        area: Rect::default(),
        search_query: None,
        search_query_cursor: 0,
        selection_extend_mode: false,
        edit_location,
        edit_name,
        opened_with_invalid_utf8,
    }
}

fn start_editor_loading(
    app: &mut AppState,
    file_path: String,
    spec: EditorOpenSpec,
) {
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_t = Arc::clone(&cancel);
    let (tx, rx) = mpsc::channel();
    let spec_thread = spec.clone();
    std::thread::spawn(move || {
        let result = match spec_thread {
            EditorOpenSpec::Fs { path } => util::read_path_chunked(&path, &cancel_t),
            EditorOpenSpec::Archive { loc, name } => {
                if cancel_t.load(Ordering::Relaxed) {
                    Err(io::Error::new(
                        io::ErrorKind::Interrupted,
                        "editor load cancelled",
                    ))
                } else {
                    read_file(&loc, &name)
                }
            }
        };
        let _ = tx.send(result);
    });
    app.editor_screen = Some(EditorViewState::Loading {
        file_path,
        rx,
        cancel,
        spec,
    });
}

fn cancel_editor_background(app: &mut AppState) {
    if let Some(EditorViewState::Loading { cancel, .. }) = app.editor_screen.as_ref() {
        cancel.store(true, Ordering::Relaxed);
    }
}

/// Poll background editor file read (bytes only — does **not** call [`Editor::new`], so the UI stays responsive).
/// Call **before** [`EventHandler::handle_events`] so Esc can clear [`EditorViewState::BytesLoaded`] before
/// [`finish_editor_pending_decode`] runs. See [`finish_editor_pending_decode`].
pub fn poll_editor_loading(app: &mut AppState) -> bool {
    let rx = match &mut app.editor_screen {
        Some(EditorViewState::Loading { rx, .. }) => rx,
        _ => return false,
    };
    match rx.try_recv() {
        Ok(Ok(bytes)) => {
            let (file_path, spec) = match std::mem::take(&mut app.editor_screen) {
                Some(EditorViewState::Loading {
                    file_path, spec, ..
                }) => (file_path, spec),
                _ => return false,
            };
            app.editor_screen = Some(EditorViewState::BytesLoaded {
                file_path,
                bytes,
                spec,
            });
            app.editor_confirm_pending = false;
            app.clear_timed_toast();
            true
        }
        Ok(Err(_)) => {
            app.editor_screen = None;
            if app.find_dialog.is_some() {
                app.focus = Focus::FindDialog;
            }
            true
        }
        Err(mpsc::TryRecvError::Empty) => false,
        Err(mpsc::TryRecvError::Disconnected) => {
            app.editor_screen = None;
            if app.find_dialog.is_some() {
                app.focus = Focus::FindDialog;
            }
            true
        }
    }
}

/// After input is processed, build [`Editor`] from pending bytes. Can block for large (but capped) files; must run **after** Esc handling.
pub fn finish_editor_pending_decode(app: &mut AppState) -> bool {
    let (file_path, bytes, spec) = match app.editor_screen.take() {
        Some(EditorViewState::BytesLoaded {
            file_path,
            bytes,
            spec,
        }) => (file_path, bytes, spec),
        other => {
            // Must restore: `take()` already ran; dropping Ready/Warn/Loading would clear the editor.
            app.editor_screen = other;
            return false;
        }
    };
    if bytes.len() as u64 > EDITOR_MAX_EMBEDDED_BYTES {
        drop(bytes);
        editor_too_large_toast(app);
        if app.find_dialog.is_some() {
            app.focus = Focus::FindDialog;
        }
        return true;
    }
    let ready = editor_ready_from_bytes(file_path, bytes, spec);
    app.editor_screen = Some(EditorViewState::Ready(ready));
    app.editor_confirm_pending = false;
    app.clear_timed_toast();
    true
}

/// Paste the given text as-is at the editor cursor (same as Ctrl+V). Replaces selection if any.
/// Returns Some(Continue) when the editor is open and paste was applied (or text was empty); None when not in editor.
pub fn paste_text_as_is(
    app: &mut AppState,
    text: &str,
) -> Option<AppAction> {
    let ed = match app.editor_screen.as_mut()? {
        EditorViewState::Ready(ed) => ed,
        _ => return None,
    };
    if text.is_empty() {
        return Some(AppAction::Continue);
    }
    let mut cursor = ed.editor.get_cursor();
    let mut selection = ed.editor.get_selection();
    let code = ed.editor.code_mut();
    code.tx();
    code.set_state_before(cursor, selection);
    if let Some(sel) = &selection {
        if !sel.is_empty() {
            let (start, end) = sel.sorted();
            code.remove(start, end);
            cursor = start;
            selection = None;
        }
    }
    code.insert(cursor, &text);
    cursor += text.chars().count();
    code.set_state_after(cursor, selection);
    code.commit();
    ed.editor.set_cursor(cursor);
    ed.editor.set_selection(selection);
    ed.editor.reset_highlight_cache();
    ed.selection_extend_mode = false;
    ed.editor.focus(&ed.area);
    Some(AppAction::Continue)
}

/// Open the currently selected file in the embedded editor. Returns true if opened.
/// Supports both filesystem and files inside ZIP archives.
pub fn open_editor(app: &mut AppState) -> bool {
    let loc = app.get_current_location();
    if let Some(file) = app.active_panel_mut().get_selected_file() {
        if !file.is_dir && !file.is_parent_dir() {
            let file_path_str = join_path_display(&loc, &file.name);
            let spec = match &loc {
                PanelLocation::Fs(p) => EditorOpenSpec::Fs {
                    path: FileOperations::join_path(p, &file.name),
                },
                PanelLocation::Archive { .. } => EditorOpenSpec::Archive {
                    loc: loc.clone(),
                    name: file.name.clone(),
                },
            };
            let size = match editor_source_size(&loc, file) {
                Ok(s) => s,
                Err(_) => return false,
            };
            if size > EDITOR_MAX_EMBEDDED_BYTES {
                editor_too_large_toast(app);
                return false;
            }
            if size > EDITOR_LARGE_FILE_WARN_BYTES {
                app.editor_screen = Some(EditorViewState::WarnLargeFile {
                    file_path: file_path_str,
                    size_bytes: size,
                    spec,
                });
                app.editor_confirm_pending = false;
                app.clear_timed_toast();
                return true;
            }
            let (content, opened_with_invalid_utf8) = match read_file(&loc, &file.name) {
                Ok(bytes) => (
                    String::from_utf8_lossy(&bytes).into_owned(),
                    std::str::from_utf8(&bytes).is_err(),
                ),
                Err(_) => return false,
            };
            let lang = get_lang_from_path(&file_path_str);
            let theme = vesper();
            let editor = Editor::new(lang, &content, theme);
            let (edit_location, edit_name) = match &loc {
                PanelLocation::Archive { .. } => (Some(loc.clone()), Some(file.name.clone())),
                PanelLocation::Fs(_) => (None, None),
            };
            app.editor_screen = Some(EditorViewState::Ready(
                EditorScreenState {
                    file_path: file_path_str,
                    initial_content: content,
                    editor,
                    area: Rect::default(),
                    search_query: None,
                    search_query_cursor: 0,
                    selection_extend_mode: false,
                    edit_location,
                    edit_name,
                    opened_with_invalid_utf8,
                },
            ));
            app.editor_confirm_pending = false;
            app.clear_timed_toast();
            return true;
        }
    }
    false
}

/// Open a file by path in the editor (e.g. from Find file results). Returns true if opened.
pub fn open_editor_path(
    app: &mut AppState,
    path: std::path::PathBuf,
) -> bool {
    if !path.is_file() {
        return false;
    }
    let file_path_str = path.display().to_string();
    let size = match std::fs::metadata(&path) {
        Ok(m) => m.len(),
        Err(_) => return false,
    };
    if size > EDITOR_MAX_EMBEDDED_BYTES {
        editor_too_large_toast(app);
        return false;
    }
    if size > EDITOR_LARGE_FILE_WARN_BYTES {
        app.editor_screen = Some(EditorViewState::WarnLargeFile {
            file_path: file_path_str,
            size_bytes: size,
            spec: EditorOpenSpec::Fs { path },
        });
        app.editor_confirm_pending = false;
        app.clear_timed_toast();
        return true;
    }
    let (content, opened_with_invalid_utf8) = match std::fs::read(&path) {
        Ok(bytes) => (
            String::from_utf8_lossy(&bytes).into_owned(),
            std::str::from_utf8(&bytes).is_err(),
        ),
        Err(_) => return false,
    };
    let lang = get_lang_from_path(&file_path_str);
    let theme = vesper();
    let editor = Editor::new(lang, &content, theme);
    app.editor_screen = Some(EditorViewState::Ready(
        EditorScreenState {
            file_path: file_path_str,
            initial_content: content,
            editor,
            area: Rect::default(),
            search_query: None,
            search_query_cursor: 0,
            selection_extend_mode: false,
            edit_location: None,
            edit_name: None,
            opened_with_invalid_utf8,
        },
    ));
    app.editor_confirm_pending = false;
    app.clear_timed_toast();
    true
}

/// Find next occurrence of query in the **opened file only** (editor buffer).
/// Search from current cursor, wrap from start if not found.
fn find_next(
    ed: &mut EditorScreenState,
    query: &str,
) {
    if query.is_empty() {
        return;
    }
    let content = ed.editor.get_content();
    let content_chars: Vec<char> = content.chars().collect();
    let query_chars: Vec<char> = query.chars().collect();
    let len = content_chars.len();
    let qlen = query_chars.len();
    if qlen > len {
        return;
    }
    let cursor = ed.editor.get_cursor();
    // Search from cursor + 1
    let mut pos = (cursor + 1).min(len);
    while pos + qlen <= len {
        if content_chars[pos..pos + qlen] == query_chars[..] {
            ed.editor.set_cursor(pos);
            ed.editor
                .set_selection(Some(Selection::new(pos, pos + qlen)));
            ed.editor.reset_highlight_cache();
            ed.editor.focus(&ed.area);
            return;
        }
        pos += 1;
    }
    // Wrap: search from start up to cursor
    pos = 0;
    while pos + qlen <= len && pos <= cursor {
        if content_chars[pos..pos + qlen] == query_chars[..] {
            ed.editor.set_cursor(pos);
            ed.editor
                .set_selection(Some(Selection::new(pos, pos + qlen)));
            ed.editor.reset_highlight_cache();
            ed.editor.focus(&ed.area);
            return;
        }
        pos += 1;
    }
}

/// Handle key when embedded editor is open. Returns Some(action) when handled, None if not in editor.
pub fn handle_editor_key(
    app: &mut AppState,
    key: KeyEvent,
) -> Option<AppAction> {
    if matches!(
        app.editor_screen,
        Some(EditorViewState::WarnLargeFile { .. })
    ) {
        match key.code {
            KeyCode::Esc | KeyCode::Char('\x1b') => {
                app.editor_screen = None;
                if app.find_dialog.is_some() {
                    app.focus = Focus::FindDialog;
                }
                return Some(AppAction::Continue);
            }
            KeyCode::Enter => {
                let (file_path, spec, size_bytes) = match app.editor_screen.take() {
                    Some(EditorViewState::WarnLargeFile {
                        file_path,
                        spec,
                        size_bytes,
                    }) => (file_path, spec, size_bytes),
                    _ => return Some(AppAction::Continue),
                };
                if size_bytes > EDITOR_MAX_EMBEDDED_BYTES {
                    editor_too_large_toast(app);
                    if app.find_dialog.is_some() {
                        app.focus = Focus::FindDialog;
                    }
                    return Some(AppAction::Continue);
                }
                start_editor_loading(app, file_path, spec);
                return Some(AppAction::Continue);
            }
            _ => return Some(AppAction::Continue),
        }
    }
    if matches!(
        app.editor_screen,
        Some(EditorViewState::Loading { .. })
    ) {
        if key.code == KeyCode::Esc || key.code == KeyCode::Char('\x1b') {
            cancel_editor_background(app);
            app.editor_screen = None;
            if app.find_dialog.is_some() {
                app.focus = Focus::FindDialog;
            }
            return Some(AppAction::Continue);
        }
        return Some(AppAction::Continue);
    }
    if matches!(
        app.editor_screen,
        Some(EditorViewState::BytesLoaded { .. })
    ) {
        if key.code == KeyCode::Esc || key.code == KeyCode::Char('\x1b') {
            app.editor_screen = None;
            if app.find_dialog.is_some() {
                app.focus = Focus::FindDialog;
            }
            return Some(AppAction::Continue);
        }
        return Some(AppAction::Continue);
    }
    let ed = match app.editor_screen.as_mut()? {
        EditorViewState::Ready(ed) => ed,
        _ => return None,
    };
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    #[cfg(target_os = "macos")]
    let cmd_like =
        key.modifiers.contains(KeyModifiers::SUPER) || key.modifiers.contains(KeyModifiers::META);
    #[cfg(not(target_os = "macos"))]
    let cmd_like = false;
    // On macOS, Cmd+V/Cmd+C are the standard shortcuts; also accept Ctrl+V/Ctrl+C. Use same code path for both.
    let paste_mod = ctrl || cmd_like;
    let copy_mod = ctrl || cmd_like;

    if app.ctrl_x_chord_pending {
        app.ctrl_x_chord_pending = false;
        if key.code == KeyCode::Esc
            || ctrl_x_chord::is_ctrl_x_prefix(key.code, key.modifiers)
        {
            return Some(AppAction::Continue);
        }
        if matches!(key.code, KeyCode::Char(c) if c.eq_ignore_ascii_case(&'f')) {
            if ed.search_query.is_some() {
                ed.editor.clear_selection();
                ed.search_query = None;
            } else {
                ed.search_query = Some(String::new());
                ed.search_query_cursor = 0;
            }
            return Some(AppAction::Continue);
        }
    } else if ctrl_x_chord::is_ctrl_x_prefix(key.code, key.modifiers) {
        app.ctrl_x_chord_pending = true;
        return Some(AppAction::Continue);
    }

    // When search bar is open, handle search-specific keys first
    if let Some(ref mut query) = ed.search_query {
        let cursor = &mut ed.search_query_cursor;
        let len_chars = query.chars().count();
        if *cursor > len_chars {
            *cursor = len_chars;
        }
        match key.code {
            KeyCode::Esc => {
                ed.editor.clear_selection();
                ed.search_query = None;
                return Some(AppAction::Continue);
            }
            KeyCode::Enter => {
                let query_snapshot = query.clone();
                find_next(ed, &query_snapshot);
                return Some(AppAction::Continue);
            }
            KeyCode::Left => {
                *cursor = cursor.saturating_sub(1);
                return Some(AppAction::Continue);
            }
            KeyCode::Right => {
                *cursor = (*cursor + 1).min(len_chars);
                return Some(AppAction::Continue);
            }
            KeyCode::Backspace => {
                if *cursor > 0 {
                    let mut chars: Vec<char> = query.chars().collect();
                    chars.remove(*cursor - 1);
                    *query = chars.into_iter().collect();
                    *cursor -= 1;
                }
                return Some(AppAction::Continue);
            }
            KeyCode::Delete => {
                let mut chars: Vec<char> = query.chars().collect();
                if *cursor < chars.len() {
                    chars.remove(*cursor);
                    *query = chars.into_iter().collect();
                }
                return Some(AppAction::Continue);
            }
            KeyCode::Char(c) if !ctrl => {
                let mut chars: Vec<char> = query.chars().collect();
                let pos = (*cursor).min(chars.len());
                chars.insert(pos, c);
                *query = chars.into_iter().collect();
                *cursor = pos + 1;
                return Some(AppAction::Continue);
            }
            KeyCode::F(3) => {
                ed.editor.clear_selection();
                ed.search_query = None;
                return Some(AppAction::Continue);
            }
            KeyCode::F(8) => {
                ed.editor.apply(DeleteLine);
                ed.selection_extend_mode = false;
                ed.editor.focus(&ed.area);
                return Some(AppAction::Continue);
            }
            _ => return Some(AppAction::Continue),
        }
    }

    if key.code == KeyCode::F(10) {
        return Some(AppAction::Quit);
    }
    if key.code == KeyCode::F(2) {
        return Some(AppAction::EditorSave);
    }
    // F3: 1st = start selection (extend mode on). 2nd = stop extending, keep selection. 3rd = clear & start new.
    if key.code == KeyCode::F(3) {
        if ed.selection_extend_mode {
            ed.selection_extend_mode = false; // stop extending, selection stays
        } else {
            ed.editor.clear_selection();
            ed.selection_extend_mode = true; // clear any selection, start new from cursor
        }
        return Some(AppAction::Continue);
    }
    // F8: delete the entire line under the cursor (same as Ctrl+K in the editor widget).
    if key.code == KeyCode::F(8) {
        ed.editor.apply(DeleteLine);
        ed.selection_extend_mode = false;
        ed.editor.focus(&ed.area);
        return Some(AppAction::Continue);
    }
    if key.code == KeyCode::Esc {
        let content = ed.editor.get_content();
        if content != ed.initial_content {
            app.editor_confirm_pending = true;
            app.editor_confirm_focus = 0;
        } else {
            return Some(AppAction::EditorClose);
        }
        return Some(AppAction::Continue);
    }
    // Ctrl+C / Cmd+C (macOS): copy to clipboard, then clear selection
    if key.code == KeyCode::Char('c') && copy_mod {
        if let Some(text) = ed.editor.get_selection_text() {
            let _ = ed.editor.set_clipboard(&text);
        }
        ed.editor.clear_selection();
        return Some(AppAction::Continue);
    }
    // Ctrl+V / Cmd+V (macOS): always use paste-as-is (same logic as Event::Paste; never use crate's smart_paste).
    if key.code == KeyCode::Char('v') && paste_mod {
        let text = ed.editor.get_clipboard().unwrap_or_default();
        if !text.is_empty() {
            let mut cursor = ed.editor.get_cursor();
            let mut selection = ed.editor.get_selection();
            let code = ed.editor.code_mut();
            code.tx();
            code.set_state_before(cursor, selection);
            if let Some(sel) = &selection {
                if !sel.is_empty() {
                    let (start, end) = sel.sorted();
                    code.remove(start, end);
                    cursor = start;
                    selection = None;
                }
            }
            code.insert(cursor, &text);
            cursor += text.chars().count();
            code.set_state_after(cursor, selection);
            code.commit();
            ed.editor.set_cursor(cursor);
            ed.editor.set_selection(selection);
            ed.editor.reset_highlight_cache();
        }
        ed.selection_extend_mode = false;
        ed.editor.focus(&ed.area);
        return Some(AppAction::Continue);
    }

    // Extend when Shift (if reported) or F3 selection mode (MC-style).
    let extend = key.modifiers.contains(KeyModifiers::SHIFT) || ed.selection_extend_mode;
    // After 2nd F3: selection is visible but "stopped" — arrows must not clear it, typing must preserve it.
    let selection_frozen =
        !ed.selection_extend_mode && ed.editor.get_selection().map_or(false, |s| !s.is_empty());

    if key.code == KeyCode::Left {
        let cursor = ed.editor.get_cursor();
        if cursor > 0 {
            let new_cursor = cursor - 1;
            if extend {
                ed.editor.extend_selection(new_cursor);
            } else if !selection_frozen {
                ed.editor.clear_selection();
            }
            ed.editor.set_cursor(new_cursor);
        }
        ed.editor.focus(&ed.area);
        return Some(AppAction::Continue);
    }
    if key.code == KeyCode::Right {
        let code = ed.editor.code_ref();
        let cursor = ed.editor.get_cursor();
        let len = code.len_chars();
        if cursor < len {
            let new_cursor = cursor + 1;
            if extend {
                ed.editor.extend_selection(new_cursor);
            } else if !selection_frozen {
                ed.editor.clear_selection();
            }
            ed.editor.set_cursor(new_cursor);
        }
        ed.editor.focus(&ed.area);
        return Some(AppAction::Continue);
    }
    if key.code == KeyCode::Up {
        let code = ed.editor.code_ref();
        let cursor = ed.editor.get_cursor();
        let (row, col) = code.point(cursor);
        if row > 0 {
            let prev_start = code.line_to_char(row - 1);
            let prev_len = code.line_len(row - 1);
            let new_cursor = prev_start + col.min(prev_len);
            if extend {
                ed.editor.extend_selection(new_cursor);
            } else if !selection_frozen {
                ed.editor.clear_selection();
            }
            ed.editor.set_cursor(new_cursor);
        }
        ed.editor.focus(&ed.area);
        return Some(AppAction::Continue);
    }
    if key.code == KeyCode::Down {
        let code = ed.editor.code_ref();
        let cursor = ed.editor.get_cursor();
        let (row, col) = code.point(cursor);
        if row + 1 < code.len_lines() {
            let next_start = code.line_to_char(row + 1);
            let next_len = code.line_len(row + 1);
            let new_cursor = next_start + col.min(next_len);
            if extend {
                ed.editor.extend_selection(new_cursor);
            } else if !selection_frozen {
                ed.editor.clear_selection();
            }
            ed.editor.set_cursor(new_cursor);
        }
        ed.editor.focus(&ed.area);
        return Some(AppAction::Continue);
    }

    // Home: start of current line
    if key.code == KeyCode::Home {
        let new_cursor = {
            let code = ed.editor.code_ref();
            let cursor = ed.editor.get_cursor();
            code.line_boundaries(cursor).0
        };
        if extend {
            ed.editor.extend_selection(new_cursor);
        } else if !selection_frozen {
            ed.editor.clear_selection();
        }
        ed.editor.set_cursor(new_cursor);
        ed.editor.focus(&ed.area);
        return Some(AppAction::Continue);
    }
    // End: end of current line (before newline if line ends with \n)
    if key.code == KeyCode::End {
        let new_cursor = {
            let code = ed.editor.code_ref();
            let cursor = ed.editor.get_cursor();
            let (line_start, line_end) = code.line_boundaries(cursor);
            let end_pos = line_end.min(code.len_chars());
            if end_pos > line_start {
                let last_ch = code.slice(end_pos - 1, end_pos);
                if last_ch == "\n" {
                    end_pos - 1
                } else {
                    end_pos
                }
            } else {
                line_start
            }
        };
        if extend {
            ed.editor.extend_selection(new_cursor);
        } else if !selection_frozen {
            ed.editor.clear_selection();
        }
        ed.editor.set_cursor(new_cursor);
        ed.editor.focus(&ed.area);
        return Some(AppAction::Continue);
    }
    if key.code == KeyCode::PageUp {
        let new_cursor = {
            let code = ed.editor.code_ref();
            let (row, col) = code.point(ed.editor.get_cursor());
            let page_height = ed.area.height as usize;
            let new_row = row.saturating_sub(page_height);
            code.line_to_char(new_row) + col.min(code.line_len(new_row))
        };
        if extend {
            ed.editor.extend_selection(new_cursor);
        } else if !selection_frozen {
            ed.editor.clear_selection();
        }
        ed.editor.set_cursor(new_cursor);
        ed.editor.focus(&ed.area);
        return Some(AppAction::Continue);
    }
    if key.code == KeyCode::PageDown {
        let new_cursor = {
            let code = ed.editor.code_ref();
            let (row, col) = code.point(ed.editor.get_cursor());
            let page_height = ed.area.height as usize;
            let total_lines = code.len_lines();
            let new_row = (row + page_height).min(total_lines.saturating_sub(1));
            code.line_to_char(new_row) + col.min(code.line_len(new_row))
        };
        if extend {
            ed.editor.extend_selection(new_cursor);
        } else if !selection_frozen {
            ed.editor.clear_selection();
        }
        ed.editor.set_cursor(new_cursor);
        ed.editor.focus(&ed.area);
        return Some(AppAction::Continue);
    }

    // Forward delete (Del): ratatui-code-editor only handles Backspace, not KeyCode::Delete.
    if key.code == KeyCode::Delete && !ctrl {
        let mut cursor = ed.editor.get_cursor();
        let mut selection = ed.editor.get_selection();
        let len = ed.editor.code_ref().len_chars();
        let code = ed.editor.code_mut();
        code.tx();
        code.set_state_before(cursor, selection);
        if let Some(ref sel) = selection {
            if !sel.is_empty() {
                let (start, end) = sel.sorted();
                code.remove(start, end);
                cursor = start;
                selection = None;
            }
        } else if cursor < len {
            code.remove(cursor, cursor + 1);
        }
        code.set_state_after(cursor, selection);
        code.commit();
        ed.editor.set_cursor(cursor);
        ed.editor.set_selection(selection);
        ed.editor.reset_highlight_cache();
        ed.editor.focus(&ed.area);
        return Some(AppAction::Continue);
    }

    // When selection is frozen (after 2nd F3), typing and Enter must insert at cursor and shift selection — not clear it.
    if selection_frozen && !ctrl {
        let text_opt = match key.code {
            KeyCode::Char(c) => Some(c.to_string()),
            KeyCode::Enter => Some("\n".to_string()),
            _ => None,
        };
        if let Some(text) = text_opt {
            if let Some(sel) = ed.editor.get_selection() {
                if !sel.is_empty() {
                    let cursor = ed.editor.get_cursor();
                    let (start, end) = sel.sorted();
                    let len = text.chars().count();
                    let (new_cursor, new_sel) = {
                        let code = ed.editor.code_mut();
                        code.tx();
                        code.set_state_before(cursor, Some(sel));
                        code.insert(cursor, &text);
                        let new_cursor = cursor + len;
                        let new_start = if start >= cursor { start + len } else { start };
                        let new_end = if end >= cursor { end + len } else { end };
                        let new_sel = Selection::new(new_start, new_end);
                        code.set_state_after(new_cursor, Some(new_sel));
                        code.commit();
                        (new_cursor, new_sel)
                    };
                    ed.editor.set_cursor(new_cursor);
                    ed.editor.set_selection(Some(new_sel));
                    ed.editor.reset_highlight_cache();
                    ed.editor.focus(&ed.area);
                    return Some(AppAction::Continue);
                }
            }
        }
    }

    let _ = ed.editor.input(key, &ed.area);
    Some(AppAction::Continue)
}

/// Handle mouse when embedded editor is open. Returns true if handled.
/// Clicks on the top (file path) or bottom (hint) row are not passed to the editor.
pub fn handle_editor_mouse(
    app: &mut AppState,
    mouse_event: MouseEvent,
) -> bool {
    if app.editor_confirm_pending {
        return false;
    }
    match app.editor_screen {
        Some(EditorViewState::Ready(ref mut ed)) => {
            let hint_row = ed.area.y + ed.area.height;
            if mouse_event.row < ed.area.y || mouse_event.row >= hint_row {
                return true;
            }
            let _ = ed.editor.mouse(mouse_event, &ed.area);
            true
        }
        Some(EditorViewState::WarnLargeFile { .. })
        | Some(EditorViewState::Loading { .. })
        | Some(EditorViewState::BytesLoaded { .. }) => true,
        None => false,
    }
}

/// Display name for save confirmation toast (zip entry name or file basename).
fn editor_saved_display_name(ed: &EditorScreenState) -> String {
    if let Some(name) = &ed.edit_name {
        name.trim_end_matches('/').to_string()
    } else {
        Path::new(&ed.file_path)
            .file_name()
            .and_then(|n| n.to_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| ed.file_path.clone())
    }
}

/// Zip entry or fs file name with `.text` appended (e.g. `foo.mp3` -> `foo.mp3.text`).
fn entry_name_with_text_suffix(name: &str) -> String {
    format!("{}.text", name.trim_end_matches('/'))
}

/// Write buffer to the editor's target file. If `opened_with_invalid_utf8`, writes to `*.text`, updates paths, clears the flag.
/// Returns `Ok(true)` when the `.text` redirect was used.
fn write_editor_buffer(
    ed: &mut EditorScreenState,
    content: &[u8],
) -> io::Result<bool> {
    let redirect = ed.opened_with_invalid_utf8;
    match (&ed.edit_location, &ed.edit_name) {
        (Some(loc), Some(name)) => {
            let target_name = if redirect {
                entry_name_with_text_suffix(name)
            } else {
                name.clone()
            };
            write_file(loc, &target_name, content)?;
            if redirect {
                ed.edit_name = Some(target_name.clone());
                ed.file_path = join_path_display(loc, &target_name);
                ed.opened_with_invalid_utf8 = false;
            }
            Ok(redirect)
        }
        (None, None) => {
            let path = if redirect {
                let p = Path::new(&ed.file_path);
                let stem = p.file_name().and_then(|n| n.to_str()).unwrap_or("file");
                p.with_file_name(entry_name_with_text_suffix(stem))
            } else {
                PathBuf::from(&ed.file_path)
            };
            std::fs::write(&path, content)?;
            if redirect {
                ed.file_path = path.to_string_lossy().into_owned();
                ed.opened_with_invalid_utf8 = false;
            }
            Ok(redirect)
        }
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid editor path state",
        )),
    }
}

/// F2 save: write content to file and update initial_content. Works for FS and files inside ZIP.
pub fn save(app: &mut AppState) {
    if let Some(EditorViewState::Ready(ref mut ed)) = app.editor_screen {
        let content = ed.editor.get_content();
        match write_editor_buffer(ed, content.as_bytes()) {
            Err(e) => eprintln!("Save failed: {}", e),
            Ok(used_text_suffix) => {
                ed.initial_content = content;
                let name = editor_saved_display_name(ed);
                let (duration, msg) = if used_text_suffix {
                    (
                        Duration::from_secs(6),
                        format!(
                            "Non-UTF-8 opened as text: saved as \"{}\" (original left unchanged).",
                            name
                        ),
                    )
                } else {
                    (
                        Duration::from_secs(3),
                        format!("\"{}\" has been saved.", name),
                    )
                };
                app.set_timed_toast(duration, msg);
            }
        }
    }
}

fn refresh_panels_after_editor_close(app: &mut AppState) {
    let panel_height = compute_panel_height();
    let selected_name = app
        .active_panel_mut()
        .get_selected_file()
        .map(|f| f.name.clone());
    if let Some(name) = selected_name {
        if app.active_panel() == 0 {
            let _ = app.left_panel_mut().refresh_files_restore_selection(
                Some(&name),
                None,
                Some(panel_height),
            );
            let _ = app.right_panel_mut().refresh_files_restore_selection(
                None,
                None,
                Some(panel_height),
            );
        } else {
            let _ = app.right_panel_mut().refresh_files_restore_selection(
                Some(&name),
                None,
                Some(panel_height),
            );
            let _ = app.left_panel_mut().refresh_files_restore_selection(
                None,
                None,
                Some(panel_height),
            );
        }
    }
}

/// ESC with no unsaved changes: close editor and refresh panels.
pub fn close(app: &mut AppState) {
    cancel_editor_background(app);
    app.editor_screen = None;
    app.editor_confirm_pending = false;
    app.clear_timed_toast();
    if app.find_dialog.is_some() {
        app.focus = Focus::FindDialog;
    }
    refresh_panels_after_editor_close(app);
}

/// Apply user choice from "Save changes?" dialog.
pub fn apply_confirm_choice(
    app: &mut AppState,
    choice: EditorConfirmChoice,
) {
    app.editor_confirm_pending = false;
    match choice {
        EditorConfirmChoice::Save => {
            if let Some(EditorViewState::Ready(ref mut ed)) = app.editor_screen {
                let content = ed.editor.get_content();
                if let Err(e) = write_editor_buffer(ed, content.as_bytes()) {
                    eprintln!("Save failed: {}", e);
                } else {
                    ed.initial_content = content;
                }
            }
            app.editor_screen = None;
            app.clear_timed_toast();
            if app.find_dialog.is_some() {
                app.focus = Focus::FindDialog;
            }
            refresh_panels_after_editor_close(app);
        }
        EditorConfirmChoice::Discard => {
            cancel_editor_background(app);
            app.editor_screen = None;
            app.clear_timed_toast();
            if app.find_dialog.is_some() {
                app.focus = Focus::FindDialog;
            }
            refresh_panels_after_editor_close(app);
        }
        EditorConfirmChoice::Cancel => {}
    }
}

fn draw_editor_warn_or_loading(
    f: &mut Frame,
    app: &AppState,
    file_path: &str,
    size_for_warning: Option<u64>,
    preparing_editor: bool,
) {
    let area = f.area();
    let vp: ViewerPalette = app.ui_palette.viewer;
    let content_style = Style::default().bg(vp.background).fg(vp.text);
    let header_rect = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: 1,
    };
    let content_rect = Rect {
        x: area.x,
        y: area.y + 1,
        width: area.width,
        height: area.height.saturating_sub(2),
    };
    let bottom_rect = Rect {
        x: area.x,
        y: area.y + area.height.saturating_sub(1),
        width: area.width,
        height: 1,
    };
    let subtitle = if size_for_warning.is_some() {
        "Large file"
    } else if preparing_editor {
        "Preparing editor"
    } else {
        "Loading…"
    };
    let header = Line::from(vec![
        Span::styled(
            file_path,
            Style::default().fg(vp.header_path),
        ),
        Span::raw("  "),
        Span::styled(subtitle, Style::default().fg(vp.muted)),
    ]);
    f.render_widget(
        Paragraph::new(header).style(content_style),
        header_rect,
    );
    let msg: String = if let Some(sz) = size_for_warning {
        format!(
            "Size {}. Embedded editor loads the full file into memory (max {}).\n\nEnter: continue  Esc: cancel",
            format_byte_size(sz),
            format_byte_size(EDITOR_MAX_EMBEDDED_BYTES),
        )
    } else if preparing_editor {
        "File read finished. Next step builds the editor buffer (very large files may take a while).\n\nEsc: cancel — return to panels without opening".to_string()
    } else {
        "Reading file in background — Esc to cancel".to_string()
    };
    f.render_widget(
        Paragraph::new(msg)
            .style(content_style)
            .wrap(Wrap { trim: true }),
        content_rect,
    );
    let bottom_text = if size_for_warning.is_some() {
        " Enter: load file  Esc: cancel "
    } else {
        " Esc: cancel "
    };
    f.render_widget(
        Paragraph::new(bottom_text).style(content_style.fg(vp.muted)),
        bottom_rect,
    );
}

/// Draw the embedded editor and, if editor_confirm_pending, the "Save changes?" dialog.
/// The top row shows the file path (same style as F3 viewer); the bottom row is the hint.
/// The editor content area excludes both so the cursor cannot reach those lines.
pub fn draw(
    f: &mut Frame,
    app: &mut AppState,
) {
    TimedToast::clear_if_expired(&mut app.timed_toast);
    match app.editor_screen {
        Some(EditorViewState::WarnLargeFile {
            ref file_path,
            size_bytes,
            ..
        }) => {
            draw_editor_warn_or_loading(f, app, file_path, Some(size_bytes), false);
            return;
        }
        Some(EditorViewState::Loading { ref file_path, .. }) => {
            draw_editor_warn_or_loading(f, app, file_path, None, false);
            return;
        }
        Some(EditorViewState::BytesLoaded { ref file_path, .. }) => {
            draw_editor_warn_or_loading(f, app, file_path, None, true);
            return;
        }
        None => return,
        _ => {}
    }
    if let Some(EditorViewState::Ready(ref mut ed)) = app.editor_screen {
        let area = f.area();
        let vp: ViewerPalette = app.ui_palette.viewer;
        let header_height = 1u16;
        let bottom_height = 1u16;
        let content_height = area.height.saturating_sub(header_height + bottom_height);
        ed.area = Rect {
            x: area.x,
            y: area.y + header_height,
            width: area.width,
            height: content_height,
        };
        let main_bg = app.ui_palette.chrome.main_background;
        f.render_widget(
            Block::default().style(Style::default().bg(main_bg)),
            area,
        );

        let header_rect = Rect {
            x: area.x,
            y: area.y,
            width: area.width,
            height: header_height,
        };
        let header_fill = Style::default().bg(vp.background).fg(vp.text);
        let total_lines = ed.editor.code_ref().len_lines();
        let right_info = format!("EDIT | {} lines", total_lines);
        let path_span = ed.file_path.as_str();
        let pad_len = (header_rect.width as usize)
            .saturating_sub(path_span.len() + right_info.len())
            .max(1);
        let header_line = Line::from(vec![
            Span::styled(
                path_span,
                Style::default().fg(vp.header_path),
            ),
            Span::raw(" ".repeat(pad_len)),
            Span::styled(right_info, Style::default().fg(vp.muted)),
        ]);
        f.render_widget(
            Paragraph::new(header_line).style(header_fill),
            header_rect,
        );

        f.render_widget(&ed.editor, ed.area);
        let gutter_w = editor_gutter_width(&ed.editor, ed.area.width);
        if gutter_w > 0 {
            let gutter_bg = editor_gutter_background(main_bg);
            let buf = f.buffer_mut();
            for row in 0..ed.area.height {
                let py = ed.area.y + row;
                for col in 0..gutter_w {
                    let px = ed.area.x + col;
                    buf[(px, py)].set_bg(gutter_bg);
                }
            }
        }
        if let Some((cx, cy)) = ed.editor.get_visible_cursor(&ed.area) {
            f.set_cursor_position((cx, cy));
        }
        if area.height > bottom_height {
            let hint =
                " F3: start/stop selection | ←→↑↓ extend | F8: del line | Ctrl+X F find | Ctrl+C / Ctrl+V | F2: Save | Esc: exit ";
            let row = area.bottom().saturating_sub(bottom_height);
            let w = hint.chars().count().min(area.width as usize) as u16;
            let r = Rect {
                x: area.x,
                y: row,
                width: w,
                height: 1,
            };
            f.render_widget(
                Paragraph::new(hint).style(Style::default().bg(main_bg).fg(vp.muted)),
                r,
            );
        }
        if let Some(ref query) = ed.search_query {
            let cursor = ed.search_query_cursor.min(query.chars().count());
            draw_search_bar(
                f,
                area,
                query,
                cursor,
                &app.ui_palette.dialog,
            );
        }
        if let Some(ref t) = app.timed_toast {
            toast::draw_timed_bottom_left(f, area, &app.ui_palette, t);
        }
        if app.editor_confirm_pending {
            draw_confirm_dialog(f, app);
        }
    }
}

/// TUI search window (bordered box). Searches only in the opened file buffer; not the terminal/OS.
fn draw_search_bar(
    f: &mut Frame,
    area: Rect,
    query: &str,
    cursor_pos: usize,
    d: &DialogPalette,
) {
    let title = " Find (in file) ";
    let hint = " Enter: next  ←→: move  Esc: close ";
    let inner_w = 52u16;
    let w = inner_w.min(area.width.saturating_sub(4));
    let h = 5u16;
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let rect = Rect {
        x,
        y,
        width: w,
        height: h,
    };
    let style = d.fill_style();
    f.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .style(style.fg(d.border));
    f.render_widget(block, rect);
    let inner = rect.inner(Margin {
        horizontal: 1,
        vertical: 1,
    });
    let query_display_row = Rect {
        x: inner.x,
        y: inner.y,
        width: inner.width,
        height: 1,
    };
    let combined = format!("  {}", query);
    let vw = inner.width as usize;
    let cursor_char = 2usize.saturating_add(cursor_pos);
    let display_offset = text_input::horizontal_display_offset(cursor_char, vw);
    let chars: Vec<char> = combined.chars().collect();
    let len = chars.len();
    let start = display_offset.min(len);
    let end = (display_offset + vw).min(len);
    let vis: String = chars[start..end].iter().collect();
    let pad = vw.saturating_sub(end - start);
    let shown = format!("{}{}", vis, " ".repeat(pad));
    f.render_widget(
        Paragraph::new(shown.as_str()).style(style),
        query_display_row,
    );
    let hint_row = Rect {
        x: inner.x,
        y: inner.y + 2,
        width: inner.width,
        height: 1,
    };
    f.render_widget(
        Paragraph::new(hint).style(style.fg(d.text_muted)),
        hint_row,
    );
    let cursor_screen = cursor_char.saturating_sub(display_offset);
    let col = cursor_screen.min(inner.width.saturating_sub(1) as usize);
    f.set_cursor_position((inner.x + col as u16, inner.y));
}

/// Bounding box for "Save changes?" (must match [`draw_confirm_dialog`]).
pub fn save_changes_confirm_rect(area: Rect) -> Rect {
    let max_w = 48u16;
    let w = max_w.min(area.width.saturating_sub(4));
    let h = 9u16;
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    Rect {
        x,
        y,
        width: w,
        height: h,
    }
}

/// "Save changes?" when exiting editor with unsaved changes. Tab/↑↓ cycle, Enter confirms, 1/2/3 direct.
pub fn draw_confirm_dialog(
    f: &mut Frame,
    app: &AppState,
) {
    let area = f.area();
    let rect = save_changes_confirm_rect(area);
    let d = &app.ui_palette.dialog;
    let fill_style = d.fill_style();
    let orange = d.accent;
    f.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Save changes? ")
        .style(fill_style.fg(d.border));
    f.render_widget(block, rect);
    let inner = rect.inner(Margin {
        horizontal: 1,
        vertical: 1,
    });
    const PAD_H: u16 = 2;
    let content = Rect {
        x: inner.x + PAD_H,
        y: inner.y,
        width: inner.width.saturating_sub(PAD_H * 2),
        height: inner.height,
    };
    let msg = "File was modified.";
    f.render_widget(
        Paragraph::new(msg)
            .style(fill_style)
            .alignment(Alignment::Center),
        Rect {
            x: content.x,
            y: content.y,
            width: content.width,
            height: 1,
        },
    );
    let options = [
        (0, "1", "Save and close"),
        (1, "2", "Discard and close"),
        (2, "3", "Cancel (ESC)"),
    ];
    for (i, (idx, num, label)) in options.iter().enumerate() {
        let row_rect = Rect {
            x: content.x,
            y: content.y + 2 + i as u16,
            width: content.width,
            height: 1,
        };
        let focused = app.editor_confirm_focus == *idx;
        let line = Line::from(vec![
            Span::raw("  "),
            Span::styled(*num, orange),
            Span::raw(". "),
            Span::raw(*label),
        ]);
        let style = if focused {
            d.focus_row_style()
        } else {
            fill_style
        };
        f.render_widget(
            Paragraph::new(line).style(style),
            row_rect,
        );
    }
    let hint_y = content.y + 6;
    f.render_widget(
        Paragraph::new("↑↓ / Tab: choose   Enter: confirm")
            .style(Style::default().fg(d.text_muted))
            .alignment(Alignment::Center),
        Rect {
            x: content.x,
            y: hint_y,
            width: content.width,
            height: 1,
        },
    );
}

/// Return (option_rects) for editor confirm hit-testing. Rows 0=Save, 1=Discard, 2=Cancel.
pub fn editor_confirm_option_rects(area: Rect) -> Option<[(Rect, EditorConfirmChoice); 3]> {
    let rect = save_changes_confirm_rect(area);
    let inner = rect.inner(Margin {
        horizontal: 1,
        vertical: 1,
    });
    const PAD_H: u16 = 2;
    let content = Rect {
        x: inner.x + PAD_H,
        y: inner.y,
        width: inner.width.saturating_sub(PAD_H * 2),
        height: inner.height,
    };
    Some([
        (
            Rect {
                x: content.x,
                y: content.y + 2,
                width: content.width,
                height: 1,
            },
            EditorConfirmChoice::Save,
        ),
        (
            Rect {
                x: content.x,
                y: content.y + 3,
                width: content.width,
                height: 1,
            },
            EditorConfirmChoice::Discard,
        ),
        (
            Rect {
                x: content.x,
                y: content.y + 4,
                width: content.width,
                height: 1,
            },
            EditorConfirmChoice::Cancel,
        ),
    ])
}
