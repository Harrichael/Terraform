use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use tree_sitter::{Language, Node, Parser};

use crate::app::tree::{CodeTree, NodeKind};

/// Supported source languages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceLanguage {
    Rust,
    Python,
    JavaScript,
    PlainText,
}

impl SourceLanguage {
    /// Infer from a file extension.
    pub fn from_extension(ext: &str) -> Self {
        match ext.to_ascii_lowercase().as_str() {
            "rs" => SourceLanguage::Rust,
            "py" => SourceLanguage::Python,
            "js" | "mjs" | "cjs" => SourceLanguage::JavaScript,
            _ => SourceLanguage::PlainText,
        }
    }

    /// Infer from a file path.
    pub fn from_path(path: &Path) -> Self {
        path.extension()
            .and_then(|e| e.to_str())
            .map(Self::from_extension)
            .unwrap_or(SourceLanguage::PlainText)
    }
}

// ─── Public API ──────────────────────────────────────────────────────────────

/// Parse a single source file into a standalone `CodeTree`.
pub fn parse_source(source: &str, lang: &SourceLanguage, file_name: &str) -> Result<CodeTree> {
    let mut tree = CodeTree::new();
    let root_id = tree.add_node(
        NodeKind::File,
        file_name,
        (0, source.len()),
        (0, source.lines().count().saturating_sub(1)),
        0,
        None,
    );
    parse_into_tree(&mut tree, source, lang, root_id, 1)?;
    Ok(tree)
}

/// Walk a directory and build a hierarchical `CodeTree` (Folder → File → constructs).
/// Identical top-level class/struct names found in multiple files are deduplicated
/// into the lib section with SymRef nodes in their respective files.
pub fn parse_directory(dir: &Path) -> Result<CodeTree> {
    let mut tree = CodeTree::new();

    let dir_name = dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(".")
        .to_string();

    let root_id = tree.add_node(
        NodeKind::Folder,
        dir_name,
        (0, 0),
        (0, 0),
        0,
        None,
    );

    // Start the root folder at File-level granularity so only folders/files
    // are shown until the user explicitly drills down.
    if let Some(n) = tree.get_mut(root_id) {
        n.granularity_limit = Some(NodeKind::File);
    }

    // symbol_map: (name, kind_label) → first-occurrence node id (canonical definition)
    let mut symbol_map: HashMap<(String, String), usize> = HashMap::new();

    walk_dir(dir, &mut tree, root_id, 1, &mut symbol_map)?;

    Ok(tree)
}

// ─── Internal helpers ────────────────────────────────────────────────────────

/// Recursively build folder/file nodes for `dir` under `parent_id`.
fn walk_dir(
    dir: &Path,
    tree: &mut CodeTree,
    parent_id: usize,
    depth: usize,
    symbol_map: &mut HashMap<(String, String), usize>,
) -> Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .with_context(|| format!("Cannot read directory: {}", dir.display()))?
        .filter_map(|e| e.ok())
        .collect();

    // Sort: directories first, then files — both alphabetically.
    entries.sort_by_key(|e| {
        let is_file = e.path().is_file();
        (is_file, e.file_name())
    });

    for entry in entries {
        let path = entry.path();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?")
            .to_string();

        // Skip hidden files/dirs and common build/dependency directories.
        if name.starts_with('.') || matches!(name.as_str(), "target" | "node_modules" | "__pycache__") {
            continue;
        }

        if path.is_dir() {
            let folder_id = tree.add_node(
                NodeKind::Folder,
                &name,
                (0, 0),
                (0, 0),
                depth,
                Some(parent_id),
            );
            // Sub-folders also start at File-level granularity.
            if let Some(n) = tree.get_mut(folder_id) {
                n.granularity_limit = Some(NodeKind::File);
            }
            walk_dir(&path, tree, folder_id, depth + 1, symbol_map)?;
        } else if path.is_file() {
            let lang = SourceLanguage::from_path(&path);
            let source = std::fs::read_to_string(&path).unwrap_or_default();

            let file_id = tree.add_node(
                NodeKind::File,
                &name,
                (0, source.len()),
                (0, source.lines().count().saturating_sub(1)),
                depth,
                Some(parent_id),
            );

            parse_into_tree_with_symrefs(tree, &source, &lang, file_id, depth + 1, symbol_map)?;
        }
    }

    Ok(())
}

