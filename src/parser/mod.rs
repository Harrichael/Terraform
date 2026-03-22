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
    pub fn from_path(path: &std::path::Path) -> Self {
        path.extension()
            .and_then(|e| e.to_str())
            .map(Self::from_extension)
            .unwrap_or(SourceLanguage::PlainText)
    }
}

/// Parse source text into a `CodeTree`.
pub fn parse_source(source: &str, lang: &SourceLanguage, file_name: &str) -> Result<CodeTree> {
    match lang {
        SourceLanguage::PlainText => Ok(plain_text_tree(source, file_name)),
        _ => {
            let ts_lang = ts_language(lang);
            parse_with_tree_sitter(source, ts_lang, file_name)
        }
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

/// Build a line-by-line tree for plain text / unknown files.
fn plain_text_tree(source: &str, file_name: &str) -> CodeTree {
    let mut tree = CodeTree::new();
    let total_bytes = source.len();
    let lines: Vec<&str> = source.lines().collect();
    let total_lines = lines.len();

    let root_id = tree.add_node(
        NodeKind::File,
        file_name,
        (0, total_bytes),
        (0, total_lines.saturating_sub(1)),
        0,
        None,
    );

    let mut byte_offset = 0usize;
    for (i, line) in lines.iter().enumerate() {
        let start = byte_offset;
        let end = start + line.len();
        tree.add_node(
            NodeKind::Line,
            line.trim_end(),
            (start, end),
            (i, i),
            1,
            Some(root_id),
        );
        byte_offset = end + 1; // +1 for newline
    }
    tree
}

/// Use Tree-sitter to parse a source file and extract a hierarchical CodeTree.
fn parse_with_tree_sitter(
    source: &str,
    ts_lang: Language,
    file_name: &str,
) -> Result<CodeTree> {
    let mut parser = Parser::new();
    parser
        .set_language(&ts_lang)
        .context("Failed to set tree-sitter language")?;

    let ts_tree = parser
        .parse(source, None)
        .context("Tree-sitter failed to parse source")?;

    let source_bytes = source.as_bytes();
    let lines: Vec<&str> = source.lines().collect();

    let mut tree = CodeTree::new();
    let total_bytes = source.len();
    let total_lines = lines.len();

    // Root = the file node.
    let root_id = tree.add_node(
        NodeKind::File,
        file_name,
        (0, total_bytes),
        (0, total_lines.saturating_sub(1)),
        0,
        None,
    );

    // Walk the Tree-sitter CST and map interesting node kinds to our CodeTree.
    walk_ts_node(
        ts_tree.root_node(),
        source_bytes,
        &lines,
        &mut tree,
        root_id,
        1,
    );

    Ok(tree)
}

/// Recursively walk a tree-sitter node, lifting out "interesting" constructs.
fn walk_ts_node(
    node: Node<'_>,
    source: &[u8],
    lines: &[&str],
    tree: &mut CodeTree,
    parent_id: usize,
    depth: usize,
) {
    for child in node.children(&mut node.walk()) {
        let kind_opt = classify_ts_node(&child);
        if let Some((our_kind, name)) = kind_opt {
            let start_byte = child.start_byte();
            let end_byte = child.end_byte();
            let start_line = child.start_position().row;
            let end_line = child.end_position().row;

            // Extract the name text.
            let display_name = name
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

            // Only recurse into container kinds (not lines).
            if our_kind != NodeKind::Line {
                walk_ts_node(child, source, lines, tree, node_id, depth + 1);
            }
        } else {
            // Not interesting itself — pass parent through and keep looking.
            walk_ts_node(child, source, lines, tree, parent_id, depth);
        }
    }
}

/// Map a tree-sitter node type to our `NodeKind` and optionally the sub-node
/// that holds the symbol name.
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
        "block" => Some((NodeKind::Block, None)),

        // ── Python ───────────────────────────────────────────────────────
        "function_definition" => Some((NodeKind::Function, node.child_by_field_name("name"))),
        "class_definition" => Some((NodeKind::Class, node.child_by_field_name("name"))),
        "decorated_definition" => Some((NodeKind::Function, None)),

        // ── JavaScript ──────────────────────────────────────────────────
        "function_declaration" | "function" => {
            Some((NodeKind::Function, node.child_by_field_name("name")))
        }
        "method_definition" => Some((NodeKind::Function, node.child_by_field_name("name"))),
        "class_declaration" | "class" => {
            Some((NodeKind::Class, node.child_by_field_name("name")))
        }
        "arrow_function" => Some((NodeKind::Function, None)),

        _ => None,
    }
}

fn extract_node_text(node: Node<'_>, source: &[u8]) -> String {
    source[node.start_byte()..node.end_byte()]
        .iter()
        .map(|&b| b as char)
        .collect()
}

fn first_line_preview(node: &Node<'_>, source: &[u8], lines: &[&str]) -> String {
    let row = node.start_position().row;
    lines
        .get(row)
        .map(|l| l.trim().to_string())
        .unwrap_or_else(|| {
            // Fallback: first 40 bytes of the node text.
            let s = node.start_byte();
            let e = (s + 40).min(node.end_byte()).min(source.len());
            String::from_utf8_lossy(&source[s..e])
                .trim()
                .to_string()
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
    fn test_plain_text_gives_lines() {
        let src = "line one\nline two\nline three";
        let tree = parse_source(src, &SourceLanguage::PlainText, "notes.txt").unwrap();
        assert_eq!(tree.len(), 4); // 1 file root + 3 lines
        let vis = tree.visible_nodes();
        assert_eq!(vis.len(), 4);
    }

    #[test]
    fn test_language_from_extension() {
        assert_eq!(
            SourceLanguage::from_extension("rs"),
            SourceLanguage::Rust
        );
        assert_eq!(
            SourceLanguage::from_extension("py"),
            SourceLanguage::Python
        );
        assert_eq!(
            SourceLanguage::from_extension("txt"),
            SourceLanguage::PlainText
        );
    }
}
