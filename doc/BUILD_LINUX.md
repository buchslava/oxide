# Building on Linux (Debian/Ubuntu)

This document describes the system dependencies and build flow for Oxide on Debian-based Linux distributions.

## Required apt packages

Install the following packages before building:

```bash
sudo apt update
sudo apt install -y build-essential pkg-config libxcb1-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev
```

### Package rationale

| Package | Purpose |
|---------|---------|
| **build-essential** | Provides GCC, g++, and make. Required because the `tree-sitter` grammars (used by `ratatui-code-editor` for syntax highlighting) are written in C and compiled during the build. |
| **pkg-config** | Used to locate libraries. Many crates that link to system libraries rely on it. |
| **libxcb1-dev** | XCB client library. Required by `x11rb` (used by `arboard` for clipboard support in the editor). |
| **libxcb-render0-dev** | XCB Render extension. |
| **libxcb-shape0-dev** | XCB Shape extension. |
| **libxcb-xfixes0-dev** | XCB XFixes extension (clipboard functionality). |

The `arboard` crate (clipboard support for Ctrl+C/V in the embedded editor) is pulled in via `ratatui-code-editor`. On Linux/X11, it uses `x11rb`, which links against libxcb.

## Build flow

### 1. Install system dependencies

```bash
sudo apt update
sudo apt install -y build-essential pkg-config libxcb1-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev
```

### 2. Install Rust (if not already installed)

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
```

### 3. Build

```bash
cd /path/to/oxide
cargo build --release
```

### 4. Run

```bash
./target/release/xd
```

### 5. Optional: Install to ~/.cargo/bin

```bash
cargo install --path .
```

## One-liner flow

```bash
# System deps
sudo apt install -y build-essential pkg-config libxcb1-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev

# Rust (if not installed)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source ~/.cargo/env

# Build
cd oxide && cargo build --release
./target/release/xd
```

## Notes

- **Wayland**: The X11 stack works under Wayland via XWayland, so clipboard should function normally.
- **Headless/Server**: Clipboard features may not work without an X session. The app should still build and run; editor copy/paste may fail. Installing the packages above is still recommended for a full build.
- **Minimal systems**: On minimal containers or stripped-down images, you may also need `ca-certificates` and `curl` for rustup:

  ```bash
  sudo apt install -y ca-certificates curl
  ```
