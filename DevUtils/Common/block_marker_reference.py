"""Strict paired proof for real BlockMarker particles using their actual atlas sprite.

Separate from the dedicated-texture marker gate; never substitutes its verdict.
CPU source hashes establish input equivalence, not GPU execution.
"""
import math
import re
from pathlib import Path
from PIL import Image, ImageChops, ImageStat

BOX = (530, 60, 750, 320)


def lighting_inputs(document, game_time):
    captures = document.get("captures", [])
    if len(captures) != 1:
        return False
    expected = ([0,1,1.5,0,0,0,.5,1,1,1,1,1,1] if game_time == 6000
        else [0,.24,1.5,0,0,0,.5,.48000002,.48000002,1,1,1,1] if game_time == 18000 else None)
    try:
        actual = [float(v) for v in captures[0]["lightmapSemanticFingerprint"].split(",")]
        return (expected is not None and len(actual) == len(expected)
            and all(math.isfinite(a) and abs(a-b) < 1e-6 for a,b in zip(actual,expected)))
    except (KeyError, TypeError, ValueError, AttributeError):
        return False


def expected_sprite(scenario):
    if scenario == "barrier":
        return "minecraft:item/barrier"
    if isinstance(scenario, str) and re.fullmatch(r"light-(?:[0-9]|1[0-5])", scenario):
        return f"minecraft:item/light_{int(scenario[6:]):02d}"
    raise ValueError("unsupported marker scenario")


def pack_source_hash(variant):
    import io
    from capture_runner import asymmetric_png, GUI_PACK_COLORS
    if variant not in ("a", "b"):
        raise ValueError("unsupported marker pack variant")
    value = 0xcbf29ce484222325
    with Image.open(io.BytesIO(asymmetric_png(16,16,GUI_PACK_COLORS[variant],variant))) as pixels:
        for byte in pixels.convert("RGBA").tobytes():
            value = ((value ^ byte) * 0x100000001b3) & 0xffffffffffffffff
    return f"{value:016x}"


def ownership(artifact):
    validation = artifact.get("validation", {})
    metrics = artifact.get("metrics", {})
    correlation = artifact.get("capture", {}).get("whole_frame_gameplay_attachments", {}).get("correlation_doc", {})
    gl_calls = metrics.get("rust_gal_slice", {}).get("backend_sync", {}).get("gl_calls")
    return (artifact.get("implementation_attribution") == "rust-vulkan"
        and all(validation.get(k) is True for k in ("complete", "crash_free", "device_loss_free",
            "vulkan_validation_clean", "vulkan_validation_passed", "validation_layer_exercised"))
        and validation.get("rss_guard_triggered") is False
        and validation.get("vanilla_dh_isolation", {}).get("passed") is True
        and type(gl_calls) in (int, float) and gl_calls == 0
        and correlation.get("java_vulkan_frame_execution") is False
        and correlation.get("rust_whole_frame_presenter") is True)


def source_inputs(f):
    source = f.get("sourcePixels", {})
    uv = f.get("localUv", [])
    return (isinstance(source, dict) and source.get("width") == 16 and source.get("height") == 16
        and re.fullmatch(r"[0-9a-f]{16}", str(source.get("rgbaFnv64", ""))) is not None
        and len(uv) == 4 and all(type(v) in (float,int) and math.isfinite(v) for v in uv)
        and all(abs(v-e) < .001 for v,e in zip(uv,(0,1,0,1)))
        and type(f.get("light")) is int and 0 <= f["light"] <= 0x00f000f0)


def local(a, b):
    if a.size != (1280, 720) or b.size != a.size:
        raise ValueError("marker capture must be 1280x720")
    cells = []
    for y in range(BOX[1], BOX[3], 20):
        for x in range(BOX[0], BOX[2], 20):
            box = (x, y, x+20, y+20)
            means = ImageStat.Stat(ImageChops.difference(a.crop(box), b.crop(box))).mean
            cells.append({"box": box, "mean_rgb_abs": means, "passed": max(means) <= 6})
    return {"passed": all(c["passed"] for c in cells), "cells": cells, "tolerance": 6, "box": BOX}


