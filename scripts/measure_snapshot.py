#!/usr/bin/env python3
"""Measure egui_kittest snapshot PNGs at the pixel level.

Our frontend renders widget snapshots headlessly to PNGs under
`frontend/tests/snapshots/` (see DEVELOPMENT.md, "Visual regression tests").
When you need to *measure* one — e.g. confirm a margin is symmetric to the
pixel rather than eyeball it — this tool reads the pixels for you and reports
logical-pixel boundaries and margins, with the rendering conventions baked in:

  * PPP = 2.0          The harness renders at 2x device scale, so every PNG is
                       twice its logical size. Logical px = device px / 2.
  * Harness margin     The snapshot harness crops tightly but leaves an 8
    = 8 logical px      logical-px (16 device-px) fully transparent margin
                       around the content on every side.
  * Border gray 0xC8   A resting widget border paints #C8C8C8 (200,200,200).
  * Background          Fully transparent (alpha 0).

Pixels are classified as **background** (transparent), **border** (~#C8C8C8
gray), **fill** (the dominant opaque non-border color — usually the widget
interior), or **content** (any other opaque color: text, icons, tints).

Usage:
    scripts/measure_snapshot.py <png>                 # whole-image summary
    scripts/measure_snapshot.py <png> --row N         # horizontal scan at row N
    scripts/measure_snapshot.py <png> --col N         # vertical scan at col N
    scripts/measure_snapshot.py <png> --row N --logical   # N is in logical px
    scripts/measure_snapshot.py <png> --row N --json      # machine-readable

Coordinates (N) are **device pixels** by default — the same numbers `identify`
and `convert` report. Pass --logical to give N in logical px instead (it is
multiplied by PPP internally). With no --row/--col, prints a summary: image
size, the bounding box of all non-transparent content, the harness margin on
each side, and left/right & top/bottom symmetry checks.
"""

import argparse
import json
import sys


def _load_deps():
    """Import Pillow + NumPy lazily, with a friendly message if missing.

    Kept out of module top-level so the pure pixel-classification helpers below
    can be imported and unit-tested without these (heavier) dependencies.
    """
    try:
        import numpy as np
        from PIL import Image

        return np, Image
    except ImportError as e:  # pragma: no cover - environment guard
        sys.exit(
            f"error: {e}. This needs Pillow + NumPy. In the dev container they "
            "are preinstalled (apt python3-pil/python3-numpy); rebuild the "
            "image with `docker compose build` if `from PIL import Image` fails."
        )

# --- Baked-in rendering conventions (see module docstring). ------------------
PPP = 2.0
HARNESS_MARGIN_LOGICAL = 8.0
BORDER_GRAY = 200  # 0xC8

# Classification tolerances.
ALPHA_BG_MAX = 8      # alpha <= this -> background (transparent)
GRAY_CHROMA_MAX = 16  # max-min channel spread to still count as neutral gray
BORDER_TOL = 28       # how far each channel may sit from BORDER_GRAY
FILL_WHITE_MIN = 240  # min channel for the "near white" fast path


def dev_to_log(px):
    """Device pixels -> logical pixels (float)."""
    return px / PPP


def fmt_px(dev):
    """Format a device-pixel count as 'Ndev / M.Mlog'."""
    return f"{dev}dev / {dev_to_log(dev):.1f}log"


def classify(px, fill_rgb=None):
    """Classify one RGBA pixel tuple as background/border/fill/content."""
    r, g, b, a = int(px[0]), int(px[1]), int(px[2]), int(px[3])
    if a <= ALPHA_BG_MAX:
        return "background"
    chroma = max(r, g, b) - min(r, g, b)
    if chroma <= GRAY_CHROMA_MAX and all(
        abs(c - BORDER_GRAY) <= BORDER_TOL for c in (r, g, b)
    ):
        return "border"
    if fill_rgb is not None and (r, g, b) == fill_rgb:
        return "fill"
    if fill_rgb is None and min(r, g, b) >= FILL_WHITE_MIN:
        return "fill"
    return "content"


def hexcolor(px):
    r, g, b, a = (int(c) for c in px[:4])
    if a == 0:
        return "#00000000"
    if a == 255:
        return f"#{r:02X}{g:02X}{b:02X}"
    return f"#{r:02X}{g:02X}{b:02X}{a:02X}"


