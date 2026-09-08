"""Read-only attribution of alpha-fixture differences; never a parity waiver."""
from __future__ import annotations

import re

ITEMS = ("apple", "feather", "paper", "diamond", "iron_ingot", "stick", "redstone", "arrow", "coal")
STEPS = ((6, 0), (12, 25), (18, 26), (30, 128), (39, 255))
SAMPLE = re.compile(
    r"FrozenCelestialStagePixel frame=(\d+) stage=(\S+) screen=\((\d+), (\d+)\) "
    r"texture=\d+ rgba=\((\d+),(\d+),(\d+),(\d+)\)"
)


def probe_points():
    return {(376 + 60 * item + center + dx, y)
            for item in range(9) for center, _ in STEPS
            for y in range(674, 677) for dx in range(-1, 2)}


def read_stage_samples(lines, frame, stage):
    if type(frame) is not int or frame <= 0 or stage not in ("main-after-post", "main-after-gui", "window-after-blit"):
        raise ValueError("require a positive correlated frame and known stage")
    points = probe_points()
    result = {}
    for line in lines:
        match = SAMPLE.search(line)
        if not match or int(match[1]) != frame or match[2] != stage:
            continue
        point = (int(match[3]), int(match[4]))
        color = tuple(map(int, match.group(5, 6, 7, 8)))
        if point not in points or point in result or any(channel > 255 for channel in color):
            raise ValueError("invalid or duplicated correlated background sample")
        result[point] = color[:3]
    if result.keys() != points:
        raise ValueError("missing correlated background samples")
    return result


def compare_background_samples(reference_world, reference_gui, current_world, current_gui, reference_image):
    """Keep pre-GUI and presented differences separate, without correcting either."""
    points = probe_points()
    if reference_world.keys() != points or reference_gui.keys() != points:
        raise ValueError("incomplete reference sample sets")
    if any(image.size != (1280, 720) for image in (current_world, current_gui, reference_image)):
        raise ValueError("background fixture requires native 1280x720 images")
    images = [image.convert("RGB") for image in (current_world, current_gui, reference_image)]
    world, gui, reference = images
    if any(reference.getpixel(point) != reference_gui[point] for point in points):
        raise ValueError("Frozen post-GUI observations do not match the captured presentation")
    rows = []
    for item, name in enumerate(ITEMS):
        for center, alpha in STEPS:
            patch = [(376 + 60 * item + center + dx, y)
                     for y in range(674, 677) for dx in range(-1, 2)]
            before = [tuple(a-b for a,b in zip(world.getpixel(p), reference_world[p])) for p in patch]
            after = [tuple(a-b for a,b in zip(gui.getpixel(p), reference_gui[p])) for p in patch]
            rows.append({"item": name, "source_alpha": alpha,
                         "world_mean_rgb_abs": [sum(abs(d[c]) for d in before)/9 for c in range(3)],
                         "presented_mean_rgb_abs": [sum(abs(d[c]) for d in after)/9 for c in range(3)],
                         "pixels": [{"point": list(p), "world_delta_rgb": list(b),
                                     "presented_delta_rgb": list(a)} for p,b,a in zip(patch,before,after)]})
    return {"diagnostic_only": True, "reference_presentation_verified": True, "patches": rows}


def validate_attachment_correlation(attachments, correlation, frame):
    keys = ("gameplay_frame_id", "correlation_id", "deterministic_rendered_frame_index", "gal_submission_id")
    if any(type(attachments.get(key)) is not int or attachments[key] <= 0
           or attachments[key] != correlation.get(key) for key in keys):
        raise ValueError("attachment and presentation identities differ")
    if (attachments["deterministic_rendered_frame_index"] != frame
        or attachments.get("synthetic_shader_scene") is not False
        or attachments.get("java_iris_participation") is not False
        or attachments.get("png_row_origin") != "top-left"
        or correlation.get("java_vulkan_frame_execution") is not False
        or correlation.get("rust_whole_frame_presenter") is not True
        or correlation.get("same_acquired_presented_image") is not True):
        raise ValueError("background observation is not the correlated exclusive Rust frame")


def validate_modes(reference, current):
    for artifact, mode, attribution in (
        (reference, "frozen-opengl-shaders-off", "java-opengl"),
        (current, "current-rust-vulkan-shaders-off", "rust-vulkan"),
    ):
        if artifact.get("mode", {}).get("name") != mode or artifact.get("implementation_attribution") != attribution:
            raise ValueError("background diagnostic requires Frozen Java OpenGL and exclusive vanilla Rust Vulkan")


def main():
    import argparse
    import json
    from pathlib import Path
    from PIL import Image, ImageChops
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--artifact-dir", type=Path, required=True)
    args = parser.parse_args()
    manifest = json.loads((args.artifact_dir / "graphics_audit_manifest.json").read_text())
    pairs = manifest["cross_repository_visual_parity"]["pairs"]
    if len(pairs) != 1 or pairs[0]["fixture_equivalence"]["status"] != "passed":
        raise ValueError("require one fixture-equivalent real image pair")
    pair = pairs[0]
    validate_modes(json.loads(Path(pair["baseline_artifact"]).read_text()),
                   json.loads(Path(pair["current_artifact"]).read_text()))
    frozen_frame = pair["fixture_equivalence"]["baseline"]["poses"][0]["frame"]
    current_frame = pair["fixture_equivalence"]["current"]["poses"][0]["frame"]
    def one(root, pattern):
        paths = list(root.glob(pattern))
        if len(paths) != 1:
            raise ValueError(f"require exactly one {pattern} in {root}")
        return paths[0]
    frozen_capture = Path(pair["baseline_artifact"]).parent / "capture"
    current_capture = Path(pair["current_artifact"]).parent / "capture"
    log = one(frozen_capture, "runClient_*.log")
    with log.open() as lines:
        before = read_stage_samples(lines, frozen_frame, "main-after-post")
    with log.open() as lines:
        after = read_stage_samples(lines, frozen_frame, "main-after-gui")
    root = current_capture / "whole_frame_gameplay_attachments"
    attachments = json.loads(one(root,"gameplay-attachments-frame-*.json").read_text())
    correlation = json.loads(one(root,"gameplay-correlation-frame-*.json").read_text())
    validate_attachment_correlation(attachments,correlation,current_frame)
    with Image.open(root / "attachment-world_final_pre_gui.png") as world, \
         Image.open(root / "attachment-final_output.png") as final, \
         Image.open(pair["current_image"]) as current, Image.open(pair["baseline_image"]) as frozen:
        if final.size != current.size or ImageChops.difference(final.convert("RGB"),current.convert("RGB")).getbbox():
            raise ValueError("Rust attachment does not match the captured presentation")
        report = compare_background_samples(before,after,world,final,frozen)
    report.update(frozen_frame=frozen_frame, rust_frame=current_frame,
                  fixture_equivalent=True, rust_presentation_verified=True)
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
