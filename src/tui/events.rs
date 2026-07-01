use color_eyre::{Result, eyre::WrapErr};
use ratatui::crossterm::event::{self, Event, EventStream, KeyCode, KeyEvent, KeyEventKind};
use std::time::Duration;
use tokio::time::sleep;
use tokio_stream::StreamExt;

use crate::tui::app::*;

impl App {
    pub async fn handle_events(&mut self, event_stream: &mut EventStream) -> Result<()> {
        tokio::select! {
            Some(Ok(event)) = event_stream.next() => {
                match event {
                    Event::Key(key) => {
                        if key.kind == KeyEventKind::Press {
                            match self.input_mode {
                                InputMode::Normal => self
                                    .handle_normal_mode(key)
                                    .await
                                    .wrap_err_with(|| format!("handling key event failed:\n{key:#?}")),

                                InputMode::Editing => self
                                    .handle_editing_mode(key)
                                    .await
                                    .wrap_err_with(|| format!("handling key event failed:\n{key:#?}")),
                            }
                        } else {
                            Ok(())
                        }
                    }
                    _ => Ok(())
                }
            }
            _ = sleep(Duration::from_millis(250)) => {
                Ok(())
            }
        }
    }

    pub async fn handle_normal_mode(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Char('e') => {
                self.input_mode = InputMode::Editing;
            }
            KeyCode::Char('q') => self.exit(),
            _ => {}
        }
        Ok(())
    }

    pub async fn handle_editing_mode(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Enter => self.submit_message().await?,
            KeyCode::Backspace => self.delete_char(),
            KeyCode::Char(to_insert) => self.enter_char(to_insert),
            KeyCode::Left => self.move_cursor_left(),
            KeyCode::Right => self.move_cursor_right(),
            KeyCode::Esc => self.input_mode = InputMode::Normal,
            KeyCode::End => self.move_cursor_to_end(),
            KeyCode::Home => self.reset_cursor(),
            _ => {}
        }
        Ok(())
    }
}
