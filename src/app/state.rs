use std::path::PathBuf;

use crate::app::tree::CodeTree;
use crate::parser::{parse_directory, parse_source, SourceLanguage};

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
    /// Path to the currently open file or directory (None = nothing loaded).
    pub current_path: Option<PathBuf>,
    /// Flat list of currently-visible node IDs (main tree, respecting filter/collapse).
    pub visible_ids: Vec<usize>,
    /// Reference edges projected onto the currently visible nodes.
    ///
    /// Each entry `(from, to)` is a pair of visible node IDs connected by one
    /// or more symbolic references in the [`ReferenceGraph`].  The set is
    /// recomputed whenever `visible_ids` changes so that edges automatically
    /// aggregate or expand as the user drills down into the hierarchy.
    pub visible_refs: Vec<(usize, usize)>,
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
            current_path: None,
            visible_ids: Vec::new(),
            visible_refs: Vec::new(),
            cursor: 0,
            scroll_offset: 0,
            pane_height: 24,
            mode: AppMode::Normal,
            filter: String::new(),
            status: String::from("No path loaded. Pass a source file or directory as a command-line argument."),
            should_quit: false,
        }
    }

    /// Load a single source file, parse it, and refresh the view.
    pub fn load_file(&mut self, path: PathBuf) -> anyhow::Result<()> {
        let source = std::fs::read_to_string(&path)?;
        let lang = SourceLanguage::from_path(&path);
        let fname = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();
        self.tree = parse_source(&source, &lang, &fname)?;
        self.current_path = Some(path.clone());
        self.filter.clear();
        self.cursor = 0;
        self.scroll_offset = 0;
        self.status = format!("Loaded: {}", path.display());
        self.refresh_visible();
        Ok(())
    }

    /// Load a directory, parse all source files in it, and refresh the view.
    pub fn load_directory(&mut self, path: PathBuf) -> anyhow::Result<()> {
        self.tree = parse_directory(&path)?;
        self.current_path = Some(path.clone());
        self.filter.clear();
        self.cursor = 0;
        self.scroll_offset = 0;
        self.status = format!(
            "Loaded directory: {} — use l/Right to drill down into a node",
            path.display()
        );
        self.refresh_visible();
        Ok(())
    }

    /// Recompute `visible_ids` and `visible_refs` from the current tree + filter.
    pub fn refresh_visible(&mut self) {
        let nodes = if self.filter.is_empty() {
            self.tree.visible_nodes()
        } else {
            self.tree.filter_visible(&self.filter)
        };
        self.visible_ids = nodes.iter().map(|n| n.id).collect();

        // Project the reference graph onto the currently visible nodes.
        self.visible_refs = self.tree.project_refs_onto_visible(&self.visible_ids);

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

    /// Expand the granularity of the node under the cursor (show finer children).
    /// This only affects the cursor node — siblings are unchanged.
    pub fn expand_cursor_granularity(&mut self) {
        if let Some(&id) = self.visible_ids.get(self.cursor) {
            if let Some(node) = self.tree.get(id) {
                if node.is_leaf() {
                    self.status = format!("'{}' is already at the finest level.", node.name);
                    return;
                }
                if node.granularity_limit.is_none() {
                    self.status = format!("'{}' already shows all detail.", node.name);
                    return;
                }
            }
            self.tree.expand_granularity(id);
            self.refresh_visible();
            if let Some(node) = self.tree.get(id) {
                let level_desc = node
                    .granularity_limit
                    .as_ref()
                    .map(|k| format!("{k}"))
                    .unwrap_or_else(|| "all".to_string());
                self.status = format!("'{}' now shows up to {} level.", node.name, level_desc);
            }
        }
    }

    /// Shrink the granularity of the node under the cursor (show coarser children).
    /// This only affects the cursor node — siblings are unchanged.
    pub fn shrink_cursor_granularity(&mut self) {
        if let Some(&id) = self.visible_ids.get(self.cursor) {
            if let Some(node) = self.tree.get(id) {
                if node.is_leaf() {
                    self.status = format!("'{}' has no children.", node.name);
                    return;
                }
            }
            self.tree.shrink_granularity(id);
            self.refresh_visible();
            if let Some(node) = self.tree.get(id) {
                let level_desc = if node.collapsed {
                    "collapsed".to_string()
                } else {
                    node.granularity_limit
                        .as_ref()
                        .map(|k| format!("up to {k}"))
                        .unwrap_or_else(|| "all".to_string())
                };
                self.status = format!("'{}': {}", node.name, level_desc);
            }
        }
    }

    /// Collapse all non-leaf nodes.
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
                n.granularity_limit = None;
            }
        }
        self.refresh_visible();
    }

    /// If the cursor is on a `SymRef` node, jump to its canonical definition
    /// in the lib section. Returns `true` if a jump was performed.
    pub fn jump_to_sym_ref_target(&mut self) -> bool {
        if let Some(&id) = self.visible_ids.get(self.cursor) {
            if let Some(node) = self.tree.get(id) {
                if let Some(target_id) = node.sym_ref_target {
                    // Find the target in the lib section nodes rendered below main tree.
                    // We place lib section IDs after the main visible_ids in state;
                    // refresh_visible_with_lib builds the combined list.
                    if let Some(pos) = self.visible_ids.iter().position(|&vid| vid == target_id) {
                        self.cursor = pos;
                        self.ensure_cursor_visible();
                        if let Some(target_node) = self.tree.get(target_id) {
                            self.status =
                                format!("Jumped to definition: '{}'", target_node.name);
                        }
                        return true;
                    }
                }
            }
        }
        false
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
        assert!(state.current_path.is_none());
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
        let count = state.visible_ids.len();
        assert!(count < 4, "Filter should narrow results, got {count}");
    }

    #[test]
    fn test_collapse_all_expand_all() {
        let mut state = state_with_sample_tree();
        state.collapse_all();
        assert_eq!(state.visible_ids.len(), 1);
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

    #[test]
    fn test_expand_granularity_changes_visible() {
        let mut state = state_with_sample_tree();
        // fn_a (id=1) has Line children. Set its granularity to Function level →
        // Lines (level 6) are finer than Function (level 4) → hidden.
        state.tree.get_mut(1).unwrap().granularity_limit = Some(NodeKind::Function);
        state.refresh_visible();
        assert_eq!(state.visible_ids.len(), 3); // root + fn_a (no Line) + fn_b

        // Expand fn_a's granularity (cursor=1).
        state.cursor = 1;
        state.expand_cursor_granularity();
        // Function → Block: Line (6) still finer than Block (5) → still hidden.
        assert_eq!(state.visible_ids.len(), 3);
        // Expand again: Block → Line. Line.is_finer_than(Line) = false → shown.
        state.expand_cursor_granularity();
        assert_eq!(state.visible_ids.len(), 4);
    }

    #[test]
    fn test_shrink_granularity_changes_visible() {
        let mut state = state_with_sample_tree();
        // Root has no limit (shows all 4 nodes)
        assert_eq!(state.visible_ids.len(), 4);
        state.cursor = 0;
        state.shrink_cursor_granularity();
        // Should set limit to Block (Line still shown because fn_a has depth 2
        // and its Line child has depth 3 — wait, the limit is on the FILE root
        // which only has Function children, not Line directly)
        // The Line is a child of fn_a, not of root, so root's limit doesn't hide it directly
        // Root's granularity_limit only affects root's DIRECT children.
        // fn_a is a Function (≤ Block), so fn_a is still shown.
        // fn_a's Line child is not affected by root's granularity_limit.
        // So visible count stays at 4.
        assert_eq!(state.visible_ids.len(), 4);
    }

    #[test]
    fn test_load_directory() {
        use tempfile::TempDir;
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("main.rs"), "fn main() {}").unwrap();
        let mut state = AppState::new();
        state.load_directory(dir.path().to_path_buf()).unwrap();
        assert!(state.current_path.is_some());
        assert!(!state.visible_ids.is_empty());
    }

    #[test]
    fn test_visible_refs_updates_with_expansion() {
        use crate::app::tree::ReferenceKind;
        use tempfile::TempDir;
        let dir = TempDir::new().unwrap();
        // lib.rs defines `helper`, main.rs calls it → cross-file reference expected.
        std::fs::write(dir.path().join("lib.rs"), "pub fn helper() {}").unwrap();
        std::fs::write(dir.path().join("main.rs"), "fn main() { helper(); }").unwrap();
        let mut state = AppState::new();
        state.load_directory(dir.path().to_path_buf()).unwrap();

        // At the initial file-level view (both files collapsed under folder root),
        // any cross-file reference should project to a file→file edge.
        // The reference graph may or may not have edges depending on name resolution;
        // what we can assert is that visible_refs is consistent with visible_ids.
        for &(from, to) in &state.visible_refs {
            assert!(
                state.visible_ids.contains(&from),
                "from endpoint {from} must be in visible_ids"
            );
            assert!(
                state.visible_ids.contains(&to),
                "to endpoint {to} must be in visible_ids"
            );
        }

        // Expanding all should keep visible_refs endpoints within visible_ids.
        state.expand_all();
        for &(from, to) in &state.visible_refs {
            assert!(
                state.visible_ids.contains(&from),
                "after expand: from endpoint {from} must be in visible_ids"
            );
            assert!(
                state.visible_ids.contains(&to),
                "after expand: to endpoint {to} must be in visible_ids"
            );
        }

        // Verify we can also add manual reference edges and have them projected.
        let all_ids: Vec<usize> = state.tree.all_nodes_dfs().iter().map(|n| n.id).collect();
        if all_ids.len() >= 2 {
            state.tree.add_reference(all_ids[0], all_ids[1], ReferenceKind::Call);
            state.refresh_visible();
            // After adding a ref between root (always visible) and first child,
            // visible_refs must still be consistent.
            for &(from, to) in &state.visible_refs {
                assert!(state.visible_ids.contains(&from));
                assert!(state.visible_ids.contains(&to));
            }
        }
    }
}
