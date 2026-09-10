"""Local, paired ice-overlay witness with an independently captured hidden control."""
import json
from pathlib import Path
from PIL import Image, ImageChops, ImageStat

SCENARIOS = ("water-overlay-hidden", "water-overlay-isolation")
BOX = (300, 400, 430, 620)


def images(baseline, current):
    if baseline.size != (1280, 720) or current.size != baseline.size:
        return {"passed": False, "status": "extent-mismatch"}
    regions = []
    for y0, y1 in zip((400, 455, 510, 565), (455, 510, 565, 620)):
        for x0, x1 in zip((300, 343, 386), (343, 386, 430)):
            box = (x0, y0, x1, y1)
            error = ImageStat.Stat(ImageChops.difference(baseline.crop(box), current.crop(box))).mean
            regions.append({"crop_box": box, "mean_rgb_abs": error, "passed": max(error) <= 6})
    detail = max(ImageStat.Stat(baseline.crop(BOX)).stddev) > 2
    return {"passed": detail and all(r["passed"] for r in regions),
            "baseline_detail_present": detail, "regions": regions}


def effect(hidden, visible):
    if hidden.size != (1280, 720) or visible.size != hidden.size:
        return {"passed": False, "status": "extent-mismatch"}
    before, after = hidden.crop(BOX), visible.crop(BOX)
    change = ImageStat.Stat(ImageChops.difference(before, after)).mean
    # The fixture overlay is green; foliage alone cannot satisfy the required
    # change from an otherwise identical independently captured hidden control.
    green = sum(g > r + 8 and g > b + 8 for r, g, b in after.getdata())
    changed_green = sum(
        g > r + 8 and g > b + 8 and max(abs(a - b) for a, b in zip(old, new)) > 3
        for old, new in zip(before.getdata(), after.getdata())
        for r, g, b in (new,))
    return {"passed": max(change) > 3 and green >= 256 and changed_green >= 256,
            "mean_rgb_change": change, "green_pixels": green,
            "changed_green_pixels": changed_green}


def fixture(document):
    from graphics_harness import water_detail_fixture_matches
    if not isinstance(document, dict):
        return False
    f = document.get("mixedFluidFixture") or {}
    return (water_detail_fixture_matches(document)
        and isinstance(f, dict) and type(f.get("cells")) is int
        and type(f.get("matchingCells")) is int and f.get("complete") is True
        and f == {"fixture": "sealed-mixed-fluid-v1", "variant": "ice",
                  "enclosure": "minecraft:ice", "placement": "146,99,532/west",
                  "cells": 64, "matchingCells": 64, "complete": True}
        and document["captures"][0].get("gameTime") == 6000)


def load_pair(pair):
    from graphics_harness import deterministic_capture_document
    if pair.get("fixture_equivalence", {}).get("status") != "passed":
        raise ValueError("incomparable overlay fixture")
    docs = [deterministic_capture_document(Path(pair[side + "_artifact"])) or {}
            for side in ("baseline", "current")]
    if not all(fixture(doc) for doc in docs):
        raise ValueError("overlay requires complete canonical ice fixture and exact camera/time")
    result = []
    for side in ("baseline", "current"):
        with Image.open(pair[side + "_image"]) as image:
            result.append(image.convert("RGB"))
    return result


def report(visual, scenario, reference=None):
    if scenario not in SCENARIOS:
        return {"passed": True, "requested": False}
    result = {"passed": False, "requested": True, "scenario": scenario}
    try:
        pairs = visual.get("pairs", [])
        if len(pairs) != 1:
            raise ValueError("overlay witness requires exactly one paired pose")
        current = load_pair(pairs[0])
        result["local"] = images(*current)
        if scenario == "water-overlay-hidden":
            result["passed"] = result["local"]["passed"]
            return result
        if reference is None:
            raise ValueError("visible overlay requires an accepted hidden-control pair")
        before = json.loads(Path(reference).read_text())
        control = before.get("water_overlay_parity", {})
        if (before.get("success") is not True or control.get("passed") is not True
                or control.get("scenario") != "water-overlay-hidden"):
            raise ValueError("reference is not an accepted hidden-overlay control")
        prior = before.get("cross_repository_visual_parity", {}).get("pairs", [])
        if len(prior) != 1:
            raise ValueError("hidden control requires exactly one paired pose")
        hidden = load_pair(prior[0])
        result["reference_local"] = images(*hidden)
        result["effects"] = [effect(a, b) for a, b in zip(hidden, current)]
        result["passed"] = (result["local"]["passed"] and result["reference_local"]["passed"]
                            and all(e["passed"] for e in result["effects"]))
    except (OSError, ValueError, KeyError, TypeError) as error:
        result["error"] = str(error)
    return result
