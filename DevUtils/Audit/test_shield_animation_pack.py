"""Shared resource-only animation inputs; never substitute static parity for timing."""
import io
import json
import sys
import tempfile
import unittest
from pathlib import Path
from PIL import Image

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "Common"))
import capture_runner
import graphics_harness


class ShieldAnimationPackTests(unittest.TestCase):
    def test_three_distinct_source_frames_are_not_reordered_to_playback_order(self):
        payload = capture_runner.shield_animation_png()
        self.assertEqual(payload, capture_runner.shield_animation_png())
        with Image.open(io.BytesIO(payload)) as image:
            self.assertEqual(image.size, (64,192))
            for index, color in enumerate(capture_runner.SHIELD_ANIMATION_COLORS):
                self.assertEqual(image.crop((0,index*64,64,(index+1)*64)).getextrema(),
                                 tuple((value,value) for value in color))

    def test_only_texture_and_animation_metadata_change(self):
        for scenario in ("shield-animation", "shield-animation-interpolated"):
            with self.subTest(scenario=scenario), tempfile.TemporaryDirectory() as root:
                spec, = capture_runner.gui_resource_pack_specs(scenario)
                target = Path(root) / "pack"
                capture_runner.write_gui_resource_pack(target, spec)
                texture = "assets/minecraft/textures/entity/shield_base_nopattern.png"
                self.assertEqual({p.relative_to(target).as_posix() for p in target.rglob("*") if p.is_file()},
                                 {"pack.mcmeta",texture,texture+".mcmeta"})
                meta = json.loads((target/(texture+".mcmeta")).read_text())["animation"]
                self.assertEqual((meta["width"],meta["height"]),(64,64))
                self.assertEqual(meta["interpolate"],scenario.endswith("-interpolated"))
                self.assertEqual([(f["index"],f["time"]) for f in meta["frames"]],[(2,3),(0,5),(2,2),(1,7)])
                self.assertEqual((target/texture).read_bytes(),capture_runner.shield_animation_png())

    def test_static_or_missing_pixels_never_admit_animation(self):
        for scenario in ("shield-animation", "shield-animation-interpolated"):
            result = graphics_harness.model_item_foil_parity_report({"pairs":[]},"shield",scenario)
            self.assertTrue(result["requested"])
            self.assertFalse(result["passed"])
            self.assertIn("phase-matched",result["reason"])


if __name__ == "__main__":
    unittest.main()
