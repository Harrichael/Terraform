//! Request handling as a pure function of (method, path) -> response, so the
//! whole contract is testable without opening a socket. `main.rs` is the only
//! thing that knows about `tiny_http`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use coalesce::Cursor;
use entity_graph::{EntityGraph, EntityId};
use graph_diff::{FileDiff, GraphDiff, Status};
use tempfile::TempDir;

use crate::dto::{self, DiffView, ErrorDto, GraphDto, SourceDto};
use crate::text_index::{Limits, TextIndex};

pub struct Response {
    pub status: u16,
    pub content_type: &'static str,
    pub body: Vec<u8>,
}

const JSON: &str = "application/json";
const HTML: &str = "text/html; charset=utf-8";
const JS: &str = "application/javascript; charset=utf-8";

/// The ES modules `index.html` imports, served at `/ui/<name>` and embedded
/// in the binary the same way `index_html` is.
const UI_MODULES: &[(&str, &str)] = &[
    ("common.js", include_str!("../ui/common.js")),
    ("nodes.js", include_str!("../ui/nodes.js")),
    ("model.js", include_str!("../ui/model.js")),
    ("layout.js", include_str!("../ui/layout.js")),
    ("code.js", include_str!("../ui/code.js")),
    ("panel.js", include_str!("../ui/panel.js")),
    ("search.js", include_str!("../ui/search.js")),
    ("app.js", include_str!("../ui/app.js")),
];

/// Largest source file `/source` will serve; the viewer renders the whole
/// file in one `<pre>`, so anything bigger is refused rather than truncated.
const MAX_SOURCE_BYTES: u64 = 2 * 1024 * 1024;

/// `/search` result caps, one per kind.
const SEARCH_LIMITS: Limits = Limits { file: 30, path: 30, content: 100 };

/// Everything diff mode adds: the change tags for the union graph the server
/// is serving, and both texts of every changed file.
struct DiffState {
    base_label: String,
    base_commit: String,
    entity_status: Vec<Status>,
    reference_status: Vec<Status>,
    churn: Vec<(usize, usize)>,
    files: HashMap<EntityId, FileDiff>,
    // The extracted base tree is only read while diffing, but the graph's
    // paths still name it, so it is kept for the server's lifetime rather
    // than deleted under a running process. Absent when the caller owns it.
    _base_tree: Option<TempDir>,
}

pub struct Server {
    graph: EntityGraph,
    // The graph never changes after load, so its JSON is rendered once; on a
    // real repo it is by far the largest payload.
    graph_json: String,
    cursor: Mutex<Cursor>,
    // The project directory (or single file) the graph was loaded from;
    // `EntityGraph::file_path` results are resolved against it.
    root: PathBuf,
    diff: Option<DiffState>,
    index: TextIndex,
    index_html: &'static str,
}

impl Server {
    pub fn new(graph: EntityGraph, root: PathBuf, index_html: &'static str) -> Self {
        let graph_json = serde_json::to_string(&GraphDto::from(&graph))
            .expect("GraphDto serialization is infallible");
        let cursor = Mutex::new(Cursor::new(&graph));
        let index = build_text_index(&graph, &root, None);
        Server { graph, graph_json, cursor, root, diff: None, index, index_html }
    }

    /// Serve a diff: the union graph is the graph, the change tags ride along
    /// as side tables. `base_label` is the ref as the user typed it.
    pub fn with_diff(
        diff: GraphDiff,
        base_label: String,
        base_commit: String,
        base_tree: Option<TempDir>,
        root: PathBuf,
        index_html: &'static str,
    ) -> Self {
        let GraphDiff { graph, entity_status, reference_status, churn, files } = diff;
        let index = build_text_index(&graph, &root, Some(&files));
        let state = DiffState {
            base_label,
            base_commit,
            entity_status,
            reference_status,
            churn,
            files,
            _base_tree: base_tree,
        };
        let graph_json = serde_json::to_string(&dto::graph_dto_with_diff(&graph, &state.view()))
            .expect("GraphDto serialization is infallible");
        let cursor = Mutex::new(Cursor::new(&graph));
        Server { graph, graph_json, cursor, root, diff: Some(state), index, index_html }
    }

    pub fn graph(&self) -> &EntityGraph {
        &self.graph
    }

