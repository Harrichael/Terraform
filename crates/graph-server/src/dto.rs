//! JSON shapes for the HTTP contract in `ui/CONTRACT.md`. Field names and
//! kind strings are the wire format; the fixture test in `handlers.rs` pins
//! them to `ui/fixture.json`.

use coalesce::{Coalesced, CoalescedEdge};
use entity_graph::{Entity, EntityGraph, EntityKind, Reference, ReferenceKind};
use serde::Serialize;

#[derive(Serialize)]
pub struct GraphDto {
    pub root: String,
    pub nodes: Vec<NodeDto>,
    pub references: Vec<ReferenceDto>,
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
}

#[derive(Serialize)]
pub struct ReferenceDto {
    pub from: usize,
    pub to: usize,
    pub kind: &'static str,
}

#[derive(Serialize)]
pub struct CoalescedDto {
    pub leaves: Vec<usize>,
    pub edges: Vec<ReferenceDto>,
}

#[derive(Serialize)]
pub struct ErrorDto {
    pub error: String,
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

impl From<&Entity> for NodeDto {
    fn from(e: &Entity) -> Self {
        NodeDto {
            id: e.id.0,
            kind: entity_kind(e.kind),
            name: e.name.clone(),
            path: e.path.to_string_lossy().replace('\\', "/"),
            line_start: e.line_range.start,
            line_end: e.line_range.end,
            parent: e.parent.map(|p| p.0),
        }
    }
}

impl From<&Reference> for ReferenceDto {
    fn from(r: &Reference) -> Self {
        ReferenceDto { from: r.from.0, to: r.to.0, kind: reference_kind(r.kind) }
    }
}

impl From<&CoalescedEdge> for ReferenceDto {
    fn from(e: &CoalescedEdge) -> Self {
        ReferenceDto { from: e.from.0, to: e.to.0, kind: reference_kind(e.kind) }
    }
}

impl From<&EntityGraph> for GraphDto {
    fn from(graph: &EntityGraph) -> Self {
        GraphDto {
            root: root_name(graph).to_string(),
            // The arena index is the id, so iterating in order yields the
            // dense, id-sorted `nodes` the contract promises.
            nodes: graph.entities.iter().map(NodeDto::from).collect(),
            references: graph.references.iter().map(ReferenceDto::from).collect(),
        }
    }
}

impl From<&Coalesced> for CoalescedDto {
    fn from(c: &Coalesced) -> Self {
        CoalescedDto {
            leaves: c.leaves.iter().map(|id| id.0).collect(),
            edges: c.edges.iter().map(ReferenceDto::from).collect(),
        }
    }
}

pub fn root_name(graph: &EntityGraph) -> &str {
    graph
        .entities
        .iter()
        .find(|e| e.parent.is_none())
        .map(|e| e.name.as_str())
        .unwrap_or("")
}
