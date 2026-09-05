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
      "line_start": 0, "line_end": 0, "parent": null },
    { "id": 1, "kind": "file", "name": "main.rs", "path": "Terraform/main.rs",
      "line_start": 0, "line_end": 40, "parent": 0 }
  ],
  "references": [
    { "from": 3, "to": 7, "kind": "call" }
  ]
}
```
- `kind` ∈ `folder | module | file | class | function`
- reference `kind` ∈ `call | import | type_ref | var_ref | generic`
- `line_start`/`line_end` are 0-indexed, `0/0` when not applicable.
- Containment is `parent` on the node; there is no separate edge list.
- `nodes` is sorted by `id` and dense (`nodes[i].id == i`).

## GET /coalesced.json  — the current coalesced view
```json
{ "leaves": [0, 5, 9], "edges": [ { "from": 0, "to": 5, "kind": "call" } ] }
```
Leaves are entity ids; the UI joins them against `/graph.json` for labels.
Edges are deduplicated on `(from, to, kind)`; self-loops never appear.

## POST /coalesced/zoom-in?id=N
Replace leaf `N` with its children. `200` + the new coalesced payload on
change; `409` + `{"error": "..."}` if `N` is not a leaf or has no children.

## POST /coalesced/zoom-out?id=N
Replace leaf `N` (and its sibling leaves) with its parent. `200` + payload on
change; `409` if `N` is not a leaf or is a root.

## POST /coalesced/reset
Back to the root leaves. `200` + payload.

Anything else: `404`.
