#!/bin/sh
# Installs a Brindle release on Linux x86-64: downloads the release's archive,
# checks it against the release's SHA256SUMS, installs brindle and its remote
# helper in $PREFIX/bin (default ~/.local) and adds the launcher entry and
# icon. Each release publishes this script as install.sh, stamped with its own
# version:
#
#   curl -fsSL https://github.com/scmccart/brindle/releases/latest/download/install.sh | sh
#   curl -fsSL .../install.sh | env BRINDLE_VERSION=0.4.0 sh   # another release
#   curl -fsSL .../install.sh | env PREFIX=/usr/local sh       # system-wide; uses sudo to copy
#   curl -fsSL .../install.sh | sh -s -- --uninstall           # remove it again
#
# Re-run it to update; a Brindle that is running keeps its old build until it
# quits. Uninstalling keeps your config in ~/.config/brindle. A copy that was
# never stamped (this file in a checkout) installs the latest release.
# BRINDLE_DOWNLOAD_BASE replaces the release's download URL, for testing
# against a directory of built assets (file:///DIR).
#
# scripts/install.sh and scripts/uninstall.sh do the same from a checkout.
# They keep their own copies of these steps, since this script must run on
# its own. Its whole body is functions called on the last line, so a download
# cut off partway runs nothing.

usage() {
    cat <<'EOF'
usage: install.sh [--uninstall]

Installs Brindle in $PREFIX/bin (default ~/.local/bin), with the launcher
entry and icon, or with --uninstall removes them again.

  BRINDLE_VERSION        the release to install (default: this script's)
  PREFIX                 where to install (default: ~/.local)
  BRINDLE_DOWNLOAD_BASE  where to download from, for testing (file:///DIR)
EOF
}

say() {
    printf 'install.sh: %s\n' "$*"
}

die() {
    printf 'install.sh: %s\n' "$*" >&2
    exit 1
}

# Whether the nearest existing ancestor of $1 is writable (as in
# scripts/install.sh).
writable() {
    d=$1
    while [ ! -e "$d" ]; do d=$(dirname "$d"); done
    [ -w "$d" ]
}

# Sets $sudo: empty when the user can write $PREFIX, else sudo, which is then
# used only to write there.
choose_sudo() {
    if writable "$PREFIX/bin" && writable "$PREFIX/share"; then
        sudo=
    elif command -v sudo >/dev/null 2>&1; then
        sudo=sudo
        say "$PREFIX isn't writable by you; using sudo to write there"
    else
        die "$PREFIX isn't writable by you, and sudo is missing"
    fi
}

# Runs a command, through sudo when choose_sudo chose it.
as_owner() {
    if [ -n "$sudo" ]; then sudo "$@"; else "$@"; fi
}

# Downloads URL $1 to file $2.
fetch() {
    case $1 in
        file://*) cp "${1#file://}" "$2" ;;
        *)
            if [ -z "$downloader" ]; then
                if command -v curl >/dev/null 2>&1; then
                    downloader=curl
                elif command -v wget >/dev/null 2>&1; then
                    downloader=wget
                else
                    die "needs curl or wget to download the release"
                fi
            fi
            if [ "$downloader" = curl ]; then
                curl -fsSL --proto-redir '=https' -o "$2" "$1"
            else
                wget -q -O "$2" "$1"
            fi
            ;;
    esac || die "can't download $1; nothing was installed"
}

# Prints the SHA-256 of file $1, in lower case.
sha256() {
    if [ "$hasher" = sha256sum ]; then
        sha256sum "$1"
    else
        shasum -a 256 "$1"
    fi | cut -d ' ' -f 1 | tr 'A-F' 'a-f'
}

# Stops with what brindle printed ($1) and the build's requirements.
not_runnable() {
    printf '%s\n' "$1" >&2
    glibc=$(getconf GNU_LIBC_VERSION 2>/dev/null) || glibc='no glibc'
    die "brindle doesn't run on this system ($glibc). Release builds are made on Ubuntu 26.04 and need a glibc at least that new, plus the xkbcommon, xcb and FreeType libraries; to run elsewhere, build from source (see README.md's Building)"
}

install_release() {
    os=$(uname -s)
    arch=$(uname -m)
    case $os/$arch in
        Linux/x86_64 | Linux/amd64) ;;
        *) die "only Linux x86-64 has a release archive, and this is $os $arch; build from source instead (see README.md's Building)" ;;
    esac
    downloader=
    if command -v sha256sum >/dev/null 2>&1; then
        hasher=sha256sum
    elif command -v shasum >/dev/null 2>&1; then
        hasher=shasum
    else
        die "needs sha256sum or shasum to check the download"
    fi

    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' EXIT
    trap 'exit 1' HUP INT TERM

    # The release stamps its version over this placeholder; a version never
    # holds an @, so one still here means the copy was never stamped.
    stamped='@VERSION@'
    case $stamped in *@*) stamped= ;; esac
    version=${BRINDLE_VERSION:-$stamped}
    if [ -z "$version" ]; then
        api=https://api.github.com/repos/scmccart/brindle/releases/latest
        fetch "$api" "$tmp/latest.json"
        version=$(sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' "$tmp/latest.json" | head -n 1)
        [ -n "$version" ] || die "$api names no release; set BRINDLE_VERSION"
    fi
    version=${version#v}
    case $version in
        '' | *[!0-9A-Za-z.+-]*) die "'$version' isn't a version" ;;
    esac

    base=${BRINDLE_DOWNLOAD_BASE:-https://github.com/scmccart/brindle/releases/download/v$version}
    base=${base%/}
    name=brindle-$version-x86_64-unknown-linux-gnu
    archive=$name.tar.gz

    say "downloading Brindle $version"
    fetch "$base/SHA256SUMS" "$tmp/SHA256SUMS"
    fetch "$base/$archive" "$tmp/$archive"
    want=$(awk -v f="$archive" '$2 == f || $2 == "*" f { print $1; exit }' "$tmp/SHA256SUMS" | tr 'A-F' 'a-f')
    [ -n "$want" ] || die "SHA256SUMS has no line for $archive; nothing was installed"
    got=$(sha256 "$tmp/$archive")
    [ "$got" = "$want" ] || die "checksum failed for $archive (expected $want, got $got); nothing was installed"

    tar -xzf "$tmp/$archive" -C "$tmp"
    for file in brindle brindle-remote-linux-x86_64; do
        [ -f "$tmp/$name/$file" ] || die "$archive has no $name/$file; nothing was installed"
    done
    # Check that it runs here before replacing anything. A temporary
    # directory mounted noexec can't run it at all (status 126); the
    # installed copy is checked below instead.
    status=0
    said=$("$tmp/$name/brindle" --version 2>&1) || status=$?
    case $status in
        0 | 126) ;;
        *) not_runnable "$said" ;;
    esac

    if [ "$(id -u)" -eq 0 ] && [ -n "${SUDO_USER:-}" ]; then
        say "note: running as root through sudo, so what goes in $PREFIX belongs to root; to install for $SUDO_USER, run it without sudo"
    fi
    choose_sudo
    # install(1) replaces each file rather than writing over it, so a running
    # Brindle is unaffected.
    as_owner install -Dm755 "$tmp/$name/brindle" "$PREFIX/bin/brindle"
    as_owner install -Dm755 "$tmp/$name/brindle-remote-linux-x86_64" "$PREFIX/bin/brindle-remote-linux-x86_64"
    said=$("$PREFIX/bin/brindle" --version 2>&1) || not_runnable "$said"
    # Run the installed copy, so the desktop entry's Exec points at it.
    as_owner "$PREFIX/bin/brindle" --install-desktop --prefix "$PREFIX"

    case ":$PATH:" in
        *":$PREFIX/bin:"*) ;;
        *) say "note: $PREFIX/bin is not on your PATH; the launcher entry works regardless" ;;
    esac
    say "installed $said in $PREFIX/bin"
}

