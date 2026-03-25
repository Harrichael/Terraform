use std::collections::{HashMap, HashSet};

use crate::graph::entity::EntityId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GraphTreeNodeId(pub usize);

/// Whether this node closes a cycle in the underlying entity graph.
///
/// Analogous to a filesystem symlink: a `Cycle` node displays the entity and
/// its direct graph neighbours as un-expanded stubs, but does not recurse
/// further so the tree remains finite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeKind {
    /// Regular node; its full subtree is expanded.
    Normal,
    /// This node's entity already appears on the path from the tree root to
    /// here.  Its children are the entity's direct neighbours shown as
    /// leaf stubs only — no further recursion.
    Cycle,
    /// A leaf reference inside a Cycle node.  Marks where the back-edge points
    /// without expanding the subtree further.  Deep copies stop at Ref nodes.
    Ref,
}

/// A node in the tree view of the entity graph.
///
/// Each tree node represents an entity as it appears in the reference tree
/// at a particular depth. The tree structure is derived from reference relationships,
/// not containment.
#[derive(Debug, Clone)]
pub struct GraphTreeNode {
    /// Unique identifier for this tree node (index into the tree's arena).
    pub id: GraphTreeNodeId,
    /// The entity from the graph this tree node represents.
    pub entity_id: EntityId,

    // TREE STRUCTURE
    pub parent: Option<GraphTreeNodeId>,
    pub children: Vec<GraphTreeNodeId>,
    pub kind: NodeKind,
}

impl GraphTreeNode {
    pub fn new(id: GraphTreeNodeId, entity_id: EntityId, children: Vec<GraphTreeNodeId>
        ) -> Self {
        GraphTreeNode {
            id,
            entity_id,
            parent: None,
            children,
            kind: NodeKind::Normal,
        }
    }

    pub fn is_leaf(&self) -> bool {
        self.children.is_empty()
    }

    pub fn is_cycle(&self) -> bool {
        self.kind == NodeKind::Cycle
    }

    pub fn is_ref(&self) -> bool {
        self.kind == NodeKind::Ref
    }
}

/// A tree view of the entity graph.
///
/// This represents entities arranged in a tree structure derived from their
/// symbolic reference relationships, at a specific granularity level.
/// Finer-grained entities are aggregated up to that level.
pub struct GraphTree {
    /// Arena-allocated tree nodes; index == tree_node.id
    pub nodes: Vec<GraphTreeNode>,
    pub nodes_by_entity: HashMap<EntityId, Vec<GraphTreeNodeId>>,
}

impl GraphTree {
    pub fn new() -> Self {
        GraphTree {
            nodes: Vec::new(),
            nodes_by_entity: HashMap::new(),
        }
    }

    pub fn get(&self, id: GraphTreeNodeId) -> Option<&GraphTreeNode> {
        self.nodes.get(id.0)
    }

    // All entity IDs need to be inserted before their edge is added.
    pub fn insert_entity(&mut self, entity_id: EntityId,
        children: Vec<GraphTreeNodeId>
        ) -> GraphTreeNodeId {
        let id = GraphTreeNodeId(self.nodes.len());
        self.nodes_by_entity.entry(entity_id).or_default().push(id);
        self.nodes.push(GraphTreeNode::new(id, entity_id, children));
        id
    }

    // As the cursor moves, we may need to remove entities from the tree.
    // Should be called BEFORE new edges are added
    pub fn remove_entity(&mut self, entity_id: EntityId) {
        if let Some(node_ids) = self.nodes_by_entity.remove(&entity_id) {
            let parent_ids_set = node_ids
                .iter()
                .filter_map(|&node_id| self.nodes.get(node_id.0))
                .filter_map(|node| node.parent)
                .collect::<HashSet<_>>();

            let child_ids_set = node_ids
                .iter()
                .filter_map(|&node_id| self.nodes.get(node_id.0))
                .flat_map(|node| node.children.iter().copied())
                .collect::<HashSet<_>>();

            // Removal of parent lists
            for parent_id in parent_ids_set {
                if let Some(parent_node) = self.nodes.get_mut(parent_id.0) {
                    parent_node.children.retain(|child_id| !node_ids.contains(child_id));
                }
            }

            // Removal of child lists
            for child_id in child_ids_set {
                if let Some(child_node) = self.nodes.get_mut(child_id.0) {
                    if let Some(parent_id) = child_node.parent {
                        if node_ids.contains(&parent_id) {
                            child_node.parent = None;
                        }
                    }
                }
            }

            // Finally, remove the nodes themselves
            self.nodes.retain(|node| !node_ids.contains(&node.id));
        }
    }

