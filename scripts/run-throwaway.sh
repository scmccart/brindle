#!/bin/sh
# Runs Brindle with throwaway state, for trying a build by hand without
# touching your real config, saved session or terminal server:
#
#   scripts/run-throwaway.sh                   # a fresh default config
#   scripts/run-throwaway.sh --config FILE     # use FILE in place
#   scripts/run-throwaway.sh --keep            # keep the directory afterwards
#   scripts/run-throwaway.sh -- terminal -d /tmp   # arguments after -- go to brindle
#
# On Windows use scripts/run-throwaway.ps1. It takes the same options, and
# one more, -NoLinuxHelper: only a Windows build lacks the Linux helper.
#
# It makes a private directory (under /tmp, since the terminal server's
# socket paths have a 108-byte limit) and points BRINDLE_STATE_DIR,
# BRINDLE_SERVER_DIR and BRINDLE_CACHE_DIR (where a local agent's attached
# text is kept) into it. So the launch doesn't hand itself to a Brindle
# you already have running, and the sessions run in their own terminal
# server, which is stopped (and the directory removed) when Brindle exits.
# --keep keeps the directory, for its logs and config, but still stops the
# server.
#
# Without --config, BRINDLE_CONFIG is a fresh config.toml in that directory,
# removed with it. --config FILE points BRINDLE_CONFIG at FILE itself (its
# absolute path): edits to it apply live, the settings dialog writes it, and
# the cleanup leaves it. Pass a copy to keep an original untouched. A profile
# in it that runs tmux attaches to the tmux server it names, which is yours
# unless its args pass `-L` or `-S`. Check it first.
#
# The binary is BRINDLE_BIN, else target/release/brindle, else
# target/debug/brindle. `brindle-remote` must be beside it (cargo build
# builds both): it runs the terminal server, and SSH and container profiles
# deploy it. For a binary in this checkout's target/, a helper older than
# any file under remote-agent/src, remote-protocol/src, term-core/src or
# transcript/src, or than remote-agent/Cargo.toml or Cargo.lock, stops the
# run: run cargo build. It builds nothing itself.
#
# This opens a window on your desktop and takes focus: don't start it
# while typing elsewhere.
set -eu

root=$(CDPATH='' cd "$(dirname "$0")/.." && pwd)
config=
keep=0
while [ $# -gt 0 ]; do
    case $1 in
        --config)
            [ $# -ge 2 ] || { echo "run-throwaway: --config needs a file" >&2; exit 2; }
            config=$2
            shift 2
            ;;
        --keep)
            keep=1
            shift
            ;;
        --)
            shift
            break
            ;;
        -h | --help)
            sed -n '2,/^set -eu/p' "$0" | sed '$d; s/^# \{0,1\}//'
            exit 0
            ;;
        *)
            echo "run-throwaway: unknown option $1 (arguments for brindle go after --)" >&2
            exit 2
            ;;
    esac
done

bin=${BRINDLE_BIN:-}
if [ -z "$bin" ]; then
    for candidate in "$root/target/release/brindle" "$root/target/debug/brindle"; do
        [ -x "$candidate" ] && bin=$candidate && break
    done
fi
if [ -z "$bin" ] || [ ! -x "$bin" ]; then
    echo "run-throwaway: no brindle binary (cargo build --release, or set BRINDLE_BIN)" >&2
    exit 1
fi
# Absolute, to tell whether it is under target/.
bin=$(CDPATH='' cd -- "$(dirname -- "$bin")" && pwd)/$(basename -- "$bin")
helper=$(dirname -- "$bin")/brindle-remote
if [ ! -x "$helper" ]; then
    echo "run-throwaway: $helper is missing (cargo build builds it)" >&2
    exit 1
fi
# A helper built here must be no older than its inputs. The four directories
# are tests/common/server.rs `check_helper`'s, and scripts/linux-helper.ps1
# checks the same list: change the three together. The manifest and the
# lockfile catch a version bump or a dependency change that touches no source.
case $bin in
    "$root"/target/*)
        newer=$(find "$root/remote-agent/src" "$root/remote-protocol/src" \
            "$root/term-core/src" "$root/transcript/src" \
            "$root/remote-agent/Cargo.toml" "$root/Cargo.lock" \
            -type f -newer "$helper" 2>/dev/null | head -n 1)
        if [ -n "$newer" ]; then
            build="cargo build"
            [ "$(basename -- "$(dirname -- "$bin")")" = release ] && build="cargo build --release"
            echo "run-throwaway: $helper is older than ${newer#"$root"/}: run $build" >&2
            exit 1
        fi
        ;;
esac
if [ -n "$config" ]; then
    if [ ! -f "$config" ]; then
        echo "run-throwaway: $config is not a file" >&2
        exit 1
    fi
    # Absolute, since Brindle's working directory can differ from this one.
    config=$(CDPATH='' cd -- "$(dirname -- "$config")" && pwd)/$(basename -- "$config")
fi

dir=$(mktemp -d /tmp/brindle-throwaway.XXXXXX)
mkdir "$dir/state" "$dir/server"
chmod 700 "$dir/server"

cleanup() {
    BRINDLE_SERVER_DIR="$dir/server" "$helper" stop >/dev/null 2>&1 || true
    if [ "$keep" = 1 ]; then
        echo "run-throwaway: kept $dir" >&2
    else
        rm -rf -- "$dir"
    fi
}
trap cleanup EXIT
trap 'exit 130' INT TERM HUP

echo "run-throwaway: $bin (state in $dir)" >&2
if [ -n "$config" ]; then
    echo "run-throwaway: config $config (edits apply live; the settings dialog writes it)" >&2
fi
# Not `exec`, so the trap runs. brindle is a GUI program and may return as
# soon as the window closes.
BRINDLE_CONFIG="${config:-$dir/config.toml}" \
BRINDLE_STATE_DIR="$dir/state" \
BRINDLE_SERVER_DIR="$dir/server" \
BRINDLE_CACHE_DIR="$dir/cache" \
    "$bin" "$@"
