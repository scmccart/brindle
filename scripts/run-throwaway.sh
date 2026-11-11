#!/bin/sh
# Runs Brindle with throwaway state, for trying a build by hand without
# touching your real config, saved session or terminal server:
#
#   scripts/run-throwaway.sh                   # a default config
#   scripts/run-throwaway.sh --config FILE     # start from a copy of FILE
#   scripts/run-throwaway.sh --keep            # keep the directory afterwards
#   scripts/run-throwaway.sh -- -d /tmp        # arguments after -- go to brindle
#
# On Windows use scripts/run-throwaway.ps1.
#
# It makes a private directory (under /tmp, since the terminal server's
# socket paths have a 108-byte limit) and points BRINDLE_CONFIG,
# BRINDLE_STATE_DIR and BRINDLE_SERVER_DIR into it. So the settings dialog
# writes the throwaway config, the launch doesn't hand itself to a Brindle
# you already have running, and the sessions run in their own terminal
# server, which is stopped (and the directory removed) when Brindle exits.
#
# The binary is BRINDLE_BIN, else target/release/brindle, else
# target/debug/brindle; `brindle-remote` must be beside it (cargo build
# builds both).
#
# --config copies a file in, so a copied config's profile that runs tmux would
# attach to the tmux server it names, which is yours unless its args pass `-L`
# or `-S`. Check it first.
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
helper=$(dirname -- "$bin")/brindle-remote
if [ ! -x "$helper" ]; then
    echo "run-throwaway: $helper is missing (cargo build builds it)" >&2
    exit 1
fi
if [ -n "$config" ] && [ ! -f "$config" ]; then
    echo "run-throwaway: $config is not a file" >&2
    exit 1
fi

dir=$(mktemp -d /tmp/brindle-throwaway.XXXXXX)
mkdir "$dir/state" "$dir/server"
chmod 700 "$dir/server"
if [ -n "$config" ]; then
    cp -- "$config" "$dir/config.toml"
fi

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
# Not `exec`, so the trap runs. brindle is a GUI program and may return as
# soon as the window closes.
BRINDLE_CONFIG="$dir/config.toml" \
BRINDLE_STATE_DIR="$dir/state" \
BRINDLE_SERVER_DIR="$dir/server" \
    "$bin" "$@"