/// Parse source text and add code constructs directly into `tree` under `parent_id`.
fn parse_into_tree(
    tree: &mut CodeTree,
    source: &str,
    lang: &SourceLanguage,
    parent_id: usize,
    depth: usize,
) -> Result<()> {
    match lang {
        SourceLanguage::PlainText => {
            add_plain_text_lines(tree, source, parent_id, depth);
        }
        _ => {
            let ts_lang = ts_language(lang);
            add_ts_constructs(tree, source, ts_lang, parent_id, depth)?;
        }
    }
    Ok(())
}

/// Like `parse_into_tree`, but also checks/registers top-level Class nodes for
/// symbolic reference deduplication across files.
fn parse_into_tree_with_symrefs(
    tree: &mut CodeTree,
    source: &str,
    lang: &SourceLanguage,
    file_id: usize,
    depth: usize,
    symbol_map: &mut HashMap<(String, String), usize>,
) -> Result<()> {
    parse_into_tree(tree, source, lang, file_id, depth)?;

    // Collect the just-added top-level Class nodes (children of file_id at depth `depth`).
    let file_children: Vec<usize> = tree
        .get(file_id)
        .map(|n| n.children.clone())
        .unwrap_or_default();

    let mut to_replace: Vec<(usize, usize)> = Vec::new(); // (child_id, canonical_id)

    for child_id in file_children {
        if let Some(child) = tree.get(child_id) {
            if matches!(child.kind, NodeKind::Class) {
                let key = (child.name.clone(), "cls".to_string());
                match symbol_map.get(&key) {
                    None => {
                        // First occurrence — register as canonical.
                        symbol_map.insert(key, child_id);
                    }
                    Some(&canonical_id) => {
                        // Duplicate — schedule replacement with a SymRef.
                        // Also promote canonical to lib if not already.
                        to_replace.push((child_id, canonical_id));
                    }
                }
            }
        }
    }

    for (child_id, canonical_id) in to_replace {
        // Promote canonical to lib.
        tree.mark_as_lib(canonical_id);

        // Replace the duplicate node with a SymRef in the tree.
        // We do this by converting the existing node to a SymRef in place
        // (removing its children and pointing to the canonical).
        if let Some(node) = tree.get_mut(child_id) {
            let ref_name = format!("→ {}", node.name.clone());
            node.kind = NodeKind::SymRef;
            node.name = ref_name;
            node.sym_ref_target = Some(canonical_id);
            node.children.clear();
        }
    }

    Ok(())
}

fn add_plain_text_lines(tree: &mut CodeTree, source: &str, parent_id: usize, depth: usize) {
    let mut byte_offset = 0usize;
    for (i, line) in source.lines().enumerate() {
        let start = byte_offset;
        let end = start + line.len();
        tree.add_node(
            NodeKind::Line,
            line.trim_end(),
            (start, end),
            (i, i),
            depth,
            Some(parent_id),
        );
        byte_offset = end + 1;
    }
}

fn ts_language(lang: &SourceLanguage) -> Language {
    match lang {
        SourceLanguage::Rust => tree_sitter_rust::LANGUAGE.into(),
        SourceLanguage::Python => tree_sitter_python::LANGUAGE.into(),
        SourceLanguage::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
        SourceLanguage::PlainText => unreachable!(),
    }
}

fn add_ts_constructs(
    tree: &mut CodeTree,
    source: &str,
    ts_lang: Language,
    parent_id: usize,
    depth: usize,
) -> Result<()> {
    let mut parser = Parser::new();
    parser
        .set_language(&ts_lang)
        .context("Failed to set tree-sitter language")?;

    let ts_tree = parser
        .parse(source, None)
        .context("Tree-sitter failed to parse source")?;

    let source_bytes = source.as_bytes();
    let lines: Vec<&str> = source.lines().collect();

    walk_ts_node(ts_tree.root_node(), source_bytes, &lines, tree, parent_id, depth);

    Ok(())
}

