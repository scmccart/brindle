#!/bin/sh
# Packs the release build into the Linux release archive,
# target/dist/brindle-<version>-x86_64-unknown-linux-gnu.tar.gz: brindle, the
# static Linux x86-64 remote helper (as brindle-remote-linux-x86_64, the name
# Brindle looks for beside itself) and Brindle's licences, in a top-level
# directory of the same name. Prints the archive's path.
#
#   cargo build --release --locked
#   cargo build --release --locked -p brindle-remote-agent --target x86_64-unknown-linux-musl
#   scripts/package-linux.sh [EXE [REMOTE_AGENT]]
#
# It refuses binaries whose --version isn't the crate version, and a helper
# that isn't statically linked, since it must run on any Linux host.
set -eu

cd "$(dirname "$0")/.."
exe=${1:-target/release/brindle}
agent=${2:-target/x86_64-unknown-linux-musl/release/brindle-remote}

version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)
[ -n "$version" ] || { echo "package-linux: no version in Cargo.toml" >&2; exit 1; }

for f in "$exe" "$agent"; do
    [ -x "$f" ] || { echo "package-linux: no $f; build it first" >&2; exit 1; }
done
got=$("$exe" --version)
[ "$got" = "brindle $version" ] || { echo "package-linux: $exe --version is '$got', expected 'brindle $version'" >&2; exit 1; }
got=$("$agent" --version)
[ "$got" = "$version" ] || { echo "package-linux: $agent --version is '$got', expected '$version'" >&2; exit 1; }
case $(file -b "$agent") in
    ELF*static*) ;;
    *) echo "package-linux: $agent is not a statically linked ELF binary" >&2; exit 1 ;;
esac

name=brindle-$version-x86_64-unknown-linux-gnu
dist=target/dist
rm -rf "${dist:?}/$name" "$dist/$name.tar.gz"
mkdir -p "$dist/$name"
install -m755 "$exe" "$dist/$name/brindle"
install -m755 "$agent" "$dist/$name/brindle-remote-linux-x86_64"
install -m644 LICENSE-MIT LICENSE-APACHE "$dist/$name/"
tar -C "$dist" --owner=0 --group=0 --numeric-owner --sort=name -czf "$dist/$name.tar.gz" "$name"
rm -rf "${dist:?}/$name"
echo "$(pwd)/$dist/$name.tar.gz"
