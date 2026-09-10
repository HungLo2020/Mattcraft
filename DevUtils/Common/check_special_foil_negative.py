"""Recheck a real fault-injected Rust capture against a retained Frozen capture.

This never admits a rendering capability or rewrites the capture manifest.
The dedicated pixel gate must reject every foil icon, with valid source/timing
evidence and an unchanged control. Ordinary validation must remain clean.
"""
import argparse
import json
from pathlib import Path

import graphics_harness as h
from gui_special_foil_reference import report


def negative_pixels_rejected(rejected):
    pairs = rejected.get("pairs", [])
    if len(pairs) != 1: return False
    row = pairs[0]
    probes = row.get("probes", [])
    return (rejected.get("passed") is False
        and row.get("actual_fixture_sources_and_timing_verified") is True
        and len(probes) == 9 and [p.get("slot") for p in probes] == list(range(1,10))
        and probes[0].get("passed") is True
        and all(p.get("passed") is False and p.get("visible") is True for p in probes[1:]))


def check(reference, candidate, fault):
    prior = h.read_json(reference/h.MANIFEST_NAME)
    if prior.get("success") is not True:
        raise ValueError("negative control requires an accepted positive reference")
    checked = report(prior["cross_repository_visual_parity"], "special-foil")
    if not checked["passed"] or len(checked["pairs"]) != 1:
        raise ValueError("positive reference no longer passes")
    baseline = Path(prior["cross_repository_visual_parity"]["pairs"][0]["baseline_artifact"])
    current = candidate/"current-rust-vulkan-shaders-off/capture/run-01/graphics_audit_artifact.json"
    doc = h.read_json(current)
    validation = doc.get("validation", {})
    if (any(validation.get(key) is not True for key in
            ("complete", "crash_free", "device_loss_free", "vulkan_validation_clean"))
            or validation.get("messages") != []):
        raise ValueError("fault capture must render normally with clean validation")
    logs = list((current.parent/"capture").glob("runClient_*.log"))
    marker = "special-foil-negative-control="+fault
    if len(logs) != 1 or logs[0].read_text(errors="replace").count(marker) != 1:
        raise ValueError("missing unique actual native mutation observation")
    parity = h.cross_repository_parity_report([baseline, current])
    if parity.get("passed") is not True:
        raise ValueError("negative-control fixture conditions differ")
    out = candidate/"negative-control"
    out.mkdir(exist_ok=True)
    visual = h.write_cross_repo_visual_pairs(out, parity, 6.0)
    rejected = report(visual, "special-foil")
    if len(rejected["pairs"]) != 1:
        raise ValueError("negative control needs one actual paired frame")
    accepted_negative = negative_pixels_rejected(rejected)
    result = dict(schema="special-foil-real-negative-control-v1", fault=fault,
                  passed=accepted_negative, reference=str(reference), candidate=str(candidate),
                  fixture_equivalent=True, validation_clean=True, pixel_gate=rejected)
    target = out/"negative-control.json"
    with target.open("x") as stream:
        json.dump(result, stream, indent=2, sort_keys=True)
    return result


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("reference", type=Path)
    parser.add_argument("candidate", type=Path)
    parser.add_argument("fault", choices=("zero-contribution", "standard-projection"))
    args = parser.parse_args()
    result = check(args.reference.resolve(), args.candidate.resolve(), args.fault)
    print(json.dumps(result))
    raise SystemExit(0 if result["passed"] else 1)
