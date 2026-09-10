"""Frozen-only r347 held-trident reference; selected before a valid Current image.

Ordinary vanilla resources, speed0/strength0.5, 1280x720, observed GUI scale3.
No alignment, color correction, mask fitting, or candidate-selected probes.
"""
from pathlib import Path
from PIL import Image

PROBES = [["left_tip",1045,380,[[139,159,169],[139,159,169],[138,158,168],[139,159,169],[139,159,169],[138,158,168],[139,159,169],[139,159,168],[138,158,167]]],["middle_tip",1100,340,[[170,172,179],[170,172,179],[165,189,198],[170,172,179],[170,172,179],[165,189,198],[170,172,179],[170,172,179],[165,189,198]]],["middle_front",1120,380,[[170,190,210],[170,190,210],[170,190,210],[170,190,210],[170,190,211],[170,190,211],[170,190,211],[170,190,211],[170,190,211]]],["right_tip",1180,310,[[167,189,204],[167,189,204],[168,189,204],[167,189,203],[167,189,203],[167,189,204],[167,189,203],[167,189,203],[167,189,203]]],["right_front",1200,375,[[164,188,194],[164,188,194],[164,188,194],[137,157,165],[164,188,195],[164,188,195],[137,157,165],[164,188,195],[164,188,195]]],["crossbar_left",1090,463,[[79,119,139],[79,120,140],[79,120,140],[79,120,139],[79,120,140],[79,120,140],[79,120,140],[79,120,140],[79,120,140]]],["crossbar_middle",1135,465,[[125,141,168],[125,141,168],[124,141,168],[81,120,143],[80,120,143],[80,120,143],[81,120,143],[80,120,143],[80,120,143]]],["crossbar_right",1200,475,[[56,91,100],[68,114,119],[67,114,119],[56,91,99],[67,114,119],[67,114,119],[56,91,99],[55,91,99],[67,114,118]]],["shaft_left",1165,530,[[54,91,95],[54,91,95],[54,91,95],[54,91,95],[54,91,95],[54,91,95],[54,91,95],[54,90,95],[54,90,95]]],["shaft_front",1230,620,[[86,146,144],[86,146,144],[86,146,143],[86,146,144],[86,146,144],[86,146,143],[86,146,144],[86,146,144],[86,146,143]]],["gui_tip",413,672,[[221,215,234],[221,215,233],[255,254,255],[255,254,255],[255,254,255],[155,150,167],[255,254,255],[255,254,255],[155,150,167]]],["gui_shaft",385,699,[[58,96,112],[58,96,112],[58,96,112],[100,158,174],[100,158,174],[100,158,173],[100,158,174],[100,158,174],[100,158,174]]]]
TOLERANCE = 2

def compare_images(baseline, current):
    if baseline.size != (1280,720) or current.size != baseline.size:
        raise ValueError("model foil reference requires equivalent 1280x720 captures")
    baseline, current = baseline.convert("RGB"), current.convert("RGB")
    rows = []
    for name,x,y,golden in PROBES:
        a = list(baseline.crop((x-1,y-1,x+2,y+2)).getdata())
        b = list(current.crop((x-1,y-1,x+2,y+2)).getdata())
        error = lambda p,q: max(abs(u-v) for lhs,rhs in zip(p,q) for u,v in zip(lhs,rhs))
        anchor, pair = error(a,golden), error(a,b)
        rows.append(dict(name=name, baseline_max_channel_error=anchor,
                         pair_max_channel_error=pair, passed=anchor<=TOLERANCE and pair<=TOLERANCE))
    return dict(schema="model-foil-pixels-v1", passed=all(row["passed"] for row in rows), probes=rows)

def compare_paths(baseline, current):
    with Image.open(Path(baseline)) as a, Image.open(Path(current)) as b:
        return compare_images(a,b)
