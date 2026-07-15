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
        let inner_width = frame.area().width.saturating_sub(2) as usize;
        let input_lines = if inner_width > 0 {
            let (display, _, _) = Self::word_wrap_input(&self.input, inner_width, 0);
            display.chars().filter(|c| *c == '\n').count() + 1
        } else {
            1
        };

        let layout = Layout::vertical([
            Constraint::Min(1),
            Constraint::Length(input_lines as u16 + 2),
            Constraint::Length(1),
        ]);
        let [content_area, input_area, status_area] = frame.area().layout(&layout);

        frame.render_widget(
            Block::default().style(Style::default().bg(self.theme.bg)),
            frame.area(),
        );

        match self.console_mode {
            ConsoleMode::Normal | ConsoleMode::Editing => {
                Self::render_entries(
                    frame,
                    &self.messages,
                    &mut self.chat_scroll_offset,
                    self.follow_output,
                    content_area,
                    "Messages",
                    &self.theme,
                    &mut self.last_viewport,
                );
            }
            ConsoleMode::Logs => {
                Self::render_entries(
                    frame,
                    &self.logs,
                    &mut self.log_scroll_offset,
                    self.follow_logs,
                    content_area,
                    "Logs",
                    &self.theme,
                    &mut self.last_viewport,
                );
            }
        }

        self.render_input(frame, input_area, inner_width);
        self.render_status_bar(frame, status_area);
    }

    fn render_entries(
        frame: &mut Frame,
        entries: &[ChatEntry],
        scroll_offset: &mut u16,
        follow: bool,
        area: Rect,
        title: &str,
        theme: &crate::tui::theme::Theme,
        last_viewport: &mut (u16, u16),
    ) {
        let outer_block = Block::bordered().title(title);
        let inner = outer_block.inner(area);
        frame.render_widget(outer_block, area);

        *last_viewport = (inner.width, inner.height);
        let msg_inner_width = inner.width.saturating_sub(2) as usize;

        let msg_heights: Vec<u16> = entries
            .iter()
            .map(|m| App::wrapped_line_count(&m.message, msg_inner_width) + 2)
            .collect();
        let total_h: u16 = msg_heights.iter().sum();

        if follow {
            *scroll_offset = total_h.saturating_sub(inner.height);
        }
        let max_scroll = total_h.saturating_sub(inner.height);
        *scroll_offset = (*scroll_offset).min(max_scroll);

        let mut cumulative_y: u16 = 0;
        for (i, msg) in entries.iter().enumerate() {
            let msg_h = msg_heights[i];
            let msg_top = cumulative_y;
            cumulative_y += msg_h;

            if cumulative_y <= *scroll_offset {
                continue;
            }
            if msg_top >= *scroll_offset + inner.height {
                break;
            }

            let hidden_top = scroll_offset.saturating_sub(msg_top);
            let viewport_y = msg_top.saturating_sub(*scroll_offset);
            let visible_h = (msg_h - hidden_top).min(inner.height.saturating_sub(viewport_y));
            if visible_h == 0 {
                continue;
            }

            let rect = Rect::new(inner.x, inner.y + viewport_y, inner.width, visible_h);

            let (border_color, border_type, label) = match msg.sender.as_str() {
                "user" => (theme.user_border, BorderType::Rounded, " you ".into()),
                "svarog" => (
                    theme.assistant_border,
                    BorderType::Rounded,
                    " svarog ".into(),
                ),
                s if s.starts_with("kb:") => (theme.kb_border, BorderType::Plain, format!(" {s} ")),
                "error" => (theme.status_busy, BorderType::Double, " error ".into()),
                "status" => (theme.status_ok, BorderType::Plain, " status ".into()),
                _ => (theme.system_border, BorderType::Double, " system ".into()),
            };

            let block = Block::bordered()
                .title(Line::from(label).style(Style::default().fg(border_color)))
                .border_type(border_type)
                .border_style(Style::default().fg(border_color));

            let paragraph = Paragraph::new(msg.message.as_str())
                .wrap(Wrap { trim: true })
                .block(block)
                .scroll((hidden_top, 0));

            frame.render_widget(paragraph, rect);
        }
    }

    fn render_input(&self, frame: &mut Frame, area: Rect, inner_width: usize) {
        let title = Line::from(" svarog ".bold());
        let input_style = match self.console_mode {
            ConsoleMode::Normal | ConsoleMode::Logs => Style::default(),
            ConsoleMode::Editing if self.ingesting => {
                Style::default().fg(self.theme.input_disabled)
            }
            ConsoleMode::Editing => Style::default().fg(self.theme.input_active),
        };

        let (display, cursor_line, cursor_col) =
            Self::word_wrap_input(&self.input, inner_width, self.char_index);

        let input = Paragraph::new(display)
            .style(input_style)
            .block(Block::bordered().title(title.centered()));
        frame.render_widget(input, area);

        if matches!(self.console_mode, ConsoleMode::Editing) {
            frame.set_cursor_position(Position::new(
                area.x + cursor_col as u16 + 1,
                area.y + cursor_line as u16 + 1,
            ));
        }
    }

    fn word_wrap_input(text: &str, width: usize, char_index: usize) -> (String, usize, usize) {
        if width == 0 || text.is_empty() {
            return (text.to_string(), 0, char_index);
        }

        let chars: Vec<char> = text.chars().collect();
        let mut lines: Vec<String> = Vec::new();
        let mut line_starts: Vec<usize> = vec![0];
        let mut pos = 0;

        while pos < chars.len() {
            let remaining = chars.len() - pos;

            if remaining <= width {
                lines.push(chars[pos..].iter().collect());
                break;
            }

            // Find last space within line width to break at
            let slice = &chars[pos..pos + width];
            let break_at = if let Some(last_space) = slice.iter().rposition(|c| *c == ' ') {
                last_space + 1 // break after space
            } else {
                width // no space — force break
            };

            lines.push(chars[pos..pos + break_at].iter().collect());
            pos += break_at;
            line_starts.push(pos);
        }

        // Find cursor line and column
        let mut cursor_line = lines.len().saturating_sub(1);
        let mut cursor_col = char_index.saturating_sub(*line_starts.last().unwrap_or(&0));

        for (i, &start) in line_starts.iter().enumerate() {
            let end = if i + 1 < line_starts.len() {
                line_starts[i + 1]
            } else {
                chars.len() + 1
            };
            if char_index < end {
                cursor_line = i;
                cursor_col = char_index - start;
                break;
            }
        }

        (lines.join("\n"), cursor_line, cursor_col)
    }

    fn render_status_bar(&self, frame: &mut Frame, area: Rect) {
        use ratatui::text::Span;
        let mut spans: Vec<Span> = vec![];

        if self.ingesting {
            spans.push(" ⟳ ".into());
            spans.push(Span::styled(
                &self.status_line,
                Style::default().fg(self.theme.status_busy),
            ));
            spans.push("  ".into());
        } else if !self.status_line.is_empty() {
            spans.push(Span::styled(
                &self.status_line,
                Style::default().fg(self.theme.status_ok),
            ));
            spans.push("  ".into());
        }

        let accent =
            |s: &'static str| Span::styled(s, Style::default().fg(self.theme.accent).bold());

        match self.console_mode {
            ConsoleMode::Normal => {
                spans.extend([
                    " Edit ".into(),
                    accent("<E>"),
                    " Ingest ".into(),
                    accent("<I>"),
                    " Logs ".into(),
                    accent("<L>"),
                    " Scroll ".into(),
                    accent("<↑/↓>"),
                    " Quit ".into(),
                    accent("<Q>"),
                ]);
            }
            ConsoleMode::Editing => {
                spans.extend([" Normal ".into(), accent("<Esc>")]);
            }
            ConsoleMode::Logs => {
                spans.extend([
                    " Back ".into(),
                    accent("<Esc>"),
                    " Scroll ".into(),
                    accent("<↑/↓>"),
                    " Quit ".into(),
                    accent("<Q>"),
                ]);
            }
        }

        frame.render_widget(Paragraph::new(Line::from(spans)), area);
    }
}
