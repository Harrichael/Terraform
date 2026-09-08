//! JSON shapes for the HTTP contract in `ui/CONTRACT.md`. Field names and
//! kind strings are the wire format; the fixture test in `handlers.rs` pins
//! them to `ui/fixture.json`.

use coalesce::{Coalesced, CoalescedEdge};
use entity_graph::{Entity, EntityGraph, EntityId, EntityKind, Reference, ReferenceKind};
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
    pub loc: usize,
}

#[derive(Serialize)]
pub struct ReferenceDto {
    pub from: usize,
    pub to: usize,
    pub kind: &'static str,
    pub sites: Vec<usize>,
}

#[derive(Serialize)]
pub struct CoalescedDto {
    pub leaves: Vec<usize>,
    pub edges: Vec<ReferenceDto>,
}

#[derive(Serialize)]
pub struct SourceDto {
    pub id: usize,
    pub path: String,
    pub text: String,
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

fn node_dto(e: &Entity, loc: usize) -> NodeDto {
    NodeDto {
        id: e.id.0,
        kind: entity_kind(e.kind),
        name: e.name.clone(),
        path: e.path.to_string_lossy().replace('\\', "/"),
        line_start: e.line_range.start,
        line_end: e.line_range.end,
        parent: e.parent.map(|p| p.0),
        loc,
    }
}

/// Lines of code per entity: the inclusive line range when the entity has
/// one, otherwise (folders) the sum over its children. Post-order over the
/// forest so every child is settled before its parent is read.
fn loc_per_entity(graph: &EntityGraph) -> Vec<usize> {
    let mut loc = vec![0; graph.entities.len()];
    let mut stack: Vec<(EntityId, bool)> =
        graph.entities.iter().filter(|e| e.parent.is_none()).map(|e| (e.id, false)).collect();
    while let Some((id, children_done)) = stack.pop() {
        let e = &graph.entities[id.0];
        if !children_done {
            stack.push((id, true));
            stack.extend(e.children.iter().map(|&c| (c, false)));
            continue;
        }
        loc[id.0] = if e.line_range != (0..0) {
            e.line_range.end - e.line_range.start + 1
        } else {
            e.children.iter().map(|c| loc[c.0]).sum()
        };
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
        }
    }
}

// Coalesced edges aggregate many raw references; the UI derives sites from
// `/graph.json` instead, so none are carried here.
impl From<&CoalescedEdge> for ReferenceDto {
    fn from(e: &CoalescedEdge) -> Self {
        ReferenceDto { from: e.from.0, to: e.to.0, kind: reference_kind(e.kind), sites: Vec::new() }
    }
}

impl From<&EntityGraph> for GraphDto {
    fn from(graph: &EntityGraph) -> Self {
        let loc = loc_per_entity(graph);
        GraphDto {
            root: root_name(graph).to_string(),
            // The arena index is the id, so iterating in order yields the
            // dense, id-sorted `nodes` the contract promises.
            nodes: graph.entities.iter().zip(&loc).map(|(e, &l)| node_dto(e, l)).collect(),
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
