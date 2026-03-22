/// Hierarchical code node kinds, from coarsest to finest granularity.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NodeKind {
    Module,
    File,
    Class,
    Function,
    Block,
    Line,
}

impl std::fmt::Display for NodeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            NodeKind::Module => "module",
            NodeKind::File => "file",
            NodeKind::Class => "class/struct",
            NodeKind::Function => "fn/method",
            NodeKind::Block => "block",
            NodeKind::Line => "line",
        };
        write!(f, "{s}")
    }
}

/// A single node in the code tree.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct CodeNode {
    /// Unique identifier within the tree (index into the flat arena).
    pub id: usize,
    pub kind: NodeKind,
    /// Display name (symbol name, file name, line content, …)
    pub name: String,
    /// Optional detail text shown in a secondary column.
    pub detail: Option<String>,
    /// Source byte range in the original file: (start_byte, end_byte).
    pub byte_range: (usize, usize),
    /// Source line range: (first_line, last_line), 0-indexed.
    pub line_range: (usize, usize),
    /// Depth in the tree (0 = root).
    pub depth: usize,
    /// Index of parent node, None for the root.
    pub parent: Option<usize>,
    /// Indices of direct children (in insertion order).
    pub children: Vec<usize>,
    /// Whether this node is currently collapsed.
    pub collapsed: bool,
}

impl CodeNode {
    pub fn new(
        id: usize,
        kind: NodeKind,
        name: impl Into<String>,
        byte_range: (usize, usize),
        line_range: (usize, usize),
        depth: usize,
        parent: Option<usize>,
    ) -> Self {
        CodeNode {
            id,
            kind,
            name: name.into(),
            detail: None,
            byte_range,
            line_range,
            depth,
            parent,
            children: Vec::new(),
            collapsed: false,
        }
    }

    /// True when this node has no children.
    pub fn is_leaf(&self) -> bool {
        self.children.is_empty()
    }
}

/// Arena-allocated tree of code nodes for a single source file.
#[derive(Debug, Default)]
pub struct CodeTree {
    /// Flat storage; index == node.id.
    nodes: Vec<CodeNode>,
    /// Root node index (always 0 when the tree is non-empty).
    pub root: Option<usize>,
}

impl CodeTree {
    pub fn new() -> Self {
        Self::default()
    }

    /// Allocate a new node and return its id.
    pub fn add_node(
        &mut self,
        kind: NodeKind,
        name: impl Into<String>,
        byte_range: (usize, usize),
        line_range: (usize, usize),
        depth: usize,
        parent: Option<usize>,
    ) -> usize {
        let id = self.nodes.len();
        let node = CodeNode::new(id, kind, name, byte_range, line_range, depth, parent);
        self.nodes.push(node);
        if let Some(pid) = parent {
            self.nodes[pid].children.push(id);
        }
        if self.root.is_none() {
            self.root = Some(id);
        }
        id
    }

    pub fn get(&self, id: usize) -> Option<&CodeNode> {
        self.nodes.get(id)
    }

    pub fn get_mut(&mut self, id: usize) -> Option<&mut CodeNode> {
        self.nodes.get_mut(id)
    }

    /// Toggle the collapsed state of a node.
    pub fn toggle_collapse(&mut self, id: usize) {
        if let Some(n) = self.nodes.get_mut(id) {
            n.collapsed = !n.collapsed;
        }
    }

    /// Set detail text for a node.
    #[allow(dead_code)]
    pub fn set_detail(&mut self, id: usize, detail: impl Into<String>) {
        if let Some(n) = self.nodes.get_mut(id) {
            n.detail = Some(detail.into());
        }
    }

