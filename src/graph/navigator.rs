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
    /// 2. Build a deduplicated adjacency list from cursor references
    ///    (self-loops suppressed).
    /// 3. Walk the reference graph with a two-pass DFS spanning forest.
    ///
    /// **Why two passes?**
    ///
    /// A single DFS pass over leaves in their natural order would process
    /// target nodes as standalone roots before their source nodes reach them
    /// through the adjacency list.  For example, if `entity.rs` (target) is
    /// alphabetically before `tree.rs` (source), a naive single pass would
    /// visit `entity.rs` first and mark it visited, so the later edge
    /// `tree.rs → entity.rs` is silently dropped.
    ///
    /// The fix is to run DFS in two passes:
    ///
    /// - **Pass 1** starts only from leaves that have *no incoming edges* from
    ///   other leaves in the adjacency list (i.e., true "sources").  This lets
    ///   each source claim its targets during its own DFS before any target is
    ///   visited as a standalone root.
    ///
    /// - **Pass 2** handles any remaining unvisited leaves — nodes in cycles or
    ///   disconnected components not reachable from any source.
    ///
    /// Additional benefits of the spanning forest approach:
    ///
    /// - **No cycles in the view tree.**  Mutual references (A→B and B→A) no
    ///   longer create `NodeKind::Cycle` / `NodeKind::Ref` stubs; the user
    ///   just sees A→B without the confusing ↺ marker.
    ///
    /// - **No exponential node growth.**  Previously, inserting hundreds of
    ///   deduplicated edges between the same small set of folder-level leaves
    ///   triggered `deep_copy_subtree` for each duplicate parent, growing the
    ///   tree exponentially.  The DFS guarantees each entity is placed in the
    ///   tree exactly once.
    fn sync_tree(&mut self) {
        self.view_tree = GraphTree::new();

        // All active leaves must exist before any edges are wired.
        for &leaf in self.cursor.active() {
            self.view_tree.insert_entity(leaf, vec![]);
        }

        // Build a deduplicated adjacency list (self-loops suppressed).
        let mut adj: std::collections::HashMap<EntityId, Vec<EntityId>> =
            std::collections::HashMap::new();
        {
            let mut seen = std::collections::HashSet::new();
            for cursor_ref in &self.cursor.references {
                if cursor_ref.from_leaf != cursor_ref.to_leaf
                    && seen.insert((cursor_ref.from_leaf, cursor_ref.to_leaf))
                {
                    adj.entry(cursor_ref.from_leaf)
                        .or_default()
                        .push(cursor_ref.to_leaf);
                }
            }
        }

        // Compute the set of leaves that are targets of other leaves.
        // These nodes should not start their own DFS tree in pass 1 — they
        // should be adopted by their source nodes first.
        let mut has_incoming = std::collections::HashSet::new();
        for targets in adj.values() {
            for &t in targets {
                has_incoming.insert(t);
            }
        }

        let leaves: Vec<EntityId> = self.cursor.active().to_vec();
        let mut visited = std::collections::HashSet::new();

        // Pass 1: DFS from source leaves only (no incoming edges from other leaves).
        // This ensures each source claims its targets before they are visited
        // as standalone roots.
        for &root in &leaves {
            if visited.contains(&root) || has_incoming.contains(&root) {
                continue;
            }
            self.dfs_from(root, None, &adj, &mut visited);
        }

        // Pass 2: process any remaining unvisited leaves (cycle members or
        // nodes unreachable from any source, e.g. isolated leaves).
        for &root in &leaves {
            if visited.contains(&root) {
                continue;
            }
            self.dfs_from(root, None, &adj, &mut visited);
        }
    }

    /// Run a single DFS subtree rooted at `start`, wiring `insert_edge` for
    /// every newly-discovered node.  `parent` is the entity that "owns" `start`
    /// in the spanning tree (`None` for forest roots).
    fn dfs_from(
        &mut self,
        start: EntityId,
        parent: Option<EntityId>,
        adj: &std::collections::HashMap<EntityId, Vec<EntityId>>,
        visited: &mut std::collections::HashSet<EntityId>,
    ) {
        let mut stack: Vec<(EntityId, Option<EntityId>)> = vec![(start, parent)];
        while let Some((node, par)) = stack.pop() {
            if !visited.insert(node) {
                continue;
            }
            if let Some(p) = par {
                self.view_tree.insert_edge(p, node);
            }
            if let Some(children) = adj.get(&node) {
                // Push in reverse so the first adjacency-list child is
                // processed first (LIFO stack, so last-pushed = first-popped).
                for &child in children.iter().rev() {
                    if !visited.contains(&child) {
                        stack.push((child, Some(node)));
                    }
                }
            }
        }
    }
}
