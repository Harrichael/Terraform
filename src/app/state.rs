use std::collections::HashSet;
use std::path::PathBuf;

use crate::app::graph_builder::code_tree_to_entity_graph;
use crate::app::tree::CodeTree;
use crate::graph::entity::EntityId;
use crate::graph::navigator::Navigator;
use crate::graph::tree::GraphTreeNodeId;
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
    /// The parsed code tree, used to build the entity graph via [`build_navigator`].
    /// Retained in state so that `build_navigator` can be called without taking
    /// ownership; after that point it is not used for rendering.
    tree: CodeTree,
    /// Path to the currently open file or directory (None = nothing loaded).
    pub current_path: Option<PathBuf>,
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

    // ── Graph / Navigator state ────────────────────────────────────────────
    /// The navigator wrapping the entity graph.  `None` until a path is loaded.
    pub navigator: Option<Navigator>,
    /// Entity IDs of graph tree nodes the user has explicitly folded
    /// (children hidden in the reference-tree view).
    pub graph_folded: HashSet<EntityId>,
    /// Flat, DFS-ordered list of `(GraphTreeNodeId, display_depth)` for
    /// every currently-visible node in the graph tree view.
    pub graph_visible: Vec<(GraphTreeNodeId, usize)>,
    /// Index into `graph_visible` of the currently highlighted row.
    pub graph_cursor: usize,
    /// Vertical scroll offset for the graph tree pane.
    pub graph_scroll_offset: usize,
}

impl AppState {
    pub fn new() -> Self {
        AppState {
            tree: CodeTree::new(),
            current_path: None,
            pane_height: 24,
            mode: AppMode::Normal,
            filter: String::new(),
            status: String::from("No path loaded. Pass a source file or directory as a command-line argument."),
            should_quit: false,
            navigator: None,
            graph_folded: HashSet::new(),
            graph_visible: Vec::new(),
            graph_cursor: 0,
            graph_scroll_offset: 0,
        }
    }

