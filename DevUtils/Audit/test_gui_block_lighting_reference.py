import unittest
import sys
from pathlib import Path
from PIL import Image
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "Common"))
from gui_block_lighting_reference import compare


class GuiBlockLightingReferenceTest(unittest.TestCase):
    def fixture(self, top=200):
        image = Image.new("RGB", (1280, 720))
        for slot in (0, 6):
            center = 640 + (slot - 4) * 60
            for dx, dy, color in ((0, -14, top), (-3, -8, 130), (3, -8, 100)):
                for x in range(center + dx * 3 - 1, center + dx * 3 + 2):
                    for y in range(720 + dy * 3 - 1, 720 + dy * 3 + 2):
                        image.putpixel((x, y), (color,) * 3)
        return image

    def test_matching_bright_tops_pass(self):
        self.assertTrue(compare(self.fixture(), self.fixture(), 3)["passed"])

    def test_dark_tops_fail_even_if_both_backends_match(self):
        self.assertFalse(compare(self.fixture(80), self.fixture(80), 3)["passed"])

    def test_wrong_current_light_space_fails(self):
        self.assertFalse(compare(self.fixture(), self.fixture(80), 3)["passed"])

    def test_close_colors_do_not_hide_wrong_orientation(self):
        self.assertFalse(compare(self.fixture(131), self.fixture(129), 3)["passed"])

    def test_mismatched_extents_rejected(self):
        with self.assertRaises(ValueError):
            compare(self.fixture(), Image.new("RGB", (640, 360)), 3)

    def test_log_variant_has_three_additional_face_checks(self):
        report = compare(self.fixture(), self.fixture(), 3, include_logs=True)
        self.assertEqual(3, len([p for p in report["probes"] if p["item"] == "oak_log" ]))

    def test_shared_harness_admits_explicit_log_fixture(self):
        from graphics_harness import parse_args
        args = parse_args(["capture", "--hotbar-item-fixture", "standard-3d-logs"])
        self.assertEqual("standard-3d-logs", args.hotbar_item_fixture)

    def test_required_gate_rejects_missing_pairs_and_evidence(self):
        from graphics_harness import gui_block_lighting_parity_report as report
        self.assertFalse(report({}, "standard-3d-logs")["passed"])
        self.assertFalse(report({"pairs": [{}]}, "standard-3d-logs")["passed"])
        self.assertFalse(report({}, "standard-3d")["requested"])

    def test_required_gate_checks_actual_images_and_observed_scale(self):
        import tempfile
        from unittest.mock import patch
        import graphics_harness as harness
        with tempfile.TemporaryDirectory() as temporary:
            image = Path(temporary) / "image.png"
            self.fixture().save(image)
            pair = dict(baseline_image=str(image), current_image=str(image),
                        baseline_artifact="baseline/artifact.json", current_artifact="current/artifact.json")
            with patch.object(harness, "latest_capture_meta_path", return_value=Path("meta.txt")), \
                 patch.object(harness, "read_key_values", return_value={"forced_option_guiScale": "3"}):
                self.assertTrue(harness.gui_block_lighting_parity_report({"pairs": [pair]}, "standard-3d-logs")["passed"])
                self.fixture(80).save(image)
                self.assertFalse(harness.gui_block_lighting_parity_report({"pairs": [pair]}, "standard-3d-logs")["passed"])
