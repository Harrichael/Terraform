use similar::{DiffOp, TextDiff};

use crate::{LineOp, Status, Tag};

/// Largest side a line diff is attempted on. Over this the file still gets a
/// status and whole-file churn, but no ops or texts: the payload would dwarf
/// everything else in the viewer and the diff itself is not cheap.
const MAX_DIFF_BYTES: usize = 1024 * 1024;

/// One file's two sides. `ops` always describes the change (a whole-file
/// insert or delete for a one-sided file), but only counts as publishable
/// line detail when `diffable`.
pub(crate) struct FileWork {
    pub old: Option<Vec<u8>>,
    pub new: Option<Vec<u8>>,
    pub status: Status,
    pub ops: Vec<LineOp>,
    pub diffable: bool,
}

pub(crate) fn analyze(old: Option<Vec<u8>>, new: Option<Vec<u8>>) -> FileWork {
    let (status, ops, diffable) = match (old.as_deref(), new.as_deref()) {
        (Some(o), Some(n)) if o == n => (Status::Same, Vec::new(), false),
        (Some(o), Some(n)) => match (small_utf8(o), small_utf8(n)) {
            (Some(o), Some(n)) => (Status::Modified, line_ops(o, n), true),
            // Binary or oversized: report the change as a wholesale replace so
            // churn still reflects the file's size.
            _ => (Status::Modified, vec![delete_all(o), insert_all(n)], false),
        },
        (None, Some(n)) => (Status::Added, vec![insert_all(n)], small_utf8(n).is_some()),
        (Some(o), None) => (Status::Removed, vec![delete_all(o)], small_utf8(o).is_some()),
        (None, None) => (Status::Same, Vec::new(), false),
    };
    FileWork { old, new, status, ops, diffable }
}

pub(crate) fn line_count(bytes: &[u8]) -> usize {
    bytes.split_inclusive(|&b| b == b'\n').count()
}

fn small_utf8(bytes: &[u8]) -> Option<&str> {
    (bytes.len() <= MAX_DIFF_BYTES).then(|| std::str::from_utf8(bytes).ok()).flatten()
}

fn delete_all(bytes: &[u8]) -> LineOp {
    LineOp { tag: Tag::Delete, old_start: 0, old_len: line_count(bytes), new_start: 0, new_len: 0 }
}

fn insert_all(bytes: &[u8]) -> LineOp {
    LineOp { tag: Tag::Insert, old_start: 0, old_len: 0, new_start: 0, new_len: line_count(bytes) }
}

/// A `Replace` becomes a Delete followed by an Insert, each still carrying its
/// position on the opposite side, which is what lets a consumer interleave the
/// two files into one listing.
fn line_ops(old: &str, new: &str) -> Vec<LineOp> {
    let mut ops = Vec::new();
    for op in TextDiff::from_lines(old, new).ops() {
        match *op {
            DiffOp::Equal { old_index, new_index, len } => ops.push(LineOp {
                tag: Tag::Equal,
                old_start: old_index,
                old_len: len,
                new_start: new_index,
                new_len: len,
            }),
            DiffOp::Delete { old_index, old_len, new_index } => ops.push(LineOp {
                tag: Tag::Delete,
                old_start: old_index,
                old_len,
                new_start: new_index,
                new_len: 0,
            }),
            DiffOp::Insert { old_index, new_index, new_len } => ops.push(LineOp {
                tag: Tag::Insert,
                old_start: old_index,
                old_len: 0,
                new_start: new_index,
                new_len,
            }),
            DiffOp::Replace { old_index, old_len, new_index, new_len } => {
                ops.push(LineOp {
                    tag: Tag::Delete,
                    old_start: old_index,
                    old_len,
                    new_start: new_index,
                    new_len: 0,
                });
                ops.push(LineOp {
                    tag: Tag::Insert,
                    old_start: old_index + old_len,
                    old_len: 0,
                    new_start: new_index,
                    new_len,
                });
            }
        }
    }
    ops
}
