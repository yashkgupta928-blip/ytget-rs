//! ytget-rs: a mouse-friendly terminal UI for yt-dlp.
//!
//! This file owns the terminal (raw mode, alternate screen, mouse capture) and the event
//! loop. State and input handling live in `app.rs`, drawing in `ui.rs`, and everything
//! that touches yt-dlp in `ytdlp.rs` and `download.rs`.

mod app;
mod config;
mod download;
mod text;
mod types;
mod ui;
mod ytdlp;

use std::io::{self, Stdout};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use crossterm::cursor::Show;
use crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture, Event, KeyEventKind,
};
use crossterm::execute;
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use app::App;

const TICK: Duration = Duration::from_millis(100);

/// Undo everything `TerminalGuard::enter` did. Safe to call more than once, and from the
/// panic hook.
fn restore_terminal() {
    let _ = disable_raw_mode();
    let mut out = io::stdout();
    let _ = execute!(out, DisableBracketedPaste, DisableMouseCapture, LeaveAlternateScreen, Show);
}

/// Restores the terminal when dropped, so every way out of `main` (normal return, `?`,
/// unwinding after a panic) leaves the shell usable.
struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> Result<Self> {
        enable_raw_mode().context("could not enable raw mode (is this a terminal?)")?;
        // From here on the guard exists, so a failure below still restores what was set.
        let guard = TerminalGuard;
        execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture).context("could not set up the terminal")?;
        // Pasting still works without this (as typed keys); some older consoles lack it.
        let _ = execute!(io::stdout(), EnableBracketedPaste);
        Ok(guard)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore_terminal();
    }
}

fn install_panic_hook() {
    let original = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        original(info);
    }));
}

fn main() -> Result<()> {
    install_panic_hook();
    let _guard = TerminalGuard::enter()?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    terminal.clear()?;

    let mut app = App::new();
    let result = run(&mut terminal, &mut app);
    app.shutdown();
    result
}

fn run(terminal: &mut Terminal<CrosstermBackend<Stdout>>, app: &mut App) -> Result<()> {
    let mut last_tick = Instant::now();
    while !app.should_quit {
        terminal.draw(|f| ui::draw(f, app))?;

        let timeout = TICK.saturating_sub(last_tick.elapsed());
        if event::poll(timeout)? {
            handle(app, event::read()?);
            // Take whatever else is already waiting (mouse moves arrive in bursts) before
            // drawing again, so the screen never lags behind the pointer.
            let mut budget = 256;
            while budget > 0 && event::poll(Duration::ZERO)? {
                handle(app, event::read()?);
                budget -= 1;
            }
        }
        if last_tick.elapsed() >= TICK {
            app.on_tick();
            last_tick = Instant::now();
        }
    }
    Ok(())
}

fn handle(app: &mut App, ev: Event) {
    match ev {
        // Windows also reports key releases; only act on presses and repeats.
        Event::Key(key) if key.kind != KeyEventKind::Release => app.on_key(key),
        Event::Mouse(m) => app.on_mouse(m),
        Event::Paste(s) => app.on_paste(s),
        Event::Resize(..) => {} // the hit map is rebuilt on the next draw
        _ => {}
    }
}
