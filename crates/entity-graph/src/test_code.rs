//! Which entities are test code.
//!
//! No producer can answer this from its input alone: SCIP reserves a `Test`
//! symbol role but no indexer we have met sets it, and tree-sitter has no
//! notion of tests at all. So test-ness is decided by convention, in one
//! place, so every producer marks the same things:
//!
//! - a Folder named `tests`, `test` or `__tests__`;
//! - a File named the way its language names tests: Rust `tests.rs`/`test.rs`
//!   or `*_tests.rs`, Go `*_test.go`, Python `test_*.py`/`*_test.py`/
//!   `conftest.py`, JS/TS `*.test.*`/`*.spec.*`;
//! - a Module named `tests` or `test`;
//! - a Module, Class or Function whose attributes or decorators name a test
//!   framework or harness (`#[test]`, `#[cfg(test)]`, `#[tokio::test]`,
//!   `#[test_case(..)]`, `@pytest.fixture`, `@unittest.skip`), except
//!   `cfg(not(test))` and `cfg_attr(test, ..)`, which guard production code;
//! - in Python files, a Function named `test_*` or a Class named `Test*`;
//! - anything inside one of the above.
//!
//! Attributes are read from the source text rather than the syntax tree
//! because the two producers disagree on where an entity starts: tree-sitter
//! puts attributes and doc comments outside the item, rust-analyzer's
//! enclosing range includes them. Scanning outward from the start line in
//! both directions, through comments, covers both.

use std::path::Path;

use crate::{EntityGraph, EntityId, EntityKind};

/// Mark `is_test` on every entity. `read` returns the text of a file given
/// its path relative to the project root (see [`EntityGraph::file_path`]);
/// it is called at most once per file, and only when an attribute lookup is
/// actually needed.
pub fn mark(graph: &mut EntityGraph, mut read: impl FnMut(&Path) -> Option<String>) {
    // Depth-first, so one file's subtree is visited contiguously and a single
    // cached file suffices.
    let mut cached: Option<(EntityId, Option<Vec<String>>)> = None;
    let roots: Vec<EntityId> = graph.entities.iter().filter(|e| e.parent.is_none()).map(|e| e.id).collect();
    let mut stack: Vec<(EntityId, bool)> = roots.into_iter().map(|id| (id, false)).collect();
    while let Some((id, inherited)) = stack.pop() {
        let is_test = inherited || {
            let e = &graph.entities[id.0];
            match e.kind {
                EntityKind::Folder => matches!(e.name.as_str(), "tests" | "test" | "__tests__"),
                EntityKind::File => test_file_name(&e.name),
                EntityKind::Module | EntityKind::Class | EntityKind::Function => {
                    let file = graph.file_of(id);
                    let py = file.is_some_and(|f| graph.entities[f.0].name.ends_with(".py"));
                    let by_name = match e.kind {
                        EntityKind::Module => matches!(e.name.as_str(), "tests" | "test"),
                        EntityKind::Function => py && e.name.starts_with("test_"),
                        EntityKind::Class => py && e.name.starts_with("Test"),
                        _ => unreachable!(),
                    };
                    // `0..0` is the no-span sentinel, not "starts at line 0".
                    by_name
                        || e.line_range != (0..0)
                            && file.is_some_and(|f| {
                                if cached.as_ref().is_none_or(|(cf, _)| *cf != f) {
                                    let lines = graph.file_path(f).and_then(|rel| read(&rel));
                                    cached = Some((f, lines.map(|t| t.lines().map(str::to_owned).collect())));
                                }
                                let lines = cached.as_ref().and_then(|(_, l)| l.as_deref());
                                lines.is_some_and(|l| attributes_mark_test(l, e.line_range.start))
                            })
                }
            }
        };
        graph.entities[id.0].is_test = is_test;
        stack.extend(graph.entities[id.0].children.iter().map(|&c| (c, is_test)));
    }
}

fn test_file_name(name: &str) -> bool {
    let (stem, ext) = name.rsplit_once('.').unwrap_or((name, ""));
    match ext {
        "rs" => matches!(stem, "test" | "tests") || stem.ends_with("_tests"),
        "go" => stem.ends_with("_test"),
        "py" => stem == "conftest" || stem.starts_with("test_") || stem.ends_with("_test"),
        "js" | "jsx" | "ts" | "tsx" | "mjs" | "cjs" => stem.ends_with(".test") || stem.ends_with(".spec"),
        _ => false,
    }
}

