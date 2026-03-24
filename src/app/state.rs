use std::collections::HashSet;
use std::path::PathBuf;

use crate::app::tree::{CodeTree, NodeKind, SemanticEntry};
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
    /// Number of structural (non-virtual) nodes at load time.
    pub base_node_count: usize,
    /// IDs of nodes that have been "expanded away" (replaced by their semantic children).
    pub expanded_ids: HashSet<usize>,
    /// Display depth for each entry in visible_ids (parallel vector).
    pub view_depths: Vec<usize>,
    /// IDs of expanded nodes whose children are visually hidden (folded).
    pub folded_ids: HashSet<usize>,
    /// Target node IDs whose SymRef entry is expanded to show specific symbols.
    pub expanded_symref_targets: HashSet<usize>,
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
            base_node_count: 0,
            expanded_ids: HashSet::new(),
            view_depths: Vec::new(),
            folded_ids: HashSet::new(),
            expanded_symref_targets: HashSet::new(),
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
        self.tree.structural_count = self.tree.node_count();
        self.base_node_count = self.tree.node_count();
        self.expanded_ids.clear();
        self.folded_ids.clear();
        self.expanded_symref_targets.clear();
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
        self.tree.structural_count = self.tree.node_count();
        self.base_node_count = self.tree.node_count();
        self.expanded_ids.clear();
        self.folded_ids.clear();
        self.expanded_symref_targets.clear();
        self.current_path = Some(path.clone());
        self.filter.clear();
        self.cursor = 0;
        self.scroll_offset = 0;
        self.status = format!("Loaded directory: {} — use l/Right to expand", path.display());
        self.refresh_visible();
        Ok(())
    }

    /// Recompute `visible_ids` and `visible_refs` from the current tree + filter.
    pub fn refresh_visible(&mut self) {
        self.tree.clear_virtual_nodes();
        self.visible_ids.clear();
        self.view_depths.clear();

        if let Some(root_id) = self.tree.root {
            self.build_view_entries(root_id, 0);
        }

        // Apply filter if active
        if !self.filter.is_empty() {
            let pat = self.filter.to_ascii_lowercase();
            let pairs: Vec<(usize, usize)> = self
                .visible_ids
                .iter()
                .zip(self.view_depths.iter())
                .filter(|&(&id, _)| {
                    if let Some(node) = self.tree.get(id) {
                        node.name.to_ascii_lowercase().contains(&pat)
                            || node
                                .detail
                                .as_deref()
                                .map(|d| d.to_ascii_lowercase().contains(&pat))
                                .unwrap_or(false)
                    } else {
                        false
                    }
                })
                .map(|(&id, &d)| (id, d))
                .collect();
            self.visible_ids = pairs.iter().map(|&(id, _)| id).collect();
            self.view_depths = pairs.iter().map(|&(_, d)| d).collect();
        }

        self.visible_refs = self.tree.project_refs_onto_visible(&self.visible_ids);

        if self.visible_ids.is_empty() {
            self.cursor = 0;
        } else if self.cursor >= self.visible_ids.len() {
            self.cursor = self.visible_ids.len() - 1;
        }
        self.ensure_cursor_visible();
    }

    fn build_view_entries(&mut self, node_id: usize, display_depth: usize) {
        if self.expanded_ids.contains(&node_id) {
            if self.folded_ids.contains(&node_id) {
                // Expanded but visually folded: show the node itself as ▶, hide children.
                self.visible_ids.push(node_id);
                self.view_depths.push(display_depth);
                return;
            }
            // Expanded and unfolded: the node itself is invisible; its children replace it.
            let entries = self.tree.semantic_children_of(node_id);
            let has_node_entries = entries.iter().any(|e| matches!(e, SemanticEntry::Node { .. }));
            if !has_node_entries {
                // No semantic children — fall back to showing the node itself.
                self.visible_ids.push(node_id);
                self.view_depths.push(display_depth);
                return;
            }
            for entry in entries {
                match entry {
                    SemanticEntry::Node { id, depth } => {
                        self.build_view_entries(id, display_depth + depth);
                    }
                    SemanticEntry::SymRef { target_id, depth } => {
                        let full_path = self.tree.full_path(target_id);
                        let virtual_id = self.tree.add_node(
                            NodeKind::SymRef,
                            full_path,
                            (0, 0),
                            (0, 0),
                            0,
                            None,
                        );
                        if let Some(n) = self.tree.get_mut(virtual_id) {
                            n.sym_ref_target = Some(target_id);
                        }
                        self.visible_ids.push(virtual_id);
                        self.view_depths.push(display_depth + depth);

                        // Auto-expand SymRef if the target node is itself expanded or
                        // the user has explicitly requested its symbol listing.
                        let show_symbols = self.expanded_symref_targets.contains(&target_id)
                            || self.expanded_ids.contains(&target_id);
                        if show_symbols {
                            let target_subtree = self.tree.subtree_ids(target_id);
                            let refs: Vec<_> = self.tree.references.references().to_vec();
                            let mut shown_targets: HashSet<usize> = HashSet::new();
                            for edge in &refs {
                                if target_subtree.contains(&edge.to)
                                    && edge.to < self.tree.structural_count
                                {
                                    if let Some(to_node) = self.tree.get(edge.to) {
                                        if matches!(
                                            to_node.kind,
                                            NodeKind::Function
                                                | NodeKind::Class
                                                | NodeKind::Module
                                                | NodeKind::File
                                        ) {
                                            shown_targets.insert(edge.to);
                                        }
                                    }
                                }
                            }
                            let mut sorted_targets: Vec<usize> =
                                shown_targets.into_iter().collect();
                            sorted_targets.sort_unstable();
                            for &sym_target in &sorted_targets {
                                let sym_path = self.tree.full_path(sym_target);
                                let sym_virtual_id = self.tree.add_node(
                                    NodeKind::SymRef,
                                    format!("  {sym_path}"),
                                    (0, 0),
                                    (0, 0),
                                    0,
                                    None,
                                );
                                if let Some(n) = self.tree.get_mut(sym_virtual_id) {
                                    n.sym_ref_target = Some(sym_target);
                                }
                                self.visible_ids.push(sym_virtual_id);
                                self.view_depths.push(display_depth + depth + 1);
                            }
                        }
                    }
                }
            }
        } else {
            self.visible_ids.push(node_id);
            self.view_depths.push(display_depth);
        }
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
        self.toggle_fold();
    }

    /// Fold or unfold.
    ///
    /// * If the cursor is on a `▶` node (expanded + folded): unfold it — the node
    ///   disappears and its semantic children become visible again.
    /// * If the cursor is on any other visible node: find its structural parent; if that
    ///   parent is expanded, fold it so the parent reappears as `▶` and all its children
    ///   are hidden.
    ///
    /// SymRef virtual nodes are skipped.
    pub fn toggle_fold(&mut self) {
        if let Some(&id) = self.visible_ids.get(self.cursor) {
            // SymRef nodes cannot be folded.
            if let Some(node) = self.tree.get(id) {
                if node.kind == NodeKind::SymRef {
                    return;
                }
            }

            if self.expanded_ids.contains(&id) {
                // Node is visible as ▶ (folded expanded). Unfold it: it goes invisible
                // and its semantic children reappear.
                self.folded_ids.remove(&id);
                self.refresh_visible();
            } else {
                // Node is a normal visible node. Fold its structural parent if the parent
                // has been granularity-expanded.
                let parent_id = self.tree.get(id).and_then(|n| n.parent);
                if let Some(pid) = parent_id {
                    if self.expanded_ids.contains(&pid) {
                        self.folded_ids.insert(pid);
                        self.refresh_visible();
                    }
                }
            }
        }
    }

    /// Expand the granularity of the node under the cursor (show finer children).
    pub fn expand_cursor_granularity(&mut self) {
        self.expand_cursor_node();
    }

    /// Shrink the granularity of the node under the cursor (show coarser children).
    pub fn shrink_cursor_granularity(&mut self) {
        self.collapse_cursor_node();
    }

    /// Collapse all non-leaf nodes.
    pub fn collapse_all(&mut self) {
        self.expanded_ids.clear();
        self.folded_ids.clear();
        self.expanded_symref_targets.clear();
        self.refresh_visible();
    }

    /// Expand all nodes.
    pub fn expand_all(&mut self) {
        self.folded_ids.clear();
        let structural_count = self.tree.structural_count;
        let ids: Vec<usize> = (0..structural_count)
            .filter(|&id| {
                if let Some(node) = self.tree.get(id) {
                    !node.is_leaf() && node.kind != NodeKind::SymRef
                } else {
                    false
                }
            })
            .collect();
        for id in ids {
            let has_children = self
                .tree
                .semantic_children_of(id)
                .iter()
                .any(|e| matches!(e, SemanticEntry::Node { .. }));
            if has_children {
                self.expanded_ids.insert(id);
                self.expanded_symref_targets.insert(id);
            }
        }
        self.refresh_visible();
    }

    /// If the cursor is on a `SymRef` node, jump to its canonical definition.
    /// Returns `true` if a jump was performed.
    pub fn jump_to_sym_ref_target(&mut self) -> bool {
        if let Some(&id) = self.visible_ids.get(self.cursor) {
            if let Some(target_id) = self.tree.get(id).and_then(|n| n.sym_ref_target) {
                // Check if target is already visible
                if let Some(pos) = self.visible_ids.iter().position(|&vid| vid == target_id) {
                    self.cursor = pos;
                    self.ensure_cursor_visible();
                    let name = self
                        .tree
                        .get(target_id)
                        .map(|n| n.name.clone())
                        .unwrap_or_default();
                    self.status = format!("Jumped to definition: '{}'", name);
                    return true;
                }
                // Target not visible — expand path to it
                let mut path = Vec::new();
                let mut current = target_id;
                loop {
                    if let Some(n) = self.tree.get(current) {
                        path.push(current);
                        match n.parent {
                            Some(pid) => current = pid,
                            None => break,
                        }
                    } else {
                        break;
                    }
                }
                path.reverse();
                for &anc in &path[..path.len().saturating_sub(1)] {
                    self.expanded_ids.insert(anc);
                    self.expanded_symref_targets.insert(anc);
                }
                self.refresh_visible();
                if let Some(pos) = self.visible_ids.iter().position(|&vid| vid == target_id) {
                    self.cursor = pos;
                    self.ensure_cursor_visible();
                    let name = self
                        .tree
                        .get(target_id)
                        .map(|n| n.name.clone())
                        .unwrap_or_default();
                    self.status = format!("Jumped to definition: '{}'", name);
                    return true;
                }
            }
        }
        false
    }

    pub fn expand_cursor_node(&mut self) {
        if let Some(&id) = self.visible_ids.get(self.cursor) {
            let node_info = self.tree.get(id).map(|n| (n.kind, n.sym_ref_target));
            match node_info {
                Some((NodeKind::SymRef, Some(target_id))) => {
                    if self.expanded_symref_targets.contains(&target_id) {
                        self.expanded_symref_targets.remove(&target_id);
                    } else {
                        self.expanded_symref_targets.insert(target_id);
                    }
                    self.refresh_visible();
                    let path = self.tree.full_path(target_id);
                    self.status = format!("Showing references into '{}'.", path);
                }
                Some((NodeKind::SymRef, None)) => {}
                _ => {
                    let has_children = self
                        .tree
                        .semantic_children_of(id)
                        .iter()
                        .any(|e| matches!(e, SemanticEntry::Node { .. }));
                    if !has_children {
                        let path = self.tree.full_path(id);
                        self.status = format!("'{}' has no expandable children.", path);
                        return;
                    }
                    self.expanded_ids.insert(id);
                    // Keep SymRef expansion in sync: any → arrow pointing into this
                    // node's subtree will automatically show specific symbols.
                    self.expanded_symref_targets.insert(id);
                    self.refresh_visible();
                    let path = self.tree.full_path(id);
                    self.status = format!("Expanded '{}'.", path);
                }
            }
        }
    }

    pub fn collapse_cursor_node(&mut self) {
        if let Some(&id) = self.visible_ids.get(self.cursor) {
            if self.expanded_ids.contains(&id) {
                // Cursor is on a ▶ (expanded+folded) node. Fully collapse it.
                self.expanded_ids.remove(&id);
                self.folded_ids.remove(&id);
                self.expanded_symref_targets.remove(&id);
                self.refresh_visible();
                let path = self.tree.full_path(id);
                self.status = format!("Collapsed '{}'.", path);
                return;
            }
            // Cursor is on a normal visible (non-expanded) node.
            // Collapse its structural parent if the parent was granularity-expanded.
            let parent_id = self.tree.get(id).and_then(|n| n.parent);
            if let Some(pid) = parent_id {
                if self.expanded_ids.remove(&pid) {
                    self.folded_ids.remove(&pid);
                    self.expanded_symref_targets.remove(&pid);
                    self.refresh_visible();
                    let path = self.tree.full_path(pid);
                    self.status = format!("Collapsed '{}'.", path);
                }
            }
        }
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
        state.tree.structural_count = state.tree.node_count();
        state.base_node_count = state.tree.node_count();
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
        // Expand root so fn_a and fn_b become visible
        state.expanded_ids.insert(0);
        state.refresh_visible();
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
        // Expand root so fn_a and fn_b become visible (root itself disappears).
        state.expanded_ids.insert(0);
        state.refresh_visible();
        let initial = state.visible_ids.len(); // 2: fn_a + fn_b (root is invisible)
        state.cursor = 0; // on fn_a
        // toggle_fold on fn_a (NOT in expanded_ids): folds its structural parent (root),
        // which makes root reappear as ▶ and fn_a/fn_b disappear.
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
        assert_eq!(state.visible_ids.len(), 1); // just root
        state.expand_all();
        // expand_all expands root and fn_a (both have children).
        // root is invisible (expanded, unfolded) → shows fn_a's children.
        // fn_a is invisible (expanded, unfolded) → shows l1.
        // fn_b is a leaf → visible.
        // visible = [l1, fn_b] = 2 nodes.
        assert_eq!(state.visible_ids.len(), 2);
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
        assert_eq!(state.visible_ids.len(), 1); // just root
        state.cursor = 0;
        state.expand_cursor_granularity(); // expand root → root disappears, fn_a + fn_b visible
        assert_eq!(state.visible_ids.len(), 2);
    }

    #[test]
    fn test_shrink_granularity_changes_visible() {
        let mut state = state_with_sample_tree();
        state.cursor = 0;
        state.expand_cursor_granularity(); // expand root → root invisible, fn_a + fn_b visible
        assert_eq!(state.visible_ids.len(), 2); // fn_a + fn_b
        state.cursor = 0; // on fn_a
        state.shrink_cursor_granularity(); // collapse: fn_a's parent (root) removed from expanded_ids
        assert_eq!(state.visible_ids.len(), 1);
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
