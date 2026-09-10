import sys
import unittest
import tempfile
import shlex
from unittest.mock import patch
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/"Common"))
from PIL import Image, ImageDraw
from opaque_particle_reference import BOX, full_quad, projected_bounds, requested_sign
from graphics_harness import atlas_particle_local_visual_evidence

class OpaqueParticleFootprintTest(unittest.TestCase):
    def test_sign_must_reach_both_actual_receipts(self):
        visual={"pairs":[{"baseline_artifact":"a","current_artifact":"b"}]}
        good={"atlasParticleFixture":{"sizeSign":-1,"size":-.35,"layer":"OPAQUE","complete":True}}
        with patch("graphics_harness.deterministic_capture_document",return_value=good):
            self.assertTrue(requested_sign(visual,"particle-atlas-static-a",-1)["passed"])
            self.assertFalse(requested_sign(visual,"particle-atlas-static-a",1)["passed"])
        with patch("graphics_harness.deterministic_capture_document",side_effect=[good,{}]):
            self.assertFalse(requested_sign(visual,"particle-atlas-static-a",-1)["passed"])
        self.assertFalse(requested_sign({"pairs":[]},"particle-atlas-static-a",-1)["passed"])

    def test_launcher_forwards_sign_to_both_repositories(self):
        import graphics_harness as harness
        from test_graphics_harness import fake_repo
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)
            for name in ("current-rust-vulkan-shaders-off","frozen-opengl-shaders-off"):
                mode=next(m for m in harness.MATRIX_MODES if m.name==name)
                args=harness.parse_args(["capture","--gui-resource-pack-scenario","particle-atlas-static-a",
                                        "--atlas-particle-size-sign","-1"])
                args._canonical_fixture_run_source=root/"fixture"
                _,env=harness.build_capture_command(fake_repo(root,mode.target),mode,root/"capture","settled-static",args,"capture")
                self.assertIn("-Dmattmc.dev.graphicsAuditAtlasParticleSizeSign=-1",shlex.split(env["JAVA_TOOL_OPTIONS"]))
                for flag in ("-Dmattmc.dev.graphicsAuditAtlasParticle=true",
                             "-Dmattmc.dev.graphicsAuditAtlasParticleStatic=true"):
                    self.assertIn(flag,shlex.split(env["JAVA_TOOL_OPTIONS"]))
                args.gui_resource_pack_scenario="vanilla"
                with self.assertRaises(ValueError):
                    harness.build_capture_command(fake_repo(root,mode.target),mode,root/"capture","settled-static",args,"capture")

    def fixture(self):
        image=Image.new("RGB",(1280,720),(150,190,220))
        draw=ImageDraw.Draw(image)
        for y in range(300,420):
            for x in range(580,700):
                draw.point((x,y),(190+(x-580)//8,30+(y-300)//8,20))
        return image

    def test_independent_projection_is_fully_contained(self):
        left,top,right,bottom=projected_bounds()
        self.assertLess(BOX[0],left)
        self.assertLess(BOX[1],top)
        self.assertGreater(BOX[2],right)
        self.assertGreater(BOX[3],bottom)

    def test_exact_and_missing_edge(self):
        baseline=self.fixture()
        self.assertTrue(full_quad(baseline,baseline,6)["passed"])
        missing=baseline.copy()
        ImageDraw.Draw(missing).rectangle((580,396,699,419),fill=(150,190,220))
        self.assertTrue(atlas_particle_local_visual_evidence(baseline,missing,6)["passed"])
        self.assertFalse(full_quad(baseline,missing,6)["passed"])

    def test_wrong_signed_uv_orientation_and_extent(self):
        baseline=self.fixture()
        wrong=baseline.copy()
        wrong.paste(baseline.crop((580,300,700,420)).transpose(Image.Transpose.ROTATE_180),(580,300))
        self.assertFalse(full_quad(baseline,wrong,6)["passed"])
        self.assertFalse(full_quad(baseline,baseline.resize((640,360)),6)["passed"])

if __name__=="__main__": unittest.main()