/// Any attribute in the run of attribute and comment lines around `start`.
fn attributes_mark_test(lines: &[String], start: usize) -> bool {
    let trimmed = |i: usize| lines.get(i).map(|l| l.trim_start());
    let is_attr = |i: usize| trimmed(i).is_some_and(|t| t.starts_with("#[") || t.starts_with("#![") || t.starts_with('@'));
    let is_comment = |i: usize| trimmed(i).is_some_and(|t| t.starts_with("//") || t.starts_with('#') || t.starts_with('*') || t.starts_with("/*"));
    let mut hits = Vec::new();
    let mut i = start;
    while is_attr(i) || is_comment(i) {
        if is_attr(i) {
            hits.push(i);
        }
        i += 1;
    }
    let mut i = start;
    while i > 0 && (is_attr(i - 1) || is_comment(i - 1)) {
        i -= 1;
        if is_attr(i) {
            hits.push(i);
        }
    }
    hits.into_iter().any(|i| attribute_is_test(&lines[i]))
}

fn attribute_is_test(line: &str) -> bool {
    let compact: String = line.chars().filter(|c| !c.is_whitespace()).collect();
    if compact.contains("not(test)") || compact.contains("cfg_attr(test") {
        return false;
    }
    // Identifiers only: a string literal like `rename = "latest"` is not one.
    let mut in_str = false;
    let code: String = line
        .chars()
        .filter(|&c| {
            if c == '"' {
                in_str = !in_str;
                return false;
            }
            !in_str
        })
        .collect();
    code.split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .any(|ident| ident.starts_with("test") || matches!(ident, "pytest" | "rstest" | "unittest" | "doctest"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::graph_from_parents;
    use std::collections::HashMap;
    use EntityKind::*;

    /// Every convention at once, through a fake file system: path rules on
    /// folders and files, attribute rules read from source in both the
    /// tree-sitter shape (attribute above the start line) and the
    /// rust-analyzer shape (doc comment and attribute inside the range), the
    /// production-code exceptions, Python naming, and inheritance down the
    /// tree.
    #[test]
    fn marks_by_path_attribute_and_name_then_inherits() {
        let rows = [
            ("proj", Folder, None),
            ("tests", Folder, Some(0)),
            ("it.rs", File, Some(1)),
            ("helper", Function, Some(2)),
            ("lib.rs", File, Some(0)),
            ("main", Function, Some(4)),
            ("Latest", Class, Some(4)),
            ("unit", Function, Some(4)),
            ("tests", Module, Some(4)),
            ("check", Function, Some(8)),
            ("ra.rs", File, Some(0)),
            ("ra_unit", Function, Some(10)),
            ("ra_prod", Function, Some(10)),
            ("app.py", File, Some(0)),
            ("test_it", Function, Some(13)),
            ("TestCase", Class, Some(13)),
            ("run", Function, Some(13)),
            ("view.spec.ts", File, Some(0)),
            ("db_test.go", File, Some(0)),
            ("test_code.rs", File, Some(0)),
            ("smoke_tests.rs", File, Some(0)),
        ];
        let mut g = graph_from_parents(&rows, &[]);
        let files: HashMap<&str, &str> = HashMap::from([
            ("lib.rs", "#[cfg(not(test))]\nfn main() {}\n#[cfg_attr(test, derive(Debug))]\n#[serde(rename = \"latest\")]\nstruct Latest;\n/// doc\n#[test]\n// note\nfn unit() {}\n#[cfg(test)]\nmod tests {\n    fn check() {}\n}\n"),
            ("ra.rs", "//! ra\n/// docs\n#[tokio::test]\nasync fn ra_unit() {}\n\nfn ra_prod() {}\n"),
        ]);
        // Start lines: tree-sitter style points at the item, rust-analyzer
        // style at its leading doc comment.
        let starts = HashMap::from([("main", 1), ("Latest", 4), ("unit", 8), ("tests", 10), ("check", 11), ("ra_unit", 1), ("ra_prod", 5)]);
        for e in &mut g.entities {
            if let Some(&line) = starts.get(e.name.as_str()).filter(|_| e.kind != Folder && e.kind != File) {
                e.line_range = line..line;
            }
            if e.kind == File {
                e.path = format!("proj/{}", e.name).into();
            }
        }
        let mut reads = Vec::new();
        let flagged: Vec<&str> = {
            mark(&mut g, |rel| {
                reads.push(rel.display().to_string());
                files.get(rel.to_str().unwrap()).map(|s| s.to_string())
            });
            g.entities.iter().filter(|e| e.is_test).map(|e| e.name.as_str()).collect()
        };
        assert_eq!(
            flagged,
            ["tests", "it.rs", "helper", "unit", "tests", "check", "ra_unit", "test_it", "TestCase", "view.spec.ts", "db_test.go", "smoke_tests.rs"]
        );
        reads.sort();
        assert_eq!(reads, ["lib.rs", "ra.rs"], "files inside a test folder, or with nothing ranged, are never read");
    }
}
