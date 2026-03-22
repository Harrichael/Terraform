mod app;
mod parser;
mod ui;

use std::io;
use std::path::PathBuf;

use anyhow::Result;
use clap::Parser as ClapParser;
use crossterm::{
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};

use app::state::AppState;
use ui::{events::handle_event, render};

/// Terraform — terminal-based hierarchical code viewer and editor.
#[derive(ClapParser, Debug)]
#[command(author, version, about)]
struct Cli {
    /// Source file to open on startup.
    file: Option<PathBuf>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Set up terminal.
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run(&mut terminal, cli.file);

    // Always restore terminal before propagating errors.
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

fn run(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    initial_file: Option<PathBuf>,
) -> Result<()> {
    let mut state = AppState::new();

    if let Some(path) = initial_file {
        if let Err(e) = state.load_file(path) {
            state.status = format!("Error loading file: {e}");
        }
    }

    loop {
        terminal.draw(|frame| render(frame, &mut state))?;

        if handle_event(&mut state)? {
            break;
        }
    }

    Ok(())
}
