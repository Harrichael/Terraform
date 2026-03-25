//! End-to-end test infrastructure exercising the full pipeline from raw Rust
//! source text through the parser, graph builder, and [`Navigator`].
//!
//! Two test categories:
//!
//! 1. **Navigation tests** — parse a single-file Rust source, perform a
//!    scripted sequence of `zoom_in` / `zoom_out` operations, and assert on
//!    the set of entity names visible in the resulting tree after each step.
//!
//! 2. **Edge assertion test** — parse the fixture codebase under
//!    `src/fixtures/sample_crate/`, collect all symbolic reference edges from
//!    the [`EntityGraph`], render each edge as a human-readable string, sort
//!    them, and compare against the golden file
//!    `src/fixtures/expected_edges.txt`.

use std::path::PathBuf;

use crate::app::graph_builder::code_tree_to_entity_graph;
use crate::graph::entity::EntityGraph;
use crate::graph::navigator::Navigator;
use crate::graph::tree::GraphTreeNodeId;
use crate::parser::{parse_directory, parse_source, SourceLanguage};

// ── Shared fixture ───────────────────────────────────────────────────────────

/// Minimal single-file Rust source used for navigation tests.
///
/// Contains one struct (`Rect`) and three free functions (`area`, `perimeter`,
/// `describe`).  No `impl` blocks are used so the parser produces exactly one
/// entity per top-level declaration, keeping assertions simple and unambiguous.
const SAMPLE_RS: &str = r#"pub struct Rect {
    pub width: f64,
    pub height: f64,
}

pub fn area(r: &Rect) -> f64 {
    r.width * r.height
}

pub fn perimeter(r: &Rect) -> f64 {
    2.0 * (r.width + r.height)
}

pub fn describe(r: &Rect) {
    let _a = area(r);
    let _p = perimeter(r);
}
"#;

// ── Infrastructure helpers ───────────────────────────────────────────────────

/// Parse [`SAMPLE_RS`] through the full pipeline and return a [`Navigator`]
/// initialised at root-level zoom (the file node is the only active leaf).
fn sample_navigator() -> Navigator {
    let mut tree = parse_source(SAMPLE_RS, &SourceLanguage::Rust, "shapes.rs")
        .expect("sample source must parse without error");
    // structural_count bounds the nodes translated by code_tree_to_entity_graph.
    tree.structural_count = tree.node_count();
    let graph = code_tree_to_entity_graph(&tree);
    Navigator::new(graph)
}

/// Return the entity names of **all** nodes currently in `nav`'s view tree,
/// sorted alphabetically.
fn tree_entity_names(nav: &Navigator) -> Vec<String> {
    let mut names: Vec<String> = nav
        .tree()
        .nodes
        .iter()
        .filter_map(|n| nav.entity(n.entity_id))
        .map(|e| e.name.clone())
        .collect();
    names.sort();
    names
}

/// Return the [`GraphTreeNodeId`] of the first tree node whose entity name
/// equals `name`, or panic with a clear message if no such node exists.
fn find_node(nav: &Navigator, name: &str) -> GraphTreeNodeId {
    nav.tree()
        .nodes
        .iter()
        .find(|n| {
            nav.entity(n.entity_id)
                .map(|e| e.name == name)
                .unwrap_or(false)
        })
        .unwrap_or_else(|| panic!("no tree node with entity name {name:?}"))
        .id
}

/// Format a single reference edge as `"from_name --kind--> to_name"`.
fn format_edge(
    graph: &EntityGraph,
    r: &crate::graph::entity::Reference,
) -> String {
    let from_name = graph.get(r.from).map(|e| e.name.as_str()).unwrap_or("?");
    let to_name = graph.get(r.to).map(|e| e.name.as_str()).unwrap_or("?");
    format!("{from_name} --{}--> {to_name}", r.kind)
}

// ── Navigation tests ─────────────────────────────────────────────────────────

/// After construction the navigator's view tree contains only the root file
/// node because the cursor starts at the coarsest (root) zoom level.
#[test]
fn test_initial_tree_shows_file_root() {
    let nav = sample_navigator();
    assert_eq!(tree_entity_names(&nav), vec!["shapes.rs"]);
}

