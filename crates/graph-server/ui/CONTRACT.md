# graph-server HTTP contract

All JSON. Ids are `EntityId` arena indices from the loaded graph; the graph is
immutable for the server's lifetime so ids are stable across every call.

**Diff mode** (`graph-server --diff <git-ref> PATH`) serves the *union* of the
tree at `<ref>` and the working tree as one ordinary graph, so every route
below keeps its shape. What it adds is change tags, and every one of them is
omitted outside diff mode: the presence of `diff` in `/graph.json` is how the
UI detects the mode, and an absent `status` always means "unchanged". A
rename is a removed entity plus an added one, never a modified one.

## GET /
The UI shell (`ui/index.html`), embedded in the binary. It loads its logic as
ES modules from `/ui/*.js` (`common.js`, `nodes.js`, `model.js`, `layout.js`,
`code.js`, `panel.js`, `search.js`, `app.js`), also embedded in the binary.

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
- `is_test` (omitted when false) marks test code by the conventions in
  `entity_graph::test_code`; every descendant of a test node is a test node.
  `test_loc` (omitted when 0) is the part of `loc` inside test nodes, so a
  view hiding tests shows `loc - test_loc`.
- `sites` are the 0-indexed lines, in the `from` entity's file, of every
  occurrence that produced the reference; sorted, no duplicates. Producers
  always record at least one; hand-built graphs may leave it empty.
- Containment is `parent` on the node; there is no separate edge list.
- `nodes` is sorted by `id` and dense (`nodes[i].id == i`).

In diff mode only, three additions:
```json
{
  "diff": { "base": "HEAD~1", "base_commit": "9f1c2ab..." },
  "nodes": [ { "id": 3, "status": "modified", "added": 12, "removed": 3, "...": "" } ],
  "references": [ { "from": 3, "to": 7, "kind": "call", "sites": [12], "status": "added" } ]
}
```
- `diff.base` is the ref as the user typed it, `diff.base_commit` the sha it
  resolved to. The working tree is always the new side.
- node `status` ∈ `same | added | removed | modified`. A File is `same` iff
  its bytes are; a Function/Class/Module is `modified` iff lines changed
  inside its range (widened upward over its doc comments, attributes and
  decorators), so a pure line shift stays `same`; a Folder is `same` iff all
  its children are.
- node `added`/`removed` are churn: lines inserted and deleted inside the
  node's range. A File counts its whole diff, a Folder sums its children.
- reference `status` ∈ `same | added | removed`, never `modified`. `sites`
  come from the side that has the reference (new when both do), so the sites
  of a `removed` reference are lines in the **old** file.
- A removed entity keeps its old `path` and line range and hangs under the
  parent it had; the union still has one root and dense ids.

## GET /source?id=N  — the source file containing entity N
```json
{ "id": 1, "path": "src/main.rs", "text": "fn main() {\n..." }
```
`N` may be any entity; the server walks up to its File ancestor, whose id is
returned as `id`. `path` is relative to the loaded project root with `/`
separators; it is `""` when the loaded root is itself the file. The whole
file is returned, never truncated.

In diff mode, a file whose content changed (or that exists on one side only)
carries both sides and the line ops that interleave them:
```json
{ "id": 1, "path": "src/main.rs", "text": "new side", "old_text": "old side",
  "ops": [["=", 0, 3, 0, 3], ["-", 3, 2, 3, 0], ["+", 5, 0, 3, 4]] }
```
- an op is `[tag, old_start, old_len, new_start, new_len]`, 0-indexed lines,
  tag `"="` equal, `"-"` deleted from old, `"+"` inserted in new. Ops cover
  both files in order; a delete and an insert each carry their position on the
  opposite side, which is what lets them be rendered as one listing.
- `text` is `""` for a removed file, `old_text` `""` for an added one. A
  removed file's old side is read from the extracted base tree.
- Any other file — unchanged, binary, or over 1 MiB on either side — returns
  the plain shape above with no `old_text` and no `ops`.

Errors, all `{"error": "..."}`: `400` bad or missing `id`; `404` unknown id,
an entity above every File (a folder: "entity N has no source file"), or an
unreadable file (message includes the io error — the index may be stale);
`403` if the file resolves outside the project root (in diff mode, outside
whichever root that side is read from); `413` if the file is over 2 MiB;
`415` if it is not valid UTF-8.

