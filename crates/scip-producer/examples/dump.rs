//! Print what a SCIP index actually contains, for checking a new indexer's
//! conventions (roles, enclosing ranges, kinds, symbol shapes).
//! `cargo run -p scip-producer --example dump -- <index.scip> [path-substring]`

use std::collections::BTreeSet;

use protobuf::Message;
use scip::types::{Index, SymbolRole};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let index = Index::parse_from_bytes(&std::fs::read(&args[1])?)?;
    let filter = args.get(2).cloned().unwrap_or_default();

    println!(
        "tool: {} {}",
        index.metadata.tool_info.name, index.metadata.tool_info.version
    );
    println!("project_root: {}", index.metadata.project_root);
    println!("documents: {}", index.documents.len());

    let mut encodings = BTreeSet::new();
    let mut kinds = BTreeSet::new();
    let (mut occs, mut defs, mut imports, mut enclosing) = (0, 0, 0, 0);
    for doc in &index.documents {
        encodings.insert(format!("{:?}", doc.position_encoding));
        for occ in &doc.occurrences {
            occs += 1;
            defs += usize::from(occ.symbol_roles & SymbolRole::Definition as i32 != 0);
            imports += usize::from(occ.symbol_roles & SymbolRole::Import as i32 != 0);
            enclosing += usize::from(!occ.enclosing_range.is_empty());
        }
        for si in &doc.symbols {
            kinds.insert(format!("{:?}", si.kind));
        }
    }
    println!("encodings: {encodings:?}");
    println!(
        "occurrences: {occs}, definitions: {defs}, import-role: {imports}, with enclosing_range: {enclosing}"
    );
    println!("kinds: {kinds:?}");

    for doc in index
        .documents
        .iter()
        .filter(|d| d.relative_path.contains(&filter))
    {
        println!("\n=== {} ===", doc.relative_path);
        for si in &doc.symbols {
            println!(
                "  SYM kind={:?} display={:?} enclosing={:?} {}",
                si.kind, si.display_name, si.enclosing_symbol, si.symbol
            );
        }
        for occ in &doc.occurrences {
            println!(
                "  OCC {:?} roles={} encl={:?} {}",
                occ.range, occ.symbol_roles, occ.enclosing_range, occ.symbol
            );
        }
    }
    Ok(())
}
