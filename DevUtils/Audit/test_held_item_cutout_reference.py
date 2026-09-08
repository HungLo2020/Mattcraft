import sys
import unittest
from pathlib import Path
from PIL import Image
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "Common"))
from held_item_cutout_reference import PROBES, compare_images


class HeldItemCutoutReferenceTest(unittest.TestCase):
    def fixture(self, edges=True):
        image = Image.new("RGB", (1280,720), (175,205,250))
        for name,x,y,color in PROBES:
            if not edges and "edge" in name:
                continue
            for py in range(y-1,y+2):
                for px in range(x-1,x+2):
                    image.putpixel((px,py),color)
        return image

    def test_complete_faces_pass(self):
        self.assertTrue(compare_images(self.fixture(), self.fixture())["passed"])

    def test_missing_edges_fail_with_unchanged_front_controls(self):
        report = compare_images(self.fixture(), self.fixture(False))
        self.assertFalse(report["passed"])
        self.assertTrue(all(p["passed"] for p in report["probes"] if p["name"].startswith("front")))

    def test_matching_wrong_baselines_cannot_hide_missing_faces(self):
        self.assertFalse(compare_images(self.fixture(False), self.fixture(False))["passed"])

    def test_different_capture_extent_is_rejected(self):
        with self.assertRaises(ValueError):
            compare_images(self.fixture(), Image.new("RGB", (640,360)))
