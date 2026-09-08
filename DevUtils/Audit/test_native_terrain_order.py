import copy
import unittest
from DevUtils.Common.native_terrain_order import POSES, validate_order_receipts


def receipts():
    positions = [(0, 0, 0), (0, 0, -4), (0, 0, 0), (-10, 0, 0), (-4, 5, 0), (-4, -4, 0), (0, 0, 0)]
    captures, acks = [], []
    for index, (pose, position) in enumerate(zip(POSES, positions), 1):
        capture = {"index": index, "poseName": pose, "screenshot": f"/fixture/{index}.png",
                   "renderedFrameIndex": index + 20, "position": dict(zip(("x", "y", "z"), position))}
        transform = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, -position[0], -position[1], -position[2], 1]
        order = {"schema": "mattmc-static-terrain-batch-trace-v1", "frameId": index + 40,
                 "cameraSortedQuads": True, "orderedMeshRangesComplete": True,
                 "selection": "requested-mesh-key", "meshIndexCount": 12, "indexType": "U32",
                 "orderedMeshRanges": [[24, 6], [0, 6]] if index == 4 else [[0, 6], [24, 6]],
                 "instanceTransform": transform, "meshKey": "0000000000000123", "meshGeneration": 7}
        ack = {**capture, "status": "captured", "nativeTerrainOrder": order,
               "wholeFramePresentationCorrelation": {"gameplayFrameId": index + 40, "correlationId": index + 40,
                   "submissionId": index + 100, "acquiredSwapchainImage": 9, "presentedSwapchainImage": 9}}
        captures.append(capture)
        acks.append(ack)
    return captures, acks


class NativeTerrainOrderTests(unittest.TestCase):
    def test_complete_camera_crossing_is_proven_by_presented_draw_ranges(self):
        captures, acks = receipts()
        self.assertTrue(validate_order_receipts(captures, acks)["passed"])
        for ack in acks:
            ack["nativeTerrainOrder"]["indexType"] = "U16"
            for row in ack["nativeTerrainOrder"]["orderedMeshRanges"]:
                row[0] //= 2
        self.assertTrue(validate_order_receipts(captures, acks)["passed"])

    def test_every_missing_stale_or_incomplete_pose_fails(self):
        captures, acks = receipts()
        for index in range(7):
            for key, value in [("cameraSortedQuads", False), ("orderedMeshRangesComplete", False),
                               ("frameId", 1), ("meshGeneration", 8), ("meshIndexCount", 18),
                               ("orderedMeshRanges", [[0, 6], [0, 6]]),
                               ("orderedMeshRanges", [[0, 3], [12, 9]]),
                               ("orderedMeshRanges", [[0, 12], [24, 6]]),
                               ("instanceTransform", [float("nan")] * 16)]:
                bad = copy.deepcopy(acks)
                bad[index]["nativeTerrainOrder"][key] = value
                self.assertFalse(validate_order_receipts(captures, bad)["passed"], (index, key))
            self.assertFalse(validate_order_receipts(captures, acks[:index] + acks[index + 1:])["passed"])

    def test_unpresented_or_uncorrelated_receipt_cannot_replace_java_index_evidence(self):
        captures, acks = receipts()
        for key, value in [("submissionId", 0), ("gameplayFrameId", 1), ("presentedSwapchainImage", 8)]:
            bad = copy.deepcopy(acks)
            bad[0]["wholeFramePresentationCorrelation"][key] = value
            self.assertFalse(validate_order_receipts(captures, bad)["passed"])
        for index in [3, 6]:
            bad = copy.deepcopy(acks)
            bad[index]["nativeTerrainOrder"]["orderedMeshRanges"].reverse()
            self.assertFalse(validate_order_receipts(captures, bad)["passed"])
        bad = copy.deepcopy(acks)
        bad[1]["nativeTerrainOrder"]["instanceTransform"][12] += 1
        self.assertFalse(validate_order_receipts(captures, bad)["passed"])


if __name__ == "__main__":
    unittest.main()
