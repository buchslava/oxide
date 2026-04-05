//! Single-line text input state and key handling shared by dialogs (mkdir, archive, new file).
//! Cursor is a character index (for correct display with multi-byte characters).

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

use crate::browser::clipboard;
use crate::ui::dialog_layout::{self, single_input_button_rects};
use crate::ui::theme::UiPalette;

/// Single-line text input: content, cursor, and optional selection anchor (for Shift+arrow).
/// Does not implement Clone to avoid accidental expensive cloning of the text buffer.
#[derive(Debug, Default)]
pub struct TextInputState {
    pub text: String,
    /// Cursor position as character index (number of characters before the cursor).
    pub cursor: usize,
    /// When Some, selection is from min(anchor, cursor) to max(anchor, cursor). Used for Shift+arrow.
    pub anchor: Option<usize>,
}

impl TextInputState {
    pub fn new(initial: String) -> Self {
        let cursor = initial.chars().count();
        Self {
            text: initial,
            cursor,
            anchor: None,
        }
    }

    /// Selection range (start, end) with start < end, or None if no selection.
    pub fn selection_bounds(&self) -> Option<(usize, usize)> {
        let anchor_pos = self.anchor?;
        let (selection_start, selection_end) =
            (anchor_pos.min(self.cursor), anchor_pos.max(self.cursor));
        if selection_start < selection_end {
            Some((selection_start, selection_end))
        } else {
            None
        }
    }

    /// Clear selection (anchor = None). Kept for API completeness.
    #[allow(dead_code)]
    pub fn clear_selection(mut self) -> Self {
        self.anchor = None;
        self
    }

    /// Selected text, or None if no selection.
    pub fn get_selected_text(&self) -> Option<String> {
        let (s, e) = self.selection_bounds()?;
        Some(self.text.chars().skip(s).take(e - s).collect())
    }

    /// Insert a character at the cursor (replacing selection if any). Returns new state (pure transition).
    pub fn insert_char(
        mut self,
        c: char,
    ) -> Self {
        let sel = self.selection_bounds();
        self.anchor = None;
        if let Some((s, e)) = sel {
            let byte_start = self.text.char_indices().nth(s).map(|(i, _)| i).unwrap_or(0);
            let byte_end = self
                .text
                .char_indices()
                .nth(e)
                .map(|(i, _)| i)
                .unwrap_or(self.text.len());
            self.text.drain(byte_start..byte_end);
            self.cursor = s;
        }
        let byte_pos = self
            .text
            .char_indices()
            .nth(self.cursor)
            .map(|(i, _)| i)
            .unwrap_or(self.text.len());
        self.text.insert(byte_pos, c);
        self.cursor += 1;
        self
    }

    /// Insert a string at the cursor (e.g. for paste); replaces selection if any. Returns new state (pure transition).
    /// For single-line use, newlines in `s` are replaced with space.
    pub fn insert_str(
        mut self,
        s: &str,
    ) -> Self {
        let s = s.replace('\r', "").replace('\n', " ");
        let sel = self.selection_bounds();
        self.anchor = None;
        if let Some((start, end)) = sel {
            let byte_start = self
                .text
                .char_indices()
                .nth(start)
                .map(|(i, _)| i)
                .unwrap_or(0);
            let byte_end = self
                .text
                .char_indices()
                .nth(end)
                .map(|(i, _)| i)
                .unwrap_or(self.text.len());
            self.text.drain(byte_start..byte_end);
            self.cursor = start;
        }
        if s.is_empty() {
            return self;
        }
        let byte_pos = self
            .text
            .char_indices()
            .nth(self.cursor)
            .map(|(i, _)| i)
            .unwrap_or(self.text.len());
        let added_chars = s.chars().count();
        self.text.insert_str(byte_pos, &s);
        self.cursor += added_chars;
        self
    }

    /// Delete the character before the cursor or the selection. Returns new state (pure transition).
    pub fn backspace(mut self) -> Self {
        let sel = self.selection_bounds();
        self.anchor = None;
        if let Some((s, e)) = sel {
            let byte_start = self.text.char_indices().nth(s).map(|(i, _)| i).unwrap_or(0);
            let byte_end = self
                .text
                .char_indices()
                .nth(e)
                .map(|(i, _)| i)
                .unwrap_or(self.text.len());
            self.text.drain(byte_start..byte_end);
            self.cursor = s;
            return self;
        }
        if self.cursor == 0 {
            return self;
        }
        self.cursor -= 1;
        let byte_start = self
            .text
            .char_indices()
            .nth(self.cursor)
            .map(|(i, _)| i)
            .unwrap_or(0);
        let byte_end = self
            .text
            .char_indices()
            .nth(self.cursor + 1)
            .map(|(i, _)| i)
            .unwrap_or(self.text.len());
        self.text.drain(byte_start..byte_end);
        self
    }

