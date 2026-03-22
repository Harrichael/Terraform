/// Hierarchical code node kinds, ordered from coarsest to finest granularity.
/// `SymRef` is a special non-granularity kind for symbolic references.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NodeKind {
    Folder,
    Module,
    File,
    Class,
    Function,
    Block,
    Line,
    /// A symbolic reference to another node (not a granularity level).
    SymRef,
}

impl NodeKind {
    /// Granularity level: lower number = coarser detail. `None` for SymRef.
    pub fn level(&self) -> Option<u8> {
        match self {
            NodeKind::Folder => Some(0),
            NodeKind::Module => Some(1),
            NodeKind::File => Some(2),
            NodeKind::Class => Some(3),
            NodeKind::Function => Some(4),
            NodeKind::Block => Some(5),
            NodeKind::Line => Some(6),
            NodeKind::SymRef => None,
        }
    }

    /// One step coarser. Returns `None` if already at the coarsest level.
    pub fn coarser(&self) -> Option<NodeKind> {
        match self {
            NodeKind::Module => Some(NodeKind::Folder),
            NodeKind::File => Some(NodeKind::Module),
            NodeKind::Class => Some(NodeKind::File),
            NodeKind::Function => Some(NodeKind::Class),
            NodeKind::Block => Some(NodeKind::Function),
            NodeKind::Line => Some(NodeKind::Block),
            NodeKind::Folder | NodeKind::SymRef => None,
        }
    }

    /// One step finer. Returns `None` if already at the finest level.
    pub fn finer(&self) -> Option<NodeKind> {
        match self {
            NodeKind::Folder => Some(NodeKind::Module),
            NodeKind::Module => Some(NodeKind::File),
            NodeKind::File => Some(NodeKind::Class),
            NodeKind::Class => Some(NodeKind::Function),
            NodeKind::Function => Some(NodeKind::Block),
            NodeKind::Block => Some(NodeKind::Line),
            NodeKind::Line | NodeKind::SymRef => None,
        }
    }

    /// True if this kind is at a finer granularity than `other`.
    pub fn is_finer_than(&self, other: &NodeKind) -> bool {
        match (self.level(), other.level()) {
            (Some(a), Some(b)) => a > b,
            _ => false,
        }
    }
}

impl std::fmt::Display for NodeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            NodeKind::Folder => "folder",
            NodeKind::Module => "module",
            NodeKind::File => "file",
            NodeKind::Class => "class/struct",
            NodeKind::Function => "fn/method",
            NodeKind::Block => "block",
            NodeKind::Line => "line",
            NodeKind::SymRef => "symref",
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
    /// Whether this node is currently collapsed (all children hidden).
    pub collapsed: bool,
    /// Granularity limit: only show children whose kind is at most this
    /// level of detail. `None` means "show all children".
    pub granularity_limit: Option<NodeKind>,
    /// For `SymRef` nodes: the ID of the canonical definition this references.
    pub sym_ref_target: Option<usize>,
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
            granularity_limit: None,
            sym_ref_target: None,
        }
    }

    /// True when this node has no children.
    pub fn is_leaf(&self) -> bool {
        self.children.is_empty()
    }
}

/// Arena-allocated tree of code nodes for a single file or directory workspace.
#[derive(Debug, Default)]
pub struct CodeTree {
    /// Flat storage; index == node.id.
    nodes: Vec<CodeNode>,
    /// Root node index (always 0 when the tree is non-empty).
    pub root: Option<usize>,
    /// IDs of nodes that serve as canonical "library" definitions —
    /// referenced by one or more `SymRef` nodes in the main tree.
    pub lib_nodes: Vec<usize>,
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

