use ratatui::{
    Frame,
    layout::{Constraint, Layout, Position},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Text},
    widgets::{Block, Paragraph, Wrap},
};

use crate::tui::app::*;

impl App {
    pub fn render(&self, frame: &mut Frame) {
        let title = Line::from(" svarog ".bold());

        let layout = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min((self.input_lines_count + 2) as u16),
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
            InputMode::Editing => frame.set_cursor_position(Position::new(
                // Draw the cursor at the current position in the input field.
                // This position can be controlled via the left and right arrow key
                input_area.x + (self.char_index as u16 % (self.terminal_width - 2)) + 1,
                // Move one line down, from the border to the input line
                input_area.y + self.input_lines_count as u16,
            )),
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
