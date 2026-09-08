use std::collections::HashMap;

use anyhow::{Result, bail};
use entity_graph::{EntityGraph, EntityId, EntityKind};

/// Which entity on one side stands for which on the other.
pub(crate) struct Matching {
    pub old_to_new: HashMap<EntityId, EntityId>,
    pub new_to_old: HashMap<EntityId, EntityId>,
    pub new_root: EntityId,
}

/// Match the two hierarchies top-down: the roots pair up, and under a matched
/// pair a child matches the child of the other side with the same
/// `(name, kind)` and the same ordinal among its same-name same-kind siblings.
///
/// Descending only into already-matched parents is what keeps the union a
/// tree: an unmatched entity's whole subtree is unmatched too, so it can be
/// grafted wholesale under its parent's counterpart.
pub(crate) fn match_trees(old: &EntityGraph, new: &EntityGraph) -> Result<Matching> {
    let old_root = root_of(old, "base")?;
    let new_root = root_of(new, "working")?;
    let (old_name, new_name) = (&old.entities[old_root.0].name, &new.entities[new_root.0].name);
    if old_name != new_name {
        bail!(
            "the two trees have different root names ({old_name} vs {new_name}); \
             both sides must be loaded under the same directory name"
        );
    }

    let mut old_to_new = HashMap::new();
    let mut new_to_old = HashMap::new();
    let mut pending = vec![(old_root, new_root)];
    while let Some((o, n)) = pending.pop() {
        old_to_new.insert(o, n);
        new_to_old.insert(n, o);

        let mut by_key: HashMap<(&str, EntityKind, usize), EntityId> = HashMap::new();
        for (key, child) in sibling_keys(old, o) {
            by_key.insert(key, child);
        }
        for (key, child) in sibling_keys(new, n) {
            if let Some(counterpart) = by_key.remove(&key) {
                pending.push((counterpart, child));
            }
        }
    }
    Ok(Matching { old_to_new, new_to_old, new_root })
}

fn sibling_keys(
    graph: &EntityGraph,
    parent: EntityId,
) -> impl Iterator<Item = ((&str, EntityKind, usize), EntityId)> {
    let mut ordinals: HashMap<(&str, EntityKind), usize> = HashMap::new();
    graph.entities[parent.0].children.iter().map(move |&child| {
        let e = &graph.entities[child.0];
        let ordinal = ordinals.entry((e.name.as_str(), e.kind)).or_insert(0);
        *ordinal += 1;
        ((e.name.as_str(), e.kind, *ordinal - 1), child)
    })
}

fn root_of(graph: &EntityGraph, side: &str) -> Result<EntityId> {
    let mut roots = graph.entities.iter().filter(|e| e.parent.is_none());
    let root = match roots.next() {
        Some(r) => r.id,
        None => bail!("the {side} graph has no root entity"),
    };
    if roots.next().is_some() {
        bail!("the {side} graph has more than one root entity");
    }
    Ok(root)
}
