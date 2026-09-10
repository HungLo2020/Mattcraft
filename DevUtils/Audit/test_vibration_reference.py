import json
import shlex
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch
from PIL import Image
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/"Common"))
import vibration_reference as vibration
import graphics_harness as harness


class VibrationReferenceTest(unittest.TestCase):
    def test_elevated_witness_covers_all_corners_above_horizon(self):
        import vibration_motion_reference as motion
        import vibration_elevated_reference as elevated
        center,corners=motion.projected_fixture_corners(0,1)
        self.assertAlmostEqual(center[1],180.83328580,places=5)
        left,top,right,bottom=elevated.BOX
        self.assertLess(bottom,300, "keep the complete witness above world terrain and water")
        for x,y in corners: self.assertTrue(left<x<right and top<y<bottom,(x,y))

    def test_elevated_launcher_input_reaches_both_repositories(self):
        from test_graphics_harness import fake_repo
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)
            for name in ("current-rust-vulkan-shaders-off","frozen-opengl-shaders-off"):
                mode=next(m for m in harness.MATRIX_MODES if m.name==name)
                args=harness.parse_args(["capture","--vibration-particle","visible","--vibration-elevation","1"])
                args._canonical_fixture_run_source=root/"fixture"
                _,env=harness.build_capture_command(fake_repo(root,mode.target),mode,root/"capture","settled-static",args,"capture")
                self.assertIn("-Dmattmc.dev.graphicsAuditVibrationElevation=1",shlex.split(env["JAVA_TOOL_OPTIONS"]))
                args.vibration_steps=6
                with self.assertRaises(ValueError):
                    harness.build_capture_command(fake_repo(root,mode.target),mode,root/"capture","settled-static",args,"capture")

    def test_elevated_gate_rejects_stale_phase_missing_lower_edge_and_old_control(self):
        sky=Image.new("RGB",(1280,720),(110,160,240))
        initial=sky.copy();initial.paste((230,230,250),(620,150,650,220))
        advanced=sky.copy();advanced.paste((50,210,90),(620,150,650,220))
        clipped=advanced.copy();clipped.paste((110,160,240),(620,190,650,220))
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);hidden=root/"hidden.json";zero=root/"zero.json"
            for path,scenario in ((hidden,"hidden"),(zero,"visible")):
                path.write_text(json.dumps({"success":True,"vibration_parity":{
                    "passed":True,"scenario":scenario,"phase":0,"elevation":1},
                    "cross_repository_visual_parity":{"pairs":[{}]}}))
            for current,expected in ((advanced,True),(clipped,False),(initial,False),(sky,False)):
                with patch.object(vibration,"load",side_effect=[
                    ([advanced,current],{}),([sky,sky],{}),([initial,initial],{})]):
                    result=vibration.report({"pairs":[{}]},"visible",hidden,3,zero,elevation=1)
                    self.assertEqual(expected,result["passed"])
            doc=json.loads(hidden.read_text());doc["vibration_parity"]["elevation"]=0
            hidden.write_text(json.dumps(doc))
            with patch.object(vibration,"load",return_value=([advanced,advanced],{})):
                result=vibration.report({"pairs":[{}]},"visible",hidden,3,zero,elevation=1)
                self.assertFalse(result["passed"])
                self.assertIn("elevation differs",result["error"])

    def test_zero_step_witness_contains_the_full_projected_quads(self):
        import vibration_motion_reference as motion
        _,corners = motion.projected_fixture_corners(0)
        self.assertGreater(max(y for _,y in corners),390, "old crop missed lower geometry")
        left,top,right,bottom = vibration.FULL_ZERO_STEP_BOX
        for x,y in corners:
            self.assertTrue(left<x<right and top<y<bottom,(x,y))

    def test_missing_lower_quad_cannot_pass_the_old_sky_only_crop(self):
        sky = Image.new("RGB",(1280,720),(110,160,240))
        visible = sky.copy()
        visible.paste((230,230,250),(620,330,640,350))
        visible.paste((230,230,250),(590,395,630,415))
        clipped = visible.copy()
        clipped.paste((110,160,240),(590,395,630,415))
        self.assertTrue(vibration.changed_pixel_parity(sky,visible,clipped)["passed"])
        with tempfile.TemporaryDirectory() as temporary:
            reference = Path(temporary)/"control.json"
            reference.write_text(json.dumps({"success":True,
                "vibration_parity":{"passed":True,"scenario":"hidden"},
                "cross_repository_visual_parity":{"pairs":[{}]}}))
            for current,expected in ((visible,True),(clipped,False)):
                with patch.object(vibration,"load",side_effect=[([visible,current],{}),([sky,sky],{})]):
                    result = vibration.report({"pairs":[{}]},"visible",reference)
                    self.assertEqual(expected,result["passed"])
                    self.assertEqual(expected,result["full_quad_parity"]["passed"])

    def test_fixture_requires_the_actual_fixed_extraction_fraction(self):
        f = dict(fixture="ordinary-vibration-fixed-simulation-v1",hidden=False,complete=True,
            sprite="minecraft:vibration",light=240,age=12,lifetime=40,alpha=1,size=0.3,
            position=[147.64617596880336,101.0991813627766,529.7355156371002])
        doc = {"captures":[{"gameTime":6000}],"vibrationParticleFixture":f}
        with patch.object(harness,"water_detail_fixture_matches",return_value=True):
            self.assertFalse(vibration.fixture(doc,False))
            f["extractionPartialTick"] = 0.25
            self.assertFalse(vibration.fixture(doc,False))
            f["extractionPartialTick"] = 1.0
            self.assertTrue(vibration.fixture(doc,False))
            f["elevation"]=1
            self.assertFalse(vibration.fixture(doc,False,0,1), "receipt alone cannot move the actual particle")
            f["position"]=list(vibration.expected_position(0,1))
            self.assertTrue(vibration.fixture(doc,False,0,1))
            self.assertFalse(vibration.fixture(doc,False))
            f["elevation"]=0
            f["position"]=list(vibration.expected_position(0))
            f["age"] = 18
            f["simulationSteps"] = 6
            self.assertFalse(vibration.fixture(doc,False,6), "old position is not a six-tick snapshot")
            f["position"] = list(vibration.expected_position(6))
            self.assertTrue(vibration.fixture(doc,False,6))
            self.assertFalse(vibration.fixture(doc,False,0))

    def test_six_step_launcher_input_reaches_both_repositories(self):
        from test_graphics_harness import fake_repo
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for name in ("current-rust-vulkan-shaders-off","frozen-opengl-shaders-off"):
                mode = next(m for m in harness.MATRIX_MODES if m.name==name)
                args = harness.parse_args(["capture","--vibration-particle","visible","--vibration-steps","6"])
                args._canonical_fixture_run_source = root/"fixture"
                _,env = harness.build_capture_command(fake_repo(root,mode.target),mode,
                    root/"capture","settled-static",args,"capture")
                self.assertIn("-Dmattmc.dev.graphicsAuditVibrationSteps=6",shlex.split(env["JAVA_TOOL_OPTIONS"]))

    def test_both_launchers_set_identical_simulation_and_animation_inputs(self):
        from test_graphics_harness import fake_repo
        for scenario in ("hidden","visible"):
            with tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                for name in ("current-rust-vulkan-shaders-off","frozen-opengl-shaders-off"):
                    mode = next(m for m in harness.MATRIX_MODES if m.name==name)
                    args = harness.parse_args(["capture","--vibration-particle",scenario,"--vibration-phase","3"])
                    args._canonical_fixture_run_source = root/"fixture"
                    _,env = harness.build_capture_command(fake_repo(root,mode.target),mode,
                        root/"capture","settled-static",args,"capture")
                    values = shlex.split(env["JAVA_TOOL_OPTIONS"])
                    self.assertEqual("true",env["MATTMC_GRAPHICS_AUDIT"])
                    self.assertEqual(str(0x50415254),env["MATTMC_ATLAS_TRACE_TEXTURE"])
                    self.assertEqual("1",env["MATTMC_ATLAS_TRACE_SPRITE"])
                    for key,expected in (("deterministicCameraCapture.poseCount","1"),
                        ("deterministicCameraCapture.yawDelta","0.0"),
                        ("graphicsAuditVibrationParticle","true"),
                        ("graphicsAuditVibrationSteps","0"),
                        ("graphicsAuditVibrationHidden",str(scenario=="hidden").lower()),
                        ("graphicsAuditVibrationCapturePhase","3")):
                        prefix = "-Dmattmc.dev."+key+"="
                        self.assertEqual(expected,[v[len(prefix):] for v in values if v.startswith(prefix)][-1])

    def test_visible_gate_rejects_missing_effect_or_quad_even_when_tiles_pass(self):
        sky = Image.new("RGB",(1280,720),(110,160,240))
        visible = sky.copy()
        visible.paste((230,230,250),(620,330,640,360))
        visible.paste((230,230,250),(640,330,660,360))
        missing = visible.copy()
        missing.paste((110,160,240),(640,330,660,360))
        with tempfile.TemporaryDirectory() as temporary:
            reference = Path(temporary)/"control.json"
            reference.write_text(json.dumps({"success":True,
                "vibration_parity":{"passed":True,"scenario":"hidden"},
                "cross_repository_visual_parity":{"pairs":[{}]}}))
            for current,expected in ((visible,True),(missing,False),(sky,False)):
                with patch.object(vibration,"load",side_effect=[
                    ([visible,current],{}),([sky,sky],{})]):
                    self.assertEqual(expected,vibration.report({"pairs":[{}]},"visible",reference)["passed"])
        self.assertFalse(vibration.report({"pairs":[]},"visible")["passed"])

    def test_upload_evidence_is_mandatory_and_phase_zero_is_checked(self):
        pair = {"fixture_equivalence":{"status":"passed"},
            "baseline_artifact":"baseline","current_artifact":"current"}
        with patch.object(harness,"deterministic_capture_document",return_value={}), \
             patch.object(vibration,"fixture",return_value=True):
            for evidence in (None,{"passed":False},{"passed":True,"frozen":{"frame":1,"subFrame":0}}):
                with patch.object(harness,"block_display_animation_upload_equivalence",return_value=evidence):
                    with self.assertRaises(ValueError): vibration.load(pair,False)

    def test_nonzero_phase_requires_visible_change_and_accepted_phase_zero(self):
        sky = Image.new("RGB",(1280,720),(110,160,240))
        initial = sky.copy()
        initial.paste((230,230,250),(620,330,660,360))
        advanced = sky.copy()
        advanced.paste((50,210,90),(620,330,660,360))
        initial.paste((230,230,250),(590,395,630,415))
        advanced.paste((50,210,90),(590,395,630,415))
        stale_lower = advanced.copy()
        stale_lower.paste((230,230,250),(590,395,630,415))
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            hidden = root/"hidden.json"
            zero = root/"zero.json"
            for path,scenario in ((hidden,"hidden"),(zero,"visible")):
                path.write_text(json.dumps({"success":True,
                    "vibration_parity":{"passed":True,"scenario":scenario,"phase":0},
                    "cross_repository_visual_parity":{"pairs":[{}]}}))
            for current,expected in ((advanced,True),(initial,False),(stale_lower,False)):
                with patch.object(vibration,"load",side_effect=[
                    ([advanced,current],{}),([sky,sky],{}),([initial,initial],{})]):
                    self.assertEqual(expected,vibration.report({"pairs":[{}]},"visible",hidden,3,zero)["passed"])
            with patch.object(vibration,"load",side_effect=[
                ([advanced,advanced],{}),([sky,sky],{})]):
                self.assertFalse(vibration.report({"pairs":[{}]},"visible",hidden,3)["passed"])
            # Identical pictures cannot prove an animation transition, even
            # if both routes agree and the ordinary visibility gate passes.
            with patch.object(vibration,"load",side_effect=[
                ([initial,initial],{}),([sky,sky],{}),([initial,initial],{})]):
                self.assertFalse(vibration.report({"pairs":[{}]},"visible",hidden,3,zero)["passed"])

    def test_motion_requires_new_location_and_removal_of_old_image(self):
        sky = Image.new("RGB",(1280,720),(110,160,240))
        initial = sky.copy()
        initial.paste((230,230,250),(620,330,660,360))
        moved = sky.copy()
        moved.paste((230,230,250),(770,270,810,300))
        ghost = moved.copy()
        ghost.paste((230,230,250),(620,330,660,360))
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            hidden = root/"hidden.json"
            zero = root/"zero.json"
            for path,scenario in ((hidden,"hidden"),(zero,"visible")):
                path.write_text(json.dumps({"success":True,
                    "vibration_parity":{"passed":True,"scenario":scenario,"phase":0},
                    "cross_repository_visual_parity":{"pairs":[{}]}}))
            for current,expected in ((moved,True),(initial,False),(ghost,False)):
                with patch.object(vibration,"load",side_effect=[
                    ([moved,current],{}),([sky,sky],{}),([initial,initial],{})]):
                    self.assertEqual(expected,vibration.report({"pairs":[{}]},"visible",hidden,
                        steps=6,motion_reference=zero)["passed"])
            with patch.object(vibration,"load",side_effect=[([moved,moved],{}),([sky,sky],{})]):
                self.assertFalse(vibration.report({"pairs":[{}]},"visible",hidden,steps=6)["passed"])

    def test_motion_witness_contains_projected_quads_on_correct_camera_side(self):
        import vibration_motion_reference as motion
        center,corners = motion.projected_fixture_corners()
        self.assertAlmostEqual(790.43455775,center[0],places=5)
        self.assertAlmostEqual(309.30522151,center[1],places=5)
        self.assertEqual(8,len(corners))
        left,top,right,bottom = motion.BOX
        for x,y in corners:
            self.assertTrue(left<x<right and top<y<bottom,(x,y))


if __name__=="__main__": unittest.main()