    /// Expand the granularity of node `id`: show one finer level of children.
    ///
    /// If currently showing all children (no limit), this is a no-op.
    /// If there is a limit, it moves one step finer; reaching the finest
    /// level removes the limit entirely (show everything).
    pub fn expand_granularity(&mut self, id: usize) {
        if let Some(node) = self.nodes.get_mut(id) {
            match &node.granularity_limit {
                None => {} // Already at maximum detail.
                Some(current) => {
                    // Move one step finer; None means "remove limit" (show all).
                    node.granularity_limit = current.finer();
                    node.collapsed = false;
                }
            }
        }
    }

    /// Shrink the granularity of node `id`: hide one finer level of children.
    ///
    /// Steps from "show all" → Block → Function → Class → File → Module →
    /// Folder → collapsed.
    pub fn shrink_granularity(&mut self, id: usize) {
        if let Some(node) = self.nodes.get_mut(id) {
            // Clone the current limit to avoid a simultaneous borrow conflict.
            let current_limit = node.granularity_limit.clone();
            match current_limit {
                None => {
                    // Currently showing all — hide Lines.
                    node.granularity_limit = Some(NodeKind::Block);
                }
                Some(current) => {
                    match current.coarser() {
                        Some(coarser) => {
                            node.granularity_limit = Some(coarser);
                        }
                        None => {
                            // Already at Folder level — collapse completely.
                            node.collapsed = true;
                            node.granularity_limit = None;
                        }
                    }
                }
            }
        }
    }

    /// Register `id` as a canonical library definition.
    pub fn mark_as_lib(&mut self, id: usize) {
        if !self.lib_nodes.contains(&id) {
            self.lib_nodes.push(id);
        }
    }

    /// Add a SymRef node pointing to `target_id` as a child of `parent_id`.
    #[allow(dead_code)]
    pub fn add_sym_ref(
        &mut self,
        name: impl Into<String>,
        target_id: usize,
        depth: usize,
        parent_id: usize,
    ) -> usize {
        let id = self.add_node(
            NodeKind::SymRef,
            name,
            (0, 0),
            (0, 0),
            depth,
            Some(parent_id),
        );
        if let Some(n) = self.nodes.get_mut(id) {
            n.sym_ref_target = Some(target_id);
        }
        id
    }

    /// Set detail text for a node.
    #[allow(dead_code)]
    pub fn set_detail(&mut self, id: usize, detail: impl Into<String>) {
        if let Some(n) = self.nodes.get_mut(id) {
            n.detail = Some(detail.into());
        }
    }

