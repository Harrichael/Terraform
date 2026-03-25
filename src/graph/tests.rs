use std::ops::Range;
use std::path::PathBuf;

use super::entity::{Entity, EntityGraph, EntityId, EntityKind, Reference, ReferenceKind, ReferenceId};
use super::cursor::Cursor;
use super::tree::{GraphTree, GraphTreeNodeId, NodeKind};
// use super::tree::ReferenceTree;
// use super::navigator::Navigator;

fn make_entity(id: usize, name: &str, kind: EntityKind, parent: Option<EntityId>) -> Entity {
    Entity {
        id: EntityId(id),
        kind,
        name: name.to_string(),
        parent,
        children: Vec::new(),
        path: PathBuf::from(format!("{}.rs", name)),
        byte_range: Range { start: 0, end: 0 },
        line_range: Range { start: 0, end: 0 },
    }
}

#[test]
fn test_entity_graph_get() {
    let mut graph = EntityGraph {
        entities: vec![
            make_entity(0, "root", EntityKind::Folder, None),
            make_entity(1, "child", EntityKind::Module, Some(EntityId(0))),
        ],
        references: Vec::new(),
    };

    graph.entities[0].children.push(EntityId(1));

    assert!(graph.get(EntityId(0)).is_some());
    assert!(graph.get(EntityId(1)).is_some());
    assert!(graph.get(EntityId(999)).is_none());
}

#[test]
fn test_cursor_initialization() {
    let mut graph = EntityGraph {
        entities: vec![
            make_entity(0, "root", EntityKind::Folder, None),
            make_entity(1, "child", EntityKind::Module, Some(EntityId(0))),
        ],
        references: Vec::new(),
    };

    graph.entities[0].children.push(EntityId(1));

    let cursor = Cursor::new(&graph);

    assert_eq!(cursor.active(), &[EntityId(0)]);
    assert_eq!(cursor.references.len(), 0);
}

#[test]
fn test_cursor_with_references() {
    let mut graph = EntityGraph {
        entities: vec![
            make_entity(0, "root", EntityKind::Folder, None),
            make_entity(1, "module_a", EntityKind::Module, Some(EntityId(0))),
            make_entity(2, "module_b", EntityKind::Module, Some(EntityId(0))),
        ],
        references: vec![Reference {
            from: EntityId(1),
            to: EntityId(2),
            kind: ReferenceKind::Call,
        }],
    };

    graph.entities[0].children = vec![EntityId(1), EntityId(2)];

    let cursor = Cursor::new(&graph);

    assert_eq!(cursor.references.len(), 1);
    assert_eq!(cursor.references[0].reference_id, ReferenceId(0));
}

#[test]
fn test_cursor_move_down() {
    let mut graph = EntityGraph {
        entities: vec![
            make_entity(0, "root", EntityKind::Folder, None),
            make_entity(1, "module", EntityKind::Module, Some(EntityId(0))),
            make_entity(2, "class_a", EntityKind::Class, Some(EntityId(1))),
            make_entity(3, "class_b", EntityKind::Class, Some(EntityId(1))),
        ],
        references: Vec::new(),
    };

    graph.entities[0].children = vec![EntityId(1)];
    graph.entities[1].children = vec![EntityId(2), EntityId(3)];

    let mut cursor = Cursor::new(&graph);
    let result = cursor.move_down(EntityId(0), &graph);

    assert!(result);
    assert_eq!(cursor.active().len(), 1);
    assert_eq!(cursor.active()[0], EntityId(1));
}

#[test]
fn test_cursor_move_down_no_children() {
    let graph = EntityGraph {
        entities: vec![make_entity(0, "leaf", EntityKind::Function, None)],
        references: Vec::new(),
    };

    let mut cursor = Cursor::new(&graph);
    let result = cursor.move_down(EntityId(0), &graph);

    assert!(!result);
    assert_eq!(cursor.active(), &[EntityId(0)]);
}

#[test]
fn test_cursor_move_up() {
    let mut graph = EntityGraph {
        entities: vec![
            make_entity(0, "root", EntityKind::Folder, None),
            make_entity(1, "module", EntityKind::Module, Some(EntityId(0))),
            make_entity(2, "class_a", EntityKind::Class, Some(EntityId(1))),
            make_entity(3, "class_b", EntityKind::Class, Some(EntityId(1))),
        ],
        references: Vec::new(),
    };

    graph.entities[0].children = vec![EntityId(1)];
    graph.entities[1].children = vec![EntityId(2), EntityId(3)];

    let mut cursor = Cursor::new(&graph);
    cursor.move_down(EntityId(0), &graph);
    cursor.move_down(EntityId(1), &graph);

    assert_eq!(cursor.active().len(), 2);

    let result = cursor.move_up(EntityId(2), &graph);

    assert!(result);
    assert_eq!(cursor.active(), &[EntityId(1)]);
}

#[test]
fn test_cursor_move_up_at_root() {
    let graph = EntityGraph {
        entities: vec![make_entity(0, "root", EntityKind::Folder, None)],
        references: Vec::new(),
    };

    let mut cursor = Cursor::new(&graph);
    let result = cursor.move_up(EntityId(0), &graph);

    assert!(!result);
    assert_eq!(cursor.active(), &[EntityId(0)]);
}

