use color_eyre::{Result, eyre::WrapErr};
use ratatui::{DefaultTerminal, crossterm::event::EventStream};
use tokio::sync::mpsc;

use crate::events::*;

#[derive(Debug, Default)]
pub enum ConsoleMode {
    #[default]
    Normal,   // chat view, navigation keys
    Editing,  // chat view, typing in input
    Logs,     // logs view, navigation keys
}

pub struct ChatEntry {
    pub sender: String,
    pub message: String,
}

impl ChatEntry {
    pub fn new(sender: String, message: String) -> Self {
        Self { sender, message }
    }
}

pub struct App {
    pub theme: crate::tui::theme::Theme,
    
    pub console_mode: ConsoleMode,

    pub input: String,
    pub messages: Vec<ChatEntry>,
    pub chat_scroll_offset: u16,
    pub char_index: usize,
    pub follow_output: bool,
    pub status_line: String,

    pub log_scroll_offset: u16,
    pub logs: Vec<ChatEntry>,    
    pub follow_logs: bool,

    pub last_viewport: (u16, u16),
    pub ingesting: bool,
    exit: bool,
    pub tx: mpsc::Sender<Request>,
    pub rx: mpsc::Receiver<Response>,
}

impl App {
    pub fn new(
        tx: mpsc::Sender<Request>,
        rx: mpsc::Receiver<Response>,
    ) -> Self {
        Self {
            theme: crate::tui::theme::Theme::default(),
            input: String::new(),
            char_index: 0,
            console_mode: ConsoleMode::Normal,
            messages: Vec::new(),
            chat_scroll_offset: 0,
            log_scroll_offset: 0,
            logs: Vec::new(),
            ingesting: false,
            follow_output: true,
            follow_logs: true,
            status_line: String::new(),
            last_viewport: (0, 0),
            exit: false,
            tx,
            rx,
        }
    }

    fn clamp_cursor(&self, new_cursor_pos: usize) -> usize {
        new_cursor_pos.clamp(0, self.input.chars().count())
    }

    pub fn reset_cursor(&mut self) {
        self.char_index = 0;
    }

    pub fn move_cursor_left(&mut self) {
        if let Some(cursor_moved_left) = self.char_index.checked_sub(1) {
            self.char_index = self.clamp_cursor(cursor_moved_left);
        }
    }

    pub fn move_cursor_right(&mut self) {
        if let Some(cursor_moved_right) = self.char_index.checked_add(1) {
            self.char_index = self.clamp_cursor(cursor_moved_right);
        }
    }

    pub fn move_cursor_to_end(&mut self) {
        self.char_index = self.clamp_cursor(self.input.chars().count());
    }

    pub fn scroll_up(&mut self, lines: u16) {
        let offset = self.active_scroll_offset_mut();
        *offset = offset.saturating_sub(lines);
        match self.console_mode {
            ConsoleMode::Normal | ConsoleMode::Editing => self.follow_output = false,
            ConsoleMode::Logs => self.follow_logs = false,
        }
    }

    pub fn scroll_down(&mut self, lines: u16) {
        let (viewport_h, viewport_w) = self.last_viewport;
        let entries = match self.console_mode {
            ConsoleMode::Normal | ConsoleMode::Editing => &self.messages,
            ConsoleMode::Logs => &self.logs,
        };
        let total_h = Self::total_content_height(entries, viewport_w);
        let max_scroll = total_h.saturating_sub(viewport_h);

        let offset = self.active_scroll_offset_mut();
        *offset = offset.saturating_add(lines).min(max_scroll);
        let at_bottom = *offset >= max_scroll;

        match self.console_mode {
            ConsoleMode::Normal | ConsoleMode::Editing => {
                if at_bottom { self.follow_output = true; }
            }
            ConsoleMode::Logs => {
                if at_bottom { self.follow_logs = true; }
            }
        }
    }

    fn active_scroll_offset_mut(&mut self) -> &mut u16 {
        match self.console_mode {
            ConsoleMode::Normal | ConsoleMode::Editing => &mut self.chat_scroll_offset,
            ConsoleMode::Logs => &mut self.log_scroll_offset,
        }
    }

    pub fn wrapped_line_count(text: &str, width: usize) -> u16 {
        if width == 0 { return 1; }
        text.split('\n')
            .map(|line| {
                let len = line.chars().count();
                if len == 0 { 1 } else { ((len + width - 1) / width) as u16 }
            })
            .sum::<u16>()
            .max(1)
    }

    pub fn enter_char(&mut self, new_char: char) {
        let index = self.byte_index();
        self.input.insert(index, new_char);
        self.move_cursor_right();
    }

    pub fn delete_char(&mut self) {
        let is_not_cursor_leftmost = self.char_index != 0;
        if is_not_cursor_leftmost {
            // Method "remove" is not used on the saved text for deleting the selected char.
            // Reason: Using remove on String works on bytes instead of the chars.
            // Using remove would require special care because of char boundaries.

            let current_index = self.char_index;
            let from_left_to_current_index = current_index - 1;

            // Getting all characters before the selected character.
            let before_char_to_delete = self.input.chars().take(from_left_to_current_index);
            // Getting all characters after selected character.
            let after_char_to_delete = self.input.chars().skip(current_index);

            // Put all characters together except the selected one.
            // By leaving the selected one out, it is forgotten and therefore deleted.
            self.input = before_char_to_delete.chain(after_char_to_delete).collect();
            self.move_cursor_left();
        }
    }

    /// Returns the byte index based on the character position.
    ///
    /// Since each character in a string can contain multiple bytes, it's necessary to calculate
    /// the byte index based on the index of the character.
    fn byte_index(&self) -> usize {
        self.input
            .char_indices()
            .map(|(i, _)| i)
            .nth(self.char_index)
            .unwrap_or(self.input.len())
    }

    pub async fn submit_message(&mut self) -> Result<()> {
        self.messages
            .push(ChatEntry::new("user".to_string(), self.input.clone()));
        self.tx.send(Request::Prompt(self.input.clone())).await?;
        self.input.clear();
        self.reset_cursor();
        Ok(())
    }

    pub fn total_content_height(messages: &[ChatEntry], viewport_width: u16) -> u16 {
        let msg_inner_width = viewport_width.saturating_sub(2) as usize;
        messages
            .iter()
            .map(|m| App::wrapped_line_count(&m.message, msg_inner_width) + 2)
            .sum()
    }

    pub async fn run(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        while !self.exit {
            terminal.draw(|frame| {
                self.render(frame);
            })?;
            let mut event_stream = EventStream::new();
            self.handle_events(&mut event_stream)
                .await
                .wrap_err("handle events failed")?;
            while let Ok(resp) = self.rx.try_recv() {
                match resp {
                    Response::CompleteResponse(msg) => {
                        self.messages.push(ChatEntry::new("svarog".into(), msg));
                    }
                    Response::ContextFound(contexts) => {
                        for (score, preview) in &contexts {
                            self.logs.push(ChatEntry::new(
                                format!("kb: score {:.2}", score),
                                preview.clone(),
                            ));
                        }
                    }
                    Response::Status(status) => {
                        self.logs.push(ChatEntry::new("status".into(), status.clone()));
                        if status.contains("complete") || status.contains("failed") {
                            self.ingesting = false;
                        }
                        self.status_line = status;
                    }
                    Response::Error(e) => {
                        self.logs.push(ChatEntry::new("error".into(), e.clone()));
                        self.status_line = format!("Error: {e}");
                    }
                }
            }
        }
        Ok(())
    }

    pub fn exit(&mut self) {
        self.exit = true;
    }
}
