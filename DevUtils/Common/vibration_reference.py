"""Fixed-simulation vibration: ordinary extraction, animated atlas, positive pixels."""
import json
from pathlib import Path
from PIL import Image, ImageChops
from shriek_reference import local, effect, changed_pixel_parity
import shriek_reference as stationary_metrics
import vibration_motion_reference as moving_metrics
import vibration_elevated_reference as elevated_metrics

# Independent projection of the zero-step quads reaches y=414.22. Retain the
# original sky witness AND cover the full geometry; transparent corners must
# not hide an omitted lower edge when the sprite animation phase changes.
FULL_ZERO_STEP_BOX = (560,300,720,420)

def full_quad_changed_pixel_parity(hidden, baseline, current, elevation=0):
    if any(p.size != (1280,720) for p in (hidden,baseline,current)):
        return {"passed":False,"error":"extent"}
    box = elevated_metrics.BOX if elevation else FULL_ZERO_STEP_BOX
    changes = ImageChops.difference(hidden.crop(box),baseline.crop(box))
    errors = ImageChops.difference(baseline.crop(box),current.crop(box))
    selected = [e for c,e in zip(changes.getdata(),errors.getdata()) if max(c)>3]
    mean = [sum(p[c] for p in selected)/len(selected) for c in range(3)] if selected else []
    return {"passed":len(selected)>=32 and max(mean)<=6,
        "box":box,"pixels":len(selected),"mean_rgb_abs":mean}

def fixture(doc, hidden, steps=0, elevation=0):
    from graphics_harness import water_detail_fixture_matches
    f = doc.get("vibrationParticleFixture")
    return (isinstance(f, dict) and water_detail_fixture_matches(doc)
        and doc["captures"][0].get("gameTime") == 6000
        and f.get("fixture") == "ordinary-vibration-fixed-simulation-v1"
        and f.get("hidden") is hidden and f.get("complete") is True
        and f.get("sprite") == "minecraft:vibration" and f.get("light") == 240
        and f.get("age") == 12 + steps and f.get("lifetime") == 40
        and f.get("simulationSteps",0) == steps
        and f.get("elevation",0) == elevation
        and f.get("extractionPartialTick") == 1.0
        and f.get("alpha") == (0 if hidden else 1)
        and isinstance(f.get("size"), (int,float)) and abs(f["size"] - 0.3) < 0.000001
        and isinstance(f.get("position"), list) and len(f["position"]) == 3
        and all(isinstance(a,(int,float)) and abs(a-b) < 0.000001 for a,b in zip(f["position"],
            expected_position(steps,elevation))))

def expected_position(steps, elevation=0):
    origin=(147.64617596880336,101.0991813627766+elevation,529.7355156371002)
    target=(149.5,102.5+elevation,526.5)
    return tuple(a+(b-a)*steps/27 for a,b in zip(origin,target))

def load(pair, hidden, phase=0, steps=0, elevation=0):
    from graphics_harness import deterministic_capture_document, block_display_animation_upload_equivalence
    if pair.get("fixture_equivalence",{}).get("status") != "passed":
        raise ValueError("incomparable fixture")
    docs = [deterministic_capture_document(Path(pair[s+"_artifact"])) or {} for s in ("baseline","current")]
    if not all(fixture(d,hidden,steps,elevation) for d in docs): raise ValueError("missing or incorrect vibration fixture")
    animation = block_display_animation_upload_equivalence(docs[0], Path(pair["current_artifact"]),
        required_sprite="minecraft:vibration", required_texture=0x50415254, required_mip_levels=1)
    if not animation or not animation.get("passed"): raise ValueError("vibration uploaded-pixel mismatch")
    if animation["frozen"].get("frame") != phase or animation["frozen"].get("subFrame") != 0:
        raise ValueError("vibration capture must use its declared phase")
    images = []
    for side in ("baseline","current"):
        with Image.open(pair[side+"_image"]) as image: images.append(image.convert("RGB"))
    return images, animation

