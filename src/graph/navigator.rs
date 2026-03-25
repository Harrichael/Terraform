use crate::graph::cursor::Cursor;
use crate::graph::entity::{Entity, EntityGraph, EntityId};
use crate::graph::tree::{GraphTree, GraphTreeNodeId};

/// The central controller that wraps the static entity graph and the dynamic
/// cursor, projecting the cursor's current leaf set into a renderable
/// [`GraphTree`].
///
/// The Navigator owns three things:
/// - `graph`     — the immutable entity/reference database.
/// - `cursor`    — the mutable navigation state (which leaves are active).
/// - `view_tree` — the derived, renderable tree; always in sync with `cursor`.
pub struct Navigator {
    graph: EntityGraph,
    cursor: Cursor,
    view_tree: GraphTree,
}

impl Navigator {
    /// Build a navigator starting at the root level of `graph`.
    pub fn new(graph: EntityGraph) -> Self {
        let cursor = Cursor::new(&graph);
        let mut navigator = Self {
            graph,
            cursor,
            view_tree: GraphTree::new(),
        };
        navigator.sync_tree();
        navigator
    }

    // --- PUBLIC API FOR THE UI ---

    pub fn tree(&self) -> &GraphTree {
        &self.view_tree
    }

    /// Look up an entity from the graph by its ID.
    ///
    /// Used by the UI layer to retrieve entity metadata (name, kind, etc.)
    /// for rendering tree nodes without exposing the graph directly.
    pub fn entity(&self, id: EntityId) -> Option<&Entity> {
        self.graph.get(id)
    }

    /// Expand the entity represented by `focused_tree_node` one level down.
    ///
    /// Returns the [`EntityId`] of the first revealed child so the UI can
    /// move its highlight there, or `None` if the entity has no children.
    pub fn zoom_in(&mut self, focused_tree_node: GraphTreeNodeId) -> Option<EntityId> {
        let entity_id = self.view_tree.get(focused_tree_node)?.entity_id;
        if self.cursor.move_down(entity_id, &self.graph) {
            self.sync_tree();
            self.graph.get(entity_id)?.children.first().copied()
        } else {
            None
        }
    }

    /// Collapse the entity represented by `focused_tree_node` one level up.
    ///
    /// Returns the [`EntityId`] of the parent so the UI can move its
    /// highlight there, or `None` if the entity has no parent (already root).
    pub fn zoom_out(&mut self, focused_tree_node: GraphTreeNodeId) -> Option<EntityId> {
        let entity_id = self.view_tree.get(focused_tree_node)?.entity_id;
        if self.cursor.move_up(entity_id, &self.graph) {
            self.sync_tree();
            self.graph.get(entity_id)?.parent
        } else {
            None
        }
    }

    // --- PRIVATE SYNCHRONIZATION LOGIC ---

    /// Rebuild `view_tree` from the cursor's current state.
    ///
    /// Algorithm:
    /// 1. Register every active leaf as a `GraphTree` node.
    /// 2. For each symbolic reference between *distinct* leaves, insert a tree
    ///    edge via [`GraphTree::insert_edge`], which handles cycle detection and
    ///    `NodeKind::Cycle` / `NodeKind::Ref` marking automatically.
    fn sync_tree(&mut self) {
        self.view_tree = GraphTree::new();

        // All active leaves must exist before any edges are wired.
        for &leaf in self.cursor.active() {
            self.view_tree.insert_entity(leaf, vec![]);
        }

        // Many cursor references may collapse to the same (from_leaf, to_leaf) pair at the
        // current zoom level (e.g. hundreds of function-to-function edges all map to the same
        // pair of folder leaves).  Inserting duplicates is both redundant and triggers
        // deep_copy_subtree for every extra copy, causing exponential tree growth.
        // Deduplicate here so each unique pair is inserted exactly once.
        let mut seen = std::collections::HashSet::new();
        for cursor_ref in &self.cursor.references {
            if cursor_ref.from_leaf != cursor_ref.to_leaf
                && seen.insert((cursor_ref.from_leaf, cursor_ref.to_leaf))
            {
                self.view_tree.insert_edge(cursor_ref.from_leaf, cursor_ref.to_leaf);
            }
        }
    }
}
