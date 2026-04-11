# Oxide

**xd** — A TUI file manager in the style of Midnight Commander. Built with Rust, Ratatui, and Crossterm.

## Overview

Oxide is a terminal user interface (TUI) file manager with a dual-panel layout. The compiled binary is named **xd**. For background on the project and technical choices, see [the intro article](doc/articles/intro.md).

![Oxide](doc/articles/images/intro.png)

> **Warning:** This is a **pilot** build. It has **not** been fully tested end-to-end, and it may contain bugs or rough edges. So far it has seen real use only from the author—mostly on **macOS**, with lighter use on **Linux**. Be careful with important files and production workflows until you are comfortable with how it behaves.

### Features

See [FEATURES.md](doc/FEATURES.md) for a full list of features and key bindings.

## Build

For Linux (Debian/Ubuntu), see [BUILD_LINUX.md](doc/BUILD_LINUX.md) for system dependencies and build flow.

### Prerequisites

- [Rust](https://www.rust-lang.org/tools/install) (edition 2021)
- On Unix: `libc`, `nix` (for permissions, subshell PTY)

For theory on the subshell, PTY, terminals, and file descriptors, see [SHELL_PTY_TERMINAL.md](doc/SHELL_PTY_TERMINAL.md).

### Precompiled binaries

Prebuilt **xd** binaries live under [install/](install/):

- **Generic Linux** (x86_64): [`install/linux-x86_64/xd`](install/linux-x86_64/xd)
- **Intel macOS** (x86_64): [`install/darwin-x86_64/xd`](install/darwin-x86_64/xd)

Install with [install/install.sh](install/install.sh) (run from the `install` directory and pass the path to the binary that matches your machine):

```bash
cd install
./install.sh -p "$HOME/.local/bin" linux-x86_64/xd    # Linux
./install.sh -p "$HOME/.local/bin" darwin-x86_64/xd # Intel Mac
```

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

### Build static binary (most portable)

Some dependencies (for example **zstd** inside **zip**) compile **C** code. Installing the Rust stdlib for `x86_64-unknown-linux-musl` is not enough on its own: the build also needs a **musl C toolchain** for that target (so `cc` can find something like `x86_64-linux-musl-gcc`, or an equivalent via Zig—see below).

✅ No glibc dependency  
✅ Runs on almost any Linux (Debian, Ubuntu, Kali, Alpine)

**On Debian/Ubuntu amd64** (build *on* Linux):

```bash
sudo apt install musl-tools
rustup target add x86_64-unknown-linux-musl
cargo build --release --target x86_64-unknown-linux-musl
```

The `musl-tools` package provides `x86_64-linux-musl-gcc` on typical PC images. More Linux detail: [BUILD_LINUX.md](doc/BUILD_LINUX.md#static-linux-binary-musl).

**Cross-compiling from macOS** — Rust’s Apple toolchain does not supply a Linux musl C compiler, so plain `cargo build --target x86_64-unknown-linux-musl` fails at crates like `zstd-sys`. Practical options:

1. **Zig as linker** — install [Zig](https://ziglang.org/download/) (e.g. `brew install zig`), then [`cargo-zigbuild`](https://github.com/rust-cross/cargo-zigbuild): `cargo install cargo-zigbuild`, then  
   `cargo zigbuild --release --target x86_64-unknown-linux-musl`
2. **Install a musl cross toolchain** for macOS (Homebrew or similar) and set `CC_x86_64_unknown_linux_musl` / linker in `~/.cargo/config.toml` per that toolchain’s instructions.
3. **Build inside Linux** (VM, container, CI) using the Debian commands above and copy `target/x86_64-unknown-linux-musl/release/xd` out.

Release binary: `target/x86_64-unknown-linux-musl/release/xd`

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

For Rust crate dependencies and Cursor / AI assistant skills (vendored and project-local), see [DEPENDENCIES_AND_SKILLS.md](doc/DEPENDENCIES_AND_SKILLS.md).

## License

[LICENSE](LICENSE) — MIT License.
