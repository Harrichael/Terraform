//! Two [`EntityGraph`]s of the same project, one graph.
//!
//! [`diff`] takes a base tree and a working tree — each already parsed by a
//! producer, each rooted at a Folder of the same name — and returns a **union
//! graph**: every entity and reference that exists on either side, exactly
//! once, as an ordinary `EntityGraph` with a single root and dense ids. Every
//! consumer of an entity graph (coalescing, layout, a UI) therefore works on
//! a diff unchanged; what changed is a set of side tables indexed by union id
//! ([`GraphDiff::entity_status`], [`GraphDiff::churn`],
//! [`GraphDiff::reference_status`]) plus per-file line ops
//! ([`GraphDiff::files`]) for showing old and new together.
//!
//! Entities are matched **top-down**: the roots pair up, and under a matched
//! pair a child matches the other side's child with the same name, the same
//! kind, and the same ordinal among its same-name same-kind siblings. New's
//! entities come first in the union, in new's order; an old entity with no
//! counterpart is appended under its parent's counterpart, with its whole
//! subtree, and marked `Removed`.
//!
//! Because matching is by name, a **rename shows up as a removed entity plus
//! an added one**, never as one modified entity. Same for a moved file.
//!
//! ```no_run
//! # use std::path::Path;
//! let (base_root, work_root) = (Path::new("/tmp/base/proj"), Path::new("/home/me/proj"));
//! let base = treesitter_producer::graph_from_path(base_root)?;
//! let work = treesitter_producer::graph_from_path(work_root)?;
//! let diff = graph_diff::diff(&base, base_root, &work, work_root)?;
//! for e in &diff.graph.entities {
//!     let (plus, minus) = diff.churn[e.id.0];
//!     println!("{:?} {} +{plus} -{minus}", diff.entity_status[e.id.0], e.name);
//! }
//! # Ok::<(), anyhow::Error>(())
//! ```

mod files;
mod matching;
mod union;

use std::collections::HashMap;
use std::ops::Range;
use std::path::Path;

use anyhow::Result;
use entity_graph::{EntityGraph, EntityId, EntityKind};

use files::FileWork;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Status {
    Same,
    Added,
    Removed,
    Modified,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tag {
    Equal,
    Delete,
    Insert,
}

/// A run of lines shared by, or exclusive to, one side of a file. Deletes and
/// inserts carry their position on the opposite side too, so a consumer can
/// interleave both files into a single listing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineOp {
    pub tag: Tag,
    pub old_start: usize,
    pub old_len: usize,
    pub new_start: usize,
    pub new_len: usize,
}

/// Line detail for one file. A side is `None` when the file exists only on
/// the other side. Absent from [`GraphDiff::files`] entirely when the file is
/// unchanged, or when it is binary or too large to diff.
#[derive(Debug, Clone)]
pub struct FileDiff {
    pub old_text: Option<String>,
    pub new_text: Option<String>,
    pub ops: Vec<LineOp>,
}

pub struct GraphDiff {
    /// The union graph; ids are fresh and unrelated to either input's.
    pub graph: EntityGraph,
    /// Indexed by union `EntityId`.
    pub entity_status: Vec<Status>,
    /// Indexed by index into `graph.references`; never [`Status::Modified`].
    pub reference_status: Vec<Status>,
    /// `(added lines, removed lines)` per union entity.
    pub churn: Vec<(usize, usize)>,
    /// Keyed by union File entity.
    pub files: HashMap<EntityId, FileDiff>,
}

