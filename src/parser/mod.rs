use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use tree_sitter::{Language, Node, Parser};

use crate::app::tree::{CodeTree, NodeKind, ReferenceKind};

/// Supported source languages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceLanguage {
    Rust,
    Python,
    JavaScript,
    TypeScript,
    Tsx,
    Sql,
    PlainText,
}

impl SourceLanguage {
    /// Infer from a file extension.
    pub fn from_extension(ext: &str) -> Self {
        match ext.to_ascii_lowercase().as_str() {
            "rs" => SourceLanguage::Rust,
            "py" => SourceLanguage::Python,
            "js" | "mjs" | "cjs" => SourceLanguage::JavaScript,
            "ts" => SourceLanguage::TypeScript,
            "tsx" => SourceLanguage::Tsx,
            "sql" => SourceLanguage::Sql,
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
///
/// After building the contains topology the parser performs a second pass to
/// populate the [`ReferenceGraph`] with call and import edges extracted from
/// each source file.  These edges represent the *symbolic* relationships
/// between constructs (function calls, type usages, imports) and are
/// independent of the directory containment structure.
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

    // Phase 1: build the contains topology (folder → file → constructs).
    // Collect (file_id, source, lang) so we can do reference extraction after.
    let mut file_sources: Vec<(usize, String, SourceLanguage)> = Vec::new();
    walk_dir(dir, &mut tree, root_id, 1, &mut file_sources)?;

    // Phase 2: build a name → node_id map from the completed tree.
    let name_to_id = build_name_to_id_map(&tree);

    // Phase 3: for each file extract raw references and resolve them to node IDs.
    for (file_id, source, lang) in &file_sources {
        for (from_id, ref_name, kind) in extract_raw_refs(source, lang, &tree, *file_id) {
            if let Some(&to_id) = name_to_id.get(&ref_name) {
                if from_id != to_id {
                    tree.add_reference(from_id, to_id, kind);
                }
            }
        }
    }

    Ok(tree)
}

// ─── Internal helpers ────────────────────────────────────────────────────────

/// Recursively build folder/file nodes for `dir` under `parent_id`.
///
/// Each processed file's source text is appended to `file_sources` for the
/// subsequent reference-extraction pass.
fn walk_dir(
    dir: &Path,
    tree: &mut CodeTree,
    parent_id: usize,
    depth: usize,
    file_sources: &mut Vec<(usize, String, SourceLanguage)>,
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
            walk_dir(&path, tree, folder_id, depth + 1, file_sources)?;
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

            parse_into_tree(tree, &source, &lang, file_id, depth + 1)?;
            file_sources.push((file_id, source, lang));
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

// ─── Reference extraction ─────────────────────────────────────────────────────

/// Build a name → node_id lookup from all Function, Class, Module, and File
/// nodes in the tree.  The first occurrence wins for duplicate names.
///
/// **Limitation**: symbols that share a name across files (e.g. two files each
/// defining `fn main`) can only be mapped to the first occurrence.  Fully
/// disambiguating same-named symbols would require tracking parent context or
/// module path, which is left for future work.
fn build_name_to_id_map(tree: &CodeTree) -> HashMap<String, usize> {
    let mut map = HashMap::new();
    for node in tree.all_nodes_dfs() {
        if matches!(node.kind, NodeKind::Function | NodeKind::Class | NodeKind::Module | NodeKind::File) {
            map.entry(node.name.clone()).or_insert(node.id);
        }
    }
    map
}

/// Extract raw symbolic references from one file's source.
///
/// Returns `(from_node_id, ref_name, ref_kind)` triples where `from_node_id`
/// is the innermost Function/Class/File node that textually contains the
/// reference site (determined by byte-range containment).
fn extract_raw_refs(
    source: &str,
    lang: &SourceLanguage,
    tree: &CodeTree,
    file_id: usize,
) -> Vec<(usize, String, ReferenceKind)> {
    if matches!(lang, SourceLanguage::PlainText | SourceLanguage::Sql) {
        return Vec::new();
    }
    let ts_lang = ts_language(lang);
    let mut parser = Parser::new();
    if parser.set_language(&ts_lang).is_err() {
        return Vec::new();
    }
    let ts_tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return Vec::new(),
    };

    // Build (node_id, byte_range) pairs for each Function/Class under this file.
    let scope_nodes = collect_scope_nodes(tree, file_id);

    let source_bytes = source.as_bytes();
    let mut result = Vec::new();
    collect_ref_edges(
        ts_tree.root_node(),
        source_bytes,
        &scope_nodes,
        file_id,
        &mut result,
    );
    result
}

/// Collect all Function and Class node IDs with their byte ranges under `file_id`.
fn collect_scope_nodes(tree: &CodeTree, file_id: usize) -> Vec<(usize, (usize, usize))> {
    let mut out = Vec::new();
    let mut stack = vec![file_id];
    while let Some(id) = stack.pop() {
        if let Some(node) = tree.get(id) {
            if id != file_id && matches!(node.kind, NodeKind::Function | NodeKind::Class) {
                out.push((id, node.byte_range));
            }
            for &child in &node.children {
                stack.push(child);
            }
        }
    }
    out
}

/// Find the innermost scope node (smallest byte range) that contains `byte_offset`,
/// falling back to `file_id` when no function/class scope contains it.
///
/// This is O(n) in the number of scope nodes per call-site.  For typical
/// source files the number of functions is small enough that this is fast.
/// A future optimisation could sort `scope_nodes` by start position and use
/// binary search to find candidates before the linear containment filter.
fn find_containing_scope(
    byte_offset: usize,
    scope_nodes: &[(usize, (usize, usize))],
    file_id: usize,
) -> usize {
    scope_nodes
        .iter()
        .filter(|(_, (start, end))| *start <= byte_offset && byte_offset < *end)
        .min_by_key(|(_, (start, end))| end - start)
        .map(|(id, _)| *id)
        .unwrap_or(file_id)
}

/// Recursively walk a tree-sitter AST to collect call and import references.
fn collect_ref_edges(
    node: Node<'_>,
    source: &[u8],
    scope_nodes: &[(usize, (usize, usize))],
    file_id: usize,
    result: &mut Vec<(usize, String, ReferenceKind)>,
) {
    let kind = node.kind();

    // Call expressions (Rust, JS/TS) and plain calls (Python).
    if kind == "call_expression" || kind == "call" {
        let from_id = find_containing_scope(node.start_byte(), scope_nodes, file_id);
        if let Some(fn_node) = node.child_by_field_name("function") {
            if let Some(name) = extract_leaf_ident(fn_node, source) {
                if !is_trivial_name(&name) {
                    result.push((from_id, name, ReferenceKind::Call));
                }
            }
        }
    }

    // Rust `use` declarations → Import from the file node.
    if kind == "use_declaration" {
        for name in extract_use_leaf_names(node, source) {
            if !is_trivial_name(&name) {
                result.push((file_id, name, ReferenceKind::Import));
            }
        }
    }

    // Python / JS / TS import statements → Import from the file node.
    if kind == "import_statement" || kind == "import_from_statement" {
        for name in extract_import_leaf_names(node, source) {
            if !is_trivial_name(&name) {
                result.push((file_id, name, ReferenceKind::Import));
            }
        }
    }

    for child in node.children(&mut node.walk()) {
        collect_ref_edges(child, source, scope_nodes, file_id, result);
    }
}

/// Extract the leaf identifier from a call-expression's function node.
///
/// Handles simple identifiers, field/member access (`obj.method`), and
/// qualified paths (`Module::function`).
fn extract_leaf_ident(node: Node<'_>, source: &[u8]) -> Option<String> {
    match node.kind() {
        "identifier"
        | "field_identifier"
        | "property_identifier"
        | "type_identifier"
        | "shorthand_property_identifier" => Some(extract_node_text(node, source)),
        _ => {
            // For qualified paths take the rightmost identifier-like child.
            let mut cursor = node.walk();
            let children: Vec<_> = node.named_children(&mut cursor).collect();
            for child in children.iter().rev() {
                if let Some(name) = extract_leaf_ident(*child, source) {
                    return Some(name);
                }
            }
            None
        }
    }
}

/// Extract leaf names from a Rust `use_declaration` node.
fn extract_use_leaf_names(node: Node<'_>, source: &[u8]) -> Vec<String> {
    let mut names = Vec::new();
    collect_use_names(node, source, &mut names);
    names
}

fn collect_use_names(node: Node<'_>, source: &[u8], names: &mut Vec<String>) {
    match node.kind() {
        "identifier" | "type_identifier" => {
            names.push(extract_node_text(node, source));
        }
        // `use foo as Bar` — extract the original name only.
        "use_as_clause" => {
            if let Some(first) = node.named_child(0) {
                collect_use_names(first, source, names);
            }
        }
        // `use_wildcard` (`use module::*`) — skip (no specific name to resolve).
        "use_wildcard" | "self" => {}
        _ => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                collect_use_names(child, source, names);
            }
        }
    }
}

