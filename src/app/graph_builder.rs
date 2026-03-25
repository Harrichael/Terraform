//! Converts the parsed [`CodeTree`] into a [`EntityGraph`] suitable for the
//! [`Navigator`] core engine.
//!
//! The [`CodeTree`] produced by the parser contains all granularity levels
//! (Folder → Module → File → Class → Function → Block → Line) plus virtual
//! SymRef nodes.  The [`EntityGraph`] only models the coarser levels
//! (Folder → Module → File → Class → Function); Block, Line, and SymRef nodes
//! are filtered out and their reference edges are remapped to the nearest
//! function-or-coarser ancestor.

use std::collections::HashMap;

use crate::app::tree::{CodeTree, NodeKind, ReferenceKind};
use crate::graph::entity::{
    Entity, EntityGraph, EntityId, EntityKind,
    Reference as GraphReference, ReferenceKind as GraphReferenceKind,
};

/// Build an [`EntityGraph`] from a fully-parsed [`CodeTree`].
///
/// Only structural nodes up to [`NodeKind::Function`] are translated;
/// Block, Line, and SymRef nodes are skipped.  References that originate
/// from or target finer-grained nodes are remapped upward to the nearest
/// entity-level ancestor so no symbolic edges are lost.
pub fn code_tree_to_entity_graph(tree: &CodeTree) -> EntityGraph {
    let is_entity_kind = |k: &NodeKind| {
        matches!(
            k,
            NodeKind::Folder
                | NodeKind::Module
                | NodeKind::File
                | NodeKind::Class
                | NodeKind::Function
        )
    };

    // ------------------------------------------------------------------
    // Pass 1: Create entity stubs for every qualifying structural node.
    // We keep a mapping code_node_id → EntityId for use in later passes.
    // ------------------------------------------------------------------
    let structural_count = tree.structural_count;
    let mut id_map: HashMap<usize, EntityId> = HashMap::new();
    let mut entities: Vec<Entity> = Vec::new();

    for code_id in 0..structural_count {
        if let Some(node) = tree.get(code_id) {
            if !is_entity_kind(&node.kind) {
                continue;
            }
            let entity_id = EntityId(entities.len());
            id_map.insert(code_id, entity_id);

            let kind = match node.kind {
                NodeKind::Folder => EntityKind::Folder,
                NodeKind::Module => EntityKind::Module,
                NodeKind::File => EntityKind::File,
                NodeKind::Class => EntityKind::Class,
                NodeKind::Function => EntityKind::Function,
                _ => unreachable!(),
            };

            // Build a best-effort path from the node name; the path field is
            // informational and not used by any graph algorithm.
            let path = std::path::PathBuf::from(&node.name);

            entities.push(Entity {
                id: entity_id,
                kind,
                name: node.name.clone(),
                parent: None,       // filled in pass 2
                children: Vec::new(), // filled in pass 2
                path,
                byte_range: node.byte_range.0..node.byte_range.1,
                line_range: node.line_range.0..node.line_range.1,
            });
        }
    }

    // ------------------------------------------------------------------
    // Pass 2: Wire parent / children relationships.
    // For a node whose direct parent is not in the entity graph (e.g. a
    // Function whose parent is a Block), we walk up until we find an
    // entity-level ancestor.
    // ------------------------------------------------------------------
    for code_id in 0..structural_count {
        if let Some(&entity_id) = id_map.get(&code_id) {
            if let Some(node) = tree.get(code_id) {
                // Parent: walk up to find nearest entity ancestor.
                let parent_entity_id =
                    node.parent.and_then(|p| find_entity_ancestor(p, tree, &id_map));
                entities[entity_id.0].parent = parent_entity_id;

                // Children: direct entity children, or nearest entity descendants
                // when an intermediate Block/Line node is present.
                let children = collect_entity_children(code_id, tree, &id_map);
                entities[entity_id.0].children = children;
            }
        }
    }

    // ------------------------------------------------------------------
    // Pass 3: Convert reference edges.
    // Remap both endpoints to their nearest entity-level ancestor so that
    // references inside Block/Line nodes still appear at the Function level.
    // Self-loops produced by remapping are discarded.
    // ------------------------------------------------------------------
    let mut seen: std::collections::HashSet<(EntityId, EntityId)> = std::collections::HashSet::new();
    let mut references: Vec<GraphReference> = Vec::new();

    for r in tree.references.references() {
        let from = match find_entity_ancestor_or_self(r.from, tree, &id_map) {
            Some(id) => id,
            None => continue,
        };
        let to = match find_entity_ancestor_or_self(r.to, tree, &id_map) {
            Some(id) => id,
            None => continue,
        };
        if from == to {
            continue; // skip self-loops produced by remapping
        }
        let key = (from, to);
        if seen.contains(&key) {
            continue; // deduplicate
        }
        seen.insert(key);

        let kind = match r.kind {
            ReferenceKind::Call => GraphReferenceKind::Call,
            ReferenceKind::Import => GraphReferenceKind::Import,
            ReferenceKind::TypeRef => GraphReferenceKind::TypeRef,
            ReferenceKind::VarRef => GraphReferenceKind::VarRef,
            ReferenceKind::Generic => GraphReferenceKind::Generic,
        };
        references.push(GraphReference { from, to, kind });
    }

    EntityGraph { entities, references }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Walk up the containment hierarchy from `code_id` (exclusive) until we
/// reach a node that has an [`EntityId`] mapping.
fn find_entity_ancestor(
    mut code_id: usize,
    tree: &CodeTree,
    id_map: &HashMap<usize, EntityId>,
) -> Option<EntityId> {
    loop {
        if let Some(&eid) = id_map.get(&code_id) {
            return Some(eid);
        }
        match tree.get(code_id).and_then(|n| n.parent) {
            Some(parent) => code_id = parent,
            None => return None,
        }
    }
}

/// Same as [`find_entity_ancestor`] but also accepts `code_id` itself.
fn find_entity_ancestor_or_self(
    code_id: usize,
    tree: &CodeTree,
    id_map: &HashMap<usize, EntityId>,
) -> Option<EntityId> {
    if let Some(&eid) = id_map.get(&code_id) {
        return Some(eid);
    }
    find_entity_ancestor(code_id, tree, id_map)
}

/// Collect the entity-level children of `code_id`.
///
/// For children that are not entity nodes themselves (Block, Line), descend
/// recursively until entity-level descendants are found.
fn collect_entity_children(
    code_id: usize,
    tree: &CodeTree,
    id_map: &HashMap<usize, EntityId>,
) -> Vec<EntityId> {
    let Some(node) = tree.get(code_id) else {
        return Vec::new();
    };
    let mut result = Vec::new();
    for &child_code_id in &node.children {
        if let Some(&child_entity_id) = id_map.get(&child_code_id) {
            result.push(child_entity_id);
        } else {
            // Non-entity child (Block/Line): recurse into its descendants.
            result.extend(collect_entity_children(child_code_id, tree, id_map));
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::tree::{CodeTree, NodeKind, ReferenceKind};
    use crate::graph::entity::EntityKind;

    fn two_file_tree() -> CodeTree {
        let mut tree = CodeTree::new();
        let root = tree.add_node(NodeKind::Folder, "root", (0, 200), (0, 40), 0, None);
        let fa = tree.add_node(NodeKind::File, "a.rs", (0, 100), (0, 20), 1, Some(root));
        let _fn_a = tree.add_node(NodeKind::Function, "fn_a", (0, 50), (0, 10), 2, Some(fa));
        let fb = tree.add_node(NodeKind::File, "b.rs", (101, 200), (21, 40), 1, Some(root));
        let _fn_b = tree.add_node(NodeKind::Function, "fn_b", (101, 150), (21, 30), 2, Some(fb));
        tree.structural_count = tree.node_count();
        tree
    }

    #[test]
    fn test_entity_count_matches_entity_kinds() {
        let tree = two_file_tree();
        let graph = code_tree_to_entity_graph(&tree);
        // root(Folder) + a.rs(File) + fn_a(Function) + b.rs(File) + fn_b(Function) = 5
        assert_eq!(graph.entities.len(), 5);
    }

    #[test]
    fn test_block_and_line_nodes_excluded() {
        let mut tree = CodeTree::new();
        let root = tree.add_node(NodeKind::File, "f.rs", (0, 100), (0, 10), 0, None);
        let func = tree.add_node(NodeKind::Function, "fn_x", (0, 50), (0, 5), 1, Some(root));
        let _blk = tree.add_node(NodeKind::Block, "{}", (0, 30), (0, 3), 2, Some(func));
        let _ln = tree.add_node(NodeKind::Line, "let x = 1;", (0, 20), (0, 2), 3, Some(func));
        tree.structural_count = tree.node_count();
        let graph = code_tree_to_entity_graph(&tree);
        // Only File + Function should appear
        assert_eq!(graph.entities.len(), 2);
        assert!(graph.entities.iter().all(|e| matches!(
            e.kind,
            EntityKind::File | EntityKind::Function
        )));
    }

    #[test]
    fn test_parent_child_wiring() {
        let tree = two_file_tree();
        let graph = code_tree_to_entity_graph(&tree);
        // Find the folder (root)
        let folder = graph.entities.iter().find(|e| e.kind == EntityKind::Folder).unwrap();
        assert_eq!(folder.children.len(), 2, "folder should have 2 file children");
        assert!(folder.parent.is_none());
    }

    #[test]
    fn test_references_converted() {
        let mut tree = two_file_tree();
        // fn_a (id=2) calls fn_b (id=4)
        tree.add_reference(2, 4, ReferenceKind::Call);
        let graph = code_tree_to_entity_graph(&tree);
        assert_eq!(graph.references.len(), 1);
        assert!(graph.references[0].from != graph.references[0].to);
    }

    #[test]
    fn test_references_deduplicated() {
        let mut tree = two_file_tree();
        tree.add_reference(2, 4, ReferenceKind::Call);
        tree.add_reference(2, 4, ReferenceKind::Import);
        let graph = code_tree_to_entity_graph(&tree);
        // Deduplication by (from, to) pair keeps only one edge
        assert_eq!(graph.references.len(), 1);
    }

    #[test]
    fn test_empty_tree_gives_empty_graph() {
        let mut tree = CodeTree::new();
        tree.structural_count = 0;
        let graph = code_tree_to_entity_graph(&tree);
        assert!(graph.entities.is_empty());
        assert!(graph.references.is_empty());
    }
}
