"""Held generated-item face witnesses for the flat-item-foil-cutout fixture.

Fixed interior samples from the authoritative Frozen OpenGL r299 capture,
under the shared 1280x720 static camera fixture. Four extrusion faces and four
front-face controls prevent a whole-frame average from hiding missing edges.
No image alignment, color normalization, or changed parity tolerance.
"""
from PIL import Image

PROBES = (
    ("outer_green_edge", 1255, 360, (33, 196, 91)),
    ("hole_red_edge", 1070, 570, (222, 35, 63)),
    ("hole_blue_edge", 1140, 686, (39, 68, 228)),
    ("hole_yellow_edge", 1200, 697, (228, 195, 30)),
    ("front_red", 1010, 530, (175, 28, 50)),
    ("front_green", 1190, 450, (26, 156, 73)),
    ("front_blue", 1040, 660, (30, 52, 173)),
    ("front_yellow", 1250, 660, (172, 146, 22)),
)


def compare_images(frozen, current):
    if frozen.size != (1280, 720) or current.size != frozen.size:
        raise ValueError("held cutout witnesses require the shared 1280x720 fixture")
    frozen, current = frozen.convert("RGB"), current.convert("RGB")
    rows = []
    for name, x, y, golden in PROBES:
        points = [(px, py) for py in range(y-1, y+2) for px in range(x-1, x+2)]
        baseline = [frozen.getpixel(p) for p in points]
        observed = [current.getpixel(p) for p in points]
        baseline_error = max(abs(a[c]-golden[c]) for a in baseline for c in range(3))
        pair_error = max(abs(a[c]-b[c]) for a,b in zip(baseline, observed) for c in range(3))
        rows.append(dict(name=name, center=[x,y], golden=golden,
                         baseline_max_channel_error=baseline_error,
                         pair_max_channel_error=pair_error,
                         passed=baseline_error <= 2 and pair_error <= 2))
    return dict(schema="held-item-cutout-faces-v1", probes=rows,
                passed=all(row["passed"] for row in rows))


def compare_paths(frozen, current):
    with Image.open(frozen) as a, Image.open(current) as b:
        return compare_images(a, b)