/// Extract leaf names from Python/JS/TS import statements.
fn extract_import_leaf_names(node: Node<'_>, source: &[u8]) -> Vec<String> {
    let mut names = Vec::new();
    collect_import_names(node, source, &mut names);
    names
}

fn collect_import_names(node: Node<'_>, source: &[u8], names: &mut Vec<String>) {
    match node.kind() {
        "identifier" => {
            names.push(extract_node_text(node, source));
        }
        // Python dotted names (e.g. `os.path`) — take the last segment.
        "dotted_name" | "relative_import" => {
            let text = extract_node_text(node, source);
            let leaf = text.split('.').last().unwrap_or(&text).trim().to_string();
            if !leaf.is_empty() {
                names.push(leaf);
            }
        }
        // JS `import_specifier`: `{ A as B }` → extract A.
        "import_specifier" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                names.push(extract_node_text(name_node, source));
            } else if let Some(first) = node.named_child(0) {
                names.push(extract_node_text(first, source));
            }
        }
        // Skip raw string module paths in JS/TS `from 'module'`.
        "string" | "string_fragment" => {}
        // `import module as alias` / `aliased_import` — use the original name.
        "as_pattern" | "aliased_import" => {
            if let Some(first) = node.named_child(0) {
                collect_import_names(first, source, names);
            }
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                collect_import_names(child, source, names);
            }
        }
    }
}

