#!/bin/sh
set -eu

usage() {
	printf '%s\n' "Usage: install.sh [-p PREFIX] [BINARY]" >&2
	printf '%s\n' "" >&2
	printf '%s\n' "Install xd into PREFIX. Default PREFIX is \$HOME/.local/bin when writable," >&2
	printf '%s\n' "otherwise /usr/local/bin (may prompt for sudo)." >&2
	printf '%s\n' "BINARY defaults to ./xd next to this script." >&2
	exit 1
}

PREFIX="${PREFIX:-}"
BINARY=""

while [ "$#" -gt 0 ]; do
	case "$1" in
	-h | --help)
		usage
		;;
	-p)
		[ "$#" -ge 2 ] || usage
		PREFIX="$2"
		shift 2
		;;
	-*)
		printf 'unknown option: %s\n' "$1" >&2
		usage
		;;
	*)
		break
		;;
	esac
done

[ "$#" -le 1 ] || usage
[ "$#" -eq 1 ] && BINARY="$1"

case "$0" in
/*) d=$(dirname "$0") ;;
*) d=$(dirname "$(pwd)/$0") ;;
esac
cd "$d" || exit 1
here=$(pwd)

if [ -z "$BINARY" ]; then
	BINARY="$here/xd"
elif [ ! -f "$BINARY" ] && [ -f "$here/$BINARY" ]; then
	BINARY="$here/$BINARY"
fi

if [ ! -f "$BINARY" ]; then
	printf 'binary not found: %s\n' "$BINARY" >&2
	exit 1
fi

if [ -z "$PREFIX" ]; then
	if [ -n "${HOME:-}" ] && mkdir -p "$HOME/.local/bin" 2>/dev/null && [ -w "$HOME/.local/bin" ]; then
		PREFIX="$HOME/.local/bin"
	else
		PREFIX="/usr/local/bin"
	fi
fi

mkdir -p "$PREFIX" || {
	printf 'cannot create directory: %s\n' "$PREFIX" >&2
	exit 1
}

if [ -w "$PREFIX" ]; then
	cp -- "$BINARY" "$PREFIX/xd"
	chmod 755 "$PREFIX/xd"
else
	if command -v sudo >/dev/null 2>&1; then
		sudo cp -- "$BINARY" "$PREFIX/xd"
		sudo chmod 755 "$PREFIX/xd"
	else
		printf 'no write access to %s and sudo not found\n' "$PREFIX" >&2
		exit 1
	fi
fi

printf 'Installed xd -> %s/xd\n' "$PREFIX"
printf 'Ensure %s is on your PATH.\n' "$PREFIX"
