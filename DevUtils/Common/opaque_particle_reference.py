"""Full footprint of the ordinary camera-facing flame at distance 3, size .35."""
import math
from PIL import ImageChops, ImageStat

BOX = (576, 296, 704, 424)

def requested_sign(visual, scenario, sign):
    if scenario != "particle-atlas-static-a":
        return {"passed":sign == 1,"requested":sign != 1}
    from pathlib import Path
    from graphics_harness import deterministic_capture_document
    pairs=visual.get("pairs",[])
    receipts=[]
    for pair in pairs:
        for side in ("baseline","current"):
            doc=deterministic_capture_document(Path(pair[side+"_artifact"])) or {}
            receipts.append(doc.get("atlasParticleFixture") or {})
    return {"passed":bool(pairs) and type(sign) is int and sign in (-1,1) and all(
        r.get("sizeSign")==sign and r.get("size")==.35*sign and r.get("layer")=="OPAQUE"
        and r.get("complete") is True for r in receipts),
        "requested":True,"size_sign":sign,"receipts":receipts}

def projected_bounds():
    # Independent perspective projection: vertical FOV 70, 1280x720,
    # camera-facing quad centred on the camera ray. Signed size preserves bounds.
    half = 360 * .35 / (3 * math.tan(math.radians(70 / 2)))
    return (640-half, 360-half, 640+half, 360+half)

def full_quad(baseline, current, tolerance):
    if baseline.size != (1280,720) or current.size != baseline.size:
        return {"passed":False,"error":"extent"}
    regions=[]
    for y in range(296,424,16):
        for x in range(576,704,16):
            box=(x,y,x+16,y+16)
            means=ImageStat.Stat(ImageChops.difference(
                baseline.crop(box).convert("RGB"),current.crop(box).convert("RGB"))).mean
            regions.append({"box":box,"mean_rgb_abs":means,"passed":max(means)<=tolerance})
    # Existing centre nonblank/source-identity checks remain required separately.
    return {"passed":all(r["passed"] for r in regions),"box":BOX,
            "projected_bounds":projected_bounds(),"regions":regions,"tolerance":tolerance}
