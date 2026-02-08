# Oxide

**xd** — A TUI file manager in the style of Midnight Commander. Built with Rust, Ratatui, and Crossterm.

## Overview

Oxide is a terminal user interface (TUI) file manager with a dual-panel layout. The compiled binary is named **xd**.

### Features

- **Two-panel layout** — Single- or double-column view modes
- **Full keyboard navigation** — Arrow keys, Tab, Enter, F-keys
- **File operations** — Copy, move, delete with overwrite and error handling
- **Viewer (F3)** — Text and hex modes, scroll
- **Embedded editor (F4)** — Syntax highlighting, Ctrl+F search, save/discard
- **Create directory (F7)**
- **Rename / Attributes (F2)** — Change name, permissions, owner/group (Unix)
- **Size info (F9)** — Total size of selected files and folders (background calculation)
- **Shell relay (Ctrl+O)** — Spawn subshell, run commands, return to panels
- **Command line** — Run shell commands
- **Mouse support** — Clicks, scroll
- **Disk space display** — Shows usage on Unix

## Build

### Prerequisites

- [Rust](https://www.rust-lang.org/tools/install) (edition 2021)
- On Unix: `libc`, `nix` (for permissions, subshell PTY)

### Build commands

```bash
# Build debug binary
cargo build

# Build release binary (optimized)
cargo build --release

# Build and run
cargo run
cargo run --release
```

### Output

- Debug: `target/debug/xd`
- Release: `target/release/xd`

### Install (optional)

```bash
cargo install --path .
# Installs xd to ~/.cargo/bin
```

## Usage

```bash
xd
```

### Key bindings

| Category | Keys | Action |
|----------|------|--------|
| **Panels** | ↑↓ PgUp/PgDn | Navigate |
| | Tab | Switch panel |
| | ← → | Move column |
| | Enter | Open dir / run |
| | Space | Mark |
| | * | Invert selection |
| **F-keys** | F1 | Settings / Help |
| | F2 | Rename / Attributes |
| | F3 | View |
| | F4 | Edit |
| | F5 | Copy |
| | F6 | Move |
| | F7 | New directory |
| | F8 | Delete |
| | F9 | Size info |
| | F10 | Quit |
| **Shortcuts** | Ctrl+O | Shell |
| | Ctrl+R | Refresh |
| | Ctrl+T | View mode |
| | Type char | Command line |
| | Tab/Esc | Panel focus |
| **Editor** | Shift+←→↑↓ | Select |
| | F3 | Line numbers |
| | Ctrl+C/V | Copy/Paste |
| | F2 | Save |
| | Ctrl+F | Find |
| | Esc | Exit |
| **Viewer** | H | Hex/text toggle |
| | ↑↓ PgUp/PgDn | Scroll |
| | Esc | Close |

## Dependencies

- `chrono` — Date/time
- `ratatui` — TUI framework
- `crossterm` — Terminal I/O
- `dirs` — Home directory
- `ratatui-code-editor` — Embedded editor
- `libc`, `nix` — Unix-only (permissions, subshell PTY)

## License

[LICENSE](LICENSE) — MIT License.
