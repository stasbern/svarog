use color_eyre::{Result, eyre::WrapErr};
use ratatui::{DefaultTerminal, crossterm::event::EventStream};
use std::sync::Arc;
use tokio::sync::mpsc;

use crate::events::*;
use crate::rig::knowledge::KnowledgeBase;

#[derive(Debug, Default)]
pub enum InputMode {
    #[default]
    Normal,
    Editing,
}

pub struct App {
    pub input_mode: InputMode,
    pub input: String,
    pub messages: Vec<String>,
    pub scroll_offset: u16,
    pub char_index: usize,

    exit: bool,
    tx: mpsc::Sender<Request>,
    rx: mpsc::Receiver<Response>,
    pub(crate) knowledge: Arc<KnowledgeBase>,
    pub(crate) status_tx: mpsc::Sender<String>,
    status_rx: mpsc::Receiver<String>,
}

impl App {
    pub fn new(
        tx: mpsc::Sender<Request>,
        rx: mpsc::Receiver<Response>,
        knowledge: Arc<KnowledgeBase>,
    ) -> Self {
        let (status_tx, status_rx) = mpsc::channel::<String>(32);
        Self {
            input: String::new(),
            char_index: 0,
            input_mode: InputMode::Normal,
            messages: Vec::new(),
            scroll_offset: 0,
            exit: false,
            tx,
            rx,
            knowledge,
            status_tx,
            status_rx,
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
        self.messages.push(self.input.clone());
        self.tx.send(Request::Prompt(self.input.clone())).await?;
        self.input.clear();
        self.reset_cursor();
        Ok(())
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
            while let Ok(Response::CompleteResponse(msg)) = self.rx.try_recv() {
                self.messages.push(msg);
            }
            while let Ok(status) = self.status_rx.try_recv() {
                self.messages.push("[system]: ".to_string() + &status);
            }
        }
        Ok(())
    }

    pub fn exit(&mut self) {
        self.exit = true;
    }
}
