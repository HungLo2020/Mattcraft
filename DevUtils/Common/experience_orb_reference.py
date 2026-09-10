"""Local pixel evidence for the shared orb fixture; not a route-admission gate.

Requires a real hidden control in each backend. Identical empty pictures must
never establish rendering coverage. Ownership/input/capture checks are separate.
"""
from PIL import ImageChops
import math
import json
import re


def pack_signature(meta, scenario):
    variants = {"experience-orb-occlusion": ("occlusion",), "experience-orb-a": ("a",), "experience-orb-b": ("b",),
                "experience-orb-replacement": ("b", "a")}
    if scenario not in variants:
        raise ValueError("unknown expected orb pack scenario")
    names = ["mattmc-experience-orb-" + variant for variant in variants[scenario]]
    if (meta.get("gui_resource_pack_scenario") != scenario
            or json.loads(meta.get("gui_resource_pack_selected", "null")) != ["file/" + name for name in names]):
        raise ValueError("wrong selected orb resource packs")
    hashes = tuple(meta.get("gui_resource_pack_" + name + "_sha256", "") for name in names)
    if any(not isinstance(value, str) or not re.fullmatch("[0-9a-f]{64}", value) for value in hashes):
        raise ValueError("missing generated orb pack content hashes")
    return hashes


def validate_pack_reload(document, removed_pack):
    receipt = document.get("worldResourceReload") or {}
    if (receipt.get("schema") != "normal-world-resource-reload-v1"
            or any(receipt.get(key) is not True for key in ("requested", "futureComplete", "complete"))
            or type(receipt.get("presentations")) is not int or receipt["presentations"] < 2):
        raise ValueError("missing completed and presented orb pack reload")
    before, after = receipt.get("selectedBefore"), receipt.get("selectedAtCapture")
    if (not isinstance(before, list) or not isinstance(after, list)
            or any(not isinstance(pack, str) for pack in before + after)
            or before.count(removed_pack) != 1
            or after != [pack for pack in before if pack != removed_pack]
            or [p for p in before if p.startswith("file/")] !=
                ["file/mattmc-experience-orb-b", "file/mattmc-experience-orb-a"]
            or removed_pack != "file/mattmc-experience-orb-a"):
        raise ValueError("orb reload did not replace pack A with pack B")

BOX = (700, 300, 765, 370)
SIZE = (1280, 720)
POSE_COUNT = 5
POSES = ["initial", "right", "left", "return", "initial"]
VALUES = [1, 3, 7, 17, 37, 73, 149, 307, 617, 1237, 2477]

def environment_signature(document):
    captures = document.get("captures", [])
    signature = tuple(tuple(capture.get(key) for key in
                            ("lightmapSemanticFingerprint", "weatherSemanticFingerprint", "shaderPack"))
                      for capture in captures)
    if len(signature) != POSE_COUNT or any(not isinstance(value, str) or not value
                                           for row in signature for value in row):
        raise ValueError("missing five-pose lighting/weather/settings evidence")
    return signature


