# Oxide

**xd** — A TUI file manager in the style of Midnight Commander. Built with Rust, Ratatui, and Crossterm.

## Overview

Oxide is a terminal user interface (TUI) file manager with a dual-panel layout. The compiled binary is named **xd**.

### Features

See [FEATURES.md](doc/FEATURES.md) for a full list of features and key bindings.

## Build

For Linux (Debian/Ubuntu), see [BUILD_LINUX.md](doc/BUILD_LINUX.md) for system dependencies and build flow.

### Prerequisites

- [Rust](https://www.rust-lang.org/tools/install) (edition 2021)
- On Unix: `libc`, `nix` (for permissions, subshell PTY)

For theory on the subshell, PTY, terminals, and file descriptors, see [SHELL_PTY_TERMINAL.md](doc/SHELL_PTY_TERMINAL.md).

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

### Portable tarball installer (Linux & macOS)

The repo includes a POSIX shell installer and a helper script that packages the release binary plus `install.sh` into a gzip-compressed tarball.

**Create a release archive** (run on the OS and CPU architecture you want to ship; the archive name includes `uname` output, e.g. `linux-x86_64` or `darwin-arm64`):

```bash
./scripts/make-dist.sh
```

This runs `cargo build --release` and writes `dist/xd-<version>-<os>-<arch>.tar.gz` (the `dist/` directory is gitignored).

**Install from the tarball** on the target machine:

```bash
tar xzf xd-0.1.0-linux-x86_64.tar.gz   # use the file name you built or downloaded
./install.sh
```

By default, `install.sh` copies `xd` into `$HOME/.local/bin` when that directory is writable; otherwise it uses `/usr/local/bin` and may invoke `sudo`. Override the destination:

```bash
./install.sh -p /path/to/bin
# or
PREFIX=/path/to/bin ./install.sh
```

**Install a local release build without packaging:**

```bash
cargo build --release
./install/install.sh -p "$HOME/.local/bin" target/release/xd
```

To create a Debian (.deb) package, see [PACKAGING_DEB.md](doc/PACKAGING_DEB.md).

## Usage

```bash
xd
```

See [FEATURES.md](doc/FEATURES.md) for key bindings and feature details.

## Dependencies

- `chrono` — Date/time
- `ratatui` — TUI framework
- `crossterm` — Terminal I/O
- `dirs` — Home directory
- `ratatui-code-editor` — Embedded editor
- `libc`, `nix` — Unix-only (permissions, subshell PTY)

## Cursor / AI assistant skills

This repo includes [agent skills](https://github.com/sickn33/antigravity-awesome-skills) under `.cursor/skills/` for use in Cursor Chat (e.g. `@rust-pro`). They are vendored from the community collection [**antigravity-awesome-skills**](https://github.com/sickn33/antigravity-awesome-skills) (MIT).

| Skill | Location | Upstream source |
|--------|----------|-----------------|
| **rust-pro** | `.cursor/skills/rust-pro/SKILL.md` | [`skills/rust-pro`](https://github.com/sickn33/antigravity-awesome-skills/tree/main/skills/rust-pro) |
| **rust-async-patterns** | `.cursor/skills/rust-async-patterns/SKILL.md` | [`skills/rust-async-patterns`](https://github.com/sickn33/antigravity-awesome-skills/tree/main/skills/rust-async-patterns) (includes `resources/implementation-playbook.md`) |
| **posix-shell-pro** | `.cursor/skills/posix-shell-pro/SKILL.md` | [`skills/posix-shell-pro`](https://github.com/sickn33/antigravity-awesome-skills/tree/main/skills/posix-shell-pro) |

The upstream `posix-shell-pro` skill references a playbook file that is not shipped in that repository; this project adds `.cursor/skills/posix-shell-pro/resources/implementation-playbook.md` as a short pointer so that instruction is not a dead link.

### Project-local skills

Design and architecture notes maintained in-repo (YAML frontmatter for id/tags). Reference them in Chat via path or `@`-mention if your Cursor setup indexes them; you can also copy or symlink into `.cursor/skills/` for the same layout as vendored skills.

| Topic | File |
|-------|------|
| Single Responsibility Principle (SRP) | [`skills/principles/single_responsibility.md`](skills/principles/single_responsibility.md) |

## License

[LICENSE](LICENSE) — MIT License.