## GET /coalesced.json  — the current coalesced view
```json
{ "leaves": [0, 5, 9],
  "edges": [ { "from": 0, "to": 5, "kind": "call", "sites": [], "refs": [3, 7], "status": "mixed" } ] }
```
Leaves are entity ids; the UI joins them against `/graph.json` for labels.
Edges are deduplicated on `(from, to, kind)`; self-loops never appear.
Coalesced edges never carry sites (`sites` is always `[]`): an edge stands
for many raw references, listed in `refs` as indices into `/graph.json`'s
`references`, in graph order — that is where the UI reads their sites.
In diff mode an edge also carries `status` ∈ `same | added | removed |
mixed`, the status its members agree on, or `mixed` when they do not.

## GET /search?q=...  — substring search over files
```json
{ "query": "main file:lib",
  "hits": [
    { "kind": "file",    "id": 2, "path": "src/main.rs", "text": "main.rs",     "start": 0, "end": 4 },
    { "kind": "path",    "id": 9, "path": "src/domain/x.rs", "text": "src/domain/x.rs", "start": 4, "end": 8 },
    { "kind": "content", "id": 2, "path": "src/main.rs", "line": 0, "text": "fn main() {", "start": 3, "end": 7 }
  ],
  "more": { "file": 0, "path": 0, "content": 12 } }
```
Substring search, case-insensitive, over three kinds built from every File
entity in the graph at load: `file` (the entity's name), `path` (its wire
path, `/`-separated), `content` (its lines, one document per line). Only
files the server can read text for are indexed — same texts `/source` would
serve, so a file that fails to read (unreadable, non-UTF-8, too big, or, in
diff mode, absent from both sides) is simply not searchable; a removed file
is indexed from its old text, an edited file from its new text.

- `q` is percent-encoded (`+` or `%20` for space). Missing `q` is `400`
  `` missing query parameter `q` ``; `q` that percent-decodes to invalid UTF-8
  is `400`; a query with no terms (`q=`) is `200` with no hits. Any other
  method is `405`. `query` echoes the trimmed query as typed.
- **Terms.** The query is split on whitespace into terms that must *all*
  hold. Double quotes join a phrase into one term and are removed
  (`"fn respond"`). A term may carry a tag, `file:`, `path:` or `content:`
  (case-insensitive, written outside any quotes); anything else with a colon
  (`foo:bar`, `C:\x`) is a plain term, and `"file:x"` in quotes is too. A tag
  with nothing after it is ignored.
- **How a term holds.** For a hit of kind K, a bare term or a term tagged K
  must occur in the hit's own text (the name, the path, or the line). A term
  tagged with another kind is a filter on the hit's file: `file:` on its
  name, `path:` on its path, `content:` on any one of its lines. A kind only
  produces hits when at least one term is about its own text, so `file:x`
  alone lists files, not every line inside them. Hence `class file:resolver`
  is "lines containing `class` in files whose name contains `resolver`", and
  `file:resolver content:class` additionally lists those files themselves.
  A hit's `start`/`end` mark the first own-text term.
- A hit's `kind` is which of the three kinds matched — not an entity kind,
  even though `"file"` happens to also be one. `path` is always the file's
  wire path, regardless of which kind matched; `text` is the thing the term
  was found in (name / path / line) and `start`/`end` the marked span within
  it. `line` (0-indexed, omitted on file/path hits) is present only on
  content hits.
- `start`/`end` are **UTF-16 code units** into `text`, not chars or bytes,
  because the UI slices JS strings with them.
- **Ordering.** File hits sort by (name length, path), path hits by (path
  length, path), content hits by (path, line); output is all file hits, then
  all path hits, then all content hits. A path hit whose mark falls inside
  the trailing filename segment, for a file that is also a file hit, is
  dropped — it is the same occurrence — but a mark in a directory segment
  (`main/` in `main/src/main.rs` for `main`) is kept as its own hit.
- **Limits.** Each kind is capped (`file` 30, `path` 30, `content` 100); the
  count past the cap for a kind is `more.<kind>`, `0` when nothing was cut.

## POST /coalesced/zoom-in?id=N
Replace leaf `N` with its children. `200` + the new coalesced payload on
change; `409` + `{"error": "..."}` if `N` is not a leaf or has no children.

## POST /coalesced/zoom-out?id=N
Replace leaf `N` (and its sibling leaves) with its parent. `200` + payload on
change; `409` if `N` is not a leaf or is a root.

## POST /coalesced/reset
Back to the root leaves. `200` + payload.

Anything else: `404`.
