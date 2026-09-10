"""Strict dropped-item probes from Frozen Java OpenGL r341.

1280x720 canonical fixture, count1, frozen simulation, bob=float(pi),
clamped foil pack, selected slot2, GUI scale2. No alignment/color correction.
Source: dropped-item-foil-reference-r341/01_initial.png; selected before Rust.
"""
from PIL import Image

PROBES = [{"name":"diamond_red","center":[395,230],"pixels":[[183,35,80],[183,35,80],[183,35,80],[183,35,80],[183,35,80],[183,35,80],[183,35,80],[183,35,80],[183,35,80]]},{"name":"diamond_green","center":[416,230],"pixels":[[37,161,102],[37,161,102],[37,161,102],[37,161,102],[37,161,102],[37,161,102],[37,161,102],[37,161,102],[37,161,102]]},{"name":"diamond_blue","center":[405,350],"pixels":[[40,59,201],[40,59,201],[40,59,201],[40,59,201],[40,59,201],[40,59,201],[40,59,201],[40,59,201],[40,59,201]]},{"name":"diamond_yellow","center":[430,350],"pixels":[[180,152,53],[180,152,53],[180,152,53],[180,152,53],[180,152,53],[180,152,53],[180,152,53],[180,152,53],[180,152,53]]},{"name":"stone_top","center":[558,348],"pixels":[[136,133,156],[136,133,156],[136,133,156],[125,122,145],[125,122,145],[125,122,145],[152,149,172],[113,111,134],[113,111,134]]},{"name":"stone_front","center":[562,378],"pixels":[[70,67,90],[83,80,103],[83,80,103],[70,67,90],[83,80,103],[70,67,90],[70,67,90],[70,67,90],[70,67,90]]}]


# Five-copy stack reference: Frozen OpenGL r344, fixed before any Rust stack capture.
STACK_PROBES = [{"name":"diamond_red","center":[395,230],"pixels":[[183,35,80],[183,35,80],[183,35,80],[183,35,80],[183,35,80],[183,35,80],[183,35,80],[183,35,80],[183,35,80]]},{"name":"diamond_green","center":[416,230],"pixels":[[37,161,102],[37,161,102],[37,161,102],[37,161,102],[37,161,102],[37,161,102],[37,161,102],[37,161,102],[37,161,102]]},{"name":"diamond_blue","center":[405,350],"pixels":[[40,59,201],[40,59,201],[40,59,201],[40,59,201],[40,59,201],[40,59,201],[40,59,201],[40,59,201],[40,59,201]]},{"name":"diamond_yellow","center":[430,350],"pixels":[[180,152,53],[180,152,53],[180,152,53],[180,152,53],[180,152,53],[180,152,53],[180,152,53],[180,152,53],[180,152,53]]},{"name":"diamond_stack_tip","center":[375,122],"pixels":[[182,35,80],[182,35,80],[182,35,80],[182,35,80],[182,35,80],[182,35,80],[182,35,80],[182,35,80],[182,35,80]]},{"name":"diamond_stack_bottom","center":[403,441],"pixels":[[40,59,201],[40,59,201],[40,59,201],[40,59,201],[40,59,201],[40,59,201],[40,59,201],[40,59,201],[40,59,201]]},{"name":"stone_left_front","center":[520,410],"pixels":[[83,80,103],[83,80,103],[75,72,95],[83,80,103],[83,80,103],[75,72,95],[83,80,103],[83,80,103],[75,72,95]]},{"name":"stone_middle_front","center":[552,385],"pixels":[[75,72,95],[75,72,95],[75,72,95],[75,72,95],[75,72,95],[75,72,95],[75,72,95],[75,72,95],[75,72,95]]},{"name":"stone_right_front","center":[604,374],"pixels":[[83,80,103],[83,80,103],[83,80,103],[83,80,103],[83,80,103],[75,72,95],[83,80,103],[83,80,103],[75,72,95]]},{"name":"stone_left_top","center":[515,371],"pixels":[[152,149,172],[152,149,172],[152,149,172],[136,133,156],[136,133,156],[136,133,156],[136,133,156],[136,133,156],[136,133,156]]},{"name":"stone_middle_top","center":[546,349],"pixels":[[152,149,172],[136,133,156],[136,133,156],[136,133,156],[136,133,156],[136,133,156],[152,149,172],[136,133,156],[125,122,145]]},{"name":"stone_right_top","center":[596,338],"pixels":[[152,149,172],[152,149,172],[152,149,172],[113,111,134],[136,133,156],[136,133,156],[113,111,134],[113,111,134],[136,133,156]]}]


def compare_images(frozen, current, count=1):
    if type(count) is not int or count not in (1, 64):
        raise ValueError("dropped stack count lacks an accepted pixel reference")
    if frozen.size != (1280, 720) or current.size != frozen.size:
        raise ValueError("dropped foil requires equivalent 1280x720 captures")
    frozen, current = frozen.convert("RGB"), current.convert("RGB")
    rows = []
    for probe in PROBES if count == 1 else STACK_PROBES:
        x, y = probe["center"]
        points = [(px, py) for py in range(y-1, y+2) for px in range(x-1, x+2)]
        baseline = [frozen.getpixel(p) for p in points]
        observed = [current.getpixel(p) for p in points]
        reference_error = max(abs(a[c]-b[c]) for a,b in zip(baseline,probe["pixels"]) for c in range(3))
        pair_error = max(abs(a[c]-b[c]) for a,b in zip(baseline,observed) for c in range(3))
        rows.append(dict(name=probe["name"], center=probe["center"],
                         baseline_max_channel_error=reference_error,
                         pair_max_channel_error=pair_error,
                         passed=reference_error <= 2 and pair_error <= 2))
    return dict(schema="dropped-item-foil-pixels-v1", stack_count=count,
                passed=all(r["passed"] for r in rows), probes=rows)


def compare_paths(frozen, current, count=1):
    with Image.open(frozen) as a, Image.open(current) as b:
        return compare_images(a, b, count)