def dominant_fill(line):
    """Most common opaque, non-border color in a 1-D line of RGBA pixels.

    This is what we label "fill": the widget interior, whatever color it is
    (white for text inputs, a tint for builder section frames, ...).
    """
    counts = {}
    for px in line:
        if classify(px) in ("background", "border"):
            continue
        counts[(int(px[0]), int(px[1]), int(px[2]))] = (
            counts.get((int(px[0]), int(px[1]), int(px[2])), 0) + 1
        )
    if not counts:
        return None
    return max(counts, key=counts.get)


def run_length(line, fill_rgb):
    """Collapse a 1-D RGBA line into [(class, color, start, end_inclusive)]."""
    segments = []
    for i, px in enumerate(line):
        cls = classify(px, fill_rgb)
        color = hexcolor(px)
        if segments and segments[-1][0] == cls and segments[-1][1] == color:
            segments[-1][3] = i
        else:
            segments.append([cls, color, i, i])
    return segments


def margin_report(lead_bg, trail_bg, axis_lo, axis_hi):
    """Build a margin dict for one axis given leading/trailing background runs.

    `axis_lo`/`axis_hi` name the two sides, e.g. ("left", "right").
    """
    lo_log = dev_to_log(lead_bg)
    hi_log = dev_to_log(trail_bg)
    return {
        axis_lo: {
            "device": lead_bg,
            "logical": lo_log,
            "beyond_harness": lo_log - HARNESS_MARGIN_LOGICAL,
        },
        axis_hi: {
            "device": trail_bg,
            "logical": hi_log,
            "beyond_harness": hi_log - HARNESS_MARGIN_LOGICAL,
        },
        "symmetric": lead_bg == trail_bg,
        "skew_device": lead_bg - trail_bg,
    }


def lead_trail_bg(line, fill_rgb):
    """Count leading and trailing background pixels in a line."""
    n = len(line)
    lead = 0
    while lead < n and classify(line[lead], fill_rgb) == "background":
        lead += 1
    trail = 0
    while trail < n and classify(line[n - 1 - trail], fill_rgb) == "background":
        trail += 1
    return lead, trail


def analyze_line(line, axis_lo, axis_hi):
    fill_rgb = dominant_fill(line)
    segments = run_length(line, fill_rgb)
    lead_bg, trail_bg = lead_trail_bg(line, fill_rgb)
    return {
        "length": len(line),
        "fill_color": (
            "#%02X%02X%02X" % fill_rgb if fill_rgb else None
        ),
        "segments": [
            {
                "class": cls,
                "color": color,
                "start": s,
                "end": e,
                "width_device": e - s + 1,
                "width_logical": dev_to_log(e - s + 1),
            }
            for cls, color, s, e in segments
        ],
        "margins": margin_report(lead_bg, trail_bg, axis_lo, axis_hi),
    }


def whole_image_summary(arr):
    import numpy as np

    h, w = arr.shape[:2]
    opaque = arr[:, :, 3] > ALPHA_BG_MAX
    if not opaque.any():
        return {"empty": True, "width": w, "height": h}
    rows = np.where(opaque.any(axis=1))[0]
    cols = np.where(opaque.any(axis=0))[0]
    top, bottom = int(rows[0]), int(rows[-1])
    left, right = int(cols[0]), int(cols[-1])
    left_m, right_m = left, w - 1 - right
    top_m, bottom_m = top, h - 1 - bottom
    return {
        "width": w,
        "height": h,
        "width_logical": dev_to_log(w),
        "height_logical": dev_to_log(h),
        "content_bbox": {
            "left": left,
            "top": top,
            "right": right,
            "bottom": bottom,
            "width_device": right - left + 1,
            "height_device": bottom - top + 1,
        },
        "margins": {
            **margin_report(left_m, right_m, "left", "right"),
            **{
                "top": margin_report(top_m, bottom_m, "top", "bottom")["top"],
                "bottom": margin_report(top_m, bottom_m, "top", "bottom")[
                    "bottom"
                ],
                "vertical_symmetric": top_m == bottom_m,
                "vertical_skew_device": top_m - bottom_m,
            },
        },
    }


def resolve_index(n, logical, limit, name):
    idx = int(round(n * PPP)) if logical else int(n)
    if idx < 0 or idx >= limit:
        sys.exit(
            f"error: --{name} {n} resolves to device index {idx}, "
            f"outside 0..{limit - 1}"
        )
    return idx


