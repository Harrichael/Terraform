//! Print the EntityGraph produced from an index, for eyeballing a new indexer.
//! `cargo run -p scip-producer --example graph -- <index.scip> <project_root>`

use std::path::Path;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let graph = scip_producer::graph_from_index(Path::new(&args[1]), Path::new(&args[2]))?;
    println!(
        "entities: {}  references: {}",
        graph.entities.len(),
        graph.references.len()
    );
    for e in &graph.entities {
        println!(
            "  [{}] {:<9} {}  lines {:?} bytes {:?} parent={:?}",
            e.id.0,
            e.kind.to_string(),
            e.path.display(),
            e.line_range,
            e.byte_range,
            e.parent.map(|p| p.0)
        );
    }
    for r in &graph.references {
        let name = |id: entity_graph::EntityId| graph.entities[id.0].path.display().to_string();
        println!("  {} -> {} [{}]", name(r.from), name(r.to), r.kind);
    }
    Ok(())
}
