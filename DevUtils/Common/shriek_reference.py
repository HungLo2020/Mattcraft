"""Ordinary shriek two-quad fixture, with an independently captured delayed control."""
import json
from pathlib import Path
from PIL import Image, ImageChops, ImageStat

BOX = (560, 300, 720, 390)

def fixture(doc, delayed):
    from graphics_harness import water_detail_fixture_matches
    f = doc.get("shriekParticleFixture") or {}
    return (water_detail_fixture_matches(doc)
        and doc["captures"][0].get("gameTime") == 6000
        and f.get("fixture") == "ordinary-shriek-two-quad-v1"
        and f.get("complete") is True and f.get("delayed") is delayed
        and f.get("sprite") == "minecraft:shriek" and f.get("light") == 240
        and f.get("lifetime") == 2_000_000_000
        and isinstance(f.get("size"), (int,float)) and abs(f["size"] - 0.31875) < 0.000001
        and len(f.get("position", [])) == 3
        and all(abs(a-b) < 0.000001 for a,b in zip(f["position"],
            (147.64617596880336,101.0991813627766,529.7355156371002))))

def local(a, b):
    if a.size != (1280,720) or b.size != a.size:
        return {"passed":False, "error":"extent"}
    regions = []
    for y in (300,330,360):
        for x in (560,600,640,680):
            box = (x,y,x+40,y+30)
            error = ImageStat.Stat(ImageChops.difference(a.crop(box),b.crop(box))).mean
            regions.append({"box":box,"mean_rgb_abs":error,"passed":max(error)<=6})
    return {"passed":all(r["passed"] for r in regions),"regions":regions}

def effect(before, after):
    if before.size != (1280,720) or after.size != before.size:
        return {"passed":False, "error":"extent"}
    delta = ImageChops.difference(before.crop(BOX),after.crop(BOX))
    mean = ImageStat.Stat(delta).mean
    changed = sum(max(pixel)>3 for pixel in delta.getdata())
    return {"passed":max(mean)>0.1 and changed>=32,"mean_rgb_change":mean,"changed_pixels":changed}

def load(pair, delayed):
    from graphics_harness import deterministic_capture_document
    if pair.get("fixture_equivalence",{}).get("status") != "passed":
        raise ValueError("incomparable fixture")
    result = []
    for side in ("baseline","current"):
        doc = deterministic_capture_document(Path(pair[side+"_artifact"])) or {}
        if not fixture(doc, delayed): raise ValueError("missing or incorrect shriek fixture")
        with Image.open(pair[side+"_image"]) as image: result.append(image.convert("RGB"))
    return result

def changed_pixel_parity(hidden, baseline, current):
    if any(image.size != (1280,720) for image in (hidden,baseline,current)):
        return {"passed":False,"error":"extent"}
    changes = ImageChops.difference(hidden.crop(BOX),baseline.crop(BOX))
    errors = ImageChops.difference(baseline.crop(BOX),current.crop(BOX))
    selected = [error for change,error in zip(changes.getdata(),errors.getdata()) if max(change)>3]
    means = [sum(pixel[c] for pixel in selected)/len(selected) for c in range(3)] if selected else []
    return {"passed":len(selected)>=32 and max(means)<=6,
            "pixels":len(selected),"mean_rgb_abs":means}

def report(visual, scenario, reference=None):
    if scenario is None: return {"passed":True,"requested":False}
    result = {"passed":False,"requested":True,"scenario":scenario}
    try:
        pairs = visual.get("pairs",[])
        if len(pairs)!=1: raise ValueError("exactly one shriek pose required")
        current = load(pairs[0], scenario=="delayed")
        result["local"] = local(*current)
        if scenario=="delayed":
            result["passed"] = result["local"]["passed"]
            return result
        if reference is None: raise ValueError("visible shriek requires delayed control")
        prior = json.loads(Path(reference).read_text())
        receipt = prior.get("shriek_parity",{})
        if prior.get("success") is not True or receipt.get("passed") is not True or receipt.get("scenario")!="delayed":
            raise ValueError("unaccepted delayed control")
        old = prior["cross_repository_visual_parity"]["pairs"]
        if len(old)!=1: raise ValueError("exactly one control pose required")
        control = load(old[0], True)
        result["control_local"] = local(*control)
        result["effects"] = [effect(a,b) for a,b in zip(control,current)]
        result["changed_pixel_parity"] = changed_pixel_parity(control[0],*current)
        result["passed"] = (result["local"]["passed"] and result["control_local"]["passed"]
            and result["changed_pixel_parity"]["passed"]
            and all(e["passed"] for e in result["effects"]))
    except (OSError,ValueError,KeyError,TypeError) as error:
        result["error"] = str(error)
    return result
