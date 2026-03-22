use std::path::PathBuf;

use crate::app::tree::CodeTree;
use crate::parser::{parse_source, SourceLanguage};

/// The overall application mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppMode {
    /// Normal navigation.
    Normal,
    /// User is typing a filter pattern.
    Filter,
    /// Inline help overlay is shown.
    Help,
}

/// All runtime state for the TUI application.
pub struct AppState {
    /// The parsed code tree being viewed.
    pub tree: CodeTree,
    /// Path to the currently open file (None when empty/demo).
    pub current_file: Option<PathBuf>,
    /// Flat list of currently-visible node ids (after filter / collapse).
    pub visible_ids: Vec<usize>,
    /// Index into `visible_ids` of the currently highlighted row.
    pub cursor: usize,
    /// Vertical scroll offset for the tree pane (in rows).
    pub scroll_offset: usize,
    /// Height of the tree pane (updated every render).
    pub pane_height: usize,
    /// Current UI mode.
    pub mode: AppMode,
    /// Current filter string (empty = no filter).
    pub filter: String,
    /// Status message shown at the bottom bar.
    pub status: String,
    /// Whether the application should quit.
    pub should_quit: bool,
}

impl AppState {
    pub fn new() -> Self {
        AppState {
            tree: CodeTree::new(),
            current_file: None,
            visible_ids: Vec::new(),
            cursor: 0,
            scroll_offset: 0,
            pane_height: 24,
            mode: AppMode::Normal,
            filter: String::new(),
            status: String::from("No file loaded. Pass a source file path as a command-line argument."),
            should_quit: false,
        }
    }

    /// Load a source file, parse it, and refresh the view.
    pub fn load_file(&mut self, path: PathBuf) -> anyhow::Result<()> {
        let source = std::fs::read_to_string(&path)?;
        let lang = SourceLanguage::from_path(&path);
        let fname = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();
        self.tree = parse_source(&source, &lang, &fname)?;
        self.current_file = Some(path.clone());
        self.filter.clear();
        self.cursor = 0;
        self.scroll_offset = 0;
        self.status = format!("Loaded: {}", path.display());
        self.refresh_visible();
        Ok(())
    }

    /// Recompute `visible_ids` from the current tree + filter.
    pub fn refresh_visible(&mut self) {
        let nodes = if self.filter.is_empty() {
            self.tree.visible_nodes()
        } else {
            self.tree.filter_visible(&self.filter)
        };
        self.visible_ids = nodes.iter().map(|n| n.id).collect();

        // Keep cursor in bounds.
        if self.visible_ids.is_empty() {
            self.cursor = 0;
        } else if self.cursor >= self.visible_ids.len() {
            self.cursor = self.visible_ids.len() - 1;
        }
        self.ensure_cursor_visible();
    }

    /// Move cursor up by `n` rows.
    pub fn move_cursor_up(&mut self, n: usize) {
        self.cursor = self.cursor.saturating_sub(n);
        self.ensure_cursor_visible();
    }

    /// Move cursor down by `n` rows.
    pub fn move_cursor_down(&mut self, n: usize) {
        if !self.visible_ids.is_empty() {
            self.cursor = (self.cursor + n).min(self.visible_ids.len() - 1);
        }
        self.ensure_cursor_visible();
    }

    /// Toggle collapse on the node under the cursor.
    pub fn toggle_cursor_collapse(&mut self) {
        if let Some(&id) = self.visible_ids.get(self.cursor) {
            if let Some(node) = self.tree.get(id) {
                if node.is_leaf() {
                    self.status = format!("'{}' has no children to collapse.", node.name);
                    return;
                }
            }
            self.tree.toggle_collapse(id);
            self.refresh_visible();
        }
    }

    /// Collapse all nodes at the cursor's current depth and below.
    pub fn collapse_all(&mut self) {
        let ids: Vec<usize> = self
            .tree
            .all_nodes_dfs()
            .iter()
            .filter(|n| !n.is_leaf())
            .map(|n| n.id)
            .collect();
        for id in ids {
            if let Some(n) = self.tree.get_mut(id) {
                n.collapsed = true;
            }
        }
        self.refresh_visible();
    }

    /// Expand all nodes.
    pub fn expand_all(&mut self) {
        let ids: Vec<usize> = self
            .tree
            .all_nodes_dfs()
            .iter()
            .map(|n| n.id)
            .collect();
        for id in ids {
            if let Some(n) = self.tree.get_mut(id) {
                n.collapsed = false;
            }
        }
        self.refresh_visible();
    }

    /// Enter filter mode.
    pub fn enter_filter(&mut self) {
        self.mode = AppMode::Filter;
        self.filter.clear();
        self.status = String::from("Filter: (type pattern, Enter to confirm, Esc to cancel)");
        self.refresh_visible();
    }

