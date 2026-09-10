"""Fixed oak-leaf terrain particle; static CPU inputs plus paired rendered evidence."""
import json
import math
import re
from collections.abc import Mapping
from pathlib import Path
from PIL import Image, ImageChops, ImageStat

NAME = "oak-leaves-cutout-terrain-v1"
GLASS_NAME = "blue-glass-translucent-terrain-v1"
# Full sky footprint, including a guard band. The CPU projection diagnostic
# uses (ndcY*.5+.5)*height: bottom-origin, unlike top-origin PNG pixels.
BOX = (560, 100, 720, 280)


def fixture_inputs(f):
    if not isinstance(f, Mapping):
        return False
    source = f.get("sourcePixels") or {}
    position, color = f.get("position"), f.get("color")
    glass = f.get("fixture") == GLASS_NAME
    identity = "blue_stained_glass" if glass else "oak_leaves"
    return (f.get("fixture") in (NAME, GLASS_NAME) and f.get("complete") is True
        and f.get("block") == "minecraft:" + identity
        and f.get("sprite") == "minecraft:block/" + identity
        and f.get("alphaTested") is (not glass) and f.get("staticSource") is True
        and (not glass or f.get("translucent") is True)
        and type(f.get("hidden")) is bool and f.get("size") == .35
        and f.get("localUv") == [.5, .25, .25, .5]
        and isinstance(position, list) and len(position) == 3
        and all(type(a) in (int, float) and math.isfinite(a) and abs(a-b) < 1e-6
            for a,b in zip(position, (147.64617596880336,102.0991813627766,529.7355156371002)))
        and isinstance(color, list) and len(color) == 4
        and all(type(v) in (int,float) and math.isfinite(v) and 0 <= v <= 1 for v in color)
        and color[3] == (0 if f["hidden"] else 1) and type(f.get("light")) is int
        and isinstance(source, Mapping) and source.get("width") == 16 and source.get("height") == 16
        and isinstance(source.get("rgbaFnv64"), str)
        and re.fullmatch(r"[0-9a-f]{16}", source["rgbaFnv64"]) is not None
        and (source_alpha(source, glass)))


def source_alpha(source, glass):
    keys = ("quarterTransparentPixels", "quarterOpaquePixels", "quarterTranslucentPixels") if glass else (
        "quarterTransparentPixels", "quarterOpaquePixels")
    if not all(type(source.get(k)) is int and 0 <= source[k] <= 16 for k in keys):
        return False
    return (sum(source[k] for k in keys)==16 and (source["quarterTranslucentPixels"]>0 if glass
        else source["quarterTransparentPixels"]>0 and source["quarterOpaquePixels"]>0))


def source_evidence(documents):
    receipts = [d.get("terrainParticleFixture") for d in documents]
    return {"passed":len(receipts) == 2 and all(fixture_inputs(r) for r in receipts)
            and receipts[0] == receipts[1], "receipts":receipts,
            "evidence_kind":"static-fixture-cpu-input-not-gpu-readback"}


def local(a, b, tolerance=6):
    if a.size != (1280,720) or b.size != a.size:
        return {"passed":False, "status":"cutout-particle-invalid-extent"}
    regions = []
    for y in range(BOX[1],BOX[3],20):
        for x in range(BOX[0],BOX[2],20):
            box = (x,y,x+20,y+20)
            means = ImageStat.Stat(ImageChops.difference(
                a.crop(box).convert("RGB"), b.crop(box).convert("RGB"))).mean
            regions.append({"box":box,"mean_rgb_abs":means,"passed":max(means)<=tolerance})
    passed = all(r["passed"] for r in regions)
    return {"passed":passed,"status":"complete" if passed else "cutout-particle-local-mismatch",
            "crop_box":BOX,"regions":regions,"tolerance":tolerance}