// #[test]
// fn test_reference_tree_add_node() {
//     let mut tree = ReferenceTree::new();
//
//     let id1 = tree.add_node(EntityId(10), 0, None);
//     let id2 = tree.add_node(EntityId(20), 1, Some(id1));
//
//     assert_eq!(id1, 0);
//     assert_eq!(id2, 1);
//     assert_eq!(tree.root, Some(0));
//     assert_eq!(tree.nodes.len(), 2);
//
//     let node1 = tree.get(id1).unwrap();
//     assert_eq!(node1.entity_id, EntityId(10));
//     assert_eq!(node1.depth, 0);
//     assert_eq!(node1.children, vec![1]);
//
//     let node2 = tree.get(id2).unwrap();
//     assert_eq!(node2.entity_id, EntityId(20));
//     assert_eq!(node2.parent, Some(id1));
// }

// #[test]
// fn test_tree_node_is_leaf() {
//     let mut tree = ReferenceTree::new();
//
//     let id1 = tree.add_node(EntityId(10), 0, None);
//     let _id2 = tree.add_node(EntityId(20), 1, Some(id1));
//
//     let node1 = tree.get(id1).unwrap();
//     assert!(!node1.is_leaf());
//
//     let node2 = tree.get(1).unwrap();
//     assert!(node2.is_leaf());
// }

// #[test]
// fn test_navigator_creation() {
//     let mut graph = EntityGraph {
//         entities: vec![make_entity(0, "root", EntityKind::Folder, None)],
//         references: Vec::new(),
//     };
//
//     graph.entities[0].children = Vec::new();
//
//     let navigator = Navigator::new(graph);
//
//     assert!(navigator.tree().root.is_some());
// }

// #[test]
// fn test_navigator_zoom_in_zoom_out() {
//     let mut graph = EntityGraph {
//         entities: vec![
//             make_entity(0, "root", EntityKind::Folder, None),
//             make_entity(1, "module", EntityKind::Module, Some(EntityId(0))),
//             make_entity(2, "class", EntityKind::Class, Some(EntityId(1))),
//         ],
//         references: Vec::new(),
//     };
//
//     graph.entities[0].children = vec![EntityId(1)];
//     graph.entities[1].children = vec![EntityId(2)];
//
//     let mut navigator = Navigator::new(graph);
//
//     let root_tree_id = navigator.tree().root.unwrap();
//     let first_child = navigator.zoom_in(super::tree::TreeNodeId(root_tree_id));
//
//     assert!(first_child.is_some());
//     assert_eq!(first_child, Some(EntityId(1)));
//
//     let parent = navigator.zoom_out(super::tree::TreeNodeId(root_tree_id));
//     assert!(parent.is_none()); // root has no parent
// }

#[test]
fn test_entity_kind_display() {
    assert_eq!(EntityKind::Folder.to_string(), "folder");
    assert_eq!(EntityKind::Module.to_string(), "module");
    assert_eq!(EntityKind::File.to_string(), "file");
    assert_eq!(EntityKind::Class.to_string(), "class/struct");
    assert_eq!(EntityKind::Function.to_string(), "fn/method");
}

#[test]
fn test_reference_kind_display() {
    assert_eq!(ReferenceKind::Call.to_string(), "call");
    assert_eq!(ReferenceKind::Import.to_string(), "import");
    assert_eq!(ReferenceKind::TypeRef.to_string(), "type_ref");
    assert_eq!(ReferenceKind::VarRef.to_string(), "var_ref");
    assert_eq!(ReferenceKind::Generic.to_string(), "generic");
}

// ─── GraphTree helpers ────────────────────────────────────────────────────────

