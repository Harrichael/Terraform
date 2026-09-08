//! Throwaway localhost viewer for an `EntityGraph`. See `ui/CONTRACT.md`.

mod dto;
mod handlers;
#[cfg(feature = "scip")]
mod index;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use clap::Parser;
use entity_graph::EntityGraph;
use tempfile::TempDir;
use tiny_http::{Header, Response, Server as HttpServer};

use crate::handlers::Server;

const INDEX_HTML: &str = include_str!("../ui/index.html");

#[derive(Parser)]
#[command(name = "graph-server", about = "Serve an entity graph to the browser")]
struct Args {
    #[arg(long, default_value_t = 7878)]
    port: u16,
    /// Build the graph from a SCIP index instead of tree-sitter (needs `--features scip`).
    #[arg(long, value_name = "index.scip")]
    scip: Option<PathBuf>,
    /// Generate the SCIP index (rust-analyzer, scip-typescript or scip-go, by
    /// manifest) under the system temp dir and build the graph from it. With
    /// --diff both sides are indexed.
    #[arg(long, conflicts_with = "scip")]
    scip_index: bool,
    /// Serve the union of this git ref and the working tree, tagged by change.
    #[arg(long, value_name = "ref", conflicts_with = "scip")]
    diff: Option<String>,
    /// Project root (or single file) to load.
    path: PathBuf,
}

fn load_graph(args: &Args, root: &Path) -> Result<EntityGraph> {
    match &args.scip {
        Some(index) => load_scip(index, root),
        None if args.scip_index => load_scip(&working_tree_index(root)?, root),
        None => load_treesitter(root),
    }
}

#[cfg(feature = "scip")]
use index::{base_index, working_tree_index};

#[cfg(not(feature = "scip"))]
fn working_tree_index(_root: &Path) -> Result<PathBuf> {
    anyhow::bail!("--scip-index requires a binary built with `--features scip`")
}

#[cfg(not(feature = "scip"))]
fn base_index(_tree: &Path, _commit: &str, _root: &Path) -> Result<PathBuf> {
    anyhow::bail!("--scip-index requires a binary built with `--features scip`")
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

/// The base tree at `base_ref`, extracted under a directory named like the
/// working tree: both producers name the root after its directory, and the
/// diff only matches trees whose roots agree.
fn extract_base(repo: &Path, base_ref: &str, root_name: &Path) -> Result<(TempDir, PathBuf)> {
    let tmp = TempDir::new().context("creating a temp dir for the base tree")?;
    let dest = tmp.path().join(root_name);
    std::fs::create_dir(&dest).with_context(|| format!("creating {}", dest.display()))?;

    let mut archive = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["archive", base_ref])
        .stdout(Stdio::piped())
        .spawn()
        .context("running `git archive` (is git installed?)")?;
    let stdout = archive.stdout.take().expect("stdout was piped");
    let untar = Command::new("tar")
        .arg("-x")
        .arg("-C")
        .arg(&dest)
        .stdin(stdout)
        .spawn()
        .context("running `tar -x`")?
        .wait()?;
    let archived = archive.wait()?;
    if !archived.success() {
        bail!("`git archive {base_ref}` failed ({archived})");
    }
    if !untar.success() {
        bail!("unpacking the base tree failed ({untar})");
    }
    Ok((tmp, dest))
}

fn base_commit(repo: &Path, base_ref: &str) -> Result<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "--verify", &format!("{base_ref}^{{commit}}")])
        .output()
        .context("running `git rev-parse` (is git installed?)")?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        bail!("{base_ref} is not a commit in {}: {}", repo.display(), stderr.trim());
    }
    Ok(String::from_utf8(out.stdout)?.trim().to_string())
}

fn load_diff(base_ref: &str, root: PathBuf, scip_index: bool) -> Result<Server> {
    if !root.is_dir() {
        bail!("--diff needs a project directory; {} is a file", root.display());
    }
    let name = root.file_name().map(Path::new).context("the project root has no name")?;
    let commit = base_commit(&root, base_ref)?;
    let (tree, old_root) = extract_base(&root, base_ref, name)?;

    let (old, new) = if scip_index {
        // The base index first: it is the one that can be reused across runs,
        // so a failure there is reported before the working tree is rebuilt.
        let old = load_scip(&base_index(&old_root, &commit, &root)?, &old_root)?;
        (old, load_scip(&working_tree_index(&root)?, &root)?)
    } else {
        (load_treesitter(&old_root)?, load_treesitter(&root)?)
    };
    println!(
        "Diffing {base_ref} ({commit}) → working tree: base {} entities, working {} entities",
        old.entities.len(),
        new.entities.len(),
    );
    let diff = graph_diff::diff(&old, &old_root, &new, &root)?;
    let base = base_ref.to_string();
    Ok(Server::with_diff(diff, base, commit, Some(tree), root, INDEX_HTML))
}

fn main() -> Result<()> {
    let args = Args::parse();
    let root = args
        .path
        .canonicalize()
        .with_context(|| format!("resolving {}", args.path.display()))?;
    let server = match &args.diff {
        Some(base_ref) => load_diff(base_ref, root, args.scip_index)?,
        None => Server::new(load_graph(&args, &root)?, root, INDEX_HTML),
    };

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

#[cfg(test)]
mod tests {
    use super::*;

    /// A SCIP index is one build of one tree, so there is no base side to
    /// diff against; clap rejects the pair rather than the loader.
    #[test]
    fn diff_and_scip_cannot_be_combined() {
        assert!(Args::try_parse_from(["graph-server", "--diff", "HEAD", "."]).is_ok());
        assert!(Args::try_parse_from(["graph-server", "--scip", "index.scip", "."]).is_ok());
        let both =
            Args::try_parse_from(["graph-server", "--diff", "HEAD", "--scip", "index.scip", "."]);
        assert!(both.is_err(), "--diff with --scip must be rejected");
    }

    /// `--scip-index` is the generated counterpart of `--scip`, so the two
    /// are exclusive; unlike `--scip` it does combine with `--diff`.
    #[test]
    fn scip_index_replaces_scip_and_allows_diff() {
        assert!(Args::try_parse_from(["graph-server", "--scip-index", "."]).is_ok());
        assert!(Args::try_parse_from(["graph-server", "--diff", "HEAD", "--scip-index", "."]).is_ok());
        assert!(Args::try_parse_from(["graph-server", "--scip-index", "--scip", "i.scip", "."]).is_err());
    }
}
