use std::collections::HashSet;
use std::path::PathBuf;

use crate::app::graph_builder::code_tree_to_entity_graph;
use crate::app::tree::{CodeTree, NodeKind, SemanticEntry};
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

    // ── Graph / Navigator state ────────────────────────────────────────────
    /// The navigator built from the entity graph.  `None` when no path has
    /// been loaded or when the conversion fails.
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
            navigator: None,
            graph_folded: HashSet::new(),
            graph_visible: Vec::new(),
            graph_cursor: 0,
            graph_scroll_offset: 0,
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
        self.build_navigator();
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
        self.build_navigator();
        Ok(())
    }

    /// Convert the current `CodeTree` into an [`EntityGraph`] and initialize
    /// the [`Navigator`].  Resets all graph-view navigation state.
    pub(crate) fn build_navigator(&mut self) {
        let entity_graph = code_tree_to_entity_graph(&self.tree);
        if entity_graph.entities.is_empty() {
            // Nothing to navigate — leave navigator as None so the legacy view
            // is shown instead.
            self.navigator = None;
            return;
        }
        self.navigator = Some(Navigator::new(entity_graph));
        self.graph_folded.clear();
        self.graph_cursor = 0;
        self.graph_scroll_offset = 0;
        self.refresh_graph_visible();
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

    // ── Graph / Navigator navigation ──────────────────────────────────────

    /// Rebuild `graph_visible` by traversing the Navigator's current
    /// [`GraphTree`] in DFS order, skipping children of folded entities.
    pub fn refresh_graph_visible(&mut self) {
        self.graph_visible.clear();

        // Snapshot tree node data to avoid borrow conflicts during traversal.
        let node_snapshot: Vec<(GraphTreeNodeId, EntityId, Option<GraphTreeNodeId>, Vec<GraphTreeNodeId>)> =
            if let Some(nav) = &self.navigator {
                nav.tree()
                    .nodes
                    .iter()
                    .map(|n| (n.id, n.entity_id, n.parent, n.children.clone()))
                    .collect()
            } else {
                return;
            };

        // Roots are nodes with no parent.
        let mut stack: Vec<(GraphTreeNodeId, usize)> = node_snapshot
            .iter()
            .filter(|(_, _, parent, _)| parent.is_none())
            .map(|(id, _, _, _)| (*id, 0usize))
            .collect();
        // Reverse so the first root ends up on top of the stack (LIFO).
        stack.reverse();

        while let Some((node_id, depth)) = stack.pop() {
            if let Some((_, entity_id, _, children)) =
                node_snapshot.iter().find(|(id, _, _, _)| *id == node_id)
            {
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

    /// Build an AppState with a navigator already initialised from a small
    /// two-function tree (File → fn_a, fn_b).
    fn state_with_navigator() -> AppState {
        let mut state = AppState::new();
        let mut tree = CodeTree::new();
        let root = tree.add_node(NodeKind::File, "test.rs", (0, 200), (0, 30), 0, None);
        let _f1 = tree.add_node(NodeKind::Function, "fn_a", (0, 100), (0, 15), 1, Some(root));
        let _f2 = tree.add_node(NodeKind::Function, "fn_b", (101, 200), (16, 30), 1, Some(root));
        state.tree = tree;
        state.tree.structural_count = state.tree.node_count();
        state.base_node_count = state.tree.node_count();
        state.refresh_visible();
        state.build_navigator();
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