/// Names that are too common/generic to be meaningful reference targets.
///
/// This list filters out standard-library/runtime functions, common helpers,
/// and language built-ins so that the reference graph is not cluttered with
/// noise.  Extend the list as new noisy patterns are encountered; all entries
/// should have near-universal presence across many files (e.g. `new`, `len`,
/// `println`) rather than project-specific names.
fn is_trivial_name(name: &str) -> bool {
    name.len() <= 1
        || matches!(
            name,
            "new"
                | "clone"
                | "unwrap"
                | "unwrap_or"
                | "unwrap_or_else"
                | "expect"
                | "to_string"
                | "to_owned"
                | "into"
                | "from"
                | "as_ref"
                | "as_mut"
                | "push"
                | "pop"
                | "len"
                | "is_empty"
                | "iter"
                | "iter_mut"
                | "get"
                | "set"
                | "insert"
                | "remove"
                | "contains"
                | "contains_key"
                | "println"
                | "print"
                | "eprintln"
                | "eprint"
                | "format"
                | "panic"
                | "Some"
                | "None"
                | "Ok"
                | "Err"
                | "true"
                | "false"
                | "self"
                | "Self"
                | "super"
                | "map"
                | "filter"
                | "collect"
                | "fold"
                | "for_each"
                | "find"
                | "any"
                | "all"
                | "flat_map"
                | "and_then"
                | "or_else"
                | "ok_or"
                | "next"
                | "parse"
                | "split"
                | "join"
                | "trim"
                | "default"
                | "ok"
                | "err"
                | "is_some"
                | "is_none"
                | "is_ok"
                | "is_err"
                | "log"
                | "error"
                | "warn"
                | "info"
                | "debug"
                | "trace"
                | "console"
                | "Object"
                | "Array"
                | "Math"
                | "JSON"
                | "input"
                | "range"
                | "enumerate"
                | "zip"
                | "list"
                | "dict"
                | "str"
                | "int"
                | "float"
                | "bool"
                | "assert"
                | "assert_eq"
                | "assert_ne"
                | "vec"
        )
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
        SourceLanguage::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        SourceLanguage::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
        SourceLanguage::Sql => tree_sitter_sequel::LANGUAGE.into(),
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

        // ── TypeScript-only constructs ────────────────────────────────────
        // TypeScript files are also matched by the JS patterns above; the
        // entries below cover TS-specific node types not present in JS.
        // Interfaces, type aliases, and enums map to Class.
        "interface_declaration" => Some((NodeKind::Class, node.child_by_field_name("name"))),
        "type_alias_declaration" => Some((NodeKind::Class, node.child_by_field_name("name"))),
        "enum_declaration" => Some((NodeKind::Class, node.child_by_field_name("name"))),
        // Namespaces / internal modules map to Module.
        "internal_module" => Some((NodeKind::Module, node.child_by_field_name("name"))),
        // Abstract / interface method signatures map to Function.
        "abstract_method_signature" | "method_signature" => {
            Some((NodeKind::Function, node.child_by_field_name("name")))
        }

        // ── SQL (tree-sitter-sequel) ──────────────────────────────────────
        // Tables and views are class-level constructs; the name lives in the
        // first `object_reference` named child.
        "create_table" => {
            let name_node = find_named_child_by_kind(node, "object_reference");
            Some((NodeKind::Class, name_node))
        }
        "create_view" => {
            let name_node = find_named_child_by_kind(node, "object_reference");
            Some((NodeKind::Class, name_node))
        }
        // Individual SQL statements (SELECT, INSERT, …) are shown as Blocks.
        "statement" => Some((NodeKind::Block, None)),

        // ── Shared Python + JS + TS basic constructs ─────────────────────
        // Note: Python uses `if_statement` / `for_statement` / `while_statement`,
        // JS/TS use the same names, so these are truly shared across three languages.
        "if_statement" | "for_statement" | "while_statement" => Some((NodeKind::Block, None)),

        _ => None,
    }
}

