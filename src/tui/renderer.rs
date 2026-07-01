use ratatui::{
    Frame,
    layout::{Constraint, Layout, Position},
    style::{Color, Style, Stylize},
    text::{Line, Text},
    widgets::{Block, Paragraph, Wrap},
};

use crate::tui::app::*;

impl App {
    pub fn render(&self, frame: &mut Frame) {
        let title = Line::from(" svarog ".bold());

        let inner_width = frame.area().width.saturating_sub(2) as usize;
        let input_lines = if inner_width > 0 {
            let char_count = self.input.chars().count();
            if char_count == 0 {
                1
            } else {
                (char_count / inner_width) + 1
            }
        } else {
            1
        };
        let layout = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(input_lines as u16 + 2),
            Constraint::Min(1),
        ]);
        let [instructions_area, input_area, messages_area] = frame.area().layout(&layout);

        let (instructiuons, style) = match self.input_mode {
            InputMode::Normal => (
                Line::from(vec![
                    " Editing ".into(),
                    "<E>".blue().bold(),
                    " Quit ".into(),
                    "<Q> ".blue().bold(),
                ]),
                Style::default(),
            ),
            InputMode::Editing => (
                Line::from(vec![" Normal ".into(), "<Esc>".blue().bold()]),
                Style::default(),
            ),
        };

        let instructions_text = Text::from(instructiuons).patch_style(style);
        frame.render_widget(Paragraph::new(instructions_text), instructions_area);

        let input = Paragraph::new(self.input.as_str())
            .wrap(Wrap { trim: true })
            .style(match self.input_mode {
                InputMode::Normal => Style::default(),
                InputMode::Editing => Style::default().fg(Color::Yellow),
            })
            .block(Block::bordered().title(title.centered()));
        frame.render_widget(input, input_area);

        match self.input_mode {
            // Hide the cursor. `Frame` does this by default, so we don't need to do anything here
            InputMode::Normal => {}

            // Make the cursor visible and ask ratatui to put it at the specified coordinates after
            // rendering
            #[expect(clippy::cast_possible_truncation)]
            InputMode::Editing => {
                let cursor_line = if inner_width > 0 {
                    self.char_index / inner_width
                } else {
                    0
                };
                let cursor_col = if inner_width > 0 {
                    self.char_index % inner_width
                } else {
                    0
                };
                frame.set_cursor_position(Position::new(
                    input_area.x + cursor_col as u16 + 1,
                    input_area.y + cursor_line as u16 + 1,
                ));
            }
        }

        let combined_messages: String = self
            .messages
            .iter()
            .enumerate()
            .map(|(i, m)| format!("{i}: {m}\n"))
            .collect();

        let messages_widget = Paragraph::new(combined_messages)
            .block(Block::bordered().title("Messages"))
            .wrap(Wrap { trim: true })
            .scroll((self.scroll_offset, 0));

        frame.render_widget(messages_widget, messages_area);
    }
}
