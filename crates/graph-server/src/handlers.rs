//! Request handling as a pure function of (method, path) -> response, so the
//! whole contract is testable without opening a socket. `main.rs` is the only
//! thing that knows about `tiny_http`.

use std::sync::Mutex;

use coalesce::Cursor;
use entity_graph::{EntityGraph, EntityId};

use crate::dto::{CoalescedDto, ErrorDto, GraphDto};

pub struct Response {
    pub status: u16,
    pub content_type: &'static str,
    pub body: Vec<u8>,
}

const JSON: &str = "application/json";
const HTML: &str = "text/html; charset=utf-8";

pub struct Server {
    graph: EntityGraph,
    // The graph never changes after load, so its JSON is rendered once; on a
    // real repo it is by far the largest payload.
    graph_json: String,
    cursor: Mutex<Cursor>,
    index_html: &'static str,
}

impl Server {
    pub fn new(graph: EntityGraph, index_html: &'static str) -> Self {
        let graph_json = serde_json::to_string(&GraphDto::from(&graph))
            .expect("GraphDto serialization is infallible");
        let cursor = Mutex::new(Cursor::new(&graph));
        Server { graph, graph_json, cursor, index_html }
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
            "/" | "/graph.json" | "/coalesced.json" if method != "GET" => {
                error(405, "method not allowed; use GET")
            }
            "/" => ok(HTML, self.index_html.as_bytes().to_vec()),
            "/graph.json" => ok(JSON, self.graph_json.clone().into_bytes()),
            "/coalesced.json" => self.coalesced_response(),

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

    fn coalesced_response(&self) -> Response {
        let coalesced = self.cursor.lock().unwrap().coalesced();
        ok(JSON, serde_json::to_vec(&CoalescedDto::from(&coalesced)).unwrap())
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

fn error(status: u16, message: &str) -> Response {
    let body = serde_json::to_vec(&ErrorDto { error: message.to_string() }).unwrap();
    Response { status, content_type: JSON, body }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use entity_graph::EntityKind::{self, *};
    use entity_graph::ReferenceKind::*;
    use entity_graph::test_support::graph_from_parents;
    use serde_json::Value;

    use super::*;

    /// Mirrors `ui/fixture.json` node for node. `graph_from_parents` only
    /// knows names and kinds, so paths and line ranges are overlaid after.
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
        graph
    }

    fn server() -> Server {
        Server::new(fixture_graph(), "<!doctype html><title>t</title>")
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
    }
}