    pub fn respond(&self, method: &str, path_and_query: &str) -> Response {
        let (path, query) = match path_and_query.split_once('?') {
            Some((p, q)) => (p, Some(q)),
            None => (path_and_query, None),
        };

        match path {
            "/" | "/graph.json" | "/coalesced.json" | "/source" | "/search" if method != "GET" => {
                error(405, "method not allowed; use GET")
            }
            "/" => ok(HTML, self.index_html.as_bytes().to_vec()),
            "/graph.json" => ok(JSON, self.graph_json.clone().into_bytes()),
            "/coalesced.json" => self.coalesced_response(),
            "/source" => match parse_id(query) {
                Ok(id) => self.source(id),
                Err(msg) => error(400, msg),
            },
            "/search" => self.search(query),
            p if p.starts_with("/ui/") && method != "GET" => {
                error(405, "method not allowed; use GET")
            }
            p if p.starts_with("/ui/") => ui_module(p),

            "/coalesced/zoom-in" | "/coalesced/zoom-out" | "/coalesced/reset"
                if method != "POST" =>
            {
                error(405, "method not allowed; use POST")
            }
            "/coalesced/reset" => {
                *self.cursor.lock().unwrap() = Cursor::new(&self.graph);
                self.coalesced_response()
            }
            "/coalesced/zoom-in" => match parse_id(query) {
                Ok(id) => self.zoom(id, true),
                Err(msg) => error(400, msg),
            },
            "/coalesced/zoom-out" => match parse_id(query) {
                Ok(id) => self.zoom(id, false),
                Err(msg) => error(400, msg),
            },

            _ => error(404, &format!("no route for {method} {path}")),
        }
    }

    fn source(&self, id: EntityId) -> Response {
        let Some(rel) = self.graph.file_path(id) else {
            return error(404, &format!("entity {} has no source file", id.0));
        };
        let file_id = file_ancestor(&self.graph, id);
        match self.diff.as_ref().filter(|d| d.files.contains_key(&file_id)) {
            Some(d) => self.diff_source(file_id, &rel, d),
            None => match read_source(&self.root, &rel) {
                Ok(text) => {
                    let path = wire_path(&rel);
                    let dto = SourceDto { id: file_id.0, path, text, old_text: None, ops: None };
                    ok(JSON, serde_json::to_vec(&dto).unwrap())
                }
                Err(resp) => resp,
            },
        }
    }

    /// Both sides of a changed file, plus the line ops that interleave them.
    /// The texts come from the diff computed at load, not from disk: the ops
    /// only line up with the exact texts they were computed from, and the
    /// working tree may have moved on since. The side a one-sided file does
    /// not have is empty rather than an error.
    fn diff_source(&self, file_id: EntityId, rel: &Path, d: &DiffState) -> Response {
        let file = &d.files[&file_id];
        let dto = SourceDto {
            id: file_id.0,
            path: wire_path(rel),
            text: file.new_text.clone().unwrap_or_default(),
            old_text: Some(file.old_text.clone().unwrap_or_default()),
            ops: Some(file.ops.iter().map(dto::op_dto).collect()),
        };
        ok(JSON, serde_json::to_vec(&dto).unwrap())
    }

    /// `q` is percent-encoded, as query values are; a missing or non-UTF-8
    /// `q` is the only way this 400s, since an empty needle is a valid (if
    /// empty) search.
    fn search(&self, query: Option<&str>) -> Response {
        let raw = query.unwrap_or("").split('&').find_map(|kv| kv.strip_prefix("q="));
        let Some(raw) = raw else { return error(400, "missing query parameter `q`") };
        let q = match percent_decode(raw) {
            Ok(q) => q,
            Err(()) => return error(400, "query parameter `q` is not valid UTF-8"),
        };
        let result = self.index.search(&q, SEARCH_LIMITS);
        ok(JSON, serde_json::to_vec(&dto::search_dto(&result)).unwrap())
    }

    fn coalesced_response(&self) -> Response {
        let coalesced = self.cursor.lock().unwrap().coalesced();
        let status = self.diff.as_ref().map(|d| d.reference_status.as_slice());
        ok(JSON, serde_json::to_vec(&dto::coalesced_dto(&coalesced, status)).unwrap())
    }

    // A no-op move is a 409 rather than a 200 with the unchanged payload so the
    // UI can tell "nothing to expand here" apart from a successful zoom
    // without diffing leaf sets.
    fn zoom(&self, id: EntityId, down: bool) -> Response {
        let mut cursor = self.cursor.lock().unwrap();
        let moved = if down {
            cursor.move_down(id, &self.graph)
        } else {
            cursor.move_up(id, &self.graph)
        };
        if moved {
            drop(cursor);
            return self.coalesced_response();
        }

        let Some(entity) = self.graph.get(id) else {
            return error(409, &format!("unknown entity id {}", id.0));
        };
        if !cursor.leaves.contains(&id) {
            return error(409, &format!("entity {} is not a current leaf", id.0));
        }
        if down {
            error(409, &format!("entity {} has no children", id.0))
        } else {
            debug_assert!(entity.parent.is_none());
            error(409, &format!("entity {} is a root", id.0))
        }
    }
}

