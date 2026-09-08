"""Local, predetermined lava witnesses; no image fitting or threshold relaxation."""
import math


def lava_mip_evidence(documents):
    """An option alone does not prove either an allocation or a complete mip chain."""
    try:
        if len(documents) != 2:
            raise ValueError("two captured mip observations required")
        observations = [doc["blockDisplayAnimationAtCapture"] for doc in documents]
        actual = [obs["mipLevels"] for obs in observations]
        # Historical one-level captures predate requested-level diagnostics;
        # they remain one-level evidence only, never multi-mip admission.
        requested = [obs.get("requestedMipLevels", 1 if obs["mipLevels"] == 1 else None)
                     for obs in observations]
        if any(type(value) is not int or not 1 <= value <= 32 for value in actual + requested):
            raise ValueError("missing valid requested/allocated mip counts")
        if len(set(actual + requested)) != 1:
            raise ValueError("requested and observed mip counts disagree")
        return {"passed": True, "levels": actual[0], "requested": requested, "observed": actual}
    except (KeyError, TypeError, ValueError) as error:
        return {"passed": False, "error": str(error)}


def required_lava_report(visual_report, requested, flowing=None):
    pairs = visual_report.get("pairs", []) if isinstance(visual_report, dict) else []
    passed = isinstance(pairs, list) and bool(pairs) and all(
        isinstance(pair, dict) and pair.get("status") == "complete"
        and all(isinstance(pair.get(key), dict) and pair[key].get("passed") is True
                for key in ("lava_surface", "lava_animation"))
        and (flowing is None or pair["lava_surface"].get("flowing") is flowing) for pair in pairs)
    return {"requested": requested, "passed": not requested or passed}


def lava_phase_change_evidence(before, after):
    """Both paired phases must pass, and the same pixels must actually change."""
    return _lava_image_change_evidence(before, after, mip_sampling=False)


def lava_mip_sampling_evidence(without_mips, with_mips):
    """Same level-zero upload, different allocated chains, matching pixel changes.

    Callers must additionally establish equivalent pack/world/settings inputs;
    this is the sampling witness, not a substitute for the paired harness gates.
    """
    return _lava_image_change_evidence(without_mips, with_mips, mip_sampling=True)


def _lava_image_change_evidence(before, after, *, mip_sampling):
    from PIL import Image
    try:
        if not all(required_lava_report({"pairs": [pair]}, True)["passed"] for pair in (before, after)):
            raise ValueError("both individual phase comparisons must pass")
        digests = []
        boxes = []
        images = []
        for pair in (before, after):
            animation = pair["lava_animation"]
            digest = animation["frozen"]["uploadedRgbaFnv64"]
            if animation["native"]["retained_rgba_fnv64"] != digest:
                raise ValueError("unmatched phase upload")
            digests.append(digest)
            boxes.append([patch["box"] for patch in pair["lava_surface"]["patches"]])
            phase_images = []
            for side in ("baseline", "current"):
                with Image.open(pair[side + "_image"]) as source:
                    if source.size != (1280, 720):
                        raise ValueError("unexpected transition image size")
                    phase_images.append(source.convert("RGB"))
            images.append(phase_images)
        if boxes[0] != boxes[1] or len(boxes[0]) != 9:
            raise ValueError("different projected witnesses")
        if mip_sampling:
            modes = [pair["lava_surface"]["flowing"] for pair in (before, after)]
            if any(type(mode) is not bool for mode in modes) or modes[0] != modes[1]:
                raise ValueError("different or unobserved lava surface modes")
            sprite = "minecraft:block/lava_flow" if modes[0] else "minecraft:block/lava_still"
            if any(pair["lava_animation"]["frozen"]["spriteName"] != sprite for pair in (before, after)):
                raise ValueError("uploaded sprite does not match the lava surface mode")
            if digests[0] != digests[1]:
                raise ValueError("level-zero source changed during mip comparison")
            for pair, levels in ((before, 1), (after, 5)):
                mips = pair["lava_mips"]
                if mips != {"passed": True, "levels": levels,
                            "requested": [levels, levels], "observed": [levels, levels]}:
                    raise ValueError("missing requested and observed mip chain evidence")
                if any(type(value) is not int for value in
                       [mips["levels"], *mips["requested"], *mips["observed"]]):
                    raise ValueError("invalid mip count types")
                animation = pair["lava_animation"]
                native, frozen = animation["native"], animation["frozen"]
                if (native["mip_levels"] != levels or frozen["mipLevels"] != levels
                        or len(native["retained_mip_rgba_fnv64"]) != levels
                        or native["retained_mip_rgba_fnv64"] != frozen["uploadedMipRgbaFnv64"]
                        or native["retained_mip_rgba_fnv64"][0] != digests[0]):
                    raise ValueError("incomplete or mismatched uploaded mip chain")
        elif digests[0] == digests[1]:
            raise ValueError("unchanged phase uploads")
        changes = []
        for box in boxes[0]:
            pixels = [[list(image.crop(tuple(box)).getdata()) for image in phase] for phase in images]
            # Signed differences reject motion in the wrong direction; magnitude
            # alone could accept an unrelated animated pattern.
            deltas = [[[b - a for a, b in zip(old, new)]
                       for old, new in zip(pixels[0][side], pixels[1][side])] for side in (0, 1)]
            means = [[sum(abs(p[c]) for p in delta) / 25 for c in range(3)] for delta in deltas]
            error = [sum(abs(a[c] - b[c]) for a, b in zip(*deltas)) / 25 for c in range(3)]
            changes.append({"box": box, "mean_abs_change": means, "signed_change_error": error,
                            "changed": min(max(mean) for mean in means) > 1})
        changed = sum(row["changed"] for row in changes)
        return {"passed": changed >= 6 and all(max(row["signed_change_error"]) <= 6 for row in changes),
                "changed_patches": changed, "patches": changes,
                "level_zero_upload_hashes" if mip_sampling else "phase_upload_hashes": digests}
    except (OSError, KeyError, TypeError, ValueError, IndexError) as error:
        return {"passed": False, "error": str(error)}


