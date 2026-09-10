import copy
import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch
from PIL import Image
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "Common"))
import capture_runner
import water_overlay_reference as overlay


class WaterOverlayTest(unittest.TestCase):
    def test_both_pack_variants_keep_one_pose_on_both_launchers(self):
        import shlex
        import graphics_harness as harness
        from test_graphics_harness import fake_repo
        for scenario in overlay.SCENARIOS:
            with tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                for name in ('current-rust-vulkan-shaders-off', 'frozen-opengl-shaders-off'):
                    mode = next(mode for mode in harness.MATRIX_MODES if mode.name == name)
                    args = harness.parse_args(['capture','--world-static-terrain-scenario','translucent-mixed',
                        '--gui-resource-pack-scenario',scenario])
                    args._canonical_fixture_run_source = root/'fixture'
                    _, env = harness.build_capture_command(fake_repo(root, mode.target), mode,
                        root/'capture','settled-static',args,'capture')
                    options = shlex.split(env['JAVA_TOOL_OPTIONS'])
                    for key, expected in (('poseCount','1'),('yawDelta','0.0')):
                        prefix = '-Dmattmc.dev.deterministicCameraCapture.' + key + '='
                        self.assertEqual(expected, [v.removeprefix(prefix) for v in options if v.startswith(prefix)][-1])

    def test_pack_pair_changes_only_overlay_pixels_not_blocks_or_models(self):
        with tempfile.TemporaryDirectory() as temporary:
            roots = [Path(temporary) / name for name in overlay.SCENARIOS]
            for root, name in zip(roots, overlay.SCENARIOS):
                capture_runner.write_gui_resource_pack(root, capture_runner.gui_resource_pack_specs(name)[0])
            payloads = [{p.relative_to(root).as_posix(): p.read_bytes() for p in root.rglob('*')
                         if p.is_file() and p.name != 'pack.mcmeta'} for root in roots]
            self.assertEqual(payloads[0].keys(), payloads[1].keys())
            self.assertEqual(['assets/minecraft/textures/block/water_overlay.png'],
                [name for name in payloads[0] if payloads[0][name] != payloads[1][name]])
            self.assertTrue(all(name.startswith('assets/minecraft/textures/block/') for name in payloads[0]))
            for root in roots:
                with Image.open(root/'assets/minecraft/textures/block/ice.png') as ice:
                    self.assertEqual({(0,0,0,0)}, set(ice.getdata()))

    def scene(self):
        image = Image.new('RGB', (1280,720), (40,70,130))
        image.paste((80,100,150), (350,400,380,620))
        return image

    def test_every_local_tile_is_required_and_blank_reference_rejected(self):
        image = self.scene()
        result = overlay.images(image, image)
        self.assertTrue(result['passed'])
        self.assertEqual(12, len(result['regions']))
        for row in result['regions']:
            wrong = image.copy()
            wrong.paste((0,0,0), row['crop_box'])
            self.assertFalse(overlay.images(image, wrong)['passed'])
        blank = Image.new('RGB', image.size)
        self.assertFalse(overlay.images(blank, blank)['passed'])
        self.assertFalse(overlay.images(image, image.resize((640,360)))['passed'])

    def test_green_alone_or_change_alone_cannot_prove_overlay(self):
        before = self.scene()
        green = before.copy()
        green.paste((20,180,30), (350,420,390,550))
        red = before.copy()
        red.paste((180,20,30), (350,420,390,550))
        self.assertTrue(overlay.effect(before, green)['passed'])
        self.assertFalse(overlay.effect(green, green)['passed'])
        self.assertFalse(overlay.effect(before, red)['passed'])
        # Existing foliage cannot supply the green witness for a separate,
        # unrelated red change elsewhere in the image.
        unrelated = green.copy()
        unrelated.paste((180,20,30), (300,550,430,620))
        self.assertFalse(overlay.effect(green, unrelated)['passed'])

    def test_fixture_requires_exact_ice_placement_camera_and_time(self):
        self.assertFalse(overlay.fixture(None))
        doc = {'cameraType':'FIRST_PERSON','dimension':'minecraft:overworld',
            'window':{'width':1280,'height':720},
            'mixedFluidFixture':{'fixture':'sealed-mixed-fluid-v1','variant':'ice','enclosure':'minecraft:ice',
                'placement':'146,99,532/west','cells':64,'matchingCells':64,'complete':True},
            'captures':[{'poseName':'initial','position':{'x':150.5,'y':100.0,'z':530.5},
                'observedYaw':105.0,'observedPitch':10.0,'gameTime':6000}]}
        self.assertTrue(overlay.fixture(doc))
        for key, value in [('variant','door'),('enclosure','minecraft:glass'),('complete',False),('matchingCells',63),('matchingCells',64.0)]:
            bad = copy.deepcopy(doc); bad['mixedFluidFixture'][key] = value
            self.assertFalse(overlay.fixture(bad))
        bad = copy.deepcopy(doc); bad['captures'][0]['gameTime'] = 6001
        self.assertFalse(overlay.fixture(bad))

    def test_visible_requires_accepted_control_and_both_backend_effects(self):
        before = self.scene(); after = before.copy()
        after.paste((20,180,30), (350,420,390,550))
        visual = {'pairs':[{}]}
        with patch.object(overlay, 'load_pair', return_value=[after,after]):
            self.assertFalse(overlay.report(visual, 'water-overlay-isolation')['passed'])
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary)/'reference.json'
            path.write_text(json.dumps({'success':True,'water_overlay_parity':{'passed':True,'scenario':'water-overlay-hidden'},
                'cross_repository_visual_parity':visual}))
            with patch.object(overlay, 'load_pair', side_effect=[[after,after],[before,before]]):
                self.assertTrue(overlay.report(visual, 'water-overlay-isolation', path)['passed'])
            with patch.object(overlay, 'load_pair', side_effect=[[after,after],[after,after]]):
                self.assertFalse(overlay.report(visual, 'water-overlay-isolation', path)['passed'])
            wrong = before.copy(); wrong.paste((0,0,0), overlay.BOX)
            with patch.object(overlay, 'load_pair', side_effect=[[after,after],[before,wrong]]):
                self.assertFalse(overlay.report(visual, 'water-overlay-isolation', path)['passed'])


if __name__ == '__main__':
    unittest.main()
