"""Strict static held-foil witnesses, generated fixture, selected slot 2.

Golden RGB values are interior 3x3 patches from Frozen Java OpenGL r315,
1280x720, static camera, glint speed 0 and strength 0.5. These complement
the GUI and whole-frame gates; neither matching missing glint nor a tiny
whole-frame error can admit this fixture. No alignment or color correction.
"""
from PIL import Image
import math

PROBES = (
    ("red", 1000, 500, (176, 32, 59)),
    ("green", 1200, 450, (27, 160, 82)),
    ("blue", 1050, 660, (31, 56, 182)),
    ("yellow", 1250, 660, (173, 150, 31)),
)

# Mixed fixture v2: centers fixed before inspecting Frozen r333; golden colors
# from Frozen Java OpenGL, clamp sampler, speed0, strength0.5, GUI scale2.
MIXED_GROUND_PROBES = (
    ("red",433,348,(144,29,69)),
    ("green",550,348,(31,127,87)),
    ("blue",433,461,(34,48,163)),
    ("yellow",550,461,(143,120,49)),
)

# Frozen OpenGL r319: authored south-face model (not generated geometry).
# Static pattern covers only a small UV interval; this is deliberately NOT
# evidence for repeat-boundary/minification behavior.
PATTERN_PROBES = (
    ("red_left_top",760,420,(144,38,49)),
    ("red_middle_top",900,420,(144,39,49)),
    ("red_right_top",1000,420,(145,39,50)),
    ("red_left_bottom",760,600,(143,40,49)),
    ("red_middle_bottom",900,600,(144,41,50)),
    ("red_right_bottom",1000,600,(144,41,50)),
    ("green_left_top",1120,420,(32,136,67)),
    ("green_right_top",1220,420,(33,136,67)),
    ("green_left_bottom",1120,600,(33,138,66)),
    ("green_right_bottom",1220,600,(33,137,67)),
)

# Predetermined grid, Frozen OpenGL r321. Wide atlas UVs cross repeat
# boundaries; these are not relocated to avoid unfavorable candidate pixels.
WIDE_PROBES = (
    ("r0c0",720,410,(155,44,51)), ("r0c1",820,410,(165,49,57)),
    ("r0c2",920,410,(140,40,64)), ("r0c3",1020,410,(158,36,53)),
    ("r0c4",1120,410,(30,137,76)), ("r0c5",1220,410,(28,142,83)),
    ("r1c0",720,480,(159,26,52)), ("r1c1",820,480,(142,26,58)),
    ("r1c2",920,480,(143,30,61)), ("r1c3",1020,480,(142,28,47)),
    ("r1c4",1120,480,(28,123,66)), ("r1c5",1220,480,(35,124,70)),
    ("r2c0",720,550,(147,35,64)), ("r2c1",820,550,(142,39,47)),
    ("r2c2",920,550,(148,44,53)), ("r2c3",1020,550,(145,37,50)),
    ("r2c4",1120,550,(31,129,65)), ("r2c5",1220,550,(38,133,71)),
    ("r3c0",720,620,(138,48,61)), ("r3c1",820,620,(144,54,48)),
    ("r3c2",920,620,(151,24,54)), ("r3c3",1020,620,(160,26,60)),
    ("r3c4",1120,620,(34,141,66)), ("r3c5",1220,620,(42,146,72)),
)


def wide_source_evidence(source):
    """Prove that observed source UVs exercise repeat in both glint axes."""
    from gui_foil_reference import source_at_pixel, standard_uv
    # Reuse the independent source-shape validation, not image-derived UVs.
    source_at_pixel(source,4,4,2)
    uvs = source["atlasUvs"]
    transformed = [standard_uv(uvs[i:i+2]) for i in range(0,8,2)]
    spans = [(min(uv[c] for uv in transformed),max(uv[c] for uv in transformed)) for c in (0,1)]
    passed = all(hi-lo >= 1 and math.floor(lo) < math.floor(hi) for lo,hi in spans)
    return dict(passed=passed, transformed_uvs=transformed, axis_ranges=spans)


def compare_images(frozen, current):
    return _compare_images(frozen, current, PROBES, "held-item-standard-foil-v1")


def compare_pattern_images(frozen, current):
    return _compare_images(frozen, current, PATTERN_PROBES, "held-item-static-pattern-foil-v1")


def compare_wide_images(frozen, current):
    return _compare_images(frozen, current, WIDE_PROBES, "held-item-wide-pattern-foil-v1")