/// Recursively walk a tree-sitter node, lifting out interesting constructs.
fn walk_ts_node(
    node: Node<'_>,
    source: &[u8],
    lines: &[&str],
    tree: &mut CodeTree,
    parent_id: usize,
    depth: usize,
) {
    for child in node.children(&mut node.walk()) {
        if let Some((our_kind, name_node)) = classify_ts_node(&child) {
            let start_byte = child.start_byte();
            let end_byte = child.end_byte();
            let start_line = child.start_position().row;
            let end_line = child.end_position().row;

            let display_name = name_node
                .map(|n| extract_node_text(n, source))
                .unwrap_or_else(|| first_line_preview(&child, source, lines));

            let node_id = tree.add_node(
                our_kind.clone(),
                display_name,
                (start_byte, end_byte),
                (start_line, end_line),
                depth,
                Some(parent_id),
            );

            // Recurse into containers (not Lines or Blocks at the leaf level).
            if our_kind != NodeKind::Line {
                walk_ts_node(child, source, lines, tree, node_id, depth + 1);
            }
        } else {
            // Not interesting itself — pass the current parent through.
            walk_ts_node(child, source, lines, tree, parent_id, depth);
        }
    }
}

/// Map a tree-sitter node type to our `NodeKind` and the name sub-node.
fn classify_ts_node<'a>(node: &Node<'a>) -> Option<(NodeKind, Option<Node<'a>>)> {
    match node.kind() {
        // ── Rust ──────────────────────────────────────────────────────────
        "function_item" | "function_signature_item" => {
            Some((NodeKind::Function, node.child_by_field_name("name")))
        }
        "impl_item" => Some((NodeKind::Class, node.child_by_field_name("type"))),
        "struct_item" => Some((NodeKind::Class, node.child_by_field_name("name"))),
        "enum_item" => Some((NodeKind::Class, node.child_by_field_name("name"))),
        "trait_item" => Some((NodeKind::Class, node.child_by_field_name("name"))),
        "mod_item" => Some((NodeKind::Module, node.child_by_field_name("name"))),
        // Basic constructs: if/for/while/loop/match
        "if_expression" | "for_expression" | "while_expression" | "loop_expression"
        | "match_expression" => Some((NodeKind::Block, None)),

        // ── Python ───────────────────────────────────────────────────────
        "function_definition" => Some((NodeKind::Function, node.child_by_field_name("name"))),
        "class_definition" => Some((NodeKind::Class, node.child_by_field_name("name"))),
        "decorated_definition" => Some((NodeKind::Function, None)),
        // Python-only basic constructs
        "with_statement" => Some((NodeKind::Block, None)),

        // ── JavaScript ──────────────────────────────────────────────────
        "function_declaration" | "function" => {
            Some((NodeKind::Function, node.child_by_field_name("name")))
        }
        "method_definition" => Some((NodeKind::Function, node.child_by_field_name("name"))),
        "class_declaration" | "class" => {
            Some((NodeKind::Class, node.child_by_field_name("name")))
        }
        "arrow_function" => Some((NodeKind::Function, None)),
        // JS-only basic constructs
        "for_in_statement" | "switch_statement" => Some((NodeKind::Block, None)),

        // ── Shared Python + JS basic constructs ──────────────────────────
        "if_statement" | "for_statement" | "while_statement" => Some((NodeKind::Block, None)),

        _ => None,
    }
}

fn extract_node_text(node: Node<'_>, source: &[u8]) -> String {
    source[node.start_byte()..node.end_byte()]
        .iter()
        .map(|&b| b as char)
        .collect()
}

/// Maximum number of bytes used as a name preview when no better name is found.
const MAX_PREVIEW_LENGTH: usize = 40;