    /// Delete the character after the cursor or the selection (forward delete). Returns new state (pure transition).
    pub fn delete_forward(mut self) -> Self {
        let sel = self.selection_bounds();
        self.anchor = None;
        if let Some((s, e)) = sel {
            let byte_start = self.text.char_indices().nth(s).map(|(i, _)| i).unwrap_or(0);
            let byte_end = self
                .text
                .char_indices()
                .nth(e)
                .map(|(i, _)| i)
                .unwrap_or(self.text.len());
            self.text.drain(byte_start..byte_end);
            self.cursor = s;
            return self;
        }
        let len_chars = self.text.chars().count();
        if self.cursor >= len_chars {
            return self;
        }
        let byte_start = self
            .text
            .char_indices()
            .nth(self.cursor)
            .map(|(i, _)| i)
            .unwrap_or(self.text.len());
        let byte_end = self
            .text
            .char_indices()
            .nth(self.cursor + 1)
            .map(|(i, _)| i)
            .unwrap_or(self.text.len());
        self.text.drain(byte_start..byte_end);
        self
    }

    /// Move cursor left. If shift, extend selection; else clear selection.
    pub fn move_left(
        self,
        shift: bool,
    ) -> Self {
        let cursor = self.cursor.saturating_sub(1);
        let anchor = if shift {
            Some(self.anchor.unwrap_or(self.cursor))
        } else {
            None
        };
        Self {
            cursor,
            anchor,
            ..self
        }
    }

    /// Move cursor right. If shift, extend selection; else clear selection.
    pub fn move_right(
        self,
        shift: bool,
    ) -> Self {
        let len = self.text.chars().count();
        let cursor = (self.cursor + 1).min(len);
        let anchor = if shift {
            Some(self.anchor.unwrap_or(self.cursor))
        } else {
            None
        };
        Self {
            cursor,
            anchor,
            ..self
        }
    }

    /// Move cursor to start. If shift, extend selection; else clear selection.
    pub fn move_home(
        self,
        shift: bool,
    ) -> Self {
        let anchor = if shift {
            Some(self.anchor.unwrap_or(self.cursor))
        } else {
            None
        };
        Self {
            cursor: 0,
            anchor,
            ..self
        }
    }

    /// Move cursor to end. If shift, extend selection; else clear selection.
    pub fn move_end(
        self,
        shift: bool,
    ) -> Self {
        let len = self.text.chars().count();
        let anchor = if shift {
            Some(self.anchor.unwrap_or(self.cursor))
        } else {
            None
        };
        Self {
            cursor: len,
            anchor,
            ..self
        }
    }

    /// Select all text (anchor=0, cursor=len).
    pub fn select_all(mut self) -> Self {
        self.anchor = Some(0);
        self.cursor = self.text.chars().count();
        self
    }

    /// Character column of the cursor for rendering (0-based, in characters).
    pub fn cursor_column(&self) -> usize {
        self.cursor
    }
}

/// Result of handling a key in a single-input dialog (one text field + Create + Cancel).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SingleInputKeyResult {
    /// State was updated (focus or text); redraw, no app action.
    Continue,
    /// User confirmed (Enter with focus on input or Create).
    Confirm,
    /// User cancelled (Esc or Ctrl+C or Enter with focus on Cancel).
    Cancel,
    /// Ctrl+O: suspend to shell.
    Suspend,
}

/// Number of focus targets: 0 = text input, 1 = primary button (Create), 2 = Cancel.
const FOCUS_COUNT: usize = 3;

/// Terminal Backspace is often sent as Ctrl+H (ASCII BS). Crossterm reports it as `Char('h')` with CONTROL.
#[inline]
pub fn is_ctrl_backspace(
    modifiers: KeyModifiers,
    c: char,
) -> bool {
    modifiers.contains(KeyModifiers::CONTROL) && c == 'h'
}