def load(manifest, hidden, scenario="barrier", source_variant=None, game_time=6000):
    from graphics_harness import deterministic_capture_document, water_detail_fixture_matches, read_json
    sprite = expected_sprite(scenario)
    if type(game_time) is not int or game_time not in (6000,18000):
        raise ValueError("marker fixture requires explicit noon or midnight time")
    if manifest.get("success") is not True:
        raise ValueError("capture did not pass existing validation gates")
    pairs = manifest.get("cross_repository_visual_parity", {}).get("pairs", [])
    if len(pairs) != 1 or pairs[0].get("fixture_equivalence", {}).get("status") != "passed":
        raise ValueError("exactly one equivalent paired pose required")
    pair = pairs[0]
    receipts, images = [], []
    for side in ("baseline", "current"):
        artifact = read_json(Path(pair[side+"_artifact"])) or {}
        if side == "current" and not ownership(artifact):
            raise ValueError("missing clean Rust-only ownership evidence")
        doc = deterministic_capture_document(Path(pair[side+"_artifact"])) or {}
        f = doc.get("blockMarkerFixture") or {}
        if source_variant is not None and (f.get("sourcePixels") or {}).get("rgbaFnv64") != pack_source_hash(source_variant):
            raise ValueError("marker source does not match requested pack variant")
        if (f.get("fixture") != "native-block-marker-v1" or f.get("complete") is not True
                or f.get("hidden") is not hidden or f.get("size") != .5
                or f.get("scenario") != scenario or f.get("sprite") != sprite
                or f.get("translucent") is not False or f.get("staticSource") is not True
                or f.get("color") != [1, 1, 1, 0 if hidden else 1]
                or not source_inputs(f)
                or not water_detail_fixture_matches(doc)
                or not lighting_inputs(doc, game_time)
                or doc["captures"][0].get("gameTime") != game_time):
            raise ValueError("missing or incorrect marker fixture")
        pos = f.get("position", [])
        expected = (147.64617596880336, 102.0991813627766, 529.7355156371002)
        if len(pos) != 3 or any(not math.isfinite(v) or abs(v-e)>1e-6 for v,e in zip(pos,expected)):
            raise ValueError("wrong marker position")
        if side == "current":
            frame = doc["captures"][0].get("renderedFrameIndex")
            if type(frame) is not int or frame <= 0:
                raise ValueError("missing captured marker frame identity")
            records = [r for r in doc.get("rustGalWorldTerrainParticles", [])
                if r.get("frameIndex") == frame and r.get("spriteId") == f["sprite"]]
            if len(records) != 1:
                raise ValueError("missing unique native marker producer")
            r = records[0]
            bounds = r.get("screenBounds", {})
            left, top, right, bottom = [bounds.get(k, float("nan")) for k in ("left","top","right","bottom")]
            if (r.get("route") != "rust-vulkan-whole-frame" or r.get("materialMode") != 1
                    or r.get("textureId") != 1419868698
                    or r.get("colorArgb") != (0x00ffffff if hidden else 0xffffffff)
                    or r.get("projected") is not True or r.get("quadSize") != .5
                    or r.get("packedLight") != f.get("light")
                    or r.get("viewport") != {"width":1280,"height":720}
                    or not BOX[0] <= left < right <= BOX[2]
                    or not BOX[1] <= 720-bottom < 720-top <= BOX[3]):
                raise ValueError("unbounded or incorrect native marker producer")
        receipts.append(f)
        with Image.open(pair[side+"_image"]) as image:
            images.append(image.convert("RGB"))
    if receipts[0] != receipts[1]:
        raise ValueError("marker CPU input receipts differ")
    return receipts, images


def report(manifest, hidden_control=None, scenario="barrier", source_variant=None, game_time=6000):
    result = {"passed": False, "scope": scenario, "source_variant": source_variant,
              "game_time":game_time, "hidden": hidden_control is None}
    try:
        receipts, images = load(manifest, hidden_control is None, scenario, source_variant, game_time)
        result["local"] = local(*images)
        result["cpu_input_receipts"] = receipts
        if hidden_control is None:
            result["passed"] = result["local"]["passed"]
            return result
        before, controls = load(hidden_control, True, scenario, source_variant, game_time)
        result["control_local"] = local(*controls)
        for old, new in zip(before, receipts):
            a, b = dict(old), dict(new)
            for value in (a, b):
                value.pop("hidden")
                value["color"] = value["color"][:3]
            if a != b:
                raise ValueError("marker inputs changed beyond visibility")
        differences = [ImageChops.difference(a,b) for a,b in zip(controls,images)]
        result["effects"] = []
        for delta in differences:
            crop = delta.crop(BOX)
            means = ImageStat.Stat(crop).mean
            changed = sum(max(p)>3 for p in crop.getdata())
            result["effects"].append({"changed_pixels": changed, "mean_rgb_change": means,
                "passed": changed >= 32 and max(means) > .1})
        result["effect_parity"] = local(*differences)
        result["passed"] = (result["local"]["passed"] and result["control_local"]["passed"]
            and result["effect_parity"]["passed"] and all(e["passed"] for e in result["effects"]))
    except (ValueError, TypeError, KeyError, IndexError, OSError) as error:
        result["error"] = str(error)
    return result