# --- Pretty-printers ---------------------------------------------------------
def print_summary(s):
    if s.get("empty"):
        print(f"{s['width']}x{s['height']} device — fully transparent (no content)")
        return
    print(
        f"Image: {s['width']}x{s['height']} device "
        f"({s['width_logical']:.1f}x{s['height_logical']:.1f} logical, PPP={PPP})"
    )
    b = s["content_bbox"]
    print(
        f"Content bbox (device): x {b['left']}..{b['right']}  "
        f"y {b['top']}..{b['bottom']}  "
        f"({b['width_device']}x{b['height_device']})"
    )
    m = s["margins"]
    print("Harness margins (logical; 'beyond' = past the 8.0 harness margin):")
    for side in ("left", "right", "top", "bottom"):
        d = m[side]
        print(
            f"  {side:<6} {d['logical']:>6.1f} log "
            f"({d['device']}dev, beyond harness {d['beyond_harness']:+.1f})"
        )
    hsym = "symmetric ✓" if m["symmetric"] else f"SKEWED {m['skew_device']:+d}dev ✗"
    vsym = (
        "symmetric ✓"
        if m["vertical_symmetric"]
        else f"SKEWED {m['vertical_skew_device']:+d}dev ✗"
    )
    print(f"  left/right:  {hsym}")
    print(f"  top/bottom:  {vsym}")


def print_line(idx, logical_in, info, axis_lo, axis_hi, kind):
    log_at = dev_to_log(idx)
    fill = info["fill_color"] or "none"
    print(
        f"{kind} {idx} device ({log_at:.1f} logical) — "
        f"length {info['length']}dev, fill color {fill}"
    )
    print("  segments:")
    for seg in info["segments"]:
        print(
            f"    {seg['class']:<10} {seg['color']:<9} "
            f"x[{seg['start']}..{seg['end']}]  w={seg['width_device']}dev"
            f" / {seg['width_logical']:.1f}log"
        )
    m = info["margins"]
    print(f"  {axis_lo}/{axis_hi} background margins (logical):")
    for side in (axis_lo, axis_hi):
        d = m[side]
        print(
            f"    {side:<6} {d['logical']:>6.1f} log "
            f"({d['device']}dev, beyond harness {d['beyond_harness']:+.1f})"
        )
    sym = "symmetric ✓" if m["symmetric"] else f"SKEWED {m['skew_device']:+d}dev ✗"
    print(f"    {axis_lo}/{axis_hi}: {sym}")


def main():
    p = argparse.ArgumentParser(
        description="Measure egui_kittest snapshot PNGs (PPP=2, 8px harness "
        "margin, border #C8C8C8 baked in).",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=__doc__,
    )
    p.add_argument("png", help="path to the snapshot PNG")
    p.add_argument("--row", type=float, help="horizontal scan at this row")
    p.add_argument("--col", type=float, help="vertical scan at this column")
    p.add_argument(
        "--logical",
        action="store_true",
        help="interpret --row/--col as logical px (multiplied by PPP)",
    )
    p.add_argument("--json", action="store_true", help="machine-readable output")
    args = p.parse_args()

    np, Image = _load_deps()
    img = Image.open(args.png).convert("RGBA")
    arr = np.asarray(img)
    h, w = arr.shape[:2]

    out = {}
    did_line = False
    if args.row is not None:
        y = resolve_index(args.row, args.logical, h, "row")
        line = [tuple(px) for px in arr[y]]
        info = analyze_line(line, "left", "right")
        out["row"] = {"index": y, **info}
        if not args.json:
            print_line(y, args.logical, info, "left", "right", "Row")
        did_line = True
    if args.col is not None:
        x = resolve_index(args.col, args.logical, w, "col")
        line = [tuple(px) for px in arr[:, x]]
        info = analyze_line(line, "top", "bottom")
        out["col"] = {"index": x, **info}
        if not args.json:
            if did_line:
                print()
            print_line(x, args.logical, info, "top", "bottom", "Col")
        did_line = True

    if not did_line:
        summary = whole_image_summary(arr)
        out["summary"] = summary
        if not args.json:
            print_summary(summary)

    if args.json:
        json.dump(out, sys.stdout, indent=2)
        print()


if __name__ == "__main__":
    main()
