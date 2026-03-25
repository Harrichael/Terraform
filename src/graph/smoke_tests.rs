use super::entity::EntityId;
use super::tree::{GraphTree, GraphTreeNodeId};

fn canonical_subtree(tree: &GraphTree, node_id: GraphTreeNodeId) -> String {
    let node = tree.get(node_id).unwrap();
    let mut child_strs: Vec<String> = node.children
        .iter()
        .map(|&c| canonical_subtree(tree, c))
        .collect();
    child_strs.sort();
    format!("{}[{}]", node.entity_id.0, child_strs.join(","))
}

fn canonical_forest(tree: &GraphTree) -> String {
    let mut root_strs: Vec<String> = tree.nodes
        .iter()
        .filter(|n| n.parent.is_none())
        .map(|n| canonical_subtree(tree, n.id))
        .collect();
    root_strs.sort();
    root_strs.join("|")
}

/// Inserting A→B after B→C→D already exists should deep-copy the full
/// B→C→D subtree under A, not just the immediate child.
#[test]
fn test_deep_copy_on_late_parent_insertion() {
    // Build with the "hard" order: leaves first, root last
    // B→C, C→D, then A→B
    let hard_order = vec![
        (EntityId(1), EntityId(2)), // B→C
        (EntityId(2), EntityId(3)), // C→D
        (EntityId(0), EntityId(1)), // A→B  (inserted after subtree exists)
    ];

    // Build with the "easy" order: root first
    // A→B, B→C, C→D
    let easy_order = vec![
        (EntityId(0), EntityId(1)), // A→B
        (EntityId(1), EntityId(2)), // B→C
        (EntityId(2), EntityId(3)), // C→D
    ];

    let build = |edges: &[(EntityId, EntityId)]| {
        let mut tree = GraphTree::new();
        for i in 0..4 { tree.insert_entity(EntityId(i), vec![]); }
        for &(from, to) in edges { tree.insert_edge(from, to); }
        tree
    };

    let t_hard = build(&hard_order);
    let t_easy = build(&easy_order);

    assert_eq!(
        canonical_forest(&t_hard),
        canonical_forest(&t_easy),
        "Deep copy failed: A's subtree differs by insertion order.\n  easy (A→B,B→C,C→D): {}\n  hard (B→C,C→D,A→B): {}",
        canonical_forest(&t_easy),
        canonical_forest(&t_hard),
    );
}

/// Tests the deep_copy_subtree path: when a child entity has two parents,
/// the first parent claims the free root node and the second triggers a deep
/// copy.  Both copies of B must carry the full B→C subtree regardless of
/// which order the edges arrive.
#[test]
fn test_deep_copy_triggered_by_multiple_parents() {
    // Graph: P1→B, P2→B, B→C
    // Expected tree:
    //   P1 └── B └── C
    //   P2 └── B └── C   (deep-copied, independent subtree)
    let edges = [
        (EntityId(0), EntityId(2)), // P1→B
        (EntityId(1), EntityId(2)), // P2→B  ← triggers deep copy on second parent
        (EntityId(2), EntityId(3)), // B→C
    ];

    let build = |edge_order: &[usize]| {
        let mut tree = GraphTree::new();
        for i in 0..4 { tree.insert_entity(EntityId(i), vec![]); }
        for &i in edge_order { tree.insert_edge(edges[i].0, edges[i].1); }
        tree
    };

    // All permutations of the 3 edges
    let permutations: &[&[usize]] = &[
        &[0, 1, 2],
        &[0, 2, 1],
        &[1, 0, 2],
        &[1, 2, 0],
        &[2, 0, 1],
        &[2, 1, 0],
    ];

    let reference = canonical_forest(&build(&[0, 1, 2]));

    for perm in permutations {
        let got = canonical_forest(&build(perm));
        assert_eq!(
            reference, got,
            "Deep copy: topology differs for permutation {:?}.\n  reference: {}\n  got:       {}",
            perm, reference, got,
        );
    }

    // Concrete structure check: each of P1 and P2 has exactly one B-child,
    // and that B-child has exactly one C-child.
    let tree = build(&[0, 1, 2]);
    for &parent_entity in &[EntityId(0), EntityId(1)] {
        let parent_nodes: Vec<_> = tree.nodes.iter()
            .filter(|n| n.entity_id == parent_entity && n.parent.is_none())
            .collect();
        assert_eq!(parent_nodes.len(), 1, "{parent_entity:?} should be a single root");

        let b_children: Vec<_> = parent_nodes[0].children.iter()
            .filter_map(|&id| tree.get(id))
            .filter(|n| n.entity_id == EntityId(2))
            .collect();
        assert_eq!(b_children.len(), 1, "{parent_entity:?} should have exactly one B child");

        let c_children: Vec<_> = b_children[0].children.iter()
            .filter_map(|&id| tree.get(id))
            .filter(|n| n.entity_id == EntityId(3))
            .collect();
        assert_eq!(c_children.len(), 1, "B under {parent_entity:?} should have exactly one C child");
    }
}

#[test]
fn test_linear_chain_order_independence() {
    let order1 = vec![(EntityId(0), EntityId(1)), (EntityId(1), EntityId(2))];
    let order2 = vec![(EntityId(1), EntityId(2)), (EntityId(0), EntityId(1))];

    let build = |edges: &[(EntityId, EntityId)]| {
        let mut tree = GraphTree::new();
        for i in 0..3 { tree.insert_entity(EntityId(i), vec![]); }
        for &(from, to) in edges { tree.insert_edge(from, to); }
        tree
    };

    let t1 = build(&order1);
    let t2 = build(&order2);

    assert_eq!(
        canonical_forest(&t1),
        canonical_forest(&t2),
        "Topology differs by insertion order.\n  A→B, B→C: {}\n  B→C, A→B: {}",
        canonical_forest(&t1),
        canonical_forest(&t2),
    );
}
