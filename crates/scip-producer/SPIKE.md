# SCIP producer spike

Indexers examined: rust-analyzer 1.98.0 (`rust-analyzer scip`), scip-typescript
0.4.0 (via `npx`), scip-go 0.2.7 (`go install github.com/scip-code/scip-go/...`;
the old `sourcegraph/scip-go` module path no longer installs). Go toolchain
lives at `/opt/homebrew/opt/go/bin/go` (the `go` shell function shadows it).

## What the indexes actually contain

| | rust-analyzer | scip-typescript | scip-go |
|---|---|---|---|
| `position_encoding` | UTF8 | Unspecified (is UTF-16) | Unspecified (is UTF-8) |
| `Import` role (0x2) | never | never | never |
| `enclosing_range` | on every definition (and, oddly, on references too) | classes/functions, not params | functions only |
| `typed_enclosing_range` | never | never | never |
| `SymbolInformation.kind` | yes (Struct, Enum, Method, StaticMethod, Module, Field, ...) | never (Unspecified) | yes (Package, Struct, Function, Method, Field) |
| `display_name` | yes, except empty for `crate/` | never | yes |
| `enclosing_symbol` | locals only | never | never |

Symbol shapes (rust-analyzer):

- struct: `rust-analyzer cargo fixture 0.1.0 geometry/Point#`
- inherent impl block: `... geometry/impl#[Point]` (kind TypeAlias, *no*
  definition occurrence; referenced by `Self`)
- inherent method: `... geometry/impl#[Point]new().`
- trait impl method: `... geometry/impl#[Point][Display]fmt().`
- free fn: `... main().`; module: `... util/inner/`; crate root: `... crate/`
- `mod foo;` in a parent file is a plain reference to `foo/` (roles=0), not a
  definition. The module's definition is the whole-file occurrence
  `[0,0,N,0]` in `foo.rs`.
- Fields `Point#x.`, enum members `EntityKind#Folder#`, locals `local 3`.

scip-typescript: `scip-typescript npm fixture-ts 0.1.0 src/\`geometry.ts\`/Point#magnitude().`,
constructor `Point#\`<constructor>\`().`, file module `src/\`geometry.ts\`/`
defined at `[0,0,0]` with a whole-file enclosing range.

scip-go: `scip-go gomod example.com/fixture <hash> \`example.com/fixture/geometry\`/Point#Magnitude().`;
the package symbol is defined at the `package geometry` token of *every* file
in the package, with no enclosing range.

## Where the planned rules had to bend

1. **Methods are not `Type#method().` in rust-analyzer.** They are
   `impl#[Self][Trait]method().`, so stripping the last descriptor yields the
   impl block, which is not an entity. Added a rust-analyzer-specific step
   between rules (a) and (b): rewrite `ns/impl#[Self]...` to `ns/Self#` and
   look that up (same-document rule still applies). Impls of foreign types
   fall through to the File, which is the honest answer.
2. **File-level modules alias to the File entity.** A Module definition with
   no enclosing range, or one starting at 0:0, is the file itself
   (rust-analyzer `crate/`/`foo/`, scip-typescript file modules, scip-go
   `package` lines). Making them separate Module entities would put a
   redundant single child under every File. Only inline `mod x { }` (an
   enclosing range starting later) becomes a Module. Known blind spot: an
   inline module whose block starts at the very first byte of a file.
3. **Import edges are textual.** No indexer sets the Import role, so an
   occurrence is an Import when its line is a `use`/`import`/`from`
   statement (after `pub`, `pub(...)`, `export`) or sits inside a Go
   `import ( ... )` block. Without source on disk no Import edges appear.
4. **Unspecified position encoding** is UTF-16 when
   `tool_info.name == "scip-typescript"`, else UTF-8.
5. **Only definition occurrences' `enclosing_range` is trusted.**
   rust-analyzer copies the *target's* enclosing range onto reference
   occurrences, which would otherwise make every reference look like a scope.
6. The same-document rule's motivating case (`mod foo;` stubs being
   definitions) does not occur with rust-analyzer, but the rule still earns
   its keep for scip-go, where a package is "defined" in every file.

Rules that held as written: kind from `SymbolInformation.kind` with suffix
fallback (`().`→Function, `#`→Class, `/`→Module, terms skipped), first
definition wins, `enclosing_range` else name range for spans, containment
fallback to innermost enclosing definition then File, references from the
innermost enclosing definition, dedup on `(from,to,kind)`, self-loops dropped.

## Smoke check on this repo (index of 22 documents, 11718 occurrences)

341 entities (17 folders, 22 files, 5 modules, 26 classes, 271 functions),
1211 references (550 Call, 524 TypeRef, 74 Import, 63 Generic). Output is
byte-identical across runs. Hand-checked edges, all correct against source:

- `src/graph/cursor.rs/Cursor/move_down -> crates/entity-graph/src/model.rs/EntityGraph/get [call]`
- `src/graph/navigator.rs/Navigator/entity -> .../EntityGraph/get [call]`
- `src/main.rs -> src/app/state.rs [import]` (`use app::state::AppState`)
- `src/main.rs -> src/app/mod.rs [generic]` (`mod app;`)
- `crates/entity-graph/src/model.rs/EntityKind/fmt` parented under
  `EntityKind` (trait impl `Display for EntityKind`).

Oddities worth knowing:

- `crate::` path qualifiers resolve to the crate-root File, so many entities
  carry a `-> src/main.rs [generic]` edge. Accurate, but noisy.
- Several entities had `byte_range 0..0` because the files were deleted or
  moved between indexing and the run (concurrent refactor); line ranges are
  kept as designed.
- rust-analyzer logged "definition ... should have been in an SCIP document"
  for some derive-generated items; those never appear in the index.
- The five Module entities are all inline `mod tests { }` blocks plus one
  inline `mod entity`. Glue `mod.rs` files are ordinary Files with only
  outgoing Generic edges to the files they declare.

## Model

No changes to `entity-graph` were needed.