    /// Return the visible nodes in DFS pre-order, respecting `collapsed` flags.
    pub fn visible_nodes(&self) -> Vec<&CodeNode> {
        let Some(root) = self.root else {
            return Vec::new();
        };
        let mut result = Vec::new();
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            let node = &self.nodes[id];
            result.push(node);
            if !node.collapsed {
                // Push children in reverse so the first child is visited first.
                for &child in node.children.iter().rev() {
                    stack.push(child);
                }
            }
        }
        result
    }

    /// Return all nodes (visible or not) in DFS pre-order.
    pub fn all_nodes_dfs(&self) -> Vec<&CodeNode> {
        let Some(root) = self.root else {
            return Vec::new();
        };
        let mut result = Vec::new();
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            let node = &self.nodes[id];
            result.push(node);
            for &child in node.children.iter().rev() {
                stack.push(child);
            }
        }
        result
    }

    /// Filter visible nodes by a text pattern (case-insensitive substring match
    /// on name and detail).  Returns indices into the `visible_nodes()` list.
    pub fn filter_visible<'a>(
        &'a self,
        pattern: &str,
    ) -> Vec<&'a CodeNode> {
        let pat = pattern.to_ascii_lowercase();
        self.visible_nodes()
            .into_iter()
            .filter(|n| {
                n.name.to_ascii_lowercase().contains(&pat)
                    || n.detail
                        .as_deref()
                        .map(|d| d.to_ascii_lowercase().contains(&pat))
                        .unwrap_or(false)
            })
            .collect()
    }

    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_tree() -> CodeTree {
        let mut tree = CodeTree::new();
        let root = tree.add_node(NodeKind::File, "main.rs", (0, 100), (0, 20), 0, None);
        let fn1 = tree.add_node(NodeKind::Function, "fn_foo", (0, 50), (0, 10), 1, Some(root));
        let _ln1 = tree.add_node(NodeKind::Line, "let x = 1;", (0, 20), (0, 0), 2, Some(fn1));
        let _fn2 = tree.add_node(NodeKind::Function, "fn_bar", (51, 100), (11, 20), 1, Some(root));
        tree
    }

    #[test]
    fn test_tree_structure() {
        let tree = sample_tree();
        assert_eq!(tree.len(), 4);
        assert_eq!(tree.root, Some(0));

        let root = tree.get(0).unwrap();
        assert_eq!(root.children.len(), 2);
        assert_eq!(root.kind, NodeKind::File);
    }

    #[test]
    fn test_visible_nodes_full() {
        let tree = sample_tree();
        let vis = tree.visible_nodes();
        assert_eq!(vis.len(), 4);
    }

    #[test]
    fn test_collapse_hides_subtree() {
        let mut tree = sample_tree();
        // Collapse fn_foo (id=1) → its child Line should be hidden.
        tree.toggle_collapse(1);
        let vis = tree.visible_nodes();
        assert_eq!(vis.len(), 3); // root, fn_foo, fn_bar  (line hidden)
    }

    #[test]
    fn test_collapse_root_hides_all_children() {
        let mut tree = sample_tree();
        tree.toggle_collapse(0);
        let vis = tree.visible_nodes();
        assert_eq!(vis.len(), 1); // only root
    }

    #[test]
    fn test_double_toggle_restores() {
        let mut tree = sample_tree();
        tree.toggle_collapse(1);
        tree.toggle_collapse(1);
        assert_eq!(tree.visible_nodes().len(), 4);
    }

    #[test]
    fn test_filter_visible() {
        let tree = sample_tree();
        let res = tree.filter_visible("foo");
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].name, "fn_foo");
    }

    #[test]
    fn test_filter_visible_empty_pattern() {
        let tree = sample_tree();
        assert_eq!(tree.filter_visible("").len(), 4);
    }

    #[test]
    fn test_filter_case_insensitive() {
        let tree = sample_tree();
        assert_eq!(tree.filter_visible("FN_FOO").len(), 1);
    }

    #[test]
    fn test_node_is_leaf() {
        let tree = sample_tree();
        assert!(!tree.get(0).unwrap().is_leaf());
        assert!(tree.get(2).unwrap().is_leaf()); // Line node
    }
}