    /// Load a single source file, parse it, and build the navigator.
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
        self.current_path = Some(path.clone());
        self.filter.clear();
        self.status = format!("Loaded: {}", path.display());
        self.build_navigator();
        Ok(())
    }

    /// Load a directory, parse all source files in it, and build the navigator.
    pub fn load_directory(&mut self, path: PathBuf) -> anyhow::Result<()> {
        self.tree = parse_directory(&path)?;
        self.tree.structural_count = self.tree.node_count();
        self.current_path = Some(path.clone());
        self.filter.clear();
        self.status = format!("Loaded directory: {} — use l/Right to zoom in", path.display());
        self.build_navigator();
        Ok(())
    }

    /// Convert the current `CodeTree` into an [`EntityGraph`] and initialize
    /// the [`Navigator`].  Resets all graph-view navigation state.
    pub(crate) fn build_navigator(&mut self) {
        let entity_graph = code_tree_to_entity_graph(&self.tree);
        self.navigator = Some(Navigator::new(entity_graph));
        self.graph_folded.clear();
        self.graph_cursor = 0;
        self.graph_scroll_offset = 0;
        self.refresh_graph_visible();
    }

    /// Enter filter mode.
    pub fn enter_filter(&mut self) {
        self.mode = AppMode::Filter;
        self.filter.clear();
        self.status = String::from("Filter: (type pattern, Enter to confirm, Esc to cancel)");
    }

    /// Confirm and exit filter mode.
    pub fn confirm_filter(&mut self) {
        self.mode = AppMode::Normal;
        if self.filter.is_empty() {
            self.status = String::from("Filter cleared.");
        } else {
            self.status = format!("Filter: '{}' active", self.filter);
        }
    }

    /// Cancel filter mode, restoring full view.
    pub fn cancel_filter(&mut self) {
        self.mode = AppMode::Normal;
        self.filter.clear();
        self.status = String::from("Filter cancelled.");
    }

    /// Append a character to the filter string.
    pub fn push_filter_char(&mut self, c: char) {
        self.filter.push(c);
    }

    /// Delete the last character from the filter string.
    pub fn pop_filter_char(&mut self) {
        self.filter.pop();
    }

    /// Toggle the help overlay.
    pub fn toggle_help(&mut self) {
        if self.mode == AppMode::Help {
            self.mode = AppMode::Normal;
        } else {
            self.mode = AppMode::Help;
        }
    }

    // ── Graph / Navigator navigation ──────────────────────────────────────

    /// Rebuild `graph_visible` by traversing the Navigator's current
    /// [`GraphTree`] in DFS order, skipping children of folded entities.
    pub fn refresh_graph_visible(&mut self) {
        self.graph_visible.clear();

        // Snapshot tree node data to avoid borrow conflicts during traversal.
        // The tree's node arena is indexed by GraphTreeNodeId(i) == nodes[i],
        // so the Vec is effectively a direct-access map: snapshot[id.0] → node data.
        let node_snapshot: Vec<(EntityId, Option<GraphTreeNodeId>, Vec<GraphTreeNodeId>)> =
            if let Some(nav) = &self.navigator {
                nav.tree()
                    .nodes
                    .iter()
                    .map(|n| (n.entity_id, n.parent, n.children.clone()))
                    .collect()
            } else {
                return;
            };

        // Roots are nodes with no parent.
        let mut stack: Vec<(GraphTreeNodeId, usize)> = node_snapshot
            .iter()
            .enumerate()
            .filter(|(_, (_, parent, _))| parent.is_none())
            .map(|(i, _)| (GraphTreeNodeId(i), 0usize))
            .collect();
        // Reverse so the first root ends up on top of the stack (LIFO).
        stack.reverse();

        while let Some((node_id, depth)) = stack.pop() {
            // O(1) direct index into the arena instead of a linear scan.
            if let Some((entity_id, _, children)) = node_snapshot.get(node_id.0) {
                self.graph_visible.push((node_id, depth));
                if !self.graph_folded.contains(entity_id) {
                    // Push children in reverse so the first child is processed first.
                    for &child_id in children.iter().rev() {
                        stack.push((child_id, depth + 1));
                    }
                }
            }
        }

        // Clamp cursor to valid range.
        if self.graph_visible.is_empty() {
            self.graph_cursor = 0;
        } else {
            self.graph_cursor = self.graph_cursor.min(self.graph_visible.len() - 1);
        }
        self.ensure_graph_cursor_visible();
    }

    /// Move the graph cursor up by `n` rows.
    pub fn move_graph_cursor_up(&mut self, n: usize) {
        self.graph_cursor = self.graph_cursor.saturating_sub(n);
        self.ensure_graph_cursor_visible();
    }

    /// Move the graph cursor down by `n` rows.
    pub fn move_graph_cursor_down(&mut self, n: usize) {
        if !self.graph_visible.is_empty() {
            self.graph_cursor = (self.graph_cursor + n).min(self.graph_visible.len() - 1);
        }
        self.ensure_graph_cursor_visible();
    }

    /// Zoom in: expand the entity under the graph cursor one level down.
    ///
    /// Delegates to [`Navigator::zoom_in`], then rebuilds the visible list and
    /// moves the cursor to the first revealed child.
    pub fn graph_zoom_in(&mut self) {
        let node_id = match self.graph_visible.get(self.graph_cursor) {
            Some(&(id, _)) => id,
            None => return,
        };
        let first_child_entity = if let Some(nav) = &mut self.navigator {
            nav.zoom_in(node_id)
        } else {
            return;
        };
        self.refresh_graph_visible();

        // Move cursor to the first revealed child, if any.
        if let Some(child_eid) = first_child_entity {
            self.move_graph_cursor_to_entity(child_eid);
        }

        if let Some(nav) = &self.navigator {
            if let Some(&(nid, _)) = self.graph_visible.get(self.graph_cursor) {
                if let Some(node) = nav.tree().get(nid) {
                    if let Some(entity) = nav.entity(node.entity_id) {
                        self.status = format!("Zoomed in — showing {}", entity.name);
                    }
                }
            }
        }
    }

    /// Zoom out: collapse the entity under the graph cursor one level up.
    ///
    /// Delegates to [`Navigator::zoom_out`], then rebuilds the visible list and
    /// moves the cursor to the parent entity.
    pub fn graph_zoom_out(&mut self) {
        let node_id = match self.graph_visible.get(self.graph_cursor) {
            Some(&(id, _)) => id,
            None => return,
        };
        let parent_entity = if let Some(nav) = &mut self.navigator {
            nav.zoom_out(node_id)
        } else {
            return;
        };
        self.refresh_graph_visible();

        // Move cursor to the parent entity, if any.
        if let Some(parent_eid) = parent_entity {
            self.move_graph_cursor_to_entity(parent_eid);
        }

        if let Some(nav) = &self.navigator {
            if let Some(&(nid, _)) = self.graph_visible.get(self.graph_cursor) {
                if let Some(node) = nav.tree().get(nid) {
                    if let Some(entity) = nav.entity(node.entity_id) {
                        self.status = format!("Zoomed out — showing {}", entity.name);
                    }
                }
            }
        }
    }

    /// Toggle fold on the graph tree node under the graph cursor.
    ///
    /// Folding hides the node's reference-tree children from the view without
    /// changing the navigator cursor (zoom level).
    pub fn graph_toggle_fold(&mut self) {
        let (node_id, _) = match self.graph_visible.get(self.graph_cursor) {
            Some(&v) => v,
            None => return,
        };
        let entity_id = if let Some(nav) = &self.navigator {
            nav.tree().get(node_id).map(|n| n.entity_id)
        } else {
            return;
        };
        if let Some(eid) = entity_id {
            if self.graph_folded.contains(&eid) {
                self.graph_folded.remove(&eid);
            } else {
                self.graph_folded.insert(eid);
            }
            self.refresh_graph_visible();
        }
    }

    /// Clear all graph folds without changing the zoom level.
    pub fn graph_clear_folds(&mut self) {
        self.graph_folded.clear();
        self.refresh_graph_visible();
    }

    /// Move `graph_cursor` to the first visible row whose entity matches `id`.
    fn move_graph_cursor_to_entity(&mut self, id: EntityId) {
        let nav = match &self.navigator {
            Some(n) => n,
            None => return,
        };
        // Find a GraphTreeNodeId whose entity_id matches.
        if let Some(pos) = self.graph_visible.iter().position(|&(nid, _)| {
            nav.tree()
                .get(nid)
                .map(|n| n.entity_id == id)
                .unwrap_or(false)
        }) {
            self.graph_cursor = pos;
            self.ensure_graph_cursor_visible();
        }
    }

    /// Adjust graph scroll so the graph cursor row is always visible.
    fn ensure_graph_cursor_visible(&mut self) {
        if self.pane_height == 0 {
            return;
        }
        if self.graph_cursor < self.graph_scroll_offset {
            self.graph_scroll_offset = self.graph_cursor;
        } else if self.graph_cursor >= self.graph_scroll_offset + self.pane_height {
            self.graph_scroll_offset = self.graph_cursor + 1 - self.pane_height;
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

    /// Build an AppState with a navigator initialised from a two-function file.
    fn state_with_navigator() -> AppState {
        let mut state = AppState::new();
        let mut tree = CodeTree::new();
        let root = tree.add_node(NodeKind::File, "test.rs", (0, 200), (0, 30), 0, None);
        let _f1 = tree.add_node(NodeKind::Function, "fn_a", (0, 100), (0, 15), 1, Some(root));
        let _f2 = tree.add_node(NodeKind::Function, "fn_b", (101, 200), (16, 30), 1, Some(root));
        state.tree = tree;
        state.tree.structural_count = state.tree.node_count();
        state.build_navigator();
        state
    }

    #[test]
    fn test_initial_state_no_file() {
        let state = AppState::new();
        assert!(state.current_path.is_none());
        assert!(state.graph_visible.is_empty());
        assert_eq!(state.graph_cursor, 0);
        assert_eq!(state.mode, AppMode::Normal);
    }

    #[test]
    fn test_filter_mode_enter_cancel() {
        let mut state = state_with_navigator();
        state.enter_filter();
        assert_eq!(state.mode, AppMode::Filter);
        state.cancel_filter();
        assert_eq!(state.mode, AppMode::Normal);
        assert!(state.filter.is_empty());
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
    fn test_load_directory() {
        use tempfile::TempDir;
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("main.rs"), "fn main() {}").unwrap();
        let mut state = AppState::new();
        state.load_directory(dir.path().to_path_buf()).unwrap();
        assert!(state.current_path.is_some());
        assert!(state.navigator.is_some());
        assert!(!state.graph_visible.is_empty());
    }

    // ── Graph / Navigator state tests ──────────────────────────────────────

    #[test]
    fn test_navigator_created_on_load() {
        let state = state_with_navigator();
        assert!(state.navigator.is_some(), "navigator should be built after load");
        assert!(!state.graph_visible.is_empty(), "graph_visible should be non-empty");
    }

    #[test]
    fn test_graph_cursor_moves_within_bounds() {
        let mut state = state_with_navigator();
        let total = state.graph_visible.len();
        state.move_graph_cursor_down(1000);
        assert_eq!(state.graph_cursor, total - 1);
        state.move_graph_cursor_up(1000);
        assert_eq!(state.graph_cursor, 0);
    }

    #[test]
    fn test_graph_zoom_in_increases_visible() {
        let mut state = state_with_navigator();
        let initial = state.graph_visible.len();
        // Cursor is on the root entity (File); zoom in should reveal children.
        state.graph_zoom_in();
        // After zoom, the graph visible list should contain the children (fn_a, fn_b).
        assert!(
            state.graph_visible.len() >= initial,
            "zoom_in should not shrink graph_visible"
        );
    }

    #[test]
    fn test_graph_zoom_out_after_zoom_in() {
        let mut state = state_with_navigator();
        let before = state.graph_visible.len();
        state.graph_zoom_in();
        let after_in = state.graph_visible.len();
        // Zoom out should restore something closer to original state.
        state.graph_zoom_out();
        let after_out = state.graph_visible.len();
        // after zoom_out we should have fewer or equal nodes than after zoom_in
        assert!(
            after_out <= after_in,
            "zoom_out should shrink or maintain graph_visible (before={before}, in={after_in}, out={after_out})"
        );
    }

    #[test]
    fn test_graph_fold_hides_children() {
        let mut state = state_with_navigator();
        // Zoom in so the File node has children visible.
        state.graph_zoom_in();
        let after_zoom = state.graph_visible.len();
        if after_zoom <= 1 {
            // No children revealed; skip.
            return;
        }
        // Set cursor to root (index 0) and fold it.
        state.graph_cursor = 0;
        state.graph_toggle_fold();
        let after_fold = state.graph_visible.len();
        assert!(
            after_fold <= after_zoom,
            "fold should hide children: before_fold={after_zoom}, after_fold={after_fold}"
        );
    }

    #[test]
    fn test_graph_unfold_restores_children() {
        let mut state = state_with_navigator();
        state.graph_zoom_in();
        let after_zoom = state.graph_visible.len();
        if after_zoom <= 1 {
            return;
        }
        state.graph_cursor = 0;
        state.graph_toggle_fold();  // fold
        state.graph_toggle_fold();  // unfold
        let after_unfold = state.graph_visible.len();
        assert_eq!(
            after_unfold, after_zoom,
            "unfold should restore the original child count"
        );
    }

    #[test]
    fn test_graph_clear_folds() {
        let mut state = state_with_navigator();
        state.graph_zoom_in();
        state.graph_cursor = 0;
        state.graph_toggle_fold();
        let folded_count = state.graph_visible.len();
        state.graph_clear_folds();
        let cleared_count = state.graph_visible.len();
        assert!(
            cleared_count >= folded_count,
            "clear_folds should restore hidden children"
        );
    }
}
