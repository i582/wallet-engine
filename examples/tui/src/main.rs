mod app;
mod http_host;
mod storage;
mod ui;

use std::io::{self, Stdout};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use wallet_engine::WalletLifecycle;

use crate::app::App;
use crate::http_host::ReqwestHttpHost;
use crate::storage::DiskStore;

#[tokio::main]
async fn main() -> Result<()> {
    let store = Arc::new(DiskStore::open_default()?);
    let api_key = std::env::var("TONCENTER_API_KEY").ok();
    let http_host = Arc::new(ReqwestHttpHost::new(api_key.clone())?);
    let lifecycle = WalletLifecycle::new(store.clone());
    let mut app = App::new(store, http_host, lifecycle, api_key.is_some()).await;
    let mut terminal = TerminalSession::new()?;

    while !app.should_quit() {
        terminal.draw(|frame| ui::render(frame, &app))?;

        if event::poll(Duration::from_millis(100))?
            && let Event::Key(key) = event::read()?
        {
            app.handle_key(key).await;
        }
    }

    app.shutdown().await;
    Ok(())
}

struct TerminalSession {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl TerminalSession {
    fn new() -> Result<Self> {
        enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen)?;
        let terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
        Ok(Self { terminal })
    }

    fn draw(&mut self, draw: impl FnOnce(&mut ratatui::Frame<'_>)) -> io::Result<()> {
        self.terminal.draw(draw).map(|_| ())
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(self.terminal.backend_mut(), LeaveAlternateScreen);
        let _ = self.terminal.show_cursor();
    }
}
