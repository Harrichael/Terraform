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
    let mut graph = builder::code_tree_to_entity_graph(&tree);
    // A single-file load has the file as root, so its relative path is empty.
    let file_of = |rel: &Path| if path.is_file() { path.to_path_buf() } else { path.join(rel) };
    entity_graph::test_code::mark(&mut graph, |rel| std::fs::read_to_string(file_of(rel)).ok());
    Ok(graph)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    use entity_graph::{EntityKind, ReferenceKind, Site};
    use tempfile::TempDir;

    /// The producer contract end to end: a directory load roots at a Folder
    /// named after the directory with one File per source file, File paths
    /// resolve back to relative filesystem paths, and a function called twice
    /// from one caller yields a single Function → Function edge carrying both
    /// occurrence lines. A single-file load roots at the File itself.
    #[test]
    fn graph_from_path_honors_root_and_reference_contract() {
        let dir = TempDir::new().unwrap();
        let project = dir.path().join("proj");
        std::fs::create_dir_all(project.join("sub")).unwrap();
        std::fs::write(project.join("sub/lib.rs"), "pub fn compute(x: i32) -> i32 { x * 2 }").unwrap();
        std::fs::write(
            project.join("main.rs"),
            "fn main() {\n    let v = compute(1);\n    let w = v;\n    let z = compute(w);\n}\n",
        )
        .unwrap();

        let graph = graph_from_path(&project).unwrap();
        let root = graph.entities.iter().find(|e| e.parent.is_none()).unwrap();
        assert_eq!((root.kind, root.name.as_str()), (EntityKind::Folder, "proj"));
        let by_name = |name: &str| graph.entities.iter().find(|e| e.name == name).unwrap().id;
        assert_eq!(graph.file_path(by_name("main.rs")), Some(PathBuf::from("main.rs")));
        assert_eq!(graph.file_path(by_name("lib.rs")), Some(PathBuf::from("sub/lib.rs")));
        assert_eq!(graph.file_path(by_name("compute")), Some(PathBuf::from("sub/lib.rs")));
        assert_eq!(graph.file_path(root.id), None);

        assert_eq!(graph.references.len(), 1);
        let r = &graph.references[0];
        assert_eq!((r.from, r.to, r.kind), (by_name("main"), by_name("compute"), ReferenceKind::Call));
        assert_eq!(r.sites, vec![Site { line: 1 }, Site { line: 3 }]);

        assert!(graph.entities.iter().all(|e| !e.is_test));

        let single = graph_from_path(&project.join("main.rs")).unwrap();
        let root = single.entities.iter().find(|e| e.parent.is_none()).unwrap();
        assert_eq!((root.kind, root.name.as_str()), (EntityKind::File, "main.rs"));
        assert_eq!(root.children.len(), 1);
        assert_eq!(single.file_path(root.id), Some(PathBuf::new()));
    }

    /// Test marking through the real parser: tree-sitter leaves attributes
    /// outside the item, so the classifier has to look above the start line;
    /// an integration-test folder marks everything beneath it.
    #[test]
    fn test_code_is_marked_from_attributes_and_paths() {
        let dir = TempDir::new().unwrap();
        let project = dir.path().join("proj");
        std::fs::create_dir_all(project.join("tests")).unwrap();
        std::fs::write(project.join("tests/it.rs"), "fn helper() {}\n").unwrap();
        std::fs::write(
            project.join("lib.rs"),
            "pub fn work() {}\n\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn checks_work() { work(); }\n}\n",
        )
        .unwrap();

        let graph = graph_from_path(&project).unwrap();
        let mut flagged: Vec<&str> = graph.entities.iter().filter(|e| e.is_test).map(|e| e.name.as_str()).collect();
        flagged.sort();
        assert_eq!(flagged, ["checks_work", "helper", "it.rs", "tests", "tests"]);
        assert!(!graph.entities.iter().any(|e| e.name == "work" && e.is_test));

        let single = graph_from_path(&project.join("lib.rs")).unwrap();
        assert!(single.entities.iter().any(|e| e.name == "checks_work" && e.is_test));
    }
}
