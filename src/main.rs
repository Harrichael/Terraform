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
    /// Source file or directory to open. Defaults to the current directory.
    path: Option<PathBuf>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Set up terminal.
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Default to the current directory when no path is given.
    let path = cli.path.unwrap_or_else(|| PathBuf::from("."));
    let result = run(&mut terminal, path);

    // Always restore terminal before propagating errors.
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

fn run(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, path: PathBuf) -> Result<()> {
    let mut state = AppState::new();

    if path.is_dir() {
        if let Err(e) = state.load_directory(path) {
            state.status = format!("Error loading directory: {e}");
        }
    } else {
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
