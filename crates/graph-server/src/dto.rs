//! JSON shapes for the HTTP contract in `ui/CONTRACT.md`. Field names and
//! kind strings are the wire format; the fixture test in `handlers.rs` pins
//! them to `ui/fixture.json`.
//!
//! Diff-mode fields are `Option` + `skip_serializing_if`, so a plain load
//! serializes byte-identically to before diffing existed.

use coalesce::{Coalesced, CoalescedEdge};
use entity_graph::{Entity, EntityGraph, EntityId, EntityKind, Reference, ReferenceKind};
use graph_diff::{LineOp, Status, Tag};
use serde::Serialize;

#[derive(Serialize)]
pub struct GraphDto {
    pub root: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diff: Option<DiffDto>,
    pub nodes: Vec<NodeDto>,
    pub references: Vec<ReferenceDto>,
}

#[derive(Serialize)]
pub struct DiffDto {
    pub base: String,
    pub base_commit: String,
}

#[derive(Serialize)]
pub struct NodeDto {
    pub id: usize,
    pub kind: &'static str,
    pub name: String,
    pub path: String,
    pub line_start: usize,
    pub line_end: usize,
    pub parent: Option<usize>,
    pub loc: usize,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub is_test: bool,
    #[serde(skip_serializing_if = "is_zero")]
    pub test_loc: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub added: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub removed: Option<usize>,
}

#[derive(Serialize)]
pub struct ReferenceDto {
    pub from: usize,
    pub to: usize,
    pub kind: &'static str,
    pub sites: Vec<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<&'static str>,
}

/// A coalesced edge stands for many raw references; `refs` are their indices
/// into `graph.references`, so the UI can join back for sites and status.
#[derive(Serialize)]
pub struct EdgeDto {
    pub from: usize,
    pub to: usize,
    pub kind: &'static str,
    pub sites: Vec<usize>,
    pub refs: Vec<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<&'static str>,
}

#[derive(Serialize)]
pub struct CoalescedDto {
    pub leaves: Vec<usize>,
    pub edges: Vec<EdgeDto>,
}

/// `[tag, old_start, old_len, new_start, new_len]`, tag one of `= - +`.
pub type OpDto = (&'static str, usize, usize, usize, usize);

#[derive(Serialize)]
pub struct SourceDto {
    pub id: usize,
    pub path: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ops: Option<Vec<OpDto>>,
}

#[derive(Serialize)]
pub struct ErrorDto {
    pub error: String,
}

/// The diff side tables a payload needs, borrowed from wherever they live.
pub struct DiffView<'a> {
    pub base: &'a str,
    pub base_commit: &'a str,
    pub entity_status: &'a [Status],
    pub reference_status: &'a [Status],
    pub churn: &'a [(usize, usize)],
}

// `EntityKind`'s Display is for humans ("class/struct"); the wire format is
// the contract's identifiers.
fn entity_kind(kind: EntityKind) -> &'static str {
    match kind {
        EntityKind::Folder => "folder",
        EntityKind::Module => "module",
        EntityKind::File => "file",
        EntityKind::Class => "class",
        EntityKind::Function => "function",
    }
}

fn reference_kind(kind: ReferenceKind) -> &'static str {
    match kind {
        ReferenceKind::Call => "call",
        ReferenceKind::Import => "import",
        ReferenceKind::TypeRef => "type_ref",
        ReferenceKind::VarRef => "var_ref",
        ReferenceKind::Generic => "generic",
    }
}

fn status(status: Status) -> &'static str {
    match status {
        Status::Same => "same",
        Status::Added => "added",
        Status::Removed => "removed",
        Status::Modified => "modified",
    }
}