def validate_document(document, *, hidden, native, icon=1, age=4, game_time=6000, age_step=0,
                      distance_sequence=False):
    """Validate fixture inputs and captured native identities, not just pixels."""
    if type(icon) is not int or not 0 <= icon < len(VALUES):
        raise ValueError("invalid expected orb icon")
    fixture = document.get("experienceOrbFixture") or {}
    if type(age_step) not in (int, float) or not math.isfinite(age_step) or age_step < 0:
        raise ValueError("invalid expected orb age step")
    final_age = age + age_step * (POSE_COUNT - 1)
    if age_step:
        expected_ages = [None] * POSE_COUNT if hidden else [age + age_step * i for i in range(POSE_COUNT)]
        if (fixture.get("ageBase") != age or fixture.get("ageStep") != age_step
                or fixture.get("poseIndex") != POSE_COUNT - 1
                or fixture.get("observedPoseAges") != expected_ages):
            raise ValueError("missing deterministic orb animation inputs")
    if (document.get("status") != "complete" or fixture.get("fixture") != "native-experience-orb-v1"
            or fixture.get("complete") is not True or fixture.get("hidden") is not hidden
            or fixture.get("present") is not (not hidden) or fixture.get("icon") != icon
            or type(fixture.get("icon")) is not int
            or fixture.get("value") != VALUES[icon] or fixture.get("age") != final_age
            or fixture.get("entityId") != 2147483534):
        raise ValueError("incomplete or incorrect orb fixture")
    position = fixture.get("position", [])
    expected = (147.80143035574073, 101.0991813627766, 529.155950217724)
    if distance_sequence:
        eye = (150.5, 100.0 + 1.6200000047683716, 530.5)
        expected_positions = [[e + (v-e) * (3+i)/3 for e,v in zip(eye,expected)] for i in range(POSE_COUNT)]
        configured = fixture.get("configuredPosePositions", [])
        observed = fixture.get("observedPosePositions", [])
        def matches(actual):
            return (len(actual) == POSE_COUNT and all(isinstance(row,list) and len(row)==3
                and all(type(v) in (int,float) and math.isfinite(v) and abs(v-e)<1e-6
                        for v,e in zip(row,target)) for row,target in zip(actual,expected_positions)))
        if (fixture.get("distanceSequence") is not True or not matches(configured)
                or (observed != [None]*POSE_COUNT if hidden else not matches(observed))):
            raise ValueError("wrong five-pose orb distance inputs")
        expected = expected_positions[-1]
    if len(position) != 3 or any(type(v) not in (int,float) or not math.isfinite(v) or abs(v-e)>1e-6
                                  for v,e in zip(position, expected)):
        raise ValueError("wrong orb fixture position")
    if hidden:
        if any(fixture.get(key) != 0 for key in ("extractions", "submits", "callbacks")):
            raise ValueError("hidden fixture reached rendering")
    elif (fixture.get("extractedIcon") != icon or fixture.get("extractedAge") != final_age
            or type(fixture.get("extractedLight")) is not int or not 0 <= fixture["extractedLight"] <= 0x00f000f0
            or fixture.get("extractions", 0) <= 0):
        raise ValueError("missing extracted orb inputs")
    captures = document.get("captures", [])
    if len(captures) != POSE_COUNT or [c.get("poseName") for c in captures] != POSES:
        raise ValueError("missing five-pose orb sequence")
    previous = 0
    previous_generation = 0
    for capture in captures:
        frame = capture.get("renderedFrameIndex")
        if (type(frame) is not int or frame <= previous or capture.get("gameTime") != game_time
                or capture.get("shaderEnabled") != "false"
                or capture.get("backend") != ("vulkan" if native else "opengl")
                or capture.get("window") != {"width":1280,"height":720}
                or capture.get("position") != {"x":150.5,"y":100.0,"z":530.5}
                or capture.get("dimension") != "minecraft:overworld"
                or any(capture.get(k) != v for k,v in (("observedYaw",105),("requestedYaw",105),
                                                       ("observedPitch",10),("requestedPitch",10)))):
            raise ValueError("incorrect captured orb frame inputs")
        previous = frame
        if not native:
            continue
        if capture.get("captureMethod") != "rust-vulkan-final-output":
            raise ValueError("orb capture is not Rust final output")
        receipts = [r for r in document.get("rustGalWorldExperienceOrbExecution", [])
                    if r.get("deterministicFrameIndex") == frame]
        if hidden:
            if any(r.get("quads", 0) != 0 or r.get("nativeOrbCount", 0) != 0 for r in receipts):
                raise ValueError("hidden capture contains submitted orbs")
            continue
        if len(receipts) != 1:
            raise ValueError("missing unique captured orb execution")
        receipt = receipts[0]
        resources = receipt.get("nativeResources", [])
        if (receipt.get("route") != "rust-vulkan-whole-frame" or receipt.get("quads") != 1
                or type(receipt.get("quads")) is not int or type(receipt.get("nativeOrbCount")) is not int
                or receipt.get("nativeOrbCount") != 1 or receipt.get("nativeResourcesComplete") is not True
                or len(resources) != 1 or type(receipt.get("gameplayFrameId")) is not int
                or type(receipt.get("submissionId")) is not int or receipt.get("gameplayFrameId", 0) <= 0
                or receipt.get("submissionId", 0) <= 0):
            raise ValueError("generic or incomplete orb execution receipt")
        resource = resources[0]
        if (resource.get("meshKey") != "5715703444854013953"
                or not str(resource.get("meshGeneration","")).isdigit()
                or int(resource["meshGeneration"]) <= 0 or resource.get("entityId") != fixture["entityId"]):
            raise ValueError("wrong typed orb resource identity")
        if age_step and int(resource["meshGeneration"]) <= previous_generation:
            raise ValueError("animated orb reused stale appearance generation")
        previous_generation = int(resource["meshGeneration"])
    return fixture