/// Return the first named child whose `kind()` equals `kind_str`, if any.
fn find_named_child_by_kind<'a>(node: &Node<'a>, kind_str: &str) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .find(|child| child.kind() == kind_str)
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
        assert_eq!(SourceLanguage::from_extension("ts"), SourceLanguage::TypeScript);
        assert_eq!(SourceLanguage::from_extension("tsx"), SourceLanguage::Tsx);
        assert_eq!(SourceLanguage::from_extension("sql"), SourceLanguage::Sql);
    }

    const TYPESCRIPT_SRC: &str = r#"
interface Animal {
    name: string;
    speak(): void;
}

type Result<T> = { value: T } | null;

enum Direction {
    Up,
    Down,
}

class Dog implements Animal {
    name: string;
    constructor(name: string) { this.name = name; }
    speak(): void { console.log("Woof"); }
}

function greet(person: string): string {
    return "Hello " + person;
}

namespace Utils {
    export function helper(): void {}
}
"#;

    #[test]
    fn test_typescript_parse_finds_interface() {
        let tree = parse_source(TYPESCRIPT_SRC, &SourceLanguage::TypeScript, "test.ts").unwrap();
        let classes: Vec<_> = tree
            .all_nodes_dfs()
            .iter()
            .filter(|n| n.kind == NodeKind::Class)
            .map(|n| n.name.clone())
            .collect();
        assert!(
            classes.iter().any(|n| n == "Animal"),
            "Expected 'Animal' interface: {classes:?}"
        );
    }

    #[test]
    fn test_typescript_parse_finds_type_alias() {
        let tree = parse_source(TYPESCRIPT_SRC, &SourceLanguage::TypeScript, "test.ts").unwrap();
        let classes: Vec<_> = tree
            .all_nodes_dfs()
            .iter()
            .filter(|n| n.kind == NodeKind::Class)
            .map(|n| n.name.clone())
            .collect();
        assert!(
            classes.iter().any(|n| n == "Result"),
            "Expected 'Result' type alias: {classes:?}"
        );
    }

    #[test]
    fn test_typescript_parse_finds_enum() {
        let tree = parse_source(TYPESCRIPT_SRC, &SourceLanguage::TypeScript, "test.ts").unwrap();
        let classes: Vec<_> = tree
            .all_nodes_dfs()
            .iter()
            .filter(|n| n.kind == NodeKind::Class)
            .map(|n| n.name.clone())
            .collect();
        assert!(
            classes.iter().any(|n| n == "Direction"),
            "Expected 'Direction' enum: {classes:?}"
        );
    }

    #[test]
    fn test_typescript_parse_finds_class() {
        let tree = parse_source(TYPESCRIPT_SRC, &SourceLanguage::TypeScript, "test.ts").unwrap();
        let classes: Vec<_> = tree
            .all_nodes_dfs()
            .iter()
            .filter(|n| n.kind == NodeKind::Class)
            .map(|n| n.name.clone())
            .collect();
        assert!(
            classes.iter().any(|n| n == "Dog"),
            "Expected 'Dog' class: {classes:?}"
        );
    }

    #[test]
    fn test_typescript_parse_finds_function() {
        let tree = parse_source(TYPESCRIPT_SRC, &SourceLanguage::TypeScript, "test.ts").unwrap();
        let fns: Vec<_> = tree
            .all_nodes_dfs()
            .iter()
            .filter(|n| n.kind == NodeKind::Function)
            .map(|n| n.name.clone())
            .collect();
        assert!(
            fns.iter().any(|n| n == "greet"),
            "Expected 'greet' function: {fns:?}"
        );
    }

    #[test]
    fn test_typescript_parse_finds_namespace() {
        let tree = parse_source(TYPESCRIPT_SRC, &SourceLanguage::TypeScript, "test.ts").unwrap();
        let modules: Vec<_> = tree
            .all_nodes_dfs()
            .iter()
            .filter(|n| n.kind == NodeKind::Module)
            .map(|n| n.name.clone())
            .collect();
        assert!(
            modules.iter().any(|n| n == "Utils"),
            "Expected 'Utils' namespace: {modules:?}"
        );
    }

    const SQL_SRC: &str = r#"
