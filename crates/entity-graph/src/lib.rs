//! The entity graph: a codebase reduced to two structures.
//!
//! 1. **Containment**: Folder → Module → File → Class → Function, stored as
//!    parent/children links on each [`Entity`]. Ids are arena indices, so a
//!    graph is immutable once built and ids are stable for its lifetime.
//! 2. **References**: directed, kinded edges between entities, independent
//!    of containment.
//!
//! This crate is the contract between *producers* (anything that builds an
//! [`EntityGraph`] from a codebase) and *consumers* (views, analysis, UIs).
//! Producers are plain functions in their own crates and must agree on:
//!
//! - one root entity, a Folder named after the project directory (or a File
//!   when a single file was loaded);
//! - references deduplicated on `(from, to, kind)`, self-loops dropped;
//! - `byte_range`/`line_range` are `0..0` when not applicable to the kind.
//!
//! Consumers never learn which producer built the graph.

mod model;

pub use model::*;

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
