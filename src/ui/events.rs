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
    // When a Navigator is present, route navigation through the graph view.
    let has_navigator = state.navigator.is_some();

    match key.code {
        // Quit
        KeyCode::Char('q') => state.should_quit = true,
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.should_quit = true
        }

        // Navigation — graph view when navigator available, legacy otherwise
        KeyCode::Up | KeyCode::Char('k') => {
            if has_navigator {
                state.move_graph_cursor_up(1);
            } else {
                state.move_cursor_up(1);
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if has_navigator {
                state.move_graph_cursor_down(1);
            } else {
                state.move_cursor_down(1);
            }
        }
        KeyCode::PageUp => {
            let h = state.pane_height.max(1);
            if has_navigator {
                state.move_graph_cursor_up(h);
            } else {
                state.move_cursor_up(h);
            }
        }
        KeyCode::PageDown => {
            let h = state.pane_height.max(1);
            if has_navigator {
                state.move_graph_cursor_down(h);
            } else {
                state.move_cursor_down(h);
            }
        }
        KeyCode::Home | KeyCode::Char('g') => {
            if has_navigator {
                state.graph_cursor = 0;
                state.graph_scroll_offset = 0;
            } else {
                state.cursor = 0;
                state.scroll_offset = 0;
            }
        }
        KeyCode::End | KeyCode::Char('G') => {
            if has_navigator {
                if !state.graph_visible.is_empty() {
                    state.graph_cursor = state.graph_visible.len() - 1;
                }
            } else if !state.visible_ids.is_empty() {
                state.cursor = state.visible_ids.len() - 1;
            }
        }

        // Fold toggle — graph fold when navigator available, legacy otherwise
        KeyCode::Char(' ') => {
            if has_navigator {
                state.graph_toggle_fold();
            } else {
                state.toggle_fold();
            }
        }

        // Enter: same as Space in graph mode; legacy fold/SymRef jump otherwise
        KeyCode::Enter => {
            if has_navigator {
                state.graph_toggle_fold();
            } else {
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
        }

        // Zoom: Right/l = zoom in (expand one level), Left/h = zoom out
        // When navigator is present these drive the Navigator cursor;
        // otherwise they adjust the legacy granularity limit.
        KeyCode::Right | KeyCode::Char('l') => {
            if has_navigator {
                state.graph_zoom_in();
            } else {
                state.expand_cursor_granularity();
            }
        }
        KeyCode::Left | KeyCode::Char('h') => {
            if has_navigator {
                state.graph_zoom_out();
            } else {
                state.shrink_cursor_granularity();
            }
        }

        // Collapse/Expand all — in graph mode clears folds only
        KeyCode::Char('[') => {
            if has_navigator {
                state.graph_clear_folds();
            } else {
                state.collapse_all();
            }
        }
        KeyCode::Char(']') => {
            if has_navigator {
                state.graph_clear_folds();
            } else {
                state.expand_all();
            }
        }

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
