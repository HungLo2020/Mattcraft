"""Focused face-lighting checks for the shared standard-3d hotbar fixture.

Compare unmodified pixels at identical locations. Stone and white wool have
the same texture on each cube face, so their upper-face brightness also gives
an independent orientation check. This supplements, never replaces, the full
cross-repository fixture/equivalence and image gates.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path

from PIL import Image


def compare(frozen: Image.Image, current: Image.Image, scale: int, include_logs: bool = False) -> dict:
    items = (("stone", 0), ("white_wool", 6)) + ((("oak_log", 8),) if include_logs else ())
    return _compare_faces(frozen, current, scale, items, ("stone", "white_wool"))


def compare_animated_block(frozen: Image.Image, current: Image.Image, scale: int) -> dict:
    # Same cube placement and interior face boxes as the stone control. The
    # shared harness separately requires actual magma/animation upload receipts.
    result = _compare_faces(frozen, current, scale, (("magma_block", 0),), ())
    result["schema"] = "gui-animated-block-faces-v1"
    # Magma includes dark red texels with equal green and blue channels.
    # Reject achromatic/blank controls without inventing a G > B constraint
    # that the vanilla source texture itself does not satisfy.
    result["magma_visible"] = all(p[side][0] > max(p[side][1], p[side][2])
        for p in result["probes"] for side in ("frozen", "current"))
    result["passed"] &= result["magma_visible"]
    return result


def _compare_faces(frozen, current, scale, items, orientation_items):
    if frozen.size != current.size or scale not in (2, 3):
        raise ValueError("equal capture extents and GUI scale 2 or 3 required")
    width, height = frozen.size
    frozen, current = frozen.convert("RGB"), current.convert("RGB")
    probes = []
    # Interior model-face samples, away from silhouette and cube edges.
    for item, slot in items:
        center = width // 2 + (slot - 4) * 20 * scale
        for face, dx, dy in (("top", 0, -14), ("left", -3, -8), ("right", 3, -8)):
            x, y = center + dx * scale, height + dy * scale
            box = (x - 1, y - 1, x + 2, y + 2)
            values = []
            for source in (frozen, current):
                pixels = [source.getpixel((px, py)) for py in range(box[1], box[3])
                          for px in range(box[0], box[2])]
                values.append([sum(p[c] for p in pixels) / len(pixels) for c in range(3)])
            error = max(abs(a - b) for a, b in zip(*values))
            probes.append(dict(item=item, face=face, box=box, frozen=values[0],
                               current=values[1], max_mean_channel_error=error,
                               passed=error <= 2.0))
    orientation = []
    for item in orientation_items:
        faces = {p["face"]: p for p in probes if p["item"] == item}
        for backend in ("frozen", "current"):
            luminance = {f: sum(p[backend]) / 3 for f, p in faces.items()}
            orientation.append(dict(item=item, backend=backend, brightness=luminance,
                                    passed=luminance["top"] > max(luminance["left"], luminance["right"])))
    return dict(schema="gui-block-face-lighting-v1", scale=scale, probes=probes,
                orientation=orientation,
                passed=all(p["passed"] for p in probes + orientation))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--frozen", type=Path, required=True)
    parser.add_argument("--current", type=Path, required=True)
    parser.add_argument("--scale", type=int, choices=(2, 3), required=True)
    parser.add_argument("--include-logs", action="store_true", help="Use standard-3d-logs fixture")
    args = parser.parse_args()
    with Image.open(args.frozen) as frozen, Image.open(args.current) as current:
        report = compare(frozen, current, args.scale, args.include_logs)
    print(json.dumps(report, indent=2))
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
