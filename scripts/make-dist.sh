#!/bin/sh
set -eu

# Build release binary and pack xd + install.sh into dist/xd-VERSION-OS-ARCH.tar.gz

root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd -P)
cd "$root" || exit 1

version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -n 1)
if [ -z "$version" ]; then
	printf 'could not read version from Cargo.toml\n' >&2
	exit 1
fi

os=$(uname -s | tr '[:upper:]' '[:lower:]')
arch=$(uname -m)
name="xd-${version}-${os}-${arch}.tar.gz"

cargo build --release

stage="$root/dist/stage"
rm -rf "$stage"
mkdir -p "$stage"
cp -- "$root/target/release/xd" "$stage/xd"
cp -- "$root/install/install.sh" "$stage/install.sh"
chmod 755 "$stage/xd" "$stage/install.sh"

mkdir -p "$root/dist"
(cd "$stage" && tar czf "$root/dist/$name" xd install.sh)

printf 'Wrote %s\n' "$root/dist/$name"
