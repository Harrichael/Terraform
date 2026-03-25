use std::ops::Range;
use std::path::PathBuf;

use super::entity::{Entity, EntityGraph, EntityId, EntityKind, Reference, ReferenceKind, ReferenceId};
use super::cursor::Cursor;
use super::navigator::Navigator;
use super::tree::{GraphTree, GraphTreeNodeId, NodeKind};

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

/// Build a GraphTree by inserting `num_nodes` entities then `edges`.
///
/// Edges are sorted into canonical order (by parent id, then child id) before
/// insertion so that the resulting topology is determined solely by the *set*
/// of edges and is independent of the caller's ordering.  This is the fix for
/// the structural instability: the cycle-cutting algorithm makes root/ancestor
/// decisions based on which edges arrive first; normalising the input order
/// here guarantees every permutation of the same edge set produces an
/// identical tree.
fn build_tree(num_nodes: usize, edges: &[(EntityId, EntityId)]) -> GraphTree {
    let mut tree = GraphTree::new();
    for i in 0..num_nodes {
        tree.insert_entity(EntityId(i), vec![]);
    }
    let mut canonical = edges.to_vec();
    canonical.sort_by_key(|&(a, b)| (a.0, b.0));
    for &(from, to) in &canonical {
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

// ─── Navigator helpers ────────────────────────────────────────────────────────

/// Return the first `Normal` tree node for `entity_id`, or `None` if absent.
fn normal_tree_node(nav: &Navigator, entity_id: EntityId) -> Option<GraphTreeNodeId> {
    nav.tree()
        .nodes_by_entity
        .get(&entity_id)?
        .iter()
        .find(|&&id| nav.tree().get(id).map_or(false, |n| n.kind == NodeKind::Normal))
        .copied()
}

// ─── Navigator deep tests ─────────────────────────────────────────────────────

/// Multiple root entities (no parent) each become active leaves and get their
/// own Normal node in the initial view tree.
#[test]
fn test_navigator_multiple_roots_initial_tree() {
    let graph = EntityGraph {
        entities: vec![
            make_entity(0, "root_a", EntityKind::Folder, None),
            make_entity(1, "root_b", EntityKind::Folder, None),
        ],
        references: vec![],
    };
    let nav = Navigator::new(graph);

    let normal_count = nav.tree().nodes.iter().filter(|n| n.kind == NodeKind::Normal).count();
    assert_eq!(normal_count, 2);
    assert!(nav.tree().nodes_by_entity.contains_key(&EntityId(0)));
    assert!(nav.tree().nodes_by_entity.contains_key(&EntityId(1)));
}

/// A reference between two root entities is wired as a tree edge immediately
/// on construction — no zoom required.
#[test]
fn test_navigator_cross_root_reference() {
    let graph = EntityGraph {
        entities: vec![
            make_entity(0, "a", EntityKind::Folder, None),
            make_entity(1, "b", EntityKind::Folder, None),
        ],
        references: vec![Reference {
            from: EntityId(0),
            to: EntityId(1),
            kind: ReferenceKind::Import,
        }],
    };
    let nav = Navigator::new(graph);

    let a_id = normal_tree_node(&nav, EntityId(0)).unwrap();
    let a = nav.tree().get(a_id).unwrap();
    assert_eq!(a.children.len(), 1);
    let child = nav.tree().get(a.children[0]).unwrap();
    assert_eq!(child.entity_id, EntityId(1));
}

/// Zooming into a three-level hierarchy one step at a time exposes exactly one
/// new level per operation: root → module → [class_a, class_b].
#[test]
fn test_navigator_deep_zoom_in_sequence() {
    let mut graph = EntityGraph {
        entities: vec![
            make_entity(0, "folder",  EntityKind::Folder, None),
            make_entity(1, "module",  EntityKind::Module, Some(EntityId(0))),
            make_entity(2, "class_a", EntityKind::Class,  Some(EntityId(1))),
            make_entity(3, "class_b", EntityKind::Class,  Some(EntityId(1))),
        ],
        references: vec![],
    };
    graph.entities[0].children = vec![EntityId(1)];
    graph.entities[1].children = vec![EntityId(2), EntityId(3)];

    let mut nav = Navigator::new(graph);

    // Level 0 — only the folder is active.
    assert!(nav.tree().nodes_by_entity.contains_key(&EntityId(0)));
    assert!(!nav.tree().nodes_by_entity.contains_key(&EntityId(1)));

    // Level 1 — zoom into folder; module appears, folder vanishes.
    let folder_node = normal_tree_node(&nav, EntityId(0)).unwrap();
    let first = nav.zoom_in(folder_node);
    assert_eq!(first, Some(EntityId(1)));
    assert!(nav.tree().nodes_by_entity.contains_key(&EntityId(1)));
    assert!(!nav.tree().nodes_by_entity.contains_key(&EntityId(2)));

    // Level 2 — zoom into module; both classes appear, module vanishes.
    let module_node = normal_tree_node(&nav, EntityId(1)).unwrap();
    let first_class = nav.zoom_in(module_node);
    assert_eq!(first_class, Some(EntityId(2)));
    assert!(nav.tree().nodes_by_entity.contains_key(&EntityId(2)));
    assert!(nav.tree().nodes_by_entity.contains_key(&EntityId(3)));
    assert!(!nav.tree().nodes_by_entity.contains_key(&EntityId(0)));
    assert!(!nav.tree().nodes_by_entity.contains_key(&EntityId(1)));
}

/// Zooming all the way in and then all the way back out restores the tree to
/// its original single-root state; each zoom_out returns the correct parent id.
#[test]
fn test_navigator_zoom_out_restores_tree() {
    let mut graph = EntityGraph {
        entities: vec![
            make_entity(0, "folder", EntityKind::Folder, None),
            make_entity(1, "module", EntityKind::Module, Some(EntityId(0))),
            make_entity(2, "class",  EntityKind::Class,  Some(EntityId(1))),
        ],
        references: vec![],
    };
    graph.entities[0].children = vec![EntityId(1)];
    graph.entities[1].children = vec![EntityId(2)];

    let mut nav = Navigator::new(graph);

    // Zoom all the way in.
    let n0 = normal_tree_node(&nav, EntityId(0)).unwrap();
    nav.zoom_in(n0);
    let n1 = normal_tree_node(&nav, EntityId(1)).unwrap();
    nav.zoom_in(n1);
    assert_eq!(nav.tree().nodes.iter().filter(|n| n.kind == NodeKind::Normal).count(), 1);
    assert!(nav.tree().nodes_by_entity.contains_key(&EntityId(2)));

    // Zoom out once — back to module level.
    let n2 = normal_tree_node(&nav, EntityId(2)).unwrap();
    let parent = nav.zoom_out(n2);
    assert_eq!(parent, Some(EntityId(1)));
    assert!(nav.tree().nodes_by_entity.contains_key(&EntityId(1)));

    // Zoom out again — back to folder level.
    let n1b = normal_tree_node(&nav, EntityId(1)).unwrap();
    let grandparent = nav.zoom_out(n1b);
    assert_eq!(grandparent, Some(EntityId(0)));
    assert!(nav.tree().nodes_by_entity.contains_key(&EntityId(0)));
    assert_eq!(nav.tree().nodes.iter().filter(|n| n.kind == NodeKind::Normal).count(), 1);
}

/// A reference between deeply nested functions is invisible at coarser zoom
/// levels and becomes a visible tree edge only once both endpoints are leaves.
#[test]
fn test_navigator_reference_remaps_through_zoom() {
    // Folder(0) → [Mod_A(1), Mod_B(2)]
    // Mod_A(1) → [Fn_X(3)],  Mod_B(2) → [Fn_Y(4)]
    // Reference: Fn_X(3) → Fn_Y(4)
    let mut graph = EntityGraph {
        entities: vec![
            make_entity(0, "folder", EntityKind::Folder,   None),
            make_entity(1, "mod_a",  EntityKind::Module,   Some(EntityId(0))),
            make_entity(2, "mod_b",  EntityKind::Module,   Some(EntityId(0))),
            make_entity(3, "fn_x",   EntityKind::Function, Some(EntityId(1))),
            make_entity(4, "fn_y",   EntityKind::Function, Some(EntityId(2))),
        ],
        references: vec![Reference {
            from: EntityId(3),
            to: EntityId(4),
            kind: ReferenceKind::Call,
        }],
    };
    graph.entities[0].children = vec![EntityId(1), EntityId(2)];
    graph.entities[1].children = vec![EntityId(3)];
    graph.entities[2].children = vec![EntityId(4)];

    let mut nav = Navigator::new(graph);

    // Level 0 — reference is internal to folder; only one node, no edges.
    assert_eq!(nav.tree().nodes.len(), 1);

    // Level 1 — reference maps up to mod_a → mod_b edge.
    let folder_node = normal_tree_node(&nav, EntityId(0)).unwrap();
    nav.zoom_in(folder_node);

    let mod_a_id = normal_tree_node(&nav, EntityId(1)).unwrap();
    let mod_a = nav.tree().get(mod_a_id).unwrap();
    assert_eq!(mod_a.children.len(), 1, "mod_a should have mod_b as its tree child");
    let edge_target = nav.tree().get(mod_a.children[0]).unwrap();
    assert_eq!(edge_target.entity_id, EntityId(2), "reference target should be mod_b");

    // Level 2 — zoom into mod_a; fn_x appears, mod_b stays as a leaf.
    let mod_a_id = normal_tree_node(&nav, EntityId(1)).unwrap();
    nav.zoom_in(mod_a_id);

    assert!(nav.tree().nodes_by_entity.contains_key(&EntityId(3)), "fn_x must be a leaf");
    assert!(nav.tree().nodes_by_entity.contains_key(&EntityId(2)), "mod_b must still be a leaf");
    assert!(!nav.tree().nodes_by_entity.contains_key(&EntityId(1)), "mod_a must not be a leaf");
}

/// Mutual references between two sibling leaves produce a clean tree with no
/// Cycle nodes.  The DFS spanning forest only inserts the forward edge (the
/// first direction encountered), so the back-edge is silently dropped and the
/// confusing ↺ marker never appears.
#[test]
fn test_navigator_mutual_references_no_cycle_nodes() {
    let mut graph = EntityGraph {
        entities: vec![
            make_entity(0, "root",  EntityKind::Folder, None),
            make_entity(1, "mod_a", EntityKind::Module, Some(EntityId(0))),
            make_entity(2, "mod_b", EntityKind::Module, Some(EntityId(0))),
        ],
        references: vec![
            Reference { from: EntityId(1), to: EntityId(2), kind: ReferenceKind::Call },
            Reference { from: EntityId(2), to: EntityId(1), kind: ReferenceKind::Call },
        ],
    };
    graph.entities[0].children = vec![EntityId(1), EntityId(2)];

    let mut nav = Navigator::new(graph);
    let root_node = normal_tree_node(&nav, EntityId(0)).unwrap();
    nav.zoom_in(root_node);

    // The DFS spanning forest suppresses back-edges, so no Cycle nodes appear.
    let cycle_nodes: Vec<_> = nav.tree().nodes.iter()
        .filter(|n| n.kind == NodeKind::Cycle)
        .collect();
    assert_eq!(cycle_nodes.len(), 0, "mutual reference should NOT produce Cycle nodes in the spanning-forest view");

    // Both entities must still be present as Normal nodes.
    let normal_entities: Vec<_> = nav.tree().nodes.iter()
        .filter(|n| n.kind == NodeKind::Normal)
        .map(|n| n.entity_id)
        .collect();
    assert!(normal_entities.contains(&EntityId(1)));
    assert!(normal_entities.contains(&EntityId(2)));
}

/// A three-entity reference cycle (A→B→C→A) produces a finite, acyclic tree.
/// The DFS spanning forest inserts only forward edges (A→B and B→C); the
/// back-edge C→A is dropped.  No Cycle nodes appear.
#[test]
fn test_navigator_three_entity_reference_cycle() {
    let mut graph = EntityGraph {
        entities: vec![
            make_entity(0, "root", EntityKind::Folder, None),
            make_entity(1, "a",   EntityKind::Module,  Some(EntityId(0))),
            make_entity(2, "b",   EntityKind::Module,  Some(EntityId(0))),
            make_entity(3, "c",   EntityKind::Module,  Some(EntityId(0))),
        ],
        references: vec![
            Reference { from: EntityId(1), to: EntityId(2), kind: ReferenceKind::Call },
            Reference { from: EntityId(2), to: EntityId(3), kind: ReferenceKind::Call },
            Reference { from: EntityId(3), to: EntityId(1), kind: ReferenceKind::Call },
        ],
    };
    graph.entities[0].children = vec![EntityId(1), EntityId(2), EntityId(3)];

    let mut nav = Navigator::new(graph);
    let root_node = normal_tree_node(&nav, EntityId(0)).unwrap();
    nav.zoom_in(root_node);

    // The spanning forest must be finite and contain no Cycle nodes.
    assert!(
        nav.tree().nodes.len() < 50,
        "tree must be finite, got {} nodes",
        nav.tree().nodes.len()
    );
    let cycle_count = nav.tree().nodes.iter().filter(|n| n.kind == NodeKind::Cycle).count();
    assert_eq!(cycle_count, 0, "3-cycle back-edge is dropped; no Cycle nodes expected");
}

/// Zooming into one sibling while leaving another at the same depth results in
/// a mixed active leaf set spanning two hierarchy levels simultaneously.
#[test]
fn test_navigator_mixed_depth_active_leaves() {
    let mut graph = EntityGraph {
        entities: vec![
            make_entity(0, "root",  EntityKind::Folder,   None),
            make_entity(1, "mod_a", EntityKind::Module,   Some(EntityId(0))),
            make_entity(2, "mod_b", EntityKind::Module,   Some(EntityId(0))),
            make_entity(3, "fn_x",  EntityKind::Function, Some(EntityId(1))),
        ],
        references: vec![],
    };
    graph.entities[0].children = vec![EntityId(1), EntityId(2)];
    graph.entities[1].children = vec![EntityId(3)];

    let mut nav = Navigator::new(graph);

    let root_node = normal_tree_node(&nav, EntityId(0)).unwrap();
    nav.zoom_in(root_node);

    // Zoom into mod_a only; mod_b stays at module depth.
    let mod_a_node = normal_tree_node(&nav, EntityId(1)).unwrap();
    nav.zoom_in(mod_a_node);

    assert!(nav.tree().nodes_by_entity.contains_key(&EntityId(3)), "fn_x should be a leaf");
    assert!(nav.tree().nodes_by_entity.contains_key(&EntityId(2)), "mod_b should still be a leaf");
    assert!(!nav.tree().nodes_by_entity.contains_key(&EntityId(0)), "root must not be active");
    assert!(!nav.tree().nodes_by_entity.contains_key(&EntityId(1)), "mod_a must not be active");
}

/// A self-referencing entity's loop has `from_leaf == to_leaf` and is therefore
/// suppressed from the view tree, keeping the single node child-free.
#[test]
fn test_navigator_self_loop_reference_suppressed() {
    let graph = EntityGraph {
        entities: vec![make_entity(0, "recursive_fn", EntityKind::Function, None)],
        references: vec![Reference {
            from: EntityId(0),
            to: EntityId(0),
            kind: ReferenceKind::Call,
        }],
    };
    let nav = Navigator::new(graph);

    let node = nav.tree().nodes.iter().find(|n| n.entity_id == EntityId(0)).unwrap();
    assert!(node.children.is_empty(), "self-loop must not appear as a tree edge");
}
