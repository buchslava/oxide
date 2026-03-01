# Creating a Debian Package

This document describes how to create a `.deb` package for Oxide (xd) to install or update the app via apt/dpkg.

## Option 1: cargo-deb (recommended)

[cargo-deb](https://crates.io/crates/cargo-deb) is a Rust-specific tool that generates `.deb` packages from Cargo projects.

### Install

```bash
cargo install cargo-deb
```

### Build package

```bash
cargo deb
```

The package is written to `target/debian/xd_0.1.0_amd64.deb` (version and arch vary). Optional metadata (maintainer, description, etc.) can be set in `Cargo.toml` under `[package.metadata.deb]`.

### Install the package

```bash
sudo dpkg -i target/debian/xd_*_amd64.deb
```

---

## Option 2: fpm (Effing Package Manager)

[fpm](https://fpm.readthedocs.io/) creates packages from directories or binaries.

### Install

```bash
gem install fpm
```

### Build package

```bash
cargo build --release
fpm -s dir -t deb -n xd -v 0.1.0 \
  ./target/release/xd=/usr/bin/xd
```

### Install the package

```bash
sudo dpkg -i xd_0.1.0_amd64.deb
```

---

## Option 3: Full Debian packaging

For publishing to a PPA or Debian-style repository:

- Create a `debian/` directory with packaging metadata
- Use `dh-cargo` or `cargo-deb` as a build helper
- Use `debuild` or `dpkg-buildpackage` to build

This workflow is more involved but suitable for official distribution.

---

## Recommendation

**cargo-deb** is the simplest for most use cases: it infers metadata from `Cargo.toml`, packages the binary correctly, and works with `cargo deb`.
