//! + / − : mark or unmark files by file pattern; wildcards (*, ?) or regex per F9 Settings (same as Find).
//! In wildcard mode, `|` separates alternative globs (not in regex mode).

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::{
    layout::{Alignment, Margin, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

use crate::app::events::AppAction;
use crate::app::state::AppState;
use crate::ui::dialog_layout::{self, DEFAULT_PAD_H};
use crate::ui::styles::{
    DIALOG_ACCENT, DIALOG_BG, DIALOG_FOCUS, DIALOG_INPUT_BG_FOCUSED, DIALOG_INPUT_BG_UNFOCUSED,
    DIALOG_INPUT_SELECTION_BG,
};
use crate::ui::text_input::{self, TextInputState};

/// Whether the dialog adds marks or removes them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatternSelectMode {
    Mark,
    Unmark,
}

/// File pattern + case option (same as Find) + Apply/Cancel.
#[derive(Debug)]
pub struct PatternSelectDialogState {
    pub mode: PatternSelectMode,
    pub pattern_input: TextInputState,
    /// Same as Find dialog "File name case sensitive" (default off).
    pub file_case_sensitive: bool,
    /// 0 = pattern, 1 = case row, 2 = Apply, 3 = Cancel.
    pub focus: usize,
}

const FOCUS_COUNT: usize = 4;

pub fn open_mark(app: &mut AppState) {
    app.pattern_select_dialog = Some(PatternSelectDialogState {
        mode: PatternSelectMode::Mark,
        pattern_input: TextInputState::new(app.last_file_name_pattern.clone()),
        file_case_sensitive: false,
        focus: 0,
    });
}

pub fn open_unmark(app: &mut AppState) {
    app.pattern_select_dialog = Some(PatternSelectDialogState {
        mode: PatternSelectMode::Unmark,
        pattern_input: TextInputState::new(app.last_file_name_pattern.clone()),
        file_case_sensitive: false,
        focus: 0,
    });
}

pub fn cancel(app: &mut AppState) {
    if let Some(d) = app.pattern_select_dialog.take() {
        app.set_last_file_name_pattern(&d.pattern_input.text);
    }
}

/// Remove dialog; caller reads fields before drop if needed.
pub fn take(app: &mut AppState) -> Option<PatternSelectDialogState> {
    app.pattern_select_dialog.take()
}

pub fn handle_key(
    app: &mut AppState,
    code: KeyCode,
    modifiers: KeyModifiers,
) -> Option<AppAction> {
    let d = app.pattern_select_dialog.take()?;
    let (new_d, action) = d.handle_key(code, modifiers);
    app.pattern_select_dialog = new_d;
    Some(action)
}

impl PatternSelectDialogState {
    #[must_use]
    fn handle_key(
        mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> (Option<Self>, AppAction) {
        let code = match code {
            KeyCode::Char('\t') => KeyCode::Tab,
            other => other,
        };
        match code {
            KeyCode::Esc => (None, AppAction::PatternSelectCancel),
            KeyCode::Char(c) if modifiers.contains(KeyModifiers::CONTROL) && c == 'o' => {
                (Some(self), AppAction::Suspend)
            }
            KeyCode::Char(c) if modifiers.contains(KeyModifiers::CONTROL) && c == 'c' => {
                if self.focus == 0 {
                    if let Some(s) = self.pattern_input.get_selected_text() {
                        crate::browser::clipboard::set(&s);
                    } else if !self.pattern_input.text.is_empty() {
                        crate::browser::clipboard::set(&self.pattern_input.text);
                    }
                    return (Some(self), AppAction::Continue);
                }
                (None, AppAction::PatternSelectCancel)
            }
            KeyCode::Char(c) if modifiers.contains(KeyModifiers::CONTROL) && c == 'a' => {
                if self.focus == 0 && !self.pattern_input.text.is_empty() {
                    self.pattern_input = self.pattern_input.select_all();
                }
                (Some(self), AppAction::Continue)
            }
            KeyCode::Char(c) if modifiers.contains(KeyModifiers::CONTROL) && c == 'v' => {
                if self.focus == 0 {
                    if let Some(s) = crate::browser::clipboard::get() {
                        self.pattern_input = self.pattern_input.insert_str(&s);
                    }
                }
                (Some(self), AppAction::Continue)
            }
            KeyCode::Tab | KeyCode::Char('\t') => {
                self.focus = (self.focus + 1) % FOCUS_COUNT;
                (Some(self), AppAction::Continue)
            }
            KeyCode::BackTab => {
                self.focus = (self.focus + FOCUS_COUNT - 1) % FOCUS_COUNT;
                (Some(self), AppAction::Continue)
            }
            KeyCode::Up => {
                self.focus = (self.focus + FOCUS_COUNT - 1) % FOCUS_COUNT;
                (Some(self), AppAction::Continue)
            }
            KeyCode::Down => {
                self.focus = (self.focus + 1) % FOCUS_COUNT;
                (Some(self), AppAction::Continue)
            }
            KeyCode::Enter => {
                if self.focus == 3 {
                    return (None, AppAction::PatternSelectCancel);
                }
                if self.focus == 1 {
                    self.file_case_sensitive = !self.file_case_sensitive;
                    return (Some(self), AppAction::Continue);
                }
                if self.focus == 2 || self.focus == 0 {
                    // Keep state until main handles `PatternSelectConfirm` and calls `take()`.
                    return (Some(self), AppAction::PatternSelectConfirm);
                }
                (Some(self), AppAction::Continue)
            }
            KeyCode::Char(' ') if self.focus == 1 => {
                self.file_case_sensitive = !self.file_case_sensitive;
                (Some(self), AppAction::Continue)
            }
            KeyCode::Backspace if self.focus == 0 => {
                self.pattern_input = self.pattern_input.backspace();
                (Some(self), AppAction::Continue)
            }
            KeyCode::Left if self.focus == 0 => {
                self.pattern_input = self
                    .pattern_input
                    .move_left(modifiers.contains(KeyModifiers::SHIFT));
                (Some(self), AppAction::Continue)
            }
            KeyCode::Right if self.focus == 0 => {
                self.pattern_input = self
                    .pattern_input
                    .move_right(modifiers.contains(KeyModifiers::SHIFT));
                (Some(self), AppAction::Continue)
            }
            KeyCode::Home if self.focus == 0 => {
                self.pattern_input = self
                    .pattern_input
                    .move_home(modifiers.contains(KeyModifiers::SHIFT));
                (Some(self), AppAction::Continue)
            }
            KeyCode::End if self.focus == 0 => {
                self.pattern_input = self
                    .pattern_input
                    .move_end(modifiers.contains(KeyModifiers::SHIFT));
                (Some(self), AppAction::Continue)
            }
            KeyCode::Char(c) if self.focus == 0 && c.is_ascii() && !c.is_control() => {
                self.pattern_input = self.pattern_input.insert_char(c);
                (Some(self), AppAction::Continue)
            }
            _ => (Some(self), AppAction::Continue),
        }
    }
}

/// Draw modal: pattern row, case row, hint, Apply/Cancel (layout aligned with single-input dialogs).
pub fn draw(
    f: &mut Frame,
    app: &mut AppState,
) {
    let Some(ref d) = app.pattern_select_dialog else {
        return;
    };
    let title = match d.mode {
        PatternSelectMode::Mark => " Select by pattern (+) ",
        PatternSelectMode::Unmark => " Deselect by pattern (−) ",
    };
    let area = f.area();
    let rect = dialog_layout::centered_dialog_rect(area, 64, 12);
    let fill_style = Style::default().bg(DIALOG_BG).fg(Color::White);
    let border_style = fill_style.fg(DIALOG_FOCUS);

    f.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .style(border_style);
    f.render_widget(block, rect);

    let inner = rect.inner(Margin {
        horizontal: 1,
        vertical: 1,
    });
    let content = dialog_layout::dialog_content_rect(inner, DEFAULT_PAD_H);

    let label_style = fill_style;
    let pattern_label = if app.persisted_settings.file_pattern_uses_regex() {
        "File pattern (regular expression, same as Find):"
    } else {
        "File pattern (wildcards * and ?, same as Find):"
    };
    f.render_widget(
        Paragraph::new(pattern_label).style(label_style),
        Rect {
            x: content.x,
            y: content.y,
            width: content.width,
            height: 1,
        },
    );

    let input_rect = Rect {
        x: content.x,
        y: content.y + 1,
        width: content.width,
        height: 1,
    };
    let input_base = if d.focus == 0 {
        DIALOG_INPUT_BG_FOCUSED
    } else {
        DIALOG_INPUT_BG_UNFOCUSED
    };
    let line = text_input::input_line_with_selection(
        &d.pattern_input,
        content.width as usize,
        Style::default().bg(input_base).fg(Color::White),
        Style::default()
            .bg(DIALOG_INPUT_SELECTION_BG)
            .fg(Color::White),
    );
    f.render_widget(
        Paragraph::new(line).style(Style::default().bg(input_base)),
        input_rect,
    );

    let case_y = content.y + 3;
    let case_style = if d.focus == 1 {
        Style::default().bg(Color::Blue).fg(Color::White)
    } else {
        fill_style
    };
    let chk = if d.file_case_sensitive { "[x]" } else { "[ ]" };
    let case_line = Line::from(vec![
        Span::raw(chk),
        Span::raw(" File name case sensitive (same as Find)"),
    ]);
    f.render_widget(
        Paragraph::new(case_line).style(case_style),
        Rect {
            x: content.x,
            y: case_y,
            width: content.width,
            height: 1,
        },
    );

    let (apply_rect, cancel_rect) = dialog_layout::single_input_button_rects(content);
    let apply_rect = Rect {
        x: apply_rect.x,
        y: case_y + 2,
        width: apply_rect.width,
        height: apply_rect.height,
    };
    let cancel_rect = Rect {
        x: cancel_rect.x,
        y: case_y + 2,
        width: cancel_rect.width,
        height: cancel_rect.height,
    };

    let apply_style = if d.focus == 2 {
        Style::default().bg(DIALOG_FOCUS).fg(Color::Black)
    } else {
        fill_style
    };
    let cancel_style = if d.focus == 3 {
        Style::default().bg(DIALOG_FOCUS).fg(Color::Black)
    } else {
        fill_style
    };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("A", DIALOG_ACCENT),
            Span::raw("pply"),
        ]))
        .style(apply_style)
        .alignment(Alignment::Center),
        apply_rect,
    );
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("C", DIALOG_ACCENT),
            Span::raw("ancel"),
        ]))
        .style(cancel_style)
        .alignment(Alignment::Center),
        cancel_rect,
    );

    if d.focus == 0 {
        if let Some((cx, cy)) = pattern_input_cursor(f.area(), &d.pattern_input) {
            f.set_cursor_position((cx, cy));
        }
    }
}