impl DiffState {
    fn view(&self) -> DiffView<'_> {
        DiffView {
            base: &self.base_label,
            base_commit: &self.base_commit,
            entity_status: &self.entity_status,
            reference_status: &self.reference_status,
            churn: &self.churn,
        }
    }
}

/// Read `rel` under `base`, refusing anything the viewer cannot render. The
/// error arm is the response to send.
fn read_source(base: &Path, rel: &Path) -> Result<String, Response> {
    let path = base.join(rel);

    // Both sides canonicalized so symlinked roots compare equal; the check
    // is what stops a hierarchy path containing `..` from escaping the
    // project (the producer contract forbids it, but the index is data).
    let (real, real_root) = match (path.canonicalize(), base.canonicalize()) {
        (Ok(p), Ok(r)) => (p, r),
        (Err(e), _) | (_, Err(e)) => {
            return Err(error(404, &format!("cannot read {}: {e}", path.display())));
        }
    };
    if !real.starts_with(&real_root) {
        return Err(error(403, &format!("{} is outside the project root", rel.display())));
    }

    let meta = match std::fs::metadata(&real) {
        Ok(m) => m,
        Err(e) => return Err(error(404, &format!("cannot read {}: {e}", path.display()))),
    };
    if meta.len() > MAX_SOURCE_BYTES {
        return Err(error(
            413,
            &format!("{} is {} bytes; limit is {MAX_SOURCE_BYTES}", rel.display(), meta.len()),
        ));
    }
    let bytes = match std::fs::read(&real) {
        Ok(b) => b,
        Err(e) => return Err(error(404, &format!("cannot read {}: {e}", path.display()))),
    };
    String::from_utf8(bytes)
        .map_err(|_| error(415, &format!("{} is not valid UTF-8", rel.display())))
}

/// Reads each File entity the same way `/source` would, so `/search` finds
/// exactly the text the code pane can open: in diff mode, a changed file's
/// new side, or the old side for a file that only exists on the base tree
/// (its line numbers are then old-side line numbers, matching where the code
/// pane opens a removed entity).
fn build_text_index(
    graph: &EntityGraph,
    root: &Path,
    diff_files: Option<&HashMap<EntityId, FileDiff>>,
) -> TextIndex {
    TextIndex::build(graph, |id| {
        let rel = graph.file_path(id)?;
        let text = match diff_files.and_then(|files| files.get(&id)) {
            Some(diff) => diff.new_text.clone().or_else(|| diff.old_text.clone())?,
            None => read_source(root, &rel).ok()?,
        };
        Some((wire_path(&rel), text))
    })
}

fn file_ancestor(graph: &EntityGraph, id: EntityId) -> EntityId {
    let mut cur = graph.get(id).expect("file_path succeeded, so the id exists");
    while cur.kind != entity_graph::EntityKind::File {
        cur = graph.get(cur.parent.expect("file_path found a File ancestor")).unwrap();
    }
    cur.id
}

fn wire_path(rel: &Path) -> String {
    rel.components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// `+` decodes to space and `%XX` (either hex case) to its byte; a `%` not
/// followed by two hex digits, including at the end of the string, is kept
/// literally rather than treated as an error.
fn percent_decode(s: &str) -> Result<String, ()> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' => {
                // `from_str_radix` alone would also accept a sign (`%+f`).
                let hex = bytes
                    .get(i + 1..i + 3)
                    .filter(|h| h.iter().all(u8::is_ascii_hexdigit))
                    .and_then(|h| std::str::from_utf8(h).ok());
                match hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                    Some(b) => {
                        out.push(b);
                        i += 3;
                    }
                    None => {
                        out.push(b'%');
                        i += 1;
                    }
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).map_err(|_| ())
}

fn parse_id(query: Option<&str>) -> Result<EntityId, &'static str> {
    let raw = query
        .unwrap_or("")
        .split('&')
        .find_map(|kv| kv.strip_prefix("id="))
        .ok_or("missing query parameter `id`")?;
    raw.parse::<usize>().map(EntityId).map_err(|_| "query parameter `id` must be a non-negative integer")
}

fn ok(content_type: &'static str, body: Vec<u8>) -> Response {
    Response { status: 200, content_type, body }
}

