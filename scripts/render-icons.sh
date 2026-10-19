#!/bin/sh
# Renders assets/brindle.svg into the Windows icons, which are committed:
#
#   assets/windows/brindle.ico      8 sizes, 16 to 256 px, each stored as PNG
#   assets/windows/brindle-256.png  the toast icon (the AUMID's IconUri)
#
# Re-run it after changing the SVG; CI fails while the SVG is newer in git
# history than the .ico. It renders with rsvg-convert, resvg or ImageMagick,
# whichever is installed first, and runs under Git Bash on Windows or on
# Linux.
set -eu

cd "$(dirname "$0")/.."
svg=assets/brindle.svg
out=assets/windows
sizes="16 20 24 32 40 48 64 256"

if command -v rsvg-convert >/dev/null; then
    render() { rsvg-convert -w "$1" -h "$1" -o "$2" "$svg"; }
elif command -v resvg >/dev/null; then
    render() { resvg -w "$1" -h "$1" "$svg" "$2"; }
elif command -v magick >/dev/null; then
    # The SVG is 512 units wide and ImageMagick reads it at 96 dpi, so this
    # density rasterizes it at the target size instead of scaling a render.
    # No date chunks, so unchanged renders stay byte-identical.
    render() {
        magick -background none -density "$(awk "BEGIN { print 96 * $1 / 512 }")" "$svg" \
            -define png:exclude-chunks=date,time -strip "png32:$2"
    }
else
    echo "render-icons.sh: install rsvg-convert, resvg or ImageMagick (magick)" >&2
    exit 1
fi

# Relative, because Git Bash's /tmp means nothing to a native magick.exe.
mkdir -p target
tmp=$(mktemp -d target/render-icons.XXXXXX)
trap 'rm -rf "$tmp"' EXIT
for size in $sizes; do
    render "$size" "$tmp/$size.png"
done
cp "$tmp/256.png" "$out/brindle-256.png"

# Little-endian bytes, through octal escapes for printf.
le() { # value, byte count
    v=$1 n=$2
    while [ "$n" -gt 0 ]; do
        printf "\\$(printf %03o $((v & 255)))"
        v=$((v >> 8)) n=$((n - 1))
    done
}

# ICONDIR, one 16-byte ICONDIRENTRY per image (a width or height of 0 means
# 256), then the PNGs in the same order.
count=$(echo $sizes | wc -w)
offset=$((6 + 16 * count))
{
    le 0 2; le 1 2; le "$count" 2
    for size in $sizes; do
        bytes=$(wc -c <"$tmp/$size.png")
        le $((size & 255)) 1; le $((size & 255)) 1; le 0 1; le 0 1
        le 1 2; le 32 2; le "$bytes" 4; le "$offset" 4
        offset=$((offset + bytes))
    done
    for size in $sizes; do
        cat "$tmp/$size.png"
    done
} >"$out/brindle.ico"

echo "wrote $out/brindle.ico ($count sizes) and $out/brindle-256.png"
