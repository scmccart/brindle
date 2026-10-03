#!/usr/bin/env bash
# Rootless build toolchain for machines without sudo / dev packages.
# Downloads Ubuntu .debs (gcc, pkg-config, -dev headers, tmux) and extracts
# them into .toolchain/root. Then: `source .toolchain/env.sh && cargo build`.
#
# On a normal machine you don't need this; install instead:
#   sudo apt install build-essential pkg-config libxkbcommon-dev libxkbcommon-x11-dev \
#     libwayland-dev libfontconfig-dev libfreetype-dev libvulkan-dev libxcb1-dev \
#     libx11-xcb-dev libssl-dev tmux
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$HERE/root"
PKGS=(build-essential pkg-config libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev
  libfontconfig-dev libfreetype-dev libxcb1-dev libx11-xcb-dev libx11-dev libvulkan-dev
  libssl-dev tmux)

mkdir -p "$HERE/debs" "$ROOT"
cd "$HERE/debs"
mapfile -t NEEDED < <(apt-get install -s --no-install-recommends "${PKGS[@]}" 2>/dev/null \
  | awk '/^Inst/ {print $2}' | grep -vE '^(build-essential|dpkg-dev|libdpkg-perl|lto-disabled-list|bzip2|xorg-sgml-doctools|xtrans-dev|libwayland-bin)$')
if [ "${#NEEDED[@]}" -gt 0 ]; then
  apt-get download "${NEEDED[@]}"
fi
for f in "$HERE"/debs/*.deb; do dpkg -x "$f" "$ROOT"; done

# Dev symlinks point at runtime libs that are installed system-wide, not in ROOT.
LIB="$ROOT/usr/lib/x86_64-linux-gnu"
for l in $(find "$LIB" -maxdepth 1 -type l); do
  if [ ! -e "$l" ]; then
    t="$(readlink "$l")"
    [ -e "/usr/lib/x86_64-linux-gnu/$t" ] && ln -sf "/usr/lib/x86_64-linux-gnu/$t" "$l"
  fi
done

# The relocated gcc driver only looks for cc1 & friends next to itself.
GCCVER="$(ls "$ROOT/usr/libexec/gcc/x86_64-linux-gnu" | head -1)"
D="$ROOT/usr/libexec/gcc/x86_64-linux-gnu/$GCCVER"
for f in /usr/libexec/gcc/x86_64-linux-gnu/"$GCCVER"/*; do
  [ -e "$D/$(basename "$f")" ] || ln -s "$f" "$D/"
done
ln -sf gcc "$ROOT/usr/bin/cc"
ln -sf g++ "$ROOT/usr/bin/c++"
echo "Toolchain ready. Run: source .toolchain/env.sh"
