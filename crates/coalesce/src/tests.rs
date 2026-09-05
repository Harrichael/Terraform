use entity_graph::test_support::graph_from_parents;
use entity_graph::{EntityId, EntityKind, ReferenceId, ReferenceKind};

use crate::{Coalesced, CoalescedEdge, Cursor};

use EntityKind::{Class, File, Folder, Function, Module};
use ReferenceKind::{Call, Import, TypeRef};

#[test]
fn test_cursor_initialization() {
    let graph = graph_from_parents(&[("root", Folder, None), ("child", Module, Some(0))], &[]);

    let cursor = Cursor::new(&graph);

    assert_eq!(cursor.active(), &[EntityId(0)]);
    assert_eq!(cursor.references.len(), 0);
}

#[test]
fn test_cursor_with_references() {
    let graph = graph_from_parents(
        &[("root", Folder, None), ("module_a", Module, Some(0)), ("module_b", Module, Some(0))],
        &[(1, 2, Call)],
    );

    let cursor = Cursor::new(&graph);

    assert_eq!(cursor.references.len(), 1);
    assert_eq!(cursor.references[0].reference_id, ReferenceId(0));
    assert_eq!(cursor.references[0].kind, Call);
}

#[test]
fn test_cursor_move_down() {
    let graph = graph_from_parents(
        &[
            ("root", Folder, None),
            ("module", Module, Some(0)),
            ("class_a", Class, Some(1)),
            ("class_b", Class, Some(1)),
        ],
        &[],
    );

    let mut cursor = Cursor::new(&graph);
    let result = cursor.move_down(EntityId(0), &graph);

    assert!(result);
    assert_eq!(cursor.active(), &[EntityId(1)]);
}

#[test]
fn test_cursor_move_down_no_children() {
    let graph = graph_from_parents(&[("leaf", Function, None)], &[]);

    let mut cursor = Cursor::new(&graph);
    let result = cursor.move_down(EntityId(0), &graph);

    assert!(!result);
    assert_eq!(cursor.active(), &[EntityId(0)]);
}

#[test]
fn test_cursor_move_up() {
    let graph = graph_from_parents(
        &[
            ("root", Folder, None),
            ("module", Module, Some(0)),
            ("class_a", Class, Some(1)),
            ("class_b", Class, Some(1)),
        ],
        &[],
    );

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
    let graph = graph_from_parents(&[("root", Folder, None)], &[]);

    let mut cursor = Cursor::new(&graph);
    let result = cursor.move_up(EntityId(0), &graph);

    assert!(!result);
    assert_eq!(cursor.active(), &[EntityId(0)]);
}

fn edge(from: usize, to: usize, kind: ReferenceKind) -> CoalescedEdge {
    CoalescedEdge { from: EntityId(from), to: EntityId(to), kind }
}

fn ids(ids: &[usize]) -> Vec<EntityId> {
    ids.iter().copied().map(EntityId).collect()
}

/// The coalesced view through a full zoom sequence over a folder with two
/// files of two functions each:
///
/// ```text
/// root(0) ─ a.rs(1) ─ fn_a1(3), fn_a2(4)
///         └ b.rs(2) ─ fn_b1(5), fn_b2(6)
/// ```
///
/// At the root everything is a self-loop. At file level the intra-file call
/// vanishes, two calls a→b fold into one, and the Call and Import between the
/// same files both survive because kind is part of the edge identity.
/// Zooming into one file yields mixed-depth edges; zooming into both restores
/// the raw references.
#[test]
fn test_coalesced_through_zoom_levels() {
    let graph = graph_from_parents(
        &[
            ("root", Folder, None),
            ("a.rs", File, Some(0)),
            ("b.rs", File, Some(0)),
            ("fn_a1", Function, Some(1)),
            ("fn_a2", Function, Some(1)),
            ("fn_b1", Function, Some(2)),
            ("fn_b2", Function, Some(2)),
        ],
        &[
            (3, 5, Call),
            (4, 6, Import),
            (3, 4, Call),
            (4, 5, Call),
            (6, 3, TypeRef),
        ],
    );
    let mut cursor = Cursor::new(&graph);

    assert_eq!(cursor.coalesced(), Coalesced { leaves: ids(&[0]), edges: vec![] });

    assert!(cursor.move_down(EntityId(0), &graph));
    assert_eq!(
        cursor.coalesced(),
        Coalesced {
            leaves: ids(&[1, 2]),
            edges: vec![edge(1, 2, Call), edge(1, 2, Import), edge(2, 1, TypeRef)],
        }
    );

    assert!(cursor.move_down(EntityId(1), &graph));
    assert_eq!(
        cursor.coalesced(),
        Coalesced {
            leaves: ids(&[2, 3, 4]),
            edges: vec![
                edge(3, 2, Call),
                edge(4, 2, Import),
                edge(3, 4, Call),
                edge(4, 2, Call),
                edge(2, 3, TypeRef),
            ],
        }
    );

    assert!(cursor.move_down(EntityId(2), &graph));
    assert_eq!(
        cursor.coalesced(),
        Coalesced {
            leaves: ids(&[3, 4, 5, 6]),
            edges: vec![
                edge(3, 5, Call),
                edge(4, 6, Import),
                edge(3, 4, Call),
                edge(4, 5, Call),
                edge(6, 3, TypeRef),
            ],
        }
    );
}

/// A file-level import has nowhere to go once the file itself is zoomed
/// into: its endpoint is no longer a leaf and no child contains it. The
/// coalesced view must not emit edges to entities that are not leaves.
#[test]
fn test_coalesced_drops_edges_to_expanded_endpoints() {
    let graph = graph_from_parents(
        &[
            ("root", Folder, None),
            ("a.rs", File, Some(0)),
            ("b.rs", File, Some(0)),
            ("fn_a", Function, Some(1)),
            ("fn_b", Function, Some(2)),
        ],
        &[(1, 4, Import), (3, 4, Call)],
    );
    let mut cursor = Cursor::new(&graph);
    assert!(cursor.move_down(EntityId(0), &graph));
    assert_eq!(
        cursor.coalesced(),
        Coalesced {
            leaves: vec![EntityId(1), EntityId(2)],
            edges: vec![
                CoalescedEdge { from: EntityId(1), to: EntityId(2), kind: Import },
                CoalescedEdge { from: EntityId(1), to: EntityId(2), kind: Call },
            ],
        }
    );

    // Zoom into a.rs: the Call re-homes to fn_a, the Import's source is now
    // the inactive a.rs and must disappear rather than dangle.
    assert!(cursor.move_down(EntityId(1), &graph));
    assert_eq!(
        cursor.coalesced(),
        Coalesced {
            leaves: vec![EntityId(2), EntityId(3)],
            edges: vec![CoalescedEdge { from: EntityId(3), to: EntityId(2), kind: Call }],
        }
    );
}
