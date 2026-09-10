"""Resource-only shield alpha fixture, shared by both capture clients."""
import io
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from PIL import Image

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "Common"))
import capture_runner
import shield_alpha_reference
import graphics_harness


class ShieldAlphaPackTests(unittest.TestCase):
    def test_local_gate_rejects_unmatched_observed_pack(self):
        receipt = dict(fixture="held-shield-v1",selectedSlot=1,mainHand="minecraft:shield",
            count=1,foil=False,usingItem=False,speed=0.0,strength=0.5,complete=True)
        pair = dict(baseline_artifact="frozen.json",current_artifact="current.json")
        with mock.patch.object(graphics_harness,"deterministic_capture_document",
                return_value=dict(hotbarItemFixture="shield",shieldFoilFixture=receipt)), \
             mock.patch.object(graphics_harness,"latest_capture_meta_path",return_value=Path("meta")), \
             mock.patch.object(graphics_harness,"read_key_values",
                return_value=dict(forced_option_guiScale="3",gui_resource_pack_scenario="vanilla")):
            report = graphics_harness.model_item_foil_parity_report(
                dict(pairs=[pair]),"shield","shield-alpha-occlusion")
        self.assertFalse(report["passed"])
        self.assertIn("matching observed resource-pack", report["pairs"][0]["reason"])

    def test_reference_rejects_handle_leak_and_identical_empty_frames(self):
        baseline = Image.new("RGB", (1280,720))
        for _, x, y, colors in shield_alpha_reference.PROBES:
            for i, color in enumerate(colors):
                baseline.putpixel((x-1+i%3,y-1+i//3), color)
        self.assertTrue(shield_alpha_reference.compare_images(baseline,baseline)["passed"])
        wrong = baseline.copy()
        wrong.paste((37,222,74),(396,682,399,685))
        self.assertFalse(shield_alpha_reference.compare_images(baseline,wrong)["passed"])
        empty = Image.new("RGB", (1280,720))
        self.assertFalse(shield_alpha_reference.compare_images(empty,empty)["passed"])

    def test_occlusion_control_keeps_only_handle_opaque(self):
        with Image.open(io.BytesIO(capture_runner.shield_alpha_png(zero=True, opaque_handle=True))) as image:
            for y in range(64):
                for x in range(64):
                    self.assertEqual(image.getpixel((x,y)),
                        (40,240,80,255) if 26 <= x < 42 and y < 12 else (224,160,80,0))

    def test_zero_alpha_control_preserves_nonzero_rgb_everywhere(self):
        with Image.open(io.BytesIO(capture_runner.shield_alpha_png(zero=True))) as image:
            self.assertEqual(image.getextrema(), ((224, 224), (160, 160), (80, 80), (0, 0)))
        spec, = capture_runner.gui_resource_pack_specs("shield-alpha-zero")
        self.assertTrue(spec["shield_alpha_zero"])

    def test_bands_include_zero_and_both_sides_of_cutout_threshold(self):
        payload = capture_runner.shield_alpha_png()
        self.assertEqual(payload, capture_runner.shield_alpha_png())
        with Image.open(io.BytesIO(payload)) as image:
            self.assertEqual(image.size, (64, 64))
            self.assertEqual(image.mode, "RGBA")
            for y in range(64):
                for x in range(64):
                    self.assertEqual(image.getpixel((x, y)),
                                     (224, 160, 80, (0, 1, 25, 26, 128, 255)[(y // 4) % 6]))

    def test_pack_changes_only_static_shield_texture(self):
        spec, = capture_runner.gui_resource_pack_specs("shield-alpha")
        with tempfile.TemporaryDirectory() as root:
            target = Path(root) / "pack"
            capture_runner.write_gui_resource_pack(target, spec)
            paths = {p.relative_to(target).as_posix() for p in target.rglob("*") if p.is_file()}
            texture = "assets/minecraft/textures/entity/shield_base_nopattern.png"
            self.assertEqual(paths, {"pack.mcmeta", texture, texture + ".mcmeta"})
            self.assertEqual((target / texture).read_bytes(), capture_runner.shield_alpha_png())
            self.assertEqual((target / (texture + ".mcmeta")).read_text(), "{}")


if __name__ == "__main__":
    unittest.main()