CREATE TABLE users (
    id INT PRIMARY KEY,
    name VARCHAR(100)
);

CREATE VIEW active_users AS
SELECT * FROM users WHERE active = 1;

SELECT id, name FROM users;
"#;

    #[test]
    fn test_sql_parse_finds_table() {
        let tree = parse_source(SQL_SRC, &SourceLanguage::Sql, "schema.sql").unwrap();
        let classes: Vec<_> = tree
            .all_nodes_dfs()
            .iter()
            .filter(|n| n.kind == NodeKind::Class)
            .map(|n| n.name.clone())
            .collect();
        assert!(
            classes.iter().any(|n| n == "users"),
            "Expected 'users' table: {classes:?}"
        );
    }

    #[test]
    fn test_sql_parse_finds_view() {
        let tree = parse_source(SQL_SRC, &SourceLanguage::Sql, "schema.sql").unwrap();
        let classes: Vec<_> = tree
            .all_nodes_dfs()
            .iter()
            .filter(|n| n.kind == NodeKind::Class)
            .map(|n| n.name.clone())
            .collect();
        assert!(
            classes.iter().any(|n| n == "active_users"),
            "Expected 'active_users' view: {classes:?}"
        );
    }

    #[test]
    fn test_sql_parse_has_statements_as_blocks() {
        let tree = parse_source(SQL_SRC, &SourceLanguage::Sql, "schema.sql").unwrap();
        let has_block = tree
            .all_nodes_dfs()
            .iter()
            .any(|n| n.kind == NodeKind::Block);
        assert!(has_block, "Expected Block nodes for SQL statements");
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
    fn test_directory_cross_file_references() {
        use tempfile::TempDir;
        let dir = TempDir::new().unwrap();
        // lib.rs defines `compute`; main.rs calls `compute` → should produce a
        // cross-file reference edge from the main.rs file node to the lib.rs
        // function node (or its file).
        std::fs::write(
            dir.path().join("lib.rs"),
            "pub fn compute(x: i32) -> i32 { x * 2 }",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("main.rs"),
            "fn main() { let v = compute(1); }",
        )
        .unwrap();
        let tree = parse_directory(dir.path()).unwrap();
        // The reference graph should contain at least one edge involving `compute`.
        let refs = tree.references.references();
        assert!(
            !refs.is_empty(),
            "Expected at least one reference edge in the graph"
        );
        // Each file still keeps its own constructs (no SymRef mutation).
        let all = tree.all_nodes_dfs();
        let fn_count = all.iter().filter(|n| n.kind == NodeKind::Function).count();
        assert!(fn_count >= 2, "Both files should have their own function nodes");
    }
}
