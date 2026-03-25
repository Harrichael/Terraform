use super::entity::EntityId;
use super::tree::{GraphTree, GraphTreeNodeId, NodeKind};

fn canonical_subtree(tree: &GraphTree, node_id: GraphTreeNodeId) -> String {
    let node = tree.get(node_id).unwrap();
    match node.kind {
        // Cycle nodes are the terminus — show entity id but never recurse.
        NodeKind::Cycle  => format!("!{}[]", node.entity_id.0),
        // Ref nodes are internal to Cycle; shown if somehow reached directly.
        NodeKind::Ref    => format!("~{}[]", node.entity_id.0),
        NodeKind::Normal => {
            let mut children = node.children.clone();
            children.sort_by_key(|&id| tree.get(id).map_or(usize::MAX, |n| n.entity_id.0));
            let child_strs: Vec<String> = children.iter()
                .map(|&c| canonical_subtree(tree, c))
                .collect();
            format!("{}[{}]", node.entity_id.0, child_strs.join(","))
        }
    }
}

fn canonical_forest(tree: &GraphTree) -> String {
    let mut roots: Vec<_> = tree.nodes.iter()
        .filter(|n| n.parent.is_none() && n.kind == NodeKind::Normal)
        .collect();
    roots.sort_by_key(|n| n.entity_id.0);
    roots.iter().map(|n| canonical_subtree(tree, n.id)).collect::<Vec<_>>().join(",")
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

/// Diamond: Root→A, Root→B, A→Target, B→Target.
/// Target has two parents so it must appear twice in the tree — once under A
/// and once under B — regardless of which order the four edges are inserted.
#[test]
fn test_diamond_all_permutations() {
    // Root=0, A=1, B=2, Target=3
    let edges = [
        (EntityId(0), EntityId(1)), // Root→A
        (EntityId(0), EntityId(2)), // Root→B
        (EntityId(1), EntityId(3)), // A→Target
        (EntityId(2), EntityId(3)), // B→Target
    ];

    let build = |order: &[usize]| {
        let mut tree = GraphTree::new();
        for i in 0..4 { tree.insert_entity(EntityId(i), vec![]); }
        for &i in order { tree.insert_edge(edges[i].0, edges[i].1); }
        tree
    };

    // All 24 permutations of 4 edges
    let permutations: &[&[usize]] = &[
        &[0,1,2,3], &[0,1,3,2], &[0,2,1,3], &[0,2,3,1], &[0,3,1,2], &[0,3,2,1],
        &[1,0,2,3], &[1,0,3,2], &[1,2,0,3], &[1,2,3,0], &[1,3,0,2], &[1,3,2,0],
        &[2,0,1,3], &[2,0,3,1], &[2,1,0,3], &[2,1,3,0], &[2,3,0,1], &[2,3,1,0],
        &[3,0,1,2], &[3,0,2,1], &[3,1,0,2], &[3,1,2,0], &[3,2,0,1], &[3,2,1,0],
    ];

    let reference = canonical_forest(&build(&[0,1,2,3]));

    for perm in permutations {
        let got = canonical_forest(&build(perm));
        assert_eq!(
            reference, got,
            "Diamond: topology differs for permutation {:?}\n  reference: {}\n  got:       {}",
            perm, reference, got,
        );
    }

    // Structural check on the canonical order: Root is the single root,
    // and Target appears as a child under both A and B.
    let tree = build(&[0,1,2,3]);
    let root = tree.nodes.iter().find(|n| n.parent.is_none()).unwrap();
    assert_eq!(root.entity_id, EntityId(0));
    assert_eq!(root.children.len(), 2);

    for &child_id in &root.children {
        let child = tree.get(child_id).unwrap(); // A or B
        assert_eq!(child.children.len(), 1);
        let target = tree.get(child.children[0]).unwrap();
        assert_eq!(target.entity_id, EntityId(3), "child of {:?} should be Target", child.entity_id);
    }
}
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

// ─── Cycle tests ──────────────────────────────────────────────────────────────

/// A self-loop (A → A).
/// A should be the root with one Cycle(A) child — no stubs because A had no
/// other outgoing edges when the back-edge was processed.
#[test]
fn test_cycle_self_loop() {
    let mut tree = GraphTree::new();
    tree.insert_entity(EntityId(0), vec![]);
    tree.insert_edge(EntityId(0), EntityId(0));

    assert_eq!(canonical_forest(&tree), "0[!0[]]");

    let root = tree.nodes.iter().find(|n| n.parent.is_none()).unwrap();
    let cycle_node = tree.get(root.children[0]).unwrap();
    assert_eq!(cycle_node.entity_id, EntityId(0));
    assert!(cycle_node.is_cycle(), "self-loop cycle node should be Cycle kind");
    // Cycle node must have exactly one Ref child.
    assert_eq!(cycle_node.children.len(), 1);
    assert_eq!(tree.get(cycle_node.children[0]).unwrap().kind, NodeKind::Ref);
}

/// Two-node cycle: A → B → A.
/// Direct mutual edge: both become independent roots, each with a Cycle child.
#[test]
fn test_cycle_two_node() {
    let mut tree = GraphTree::new();
    for i in 0..2 { tree.insert_entity(EntityId(i), vec![]); }
    tree.insert_edge(EntityId(0), EntityId(1)); // A→B
    tree.insert_edge(EntityId(1), EntityId(0)); // B→A  ← direct mutual

    // Both A and B are roots; each has a Cycle child pointing to the other.
    assert_eq!(canonical_forest(&tree), "0[!1[]],1[!0[]]");

    let a_root = tree.nodes.iter().find(|n| n.entity_id == EntityId(0) && n.parent.is_none()).unwrap();
    let b_root = tree.nodes.iter().find(|n| n.entity_id == EntityId(1) && n.parent.is_none()).unwrap();

    let cycle_b = tree.get(a_root.children[0]).unwrap();
    assert_eq!(cycle_b.kind, NodeKind::Cycle);
    assert_eq!(cycle_b.entity_id, EntityId(1));
    assert_eq!(tree.get(cycle_b.children[0]).unwrap().kind, NodeKind::Ref);

    let cycle_a = tree.get(b_root.children[0]).unwrap();
    assert_eq!(cycle_a.kind, NodeKind::Cycle);
    assert_eq!(cycle_a.entity_id, EntityId(0));
    assert_eq!(tree.get(cycle_a.children[0]).unwrap().kind, NodeKind::Ref);
}

/// Three-node cycle: A → B → C → A.
/// Cycle(A) under C; stub shows B (A's direct neighbour at detection time).
#[test]
fn test_cycle_three_node() {
    let mut tree = GraphTree::new();
    for i in 0..3 { tree.insert_entity(EntityId(i), vec![]); }
    tree.insert_edge(EntityId(0), EntityId(1)); // A→B
    tree.insert_edge(EntityId(1), EntityId(2)); // B→C
    tree.insert_edge(EntityId(2), EntityId(0)); // C→A  ← back-edge

    // A[B[Cycle(C)]], C[Cycle(A)] — C is cut out and becomes a root.
    assert_eq!(canonical_forest(&tree), "0[1[!2[]]],2[!0[]]");

    let c_root = tree.nodes.iter()
        .find(|n| n.entity_id == EntityId(2) && n.parent.is_none())
        .unwrap();
    let cycle_a = tree.get(c_root.children[0]).unwrap();
    assert_eq!(cycle_a.kind, NodeKind::Cycle);
    assert_eq!(cycle_a.entity_id, EntityId(0));
    assert_eq!(tree.get(cycle_a.children[0]).unwrap().kind, NodeKind::Ref);
}

/// Cycle buried inside a longer prefix: P → A → B → A.
/// P is the only tree root; Cycle(A) with a B stub appears under B.
#[test]
fn test_cycle_with_prefix() {
    let mut tree = GraphTree::new();
    // P=0, A=1, B=2
    for i in 0..3 { tree.insert_entity(EntityId(i), vec![]); }
    tree.insert_edge(EntityId(0), EntityId(1)); // P→A
    tree.insert_edge(EntityId(1), EntityId(2)); // A→B
    tree.insert_edge(EntityId(2), EntityId(1)); // B→A  ← back-edge

    // P[A[Cycle(B)], B[Cycle(A)]] — B is re-parented under P (A's parent), not a free root.
    assert_eq!(canonical_forest(&tree), "0[1[!2[]],2[!1[]]]");

    let roots: Vec<_> = tree.nodes.iter()
        .filter(|n| n.parent.is_none() && n.kind == NodeKind::Normal)
        .collect();
    assert_eq!(roots.len(), 1, "only P should be the root");
    assert_eq!(roots[0].entity_id, EntityId(0));
    assert_eq!(roots[0].children.len(), 2, "P should have both A and B as children");
}

/// Two independent cycles that share no nodes: (A→B→A) and (C→D→C).
/// Each produces its own Cycle node without interfering with the other.
#[test]
fn test_two_independent_cycles() {
    let mut tree = GraphTree::new();
    for i in 0..4 { tree.insert_entity(EntityId(i), vec![]); }
    tree.insert_edge(EntityId(0), EntityId(1)); // A→B
    tree.insert_edge(EntityId(1), EntityId(0)); // B→A  ← cycle 1
    tree.insert_edge(EntityId(2), EntityId(3)); // C→D
    tree.insert_edge(EntityId(3), EntityId(2)); // D→C  ← cycle 2

    // Both mutual pairs cut out: 4 independent roots, each with one Cycle child.
    assert_eq!(canonical_forest(&tree), "0[!1[]],1[!0[]],2[!3[]],3[!2[]]");

    let cycle_nodes: Vec<_> = tree.nodes.iter().filter(|n| n.kind == NodeKind::Cycle).collect();
    assert_eq!(cycle_nodes.len(), 4);
    let mut cycle_entities: Vec<usize> = cycle_nodes.iter().map(|n| n.entity_id.0).collect();
    cycle_entities.sort();
    assert_eq!(cycle_entities, vec![0, 1, 2, 3]);
}

/// Cycle where the back-edge target has a parent: 0→1→2→3→1.
///
/// When 3→1 is detected as a back-edge, 3 should be re-parented to the
/// parent of 1 (which is 0), NOT floated to a free root.
/// Expected: 0[1[2[!3[]]],3[!1[]]]
#[test]
fn test_cycle_reparented_to_target_parent() {
    let build = |edges: &[(EntityId, EntityId)]| {
        let mut tree = GraphTree::new();
        for i in 0..4 { tree.insert_entity(EntityId(i), vec![]); }
        for &(from, to) in edges { tree.insert_edge(from, to); }
        tree
    };

    let canonical_order = [
        (EntityId(0), EntityId(1)),
        (EntityId(1), EntityId(2)),
        (EntityId(2), EntityId(3)),
        (EntityId(3), EntityId(1)), // back-edge: 3→1, target parent is 0
    ];

    let tree = build(&canonical_order);
    assert_eq!(
        canonical_forest(&tree),
        "0[1[2[!3[]]],3[!1[]]]",
        "3 should be re-parented under 0 (parent of cycle-target 1), not a free root",
    );

    // 0 is the only root, with children 1 and 3.
    let roots: Vec<_> = tree.nodes.iter()
        .filter(|n| n.parent.is_none() && n.kind == NodeKind::Normal)
        .collect();
    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0].entity_id, EntityId(0));
    assert_eq!(roots[0].children.len(), 2);
}

///
/// Both A and B should appear as roots in the forest — each showing the other
/// as a child, and the back-edge marked as a cycle node.
///
/// Expected canonical output (both insertion orders):
///   0[!1[]],1[!0[]]
///
/// Currently FAILS because whichever edge is inserted first claims the
/// other entity's free root, so only one entity ends up as a root.
#[test]
fn test_mutual_edge_both_roots() {
    let build = |first: (EntityId, EntityId), second: (EntityId, EntityId)| {
        let mut tree = GraphTree::new();
        tree.insert_entity(EntityId(0), vec![]);
        tree.insert_entity(EntityId(1), vec![]);
        tree.insert_edge(first.0, first.1);
        tree.insert_edge(second.0, second.1);
        tree
    };

    let t1 = build((EntityId(0), EntityId(1)), (EntityId(1), EntityId(0)));
    let t2 = build((EntityId(1), EntityId(0)), (EntityId(0), EntityId(1)));

    assert_eq!(canonical_forest(&t1), "0[!1[]],1[!0[]]");
    assert_eq!(canonical_forest(&t2), "0[!1[]],1[!0[]]");
}
