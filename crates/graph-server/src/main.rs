//! Throwaway localhost viewer for an `EntityGraph`. See `ui/CONTRACT.md`.

mod dto;
mod handlers;

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use entity_graph::EntityGraph;
use tiny_http::{Header, Response, Server as HttpServer};

use crate::handlers::Server;

#[derive(Parser)]
#[command(name = "graph-server", about = "Serve an entity graph to the browser")]
struct Args {
    #[arg(long, default_value_t = 7878)]
    port: u16,
    /// Build the graph from a SCIP index instead of tree-sitter (needs `--features scip`).
    #[arg(long, value_name = "index.scip")]
    scip: Option<PathBuf>,
    /// Project root (or single file) to load.
    path: PathBuf,
}

fn load_graph(args: &Args) -> Result<EntityGraph> {
    match &args.scip {
        Some(index) => load_scip(index, &args.path),
        None => load_treesitter(&args.path),
    }
}

#[cfg(feature = "scip")]
fn load_scip(index: &std::path::Path, root: &std::path::Path) -> Result<EntityGraph> {
    scip_producer::graph_from_index(index, root)
        .with_context(|| format!("loading SCIP index {}", index.display()))
}

#[cfg(not(feature = "scip"))]
fn load_scip(_index: &std::path::Path, _root: &std::path::Path) -> Result<EntityGraph> {
    anyhow::bail!("--scip requires a binary built with `--features scip`")
}

#[cfg(feature = "treesitter")]
fn load_treesitter(root: &std::path::Path) -> Result<EntityGraph> {
    treesitter_producer::graph_from_path(root)
        .with_context(|| format!("parsing {}", root.display()))
}

#[cfg(not(feature = "treesitter"))]
fn load_treesitter(_root: &std::path::Path) -> Result<EntityGraph> {
    anyhow::bail!("this binary was built without the `treesitter` feature; pass --scip <index.scip>")
}

fn main() -> Result<()> {
    let args = Args::parse();
    let graph = load_graph(&args)?;
    let server = Server::new(graph, include_str!("../ui/index.html"));

    let addr = format!("127.0.0.1:{}", args.port);
    let http = HttpServer::http(&addr).map_err(|e| anyhow::anyhow!("binding {addr}: {e}"))?;
    println!(
        "Serving {} ({} entities, {} references) at http://{addr}/",
        dto::root_name(server.graph()),
        server.graph().entities.len(),
        server.graph().references.len(),
    );

    for request in http.incoming_requests() {
        let method = request.method().as_str().to_string();
        let url = request.url().to_string();
        let resp = server.respond(&method, &url);
        let header = Header::from_bytes("Content-Type", resp.content_type)
            .expect("static content-type strings are valid header values");
        let http_resp = Response::from_data(resp.body)
            .with_status_code(resp.status)
            .with_header(header);
        if let Err(e) = request.respond(http_resp) {
            eprintln!("failed to write response for {method} {url}: {e}");
        }
    }
    Ok(())
}