/// Pure key handler: (input, focus) + key → (new_input, new_focus, result). No mutation.
#[must_use]
pub fn handle_single_input_key(
    input: TextInputState,
    focus: usize,
    code: KeyCode,
    modifiers: KeyModifiers,
) -> (TextInputState, usize, SingleInputKeyResult) {
    let code = match code {
        KeyCode::Char('\t') => KeyCode::Tab,
        other => other,
    };
    match code {
        KeyCode::Tab | KeyCode::Char('\t') => {
            let new_focus = (focus + 1) % FOCUS_COUNT;
            (input, new_focus, SingleInputKeyResult::Continue)
        }
        KeyCode::BackTab => {
            let new_focus = (focus + FOCUS_COUNT - 1) % FOCUS_COUNT;
            (input, new_focus, SingleInputKeyResult::Continue)
        }
        KeyCode::Up => {
            let new_focus = (focus + FOCUS_COUNT - 1) % FOCUS_COUNT;
            (input, new_focus, SingleInputKeyResult::Continue)
        }
        KeyCode::Down => {
            let new_focus = (focus + 1) % FOCUS_COUNT;
            (input, new_focus, SingleInputKeyResult::Continue)
        }
        KeyCode::Enter => {
            let result = if focus == 2 {
                SingleInputKeyResult::Cancel
            } else {
                SingleInputKeyResult::Confirm
            };
            (input, focus, result)
        }
        KeyCode::Esc => (input, focus, SingleInputKeyResult::Cancel),
        KeyCode::Char(c) => {
            if modifiers.contains(KeyModifiers::CONTROL) {
                if c == 'o' {
                    return (input, focus, SingleInputKeyResult::Suspend);
                }
                if c == 'c' {
                    if focus == 0 {
                        if let Some(s) = input.get_selected_text() {
                            clipboard::set(&s);
                        } else if !input.text.is_empty() {
                            clipboard::set(&input.text);
                        }
                    }
                    if focus != 0 {
                        return (input, focus, SingleInputKeyResult::Cancel);
                    }
                    return (input, focus, SingleInputKeyResult::Continue);
                }
                if c == 'a' && focus == 0 && !input.text.is_empty() {
                    return (input.select_all(), focus, SingleInputKeyResult::Continue);
                }
                if c == 'v' && focus == 0 {
                    if let Some(s) = clipboard::get() {
                        return (input.insert_str(&s), focus, SingleInputKeyResult::Continue);
                    }
                    return (input, focus, SingleInputKeyResult::Continue);
                }
            }
            if is_ctrl_backspace(modifiers, c) && focus == 0 {
                return (input.backspace(), focus, SingleInputKeyResult::Continue);
            }
            if focus == 0 && c.is_ascii() && !c.is_control() {
                (input.insert_char(c), focus, SingleInputKeyResult::Continue)
            } else {
                (input, focus, SingleInputKeyResult::Continue)
            }
        }
        KeyCode::Backspace if focus == 0 => {
            (input.backspace(), focus, SingleInputKeyResult::Continue)
        }
        KeyCode::Delete if focus == 0 => {
            (input.delete_forward(), focus, SingleInputKeyResult::Continue)
        }
        KeyCode::Left if focus == 0 => (
            input.move_left(modifiers.contains(KeyModifiers::SHIFT)),
            focus,
            SingleInputKeyResult::Continue,
        ),
        KeyCode::Right if focus == 0 => (
            input.move_right(modifiers.contains(KeyModifiers::SHIFT)),
            focus,
            SingleInputKeyResult::Continue,
        ),
        KeyCode::Home if focus == 0 => (
            input.move_home(modifiers.contains(KeyModifiers::SHIFT)),
            focus,
            SingleInputKeyResult::Continue,
        ),
        KeyCode::End if focus == 0 => (
            input.move_end(modifiers.contains(KeyModifiers::SHIFT)),
            focus,
            SingleInputKeyResult::Continue,
        ),
        _ => (input, focus, SingleInputKeyResult::Continue),
    }
}

/// Cursor x position for drawing the text input (content area and width).
pub fn input_cursor_x(
    content_rect: Rect,
    input: &TextInputState,
) -> u16 {
    let col = input.cursor_column() as u16;
    content_rect.x + col.min(content_rect.width.saturating_sub(1))
}

