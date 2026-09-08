# graph-server HTTP contract

All JSON. Ids are `EntityId` arena indices from the loaded graph; the graph is
immutable for the server's lifetime so ids are stable across every call.

## GET /
The single-file UI (`ui/index.html`), embedded in the binary.

## GET /graph.json  — the raw entity graph
```json
{
  "root": "Terraform",
  "nodes": [
    { "id": 0, "kind": "folder", "name": "Terraform", "path": "Terraform",
      "line_start": 0, "line_end": 0, "parent": null, "loc": 41 },
    { "id": 1, "kind": "file", "name": "main.rs", "path": "Terraform/main.rs",
      "line_start": 0, "line_end": 40, "parent": 0, "loc": 41 }
  ],
  "references": [
    { "from": 3, "to": 7, "kind": "call", "sites": [12, 30] }
  ]
}
```
- `kind` ∈ `folder | module | file | class | function`
- reference `kind` ∈ `call | import | type_ref | var_ref | generic`
- `line_start`/`line_end` are 0-indexed and inclusive, `0/0` when not
  applicable.
- `loc` is `line_end - line_start + 1` for any node with a range; a node
  without one (folders) carries the sum of its children's `loc`.
- `sites` are the 0-indexed lines, in the `from` entity's file, of every
  occurrence that produced the reference; sorted, no duplicates. Producers
  always record at least one; hand-built graphs may leave it empty.
- Containment is `parent` on the node; there is no separate edge list.
- `nodes` is sorted by `id` and dense (`nodes[i].id == i`).

## GET /source?id=N  — the source file containing entity N
```json
{ "id": 1, "path": "src/main.rs", "text": "fn main() {\n..." }
```
`N` may be any entity; the server walks up to its File ancestor, whose id is
returned as `id`. `path` is relative to the loaded project root with `/`
separators; it is `""` when the loaded root is itself the file. The whole
file is returned, never truncated.

Errors, all `{"error": "..."}`: `400` bad or missing `id`; `404` unknown id,
an entity above every File (a folder: "entity N has no source file"), or an
unreadable file (message includes the io error — the index may be stale);
`403` if the file resolves outside the project root; `413` if the file is
over 2 MiB; `415` if it is not valid UTF-8.

## GET /coalesced.json  — the current coalesced view
```json
{ "leaves": [0, 5, 9], "edges": [ { "from": 0, "to": 5, "kind": "call", "sites": [] } ] }
```
Leaves are entity ids; the UI joins them against `/graph.json` for labels.
Edges are deduplicated on `(from, to, kind)`; self-loops never appear.
Coalesced edges never carry sites (`sites` is always `[]`): an edge stands
for many raw references, so the UI derives the underlying references and
their sites from `/graph.json`.

## POST /coalesced/zoom-in?id=N
Replace leaf `N` with its children. `200` + the new coalesced payload on
change; `409` + `{"error": "..."}` if `N` is not a leaf or has no children.

## POST /coalesced/zoom-out?id=N
Replace leaf `N` (and its sibling leaves) with its parent. `200` + payload on
change; `409` if `N` is not a leaf or is a root.

## POST /coalesced/reset
Back to the root leaves. `200` + payload.

Anything else: `404`.
