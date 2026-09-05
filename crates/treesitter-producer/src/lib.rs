//! Tree-sitter producer for [`EntityGraph`].
//!
//! Parsing is name-based: a reference is resolved by looking the callee's or
//! import's leaf identifier up in a project-wide name table, so it is cheap
//! and needs no build, at the cost of precision (same-named symbols collapse
//! onto the first definition seen). That trade-off is the reason this
//! producer exists alongside compiler-backed ones.
//!
//! The producer contract this crate honors:
//!
//! - one root entity: a Folder named after the project directory, or a File
//!   when a single file was loaded;
//! - references deduplicated on `(from, to, kind)`, with self-loops dropped
//!   after Block/Line-level endpoints are lifted to their enclosing entity.
//!
//! The intermediate [`tree::CodeTree`] is an implementation detail retained
//! from the original TUI; `graph_from_path` is the only supported entry point.

mod builder;
mod parser;
mod tree;

use std::path::Path;

use anyhow::{Context, Result};
use entity_graph::EntityGraph;

use parser::SourceLanguage;

pub fn graph_from_path(path: &Path) -> Result<EntityGraph> {
    // `.` or `..` would otherwise become the root entity's name; the root is
    // supposed to be named after the directory.
    let canonical = std::fs::canonicalize(path);
    let path = canonical.as_deref().unwrap_or(path);
    let tree = if path.is_file() {
        let source = std::fs::read_to_string(path)
            .with_context(|| format!("Cannot read file: {}", path.display()))?;
        let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("unknown");
        parser::parse_source(&source, &SourceLanguage::from_path(path), file_name)?
    } else {
        parser::parse_directory(path)?
    };
    Ok(builder::code_tree_to_entity_graph(&tree))
}

#[cfg(test)]
mod tests {
    use super::*;
    use entity_graph::{EntityKind, ReferenceKind};
    use tempfile::TempDir;

    /// The root-shape half of the producer contract: a directory load roots at
    /// a Folder named after the directory with one File per source file, and
    /// a cross-file call surfaces as a kinded Function → Function edge. A
    /// single-file load roots at the File itself.
    #[test]
    fn graph_from_path_honors_root_and_reference_contract() {
        let dir = TempDir::new().unwrap();
        let project = dir.path().join("proj");
        std::fs::create_dir(&project).unwrap();
        std::fs::write(project.join("lib.rs"), "pub fn compute(x: i32) -> i32 { x * 2 }").unwrap();
        std::fs::write(project.join("main.rs"), "fn main() { let v = compute(1); }").unwrap();

        let graph = graph_from_path(&project).unwrap();
        let root = graph.entities.iter().find(|e| e.parent.is_none()).unwrap();
        assert_eq!((root.kind, root.name.as_str()), (EntityKind::Folder, "proj"));
        let mut files: Vec<&str> = root
            .children
            .iter()
            .map(|&c| graph.get(c).unwrap())
            .filter(|e| e.kind == EntityKind::File)
            .map(|e| e.name.as_str())
            .collect();
        files.sort();
        assert_eq!(files, vec!["lib.rs", "main.rs"]);

        let by_name = |name: &str| graph.entities.iter().find(|e| e.name == name).unwrap().id;
        assert_eq!(graph.references.len(), 1);
        let r = &graph.references[0];
        assert_eq!((r.from, r.to, r.kind), (by_name("main"), by_name("compute"), ReferenceKind::Call));

        let single = graph_from_path(&project.join("main.rs")).unwrap();
        let root = single.entities.iter().find(|e| e.parent.is_none()).unwrap();
        assert_eq!((root.kind, root.name.as_str()), (EntityKind::File, "main.rs"));
        assert_eq!(root.children.len(), 1);
    }
}
