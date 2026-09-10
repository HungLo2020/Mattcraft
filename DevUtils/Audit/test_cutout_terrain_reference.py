import copy
import json
import shlex
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/"Common"))
from PIL import Image, ImageDraw
import cutout_terrain_reference as cutout
import graphics_harness as harness


class CutoutTerrainReferenceTest(unittest.TestCase):
    def receipt(self):
        return {"fixture":cutout.NAME,"complete":True,"block":"minecraft:oak_leaves",
            "sprite":"minecraft:block/oak_leaves","alphaTested":True,"staticSource":True,
            "hidden":False,"size":.35,"localUv":[.5,.25,.25,.5],
            "position":[147.64617596880336,102.0991813627766,529.7355156371002],
            "color":[.3,.5,.2,1],"light":15728640,
            "sourcePixels":{"width":16,"height":16,"rgbaFnv64":"123456789abcdef0",
                            "quarterTransparentPixels":8,"quarterOpaquePixels":8}}

    def test_input_rejects_missing_holes_static_source_and_wrong_visibility(self):
        self.assertTrue(cutout.fixture_inputs(self.receipt()))
        for field,value in (("alphaTested",False),("staticSource",False),("hidden",True),
                            ("position",[0,0,0]),("color",[1,1,float("nan"),1])):
            r=self.receipt(); r[field]=value
            self.assertFalse(cutout.fixture_inputs(r),field)
        r=self.receipt(); r["sourcePixels"]["quarterTransparentPixels"]=0
        self.assertFalse(cutout.fixture_inputs(r))
        a={"terrainParticleFixture":self.receipt()}; b=copy.deepcopy(a)
        self.assertTrue(cutout.source_evidence([a,b])["passed"])
        b["terrainParticleFixture"]["sourcePixels"]["rgbaFnv64"]="0000000000000000"
        self.assertFalse(cutout.source_evidence([a,b])["passed"])

    def test_full_footprint_rejects_missing_particle_and_filled_holes(self):
        hidden=Image.new("RGB",(1280,720),(150,190,220))
        visible=hidden.copy(); draw=ImageDraw.Draw(visible)
        for y in range(130,230,25):
            for x in range(590,690,25):
                if (x//25+y//25)%2:
                    draw.rectangle((x,y,x+24,y+24),fill=(40,90,30))
        filled=hidden.copy(); ImageDraw.Draw(filled).rectangle((590,130,689,229),fill=(40,90,30))
        self.assertTrue(cutout.local(visible,visible)["passed"])
        self.assertTrue(cutout.effect(hidden,visible)["passed"])
        self.assertFalse(cutout.effect(hidden,hidden)["passed"])
        self.assertFalse(cutout.local(visible,hidden)["passed"])
        self.assertFalse(cutout.local(visible,filled)["passed"])

    def test_producer_must_cover_actual_capture_and_stay_in_fixed_box(self):
        record={"frameIndex":3,"spriteId":"minecraft:block/oak_leaves",
            "route":"rust-vulkan-whole-frame","projected":True,
            "viewport":{"width":1280,"height":720},"materialMode":2,
            "screenBounds":{"left":585,"top":462,"right":695,"bottom":574}}
        doc={"terrainParticleFixture":self.receipt(),"captures":[{"renderedFrameIndex":3}],
             "rustGalWorldTerrainParticles":[record]}
        self.assertTrue(cutout.producer(doc))
        record["frameIndex"]=2
        self.assertFalse(cutout.producer(doc))
        record["frameIndex"]=3; record["screenBounds"]["bottom"]=621
        self.assertFalse(cutout.producer(doc))
        record["screenBounds"]={"left":585,"top":146,"right":695,"bottom":258}
        self.assertFalse(cutout.producer(doc),"must not confuse image and diagnostic coordinates")

    def test_visible_requires_accepted_control_even_with_exact_images(self):
        image=Image.new("RGB",(1280,720))
        with patch.object(cutout,"load",return_value=([{},{}],[image,image])):
            self.assertFalse(cutout.report({"pairs":[{}]},"visible")["passed"])
        self.assertFalse(cutout.report({"pairs":[]},"hidden")["passed"])

    def test_translucent_source_requires_fractional_alpha_and_translucent_producer(self):
        r=self.receipt(); r.update(fixture=cutout.GLASS_NAME,block="minecraft:blue_stained_glass",
            sprite="minecraft:block/blue_stained_glass",alphaTested=False,translucent=True)
        r["sourcePixels"].update(quarterTransparentPixels=0,quarterOpaquePixels=0,quarterTranslucentPixels=16)
        self.assertTrue(cutout.fixture_inputs(r))
        record={"frameIndex":3,"spriteId":r["sprite"],"route":"rust-vulkan-whole-frame","projected":True,
            "viewport":{"width":1280,"height":720},"materialMode":3,
            "screenBounds":{"left":585,"top":462,"right":695,"bottom":574}}
        doc={"terrainParticleFixture":r,"captures":[{"renderedFrameIndex":3}],"rustGalWorldTerrainParticles":[record]}
        self.assertTrue(cutout.producer(doc))
        record["materialMode"]=1
        self.assertFalse(cutout.producer(doc))
        r["sourcePixels"].update(quarterOpaquePixels=16,quarterTranslucentPixels=0)
        self.assertFalse(cutout.fixture_inputs(r))

    def test_visible_report_rejects_both_missing_and_filled_holes(self):
        hidden=Image.new("RGB",(1280,720),(150,190,220))
        visible=hidden.copy()
        ImageDraw.Draw(visible).rectangle((590,130,614,229),fill=(40,90,30))
        filled=visible.copy()
        ImageDraw.Draw(filled).rectangle((615,130,689,229),fill=(40,90,30))
        shown_doc={"terrainParticleFixture":self.receipt()}
        hidden_doc=copy.deepcopy(shown_doc)
        hidden_doc["terrainParticleFixture"]["hidden"]=True
        hidden_doc["terrainParticleFixture"]["color"][3]=0
        prior={"success":True,"cutout_terrain_parity":{"passed":True,"scenario":"hidden"},
               "cross_repository_visual_parity":{"pairs":[{}]}}
        for images, expected in (([visible,visible],True),([hidden,hidden],False),([visible,filled],False)):
            with patch.object(cutout,"load",side_effect=[([shown_doc,shown_doc],images),
                    ([hidden_doc,hidden_doc],[hidden,hidden])]), \
                    patch.object(Path,"read_text",return_value=json.dumps(prior)):
                self.assertEqual(expected,cutout.report({"pairs":[{}]},"visible","control")["passed"])

    def test_both_launchers_receive_explicit_fixture_and_visibility(self):
        from test_graphics_harness import fake_repo
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)
            for name in ("current-rust-vulkan-shaders-off","frozen-opengl-shaders-off"):
                mode=next(m for m in harness.MATRIX_MODES if m.name==name)
                for scenario in ("hidden","visible"):
                    args=harness.parse_args(["capture","--cutout-terrain-particle",scenario])
                    args._canonical_fixture_run_source=root/"fixture"
                    _,env=harness.build_capture_command(fake_repo(root,mode.target),mode,
                        root/"capture","settled-static",args,"capture")
                    flags=shlex.split(env["JAVA_TOOL_OPTIONS"])
                    self.assertIn("-Dmattmc.dev.graphicsAuditCutoutTerrainParticle=true",flags)
                    self.assertIn("-Dmattmc.dev.graphicsAuditCutoutTerrainHidden="+str(scenario=="hidden").lower(),flags)
                    self.assertNotIn("-Dmattmc.dev.graphicsAuditMagmaCycleCapture=true",flags)

    def test_both_launchers_receive_translucent_fixture_and_reject_ambiguous_surface(self):
        from test_graphics_harness import fake_repo
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)
            for name in ("current-rust-vulkan-shaders-off","frozen-opengl-shaders-off"):
                mode=next(m for m in harness.MATRIX_MODES if m.name==name)
                for scenario in ("hidden","visible"):
                    args=harness.parse_args(["capture","--translucent-terrain-particle",scenario])
                    args._canonical_fixture_run_source=root/"fixture"
                    _,env=harness.build_capture_command(fake_repo(root,mode.target),mode,
                        root/"capture","settled-static",args,"capture")
                    flags=shlex.split(env["JAVA_TOOL_OPTIONS"])
                    self.assertIn("-Dmattmc.dev.graphicsAuditTranslucentTerrainParticle=true",flags)
                    self.assertIn("-Dmattmc.dev.graphicsAuditTranslucentTerrainHidden="+str(scenario=="hidden").lower(),flags)
                    args.cutout_terrain_particle="visible"
                    with self.assertRaises(ValueError):
                        harness.build_capture_command(fake_repo(root,mode.target),mode,
                            root/"capture","settled-static",args,"capture")


if __name__=="__main__": unittest.main()
