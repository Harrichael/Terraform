# Terraform

> **Form your codebase to a new molding. Rapidly understand and edit your code.**

Terraform is an open-source, terminal-based platform for reimagining how developers interact with and edit code. Built in Rust using [ratatui](https://github.com/ratatui-org/ratatui) and [crossterm](https://github.com/crossterm-rs/crossterm), it provides a highly extensible TUI environment where different tools and "apps" can plug in to transform the editing experience.

---

## Features

### Hierarchical Code Viewer (Flagship App)

The first — and flagship — app treats every line of source code as a node in a dynamic tree. Users can:

- **Switch granularity on demand** — Collapse/expand views to show modules, files, classes/structs, functions/methods, blocks, or individual lines.
- **Filter nodes instantly** — Type a pattern to narrow the view to matching symbols, names, or content.
- **Keyboard-driven navigation** — Fast, mouse-free movement through any codebase.
- **Multi-language support** — Rust, Python, and JavaScript powered by [Tree-sitter](https://tree-sitter.github.io/).

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
# Open a source file
terraform path/to/your/file.rs

# No arguments — shows an empty viewer
terraform
```

### Keyboard Shortcuts

| Key | Action |
|-----|--------|
| `↑` / `k` | Move cursor up |
| `↓` / `j` | Move cursor down |
| `PgUp` | Page up |
| `PgDn` | Page down |
| `g` / `Home` | Jump to top |
| `G` / `End` | Jump to bottom |
| `Space` / `Enter` | Toggle collapse/expand node |
| `[` | Collapse all nodes |
| `]` | Expand all nodes |
| `/` | Enter filter mode |
| `Esc` | Clear filter / cancel |
| `?` / `F1` | Toggle help overlay |
| `q` / `Ctrl+C` | Quit |

---

## Architecture

```
src/
├── main.rs          # Entry point, terminal setup, render loop
├── app/
│   ├── tree.rs      # CodeNode, CodeTree — hierarchical data model
│   └── state.rs     # AppState — UI state, cursor, filter, mode
├── parser/
│   └── mod.rs       # Tree-sitter integration, language detection
└── ui/
    ├── mod.rs        # Public UI surface
    ├── mod_impl.rs   # ratatui rendering (tree panel, status bar, help)
    └── events.rs     # Keyboard event handling
```

### Node Kinds

From coarsest to finest granularity:

| Kind | Description |
|------|-------------|
| `Module` | Rust `mod` blocks or Python packages |
| `File` | Root of a single source file |
| `Class` | `struct`, `enum`, `trait`, `impl`, `class` |
| `Function` | `fn`, method definitions |
| `Block` | `{ … }` blocks |
| `Line` | Individual source lines |

---

## Tech Stack

| Component | Library |
|-----------|---------|
| TUI framework | [ratatui](https://github.com/ratatui-org/ratatui) |
| Terminal backend | [crossterm](https://github.com/crossterm-rs/crossterm) |
| Parsing | [tree-sitter](https://tree-sitter.github.io/) |
| CLI arguments | [clap](https://github.com/clap-rs/clap) |

---

## Roadmap

Future apps and features planned for the Terraform platform:

- [ ] In-place code editing (structural edits, rename, extract)
- [ ] Parameter add/remove with automatic propagation through callers
- [ ] Git integration (blame, diff, stage)
- [ ] LSP integration for richer symbol information
- [ ] AI-assisted edits
- [ ] Live collaboration
- [ ] Custom community-built TUI apps

---

## Contributing

Star the repo, open issues for feature ideas, or submit PRs for parsers, new apps, or UX improvements!

**License:** MIT
