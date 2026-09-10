import json
import sys
import tempfile
import unittest
from pathlib import Path
from PIL import Image, ImageDraw
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/"Common"))
from gui_block_layout_reference import images, report, placement_receipt
from capture_runner import gui_resource_pack_specs, write_gui_resource_pack


class BlockLayoutTest(unittest.TestCase):
    def test_ordinary_block_scale_reaches_both_launchers_without_a_pack(self):
        import graphics_harness as harness
        from test_graphics_harness import fake_repo
        args=harness.parse_args(["capture","--hotbar-item-fixture","standard-3d-logs","--item-gui-scale","2"])
        self.assertEqual(args.flat_item_gui_scale,2)
        with tempfile.TemporaryDirectory() as temporary:
            for name,mode_name in (("current","current-rust-vulkan-shaders-off"),("frozen","frozen-opengl-shaders-off")):
                target=fake_repo(Path(temporary),name)
                mode=next(mode for mode in harness.MATRIX_MODES if mode.name==mode_name)
                _,env=harness.build_capture_command(target,mode,Path(temporary)/name/"capture","correctness",args,"capture")
                self.assertEqual(env["MATTMC_CAPTURE_GUI_SCALE"],"2")

    def test_actual_placement_requires_exact_pose_clip_and_submission(self):
        import math
        x,y=205,221
        c,s=math.cos(0.2),math.sin(0.2)
        pose=[c*1.15,s*1.15,-s*0.9,c*0.9]
        pose += [x+8-(x-512+8)*pose[0]-(y+8)*pose[2],y+1-(x-512+8)*pose[1]-(y+8)*pose[3]]
        receipt=dict(schema="gui-item-placement-v1",item="minecraft:oak_slab",submitted=True,
                     x=x,y=y,originX=x-512,clip=[x,y-12,14,17],pose=pose)
        self.assertEqual(receipt,placement_receipt(dict(guiItemPlacement=receipt)))
        for change in (dict(submitted=False),dict(clip=[0,0,1280,720]),dict(pose=[1,0,0,1,0,0]),dict(pose=[float("nan")]*6)):
            with self.assertRaises(ValueError): placement_receipt(dict(guiItemPlacement=dict(receipt,**change)))
        with self.assertRaises(ValueError): placement_receipt({})

    def test_placement_flag_reaches_both_launchers(self):
        import graphics_harness as harness
        from test_graphics_harness import fake_repo
        args=harness.parse_args(["capture","--hotbar-item-fixture","standard-3d-logs",
            "--gui-resource-pack-scenario","block-item-oversized","--gui-item-placement"])
        with tempfile.TemporaryDirectory() as temporary:
            for name,mode_name in (("current","current-rust-vulkan-shaders-off"),("frozen","frozen-opengl-shaders-off")):
                target=fake_repo(Path(temporary),name)
                mode=next(mode for mode in harness.MATRIX_MODES if mode.name==mode_name)
                _,env=harness.build_capture_command(target,mode,Path(temporary)/name/"capture","correctness",args,"capture")
                self.assertIn("-Dmattmc.dev.graphicsAuditGuiItemPlacement=true",env["JAVA_TOOL_OPTIONS"])

    def pair(self):
        before=Image.new("RGB",(1280,720),(30,30,30))
        ImageDraw.Draw(before).rectangle((628,680,652,704),fill=(170,110,60))
        after=before.copy()
        ImageDraw.Draw(after).polygon([(595,664),(650,633),(685,677),(630,704)],fill=(190,130,70))
        return before,after

    def test_pixels_require_both_parity_and_actual_model_change(self):
        before,after=self.pair()
        self.assertTrue(images(after,after,before,before,3)["passed"])
        self.assertFalse(images(before,before,before,before,3)["passed"])
        self.assertFalse(images(after,before,before,before,3)["passed"])
        blank=Image.new("RGB",before.size,(0,0,0))
        self.assertFalse(images(blank,blank,before,before,3)["passed"])
        shifted=after.transform(after.size,Image.Transform.AFFINE,(1,0,8,0,1,0))
        self.assertFalse(images(after,shifted,before,before,3)["passed"])
        with self.assertRaises(ValueError): images(after,after.resize((640,360)),before,before,3)

    def test_missing_reference_and_pairs_reject(self):
        self.assertFalse(report({},"block-item-expanded",None)["passed"])
        self.assertFalse(report({},"block-item-expanded",Path("missing-reference.json"))["passed"])
        self.assertTrue(report({},"vanilla",None)["passed"])

    def test_report_requires_observed_reload_fixture_camera_and_pack(self):
        from unittest.mock import patch
        import graphics_harness as harness
        before,after=self.pair()
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)
            before.save(root/"before.png"); after.save(root/"after.png")
            def pair(name):
                return dict(baseline_artifact=str(root/name/"frozen.json"),
                            current_artifact=str(root/name/"current.json"),
                            baseline_image=str(root/(name+".png")),current_image=str(root/(name+".png")))
            reference=root/"reference.json"
            reference.write_text(json.dumps(dict(success=True,gui_block_lighting_parity=dict(passed=True),
                cross_repository_visual_parity=dict(pairs=[pair("before")]))))
            def meta(path):
                return dict(forced_option_guiScale="3",parity_fixture_id="fixture",parity_fixture_source_save_hash="hash",
                    parity_camera_x="1",parity_camera_y="2",parity_camera_z="3",parity_camera_yaw="4",parity_camera_pitch="5",
                    gui_resource_pack_selected=json.dumps(["file/mattmc-block-item-expanded"] if "after" in str(path) else []))
            def document(path):
                return dict(hotbarItemFixture="standard-3d-logs",worldResourceReload=dict(complete=True,
                    selectedAtCapture=["vanilla","file/mattmc-block-item-expanded"] if "after" in str(path) else ["vanilla"]))
            with patch.object(harness,"latest_capture_meta_path",side_effect=lambda p:p/"meta.txt"), \
                 patch.object(harness,"read_key_values",side_effect=meta), \
                 patch.object(harness,"deterministic_capture_document",side_effect=document) as docs:
                self.assertTrue(report(dict(pairs=[pair("after")]),"block-item-expanded",reference)["passed"])
                docs.side_effect=lambda p: dict(hotbarItemFixture="standard-3d-logs",worldResourceReload=dict(complete=False))
                self.assertFalse(report(dict(pairs=[pair("after")]),"block-item-expanded",reference)["passed"])

    def test_pack_only_replaces_slab_item_not_world_or_lighting_controls(self):
        import graphics_harness as harness
        args=harness.parse_args(["capture","--gui-resource-pack-scenario","block-item-expanded",
                                 "--gui-block-layout-reference","before.json"])
        self.assertEqual(args.gui_block_layout_reference,Path("before.json"))
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)/"pack"
            spec,=gui_resource_pack_specs("block-item-expanded")
            write_gui_resource_pack(root,spec)
            self.assertEqual({p.relative_to(root).as_posix() for p in root.rglob("*") if p.is_file()},
                {"pack.mcmeta","assets/minecraft/items/oak_slab.json",
                 "assets/minecraft/models/item/mattmc_expanded_oak_slab.json"})
            model=json.loads((root/"assets/minecraft/models/item/mattmc_expanded_oak_slab.json").read_text())
            self.assertEqual(model["gui_light"],"side")
            self.assertEqual(set(model["elements"][0]["faces"]),{"up","down","north","south","east","west"})
            self.assertGreater(model["elements"][0]["to"][0]-model["elements"][0]["from"][0],16)
            self.assertNotEqual(model["display"]["gui"]["translation"],[0,0,0])

    def test_both_launchers_use_same_single_pose(self):
        import shlex
        import graphics_harness as harness
        from test_graphics_harness import fake_repo
        args=harness.parse_args(["capture","--hotbar-item-fixture","standard-3d-logs",
            "--gui-resource-pack-scenario","block-item-expanded","--world-resource-reload",
            "--gui-block-layout-reference","before.json"])
        with tempfile.TemporaryDirectory() as temporary:
            for name,mode_name in (("current","current-rust-vulkan-shaders-off"),("frozen","frozen-opengl-shaders-off")):
                target=fake_repo(Path(temporary),name)
                mode=next(mode for mode in harness.MATRIX_MODES if mode.name==mode_name)
                _,env=harness.build_capture_command(target,mode,Path(temporary)/name/"capture","correctness",args,"capture")
                props=dict(value[2:].split("=",1) for value in shlex.split(env["JAVA_TOOL_OPTIONS"])
                           if value.startswith("-D") and "=" in value)
                self.assertEqual(props["mattmc.dev.deterministicCameraCapture.poseCount"],"1")
                self.assertEqual(props["mattmc.dev.deterministicCameraCapture.yawDelta"],"0.0")

    def test_oversized_variant_changes_only_explicit_item_property(self):
        import graphics_harness as harness
        args=harness.parse_args(["capture","--gui-resource-pack-scenario","block-item-oversized"])
        self.assertEqual(args.gui_resource_pack_scenario,"block-item-oversized")
        with tempfile.TemporaryDirectory() as temporary:
            roots=[Path(temporary)/name for name in ("ordinary","oversized")]
            for root,scenario in zip(roots,("block-item-expanded","block-item-oversized")):
                spec,=gui_resource_pack_specs(scenario)
                write_gui_resource_pack(root,spec)
            model="assets/minecraft/models/item/mattmc_expanded_oak_slab.json"
            self.assertEqual((roots[0]/model).read_bytes(),(roots[1]/model).read_bytes())
            ordinary,oversized=[json.loads((root/"assets/minecraft/items/oak_slab.json").read_text()) for root in roots]
            self.assertFalse(ordinary.pop("oversized_in_gui"))
            self.assertTrue(oversized.pop("oversized_in_gui"))
            self.assertEqual(ordinary,oversized)


if __name__ == "__main__": unittest.main()
