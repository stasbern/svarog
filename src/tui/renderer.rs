use ratatui::{
    Frame,
    layout::{Constraint, Layout, Position, Rect},
    style::{Style, Stylize},
    text::Line,
    widgets::{Block, BorderType, Paragraph, Wrap},
};

use crate::tui::app::*;

impl App {
    pub fn render(&mut self, frame: &mut Frame) {
        let title = Line::from(" svarog ".bold());

        let inner_width = frame.area().width.saturating_sub(2) as usize;
        let input_lines = if inner_width > 0 {
            let char_count = self.input.chars().count();
            (char_count / inner_width) + 1
        } else {
            1
        };

        let layout = Layout::vertical([
            Constraint::Min(1),                         // messages
            Constraint::Length(input_lines as u16 + 2), // input box
            Constraint::Length(1),                      // status + instructions
        ]);
        let [messages_area, input_area, status_area] = frame.area().layout(&layout);
        let base_style = Style::default().bg(self.theme.bg);
        frame.render_widget(Block::default().style(base_style), frame.area());

        // ── Messages with per-sender blocks ──────────────────────────
        let outer_block = Block::bordered().title("Messages");
        let messages_inner = outer_block.inner(messages_area);
        frame.render_widget(outer_block, messages_area);

        self.last_viewport = (messages_inner.width, messages_inner.height);

        // Width available inside each message's own block borders
        let msg_inner_width = messages_inner.width.saturating_sub(2) as usize;

        // Pre-calculate height of each message block
        let msg_heights: Vec<u16> = self
            .messages
            .iter()
            .map(|m| App::wrapped_line_count(&m.message, msg_inner_width) + 2) // +2 for block borders
            .collect();
        let total_content_height: u16 = msg_heights.iter().sum();

        // Auto-scroll to bottom if following output
        if self.follow_output {
            self.scroll_offset = total_content_height.saturating_sub(messages_inner.height);
        }
        // Clamp scroll
        let max_scroll = total_content_height.saturating_sub(messages_inner.height);
        self.scroll_offset = self.scroll_offset.min(max_scroll);

        // Render visible messages
        let mut cumulative_y: u16 = 0;
        for (i, msg) in self.messages.iter().enumerate() {
            let msg_h = msg_heights[i];
            let msg_top = cumulative_y;
            cumulative_y += msg_h;

            // Skip if entirely above viewport
            if cumulative_y <= self.scroll_offset {
                continue;
            }
            // Stop if entirely below viewport
            if msg_top >= self.scroll_offset + messages_inner.height {
                break;
            }

            // How many lines of this message are hidden above the viewport
            let hidden_top = self.scroll_offset.saturating_sub(msg_top);
            // Where in the viewport this message starts
            let viewport_y = msg_top.saturating_sub(self.scroll_offset);
            // How tall the visible portion is
            let visible_h =
                (msg_h - hidden_top).min(messages_inner.height.saturating_sub(viewport_y));

            if visible_h == 0 {
                continue;
            }

            let render_rect = Rect::new(
                messages_inner.x,
                messages_inner.y + viewport_y,
                messages_inner.width,
                visible_h,
            );

            let (border_color, border_type, sender_label) = match msg.sender.as_str() {
                "user" => (
                    self.theme.user_border,
                    BorderType::Rounded,
                    " you ".to_string(),
                ),
                "svarog" => (
                    self.theme.assistant_border,
                    BorderType::Rounded,
                    " svarog ".to_string(),
                ),
                s if s.starts_with("kb:") => {
                    (self.theme.kb_border, BorderType::Plain, format!(" {} ", s))
                }
                _ => (
                    self.theme.system_border,
                    BorderType::Double,
                    " system ".to_string(),
                ),
            };

            let block = Block::bordered()
                .title(Line::from(sender_label).style(Style::default().fg(border_color)))
                .border_type(border_type)
                .border_style(Style::default().fg(border_color));

            let paragraph = Paragraph::new(msg.message.as_str())
                .wrap(Wrap { trim: true })
                .block(block)
                .scroll((hidden_top, 0)); // scroll past the hidden top lines

            frame.render_widget(paragraph, render_rect);
        }

        // ── Input box ────────────────────────────────────────────────
        let input_style = match self.input_mode {
            InputMode::Normal => Style::default(),
            InputMode::Editing if self.ingesting => Style::default().fg(self.theme.input_disabled),
            InputMode::Editing => Style::default().fg(self.theme.input_active),
        };
        let input = Paragraph::new(self.input.as_str())
            .wrap(Wrap { trim: true })
            .style(input_style)
            .block(Block::bordered().title(title.centered()));
        frame.render_widget(input, input_area);

        if let InputMode::Editing = self.input_mode {
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

        // ── Status bar + instructions ────────────────────────────────
        let mut status_spans: Vec<ratatui::text::Span> = vec![];

        if self.ingesting {
            status_spans.push(" ⟳ ".into());
            status_spans.push(ratatui::text::Span::styled(
                &self.status_line,
                Style::default().fg(self.theme.status_busy),
            ));
            status_spans.push("  ".into());
        } else if !self.status_line.is_empty() {
            status_spans.push(ratatui::text::Span::styled(
                &self.status_line,
                Style::default().fg(self.theme.status_ok),
            ));
            status_spans.push("  ".into());
        }

        match self.input_mode {
            InputMode::Normal => {
                status_spans.extend_from_slice(&[
                    " Edit ".into(),
                    ratatui::text::Span::styled(
                        "<E>",
                        Style::default().fg(self.theme.accent).bold(),
                    ),
                    " Ingest ".into(),
                    ratatui::text::Span::styled(
                        "<I>",
                        Style::default().fg(self.theme.accent).bold(),
                    ),
                    " Scroll ".into(),
                    ratatui::text::Span::styled(
                        "<↑/↓>",
                        Style::default().fg(self.theme.accent).bold(),
                    ),
                    " Quit ".into(),
                    ratatui::text::Span::styled(
                        "<Q>",
                        Style::default().fg(self.theme.accent).bold(),
                    ),
                ]);
            }
            InputMode::Editing => {
                status_spans.extend_from_slice(&[
                    " Normal ".into(),
                    ratatui::text::Span::styled(
                        "<Esc>",
                        Style::default().fg(self.theme.accent).bold(),
                    ),
                ]);
            }
        }

        frame.render_widget(Paragraph::new(Line::from(status_spans)), status_area);
    }
}