def _crop(image):
    if image.size != SIZE:
        raise ValueError("orb fixture requires 1280x720 capture")
    return image.convert("RGB").crop(BOX)


def local_difference(current, frozen):
    delta = ImageChops.difference(_crop(current), _crop(frozen))
    pixels = list(delta.getdata())
    return {
        "box": list(BOX),
        "max_channel_abs": max(max(pixel) for pixel in pixels),
        "changed_pixels": sum(any(pixel) for pixel in pixels),
        "mean_rgb_abs": [sum(pixel[c] for pixel in pixels) / len(pixels) for c in range(3)],
    }


def frame_effect(cv, ch, fv, fh):
    current_effect = ImageChops.difference(_crop(cv), _crop(ch))
    frozen_effect = ImageChops.difference(_crop(fv), _crop(fh))
    changes = [sum(max(pixel) > 3 for pixel in image.getdata())
               for image in (current_effect, frozen_effect)]
    effect_error = max(high for low, high in ImageChops.difference(current_effect, frozen_effect).getextrema())
    visible_error = local_difference(cv, fv)["max_channel_abs"]
    hidden_error = local_difference(ch, fh)["max_channel_abs"]
    return {
        "current_changed_pixels": changes[0],
        "frozen_changed_pixels": changes[1],
        "effect_max_channel_abs": effect_error,
        "visible_max_channel_abs": visible_error,
        "hidden_max_channel_abs": hidden_error,
        # Six channel levels is the existing local pixel tolerance; require
        # it at EVERY pixel here, not averaged over a mostly empty screen.
        "passed": min(changes) >= 32 and max(effect_error, visible_error, hidden_error) <= 6,
    }


def paired_effect(current_visible, current_hidden, frozen_visible, frozen_hidden):
    sequences = (current_visible, current_hidden, frozen_visible, frozen_hidden)
    if any(len(sequence) != POSE_COUNT for sequence in sequences):
        raise ValueError("orb evidence requires five visible and hidden captures per backend")
    results = [frame_effect(*frames) for frames in zip(*sequences)]
    return {"scope": "shared-orb-local-pixels-only", "passed": all(row["passed"] for row in results),
            "poses": results}


def occlusion_pixels(cv, ch, fv, fh, reference_current, reference_frozen):
    sequences = (cv, ch, fv, fh, reference_current, reference_frozen)
    if any(len(sequence) != POSE_COUNT for sequence in sequences):
        raise ValueError("occlusion requires six complete five-pose image sequences")
    rows = []
    for current, current_hidden, frozen, frozen_hidden, before_current, before_frozen in zip(*sequences):
        block = frame_effect(current_hidden, before_current, frozen_hidden, before_frozen)
        current_leak = local_difference(current, current_hidden)["changed_pixels"]
        frozen_leak = local_difference(frozen, frozen_hidden)["changed_pixels"]
        rows.append({"block_effect": block, "current_orb_changed_pixels": current_leak,
                     "frozen_orb_changed_pixels": frozen_leak,
                     "passed": block["passed"] and current_leak == frozen_leak == 0})
    return {"scope":"orb-opaque-block-occlusion", "poses":rows,
            "passed":all(row["passed"] for row in rows)}


