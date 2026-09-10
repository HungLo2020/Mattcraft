"""Compare bounded encoded-vertex observations; never a visual acceptance gate."""
import json
import math
import re
import struct
from pathlib import Path


def f32(value):
    return struct.unpack("<f",struct.pack("<f",float(value)))[0]


def identity(values):
    return tuple(struct.pack("<f",float(value)) for value in values)


def compare(current_log,frozen_log):
    base={}; foil=[]; reference=[]; atlas=None
    with Path(current_log).open() as stream:
        for line in stream:
            if "[gui.mesh.vertex-trace] " not in line: continue
            record=json.loads(line.split("[gui.mesh.vertex-trace] ",1)[1])
            if record["bounds"] != [292,341,308,357] or record["extent"] != [34,34]:
                raise ValueError("trace does not describe the scale-two leaf fixture")
            for vertex in record["vertices"]:
                if len(vertex)!=8 or not all(math.isfinite(v) for v in vertex):
                    raise ValueError("invalid prepared vertex")
                key=identity(vertex[:3]+vertex[6:])
                encoded=list(map(f32,vertex[3:6]))
                if record["material"]=="Glint": foil.append((key,encoded))
                else:
                    if key in base and base[key]!=encoded: raise ValueError("ambiguous prepared source")
                    base[key]=encoded
    with Path(frozen_log).open() as stream:
        for line in stream:
            if atlas is None and "gui.item.raster-source" in line and "sprite=minecraft:block/oak_leaves_bushy" in line:
                match=re.search(r"slot=(\d+),(\d+),(\d+) atlas=(\d+)x(\d+)",line)
                if not match: raise ValueError("missing reference atlas placement")
                atlas=list(map(int,match.groups()))
            if "gui.leaf.vertex " not in line: continue
            match=re.search(r"item=292,341 face=(\w+) vertex=(\d+) source=([^ ]+) encoded=([^ ]+) uv=([^ ]+)",line)
            if not match: raise ValueError("unexpected reference item trace")
            source=list(map(f32,match[3].split(',')))
            encoded=list(map(f32,match[4].split(',')))
            uv=list(map(f32,match[5].split(',')))
            reference.append((match[1],int(match[2]),identity(source+uv),encoded))
    if len(reference)!=16 or atlas is None or atlas[2:] != [32,512,512] or not foil:
        raise ValueError("incomplete actual GUI vertex observations")
    if {(face,index) for face,index,_,_ in reference}!={(face,i) for face in ('west','east','north','south') for i in range(4)}:
        raise ValueError("missing opposing bush faces")
    # Semantic atlas placement minus the native guard, never a fitted offset.
    offset=[atlas[0]-1,atlas[1]-1,0]
    rows=[]
    for face,index,key,encoded in reference:
        if key not in base: raise ValueError("reference source position/UV absent in native trace")
        native=[f32(v+d) for v,d in zip(base[key],offset)]
        rows.append(dict(face=face,vertex=index,reference=encoded,native=native,
                         delta=[a-b for a,b in zip(encoded,native)]))
    if any(key not in base or value!=base[key] for key,value in foil):
        raise ValueError("native foil and base positions differ")
    return dict(diagnostic_only=True,matched_reference_vertices=16,
                native_foil_vertices_matching_base=len(foil),reference_atlas=atlas,
                explicit_offset=offset,max_abs_delta=[max(abs(row['delta'][i]) for row in rows) for i in range(3)],rows=rows)


if __name__=="__main__":
    import argparse
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("current_log");parser.add_argument("frozen_log")
    args=parser.parse_args()
    print(json.dumps(compare(args.current_log,args.frozen_log),indent=2))