def producer(doc):
    name=(doc.get("terrainParticleFixture") or {}).get("fixture")
    if name not in (NAME, GLASS_NAME):
        return False
    sprite="minecraft:block/blue_stained_glass" if name==GLASS_NAME else "minecraft:block/oak_leaves"
    captures = doc.get("captures", [])
    frame = captures[0].get("renderedFrameIndex") if len(captures)==1 else None
    records = [r for r in doc.get("rustGalWorldTerrainParticles", [])
        if r.get("frameIndex")==frame and r.get("spriteId")==sprite]
    if type(frame) is not int or frame<=0 or len(records)!=1:
        return False
    r=records[0]
    bounds=[r.get("screenBounds",{}).get(k) for k in ("left","top","right","bottom")]
    if not all(type(v) in (int,float) and math.isfinite(v) for v in bounds):
        return False
    bounds=[bounds[0],720-bounds[3],bounds[2],720-bounds[1]]
    return (r.get("route")=="rust-vulkan-whole-frame" and r.get("projected") is True
        and r.get("viewport")=={"width":1280,"height":720}
        and r.get("materialMode")== (3 if name==GLASS_NAME else 2)
        and BOX[0] <= bounds[0] < bounds[2] <= BOX[2]
        and BOX[1] <= bounds[1] < bounds[3] <= BOX[3])


def load(pair, hidden, fixture_name=NAME):
    from graphics_harness import deterministic_capture_document, water_detail_fixture_matches
    if pair.get("fixture_equivalence",{}).get("status") != "passed":
        raise ValueError("incomparable cutout fixture")
    docs, images = [], []
    for side in ("baseline", "current"):
        doc = deterministic_capture_document(Path(pair[side+"_artifact"])) or {}
        f = doc.get("terrainParticleFixture") or {}
        if (not fixture_inputs(f) or f.get("fixture")!=fixture_name or f["hidden"] is not hidden
                or not water_detail_fixture_matches(doc) or doc["captures"][0].get("gameTime") != 6000):
            raise ValueError("missing or incorrect cutout fixture")
        if side=="current" and not producer(doc):
            raise ValueError("missing bounded cutout producer")
        docs.append(doc)
        with Image.open(pair[side+"_image"]) as image:
            images.append(image.convert("RGB"))
    if not source_evidence(docs)["passed"]:
        raise ValueError("unequal static cutout inputs")
    return docs, images


def effect(before, after):
    delta = ImageChops.difference(before.crop(BOX),after.crop(BOX))
    means = ImageStat.Stat(delta).mean
    changed = sum(max(p)>3 for p in delta.getdata())
    return {"passed":max(means)>.1 and changed>=32,"changed_pixels":changed,"mean_rgb_change":means}


def report(visual, scenario, reference=None, fixture_name=NAME):
    if scenario is None:
        return {"passed":True,"requested":False}
    result={"passed":False,"requested":True,"scenario":scenario,"fixture":fixture_name}
    try:
        if scenario not in ("hidden","visible"):
            raise ValueError("invalid cutout scenario")
        pairs=visual.get("pairs",[])
        if len(pairs)!=1:
            raise ValueError("exactly one cutout pose required")
        docs, current=load(pairs[0],scenario=="hidden",fixture_name)
        result["local"]=local(*current)
        if scenario=="hidden":
            result["passed"]=result["local"]["passed"]
            return result
        if reference is None:
            raise ValueError("visible cutout requires accepted hidden control")
        path=Path(reference)
        prior=json.loads((path if path.is_file() else path/"graphics_audit_manifest.json").read_text())
        receipt=prior.get("translucent_terrain_parity" if fixture_name==GLASS_NAME else "cutout_terrain_parity",{})
        if prior.get("success") is not True or receipt.get("passed") is not True or receipt.get("scenario")!="hidden":
            raise ValueError("unaccepted hidden cutout control")
        old=prior["cross_repository_visual_parity"]["pairs"]
        if len(old)!=1:
            raise ValueError("exactly one hidden cutout pose required")
        old_docs, control=load(old[0],True,fixture_name)
        for old_doc,doc in zip(old_docs,docs):
            a=dict(old_doc["terrainParticleFixture"]); b=dict(doc["terrainParticleFixture"])
            a.pop("hidden"); b.pop("hidden")
            a["color"]=a["color"][:3]; b["color"]=b["color"][:3]
            if a!=b:
                raise ValueError("cutout inputs changed beyond visibility")
        result["control_local"]=local(*control)
        result["effects"]=[effect(a,b) for a,b in zip(control,current)]
        # Compare all cells, including transparent holes; never reuse the shriek crop.
        delta=[ImageChops.difference(a,b) for a,b in zip(control,current)]
        result["effect_parity"]=local(*delta)
        result["passed"]=(result["local"]["passed"] and result["control_local"]["passed"]
            and result["effect_parity"]["passed"] and all(e["passed"] for e in result["effects"]))
    except (OSError,ValueError,KeyError,TypeError,IndexError) as error:
        result["error"]=str(error)
    return result