def report(visual, scenario, reference=None, phase=0, phase_reference=None, steps=0, motion_reference=None, elevation=0):
    if scenario is None: return {"passed":True,"requested":False}
    result = {"passed":False,"requested":True,"scenario":scenario,"phase":phase,"simulation_steps":steps,"elevation":elevation}
    try:
        if type(phase) is not int or phase not in range(7): raise ValueError("invalid vibration phase")
        if type(elevation) is not int or elevation not in (0,1) or (elevation and steps):
            raise ValueError("elevated fixture requires initial simulation pose")
        if type(steps) is not int or steps not in (0,6) or (steps and phase):
            raise ValueError("six-step snapshot requires phase zero")
        metrics = elevated_metrics if elevation else moving_metrics if steps else stationary_metrics
        pairs = visual.get("pairs",[])
        if len(pairs)!=1: raise ValueError("exactly one vibration pose required")
        images, result["animation"] = load(pairs[0], scenario=="hidden", phase, steps, elevation)
        result["local"] = metrics.local(*images)
        if scenario=="hidden":
            result["passed"] = result["local"]["passed"]
            return result
        if reference is None: raise ValueError("visible vibration requires invisible control")
        prior = json.loads(Path(reference).read_text())
        receipt = prior.get("vibration_parity",{})
        if prior.get("success") is not True or receipt.get("passed") is not True or receipt.get("scenario")!="hidden":
            raise ValueError("unaccepted invisible control")
        if receipt.get("elevation",0) != elevation: raise ValueError("invisible control elevation differs")
        old = prior["cross_repository_visual_parity"]["pairs"]
        if len(old)!=1: raise ValueError("exactly one control pose required")
        control, result["control_animation"] = load(old[0], True, receipt.get("phase",0), 0, elevation)
        result["control_local"] = metrics.local(*control)
        result["effects"] = [metrics.effect(a,b) for a,b in zip(control,images)]
        result["changed_pixel_parity"] = metrics.changed_pixel_parity(control[0],*images)
        result["passed"] = (result["local"]["passed"] and result["control_local"]["passed"]
            and result["changed_pixel_parity"]["passed"] and all(e["passed"] for e in result["effects"]))
        if not steps:
            result["full_quad_parity"] = full_quad_changed_pixel_parity(control[0],*images,elevation=elevation)
            result["passed"] = result["passed"] and result["full_quad_parity"]["passed"]
        if phase != 0:
            if phase_reference is None: raise ValueError("nonzero phase requires accepted visible phase-zero reference")
            initial = json.loads(Path(phase_reference).read_text())
            initial_receipt = initial.get("vibration_parity",{})
            if (initial.get("success") is not True or initial_receipt.get("passed") is not True
                or initial_receipt.get("scenario") != "visible" or initial_receipt.get("phase",0) != 0
                or initial_receipt.get("elevation",0) != elevation):
                raise ValueError("unaccepted visible phase-zero reference")
            prior_pairs = initial["cross_repository_visual_parity"]["pairs"]
            if len(prior_pairs)!=1: raise ValueError("exactly one phase-zero pose required")
            previous, result["phase_zero_animation"] = load(prior_pairs[0],False,0,0,elevation)
            result["phase_zero_local"] = metrics.local(*previous)
            result["phase_effects"] = [metrics.effect(a,b) for a,b in zip(previous,images)]
            result["phase_changed_pixel_parity"] = metrics.changed_pixel_parity(previous[0],*images)
            result["full_quad_phase_parity"] = full_quad_changed_pixel_parity(previous[0],*images,elevation=elevation)
            result["passed"] = (result["passed"] and result["phase_zero_local"]["passed"]
                and all(e["passed"] for e in result["phase_effects"])
                and result["phase_changed_pixel_parity"]["passed"]
                and result["full_quad_phase_parity"]["passed"])
        if steps:
            if motion_reference is None: raise ValueError("motion requires accepted zero-step visible reference")
            initial = json.loads(Path(motion_reference).read_text())
            receipt = initial.get("vibration_parity",{})
            if (initial.get("success") is not True or receipt.get("passed") is not True
                or receipt.get("scenario") != "visible" or receipt.get("phase",0) != 0
                or receipt.get("simulation_steps",0) != 0):
                raise ValueError("unaccepted zero-step motion reference")
            old = initial["cross_repository_visual_parity"]["pairs"]
            if len(old)!=1: raise ValueError("exactly one motion reference pose required")
            previous,result["motion_reference_animation"] = load(old[0],False,0,0)
            result["motion_reference_local"] = local(*previous)
            result["motion_effects"] = [metrics.effect(a,b) for a,b in zip(previous,images)]
            result["old_location_parity"] = changed_pixel_parity(previous[0],*images)
            result["full_old_location_parity"] = full_quad_changed_pixel_parity(previous[0],*images)
            result["passed"] = (result["passed"] and result["motion_reference_local"]["passed"]
                and all(e["passed"] for e in result["motion_effects"])
                and result["old_location_parity"]["passed"]
                and result["full_old_location_parity"]["passed"])
    except (OSError,ValueError,KeyError,TypeError) as error:
        result["passed"] = False
        result["error"] = str(error)
    return result