def lava_surface_evidence(baseline_doc, current_doc, baseline, current, *, flowing=False):
    from PIL import ImageChops, ImageStat
    expected = {"fixture": "sealed-lava-basin-v2", "origin": "145, 99, 530",
                "cells": 75, "matchingCells": 75, "complete": True}
    if flowing:
        expected = {"fixture": "single-source-lava-flow-v1", "origin": "145, 98, 530",
                    "cells": 90, "matchingCells": 90, "complete": True,
                    "direction": "west", "expectedLevels": [0, 2, 4, 6]}
    try:
        if baseline.size != (1280, 720) or current.size != baseline.size:
            raise ValueError("capture size mismatch")
        points = []
        for doc in (baseline_doc, current_doc):
            if "lavaParticleOcclusionAtCapture" in doc:
                occlusion = doc["lavaParticleOcclusionAtCapture"]
                if (not isinstance(occlusion, dict)
                        or occlusion.get("schema") != "fluid-sample-particle-occlusion-v1"
                        or occlusion.get("complete") is not True or occlusion.get("ready") is not True
                        or type(occlusion.get("blockers")) is not int or occlusion["blockers"] != 0
                        or type(occlusion.get("quads")) is not int or occlusion["quads"] < 0):
                    raise ValueError("missing or obstructed particle observation")
            receipt = doc["lavaFixture"]
            if receipt != expected or any(type(receipt[k]) is not int for k in ("cells", "matchingCells")):
                raise ValueError("unobserved lava fixture")
            if flowing and any(type(level) is not int for level in receipt["expectedLevels"]):
                raise ValueError("unobserved flowing lava levels")
            captures = doc["captures"]
            if len(captures) != 1 or captures[0]["poseName"] != "initial":
                raise ValueError("unexpected capture pose")
            if (captures[0]["position"] != {"x": 150.5, "y": 100.0, "z": 530.5}
                    or captures[0]["observedYaw"] != 105.0 or captures[0]["observedPitch"] != 10.0
                    or doc["cameraType"] != "FIRST_PERSON" or doc["dimension"] != "minecraft:overworld"):
                raise ValueError("unexpected camera")
            surface = doc["lavaSurfaceAtCapture"]
            if not isinstance(surface, list) or len(surface) != 9:
                raise ValueError("missing nine projected witnesses")
            for point in surface:
                if not isinstance(point, list) or len(point) != 2 or any(
                        type(v) not in (int, float) or not math.isfinite(v) for v in point):
                    raise ValueError("invalid projection")
            if len({tuple(p) for p in surface}) != 9:
                raise ValueError("duplicate witnesses")
            points.append(surface)
        if any(abs(a - b) > 0.01 for p, q in zip(*points) for a, b in zip(p, q)):
            raise ValueError("different observed camera projections")
        patches = []
        for x, y in points[0]:
            x, y = round(x), round(y)
            box = (x - 2, y - 2, x + 3, y + 3)
            if box[0] < 0 or box[1] < 0 or box[2] > 1280 or box[3] > 720:
                raise ValueError("lava witness outside screenshot")
            crops = [image.convert("RGB").crop(box) for image in (baseline, current)]
            orange = [sum(r > 80 and r > g * 1.2 and g > b * 1.2
                          for r, g, b in crop.getdata()) for crop in crops]
            error = ImageStat.Stat(ImageChops.difference(*crops)).mean
            patches.append({"box": box, "lava_pixels": orange, "mean_rgb_abs": error,
                            "passed": min(orange) >= 13 and max(error) <= 6})
        whole = ImageStat.Stat(ImageChops.difference(baseline.convert("RGB"), current.convert("RGB"))).mean
        return {"passed": all(p["passed"] for p in patches) and max(whole) <= 6,
                "patches": patches, "whole_mean_rgb_abs": whole, "tolerance": 6, "flowing": flowing}
    except (KeyError, TypeError, ValueError, IndexError) as error:
        return {"passed": False, "error": str(error)}
