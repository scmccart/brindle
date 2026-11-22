#!/bin/sh
# Removes what install.sh, or a release's one-command install.sh
# (scripts/get.sh), installed for the same PREFIX (default ~/.local): the
# launcher entry and icon, brindle and the helper beside it. Your config in
# ~/.config/brindle is kept.
set -eu

PREFIX=${PREFIX:-$HOME/.local}
case $PREFIX in
    /*) ;;
    *) PREFIX=$(pwd)/$PREFIX ;;
esac
bin=$PREFIX/bin/brindle

writable() {
    d=$1
    while [ ! -e "$d" ]; do d=$(dirname "$d"); done
    [ -w "$d" ]
}
if writable "$PREFIX/bin" && writable "$PREFIX/share"; then sudo=; else sudo=sudo; fi

if [ -x "$bin" ]; then
    $sudo "$bin" --uninstall-desktop --prefix "$PREFIX"
else
    # Without the binary, remove the two files it would have (see src/desktop.rs).
    $sudo rm -f "$PREFIX/share/applications/brindle.desktop" \
        "$PREFIX/share/icons/hicolor/scalable/apps/brindle.svg"
fi
# The helper is brindle-remote from install.sh, brindle-remote-linux-x86_64
# from a release.
$sudo rm -f "$bin" "$PREFIX/bin/brindle-remote" "$PREFIX/bin/brindle-remote-linux-x86_64"
