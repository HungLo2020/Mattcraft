import copy
import sys
import tempfile
import unittest
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "Common"))
from lava_parity import lava_surface_evidence, required_lava_report, lava_phase_change_evidence, lava_mip_evidence, lava_mip_sampling_evidence
from PIL import Image


class LavaParityTest(unittest.TestCase):
    def test_mip_sampling_requires_unchanged_source_complete_chains_and_changed_pixels(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            boxes = [[x-2, y-2, x+3, y+3] for x, y in self.fixture()["lavaSurfaceAtCapture"]]
            pairs = []
            for levels, color in ((1, (240, 110, 20)), (5, (220, 100, 20))):
                image = root / f"mips-{levels}.png"
                Image.new("RGB", (1280, 720), color).save(image)
                hashes = [f"{value:016x}" for value in range(1, levels + 1)]
                pairs.append({"status": "complete", "baseline_image": str(image), "current_image": str(image),
                    "lava_surface": {"passed": True, "flowing": False, "patches": [{"box": b} for b in boxes]},
                    "lava_mips": {"passed": True, "levels": levels,
                                  "requested": [levels, levels], "observed": [levels, levels]},
                    "lava_animation": {"passed": True,
                        "native": {"mip_levels": levels, "retained_rgba_fnv64": hashes[0],
                                   "retained_mip_rgba_fnv64": hashes},
                        "frozen": {"mipLevels": levels, "spriteName": "minecraft:block/lava_still", "uploadedRgbaFnv64": hashes[0],
                                   "uploadedMipRgbaFnv64": hashes}}})
            self.assertTrue(lava_mip_sampling_evidence(*pairs)["passed"])
            self.assertFalse(lava_phase_change_evidence(*pairs)["passed"])
            for mode in (None, 0, True):
                bad = copy.deepcopy(pairs)
                bad[1]["lava_surface"]["flowing"] = mode
                self.assertFalse(lava_mip_sampling_evidence(*bad)["passed"])
            flow = copy.deepcopy(pairs)
            for pair in flow:
                pair["lava_surface"]["flowing"] = True
            self.assertFalse(lava_mip_sampling_evidence(*flow)["passed"])
            for pair in flow:
                pair["lava_animation"]["frozen"]["spriteName"] = "minecraft:block/lava_flow"
            self.assertTrue(lava_mip_sampling_evidence(*flow)["passed"])
            for key, value in (("levels", True), ("requested", [True, 1]), ("observed", [1, 1.0])):
                bad = copy.deepcopy(pairs)
                bad[0]["lava_mips"][key] = value
                self.assertFalse(lava_mip_sampling_evidence(*bad)["passed"])
            for key in ("lava_mips", "lava_animation", "lava_surface"):
                bad = copy.deepcopy(pairs)
                bad[1][key]["passed"] = False
                self.assertFalse(lava_mip_sampling_evidence(*bad)["passed"], key)
            for side, key in (("native", "retained_mip_rgba_fnv64"), ("frozen", "uploadedMipRgbaFnv64")):
                bad = copy.deepcopy(pairs)
                bad[1]["lava_animation"][side][key] = ["0000000000000001"]
                self.assertFalse(lava_mip_sampling_evidence(*bad)["passed"])
            bad = copy.deepcopy(pairs)
            bad[1]["lava_animation"]["native"]["retained_rgba_fnv64"] = "0000000000000009"
            bad[1]["lava_animation"]["frozen"]["uploadedRgbaFnv64"] = "0000000000000009"
            self.assertFalse(lava_mip_sampling_evidence(*bad)["passed"])
            pairs[1]["current_image"] = pairs[0]["current_image"]
            self.assertFalse(lava_mip_sampling_evidence(*pairs)["passed"])
            pairs[1]["baseline_image"] = pairs[0]["baseline_image"]
            self.assertFalse(lava_mip_sampling_evidence(*pairs)["passed"])

    def test_particle_observation_cannot_excuse_an_obstructed_or_unobserved_surface(self):
        doc = self.fixture()
        image = Image.new("RGB", (1280, 720), (240, 110, 20))
        clear = {"schema": "fluid-sample-particle-occlusion-v1", "complete": True,
                 "quads": 5, "blockers": 0, "ready": True}
        doc["lavaParticleOcclusionAtCapture"] = clear
        self.assertTrue(lava_surface_evidence(doc, doc, image, image)["passed"])
        for bad in (None, {}, {**clear, "complete": False}, {**clear, "ready": False},
                    {**clear, "blockers": 1}, {**clear, "blockers": False}, {**clear, "quads": -1}):
            other = copy.deepcopy(doc)
            other["lavaParticleOcclusionAtCapture"] = bad
            self.assertFalse(lava_surface_evidence(doc, other, image, image)["passed"])

    def test_mip_admission_requires_both_requested_and_observed_counts(self):
        old = [{"blockDisplayAnimationAtCapture": {"mipLevels": 1}} for _ in range(2)]
        self.assertEqual(1, lava_mip_evidence(old)["levels"])
        docs = [{"blockDisplayAnimationAtCapture": {"mipLevels": 5, "requestedMipLevels": 5}} for _ in range(2)]
        self.assertEqual(5, lava_mip_evidence(docs)["levels"])
        for side in (0, 1):
            for key in ("mipLevels", "requestedMipLevels"):
                for invalid in (None, 0, 1, True, 5.0, "5", 33):
                    bad = copy.deepcopy(docs)
                    bad[side]["blockDisplayAnimationAtCapture"][key] = invalid
                    self.assertFalse(lava_mip_evidence(bad)["passed"], (side, key, invalid))
            bad = copy.deepcopy(docs)
            del bad[side]["blockDisplayAnimationAtCapture"]["requestedMipLevels"]
            self.assertFalse(lava_mip_evidence(bad)["passed"])
        self.assertFalse(lava_mip_evidence([])["passed"])

    def test_flowing_lava_requires_its_own_observed_levels_and_mode(self):
        image = Image.new("RGB", (1280, 720), (240, 110, 20))
        doc = self.fixture()
        self.assertFalse(lava_surface_evidence(doc, doc, image, image, flowing=True)["passed"])
        doc["lavaFixture"] = {"fixture": "single-source-lava-flow-v1", "origin": "145, 98, 530",
            "cells": 90, "matchingCells": 90, "complete": True,
            "direction": "west", "expectedLevels": [0, 2, 4, 6]}
        self.assertTrue(lava_surface_evidence(doc, doc, image, image, flowing=True)["passed"])
        self.assertFalse(lava_surface_evidence(doc, doc, image, image)["passed"])
        for levels in ([0, 1, 2, 3], [0, 2, 4], [False, 2, 4, 6]):
            bad = copy.deepcopy(doc)
            bad["lavaFixture"]["expectedLevels"] = levels
            self.assertFalse(lava_surface_evidence(doc, bad, image, image, flowing=True)["passed"])
        pair = {"status": "complete", "lava_surface": {"passed": True, "flowing": False},
                "lava_animation": {"passed": True}}
        self.assertFalse(required_lava_report({"pairs": [pair]}, True, flowing=True)["passed"])
        pair["lava_surface"]["flowing"] = True
        self.assertTrue(required_lava_report({"pairs": [pair]}, True, flowing=True)["passed"])

    def test_phase_change_requires_real_matching_pixel_changes(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            old = Image.new("RGB", (1280, 720), (240, 110, 20))
            new = Image.new("RGB", (1280, 720), (200, 90, 10))
            for name, image in (("old", old), ("new", new), ("current", new)):
                image.save(root / (name + ".png"))
            boxes = [[x-2, y-2, x+3, y+3] for x,y in self.fixture()["lavaSurfaceAtCapture"]]
            def pair(left, right, digest):
                return {"status": "complete", "baseline_image": str(root / (left + ".png")),
                        "current_image": str(root / (right + ".png")),
                        "lava_surface": {"passed": True, "patches": [{"box": b} for b in boxes]},
                        "lava_animation": {"passed": True,
                            "frozen": {"uploadedRgbaFnv64": digest},
                            "native": {"retained_rgba_fnv64": digest}}}
            before = pair("old", "old", "0000000000000001")
            after = pair("new", "current", "0000000000000002")
            self.assertTrue(lava_phase_change_evidence(before, after)["passed"])
            self.assertFalse(lava_phase_change_evidence(before, before)["passed"])
            old.save(root / "current.png")
            self.assertFalse(lava_phase_change_evidence(before, after)["passed"])
            partial = old.copy()
            partial.paste(new.crop(tuple(boxes[0])), tuple(boxes[0]))
            partial.save(root / "current.png")
            self.assertFalse(lava_phase_change_evidence(before, after)["passed"])
            Image.new("RGB", old.size, (255, 130, 40)).save(root / "current.png")
            self.assertFalse(lava_phase_change_evidence(before, after)["passed"])

    def test_required_lava_cannot_pass_with_missing_or_failed_evidence(self):
        self.assertTrue(required_lava_report({}, False)["passed"])
        for report in ({}, {"pairs": []}, {"pairs": None},
                       {"pairs": [{"status": "complete", "lava_surface": None}]},
                       {"pairs": [{"status": "complete"}]}):
            self.assertFalse(required_lava_report(report, True)["passed"])
        pair = {"status": "complete", "lava_surface": {"passed": True}, "lava_animation": {"passed": True}}
        self.assertTrue(required_lava_report({"pairs": [pair]}, True)["passed"])
        for key in ("lava_surface", "lava_animation"):
            bad = copy.deepcopy(pair)
            bad[key]["passed"] = False
            self.assertFalse(required_lava_report({"pairs": [bad]}, True)["passed"])
    def fixture(self):
        return {"lavaFixture": {"fixture": "sealed-lava-basin-v2", "origin": "145, 99, 530",
                               "cells": 75, "matchingCells": 75, "complete": True},
                "lavaSurfaceAtCapture": [[x, y] for x in (610, 630, 650) for y in (410, 430, 450)],
                "captures": [{"poseName": "initial", "position": {"x": 150.5, "y": 100.0, "z": 530.5},
                              "observedYaw": 105.0, "observedPitch": 10.0}],
                "cameraType": "FIRST_PERSON", "dimension": "minecraft:overworld"}

    def test_each_missing_or_wrong_surface_is_rejected(self):
        doc = self.fixture()
        image = Image.new("RGB", (1280, 720), (240, 110, 20))
        self.assertTrue(lava_surface_evidence(doc, doc, image, image)["passed"])
        for index, (x, y) in enumerate(doc["lavaSurfaceAtCapture"]):
            changed = image.copy()
            changed.paste((0, 0, 0), (x - 2, y - 2, x + 3, y + 3))
            self.assertFalse(lava_surface_evidence(doc, doc, image, changed)["passed"], index)
            shifted = copy.deepcopy(doc)
            shifted["lavaSurfaceAtCapture"][index][0] += 1
            self.assertFalse(lava_surface_evidence(doc, shifted, image, image)["passed"], index)
        black = Image.new("RGB", image.size)
        self.assertFalse(lava_surface_evidence(doc, doc, black, black)["passed"])

    def test_unobserved_fixture_and_invalid_points_fail(self):
        image = Image.new("RGB", (1280, 720), (240, 110, 20))
        doc = self.fixture()
        for field, value in (("complete", False), ("matchingCells", 26), ("cells", True)):
            bad = copy.deepcopy(doc)
            bad["lavaFixture"][field] = value
            self.assertFalse(lava_surface_evidence(doc, bad, image, image)["passed"])
        for points in (None, [], [[600, 400]] * 9, [[float("nan"), 400]] * 9):
            bad = copy.deepcopy(doc)
            bad["lavaSurfaceAtCapture"] = points
            self.assertFalse(lava_surface_evidence(doc, bad, image, image)["passed"])


if __name__ == "__main__":
    unittest.main()
