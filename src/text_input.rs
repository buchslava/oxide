//! Single-line text input state and key handling shared by dialogs (mkdir, archive, new file).
//! Cursor is a character index (for correct display with multi-byte characters).

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

use crate::dialog_layout::{self, single_input_button_rects};
use crate::styles::{DIALOG_BG, DIALOG_FOCUS, DIALOG_INPUT_BG_FOCUSED, DIALOG_INPUT_BG_UNFOCUSED};

/// Single-line text input: content and cursor position (character index).
/// Does not implement Clone to avoid accidental expensive cloning of the text buffer.
#[derive(Debug, Default)]
pub struct TextInputState {
    pub text: String,
    /// Cursor position as character index (number of characters before the cursor).
    pub cursor: usize,
}

impl TextInputState {
    pub fn new(initial: String) -> Self {
        let cursor = initial.chars().count();
        Self {
            text: initial,
            cursor,
        }
    }

    /// Insert a character at the cursor. Returns new state (pure transition).
    pub fn insert_char(mut self, c: char) -> Self {
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

    /// Delete the character before the cursor. Returns new state (pure transition).
    pub fn backspace(mut self) -> Self {
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

    /// Move cursor left. Returns new state (pure transition).
    pub fn move_left(self) -> Self {
        Self {
            cursor: self.cursor.saturating_sub(1),
            ..self
        }
    }

    /// Move cursor right. Returns new state (pure transition).
    pub fn move_right(self) -> Self {
        let len = self.text.chars().count();
        Self {
            cursor: (self.cursor + 1).min(len),
            ..self
        }
    }

    /// Move cursor to start. Returns new state (pure transition).
    pub fn move_home(self) -> Self {
        Self {
            cursor: 0,
            ..self
        }
    }

    /// Move cursor to end. Returns new state (pure transition).
    pub fn move_end(self) -> Self {
        Self {
            cursor: self.text.chars().count(),
            ..self
        }
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
                    return (input, focus, SingleInputKeyResult::Cancel);
                }
            }
            if focus == 0 && c.is_ascii() && !c.is_control() {
                (input.insert_char(c), focus, SingleInputKeyResult::Continue)
            } else {
                (input, focus, SingleInputKeyResult::Continue)
            }
        }
        KeyCode::Backspace if focus == 0 => (input.backspace(), focus, SingleInputKeyResult::Continue),
        KeyCode::Left if focus == 0 => (input.move_left(), focus, SingleInputKeyResult::Continue),
        KeyCode::Right if focus == 0 => (input.move_right(), focus, SingleInputKeyResult::Continue),
        _ => (input, focus, SingleInputKeyResult::Continue),
    }
}

/// Cursor x position for drawing the text input (content area and width).
pub fn input_cursor_x(content_rect: Rect, input: &TextInputState) -> u16 {
    let col = input.cursor_column() as u16;
    content_rect.x + col.min(content_rect.width.saturating_sub(1))
}

/// Draw a single-input dialog (title, prompt, one text field, Create/Cancel buttons).
/// Uses dialog_layout and styles. Callers use dialog_layout::single_input_dialog_button_rects(area) for hit-test.
pub fn draw_single_input_dialog(
    f: &mut Frame,
    area: Rect,
    title: &str,
    prompt: &str,
    input: &TextInputState,
    focus: usize,
) {
    let (rect, content) = dialog_layout::single_input_dialog_layout(area);
    let fill_style = Style::default().bg(DIALOG_BG).fg(Color::White);
    f.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .style(fill_style.fg(DIALOG_FOCUS));
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
        DIALOG_INPUT_BG_FOCUSED
    } else {
        DIALOG_INPUT_BG_UNFOCUSED
    };
    let input_style = Style::default().bg(input_bg).fg(Color::White);
    let input_padded = format!("{:<width$}", input.text, width = content.width as usize);
    f.render_widget(Paragraph::new(input_padded).style(input_style), input_rect);
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
        Style::default().bg(DIALOG_FOCUS).fg(Color::Black)
    } else {
        fill_style
    };
    let cancel_style = if focus == 2 {
        Style::default().bg(DIALOG_FOCUS).fg(Color::Black)
    } else {
        fill_style
    };
    f.render_widget(Paragraph::new(create_btn).style(create_style), create_rect);
    f.render_widget(Paragraph::new(cancel_btn).style(cancel_style), cancel_rect);
}