def compare_moving_images(frozen, current, *, wide=False):
    """Capture-local paired pixels; clocks are validated separately.

    Static golden colors cannot substitute for observed animated pixels.
    Preserve every sampled pixel for the second-phase change check.
    """
    if frozen.size != (1280,720) or current.size != frozen.size:
        raise ValueError("moving held foil requires the shared 1280x720 fixture")
    frozen,current = frozen.convert("RGB"),current.convert("RGB")
    rows=[]
    for name,x,y,_ in (WIDE_PROBES if wide else PATTERN_PROBES):
        points=[(px,py) for py in range(y-1,y+2) for px in range(x-1,x+2)]
        a,b=[[list(im.getpixel(p)) for p in points] for im in (frozen,current)]
        error=max(abs(v[c]-w[c]) for v,w in zip(a,b) for c in range(3))
        # Broad authored-face presence checks, not color parity tolerances.
        # Keep background animation from masquerading as a missing held item.
        def present(rgb):
            r,g,b=rgb
            red_face = x < 1058 if wide else name.startswith("red")
            return (r>=130 and r>g+40 and r>b+40) if red_face else (g>=110 and g>r+40 and g>b+20)
        visible=all(present(p) for p in a+b)
        rows.append(dict(name=name,center=[x,y],baseline_pixels=a,current_pixels=b,
                         pair_max_channel_error=error,faces_visible=visible,passed=visible and error<=2))
    return dict(schema="held-item-sampler-wide-v1" if wide else "held-item-moving-foil-v1",probes=rows,passed=all(p["passed"] for p in rows))


def moving_change_evidence(before,after):
    if any(not isinstance(r,dict) or r.get("passed") is not True
           or r.get("schema")!="held-item-moving-foil-v1" for r in (before,after)):
        raise ValueError("moving hand comparison requires two passing paired frames")
    if len(before.get("probes",[]))!=10 or len(after.get("probes",[]))!=10:
        raise ValueError("moving hand comparison requires every fixed probe")
    changed={"red":False,"green":False}
    rows=[]
    for old,new in zip(before["probes"],after["probes"]):
        if old["name"]!=new["name"] or old["center"]!=new["center"]:
            raise ValueError("moving hand probe identity changed")
        deltas=[]
        for key in ("baseline_pixels","current_pixels"):
            if len(old[key])!=9 or len(new[key])!=9: raise ValueError("incomplete hand pixel patch")
            deltas.append([b[c]-a[c] for a,b in zip(old[key],new[key]) for c in range(3)])
        error=max(abs(a-b) for a,b in zip(*deltas))
        visible=any(abs(a)>4 and abs(b)>4 and a*b>0 for a,b in zip(*deltas))
        changed["red" if old["name"].startswith("red") else "green"] |= visible
        rows.append(dict(name=old["name"],delta_max_channel_error=error,visible_change=visible,passed=error<=4))
    return dict(passed=all(changed.values()) and all(p["passed"] for p in rows),faces_changed=changed,probes=rows)


def _compare_images(frozen, current, probes, schema):
    if frozen.size != (1280, 720) or current.size != frozen.size:
        raise ValueError("held foil witnesses require the shared 1280x720 fixture")
    frozen, current = frozen.convert("RGB"), current.convert("RGB")
    rows = []
    for name, x, y, golden in probes:
        points = [(px, py) for py in range(y-1, y+2) for px in range(x-1, x+2)]
        baseline = [frozen.getpixel(p) for p in points]
        observed = [current.getpixel(p) for p in points]
        baseline_error = max(abs(a[c]-golden[c]) for a in baseline for c in range(3))
        pair_error = max(abs(a[c]-b[c]) for a,b in zip(baseline, observed) for c in range(3))
        rows.append(dict(name=name, center=[x,y], golden=golden,
                         baseline_max_channel_error=baseline_error,
                         pair_max_channel_error=pair_error,
                         passed=baseline_error <= 2 and pair_error <= 2))
    return dict(schema=schema, probes=rows,
                passed=all(row["passed"] for row in rows))


def compare_paths(frozen, current):
    with Image.open(frozen) as a, Image.open(current) as b:
        return compare_images(a, b)


def compare_pattern_paths(frozen, current):
    with Image.open(frozen) as a, Image.open(current) as b:
        return compare_pattern_images(a, b)


def compare_wide_paths(frozen, current):
    with Image.open(frozen) as a, Image.open(current) as b:
        return compare_wide_images(a, b)


def compare_moving_paths(frozen, current):
    with Image.open(frozen) as a, Image.open(current) as b:
        return compare_moving_images(a, b)


def compare_sampler_wide_paths(frozen, current):
    with Image.open(frozen) as a, Image.open(current) as b:
        return compare_moving_images(a, b, wide=True)


def compare_mixed_ground_images(frozen, current):
    return _compare_images(frozen, current, MIXED_GROUND_PROBES, "mixed-ground-standard-foil-v1")


def compare_mixed_ground_paths(frozen, current):
    with Image.open(frozen) as a, Image.open(current) as b:
        return compare_mixed_ground_images(a, b)
