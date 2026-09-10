"""Held/front-lit shield reference from Frozen OpenGL r570.

Fixed 1280x720, GUI scale3, foil speed0/strength0.5, ordinary resources.
No candidate fitting, alignment, image correction or tolerance adjustment.
"""
from PIL import Image, ImageChops, ImageStat

PROBES = [["gui_plate", 394, 682, [[113, 84, 57], [113, 84, 57], [99, 72, 49], [113, 84, 57], [113, 84, 57], [99, 72, 49], [113, 84, 57], [113, 84, 57], [99, 72, 49]]], ["gui_rim", 387, 675, [[156, 153, 171], [128, 98, 71], [128, 98, 71], [156, 153, 171], [128, 98, 71], [88, 65, 45], [156, 153, 171], [128, 98, 70], [88, 65, 45]]], ["gui_bottom", 396, 704, [[99, 72, 50], [99, 72, 50], [88, 65, 46], [99, 72, 50], [99, 72, 50], [88, 65, 46], [99, 72, 50], [99, 72, 50], [99, 72, 50]]], ["held_top_edge", 800, 475, [[146, 146, 162], [146, 146, 162], [146, 146, 162], [146, 146, 162], [146, 146, 162], [146, 146, 162], [146, 146, 162], [146, 146, 162], [146, 146, 162]]], ["held_rim", 740, 520, [[59, 57, 66], [59, 57, 66], [59, 57, 66], [59, 57, 66], [59, 57, 66], [59, 57, 66], [59, 57, 66], [59, 57, 66], [59, 57, 66]]], ["held_plate", 800, 600, [[59, 43, 27], [59, 43, 27], [59, 43, 27], [59, 43, 27], [59, 43, 27], [50, 35, 21], [59, 43, 27], [59, 43, 27], [50, 35, 21]]], ["held_right", 1150, 610, [[49, 35, 19], [49, 35, 19], [49, 35, 19], [49, 35, 19], [49, 35, 19], [49, 35, 19], [49, 35, 19], [49, 35, 19], [49, 35, 19]]]]
TOLERANCE = 2
REGIONS = {"gui": (370,662,424,716), "held": (680,440,1280,655)}

# Unenchanted control from Frozen OpenGL r576, after actual resource reload.
# Same coordinates/camera and tolerance as the enchanted r570 reference.
BASE_PROBES = [
    ["gui_plate",394,682,[[108,83,44],[108,83,44],[94,71,36]]*3],
    ["gui_rim",387,675,[[151,152,157],[123,97,57],[123,97,57],
        [151,152,157],[123,97,57],[83,64,32],[151,152,157],[123,97,57],[83,64,32]]],
    ["gui_bottom",396,704,[[94,71,36],[94,71,36],[83,64,32]]*2+[[94,71,36]]*3],
    ["held_top_edge",800,475,[[143,145,153]]*9],
    ["held_rim",740,520,[[57,57,61]]*9],
    ["held_plate",800,600,[[57,43,22]]*5+[[48,35,16],[57,43,22],[57,43,22],[48,35,16]]],
    ["held_right",1150,610,[[48,35,16]]*9],
]

# Frozen r578 only: yellow base, red cross, blue border, no foil. The held
# back-face probes were independently checked and equal the plain base reference.
PATTERN_PROBES = [
    ["gui_blue_border",388,674,[[48,54,135],[48,54,135],[154,40,33],
        [48,54,135],[48,54,135],[154,40,33],[48,54,135],[223,190,53],[223,190,53]]],
    ["gui_upper_yellow",397,679,[[223,190,53]]*7+[[154,40,33]]*2],
    ["gui_red_cross",395,688,[[154,40,33],[154,40,33],[223,190,53]]*3],
    ["gui_lower_yellow",392,699,[[223,190,53]]*9],
    ["gui_bottom_border",400,704,[[218,186,52],[50,56,141],[48,54,135],
        [218,186,52],[50,56,141],[50,56,141],[50,56,141],[50,56,141],[50,56,141]]],
] + BASE_PROBES[3:]

# Frozen r581, exact same authored patterns, ENTITY foil speed0/strength.5.
# All samples are Frozen-derived; no candidate fitting or changed tolerance.
PATTERN_FOIL_PROBES = [
    ["gui_blue_border",388,674,[[57,56,158],[57,56,158],[163,42,56],
        [57,56,158],[57,56,158],[163,42,55],[57,56,158],[232,192,75],[231,192,75]]],
    ["gui_upper_yellow",397,679,[[231,192,73],[231,192,73],[231,192,74],
        [231,192,73],[231,192,73],[231,192,74],[231,192,73],[162,42,53],[162,42,54]]],
    ["gui_red_cross",395,688,[[161,42,52],[161,42,52],[230,192,72]]*3],
    ["gui_lower_yellow",392,699,[[230,192,71]]*9],
    ["gui_bottom_border",400,704,[[225,187,70],[57,58,159],[55,56,154],
        [225,187,70],[57,58,159],[57,58,160],[57,57,159],[57,58,159],[57,58,160]]],
    ["held_top_edge",800,475,[[153,147,179]]*9],
    ["held_rim",740,520,[[68,60,89]]*9],
    ["held_plate",800,600,[[68,46,51]]*5+[[59,38,45],[68,46,51],[68,46,51],[59,38,45]]],
    ["held_right",1150,610,[[63,39,55]]*9],
]

def compare_images(baseline, current, *, foil=True):
    return _compare_images(baseline,current,PROBES if foil else BASE_PROBES,
                           "shield-foil-pixels-v1" if foil else "shield-base-pixels-v1")

def compare_pattern_images(baseline,current, *, foil=False):
    return _compare_images(baseline,current,PATTERN_FOIL_PROBES if foil else PATTERN_PROBES,
                           "shield-pattern-foil-pixels-v1" if foil else "shield-pattern-pixels-v1")

def _compare_images(baseline, current, probes, schema):
    if baseline.size != (1280,720) or current.size != baseline.size:
        raise ValueError("shield reference requires equivalent 1280x720 images")
    baseline, current = baseline.convert("RGB"), current.convert("RGB")
    rows = []
    for name,x,y,golden in probes:
        a = list(baseline.crop((x-1,y-1,x+2,y+2)).getdata())
        b = list(current.crop((x-1,y-1,x+2,y+2)).getdata())
        def error(p,q):
            return max(abs(u-v) for lhs,rhs in zip(p,q) for u,v in zip(lhs,rhs))
        anchor, pair = error(a,golden), error(a,b)
        rows.append(dict(name=name, baseline_max_channel_error=anchor,
                         pair_max_channel_error=pair, passed=anchor<=TOLERANCE and pair<=TOLERANCE))
    regions = []
    for name,box in REGIONS.items():
        stats = ImageStat.Stat(ImageChops.difference(baseline.crop(box),current.crop(box)))
        regions.append(dict(name=name, box=box, mean_rgb_abs=stats.mean,
                            rms_rgb_abs=stats.rms, passed=max(stats.mean)<=TOLERANCE))
    return dict(schema=schema,
                passed=all(row["passed"] for row in rows+regions), probes=rows, regions=regions)

def compare_paths(baseline,current, *, foil=True):
    with Image.open(baseline) as a, Image.open(current) as b:
        return compare_images(a,b,foil=foil)

def compare_pattern_paths(baseline,current, *, foil=False):
    with Image.open(baseline) as a, Image.open(current) as b:
        return compare_pattern_images(a,b,foil=foil)