/// Build a Line with normal and selection spans for the given input (full line, no scroll).
/// Pads to `width` with spaces. Use for single-input dialog and rename name field.
pub fn input_line_with_selection(
    input: &TextInputState,
    width: usize,
    base_style: Style,
    selection_style: Style,
) -> Line<'static> {
    let chars: Vec<char> = input.text.chars().collect();
    let len = chars.len();
    let sel = input.selection_bounds();
    let mut spans = Vec::new();
    let mut i = 0;
    while i < len.min(width) {
        let in_sel = sel.map(|(s, e)| i >= s && i < e).unwrap_or(false);
        let style = if in_sel { selection_style } else { base_style };
        let j = (i + 1..=len.min(width))
            .find(|&k| {
                sel.map(|(s, e)| (k >= s && k < e) != (i >= s && i < e))
                    .unwrap_or(false)
            })
            .unwrap_or(len.min(width));
        let s: String = chars[i..j].iter().collect();
        if !s.is_empty() {
            spans.push(Span::styled(s, style));
        }
        i = j;
    }
    let pad = width.saturating_sub(len);
    if pad > 0 {
        spans.push(Span::styled(" ".repeat(pad), base_style));
    }
    Line::from(spans)
}

/// Build a Line with selection for a visible slice (display_offset..display_offset+visible_width).
/// Use for find dialog inputs that scroll horizontally.
pub fn input_line_with_selection_slice(
    input: &TextInputState,
    display_offset: usize,
    visible_width: usize,
    base_style: Style,
    selection_style: Style,
) -> Line<'static> {
    let chars: Vec<char> = input.text.chars().collect();
    let len = chars.len();
    let sel = input.selection_bounds();
    let mut spans = Vec::new();
    let start = display_offset.min(len);
    let end = (display_offset + visible_width).min(len);
    let mut i = start;
    while i < end {
        let in_sel = sel.map(|(s, e)| i >= s && i < e).unwrap_or(false);
        let style = if in_sel { selection_style } else { base_style };
        let j = (i + 1..=end)
            .find(|&k| {
                sel.map(|(s, e)| (k >= s && k < e) != (i >= s && i < e))
                    .unwrap_or(false)
            })
            .unwrap_or(end);
        let s: String = chars[i..j].iter().collect();
        if !s.is_empty() {
            spans.push(Span::styled(s, style));
        }
        i = j;
    }
    let pad = visible_width.saturating_sub(end - start);
    if pad > 0 {
        spans.push(Span::styled(" ".repeat(pad), base_style));
    }
    Line::from(spans)
}

/// Draw a single-input dialog (title, prompt, one text field, Create/Cancel buttons).
/// Uses dialog_layout and styles. Mouse hit-test: [`dialog_layout::hit_test_single_input_dialog`](crate::ui::dialog_layout::hit_test_single_input_dialog).
pub fn draw_single_input_dialog(
    f: &mut Frame,
    area: Rect,
    title: &str,
    prompt: &str,
    input: &TextInputState,
    focus: usize,
    palette: &UiPalette,
) {
    let d = &palette.dialog;
    let (rect, content) = dialog_layout::single_input_dialog_layout(area);
    let fill_style = d.fill_style();
    f.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .style(fill_style.fg(d.border));
    f.render_widget(block, rect);

    f.render_widget(
        Paragraph::new(prompt).style(fill_style),
        Rect {
            x: content.x,
            y: content.y,
            width: content.width,
            height: 1,
        },
    );
    let input_y = content.y + 2;
    let input_rect = Rect {
        x: content.x,
        y: input_y,
        width: content.width,
        height: 1,
    };
    let input_focused = focus == 0;
    let input_bg = if input_focused {
        d.input_bg_focused
    } else {
        d.input_bg_unfocused
    };
    let base_style = Style::default().bg(input_bg).fg(d.text);
    let selection_style = Style::default().bg(d.input_selection_bg).fg(d.text);
    let line =
        input_line_with_selection(input, content.width as usize, base_style, selection_style);
    f.render_widget(Paragraph::new(line), input_rect);
    if input_focused {
        let cursor_x = input_cursor_x(input_rect, input);
        if cursor_x < content.x + content.width {
            f.set_cursor_position((cursor_x, input_y));
        }
    }

    let (create_rect, cancel_rect) = single_input_button_rects(content);
    let create_btn = Line::from(vec![Span::raw("  Create  ")]);
    let cancel_btn = Line::from(vec![Span::raw("  Cancel  ")]);
    let create_style = if focus == 1 {
        d.focus_row_style()
    } else {
        fill_style
    };
    let cancel_style = if focus == 2 {
        d.focus_row_style()
    } else {
        fill_style
    };
    f.render_widget(Paragraph::new(create_btn).style(create_style), create_rect);
    f.render_widget(Paragraph::new(cancel_btn).style(cancel_style), cancel_rect);
}