/// Diff `old` against `new`, reading file contents from the two roots.
///
/// Both graphs must have a single root and the two roots must have the same
/// name (extract the base tree under a directory named like the working one).
/// A file the graph claims but that cannot be read is treated as empty rather
/// than failing the whole diff, since an index can outlive its tree.
pub fn diff(
    old: &EntityGraph,
    old_root: &Path,
    new: &EntityGraph,
    new_root: &Path,
) -> Result<GraphDiff> {
    let matching = matching::match_trees(old, new)?;
    let union = union::build(old, new, &matching);
    let graph = union.graph;

    let mut work: HashMap<EntityId, FileWork> = HashMap::new();
    let mut continuations: HashMap<EntityId, (Vec<bool>, Vec<bool>)> = HashMap::new();
    for e in graph.entities.iter().filter(|e| e.kind == EntityKind::File) {
        let origin = union.origin[e.id.0];
        let w = files::analyze(
            origin.old.map(|id| read_file(old_root, old, id)),
            origin.new.map(|id| read_file(new_root, new, id)),
        );
        continuations.insert(
            e.id,
            (continuation_lines(w.old.as_deref()), continuation_lines(w.new.as_deref())),
        );
        work.insert(e.id, w);
    }

    let n = graph.entities.len();
    let file_of = file_ancestors(&graph);
    let mut entity_status = vec![Status::Same; n];
    let mut churn = vec![(0, 0); n];
    // Ids are assigned parent-before-child, so descending ids is a post-order:
    // every child is settled before its parent aggregates.
    for id in (0..n).rev().map(EntityId) {
        let e = &graph.entities[id.0];
        let origin = union.origin[id.0];
        let presence = match (origin.old, origin.new) {
            (Some(_), Some(_)) => None,
            (None, Some(_)) => Some(Status::Added),
            (Some(_), None) => Some(Status::Removed),
            (None, None) => unreachable!("a union entity comes from at least one side"),
        };

        if e.kind == EntityKind::File {
            let w = &work[&id];
            entity_status[id.0] = w.status;
            churn[id.0] = whole_file_churn(&w.ops);
            continue;
        }

        // The N/A sentinel range: nothing of its own to compare, so it stands
        // for its children (a Folder, or a Module the producer gave no span).
        if e.line_range == (0..0) {
            let mut plus = 0;
            let mut minus = 0;
            let mut all_same = true;
            for &c in &e.children {
                plus += churn[c.0].0;
                minus += churn[c.0].1;
                all_same &= entity_status[c.0] == Status::Same;
            }
            churn[id.0] = (plus, minus);
            entity_status[id.0] =
                presence.unwrap_or(if all_same { Status::Same } else { Status::Modified });
            continue;
        }

        let Some(file) = file_of[id.0] else { continue };
        let (old_cont, new_cont) = &continuations[&file];
        churn[id.0] = range_churn(
            &work[&file].ops,
            origin.old.map(|o| widen(&old.entities[o.0].line_range, old_cont)),
            origin.new.map(|nn| widen(&new.entities[nn.0].line_range, new_cont)),
        );
        entity_status[id.0] =
            presence.unwrap_or(if churn[id.0] == (0, 0) { Status::Same } else { Status::Modified });
    }

    let files = work
        .into_iter()
        .filter(|(_, w)| w.diffable)
        .map(|(id, w)| {
            let diff = FileDiff {
                old_text: w.old.and_then(as_text),
                new_text: w.new.and_then(as_text),
                ops: w.ops,
            };
            (id, diff)
        })
        .collect();

    Ok(GraphDiff { graph, entity_status, reference_status: union.reference_status, churn, files })
}

fn as_text(bytes: Vec<u8>) -> Option<String> {
    String::from_utf8(bytes).ok()
}

fn read_file(root: &Path, graph: &EntityGraph, id: EntityId) -> Vec<u8> {
    let rel = graph.file_path(id).unwrap_or_default();
    let path = if rel.as_os_str().is_empty() { root.to_path_buf() } else { root.join(rel) };
    std::fs::read(path).unwrap_or_default()
}

/// The File a union entity lives in, or `None` above every File.
fn file_ancestors(graph: &EntityGraph) -> Vec<Option<EntityId>> {
    let mut file_of = vec![None; graph.entities.len()];
    for e in &graph.entities {
        file_of[e.id.0] = if e.kind == EntityKind::File {
            Some(e.id)
        } else {
            e.parent.and_then(|p| file_of[p.0])
        };
    }
    file_of
}

/// Which lines belong to the declaration that follows them: doc comments,
/// attributes and decorators. An entity's range is widened over them so that
/// editing a function's doc comment counts as editing the function.
fn continuation_lines(bytes: Option<&[u8]>) -> Vec<bool> {
    let Some(text) = bytes.and_then(|b| std::str::from_utf8(b).ok()) else {
        return Vec::new();
    };
    text.split_inclusive('\n')
        .map(|line| {
            let t = line.trim_start();
            t.starts_with("//")
                || t.starts_with("/*")
                || t.starts_with('*')
                || t.starts_with('#')
                || t.starts_with('@')
        })
        .collect()
}

/// The entity's inclusive line range as a half-open range, widened upward
/// over its attached comment/attribute lines.
fn widen(range: &Range<usize>, continuation: &[bool]) -> Range<usize> {
    let mut start = range.start;
    while start > 0 && continuation.get(start - 1).copied().unwrap_or(false) {
        start -= 1;
    }
    start..range.end + 1
}

fn whole_file_churn(ops: &[LineOp]) -> (usize, usize) {
    ops.iter().fold((0, 0), |(plus, minus), op| match op.tag {
        Tag::Insert => (plus + op.new_len, minus),
        Tag::Delete => (plus, minus + op.old_len),
        Tag::Equal => (plus, minus),
    })
}

fn range_churn(
    ops: &[LineOp],
    old_range: Option<Range<usize>>,
    new_range: Option<Range<usize>>,
) -> (usize, usize) {
    let mut plus = 0;
    let mut minus = 0;
    for op in ops {
        match op.tag {
            Tag::Insert => {
                if let Some(r) = &new_range {
                    plus += overlap(op.new_start, op.new_len, r);
                }
            }
            Tag::Delete => {
                if let Some(r) = &old_range {
                    minus += overlap(op.old_start, op.old_len, r);
                }
            }
            Tag::Equal => {}
        }
    }
    (plus, minus)
}

fn overlap(start: usize, len: usize, range: &Range<usize>) -> usize {
    range.end.min(start + len).saturating_sub(range.start.max(start))
}

#[cfg(test)]
mod tests;