fn ui_module(path: &str) -> Response {
    let name = path.trim_start_matches("/ui/");
    match UI_MODULES.iter().find(|(n, _)| *n == name) {
        Some((_, src)) => ok(JS, src.as_bytes().to_vec()),
        None => error(404, &format!("no route for GET {path}")),
    }
}

fn error(status: u16, message: &str) -> Response {
    let body = serde_json::to_vec(&ErrorDto { error: message.to_string() }).unwrap();
    Response { status, content_type: JSON, body }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use entity_graph::EntityKind::{self, *};
    use entity_graph::ReferenceKind::*;
    use entity_graph::Site;
    use entity_graph::test_support::graph_from_parents;
    use serde_json::Value;

    use super::*;

    /// Mirrors `ui/fixture.json` node for node. `graph_from_parents` only
    /// knows names and kinds, so paths, line ranges and sites are overlaid
    /// after.
    fn fixture_graph() -> EntityGraph {
        let rows: &[(&str, EntityKind, Option<usize>, &str, usize, usize)] = &[
            ("demo", Folder, None, "demo", 0, 0),
            ("src", Folder, Some(0), "demo/src", 0, 0),
            ("main.rs", File, Some(1), "demo/src/main.rs", 0, 30),
            ("main", Function, Some(2), "demo/src/main.rs/main", 2, 12),
            ("helper", Function, Some(2), "demo/src/main.rs/helper", 14, 20),
            ("lib.rs", File, Some(1), "demo/src/lib.rs", 0, 80),
            ("Point", Class, Some(5), "demo/src/lib.rs/Point", 1, 6),
            ("new", Function, Some(6), "demo/src/lib.rs/Point/new", 8, 12),
            ("distance", Function, Some(6), "demo/src/lib.rs/Point/distance", 13, 20),
            ("geometry", Module, Some(5), "demo/src/lib.rs/geometry", 22, 60),
            ("area", Function, Some(9), "demo/src/lib.rs/geometry/area", 24, 40),
            ("tests", Folder, Some(0), "demo/tests", 0, 0),
            ("smoke.rs", File, Some(11), "demo/tests/smoke.rs", 0, 25),
            ("test_area", Function, Some(12), "demo/tests/smoke.rs/test_area", 3, 15),
        ];
        let parents: Vec<_> = rows.iter().map(|r| (r.0, r.1, r.2)).collect();
        let references = [
            (2, 6, Import),
            (3, 7, Call),
            (3, 4, Call),
            (4, 8, Call),
            (10, 8, Call),
            (10, 6, TypeRef),
            (12, 9, Import),
            (13, 10, Call),
            (8, 6, TypeRef),
        ];
        let mut graph = graph_from_parents(&parents, &references);
        for (entity, row) in graph.entities.iter_mut().zip(rows) {
            entity.path = PathBuf::from(row.3);
            entity.line_range = row.4..row.5;
        }
        graph.references[1].sites = vec![Site { line: 4 }, Site { line: 9 }];
        graph.references[6].sites = vec![Site { line: 0 }];
        graph
    }

    const INDEX: &str = "<!doctype html><title>t</title>";

    fn server() -> Server {
        Server::new(fixture_graph(), PathBuf::from("/nonexistent/demo"), INDEX)
    }

    fn json(resp: &Response) -> Value {
        assert_eq!(resp.content_type, JSON);
        serde_json::from_slice(&resp.body).unwrap()
    }

    #[test]
    fn graph_dto_matches_frontend_fixture() {
        let actual = serde_json::to_value(GraphDto::from(&fixture_graph())).unwrap();
        let expected: Value = serde_json::from_str(include_str!("../ui/fixture.json")).unwrap();
        assert_eq!(actual, expected);
    }

    /// Walks the full zoom contract against the fixture: root view, a
    /// successful zoom-in whose payload must equal `ui/fixture.coalesced.json`,
    /// then every rejection class (409 no-op, 409 unknown id, 400 bad query,
    /// 404 route, 405 method).
    #[test]
    fn zoom_sequence_follows_contract() {
        let s = server();

        let root = s.respond("GET", "/coalesced.json");
        assert_eq!(root.status, 200);
        assert_eq!(json(&root), serde_json::json!({ "leaves": [0], "edges": [] }));

        let zoomed = s.respond("POST", "/coalesced/zoom-in?id=0");
        assert_eq!(zoomed.status, 200);
        let expected: Value =
            serde_json::from_str(include_str!("../ui/fixture.coalesced.json")).unwrap();
        assert_eq!(json(&zoomed), expected);

        let again = s.respond("POST", "/coalesced/zoom-in?id=0");
        assert_eq!(again.status, 409);
        assert!(json(&again)["error"].as_str().unwrap().contains("not a current leaf"));

        let out = s.respond("POST", "/coalesced/zoom-out?id=1");
        assert_eq!(out.status, 200);
        assert_eq!(json(&out), serde_json::json!({ "leaves": [0], "edges": [] }));

        let root_out = s.respond("POST", "/coalesced/zoom-out?id=0");
        assert_eq!(root_out.status, 409);
        assert!(json(&root_out)["error"].as_str().unwrap().contains("root"));

        let unknown = s.respond("POST", "/coalesced/zoom-in?id=999");
        assert_eq!(unknown.status, 409);
        assert!(json(&unknown)["error"].as_str().unwrap().contains("unknown"));

        assert_eq!(s.respond("POST", "/coalesced/zoom-in").status, 400);
        assert_eq!(s.respond("POST", "/coalesced/zoom-in?id=abc").status, 400);
        assert_eq!(s.respond("GET", "/nope").status, 404);
        assert_eq!(s.respond("GET", "/coalesced/zoom-in?id=0").status, 405);
        assert_eq!(s.respond("POST", "/graph.json").status, 405);

        s.respond("POST", "/coalesced/zoom-in?id=0");
        s.respond("POST", "/coalesced/zoom-in?id=1");
        let reset = s.respond("POST", "/coalesced/reset");
        assert_eq!(reset.status, 200);
        assert_eq!(json(&reset), serde_json::json!({ "leaves": [0], "edges": [] }));
    }

    /// `/source` resolves any entity to its File ancestor's path under the
    /// project root and serves the text; folders, unknown ids, missing files
    /// and non-UTF-8 files each map to their own status.
    #[test]
    fn source_serves_file_of_any_entity_and_rejects_the_rest() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("demo");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/main.rs"), "fn main() {}\nfn helper() {}\n").unwrap();
        std::fs::write(root.join("src/lib.rs"), [0xffu8, 0xfe, b'x']).unwrap();
        let s = Server::new(fixture_graph(), root, INDEX);

        // A function id resolves to its file (id 2) and its path.
        let by_fn = s.respond("GET", "/source?id=4");
        assert_eq!(by_fn.status, 200);
        assert_eq!(
            json(&by_fn),
            serde_json::json!({ "id": 2, "path": "src/main.rs", "text": "fn main() {}\nfn helper() {}\n" })
        );
        assert_eq!(json(&s.respond("GET", "/source?id=2")), json(&by_fn));

        let folder = s.respond("GET", "/source?id=1");
        assert_eq!(folder.status, 404);
        assert_eq!(json(&folder)["error"], "entity 1 has no source file");

        assert_eq!(s.respond("GET", "/source?id=999").status, 404);
        // smoke.rs exists in the graph but not on disk.
        let stale = s.respond("GET", "/source?id=13");
        assert_eq!(stale.status, 404);
        assert!(json(&stale)["error"].as_str().unwrap().contains("smoke.rs"));

        assert_eq!(s.respond("GET", "/source?id=6").status, 415);
        assert_eq!(s.respond("GET", "/source").status, 400);
        assert_eq!(s.respond("GET", "/source?id=x").status, 400);
        assert_eq!(s.respond("POST", "/source?id=2").status, 405);
    }

    /// Single-file load: the root is the File and its `file_path` is empty,
    /// so the loaded path itself is served.
    #[test]
    fn source_of_single_file_root_is_the_loaded_path() {
        let dir = tempfile::TempDir::new().unwrap();
        let file = dir.path().join("only.rs");
        std::fs::write(&file, "fn f() {}\n").unwrap();
        let graph = graph_from_parents(&[("only.rs", File, None), ("f", Function, Some(0))], &[]);
        let s = Server::new(graph, file, INDEX);

        let resp = s.respond("GET", "/source?id=1");
        assert_eq!(resp.status, 200);
        assert_eq!(json(&resp), serde_json::json!({ "id": 0, "path": "", "text": "fn f() {}\n" }));
    }

    /// `/search` end to end over a temp tree matching `fixture_graph()`'s
    /// file paths (`tests/smoke.rs` is deliberately left off disk, so its
    /// entity contributes no documents): unrestricted search, the `kind:`
    /// prefix syntax, percent-decoding, case folding, a needle under 3 chars,
    /// the content limit and its `more` count, and the file/path dedupe rule
    /// in both directions.
    #[test]
    fn search_finds_files_paths_and_content() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("demo");

        // 106 lines, 103 of which contain "main" — enough to trip the
        // content limit (100) and prove `more` counts the rest.
        let mut main_rs = String::from("fn main() {\n    helper();\n}\nfn helper() {}\n");
        for i in 0..102 {
            main_rs.push_str(&format!("// main line {i}\n"));
        }
        write_tree(&root, &[
            ("src/main.rs", &main_rs),
            ("src/lib.rs", "// calls main eventually\nfn lib_fn() {}\n"),
        ]);
        let s = Server::new(fixture_graph(), root, INDEX);

        // Unrestricted "main": the file hit for main.rs leads, with offsets
        // matching the contract example exactly; its path hit is suppressed
        // (the match sits in the filename tail, the same occurrence the file
        // hit already shows); content hits are ordered by path then line and
        // truncated to the limit.
        let main = json(&s.respond("GET", "/search?q=main"));
        assert_eq!(main["query"], "main");
        let hits = main["hits"].as_array().unwrap();
        assert_eq!(
            hits[0],
            serde_json::json!(
                { "kind": "file", "id": 2, "path": "src/main.rs", "text": "main.rs", "start": 0, "end": 4 }
            )
        );
        assert!(hits.iter().all(|h| h["kind"] != "path"), "main.rs's path hit must be suppressed: {hits:?}");
        let content: Vec<_> = hits.iter().filter(|h| h["kind"] == "content").collect();
        assert_eq!(content.len(), 100);
        assert_eq!((content[0]["path"].as_str(), content[0]["line"].as_i64()), (Some("src/lib.rs"), Some(0)));
        assert_eq!((content[1]["path"].as_str(), content[1]["line"].as_i64()), (Some("src/main.rs"), Some(0)));
        assert_eq!(main["more"], serde_json::json!({ "file": 0, "path": 0, "content": 4 }));

        // A match confined to a directory segment ("src") is a different
        // occurrence from any filename tail and is kept, unlike above.
        let src = json(&s.respond("GET", "/search?q=src"));
        let src_hits = src["hits"].as_array().unwrap();
        assert!(src_hits.iter().all(|h| h["kind"] == "path"), "{src_hits:?}");
        assert_eq!(src_hits.len(), 2);

        // Restricted by prefix: only path hits, shortest path first.
        let path_only = json(&s.respond("GET", "/search?q=path:src"));
        assert_eq!(path_only["query"], "path:src");
        let hits = path_only["hits"].as_array().unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0]["path"], "src/lib.rs");
        assert_eq!(hits[1]["path"], "src/main.rs");

        // `content:` prefix + case folding: "FN" finds a lowercase "fn" line.
        let fn_hits = json(&s.respond("GET", "/search?q=content:FN"));
        let lib_fn =
            fn_hits["hits"].as_array().unwrap().iter().find(|h| h["text"] == "fn lib_fn() {}").unwrap();
        assert_eq!((lib_fn["start"].as_i64(), lib_fn["end"].as_i64()), (Some(0), Some(2)));

        // A 2-char needle bypasses the postings (linear scan); "rs" matches
        // both file names, but its only path occurrence in each is the same
        // filename tail, so both path hits are suppressed too.
        let rs = json(&s.respond("GET", "/search?q=rs"));
        let rs_hits = rs["hits"].as_array().unwrap();
        let file_paths: Vec<_> =
            rs_hits.iter().filter(|h| h["kind"] == "file").map(|h| h["path"].as_str().unwrap()).collect();
        assert_eq!(file_paths, vec!["src/lib.rs", "src/main.rs"]);
        assert!(rs_hits.iter().all(|h| h["kind"] != "path"), "{rs_hits:?}");

        // Percent-decoding: "+" is a space, which splits terms; both must be
        // on the line, in any order, unless quoted into one phrase.
        let decoded = json(&s.respond("GET", "/search?q=fn+main"));
        assert_eq!(decoded["query"], "fn main");
        assert!(decoded["hits"].as_array().unwrap().iter().any(|h| h["text"] == "fn main() {"));
        assert!(json(&s.respond("GET", "/search?q=main+fn"))["hits"].as_array().unwrap().iter().any(|h| h["text"] == "fn main() {"));
        assert!(json(&s.respond("GET", "/search?q=%22main+fn%22"))["hits"].as_array().unwrap().is_empty());

        // Mixed terms: a tag on another kind filters by file. "main" lines in
        // files named "lib": one content hit and nothing else, since lib.rs's
        // own name and path do not contain "main".
        let mixed = json(&s.respond("GET", "/search?q=main+file:lib"));
        let mixed_hits = mixed["hits"].as_array().unwrap();
        assert_eq!(mixed_hits.len(), 1, "{mixed_hits:?}");
        assert_eq!((mixed_hits[0]["kind"].as_str(), mixed_hits[0]["path"].as_str(), mixed_hits[0]["line"].as_i64()), (Some("content"), Some("src/lib.rs"), Some(0)));
        // The other way round: files named "lib" that contain "main" is a
        // file hit (marked on the name) plus the same line hit.
        let both = json(&s.respond("GET", "/search?q=file:lib+content:main"));
        let kinds: Vec<_> = both["hits"].as_array().unwrap().iter().map(|h| h["kind"].as_str().unwrap()).collect();
        assert_eq!(kinds, vec!["file", "content"]);

        assert_eq!(s.respond("GET", "/search").status, 400);
        assert_eq!(s.respond("POST", "/search?q=main").status, 405);
        assert_eq!(json(&s.respond("GET", "/search?q=")), serde_json::json!({
            "query": "", "hits": [], "more": { "file": 0, "path": 0, "content": 0 }
        }));
    }

    /// Diff mode end to end over two hand-built trees (no git). The union
    /// graph carries the base labels and per-node status and churn, coalesced
    /// edges carry their member references plus an aggregated status (`mixed`
    /// where a bundle holds both an unchanged and an added reference), and
    /// `/source` serves both sides of a changed file with the ops that
    /// interleave them — reading a removed file out of the base tree.
    #[test]
    fn diff_mode_payloads_carry_change_tags() {
        const LIB: &str =
            "/// Doc.\nfn alpha() {\n    let x = 1;\n}\n\nfn beta() {\n    alpha();\n}\n";
        const LIB_EDITED: &str = "/// Doc.\nfn alpha() {\n    let x = 1;\n    let y = 2;\n}\n\n\
                                  fn beta() {\n    alpha();\n}\n";
        const UTIL: &str = "fn one() {\n    alpha();\n}\n\nfn two() {\n    1\n}\n";
        const UTIL_EDITED: &str = "fn one() {\n    alpha();\n}\n\nfn two() {\n    beta();\n}\n";

        let trees = tempfile::TempDir::new().unwrap();
        let old_root = trees.path().join("base/proj");
        let new_root = trees.path().join("work/proj");
        write_tree(&old_root, &[
            ("src/lib.rs", LIB),
            ("src/util.rs", UTIL),
            ("src/gone.rs", "fn gone() {\n    alpha();\n}\n"),
        ]);
        write_tree(&new_root, &[("src/lib.rs", LIB_EDITED), ("src/util.rs", UTIL_EDITED)]);
        let old = treesitter_producer::graph_from_path(&old_root).unwrap();
        let new = treesitter_producer::graph_from_path(&new_root).unwrap();
        let diff = graph_diff::diff(&old, &old_root, &new, &new_root).unwrap();
        let s = Server::with_diff(
            diff,
            "HEAD~1".into(),
            "0123abcd".into(),
            None,
            new_root,
            INDEX,
        );

        let graph = json(&s.respond("GET", "/graph.json"));
        assert_eq!(
            graph["diff"],
            serde_json::json!({ "base": "HEAD~1", "base_commit": "0123abcd" })
        );
        let node = |path: &str| {
            graph["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .find(|n| n["path"] == path)
                .unwrap_or_else(|| panic!("no node at {path}"))
                .clone()
        };
        let alpha = node("proj/src/lib.rs/alpha");
        assert_eq!(alpha["status"], "modified");
        assert_eq!(alpha["added"], 1);
        assert_eq!(alpha["removed"], 0);
        assert_eq!(node("proj/src/lib.rs/beta")["status"], "same");
        assert_eq!(node("proj/src/gone.rs")["status"], "removed");
        assert_eq!(node("proj/src/gone.rs")["removed"], 3);
        assert_eq!(node("proj/src")["status"], "modified");

        // Zoom to the file level: every edge names its member references, and
        // the util.rs -> lib.rs bundle holds an unchanged and an added call.
        s.respond("POST", "/coalesced/zoom-in?id=0");
        let src_id = node("proj/src")["id"].clone();
        let view = json(&s.respond("POST", &format!("/coalesced/zoom-in?id={src_id}")));
        let edges = view["edges"].as_array().unwrap();
        assert!(edges.iter().all(|e| !e["refs"].as_array().unwrap().is_empty()), "{edges:?}");
        let edge = |from: &str, to: &str| {
            edges
                .iter()
                .find(|e| e["from"] == node(from)["id"] && e["to"] == node(to)["id"])
                .unwrap_or_else(|| panic!("no edge {from} -> {to} in {edges:?}"))
        };
        assert_eq!(edge("proj/src/util.rs", "proj/src/lib.rs")["status"], "mixed");
        assert_eq!(edge("proj/src/gone.rs", "proj/src/lib.rs")["status"], "removed");

        let lib_id = node("proj/src/lib.rs")["id"].as_u64().unwrap();
        let source = json(&s.respond("GET", &format!("/source?id={lib_id}")));
        assert_eq!(source["text"], LIB_EDITED);
        assert_eq!(source["old_text"], LIB);
        assert_eq!(
            source["ops"],
            serde_json::json!([["=", 0, 3, 0, 3], ["+", 3, 0, 3, 1], ["=", 3, 5, 4, 5]])
        );

        // The removed file only exists in the base tree.
        let gone_id = node("proj/src/gone.rs")["id"].as_u64().unwrap();
        let removed = json(&s.respond("GET", &format!("/source?id={gone_id}")));
        assert_eq!(removed["text"], "");
        assert_eq!(removed["old_text"], "fn gone() {\n    alpha();\n}\n");
        assert_eq!(removed["ops"], serde_json::json!([["-", 0, 3, 0, 0]]));

        // An unchanged file keeps the plain shape, so the UI reads absent ops
        // as "nothing to interleave".
        let util_id = node("proj/src/util.rs")["id"].clone();
        let util = json(&s.respond("GET", &format!("/source?id={util_id}")));
        assert!(util.get("ops").is_some(), "util.rs did change");
        assert_eq!(util["old_text"], UTIL);
    }

    /// `/search` indexes the same diff-aware text `/source` does: a removed
    /// file only from its old side, a changed file from its new side.
    #[test]
    fn search_indexes_diff_aware_text() {
        const LIB: &str = "fn alpha() {\n    let x = 1;\n}\n";
        const LIB_EDITED: &str = "fn alpha() {\n    let x = 1;\n    let y = 2;\n}\n";
        const GONE: &str = "fn gone() {\n    alpha();\n}\n";

        let trees = tempfile::TempDir::new().unwrap();
        let old_root = trees.path().join("base/proj");
        let new_root = trees.path().join("work/proj");
        write_tree(&old_root, &[("src/lib.rs", LIB), ("src/gone.rs", GONE)]);
        write_tree(&new_root, &[("src/lib.rs", LIB_EDITED)]);
        let old = treesitter_producer::graph_from_path(&old_root).unwrap();
        let new = treesitter_producer::graph_from_path(&new_root).unwrap();
        let diff = graph_diff::diff(&old, &old_root, &new, &new_root).unwrap();
        let s = Server::with_diff(diff, "HEAD~1".into(), "0123abcd".into(), None, new_root, INDEX);

        let gone = json(&s.respond("GET", "/search?q=content:gone"));
        let texts: Vec<_> =
            gone["hits"].as_array().unwrap().iter().map(|h| h["text"].as_str().unwrap()).collect();
        assert!(texts.iter().any(|t| t.contains("fn gone()")), "{texts:?}");

        // "let y = 2;" only exists on the new side.
        let edited = json(&s.respond("GET", "/search?q=content:y+%3D+2"));
        let texts: Vec<_> =
            edited["hits"].as_array().unwrap().iter().map(|h| h["text"].as_str().unwrap()).collect();
        assert!(texts.iter().any(|t| t.contains("let y = 2;")), "{texts:?}");
    }

    fn write_tree(root: &std::path::Path, files: &[(&str, &str)]) {
        for (rel, body) in files {
            let path = root.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, body).unwrap();
        }
    }

    #[test]
    fn static_routes_have_content_types_and_bodies() {
        let s = server();

        let index = s.respond("GET", "/");
        assert_eq!(index.status, 200);
        assert_eq!(index.content_type, HTML);
        assert!(!index.body.is_empty());

        let graph = s.respond("GET", "/graph.json");
        assert_eq!(graph.status, 200);
        assert_eq!(graph.content_type, JSON);
        assert_eq!(json(&graph)["root"], "demo");

        let module = s.respond("GET", "/ui/app.js");
        assert_eq!(module.status, 200);
        assert_eq!(module.content_type, JS);
        assert!(!module.body.is_empty());

        let missing = s.respond("GET", "/ui/nope.js");
        assert_eq!(missing.status, 404);

        let wrong_method = s.respond("POST", "/ui/app.js");
        assert_eq!(wrong_method.status, 405);
    }
}
