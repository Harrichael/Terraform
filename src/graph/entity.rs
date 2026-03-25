use std::path::PathBuf;

/// Unique identifier for an entity within the graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EntityId(pub usize);

/// Unique identifier for a reference within the graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ReferenceId(pub usize);

/// Kind of code construct that an entity represents.
/// These form a hierarchy from coarsest (Folder) to finest (Function).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EntityKind {
    Folder,
    Module,
    File,
    Class,
    Function,
}

impl std::fmt::Display for EntityKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            EntityKind::Folder => "folder",
            EntityKind::Module => "module",
            EntityKind::File => "file",
            EntityKind::Class => "class/struct",
            EntityKind::Function => "fn/method",
        };
        write!(f, "{s}")
    }
}

/// Kind of symbolic relationship between two entities.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReferenceKind {
    /// A function or method call.
    Call,
    /// An import or `use` statement.
    Import,
    /// A type annotation or type usage.
    TypeRef,
    /// A variable read or write.
    VarRef,
    /// Any other symbolic reference.
    Generic,
}

impl std::fmt::Display for ReferenceKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            ReferenceKind::Call => "call",
            ReferenceKind::Import => "import",
            ReferenceKind::TypeRef => "type_ref",
            ReferenceKind::VarRef => "var_ref",
            ReferenceKind::Generic => "generic",
        };
        write!(f, "{s}")
    }
}

/// A directed symbolic reference edge from one entity to another.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Reference {
    pub from: EntityId,
    pub to: EntityId,
    pub kind: ReferenceKind,
}

/// A single entity (code construct) in the graph.
#[derive(Debug, Clone)]
pub struct Entity {
    pub id: EntityId,
    pub kind: EntityKind,
    pub name: String,

    // CONTAINMENT RELATIONS
    pub parent: Option<EntityId>,
    pub children: Vec<EntityId>,

    // SOURCE LOCATION
    pub path: PathBuf,
    // !! Ranges are 0, 0 if Not Applicable for EntityKind !!
    pub byte_range: std::ops::Range<usize>,
    pub line_range: std::ops::Range<usize>,
}

/// Graph of code entities and their symbolic references.
///
/// Stores two independent structures:
///
/// 1. **Contains topology** — the parent/child relationships captured in each
///    [`Entity`], forming the syntactic hierarchy (Folder → Module → File →
///    Class → Function).
///
/// 2. **Reference graph** — directed edges representing symbolic relationships
///    (calls, imports, type refs, etc.) between entities, independent of
///    containment.
pub struct EntityGraph {
    /// Arena-allocated entities; index == entity.id.0
    pub entities: Vec<Entity>,
    /// Directed symbolic reference edges.
    pub references: Vec<Reference>,
}

impl EntityGraph {
    /// Get an entity by ID.
    pub fn get(&self, id: EntityId) -> Option<&Entity> {
        self.entities.get(id.0)
    }
}
