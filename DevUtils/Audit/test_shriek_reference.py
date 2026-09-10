import sys
import shlex
import tempfile
import unittest
from pathlib import Path
from PIL import Image
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/"Common"))
import shriek_reference as shriek

class ShriekReferenceTest(unittest.TestCase):
    def test_both_launchers_receive_one_pose_and_same_control(self):
        import graphics_harness as harness
        from test_graphics_harness import fake_repo
        for scenario in ("delayed","visible"):
            with tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                for name in ("current-rust-vulkan-shaders-off","frozen-opengl-shaders-off"):
                    mode = next(m for m in harness.MATRIX_MODES if m.name==name)
                    args = harness.parse_args(["capture","--shriek-particle",scenario])
                    args._canonical_fixture_run_source = root/"fixture"
                    _,env = harness.build_capture_command(fake_repo(root,mode.target),mode,
                        root/"capture","settled-static",args,"capture")
                    values = shlex.split(env["JAVA_TOOL_OPTIONS"])
                    for key,expected in (("deterministicCameraCapture.poseCount","1"),
                        ("deterministicCameraCapture.yawDelta","0.0"),
                        ("graphicsAuditShriekParticle","true"),
                        ("graphicsAuditShriekDelayed",str(scenario=="delayed").lower())):
                        prefix = "-Dmattmc.dev."+key+"="
                        self.assertEqual(expected,[v[len(prefix):] for v in values if v.startswith(prefix)][-1])

    def test_missing_control_blank_pair_or_missing_second_quad_cannot_pass(self):
        sky = Image.new("RGB",(1280,720),(110,160,240))
        visible = sky.copy()
        visible.paste((230,230,250),(620,330,640,360))
        visible.paste((230,230,250),(640,330,660,360))
        self.assertTrue(shriek.local(visible,visible)["passed"])
        self.assertTrue(shriek.effect(sky,visible)["passed"])
        self.assertFalse(shriek.effect(sky,sky)["passed"])
        self.assertTrue(shriek.changed_pixel_parity(sky,visible,visible)["passed"])
        missing = visible.copy(); missing.paste((110,160,240),(640,330,660,360))
        self.assertFalse(shriek.changed_pixel_parity(sky,visible,missing)["passed"])
        self.assertFalse(shriek.changed_pixel_parity(sky,sky,sky)["passed"])
        self.assertFalse(shriek.report({"pairs":[]},"visible")["passed"])

    def test_every_tile_and_extent_required(self):
        sky = Image.new("RGB",(1280,720),(110,160,240))
        for row in shriek.local(sky,sky)["regions"]:
            wrong = sky.copy(); wrong.paste((0,0,0),row["box"])
            self.assertFalse(shriek.local(sky,wrong)["passed"])
        self.assertFalse(shriek.effect(sky,sky.resize((640,360)))["passed"])

if __name__=="__main__": unittest.main()
