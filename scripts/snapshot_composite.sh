#!/usr/bin/env bash
# Stitch an egui_kittest snapshot failure into one "old | diff | new" strip for
# one-glance review.
#
# When a snapshot test fails, egui_kittest writes sibling artifacts next to the
# committed baseline `<name>.png` (all gitignored):
#   <name>.old.png   the previous baseline (only after UPDATE_SNAPSHOTS rewrote it)
#   <name>.diff.png  the visual diff — the fastest way to see *what* changed
#   <name>.new.png   the freshly rendered image
# This wrapper appends old, diff, and new horizontally (ImageMagick `+append`)
# into `<name>.composite.png` so you can eyeball all three at once.
#
# Usage:
#   scripts/snapshot_composite.sh <path-to-snapshot.png> [-o out.png]
#
# The argument may be the baseline (`<name>.png`) or any of its siblings
# (`<name>.diff.png`, etc.) — the base name is derived either way. "old" prefers
# `<name>.old.png` and falls back to the committed `<name>.png`. To measure the
# pixels of any of these images, see scripts/measure_snapshot.py.
set -euo pipefail

if [[ $# -lt 1 ]]; then
    sed -n '2,22p' "$0" | sed 's/^# \{0,1\}//'
    exit 2
fi

input=$1
out=""
shift
while [[ $# -gt 0 ]]; do
    case "$1" in
        -o | --output)
            out=$2
            shift 2
            ;;
        *)
            echo "error: unknown argument '$1'" >&2
            exit 2
            ;;
    esac
done

command -v convert >/dev/null 2>&1 || {
    echo "error: ImageMagick 'convert' not found." >&2
    exit 1
}

# Strip any known suffix to get the base name (dir + stem, no extension).
base=$input
for suffix in .new.png .diff.png .old.png .composite.png .png; do
    if [[ $base == *"$suffix" ]]; then
        base=${base%"$suffix"}
        break
    fi
done

old="$base.old.png"
[[ -f $old ]] || old="$base.png"
diff="$base.diff.png"
new="$base.new.png"
: "${out:=$base.composite.png}"

missing=()
for f in "$old" "$diff" "$new"; do
    [[ -f $f ]] || missing+=("$f")
done
if [[ ${#missing[@]} -gt 0 ]]; then
    echo "error: missing required image(s):" >&2
    printf '  %s\n' "${missing[@]}" >&2
    echo "Run the snapshot tests first so the .diff.png/.new.png artifacts exist." >&2
    exit 1
fi

# A gray frame separates the three panels; order is old | diff | new.
convert "$old" "$diff" "$new" \
    -bordercolor '#888888' -border 4 \
    +append "$out"

echo "Wrote $out"
echo "  panels (left -> right):  old | diff | new"
echo "    old : $old"
echo "    diff: $diff"
echo "    new : $new"
