use std::collections::{HashMap, HashSet};

use crate::graph::entity::EntityId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GraphTreeNodeId(pub usize);

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
}

impl GraphTreeNode {
    pub fn new(id: GraphTreeNodeId, entity_id: EntityId, children: Vec<GraphTreeNodeId>
        ) -> Self {
        GraphTreeNode {
            id,
            entity_id,
            parent: None,
            children,
        }
    }

    pub fn is_leaf(&self) -> bool {
        self.children.is_empty()
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
            if free_root_idx < free_roots.len() {
                // Claim this free root: just wire it under the new parent.
                let root_id = free_roots[free_root_idx];
                free_root_idx += 1;
                if let Some(child_node) = self.nodes.get_mut(root_id.0) {
                    child_node.parent = Some(parent_node_id);
                }
                if let Some(parent_node) = self.nodes.get_mut(parent_node_id.0) {
                    parent_node.children.push(root_id);
                }
            } else {
                // No free roots left: deep-copy the full subtree from the first
                // existing node for this entity so every parent gets its own copy.
                let template_id = self.nodes_by_entity
                    .get(&child_entity)
                    .and_then(|ids| ids.first().copied());
                if let Some(tmpl) = template_id {
                    self.deep_copy_subtree(tmpl, Some(parent_node_id));
                } else {
                    // child_entity has no nodes at all yet; create a fresh leaf.
                    let new_id = GraphTreeNodeId(self.nodes.len());
                    self.nodes_by_entity.entry(child_entity).or_default().push(new_id);
                    self.nodes.push(GraphTreeNode::new(new_id, child_entity, vec![]));
                    if let Some(n) = self.nodes.get_mut(new_id.0) {
                        n.parent = Some(parent_node_id);
                    }
                    if let Some(p) = self.nodes.get_mut(parent_node_id.0) {
                        p.children.push(new_id);
                    }
                }
            }
        }
    }

    /// Recursively deep-copies the subtree rooted at `source_id` as a new
    /// child of `parent`, registering every new node in `nodes_by_entity`.
    fn deep_copy_subtree(
        &mut self,
        source_id: GraphTreeNodeId,
        parent: Option<GraphTreeNodeId>,
    ) -> GraphTreeNodeId {
        // Clone what we need before mutating self.
        let (entity_id, children) = {
            let src = &self.nodes[source_id.0];
            (src.entity_id, src.children.clone())
        };
        let new_id = GraphTreeNodeId(self.nodes.len());
        self.nodes_by_entity.entry(entity_id).or_default().push(new_id);
        self.nodes.push(GraphTreeNode { id: new_id, entity_id, parent, children: vec![] });
        if let Some(pid) = parent {
            if let Some(p) = self.nodes.get_mut(pid.0) {
                p.children.push(new_id);
            }
        }
        for child_id in children {
            self.deep_copy_subtree(child_id, Some(new_id));
        }
        new_id
    }
}
