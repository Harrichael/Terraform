use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};

use crate::app::{state::{AppMode, AppState}, tree::NodeKind};

/// Process one terminal event and mutate `state` accordingly.
/// Returns `true` if the application should quit.
pub fn handle_event(state: &mut AppState) -> anyhow::Result<bool> {
    if event::poll(std::time::Duration::from_millis(50))? {
        if let Event::Key(key) = event::read()? {
            match state.mode {
                AppMode::Filter => handle_filter_key(state, key),
                AppMode::Help => handle_help_key(state, key),
                AppMode::Normal => handle_normal_key(state, key),
            }
        }
    }
    Ok(state.should_quit)
}

fn handle_normal_key(state: &mut AppState, key: KeyEvent) {
    match key.code {
        // Quit
        KeyCode::Char('q') => state.should_quit = true,
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.should_quit = true
        }

        // Navigation
        KeyCode::Up | KeyCode::Char('k') => state.move_cursor_up(1),
        KeyCode::Down | KeyCode::Char('j') => state.move_cursor_down(1),
        KeyCode::PageUp => state.move_cursor_up(state.pane_height.max(1)),
        KeyCode::PageDown => state.move_cursor_down(state.pane_height.max(1)),
        KeyCode::Home | KeyCode::Char('g') => {
            state.cursor = 0;
            state.scroll_offset = 0;
        }
        KeyCode::End | KeyCode::Char('G') => {
            if !state.visible_ids.is_empty() {
                state.cursor = state.visible_ids.len() - 1;
            }
        }

        // Collapse / Expand (fold toggle for the current node)
        KeyCode::Char(' ') => state.toggle_fold(),

        // Enter: fold/unfold, or jump to SymRef target
        KeyCode::Enter => {
            if let Some(&id) = state.visible_ids.get(state.cursor) {
                if let Some(node) = state.tree.get(id) {
                    if node.kind == NodeKind::SymRef {
                        if !state.jump_to_sym_ref_target() {
                            state.status = "Definition is not currently visible.".to_string();
                        }
                        return;
                    }
                }
            }
            state.toggle_fold();
        }

        // Granularity: l / Right = expand (show finer detail on this node only)
        //              h / Left  = shrink (show coarser detail on this node only)
        KeyCode::Right | KeyCode::Char('l') => state.expand_cursor_granularity(),
        KeyCode::Left | KeyCode::Char('h') => state.shrink_cursor_granularity(),

        // Collapse/Expand all
        KeyCode::Char('[') => state.collapse_all(),
        KeyCode::Char(']') => state.expand_all(),

        // Filter
        KeyCode::Char('/') => state.enter_filter(),
        KeyCode::Esc => {
            if !state.filter.is_empty() {
                state.filter.clear();
                state.refresh_visible();
                state.status = String::from("Filter cleared.");
            }
        }

        // Help
        KeyCode::Char('?') | KeyCode::F(1) => state.toggle_help(),

        _ => {}
    }
}

fn handle_filter_key(state: &mut AppState, key: KeyEvent) {
    match key.code {
        KeyCode::Enter => state.confirm_filter(),
        KeyCode::Esc => state.cancel_filter(),
        KeyCode::Backspace => state.pop_filter_char(),
        KeyCode::Char(c) => state.push_filter_char(c),
        _ => {}
    }
}

fn handle_help_key(state: &mut AppState, key: KeyEvent) {
    match key.code {
        KeyCode::Char('?') | KeyCode::F(1) | KeyCode::Esc | KeyCode::Enter => {
            state.toggle_help()
        }
        KeyCode::Char('q') => state.should_quit = true,
        _ => {}
    }
}