def replacement_report(before, after, scenario, before_variant, after_variant, game_time=6000):
    """Require an actual A-to-B pixel change in both renderers, not only metadata."""
    result = {"passed": False, "scope": scenario, "transition": [before_variant, after_variant]}
    try:
        if before_variant == after_variant:
            raise ValueError("replacement requires distinct source variants")
        pack_source_hash(before_variant)
        pack_source_hash(after_variant)
        old_receipts, old_images = load(before, False, scenario, before_variant, game_time)
        new_receipts, new_images = load(after, False, scenario, after_variant, game_time)
        for old, new in zip(old_receipts, new_receipts):
            a, b = dict(old), dict(new)
            old_source, new_source = a.pop("sourcePixels"), b.pop("sourcePixels")
            if a != b or old_source["rgbaFnv64"] == new_source["rgbaFnv64"]:
                raise ValueError("replacement changed unrelated marker inputs or retained old source")
        result["before_local"] = local(*old_images)
        result["after_local"] = local(*new_images)
        deltas = [ImageChops.difference(a,b) for a,b in zip(old_images,new_images)]
        result["effects"] = []
        for delta in deltas:
            crop = delta.crop(BOX)
            means = ImageStat.Stat(crop).mean
            changed = sum(max(p)>3 for p in crop.getdata())
            result["effects"].append({"changed_pixels":changed,"mean_rgb_change":means,
                "passed":changed>=32 and max(means)>.1})
        result["effect_parity"] = local(*deltas)
        result["passed"] = (result["before_local"]["passed"] and result["after_local"]["passed"]
            and result["effect_parity"]["passed"] and all(e["passed"] for e in result["effects"]))
    except (ValueError, TypeError, KeyError, IndexError, OSError) as error:
        result["error"] = str(error)
    return result


def reload_evidence(manifest, before_variant, after_variant):
    from graphics_harness import world_resource_reload_report
    pack_source_hash(before_variant)
    pack_source_hash(after_variant)
    removed = "file/mattmc-block-marker-" + before_variant
    retained = "file/mattmc-block-marker-" + after_variant
    proof = world_resource_reload_report(manifest.get("cross_repository_parity", {}), True, removed)
    rows = proof.get("pairs", [])
    valid = before_variant != after_variant and proof.get("passed") is True and len(rows) == 1
    if valid:
        for receipt in rows[0]["receipts"]:
            before, after = receipt["selectedBefore"], receipt["selectedAtCapture"]
            valid = (valid and len(set(before)) == len(before) and retained in before
                and retained in after and removed not in after
                and before.index(retained) < before.index(removed))
    return {"passed":valid,"removed":removed,"retained":retained,"normal_reload":proof}


def main():
    import argparse
    import json
    from graphics_harness import read_json
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--scenario", default="barrier", choices=["barrier"]+[f"light-{i}" for i in range(16)])
    parser.add_argument("--source-variant", choices=["a", "b"], help="Require the exact generated marker texture source.")
    parser.add_argument("--game-time", type=int, choices=[6000,18000], default=6000)
    reference = parser.add_mutually_exclusive_group()
    reference.add_argument("--hidden-control", type=Path)
    reference.add_argument("--replacement-reference", type=Path)
    parser.add_argument("--previous-source-variant", choices=["a", "b"])
    parser.add_argument("--require-reload", action="store_true", help="Require completed in-session pack removal and presentations in both clients.")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.require_reload and not args.replacement_reference:
        parser.error("reload proof requires a replacement reference")
    if args.replacement_reference:
        if not args.previous_source_variant or not args.source_variant:
            parser.error("replacement requires both source variants")
        result = replacement_report(read_json(args.replacement_reference) or {}, read_json(args.manifest) or {},
            args.scenario, args.previous_source_variant, args.source_variant, args.game_time)
        if args.require_reload:
            result["reload"] = reload_evidence(read_json(args.manifest) or {}, args.previous_source_variant, args.source_variant)
            result["passed"] = result["passed"] and result["reload"]["passed"]
    else:
        if args.previous_source_variant:
            parser.error("previous source variant requires a replacement reference")
        result = report(read_json(args.manifest) or {},
            (read_json(args.hidden_control) or {}) if args.hidden_control else None, args.scenario, args.source_variant, args.game_time)
    # New receipt only: never rewrite an original manifest or prior verdict.
    with args.output.open("x") as stream:
        json.dump(result, stream, indent=2)
        stream.write("\n")
    print(json.dumps({"passed": result["passed"], "error": result.get("error"), "output": str(args.output)}))
    return 0 if result["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
