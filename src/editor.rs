//! Embedded code editor (F4): open file, edit, F2 save, ESC exit, Page Up/Down, Home/End,
//! Ctrl+F search (in file), unsaved-changes dialog.
//! Selection (MC-style): F3 starts or stops selection; then ←→↑↓ extend. Ctrl+C copies then clears.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::{Alignment, Margin, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};
use ratatui_code_editor::editor::Editor;
use ratatui_code_editor::selection::Selection;
use ratatui_code_editor::theme::vesper;

use crate::app_state::AppState;
use crate::core::location::PanelLocation;
use crate::events::AppAction;
use crate::panel::PanelOperations;

/// State when the embedded code editor is open (F4).
pub struct EditorScreenState {
    /// Display path (for title/lang). When editing inside Zip, this is the virtual path.
    pub file_path: String,
    /// Content when file was opened; used to detect unsaved changes.
    pub initial_content: String,
    pub editor: Editor,
    /// Last draw area for the editor (used for input/mouse). When search is open, height is reduced by 1.
    pub area: Rect,
    /// When Some, search bar is open and the string is the current query (Ctrl+F).
    pub search_query: Option<String>,
    /// Cursor position in the search query (0..=len). Only used when search_query is Some.
    pub search_query_cursor: usize,
    /// F3 selection mode (MC-style): when true, arrows extend selection.
    pub selection_extend_mode: bool,
    /// When editing a file inside a Zip, these are set; otherwise None (save uses file_path to fs).
    pub edit_location: Option<PanelLocation>,
    pub edit_name: Option<String>,
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

/// Paste the given text as-is at the editor cursor (same as Ctrl+V). Replaces selection if any.
/// Returns Some(Continue) when the editor is open and paste was applied (or text was empty); None when not in editor.
pub fn paste_text_as_is(
    app: &mut AppState,
    text: &str,
) -> Option<AppAction> {
    let ed = app.editor_screen.as_mut()?;
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
            let content = match crate::core::panel_backend::read_file(&loc, &file.name) {
                Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
                Err(_) => return false,
            };
            let file_path_str = crate::core::panel_backend::join_path_display(&loc, &file.name);
            let lang = get_lang_from_path(&file_path_str);
            let theme = vesper();
            let editor = Editor::new(lang, &content, theme);
            let (edit_location, edit_name) = match &loc {
                crate::core::location::PanelLocation::Zip { .. } => {
                    (Some(loc), Some(file.name.clone()))
                }
                crate::core::location::PanelLocation::Fs(_) => (None, None),
            };
            app.editor_screen = Some(EditorScreenState {
                file_path: file_path_str,
                initial_content: content,
                editor,
                area: Rect::default(),
                search_query: None,
                search_query_cursor: 0,
                selection_extend_mode: false,
                edit_location,
                edit_name,
            });
            app.editor_confirm_pending = false;
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
    let content = std::fs::read_to_string(&path).unwrap_or_default();
    let lang = get_lang_from_path(&file_path_str);
    let theme = vesper();
    let editor = Editor::new(lang, &content, theme);
    app.editor_screen = Some(EditorScreenState {
        file_path: file_path_str,
        initial_content: content,
        editor,
        area: Rect::default(),
        search_query: None,
        search_query_cursor: 0,
        selection_extend_mode: false,
        edit_location: None,
        edit_name: None,
    });
    app.editor_confirm_pending = false;
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
    let ed = app.editor_screen.as_mut()?;
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    #[cfg(target_os = "macos")]
    let cmd_like =
        key.modifiers.contains(KeyModifiers::SUPER) || key.modifiers.contains(KeyModifiers::META);
    #[cfg(not(target_os = "macos"))]
    let cmd_like = false;
    // On macOS, Cmd+V/Cmd+C are the standard shortcuts; also accept Ctrl+V/Ctrl+C. Use same code path for both.
    let paste_mod = ctrl || cmd_like;
    let copy_mod = ctrl || cmd_like;

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
                let q = query.clone();
                find_next(ed, &q);
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
            KeyCode::Char(c) if !ctrl => {
                let mut chars: Vec<char> = query.chars().collect();
                let pos = (*cursor).min(chars.len());
                chars.insert(pos, c);
                *query = chars.into_iter().collect();
                *cursor = pos + 1;
                return Some(AppAction::Continue);
            }
            KeyCode::Char('f') if ctrl => {
                ed.editor.clear_selection();
                ed.search_query = None;
                return Some(AppAction::Continue);
            }
            KeyCode::F(3) => {
                ed.editor.clear_selection();
                ed.search_query = None;
                return Some(AppAction::Continue);
            }
            _ => return Some(AppAction::Continue),
        }
    }

    // Ctrl+F: open search
    if key.code == KeyCode::Char('f') && ctrl {
        ed.search_query = Some(String::new());
        ed.search_query_cursor = 0;
        return Some(AppAction::Continue);
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
/// Clicks on the bottom (hint) row are not passed to the editor so the cursor cannot move there.
pub fn handle_editor_mouse(
    app: &mut AppState,
    mouse_event: crossterm::event::MouseEvent,
) -> bool {
    if let Some(ref mut ed) = app.editor_screen {
        let hint_row = ed.area.y + ed.area.height;
        if mouse_event.row >= hint_row {
            return true;
        }
        let _ = ed.editor.mouse(mouse_event, &ed.area);
        return true;
    }
    false
}

/// F2 save: write content to file and update initial_content. Works for FS and files inside ZIP.
pub fn save(app: &mut AppState) {
    if let Some(ref mut ed) = app.editor_screen {
        let content = ed.editor.get_content();
        let result = match (&ed.edit_location, &ed.edit_name) {
            (Some(loc), Some(name)) => {
                crate::core::panel_backend::write_file(loc, name, content.as_bytes())
            }
            _ => std::fs::write(&ed.file_path, &content),
        };
        if let Err(e) = result {
            eprintln!("Save failed: {}", e);
        } else {
            ed.initial_content = content;
        }
    }
}

fn panel_height() -> usize {
    crate::util::compute_panel_height()
}

fn refresh_panels_after_editor_close(app: &mut AppState) {
    let panel_height = panel_height();
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
    app.editor_screen = None;
    app.editor_confirm_pending = false;
    if app.find_dialog.is_some() {
        app.focus = crate::app_state::Focus::FindDialog;
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
            if let Some(ref mut ed) = app.editor_screen {
                let content = ed.editor.get_content();
                let _ = match (&ed.edit_location, &ed.edit_name) {
                    (Some(loc), Some(name)) => {
                        crate::core::panel_backend::write_file(loc, name, content.as_bytes())
                    }
                    _ => std::fs::write(&ed.file_path, &content),
                };
                ed.initial_content = content;
            }
            app.editor_screen = None;
            if app.find_dialog.is_some() {
                app.focus = crate::app_state::Focus::FindDialog;
            }
            refresh_panels_after_editor_close(app);
        }
        EditorConfirmChoice::Discard => {
            app.editor_screen = None;
            if app.find_dialog.is_some() {
                app.focus = crate::app_state::Focus::FindDialog;
            }
            refresh_panels_after_editor_close(app);
        }
        EditorConfirmChoice::Cancel => {}
    }
}

