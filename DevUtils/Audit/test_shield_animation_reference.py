import sys
import unittest
from unittest import mock
from pathlib import Path
from PIL import Image

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "Common"))
from shield_animation_reference import (FROZEN_PHASE_FOUR, FROZEN_PHASE_NINE_INTERPOLATED, FROZEN_PHASE_ZERO_INTERPOLATED,
    compare_phase_four_images, compare_phase_images)
from shield_item_reference import BASE_PROBES
import graphics_harness as harness


class ShieldAnimationReferenceTest(unittest.TestCase):
    def fixture(self, interpolated):
        image = Image.new("RGB", (1280, 720))
        for (_, x, y, _), color in zip(BASE_PROBES, FROZEN_PHASE_FOUR[interpolated]):
            image.paste(color, (x-1, y-1, x+2, y+2))
        return image

    def test_both_independent_frozen_anchors_pass_without_admitting_capability(self):
        for interpolated in (False, True):
            source = self.fixture(interpolated)
            result = compare_phase_four_images(source, source, interpolated=interpolated)
            self.assertTrue(result["passed"])
            self.assertFalse(result["capability_admitted"])

    def test_matching_blank_or_wrong_phase_images_fail_frozen_anchor(self):
        blank = Image.new("RGB", (1280, 720))
        for interpolated in (False, True):
            self.assertFalse(compare_phase_four_images(blank, blank, interpolated=interpolated)["passed"])
            wrong = self.fixture(not interpolated)
            self.assertFalse(compare_phase_four_images(wrong, wrong, interpolated=interpolated)["passed"])

    def test_stale_gui_raster_and_missing_held_geometry_fail_independently(self):
        frozen = self.fixture(True)
        for index in (0, 5):
            current = frozen.copy()
            _, x, y, _ = BASE_PROBES[index]
            current.paste((0, 0, 255), (x-1, y-1, x+2, y+2))
            self.assertFalse(compare_phase_four_images(frozen, current, interpolated=True)["passed"])

    def test_region_difference_outside_probes_is_not_ignored(self):
        frozen = self.fixture(False)
        current = frozen.copy()
        current.paste((255, 255, 255), (900, 450, 1100, 590))
        self.assertFalse(compare_phase_four_images(frozen, current, interpolated=False)["passed"])

    def test_repeated_source_phase_has_its_own_frozen_oracle(self):
        repeated = Image.new("RGB", (1280, 720))
        for (_, x, y, _), color in zip(BASE_PROBES, FROZEN_PHASE_NINE_INTERPOLATED):
            repeated.paste(color, (x-1, y-1, x+2, y+2))
        self.assertTrue(compare_phase_images(repeated, repeated, interpolated=True, phase=9)["passed"])
        wrong = self.fixture(True)
        self.assertFalse(compare_phase_images(wrong, wrong, interpolated=True, phase=9)["passed"])
        for phase, interpolated in ((9, False), (0, False), (16, True), (True, True)):
            with self.assertRaises(ValueError):
                compare_phase_images(repeated, repeated, interpolated=interpolated, phase=phase)

    def test_cycle_boundary_uses_the_independent_first_declared_frame_not_sheet_zero(self):
        boundary = Image.new("RGB", (1280, 720))
        for (_, x, y, _), color in zip(BASE_PROBES, FROZEN_PHASE_ZERO_INTERPOLATED):
            boundary.paste(color, (x-1, y-1, x+2, y+2))
        self.assertTrue(compare_phase_images(boundary, boundary, interpolated=True, phase=0)["passed"])
        sheet_zero = self.fixture(False)
        self.assertFalse(compare_phase_images(sheet_zero, sheet_zero, interpolated=True, phase=0)["passed"])

    def test_diagnostic_requires_correlated_right_owner_and_never_admits_whole_animation(self):
        fixture = {"fixture": "held-shield-v1", "selectedSlot": 1,
                   "mainHand": "minecraft:shield", "count": 1, "foil": False,
                   "usingItem": False, "speed": 0.0, "strength": 0.5, "complete": True}
        doc = {"hotbarItemFixture": "shield", "shieldFoilFixture": fixture,
               "blockDisplayAnimationAtCapture": {"frame": 1, "subFrame": 1}}
        pair = {"baseline_artifact": "frozen/artifact.json", "current_artifact": "rust/artifact.json",
                "baseline_image": "frozen.png", "current_image": "rust.png"}
        scenario = "shield-animation-interpolated"
        with mock.patch.object(harness, "deterministic_capture_document", return_value=doc), \
             mock.patch.object(harness, "latest_capture_meta_path", return_value=Path("meta")), \
             mock.patch.object(harness, "read_key_values", return_value={
                 "forced_option_guiScale": "3", "gui_resource_pack_scenario": scenario}), \
             mock.patch.object(harness, "block_display_animation_upload_equivalence") as upload, \
             mock.patch("shield_animation_reference.compare_phase_paths",
                        return_value={"passed": True}) as pixels:
            for evidence in (None, {"passed": False}, {"passed": True, "native": {
                    "texture": 1, "retained_frame": 1, "retained_subframe": 1}}, {"passed": True, "native": {
                    "texture": 4260828946, "retained_frame": 1, "retained_subframe": 0}}):
                upload.return_value = evidence
                result = harness.model_item_foil_parity_report({"pairs": [pair]}, "shield", scenario)
                self.assertFalse(result["phase_diagnostic"]["passed"])
                pixels.assert_not_called()
            upload.return_value = {"passed": True, "native": {
                "texture": 4260828946, "retained_frame": 1, "retained_subframe": 1}}
            result = harness.model_item_foil_parity_report({"pairs": [pair]}, "shield", scenario)
            self.assertTrue(result["phase_diagnostic"]["passed"])
            self.assertFalse(result["passed"])
            self.assertEqual(upload.call_args.kwargs["required_texture"], 4260828946)
            doc["blockDisplayAnimationAtCapture"]["subFrame"] = 2
            result = harness.model_item_foil_parity_report({"pairs": [pair]}, "shield", scenario)
            self.assertFalse(result["phase_diagnostic"]["passed"])
            # An empty catch-up tick may advance the clock but must not
            # replace proof of which pixels the accepted submission retained.
            doc["blockDisplayAnimationAtCapture"].update(frame=2, subFrame=1)
            upload.return_value = {"passed": True, "native": {
                "texture": 4260828946, "frame": 3, "subframe": 0,
                "retained_frame": 2, "retained_subframe": 1}}
            result = harness.model_item_foil_parity_report({"pairs": [pair]}, "shield", scenario)
            self.assertTrue(result["phase_diagnostic"]["passed"])
            self.assertFalse(result["passed"])
            self.assertEqual(pixels.call_args.kwargs["phase"], 9)
            doc["blockDisplayAnimationAtCapture"].update(frame=0, subFrame=0)
            native = upload.return_value["native"]
            native.update(retained_frame=0, retained_subframe=0)
            for tick, passed in ((0, False), (16, False), (17, True), (136, True)):
                native["retained_tick"] = tick
                result = harness.model_item_foil_parity_report({"pairs": [pair]}, "shield", scenario)
                self.assertEqual(result["phase_diagnostic"]["passed"], passed)
                self.assertFalse(result["passed"])


if __name__ == "__main__":
    unittest.main()
