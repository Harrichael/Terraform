/// The zoomable view lives in the `coalesce` crate; same alias trick as
/// `entity` below.
pub mod cursor {
    pub use coalesce::*;
}
/// The model lives in the `entity-graph` crate; this alias keeps the TUI's
/// `crate::graph::entity::*` paths valid.
pub mod entity {
    pub use entity_graph::*;
}
pub mod navigator;
pub mod tree;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod smoke_tests;
