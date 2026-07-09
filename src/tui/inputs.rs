use color_eyre::{Result, eyre::WrapErr};
use ratatui::crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyEventKind};
use std::time::Duration;
use tokio::time::sleep;
use tokio_stream::StreamExt;

use crate::tui::app::*;
use crate::events::*;

impl App {
    pub async fn handle_events(&mut self, event_stream: &mut EventStream) -> Result<()> {
        tokio::select! {
            Some(Ok(event)) = event_stream.next() => {
                match event {
                    Event::Key(key) => {
                        if key.kind == KeyEventKind::Press {
                            match self.console_mode {
                                ConsoleMode::Normal => self
                                    .handle_normal_mode(key)
                                    .await
                                    .wrap_err_with(|| format!("handling key event failed:\n{key:#?}")),

                                ConsoleMode::Editing => self
                                    .handle_editing_mode(key)
                                    .await
                                    .wrap_err_with(|| format!("handling key event failed:\n{key:#?}")),

                                ConsoleMode::Logs => self
                                    .handle_logs_mode(key)
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
            KeyCode::Char('e') => self.console_mode = ConsoleMode::Editing,
            KeyCode::Char('l') => self.console_mode = ConsoleMode::Logs,
            KeyCode::Char('i') if !self.ingesting => {
                self.ingesting = true;
                self.status_line = "Ingesting documents...".into();
                self.tx.send(Request::Ingest).await?;
            }
            KeyCode::Up | KeyCode::Char('k') => self.scroll_up(3),
            KeyCode::Down | KeyCode::Char('j') => self.scroll_down(3),
            KeyCode::Char('q') => self.exit(),
            _ => {}
        }
        Ok(())
    }

    pub async fn handle_editing_mode(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Enter if !self.ingesting => self.submit_message().await?,
            KeyCode::Enter => {} // swallow Enter while ingesting
            KeyCode::Backspace => self.delete_char(),
            KeyCode::Char(to_insert) => self.enter_char(to_insert),
            KeyCode::Left => self.move_cursor_left(),
            KeyCode::Right => self.move_cursor_right(),
            KeyCode::Esc => self.console_mode = ConsoleMode::Normal,
            KeyCode::End => self.move_cursor_to_end(),
            KeyCode::Home => self.reset_cursor(),
            _ => {}
        }
        Ok(())
    }

    pub async fn handle_logs_mode(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc => self.console_mode = ConsoleMode::Normal,
            KeyCode::Up | KeyCode::Char('k') => self.scroll_up(3),
            KeyCode::Down | KeyCode::Char('j') => self.scroll_down(3),
            KeyCode::Char('q') => self.exit(),
            _ => {}
        }
        Ok(())
    }
}
