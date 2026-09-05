# Terraform

> **Form your codebase to a new molding. Rapidly understand and edit your code.**

Terraform is an open-source, terminal-based platform for reimagining how developers interact with and edit code. Built in Rust using [ratatui](https://github.com/ratatui-org/ratatui) and [crossterm](https://github.com/crossterm-rs/crossterm), it provides a highly extensible TUI environment where different tools and "apps" can plug in to transform the editing experience.

---

## Features

### Hierarchical Code Viewer (Flagship App)

The first — and flagship — app treats every line of source code as a node in a dynamic tree. Users can:

- **Open a directory or file directly** — Pass any path (or none to open `.`). Directory roots expand through Folder → File → code constructs.
- **Switch granularity per node** — Use `l`/`Right` and `h`/`Left` to expand or shrink the detail level of a *single* node without affecting its siblings. Granularity levels: Folder → Module → File → Class/Struct → Function → Block (if/for/while) → Line.
- **Filter nodes instantly** — Type a pattern to narrow the view to matching names or content.
- **Symbolic references (Lib section)** — Symbols defined in multiple files are deduplicated; the canonical definition is promoted to a `[Lib]` section at the bottom, and duplicates become `[ref]` nodes. Press `Enter` on a `[ref]` node to jump to the definition.
- **Multi-language support** — Rust, Python, and JavaScript powered by [Tree-sitter](https://tree-sitter.github.io/). Plain text files are shown line-by-line.
- **Keyboard-driven** — Fast, mouse-free navigation throughout.

---

## Installation

### Prerequisites

- Rust toolchain (1.70+): https://rustup.rs/

### Build from source

```bash
git clone https://github.com/Harrichael/Terraform
cd Terraform
cargo build --release
```

The binary will be at `target/release/terraform`.

---

## Usage

```bash
# Open the current directory (default)
terraform

# Open a specific directory
terraform path/to/project/

# Open a single source file
terraform path/to/file.rs
```

When opening a directory, the view starts at **File granularity** — only folders and files are shown. Use `l`/`Right` on a file to drill into its code constructs.

---

## Keyboard Shortcuts

| Key | Action |
|-----|--------|
| `↑` / `k` | Move cursor up |
| `↓` / `j` | Move cursor down |
| `PgUp` | Page up |
| `PgDn` | Page down |
| `g` / `Home` | Jump to top |
| `G` / `End` | Jump to bottom |
| **`l` / `→`** | **Expand cursor node to next finer granularity** |
| **`h` / `←`** | **Shrink cursor node to next coarser granularity** |
| `Space` | Toggle full collapse/expand of cursor node |
| `Enter` | Toggle collapse, or jump to SymRef definition |
| `[` | Collapse all nodes |
| `]` | Expand all nodes |
| `/` | Enter filter mode |
| `Esc` | Clear filter / cancel |
| `?` / `F1` | Toggle help overlay |
| `q` / `Ctrl+C` | Quit |

### Granularity Levels

From coarsest to finest:

```
Folder → Module → File → Class/Struct → Function/Method → Block (if/for/while) → Line
```

`l`/`Right` expands one step finer; `h`/`Left` shrinks one step coarser. Changes apply **only to the node under the cursor** — siblings are unaffected.

---

## Architecture

A Cargo workspace. Producers build an `EntityGraph`; consumers read it and
never learn which producer built it.

```
crates/
├── entity-graph/         # The contract: Entity, EntityGraph, Reference (+ test_support fixtures)
├── treesitter-producer/  # Source files → EntityGraph via tree-sitter; one fn: graph_from_path
├── coalesce/             # Cursor: which entities are in view at this zoom, and the edges between them
├── scip-producer/        # (in progress) SCIP index → EntityGraph
└── graph-server/         # (in progress) localhost viewer for an EntityGraph
src/                      # The TUI (bin `terraform`)
├── main.rs               # Entry point, terminal setup, render loop
├── app/state.rs          # AppState — loading, zoom/fold navigation state
├── graph/
│   ├── navigator.rs      # Projects the coalesced view into a renderable GraphTree
│   └── tree.rs           # GraphTree — spanning-forest layout of the reference graph
└── ui/                   # ratatui rendering and keyboard handling
```

### Node Kinds

| Kind | Description |
|------|-------------|
| `Folder` | Directory |
| `Module` | Rust `mod`, Python packages |
| `File` | Source file |
| `Class` | `struct`, `enum`, `trait`, `impl`, `class`, `interface`, `type alias`, SQL table/view |
| `Function` | `fn`, method, `def`, TypeScript method signature |
| `Block` | `if`/`for`/`while`/`match`/`switch` constructs, SQL statements |
| `Line` | Individual source lines |

---

## Tech Stack

| Component | Library |
|-----------|---------|
| TUI framework | [ratatui](https://github.com/ratatui-org/ratatui) |
| Terminal backend | [crossterm](https://github.com/crossterm-rs/crossterm) |
| Parsing | [tree-sitter](https://tree-sitter.github.io/) (Rust, Python, JavaScript, TypeScript, TSX, SQL) |
| CLI arguments | [clap](https://github.com/clap-rs/clap) |

---

## Roadmap

- [ ] In-place structural code editing (rename, extract, inline)
- [ ] Parameter add/remove with automatic propagation through callers
- [ ] Git integration (blame, diff, stage)
- [ ] LSP integration for richer cross-file symbol references
- [ ] AI-assisted edits
- [ ] Live collaboration
- [ ] Community-built TUI apps

---

## Contributing

Star the repo, open issues for feature ideas, or submit PRs for parsers, new apps, or UX improvements!

**License:** MIT