    /// Confirm and exit filter mode.
    pub fn confirm_filter(&mut self) {
        self.mode = AppMode::Normal;
        if self.filter.is_empty() {
            self.status = String::from("Filter cleared.");
        } else {
            self.status = format!(
                "Filter: '{}' — {} results",
                self.filter,
                self.visible_ids.len()
            );
        }
    }

    /// Cancel filter mode, restoring full view.
    pub fn cancel_filter(&mut self) {
        self.mode = AppMode::Normal;
        self.filter.clear();
        self.refresh_visible();
        self.status = String::from("Filter cancelled.");
    }

    /// Append a character to the filter string.
    pub fn push_filter_char(&mut self, c: char) {
        self.filter.push(c);
        self.refresh_visible();
    }

    /// Delete the last character from the filter string.
    pub fn pop_filter_char(&mut self) {
        self.filter.pop();
        self.refresh_visible();
    }

    /// Toggle the help overlay.
    pub fn toggle_help(&mut self) {
        if self.mode == AppMode::Help {
            self.mode = AppMode::Normal;
        } else {
            self.mode = AppMode::Help;
        }
    }

    /// Adjust scroll so that the cursor row is always visible.
    fn ensure_cursor_visible(&mut self) {
        if self.pane_height == 0 {
            return;
        }
        if self.cursor < self.scroll_offset {
            self.scroll_offset = self.cursor;
        } else if self.cursor >= self.scroll_offset + self.pane_height {
            self.scroll_offset = self.cursor + 1 - self.pane_height;
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::tree::{CodeTree, NodeKind};

    fn state_with_sample_tree() -> AppState {
        let mut state = AppState::new();
        let mut tree = CodeTree::new();
        let root = tree.add_node(NodeKind::File, "test.rs", (0, 200), (0, 30), 0, None);
        let f1 = tree.add_node(NodeKind::Function, "fn_a", (0, 100), (0, 15), 1, Some(root));
        let _l1 = tree.add_node(NodeKind::Line, "let x = 1;", (0, 20), (1, 1), 2, Some(f1));
        let _f2 = tree.add_node(NodeKind::Function, "fn_b", (101, 200), (16, 30), 1, Some(root));
        state.tree = tree;
        state.refresh_visible();
        state
    }

    #[test]
    fn test_initial_state_no_file() {
        let state = AppState::new();
        assert!(state.current_file.is_none());
        assert!(state.visible_ids.is_empty());
        assert_eq!(state.cursor, 0);
        assert_eq!(state.mode, AppMode::Normal);
    }

    #[test]
    fn test_cursor_movement() {
        let mut state = state_with_sample_tree();
        assert_eq!(state.cursor, 0);
        state.move_cursor_down(1);
        assert_eq!(state.cursor, 1);
        state.move_cursor_up(1);
        assert_eq!(state.cursor, 0);
    }

    #[test]
    fn test_cursor_does_not_go_below_zero() {
        let mut state = state_with_sample_tree();
        state.move_cursor_up(100);
        assert_eq!(state.cursor, 0);
    }

    #[test]
    fn test_cursor_does_not_exceed_visible() {
        let mut state = state_with_sample_tree();
        state.move_cursor_down(1000);
        assert_eq!(state.cursor, state.visible_ids.len() - 1);
    }

    #[test]
    fn test_toggle_collapse_updates_visible() {
        let mut state = state_with_sample_tree();
        let initial = state.visible_ids.len();
        // Collapse root node.
        state.cursor = 0;
        state.toggle_cursor_collapse();
        assert!(state.visible_ids.len() < initial);
    }

    #[test]
    fn test_filter_mode_enter_cancel() {
        let mut state = state_with_sample_tree();
        state.enter_filter();
        assert_eq!(state.mode, AppMode::Filter);
        state.cancel_filter();
        assert_eq!(state.mode, AppMode::Normal);
        assert!(state.filter.is_empty());
    }

    #[test]
    fn test_filter_narrows_results() {
        let mut state = state_with_sample_tree();
        state.enter_filter();
        state.push_filter_char('a');
        // "fn_a" and "fn_bar" (from filter) — our tree has fn_a and let x=1
        let count = state.visible_ids.len();
        assert!(count < 4, "Filter should narrow results, got {count}");
    }

    #[test]
    fn test_collapse_all_expand_all() {
        let mut state = state_with_sample_tree();
        state.collapse_all();
        assert_eq!(state.visible_ids.len(), 1); // only root visible
        state.expand_all();
        assert_eq!(state.visible_ids.len(), 4);
    }

    #[test]
    fn test_help_toggle() {
        let mut state = AppState::new();
        assert_eq!(state.mode, AppMode::Normal);
        state.toggle_help();
        assert_eq!(state.mode, AppMode::Help);
        state.toggle_help();
        assert_eq!(state.mode, AppMode::Normal);
    }
}