pub fn op_dto(op: &LineOp) -> OpDto {
    let tag = match op.tag {
        Tag::Equal => "=",
        Tag::Delete => "-",
        Tag::Insert => "+",
    };
    (tag, op.old_start, op.old_len, op.new_start, op.new_len)
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

fn node_dto(e: &Entity, (loc, test_loc): (usize, usize)) -> NodeDto {
    NodeDto {
        id: e.id.0,
        kind: entity_kind(e.kind),
        name: e.name.clone(),
        path: e.path.to_string_lossy().replace('\\', "/"),
        line_start: e.line_range.start,
        line_end: e.line_range.end,
        parent: e.parent.map(|p| p.0),
        loc,
        is_test: e.is_test,
        test_loc,
        status: None,
        added: None,
        removed: None,
    }
}

/// `(loc, test_loc)` per entity. `loc` is the inclusive line range when the
/// entity has one, otherwise (folders) the sum over its children; `test_loc`
/// is how much of that is test code, so a view hiding tests can subtract it.
/// Post-order over the forest so every child is settled before its parent is
/// read.
fn loc_per_entity(graph: &EntityGraph) -> Vec<(usize, usize)> {
    let mut loc = vec![(0, 0); graph.entities.len()];
    let mut stack: Vec<(EntityId, bool)> =
        graph.entities.iter().filter(|e| e.parent.is_none()).map(|e| (e.id, false)).collect();
    while let Some((id, children_done)) = stack.pop() {
        let e = &graph.entities[id.0];
        if !children_done {
            stack.push((id, true));
            stack.extend(e.children.iter().map(|&c| (c, false)));
            continue;
        }
        let own = if e.line_range != (0..0) {
            e.line_range.end - e.line_range.start + 1
        } else {
            e.children.iter().map(|c| loc[c.0].0).sum()
        };
        let test = if e.is_test { own } else { e.children.iter().map(|c| loc[c.0].1).sum() };
        loc[id.0] = (own, test);
    }
    loc
}

impl From<&Reference> for ReferenceDto {
    fn from(r: &Reference) -> Self {
        ReferenceDto {
            from: r.from.0,
            to: r.to.0,
            kind: reference_kind(r.kind),
            sites: r.sites.iter().map(|s| s.line).collect(),
            status: None,
        }
    }
}

impl From<&EntityGraph> for GraphDto {
    fn from(graph: &EntityGraph) -> Self {
        let loc = loc_per_entity(graph);
        GraphDto {
            root: root_name(graph).to_string(),
            diff: None,
            // The arena index is the id, so iterating in order yields the
            // dense, id-sorted `nodes` the contract promises.
            nodes: graph.entities.iter().zip(&loc).map(|(e, &l)| node_dto(e, l)).collect(),
            references: graph.references.iter().map(ReferenceDto::from).collect(),
        }
    }
}

/// The union graph plus the change tags every node and reference carries in
/// diff mode.
pub fn graph_dto_with_diff(graph: &EntityGraph, view: &DiffView<'_>) -> GraphDto {
    let mut dto = GraphDto::from(graph);
    dto.diff =
        Some(DiffDto { base: view.base.to_string(), base_commit: view.base_commit.to_string() });
    for node in &mut dto.nodes {
        let (added, removed) = view.churn[node.id];
        node.status = Some(status(view.entity_status[node.id]));
        node.added = Some(added);
        node.removed = Some(removed);
    }
    for (reference, &st) in dto.references.iter_mut().zip(view.reference_status) {
        reference.status = Some(status(st));
    }
    dto
}

pub fn coalesced_dto(c: &Coalesced, reference_status: Option<&[Status]>) -> CoalescedDto {
    CoalescedDto {
        leaves: c.leaves.iter().map(|id| id.0).collect(),
        edges: c.edges.iter().map(|e| edge_dto(e, reference_status)).collect(),
    }
}

fn edge_dto(e: &CoalescedEdge, reference_status: Option<&[Status]>) -> EdgeDto {
    EdgeDto {
        from: e.from.0,
        to: e.to.0,
        kind: reference_kind(e.kind),
        // The UI derives an edge's sites from `/graph.json` via `refs`.
        sites: Vec::new(),
        refs: e.refs.iter().map(|r| r.0).collect(),
        status: reference_status.map(|all| edge_status(e, all)),
    }
}

/// One tag for a bundle of references: the members' status when they agree,
/// `mixed` when they do not.
fn edge_status(e: &CoalescedEdge, reference_status: &[Status]) -> &'static str {
    let mut members = e.refs.iter().filter_map(|r| reference_status.get(r.0));
    let Some(&first) = members.next() else { return status(Status::Same) };
    if members.all(|&s| s == first) { status(first) } else { "mixed" }
}

pub fn root_name(graph: &EntityGraph) -> &str {
    graph
        .entities
        .iter()
        .find(|e| e.parent.is_none())
        .map(|e| e.name.as_str())
        .unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;
    use entity_graph::test_support::graph_from_parents;
    use EntityKind::*;

    /// `test_loc` is the part of `loc` inside test entities: a file with an
    /// inline `mod tests` counts only the module's lines, a test file counts
    /// itself once however deep its own test children go, and a folder sums
    /// both without double counting.
    #[test]
    fn test_loc_counts_each_test_line_once() {
        let mut g = graph_from_parents(
            &[
                ("proj", Folder, None),
                ("lib.rs", File, Some(0)),
                ("work", Function, Some(1)),
                ("tests", Module, Some(1)),
                ("check", Function, Some(3)),
                ("it.rs", File, Some(0)),
                ("helper", Function, Some(5)),
            ],
            &[],
        );
        let range = |g: &mut EntityGraph, id: usize, r: std::ops::Range<usize>| g.entities[id].line_range = r;
        range(&mut g, 1, 0..19);
        range(&mut g, 2, 0..4);
        range(&mut g, 3, 10..19);
        range(&mut g, 4, 12..18);
        range(&mut g, 5, 0..9);
        range(&mut g, 6, 1..8);
        for id in [3, 4, 5, 6] {
            g.entities[id].is_test = true;
        }

        let dto = GraphDto::from(&g);
        let by_name = |n: &str| dto.nodes.iter().find(|x| x.name == n).unwrap();
        assert_eq!((by_name("lib.rs").loc, by_name("lib.rs").test_loc), (20, 10));
        assert_eq!((by_name("it.rs").loc, by_name("it.rs").test_loc), (10, 10));
        assert_eq!((by_name("proj").loc, by_name("proj").test_loc), (30, 20));
        assert!(by_name("check").is_test && !by_name("work").is_test);
        let json = serde_json::to_value(by_name("work")).unwrap();
        assert!(json.get("is_test").is_none() && json.get("test_loc").is_none(), "false/zero are omitted: {json}");
    }
}