    pub fn insert_edge(&mut self, parent_entity: EntityId, child_entity: EntityId) {
        // Collect owned so we can mutably borrow self in the loop.
        let parent_node_ids: Vec<GraphTreeNodeId> = self.nodes_by_entity
            .get(&parent_entity)
            .cloned()
            .unwrap_or_default();

        // Free root nodes for child_entity: nodes that have no parent yet.
        // We claim these one-to-one for each parent occurrence before falling
        // back to deep copies.
        let free_roots: Vec<GraphTreeNodeId> = self.nodes_by_entity
            .get(&child_entity)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|&id| self.nodes.get(id.0).map_or(false, |n| n.parent.is_none()))
            .collect();

        let mut free_root_idx = 0;

        for parent_node_id in parent_node_ids {
            let ancestor_path = self.build_ancestor_path(parent_node_id);

            if ancestor_path.contains(&child_entity) {
                // Back-edge P→C detected: C is already an ancestor of P.
                //
                // Cut the cycle:
                //  1. Detach P from its current parent; add Cycle(P) there.
                //  2. Find C's canonical (Normal) tree node and use its parent
                //     as P's new home — the "closest common parent" of P and C.
                //  3. Add Cycle(C) under P (the back-edge label).
                let c_entity = self.nodes[parent_node_id.0].entity_id;

                // Step 1 — detach P and stamp Cycle(P) in its old slot.
                if let Some(old_parent_id) = self.nodes[parent_node_id.0].parent {
                    if let Some(op) = self.nodes.get_mut(old_parent_id.0) {
                        op.children.retain(|&id| id != parent_node_id);
                    }
                    if let Some(n) = self.nodes.get_mut(parent_node_id.0) {
                        n.parent = None;
                    }
                    self.add_cycle_node(c_entity, old_parent_id);
                }

                // Step 2 — find the placement parent: the parent of C's Normal node.
                let target_parent = self.nodes_by_entity
                    .get(&child_entity)
                    .and_then(|ids| ids.iter().find(|&&id| {
                        self.nodes.get(id.0).map_or(false, |n| n.kind == NodeKind::Normal)
                    }))
                    .and_then(|&id| self.nodes.get(id.0))
                    .and_then(|n| n.parent);

                if let Some(tp) = target_parent {
                    // Re-parent P under C's parent.
                    if let Some(n) = self.nodes.get_mut(parent_node_id.0) {
                        n.parent = Some(tp);
                    }
                    if let Some(pp) = self.nodes.get_mut(tp.0) {
                        pp.children.push(parent_node_id);
                    }
                }
                // If target_parent is None, C is a root so P stays as a root too.

                // Step 3 — label the back-edge on P.
                self.add_cycle_node(child_entity, parent_node_id);
            } else if free_root_idx < free_roots.len() {
                // No cycle and a free root is available: claim it directly.
                let root_id = free_roots[free_root_idx];
                free_root_idx += 1;
                if let Some(child_node) = self.nodes.get_mut(root_id.0) {
                    child_node.parent = Some(parent_node_id);
                }
                if let Some(parent_node) = self.nodes.get_mut(parent_node_id.0) {
                    parent_node.children.push(root_id);
                }
            } else {
                // No cycle, no free roots: deep-copy an existing subtree.
                let template_id = self.nodes_by_entity
                    .get(&child_entity)
                    .and_then(|ids| ids.first().copied());
                if let Some(tmpl) = template_id {
                    self.deep_copy_subtree(tmpl, Some(parent_node_id), &ancestor_path);
                } else {
                    let new_id = GraphTreeNodeId(self.nodes.len());
                    self.nodes_by_entity.entry(child_entity).or_default().push(new_id);
                    self.nodes.push(GraphTreeNode {
                        id: new_id, entity_id: child_entity,
                        parent: Some(parent_node_id), children: vec![],
                        kind: NodeKind::Normal,
                    });
                    if let Some(p) = self.nodes.get_mut(parent_node_id.0) {
                        p.children.push(new_id);
                    }
                }
            }
        }
    }

    /// Add a `Cycle` node for `entity_id` as a child of `parent_node_id`, with
    /// a single `Ref` leaf child inside it.  The Ref is the terminus — deep
    /// copies and canonical rendering stop here.
    fn add_cycle_node(&mut self, entity_id: EntityId, parent_node_id: GraphTreeNodeId) {
        let cycle_id = GraphTreeNodeId(self.nodes.len());
        self.nodes_by_entity.entry(entity_id).or_default().push(cycle_id);
        self.nodes.push(GraphTreeNode {
            id: cycle_id, entity_id, parent: Some(parent_node_id), children: vec![],
            kind: NodeKind::Cycle,
        });
        if let Some(p) = self.nodes.get_mut(parent_node_id.0) {
            p.children.push(cycle_id);
        }
        self.add_ref_leaf(entity_id, cycle_id);
    }

    /// Add a bare `Ref` leaf for `entity_id` as a child of `parent_node_id`.
    fn add_ref_leaf(&mut self, entity_id: EntityId, parent_node_id: GraphTreeNodeId) {
        let id = GraphTreeNodeId(self.nodes.len());
        self.nodes_by_entity.entry(entity_id).or_default().push(id);
        self.nodes.push(GraphTreeNode {
            id, entity_id, parent: Some(parent_node_id), children: vec![],
            kind: NodeKind::Ref,
        });
        if let Some(p) = self.nodes.get_mut(parent_node_id.0) {
            p.children.push(id);
        }
    }

    /// Walk from `node_id` up to the tree root and collect every entity id on
    /// that path (inclusive).  Used to detect back-edges that would form cycles.
    fn build_ancestor_path(&self, node_id: GraphTreeNodeId) -> HashSet<EntityId> {
        let mut path = HashSet::new();
        let mut current = Some(node_id);
        while let Some(id) = current {
            match self.nodes.get(id.0) {
                Some(node) => { path.insert(node.entity_id); current = node.parent; }
                None => break,
            }
        }
        path
    }

    /// Recursively deep-copies the subtree rooted at `source_id` as a new
    /// child of `parent`, registering every new node in `nodes_by_entity`.
    ///
    /// `ancestor_path` is the set of entity ids from the tree root down to
    /// (and including) `parent`.  If `source_id`'s entity is already in that
    /// set the new node is marked `NodeKind::Cycle`: its immediate graph
    /// neighbours are added as un-expanded leaf stubs, then recursion stops.
    fn deep_copy_subtree(
        &mut self,
        source_id: GraphTreeNodeId,
        parent: Option<GraphTreeNodeId>,
        ancestor_path: &HashSet<EntityId>,
    ) -> GraphTreeNodeId {
        let (entity_id, src_kind, children) = {
            let src = &self.nodes[source_id.0];
            (src.entity_id, src.kind.clone(), src.children.clone())
        };

        // Ref nodes are always terminal — copy as a fresh Ref leaf.
        if src_kind == NodeKind::Ref {
            return self.add_ref_leaf_returning(entity_id, parent);
        }

        let is_cycle = ancestor_path.contains(&entity_id) || src_kind == NodeKind::Cycle;

        let new_id = GraphTreeNodeId(self.nodes.len());
        self.nodes_by_entity.entry(entity_id).or_default().push(new_id);
        self.nodes.push(GraphTreeNode {
            id: new_id, entity_id, parent,
            children: vec![],
            kind: if is_cycle { NodeKind::Cycle } else { NodeKind::Normal },
        });
        if let Some(pid) = parent {
            if let Some(p) = self.nodes.get_mut(pid.0) {
                p.children.push(new_id);
            }
        }

        if is_cycle {
            // Emit a Ref child and stop — never recurse past a Cycle node.
            self.add_ref_leaf(entity_id, new_id);
            return new_id;
        }

        let mut new_path = ancestor_path.clone();
        new_path.insert(entity_id);
        // Skip Ref children when copying (they're internal to Cycle nodes).
        for child_id in children {
            if self.nodes.get(child_id.0).map_or(false, |n| n.kind == NodeKind::Ref) {
                continue;
            }
            self.deep_copy_subtree(child_id, Some(new_id), &new_path);
        }
        new_id
    }

    /// Like `add_ref_leaf` but also wires into parent and returns the new id.
    fn add_ref_leaf_returning(
        &mut self,
        entity_id: EntityId,
        parent: Option<GraphTreeNodeId>,
    ) -> GraphTreeNodeId {
        let id = GraphTreeNodeId(self.nodes.len());
        self.nodes_by_entity.entry(entity_id).or_default().push(id);
        self.nodes.push(GraphTreeNode {
            id, entity_id, parent, children: vec![], kind: NodeKind::Ref,
        });
        if let Some(pid) = parent {
            if let Some(p) = self.nodes.get_mut(pid.0) {
                p.children.push(id);
            }
        }
        id
    }
}
