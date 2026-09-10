import sys
import tempfile
import unittest
from pathlib import Path
from PIL import Image,ImageDraw
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/"Common"))
from gui_block_foil_reference import images,sources,report,foil_residual


class BlockFoilTest(unittest.TestCase):
    def test_unenchanted_reference_keeps_same_static_camera_on_both_launchers(self):
        import shlex
        import graphics_harness as h
        from test_graphics_harness import fake_repo
        for fixture in ("standard-3d","standard-3d-logs",None):
            args=h.parse_args(["capture","--gui-resource-pack-scenario","vanilla","--item-gui-scale","3"]
                + (["--hotbar-item-fixture",fixture] if fixture else []))
            with tempfile.TemporaryDirectory() as temporary:
                root=Path(temporary)
                for name,mode_name in (("current","current-rust-vulkan-shaders-off"),("frozen","frozen-opengl-shaders-off")):
                    target=fake_repo(root,name)
                    mode=next(mode for mode in h.MATRIX_MODES if mode.name==mode_name)
                    _,env=h.build_capture_command(target,mode,root/name/"capture","correctness",args,"capture")
                    options=shlex.split(env["JAVA_TOOL_OPTIONS"])
                    expected=(("poseCount","1"),("yawDelta","0.0")) if fixture else (("poseCount","4"),("yawDelta","35.0"))
                    for key,value in expected:
                        prefix="-Dmattmc.dev.deterministicCameraCapture."+key+"="
                        self.assertEqual([s for s in options if s.startswith(prefix)][-1],prefix+value)
                    if fixture: self.assertEqual(env["MATTMC_CAPTURE_GUI_SCALE"],"3")

    def test_residual_distinguishes_base_error_and_signed_effect(self):
        def pixel(value): return Image.new("RGB",(1,1),(value,)*3)
        # Identical effect on differing backgrounds must have zero effect error.
        row=foil_residual(pixel(100),pixel(110),pixel(80),pixel(90))
        self.assertEqual(row["base_max_channel_error"],10)
        self.assertEqual(row["foil_effect_max_channel_error"],0)
        # Opposite signed changes must not cancel through unsigned clipping.
        row=foil_residual(pixel(70),pixel(110),pixel(90),pixel(90))
        self.assertEqual(row["foil_effect_mean_channel_error"],[40,40,40])
        self.assertEqual(row["foil_effect_pixels_over_eight"],1)
        with self.assertRaises(ValueError):
            foil_residual(pixel(1),pixel(1),pixel(1),Image.new("RGB",(2,1)))

    def test_pixels_require_every_foil_and_unchanged_control(self):
        from gui_special_foil_reference import image_pair
        before=Image.new("RGB",(1280,720),(20,20,20))
        boxes=[row["box"] for row in image_pair(before,before,2)["probes"]]
        for box in boxes:
            ImageDraw.Draw(before).rectangle((box[0]+3,box[1]+3,box[2]-4,box[3]-4),fill=(80,90,100))
        after=before.copy()
        for box in boxes[1:]:
            ImageDraw.Draw(after).rectangle((box[0]+3,box[1]+3,box[2]-4,box[3]-4),fill=(150,130,160))
        self.assertTrue(images(after,after,before,before,2)["passed"])
        self.assertFalse(images(before,before,before,before,2)["passed"])
        missing=after.copy();missing.paste(before.crop(boxes[5]),boxes[5])
        self.assertFalse(images(missing,missing,before,before,2)["passed"])
        dirty=after.copy();ImageDraw.Draw(dirty).rectangle(boxes[0],fill=(255,0,0))
        self.assertFalse(images(dirty,dirty,before,before,2)["passed"])
        self.assertFalse(images(after,before,before,before,2)["passed"])
        bad_reference=before.copy()
        box=boxes[3]
        ImageDraw.Draw(bad_reference).rectangle((box[0]+3,box[1]+3,box[2]-4,box[3]-4),fill=(110,100,90))
        result=images(after,after,before,bad_reference,2)
        self.assertTrue(all(row["passed"] for row in result["probes"]))
        self.assertFalse(result["passed"],"a bad unenchanted reference must still reject identical current icons")
        self.assertFalse(result["reference"]["passed"])
        self.assertFalse(result["reference"]["probes"][3]["passed"])

    def test_source_checks_support_partial_height_block_faces(self):
        names=("stone","grass_block_side","redstone_ore","oak_leaves","oak_planks",
               "oak_trapdoor","white_wool","crafting_table_side","oak_log")
        rows=[dict(sprite="minecraft:block/"+name,positions=[0,0,0,1,0,0,1,0.5,0,0,0.5,0],
                   atlasUvs=[0,0,1,0,1,1,0,1]) for name in names]
        receipt=dict(enabled=True,complete=True,sources=rows)
        self.assertEqual(len(sources(receipt)),9)
        for invalid in (dict(receipt,complete=False),dict(receipt,sources=rows[:8]),
                        dict(receipt,sources=rows+[rows[0]])):
            with self.assertRaises(ValueError): sources(invalid)
        rows[0]["positions"][0]=float("nan")
        with self.assertRaises(ValueError): sources(receipt)

    def test_missing_reference_rejects(self):
        self.assertFalse(report({},"block-item-foil",None)["passed"])
        self.assertFalse(report({},"block-item-foil-moving",None)["passed"])
        self.assertTrue(report({},"vanilla",None)["passed"])

    def test_moving_requires_actual_phase_for_every_icon(self):
        from graphics_harness import flat_item_witness_layout
        from gui_special_foil_reference import observed_timing
        boxes,_=flat_item_witness_layout(2,(1280,720))
        samples=[dict(x=(box[0]-1)//2,y=box[1]//2,scaledTicks=10000) for box in boxes[1:]]
        receipt=dict(enabled=True,complete=True,frameSequence=1,samples=samples)
        self.assertEqual(observed_timing(receipt,2,(1280,720),10000),[10000]*8)
        for invalid in (0,9999,10513):
            samples[3]["scaledTicks"]=invalid
            with self.assertRaises(ValueError): observed_timing(receipt,2,(1280,720),10000)

    def test_moving_launch_keeps_pack_geometry_and_static_camera(self):
        import graphics_harness as h
        from capture_runner import gui_resource_pack_specs
        from test_graphics_harness import fake_repo
        self.assertEqual(gui_resource_pack_specs("block-item-foil"),gui_resource_pack_specs("block-item-foil-moving"))
        args=h.parse_args(["capture","--hotbar-item-fixture","standard-3d-logs",
            "--gui-resource-pack-scenario","block-item-foil-moving","--item-gui-scale","2"])
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)
            for name,mode_name in (("current","current-rust-vulkan-shaders-off"),("frozen","frozen-opengl-shaders-off")):
                target=fake_repo(root,name)
                mode=next(mode for mode in h.MATRIX_MODES if mode.name==mode_name)
                _,env=h.build_capture_command(target,mode,root/name/"capture","correctness",args,"capture")
                options=env["JAVA_TOOL_OPTIONS"]
                self.assertIn("-Dmattmc.dev.graphicsAuditGuiItemFoilPhase=10000",options)
                self.assertIn("-Dmattmc.dev.graphicsAuditGuiItemFoilBlend=true",options)
                self.assertIn("-Dmattmc.dev.deterministicCameraCapture.poseCount=1",options)
                self.assertIn("-Dmattmc.dev.deterministicCameraCapture.yawDelta=0.0",options)

    def test_glint_only_pack_and_matching_launch_configuration(self):
        import graphics_harness as h
        from capture_runner import gui_resource_pack_specs,write_gui_resource_pack
        from test_graphics_harness import fake_repo
        args=h.parse_args(["capture","--hotbar-item-fixture","standard-3d-logs",
            "--gui-resource-pack-scenario","block-item-foil","--item-gui-scale","2"])
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)
            spec,=gui_resource_pack_specs("block-item-foil")
            write_gui_resource_pack(root/"pack",spec)
            self.assertEqual({p.relative_to(root/"pack").as_posix() for p in (root/"pack").rglob("*") if p.is_file()},
                {"pack.mcmeta","assets/minecraft/textures/misc/enchanted_glint_item.png",
                 "assets/minecraft/textures/misc/enchanted_glint_item.png.mcmeta"})
            for name,mode_name in (("current","current-rust-vulkan-shaders-off"),("frozen","frozen-opengl-shaders-off")):
                target=fake_repo(root,name)
                mode=next(mode for mode in h.MATRIX_MODES if mode.name==mode_name)
                _,env=h.build_capture_command(target,mode,root/name/"capture","correctness",args,"capture")
                self.assertEqual(env["MATTMC_CAPTURE_GUI_SCALE"],"2")
                self.assertIn("-Dmattmc.dev.graphicsAuditGuiItemFoilBlend=true",env["JAVA_TOOL_OPTIONS"])
                self.assertIn("-Dmattmc.dev.guiItemRasterTrace=true",env["JAVA_TOOL_OPTIONS"])


if __name__=="__main__": unittest.main()