/// Zooming into the root file reveals its direct child entities.
#[test]
fn test_zoom_in_reveals_children() {
    let mut nav = sample_navigator();

    let file_node = find_node(&nav, "shapes.rs");
    nav.zoom_in(file_node);

    // The file's children are Rect, area, perimeter, describe.
    assert_eq!(
        tree_entity_names(&nav),
        vec!["Rect", "area", "describe", "perimeter"],
    );
}

/// Zooming out after zooming in returns to the root file.
#[test]
fn test_zoom_out_restores_root() {
    let mut nav = sample_navigator();

    let file_node = find_node(&nav, "shapes.rs");
    nav.zoom_in(file_node);

    // zoom_out from any child collapses all siblings back to the parent file.
    let area_node = find_node(&nav, "area");
    nav.zoom_out(area_node);

    assert_eq!(tree_entity_names(&nav), vec!["shapes.rs"]);
}

/// A scripted in → out → in sequence produces the same tree state on both
/// zoom-in steps, confirming the navigation is fully reversible.
#[test]
fn test_zoom_in_out_in_sequence() {
    let mut nav = sample_navigator();

    let file_node = find_node(&nav, "shapes.rs");
    nav.zoom_in(file_node);
    let after_first_in = tree_entity_names(&nav);

    // Collapse back to file level.
    let area_node = find_node(&nav, "area");
    nav.zoom_out(area_node);

    // Zoom back in — sync_tree() rebuilds nodes, so re-acquire the file id.
    let file_node2 = find_node(&nav, "shapes.rs");
    nav.zoom_in(file_node2);
    let after_second_in = tree_entity_names(&nav);

    assert_eq!(
        after_first_in, after_second_in,
        "re-zooming into the file must yield the same child set"
    );
}

/// Zooming into a leaf entity (one with no children in the entity graph)
/// is a no-op: the view tree must remain unchanged.
#[test]
fn test_zoom_in_leaf_is_noop() {
    let mut nav = sample_navigator();

    // Zoom in to the file level first.
    let file_node = find_node(&nav, "shapes.rs");
    nav.zoom_in(file_node);
    let before = tree_entity_names(&nav);

    // `area` is a Function with no entity-graph children — zoom in must not change the tree.
    let area_node = find_node(&nav, "area");
    nav.zoom_in(area_node);
    let after = tree_entity_names(&nav);

    assert_eq!(before, after, "zooming into a leaf entity must be a no-op");
}

/// Zooming out from the root (no parent) is a no-op: the view tree remains
/// at the file level.
#[test]
fn test_zoom_out_from_root_is_noop() {
    let mut nav = sample_navigator();
    let before = tree_entity_names(&nav);

    let file_node = find_node(&nav, "shapes.rs");
    nav.zoom_out(file_node);
    let after = tree_entity_names(&nav);

    assert_eq!(before, after, "zooming out from the root must be a no-op");
}

// ── Edge assertion test ──────────────────────────────────────────────────────

/// Absolute path to `src/fixtures/sample_crate/` inside the crate.
fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("fixtures")
        .join("sample_crate")
}

/// Parse the fixture codebase, collect every symbolic reference edge from the
/// [`EntityGraph`], format each as `"from --kind--> to"`, sort them, and
/// assert they exactly match the golden file `src/fixtures/expected_edges.txt`.
///
/// The golden file documents the expected call graph of the fixture codebase
/// and must be updated whenever the fixture source or the reference-extraction
/// logic changes.
#[test]
fn test_entity_graph_edges_match_golden() {
    let dir = fixture_dir();
    let mut tree =
        parse_directory(&dir).expect("fixture directory must parse without error");
    tree.structural_count = tree.node_count();

    let graph = code_tree_to_entity_graph(&tree);

    // Collect, sort, and deduplicate edge strings.
    let mut edges: Vec<String> = graph
        .references
        .iter()
        .map(|r| format_edge(&graph, r))
        .collect();
    edges.sort();
    edges.dedup();

    // Load expected edges from the golden file.
    let golden_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("fixtures")
        .join("expected_edges.txt");
    let golden = std::fs::read_to_string(&golden_path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", golden_path.display()));
    let expected: Vec<String> = golden
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.to_string())
        .collect();

    assert_eq!(
        edges,
        expected,
        concat!(
            "EntityGraph edges do not match golden file.\n",
            "To update the golden file run:\n",
            "  cargo test test_entity_graph_edges_match_golden -- --nocapture\n",
            "and copy the 'Actual edges' block into src/fixtures/expected_edges.txt",
        ),
    );
}