# The steps of scripts/uninstall.sh.
uninstall_release() {
    bin=$PREFIX/bin/brindle
    choose_sudo
    if [ -x "$bin" ]; then
        as_owner "$bin" --uninstall-desktop --prefix "$PREFIX"
    else
        # Without the binary, remove the two files it would have (see src/desktop.rs).
        as_owner rm -f "$PREFIX/share/applications/brindle.desktop" \
            "$PREFIX/share/icons/hicolor/scalable/apps/brindle.svg"
    fi
    as_owner rm -f "$bin" "$PREFIX/bin/brindle-remote" "$PREFIX/bin/brindle-remote-linux-x86_64"
    say "uninstalled Brindle from $PREFIX; your config in ~/.config/brindle is kept"
}

main() {
    set -eu
    action=install
    for arg in "$@"; do
        case $arg in
            --uninstall) action=uninstall ;;
            -h | --help)
                usage
                exit 0
                ;;
            *) die "unknown option '$arg' (see --help)" ;;
        esac
    done

    PREFIX=${PREFIX:-$HOME/.local}
    case $PREFIX in
        /*) ;;
        *) PREFIX=$(pwd)/$PREFIX ;;
    esac

    if [ "$action" = uninstall ]; then
        uninstall_release
    else
        install_release
    fi
}

main "$@"
