#!/bin/sh
# Builds Brindle and installs it, with its launcher entry and icon.
#
#   scripts/install.sh                      # into ~/.local
#   PREFIX=/usr/local scripts/install.sh    # system-wide; uses sudo to copy
#
# Re-run it to update. Windows already open keep running the old build.
set -eu

if [ "$(id -u)" -eq 0 ]; then
    echo "install.sh: run this as your normal user; it uses sudo itself where needed" >&2
    exit 1
fi

PREFIX=${PREFIX:-$HOME/.local}
case $PREFIX in
    /*) ;;
    *) PREFIX=$(pwd)/$PREFIX ;;
esac

cd "$(dirname "$0")/.."
cargo build --release

# Whether the nearest existing ancestor of $1 is writable.
writable() {
    d=$1
    while [ ! -e "$d" ]; do d=$(dirname "$d"); done
    [ -w "$d" ]
}
if writable "$PREFIX/bin" && writable "$PREFIX/share"; then sudo=; else sudo=sudo; fi

# install(1) replaces the file instead of overwriting it in place, so a running
# Brindle is unaffected.
$sudo install -Dm755 target/release/brindle "$PREFIX/bin/brindle"
# Run the installed copy, so the desktop entry's Exec points at it.
$sudo "$PREFIX/bin/brindle" --install-desktop --prefix "$PREFIX"

case ":$PATH:" in
    *":$PREFIX/bin:"*) ;;
    *) echo "note: $PREFIX/bin is not on your PATH; the launcher entry works regardless" ;;
esac