def report(visible_manifest, hidden_manifest, *, icon=1, age=4, game_time=6000, pack_scenario=None,
           removed_pack=None, age_step=0, distance_sequence=False, occluded=False,
           unoccluded_reference=None):
    """Combine existing manifest gates, typed submission receipts and local effects."""
    from pathlib import Path
    from PIL import Image
    import graphics_harness as harness
    from block_marker_reference import ownership
    images, fixtures, environments = {}, {}, {}
    packs = []
    result = {"scope":"shared-native-orb-fixture", "icon":icon, "age":age, "game_time":game_time, "passed":False}
    try:
        reference_images = []
        reference_environments = []
        if occluded:
            if (distance_sequence or age_step or pack_scenario not in (None, "experience-orb-occlusion")
                    or not isinstance(unoccluded_reference, (tuple,list)) or len(unoccluded_reference) != 2):
                raise ValueError("occlusion requires a stationary vanilla unoccluded reference pair")
            reference = report(*unoccluded_reference, icon=icon, age=age, game_time=game_time,
                               pack_scenario=pack_scenario)
            if reference.get("passed") is not True:
                raise ValueError("unoccluded orb reference did not pass")
            reference_pair = harness.read_json(Path(unoccluded_reference[1]))["cross_repository_visual_parity"]["pairs"][0]
            for side in ("current", "baseline"):
                reference_document = harness.deterministic_capture_document(Path(reference_pair[side+"_artifact"]))
                reference_environments.append(environment_signature(reference_document))
                frames = []
                for capture in reference_document["captures"]:
                    with Image.open(capture["screenshot"]) as image:
                        frames.append(image.convert("RGB"))
                reference_images.append(frames)
            result["unoccluded_reference_verified"] = True
        for hidden, path in ((False, visible_manifest), (True, hidden_manifest)):
            manifest = harness.read_json(Path(path)) or {}
            if manifest.get("success") is not True:
                raise ValueError("existing manifest gates did not pass")
            pairs = manifest.get("cross_repository_visual_parity", {}).get("pairs", [])
            if len(pairs) != 1 or pairs[0].get("fixture_equivalence", {}).get("status") != "passed":
                raise ValueError("missing equivalent cross-repository fixture")
            documents = {}
            for native, side in ((True,"current"), (False,"baseline")):
                artifact_path = Path(pairs[0][side+"_artifact"])
                artifact = harness.read_json(artifact_path) or {}
                if pack_scenario is not None:
                    meta_path = harness.latest_capture_meta_path(artifact_path.parent / "capture")
                    if meta_path is None:
                        raise ValueError("missing orb pack capture metadata")
                    packs.append(pack_signature(harness.read_key_values(meta_path), pack_scenario))
                if native:
                    if not ownership(artifact):
                        raise ValueError("missing clean Rust-owned execution")
                elif (artifact.get("implementation_attribution") != "java-opengl"
                      or any(artifact.get("validation", {}).get(k) is not True
                             for k in ("complete","crash_free","strict_gl_error_scan_passed"))):
                    raise ValueError("baseline is not clean Frozen Java OpenGL")
                document = harness.deterministic_capture_document(artifact_path) or {}
                if occluded:
                    if environment_signature(document) != reference_environments[0 if native else 1]:
                        raise ValueError("occlusion lighting/weather/settings differ from unobstructed reference")
                    fixture = document.get("experienceOrbFixture", {})
                    if (fixture.get("occluderRequested") is not True
                            or fixture.get("occluderReady") is not True
                            or fixture.get("occluder") != {"x":147,"y":101,"z":529,"block":"minecraft:stone"}
                            or (not native and not hidden and fixture.get("callbacks",0) <= 0)):
                        raise ValueError("missing real stone occluder or Frozen orb submission")
                if removed_pack is not None:
                    if pack_scenario != "experience-orb-replacement":
                        raise ValueError("orb removal requires replacement pack fixture")
                    validate_pack_reload(document, removed_pack)
                fixtures[hidden,native] = validate_document(document, hidden=hidden, native=native,
                                                           icon=icon, age=age, game_time=game_time, age_step=age_step,
                                                           distance_sequence=distance_sequence)
                documents[native] = document
                environments[hidden,native] = environment_signature(document)
                if native:
                    correlation = artifact.get("capture", {}).get("whole_frame_gameplay_attachments", {}).get("correlation_doc", {})
                    final_frame = document["captures"][-1]["renderedFrameIndex"]
                    if (correlation.get("same_acquired_presented_image") is not True
                            or correlation.get("deterministic_rendered_frame_index") != final_frame):
                        raise ValueError("uncorrelated final presented image")
                    if not hidden:
                        receipt = next(r for r in document["rustGalWorldExperienceOrbExecution"]
                                       if r["deterministicFrameIndex"] == final_frame)
                        if (receipt["submissionId"] != correlation.get("gal_submission_id")
                                or receipt["gameplayFrameId"] != correlation.get("gameplay_frame_id")):
                            raise ValueError("orb execution names a different submitted image")
                images[hidden,native] = []
                for capture in document["captures"]:
                    with Image.open(capture["screenshot"]) as image:
                        images[hidden,native].append(image.convert("RGB"))
            for current, frozen in zip(documents[True]["captures"], documents[False]["captures"]):
                for key in ("lightmapSemanticFingerprint", "weatherSemanticFingerprint"):
                    if not current.get(key) or current[key] != frozen.get(key):
                        raise ValueError("different lighting/weather inputs")
            if not hidden and fixtures[False,True]["extractedLight"] != fixtures[False,False]["extractedLight"]:
                raise ValueError("different orb light samples")
        identity_keys = ("fixture","icon","age","entityId","value","position")
        identities = [tuple(f[k] if k != "position" else tuple(f[k]) for k in identity_keys)
                      for f in fixtures.values()]
        if len(set(identities)) != 1:
            raise ValueError("visible/hidden fixture identity changed")
        if len(set(environments.values())) != 1:
            raise ValueError("visible/hidden lighting, weather or shader-pack setting changed")
        if pack_scenario is not None:
            if len(packs) != 4 or len(set(packs)) != 1:
                raise ValueError("visible/hidden orb resource pack contents differ")
            if occluded and tuple(reference.get("pack_hashes", ())) != packs[0]:
                raise ValueError("occlusion pack contents differ from unobstructed reference")
            result["pack_scenario"] = pack_scenario
            result["pack_hashes"] = packs[0]
        if removed_pack is not None:
            result["reload_removed_pack"] = removed_pack
        if occluded:
            result["pixels"] = occlusion_pixels(images[False,True], images[True,True],
                images[False,False], images[True,False], *reference_images)
        else:
            result["pixels"] = paired_effect(images[False,True], images[True,True],
                                             images[False,False], images[True,False])
        result["passed"] = result["pixels"]["passed"]
        if distance_sequence:
            result["distance_sequence"] = True
            for side in ("current_changed_pixels", "frozen_changed_pixels"):
                sizes = [pose[side] for pose in result["pixels"]["poses"]]
                if not all(a > b for a,b in zip(sizes,sizes[1:])):
                    raise ValueError("orb did not shrink as world distance increased")
        if age_step:
            result["age_step"] = age_step
            result["animation_effects"] = []
            for index in range(1, POSE_COUNT):
                # No world/camera/light change: the pulse must visibly update,
                # and its pixel delta must match Frozen, not just the final pose.
                current_before, current_after = images[False,True][index-1:index+1]
                frozen_before, frozen_after = images[False,False][index-1:index+1]
                effect = frame_effect(current_after, current_before, frozen_after, frozen_before)
                result["animation_effects"].append(effect)
                result["passed"] &= effect["passed"]
        result["typed_submission_and_ownership_verified"] = True
    except (ValueError, KeyError, TypeError, OSError, StopIteration) as error:
        result["passed"] = False
        result["error"] = str(error)
    return result