fn first_line_preview(node: &Node<'_>, source: &[u8], lines: &[&str]) -> String {
    let row = node.start_position().row;
    lines
        .get(row)
        .map(|l| l.trim().to_string())
        .unwrap_or_else(|| {
            let s = node.start_byte();
            let e = (s + MAX_PREVIEW_LENGTH).min(node.end_byte()).min(source.len());
            String::from_utf8_lossy(&source[s..e]).trim().to_string()
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    const RUST_SRC: &str = r#"
mod geometry {
    pub struct Point {
        x: f64,
        y: f64,
    }

    impl Point {
        pub fn new(x: f64, y: f64) -> Self {
            Point { x, y }
        }

        pub fn distance(&self, other: &Point) -> f64 {
            ((self.x - other.x).powi(2) + (self.y - other.y).powi(2)).sqrt()
        }
    }
}

fn main() {
    let p = geometry::Point::new(0.0, 0.0);
    println!("{:?}", p);
}
"#;

    #[test]
    fn test_rust_parse_finds_functions() {
        let tree = parse_source(RUST_SRC, &SourceLanguage::Rust, "test.rs").unwrap();
        let names: Vec<_> = tree
            .all_nodes_dfs()
            .iter()
            .filter(|n| n.kind == NodeKind::Function)
            .map(|n| n.name.clone())
            .collect();
        assert!(names.contains(&"new".to_string()), "Expected 'new': {names:?}");
        assert!(names.contains(&"main".to_string()), "Expected 'main': {names:?}");
    }

    #[test]
    fn test_rust_parse_finds_struct() {
        let tree = parse_source(RUST_SRC, &SourceLanguage::Rust, "test.rs").unwrap();
        let classes: Vec<_> = tree
            .all_nodes_dfs()
            .iter()
            .filter(|n| n.kind == NodeKind::Class)
            .map(|n| n.name.clone())
            .collect();
        assert!(
            classes.iter().any(|n| n.contains("Point")),
            "Expected Point struct: {classes:?}"
        );
    }

    #[test]
    fn test_rust_parse_finds_blocks() {
        let src = r#"
fn example() {
    if true {
        let x = 1;
    }
    for i in 0..10 {
        println!("{}", i);
    }
}
"#;
        let tree = parse_source(src, &SourceLanguage::Rust, "example.rs").unwrap();
        let has_block = tree
            .all_nodes_dfs()
            .iter()
            .any(|n| n.kind == NodeKind::Block);
        assert!(has_block, "Expected Block nodes for if/for");
    }

    #[test]
    fn test_plain_text_gives_lines() {
        let src = "line one\nline two\nline three";
        let tree = parse_source(src, &SourceLanguage::PlainText, "notes.txt").unwrap();
        assert_eq!(tree.len(), 4); // 1 file root + 3 lines
        let vis = tree.visible_nodes();
        assert_eq!(vis.len(), 4);
    }

    #[test]
    fn test_language_from_extension() {
        assert_eq!(SourceLanguage::from_extension("rs"), SourceLanguage::Rust);
        assert_eq!(SourceLanguage::from_extension("py"), SourceLanguage::Python);
        assert_eq!(SourceLanguage::from_extension("txt"), SourceLanguage::PlainText);
    }

    #[test]
    fn test_directory_parse_creates_folder_root() {
        use tempfile::TempDir;
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("main.rs"), "fn main() {}").unwrap();
        let tree = parse_directory(dir.path()).unwrap();
        let root = tree.get(tree.root.unwrap()).unwrap();
        assert_eq!(root.kind, NodeKind::Folder);
    }

    #[test]
    fn test_directory_parse_contains_file() {
        use tempfile::TempDir;
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("lib.rs"), "pub fn hello() {}").unwrap();
        let tree = parse_directory(dir.path()).unwrap();
        let has_file = tree.all_nodes_dfs().iter().any(|n| n.kind == NodeKind::File);
        assert!(has_file);
    }

    #[test]
    fn test_directory_symref_deduplication() {
        use tempfile::TempDir;
        let dir = TempDir::new().unwrap();
        // Two files both define "struct Config" → second becomes a SymRef
        std::fs::write(dir.path().join("a.rs"), "pub struct Config {}").unwrap();
        std::fs::write(dir.path().join("b.rs"), "pub struct Config {}").unwrap();
        let tree = parse_directory(dir.path()).unwrap();
        let has_sym_ref = tree.all_nodes_dfs().iter().any(|n| n.kind == NodeKind::SymRef);
        assert!(has_sym_ref, "Expected a SymRef for duplicated Config");
        assert!(!tree.lib_nodes.is_empty(), "Expected lib entry for canonical Config");
    }
}