/// Draw the embedded editor and, if editor_confirm_pending, the "Save changes?" dialog.
/// The bottom row is reserved for the hint; the editor content area excludes it so the cursor
/// cannot reach that line.
pub fn draw(
    f: &mut Frame,
    app: &mut AppState,
) {
    if let Some(ref mut ed) = app.editor_screen {
        let area = f.area();
        let content_height = area.height.saturating_sub(1);
        ed.area = Rect {
            x: area.x,
            y: area.y,
            width: area.width,
            height: content_height,
        };
        let dark_bg = Color::Rgb(30, 30, 35);
        f.render_widget(Block::default().style(Style::default().bg(dark_bg)), area);
        f.render_widget(&ed.editor, ed.area);
        if let Some((cx, cy)) = ed.editor.get_visible_cursor(&ed.area) {
            f.set_cursor_position((cx, cy));
        }
        if area.height > 0 {
            let hint =
                " F3: start/stop selection | ←→↑↓ extend | Ctrl+C / Ctrl+V | F2: Save | Esc: exit ";
            let row = area.bottom().saturating_sub(1);
            let w = hint.chars().count().min(area.width as usize) as u16;
            let r = Rect {
                x: area.x,
                y: row,
                width: w,
                height: 1,
            };
            f.render_widget(
                Paragraph::new(hint).style(Style::default().bg(dark_bg).fg(Color::DarkGray)),
                r,
            );
        }
        if let Some(ref query) = ed.search_query {
            let cursor = ed.search_query_cursor.min(query.chars().count());
            draw_search_bar(f, area, query, cursor);
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
    // Same dialog background as F7 / F2 — distinct from editor panel.
    let dialog_bg = Color::Rgb(60, 60, 60);
    let style = Style::default().bg(dialog_bg).fg(Color::White);
    f.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .style(style.fg(Color::Cyan));
    f.render_widget(block, rect);
    let inner = rect.inner(Margin {
        horizontal: 1,
        vertical: 1,
    });
    let line0 = Rect {
        x: inner.x,
        y: inner.y,
        width: inner.width,
        height: 1,
    };
    let text = format!("  {}", query);
    f.render_widget(Paragraph::new(text.as_str()).style(style), line0);
    let hint_row = Rect {
        x: inner.x,
        y: inner.y + 2,
        width: inner.width,
        height: 1,
    };
    f.render_widget(
        Paragraph::new(hint).style(style.fg(Color::DarkGray)),
        hint_row,
    );
    let cursor_col = (2 + cursor_pos).min(inner.width as usize);
    f.set_cursor_position((inner.x + cursor_col as u16, inner.y));
}

/// "Save changes?" when exiting editor with unsaved changes. Tab/↑↓ cycle, Enter confirms, 1/2/3 direct.
pub fn draw_confirm_dialog(
    f: &mut Frame,
    app: &AppState,
) {
    let area = f.area();
    let max_w = 48u16;
    let w = max_w.min(area.width.saturating_sub(4));
    let h = 9u16;
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let rect = Rect {
        x,
        y,
        width: w,
        height: h,
    };
    let menu_bg = Color::Rgb(60, 60, 60);
    let fill_style = Style::default().bg(menu_bg).fg(Color::White);
    let orange = Color::Rgb(255, 180, 80);
    f.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Save changes? ")
        .style(fill_style.fg(Color::Cyan));
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
            Style::default().bg(Color::Cyan).fg(Color::Black)
        } else {
            fill_style
        };
        f.render_widget(Paragraph::new(line).style(style), row_rect);
    }
    let hint_y = content.y + 6;
    f.render_widget(
        Paragraph::new("↑↓ / Tab: choose   Enter: confirm")
            .style(Style::default().fg(Color::DarkGray))
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
    let max_w = 48u16;
    let w = max_w.min(area.width.saturating_sub(4));
    let h = 9u16;
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let rect = Rect {
        x,
        y,
        width: w,
        height: h,
    };
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