/// Deterministic PRNG — no external crate required.
fn xorshift(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

/// Fisher-Yates in-place shuffle driven by the PRNG above.
fn shuffle_edges(edges: &mut Vec<(EntityId, EntityId)>, seed: u64) {
    let mut state = seed;
    for i in (1..edges.len()).rev() {
        let j = (xorshift(&mut state) as usize) % (i + 1);
        edges.swap(i, j);
    }
}

/// Generate up to `num_edges` unique directed edges over `num_nodes` nodes,
/// allowing any direction (including cycles) so we get a real mixed graph.
fn gen_edges(seed: u64, num_nodes: usize, num_edges: usize) -> Vec<(EntityId, EntityId)> {
    let mut state = seed;
    let mut seen = std::collections::HashSet::new();
    let mut edges = Vec::new();
    let mut attempts = 0;
    while edges.len() < num_edges && attempts < num_edges * 20 {
        attempts += 1;
        let from = (xorshift(&mut state) as usize) % num_nodes;
        let to   = (xorshift(&mut state) as usize) % num_nodes;
        if from != to && seen.insert((from, to)) {
            edges.push((EntityId(from), EntityId(to)));
        }
    }
    edges
}

/// Build a GraphTree by inserting `num_nodes` entities then `edges` in order.
fn build_tree(num_nodes: usize, edges: &[(EntityId, EntityId)]) -> GraphTree {
    let mut tree = GraphTree::new();
    for i in 0..num_nodes {
        tree.insert_entity(EntityId(i), vec![]);
    }
    for &(from, to) in edges {
        tree.insert_edge(from, to);
    }
    tree
}

fn canonical_subtree(tree: &GraphTree, node_id: GraphTreeNodeId) -> String {
    let node = tree.get(node_id).unwrap();
    match node.kind {
        NodeKind::Cycle  => format!("!{}[]", node.entity_id.0),
        NodeKind::Ref    => format!("~{}[]", node.entity_id.0),
        NodeKind::Normal => {
            let mut children = node.children.clone();
            // Sort by entity id (numeric) — stable regardless of subtree complexity.
            children.sort_by_key(|&id| tree.get(id).map_or(usize::MAX, |n| n.entity_id.0));
            let child_strs: Vec<String> = children.iter()
                .map(|&c| canonical_subtree(tree, c))
                .collect();
            format!("{}[{}]", node.entity_id.0, child_strs.join(","))
        }
    }
}

/// Canonical string for the whole forest (all root nodes, sorted by entity id).
fn canonical_forest(tree: &GraphTree) -> String {
    let mut roots: Vec<_> = tree.nodes.iter()
        .filter(|n| n.parent.is_none() && n.kind == NodeKind::Normal)
        .collect();
    roots.sort_by_key(|n| n.entity_id.0);
    roots.iter().map(|n| canonical_subtree(tree, n.id)).collect::<Vec<_>>().join("|")
}

// ─── GraphTree tests ──────────────────────────────────────────────────────────

/// Inserting edges A→B then B→C should produce the same tree as B→C then A→B.
/// With the current implementation this FAILS: when A→B is inserted after B→C,
/// the new B node created under A inherits no children, so A's subtree is shallower
/// than when A→B is inserted first.
#[test]
fn test_graph_tree_linear_chain_order_independence() {
    // Order 1: A→B first, then B→C
    let order1 = vec![(EntityId(0), EntityId(1)), (EntityId(1), EntityId(2))];
    // Order 2: B→C first, then A→B
    let order2 = vec![(EntityId(1), EntityId(2)), (EntityId(0), EntityId(1))];

    let t1 = build_tree(3, &order1);
    let t2 = build_tree(3, &order2);

    assert_eq!(
        canonical_forest(&t1),
        canonical_forest(&t2),
        "Linear chain A→B→C: topology differs by insertion order.\n  order1 (A→B, B→C): {}\n  order2 (B→C, A→B): {}",
        canonical_forest(&t1),
        canonical_forest(&t2),
    );
}

/// Diamond graph: A→B, A→C, B→D, C→D.
/// D should appear twice (once under B, once under C) regardless of edge order.
#[test]
fn test_graph_tree_diamond_order_independence() {
    let edges = [
        (EntityId(0), EntityId(1)), // A→B
        (EntityId(0), EntityId(2)), // A→C
        (EntityId(1), EntityId(3)), // B→D
        (EntityId(2), EntityId(3)), // C→D
    ];

    // All hand-picked permutations of the four edges
    let permutations: &[&[usize]] = &[
        &[0, 1, 2, 3],
        &[3, 2, 1, 0],
        &[2, 0, 3, 1],
        &[1, 3, 0, 2],
        &[3, 0, 2, 1],
    ];

    let reference = canonical_forest(&build_tree(4, &edges));

    for perm in permutations {
        let reordered: Vec<_> = perm.iter().map(|&i| edges[i]).collect();
        let got = canonical_forest(&build_tree(4, &reordered));
        assert_eq!(
            reference, got,
            "Diamond: topology differs for permutation {:?}.\n  reference: {}\n  got:       {}",
            perm, reference, got,
        );
    }
}

/// Fuzz test: for several pseudo-random graphs, every shuffled ordering of the
/// same edge set must produce the same canonical tree topology.
#[test]
fn test_graph_tree_topology_stable_under_edge_permutations() {
    let cases: &[(u64, usize, usize)] = &[
        (0xDEAD_BEEF, 6,  8),
        (42,          8, 10),
        (137,         5,  7),
        (0x1234_5678, 9, 12),
        (999_999,     7,  9),
    ];

    for &(seed, num_nodes, num_edges) in cases {
        let edges = gen_edges(seed, num_nodes, num_edges);
        let reference = canonical_forest(&build_tree(num_nodes, &edges));

        for shuffle_seed in [1u64, 2, 3, 4, 5] {
            let mut shuffled = edges.clone();
            shuffle_edges(&mut shuffled, seed ^ shuffle_seed.wrapping_mul(0x9e37_79b9));
            let got = canonical_forest(&build_tree(num_nodes, &shuffled));

            assert_eq!(
                reference, got,
                "Fuzz: topology differs (seed={seed}, shuffle={shuffle_seed})\n  \
                 original: {edges:?}\n  shuffled: {shuffled:?}\n  \
                 reference topology: {reference}\n  got topology:       {got}",
            );
        }
    }
}