/// Hit-test Apply/Cancel (y offset matches draw).
pub fn button_rects(area: Rect) -> Option<(Rect, Rect)> {
    let rect = dialog_layout::centered_dialog_rect(area, 64, 12);
    let inner = rect.inner(Margin {
        horizontal: 1,
        vertical: 1,
    });
    let content = dialog_layout::dialog_content_rect(inner, DEFAULT_PAD_H);
    let case_y = content.y + 3;
    let (apply_rect, cancel_rect) = dialog_layout::single_input_button_rects(content);
    Some((
        Rect {
            x: apply_rect.x,
            y: case_y + 2,
            width: apply_rect.width,
            height: apply_rect.height,
        },
        Rect {
            x: cancel_rect.x,
            y: case_y + 2,
            width: cancel_rect.width,
            height: cancel_rect.height,
        },
    ))
}

/// Column for text cursor when focus is on pattern field.
pub fn pattern_input_cursor(
    area: Rect,
    input: &TextInputState,
) -> Option<(u16, u16)> {
    let rect = dialog_layout::centered_dialog_rect(area, 64, 12);
    let inner = rect.inner(Margin {
        horizontal: 1,
        vertical: 1,
    });
    let content = dialog_layout::dialog_content_rect(inner, DEFAULT_PAD_H);
    let input_rect = Rect {
        x: content.x,
        y: content.y + 1,
        width: content.width,
        height: 1,
    };
    let col = text_input::input_cursor_x(input_rect, input);
    Some((col, input_rect.y))
}
