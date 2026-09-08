"""Validate Rust-owned draw order bound to acknowledged camera captures."""
from __future__ import annotations

import json
import math
from pathlib import Path

POSES = ("translucent-front", "translucent-lateral", "translucent-orbit-left",
         "translucent-cross-opposite", "translucent-above", "translucent-below", "translucent-return")


def positive_int(value):
    return type(value) is int and value > 0


def validate_order_receipts(captures, acknowledgements):
    result = {"passed": False, "schema": "native-terrain-camera-order-evidence-v1"}
    if (not isinstance(captures, list) or len(captures) != len(POSES)
            or len(acknowledgements) != len(POSES)):
        return {**result, "reason": "missing-seven-pose-receipts"}
    orders, frames, origins, cameras, identities = [], [], [], [], []
    try:
        for index, (capture, ack, pose) in enumerate(zip(captures, acknowledgements, POSES), 1):
            if (capture["index"] != index or capture["poseName"] != pose
                    or ack["index"] != index or ack["poseName"] != pose
                    or ack["status"] != "captured" or ack["screenshot"] != capture["screenshot"]
                    or not positive_int(capture["renderedFrameIndex"])
                    or ack["renderedFrameIndex"] != capture["renderedFrameIndex"]
                    or ack["position"] != capture["position"]):
                raise ValueError("acknowledged-capture-mismatch")
            presented = ack["wholeFramePresentationCorrelation"]
            order = ack["nativeTerrainOrder"]
            if (not all(positive_int(presented[key]) for key in
                        ("gameplayFrameId", "correlationId", "submissionId", "acquiredSwapchainImage", "presentedSwapchainImage"))
                    or presented["acquiredSwapchainImage"] != presented["presentedSwapchainImage"]
                    or order["schema"] != "mattmc-static-terrain-batch-trace-v1"
                    or order["frameId"] != presented["gameplayFrameId"]
                    or order["cameraSortedQuads"] is not True
                    or order["orderedMeshRangesComplete"] is not True
                    or order["selection"] != "requested-mesh-key"):
                raise ValueError("unproven-presented-native-order")
            count = order["meshIndexCount"]
            stride = {"U16": 2, "U32": 4}[order["indexType"]]
            if not positive_int(count) or count % 6 or count * stride > 393216:
                raise ValueError("invalid-index-count")
            ranges = order["orderedMeshRanges"]
            if not isinstance(ranges, list) or not 0 < len(ranges) <= 4096:
                raise ValueError("invalid-range-count")
            quads = []
            for offset, length in ranges:
                if (type(offset) is not int or offset < 0 or offset % (6 * stride)
                        or not positive_int(length) or length % 6 or offset + length * stride > count * stride):
                    raise ValueError("invalid-quad-range")
                quads.extend(range(offset, offset + length * stride, 6 * stride))
            if sorted(quads) != list(range(0, count * stride, 6 * stride)):
                raise ValueError("missing-or-duplicated-quads")
            transform = order["instanceTransform"]
            if (not isinstance(transform, list) or len(transform) != 16
                    or any(type(v) not in (int, float) or not math.isfinite(v) for v in transform)
                    or any(transform[i] != (1 if i in (0, 5, 10, 15) else 0)
                           for i in range(16) if i not in (12, 13, 14))):
                raise ValueError("invalid-camera-transform")
            position = [capture["position"][axis] for axis in ("x", "y", "z")]
            if any(type(v) not in (int, float) or not math.isfinite(v) for v in position):
                raise ValueError("invalid-camera-position")
            origins.append([transform[12 + i] + position[i] for i in range(3)])
            cameras.append(tuple(position))
            key, generation = order["meshKey"], order["meshGeneration"]
            if not isinstance(key, str) or len(key) != 16 or int(key, 16) == 0 or not positive_int(generation):
                raise ValueError("invalid-mesh-identity")
            identities.append((key, generation, stride, count))
            frames.append(order["frameId"])
            orders.append(tuple(quads))
        if len(set(identities)) != 1:
            raise ValueError("cross-generation-order-comparison")
        if any(abs(v - origins[0][axis]) > 0.0001 for origin in origins for axis, v in enumerate(origin)):
            raise ValueError("camera-transform-not-correlated")
        if any(a >= b for a, b in zip(frames, frames[1:])):
            raise ValueError("nonmonotonic-presented-frames")
        if (len(set(cameras)) < 5 or max(p[1] for p in cameras) - min(p[1] for p in cameras) < 8
                or math.hypot(max(p[0] for p in cameras) - min(p[0] for p in cameras),
                              max(p[2] for p in cameras) - min(p[2] for p in cameras)) < 8):
            raise ValueError("camera-did-not-cross-fixture")
        if orders[0] == orders[3] or orders[0] != orders[-1] or cameras[0] != cameras[-1]:
            raise ValueError("stale-or-nonrepeatable-order")
    except (KeyError, TypeError, ValueError, IndexError) as error:
        return {**result, "reason": str(error)}
    return {**result, "passed": True, "mesh_key": identities[0][0], "mesh_generation": identities[0][1],
            "quad_count": len(orders[0]), "presented_frames": frames, "distinct_orders": len(set(orders))}


def capture_ack(capture):
    path = Path(capture["screenshot"]).with_name(f"capture_request_{capture['index']:02d}_{capture['poseName']}.ack.json")
    with path.open("rb") as stream:
        data = stream.read(1024 * 1024 + 1)
    if len(data) > 1024 * 1024:
        raise ValueError("oversized-capture-receipt")
    return json.loads(data)


def native_camera_order_evidence(document):
    captures = document.get("captures", []) if isinstance(document, dict) else []
    acknowledgements = []
    try:
        for capture in captures[:len(POSES)]:
            acknowledgements.append(capture_ack(capture))
    except (OSError, KeyError, TypeError, ValueError) as error:
        return {"passed": False, "reason": str(error)}
    return validate_order_receipts(captures, acknowledgements)