    /// Return the visible nodes in DFS pre-order, respecting `collapsed` and
    /// `granularity_limit` flags.
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
                for &child in node.children.iter().rev() {
                    let child_node = &self.nodes[child];
                    // Respect the per-node granularity limit.
                    if let Some(ref limit) = node.granularity_limit {
                        if child_node.kind.is_finer_than(limit) {
                            continue;
                        }
                    }
                    stack.push(child);
                }
            }
        }
        result
    }

    /// Return the visible lib (library/canonical) nodes.
    pub fn visible_lib_nodes(&self) -> Vec<&CodeNode> {
        self.lib_nodes
            .iter()
            .filter_map(|&id| self.nodes.get(id))
            .collect()
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
    /// on name and detail).
    pub fn filter_visible<'a>(&'a self, pattern: &str) -> Vec<&'a CodeNode> {
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
        tree.toggle_collapse(1);
        let vis = tree.visible_nodes();
        assert_eq!(vis.len(), 3); // root, fn_foo, fn_bar (line hidden)
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
    fn test_granularity_limit_hides_fine_children() {
        let mut tree = sample_tree();
        // Set fn_foo (id=1) to only show children up to Function level.
        // fn_foo's direct children are Lines (level 6 > Function level 4) → hidden.
        tree.get_mut(1).unwrap().granularity_limit = Some(NodeKind::Function);
        let vis = tree.visible_nodes();
        // root + fn_foo (its Line child hidden) + fn_bar = 3
        assert_eq!(vis.len(), 3);
        assert!(vis.iter().all(|n| n.kind != NodeKind::Line));
    }

    #[test]
    fn test_granularity_limit_none_shows_all() {
        let mut tree = sample_tree();
        tree.get_mut(0).unwrap().granularity_limit = None;
        assert_eq!(tree.visible_nodes().len(), 4);
    }

    #[test]
    fn test_shrink_granularity_from_none() {
        let mut tree = sample_tree();
        // Shrink from None → Block limit (hide Lines)
        tree.shrink_granularity(0);
        let node = tree.get(0).unwrap();
        assert_eq!(node.granularity_limit, Some(NodeKind::Block));
    }

    #[test]
    fn test_shrink_granularity_from_block_to_function() {
        let mut tree = sample_tree();
        tree.get_mut(0).unwrap().granularity_limit = Some(NodeKind::Block);
        tree.shrink_granularity(0);
        let node = tree.get(0).unwrap();
        assert_eq!(node.granularity_limit, Some(NodeKind::Function));
    }

    #[test]
    fn test_expand_granularity_from_function_to_block() {
        let mut tree = sample_tree();
        tree.get_mut(0).unwrap().granularity_limit = Some(NodeKind::Function);
        tree.expand_granularity(0);
        let node = tree.get(0).unwrap();
        assert_eq!(node.granularity_limit, Some(NodeKind::Block));
    }

    #[test]
    fn test_expand_granularity_from_block_to_line() {
        // Block.finer() = Some(Line), so expanding from Block gives Line limit.
        let mut tree = sample_tree();
        tree.get_mut(0).unwrap().granularity_limit = Some(NodeKind::Block);
        tree.expand_granularity(0);
        let node = tree.get(0).unwrap();
        assert_eq!(node.granularity_limit, Some(NodeKind::Line));
    }

    #[test]
    fn test_expand_granularity_from_line_to_none() {
        // Line.finer() = None, so expanding from Line removes the limit.
        let mut tree = sample_tree();
        tree.get_mut(0).unwrap().granularity_limit = Some(NodeKind::Line);
        tree.expand_granularity(0);
        let node = tree.get(0).unwrap();
        assert_eq!(node.granularity_limit, None);
    }

    #[test]
    fn test_expand_granularity_from_none_noop() {
        let mut tree = sample_tree();
        tree.expand_granularity(0);
        assert_eq!(tree.get(0).unwrap().granularity_limit, None);
    }

    #[test]
    fn test_sym_ref_nodes() {
        let mut tree = sample_tree();
        // Add a SymRef to fn_foo (id=1) as a child of fn_bar (id=3)
        let ref_id = tree.add_sym_ref("fn_foo", 1, 2, 3);
        let ref_node = tree.get(ref_id).unwrap();
        assert_eq!(ref_node.kind, NodeKind::SymRef);
        assert_eq!(ref_node.sym_ref_target, Some(1));
    }

    #[test]
    fn test_lib_nodes() {
        let mut tree = sample_tree();
        tree.mark_as_lib(1); // fn_foo is a lib definition
        let lib = tree.visible_lib_nodes();
        assert_eq!(lib.len(), 1);
        assert_eq!(lib[0].id, 1);
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

    #[test]
    fn test_nodekind_level_ordering() {
        assert!(NodeKind::Folder.level() < NodeKind::Line.level());
        assert!(NodeKind::Function.level() < NodeKind::Block.level());
        assert!(NodeKind::SymRef.level().is_none());
    }

    #[test]
    fn test_nodekind_coarser_finer() {
        assert_eq!(NodeKind::Function.coarser(), Some(NodeKind::Class));
        assert_eq!(NodeKind::Function.finer(), Some(NodeKind::Block));
        assert_eq!(NodeKind::Folder.coarser(), None);
        assert_eq!(NodeKind::Line.finer(), None);
    }

    #[test]
    fn test_is_finer_than() {
        assert!(NodeKind::Line.is_finer_than(&NodeKind::Function));
        assert!(!NodeKind::File.is_finer_than(&NodeKind::Function));
        assert!(!NodeKind::SymRef.is_finer_than(&NodeKind::Line));
    }
}
