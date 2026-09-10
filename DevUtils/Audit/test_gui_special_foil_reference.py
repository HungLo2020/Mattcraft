import copy
import sys
import unittest
from pathlib import Path
from PIL import Image
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/"Common"))
from gui_special_foil_reference import image_pair, stopped_timing, observed_timing, report, matching_sources, temporal_images, temporal_reference


class SpecialFoilReferenceTest(unittest.TestCase):
    def test_real_negative_checker_requires_pixel_failures_not_missing_evidence(self):
        from check_special_foil_negative import negative_pixels_rejected
        row=dict(actual_fixture_sources_and_timing_verified=True,
                 probes=[dict(slot=i,passed=i==1,visible=True) for i in range(1,10)])
        gate=dict(passed=False,pairs=[row])
        self.assertTrue(negative_pixels_rejected(gate))
        for mutate in (lambda d:d.update(passed=True),
                       lambda d:d["pairs"][0].update(actual_fixture_sources_and_timing_verified=False),
                       lambda d:d["pairs"][0]["probes"].pop(),
                       lambda d:d["pairs"][0]["probes"][0].update(passed=False),
                       lambda d:d["pairs"][0]["probes"][1].update(passed=True),
                       lambda d:d["pairs"][0]["probes"][1].update(visible=False),
                       lambda d:d["pairs"][0]["probes"][1].update(slot=1)):
            wrong=copy.deepcopy(gate);mutate(wrong)
            self.assertFalse(negative_pixels_rejected(wrong))

    def test_temporal_gate_requires_each_special_to_change_and_control_to_stay_static(self):
        before = self.fixture()
        after = before.copy()
        for slot in range(1,9):
            for x in range(375+slot*60,423+slot*60):
                for y in range(663,711):
                    r,g,b=after.getpixel((x,y));after.putpixel((x,y),(r+10,g,b))
        self.assertTrue(temporal_images(before,before,after,after,3)["passed"])
        self.assertFalse(temporal_images(before,before,before,before,3)["passed"])
        with self.assertRaises(ValueError): temporal_images(before,before,after,before,3)
        for slot in range(1,9):
            stale=after.copy();box=(375+slot*60,663,423+slot*60,711)
            stale.paste(before.crop(box),box)
            self.assertFalse(temporal_images(before,before,stale,stale,3)["passed"])
        changed_control=after.copy()
        for x in range(375,423):
            for y in range(663,711):
                r,g,b=changed_control.getpixel((x,y));changed_control.putpixel((x,y),(r+10,g,b))
        self.assertFalse(temporal_images(before,before,changed_control,changed_control,3)["passed"])
        with self.assertRaises(ValueError): temporal_reference("unused",{},3,10000)

    def test_second_special_phase_requires_reference_and_stopped_fixture_rejects_it(self):
        import graphics_harness as h
        args=h.parse_args(["capture","--gui-resource-pack-scenario","special-item-foil-moving",
            "--hotbar-item-fixture","special-foil","--flat-item-foil-phase","40000"])
        with self.assertRaises(ValueError): h.validate_fixture_combinations(args)
        args.flat_item_foil_reference="accepted-phase-directory"
        h.validate_fixture_combinations(args)
        args.gui_resource_pack_scenario="special-item-foil-pattern"
        with self.assertRaises(ValueError): h.validate_fixture_combinations(args)

    def test_moving_timing_uses_actual_clock_and_rejects_wrong_phase(self):
        receipt = self.timing()
        for sample in receipt["samples"]: sample["scaledTicks"] = 340128
        self.assertEqual([340128]*8, observed_timing(receipt,3,(1280,720),10000))
        with self.assertRaises(ValueError): stopped_timing(receipt,3,(1280,720))
        for phase in (40000, -1, 330000, True):
            with self.assertRaises(ValueError): observed_timing(receipt,3,(1280,720),phase)
        for sample in receipt["samples"]:
            del sample["scaledTicks"]
            sample.update(clockMillis=85032,speed=.5,strength=.5)
        self.assertEqual([340128]*8, observed_timing(receipt,3,(1280,720),10000))
        receipt["samples"][0]["speed"] = 0
        with self.assertRaises(ValueError): observed_timing(receipt,3,(1280,720),10000)

    def test_pattern_launches_keep_shared_fixture_single_pose_and_no_implicit_admission(self):
        import tempfile
        import graphics_harness as h
        from test_graphics_harness import fake_repo
        args = h.parse_args(["capture", "--gui-resource-pack-scenario", "special-item-foil-pattern",
                             "--hotbar-item-fixture", "special-foil"])
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name in ("current-rust-vulkan-shaders-off", "frozen-opengl-shaders-off"):
                target = fake_repo(root, name)
                mode = next(m for m in h.MATRIX_MODES if m.name == name)
                _, env = h.build_capture_command(target, mode, root/name/"capture", "correctness", args, "capture")
                options = env["JAVA_TOOL_OPTIONS"]
                self.assertIn("hotbarItemFixture=special-foil", options)
                import shlex
                # JVM properties use the final occurrence, after generic defaults.
                effective = dict(part[2:].split("=", 1) for part in shlex.split(options)
                                 if part.startswith("-D") and "=" in part)
                self.assertEqual("1", effective["mattmc.dev.deterministicCameraCapture.poseCount"])
                self.assertEqual("0.0", effective["mattmc.dev.deterministicCameraCapture.yawDelta"])
                self.assertIn("graphicsAuditGuiItemFoilBlend=true", options)
                self.assertNotIn("rustGalGuiSpecialFoil", options)
                self.assertEqual("3", env["MATTMC_CAPTURE_GUI_SCALE"])
                args.hotbar_item_fixture = "flat-items"
                with self.assertRaises(ValueError):
                    h.build_capture_command(target, mode, root/name/"invalid", "correctness", args, "capture")
                args.hotbar_item_fixture = "special-foil"
                args.gui_resource_pack_scenario = "special-item-foil-moving"
                args.flat_item_foil_phase = 10000
                _, moving = h.build_capture_command(target, mode, root/name/"moving", "correctness", args, "capture")
                self.assertIn("graphicsAuditGuiItemFoilPhase=10000", moving["JAVA_TOOL_OPTIONS"])
                self.assertEqual("3", moving["MATTMC_CAPTURE_GUI_SCALE"])
                self.assertNotIn("rustGalGuiSpecialFoil", moving["JAVA_TOOL_OPTIONS"])
                args.gui_resource_pack_scenario = "special-item-foil-pattern"

    def test_special_pack_changes_only_glint_not_item_models_or_sprites(self):
        import tempfile
        import json
        from capture_runner import gui_resource_pack_specs, write_gui_resource_pack
        from graphics_harness import parse_args
        from gui_foil_reference import pattern_pixel
        self.assertEqual("special-item-foil-pattern", parse_args([
            "capture", "--gui-resource-pack-scenario", "special-item-foil-pattern",
            "--hotbar-item-fixture", "special-foil"]).gui_resource_pack_scenario)
        spec, = gui_resource_pack_specs("special-item-foil-pattern")
        with tempfile.TemporaryDirectory() as directory:
            pack = Path(directory)/"pack"
            write_gui_resource_pack(pack, spec)
            texture = "assets/minecraft/textures/misc/enchanted_glint_item.png"
            self.assertEqual({"pack.mcmeta", texture, texture+".mcmeta"},
                {p.relative_to(pack).as_posix() for p in pack.rglob("*") if p.is_file()})
            self.assertEqual({"texture": {"blur": True, "clamp": False}},
                             json.loads((pack/(texture+".mcmeta")).read_text()))
            with Image.open(pack/texture) as image:
                self.assertEqual((16,16), image.size)
                for y in range(16):
                    for x in range(16):
                        self.assertEqual(pattern_pixel(x,y), image.getpixel((x,y)))

    def test_all_special_sources_must_match_not_just_control(self):
        sources = {"minecraft:item/"+name: dict(positions=[0, 1], atlasUvs=[0, 1])
                   for name in ("apple", "clock_00", "compass_07")}
        self.assertTrue(matching_sources(sources, copy.deepcopy(sources)))
        control = {"minecraft:item/apple": sources["minecraft:item/apple"]}
        with self.assertRaises(ValueError): matching_sources(control, control)
        for name in ("clock_00", "compass_07"):
            key = "minecraft:item/"+name
            for field in ("positions", "atlasUvs"):
                wrong = copy.deepcopy(sources)
                wrong[key][field][0] = 0.5
                with self.assertRaises(ValueError): matching_sources(sources, wrong)
            missing = copy.deepcopy(sources)
            del missing[key]
            with self.assertRaises(ValueError): matching_sources(missing, missing)
            changed_frame = copy.deepcopy(sources)
            changed_frame[key[:-2]+"12"] = changed_frame.pop(key)
            with self.assertRaises(ValueError): matching_sources(sources, changed_frame)

    def fixture(self):
        image=Image.new("RGB",(1280,720))
        for slot in range(9):
            x=375+slot*60
            for px in range(x,x+48):
                for py in range(663,711):
                    image.putpixel((px,py),(80+(px%2)*40,50+(py%2)*40,30))
        return image

    def timing(self):
        from graphics_harness import flat_item_witness_layout
        boxes,_=flat_item_witness_layout(3,(1280,720))
        return dict(enabled=True,complete=True,frameSequence=1,samples=[
            dict(x=(b[0]-1)//3,y=b[1]//3,scaledTicks=0) for b in boxes[1:]])

    def test_every_icon_is_compared_and_blank_controls_reject(self):
        self.assertTrue(image_pair(self.fixture(),self.fixture(),3)["passed"])
        self.assertFalse(image_pair(Image.new("RGB",(1280,720)),Image.new("RGB",(1280,720)),3)["passed"])
        for slot in range(9):
            image=self.fixture()
            x=375+slot*60
            for px in range(x,x+48):
                for py in range(663,711):
                    r,g,b=image.getpixel((px,py));image.putpixel((px,py),(r+3,g,b))
            self.assertFalse(image_pair(self.fixture(),image,3)["passed"])

    def test_dimensions_scales_and_missing_required_receipts_reject(self):
        with self.assertRaises(ValueError): image_pair(self.fixture(),Image.new("RGB",(1280,721)),3)
        with self.assertRaises(ValueError): image_pair(self.fixture(),self.fixture(),4)
        self.assertFalse(report({},"special-foil")["passed"])
        self.assertFalse(report({"pairs":[{}]},"special-foil")["passed"])
        self.assertFalse(report({},"standard-3d")["requested"])
        from graphics_harness import parse_args
        self.assertEqual("special-foil",parse_args(["capture","--hotbar-item-fixture","special-foil"]).hotbar_item_fixture)

    def test_stopped_timing_requires_all_actual_slots_and_rejects_fabricated_defaults(self):
        receipt=self.timing()
        self.assertTrue(stopped_timing(receipt,3,(1280,720)))
        native=copy.deepcopy(receipt)
        for sample in native["samples"]:
            del sample["scaledTicks"]
            sample.update(clockMillis=12345,speed=0,strength=0.5)
        self.assertTrue(stopped_timing(native,3,(1280,720)))
        for mutation in (lambda d:d["samples"].pop(),
                         lambda d:d["samples"][0].update(scaledTicks=False),
                         lambda d:d["samples"][0].update(scaledTicks=1),
                         lambda d:d["samples"].__setitem__(0,d["samples"][1]),
                         lambda d:d.update(complete=False)):
            invalid=copy.deepcopy(receipt);mutation(invalid)
            with self.assertRaises(ValueError): stopped_timing(invalid,3,(1280,720))
        native["samples"][0]["speed"]=0.5
        with self.assertRaises(ValueError): stopped_timing(native,3,(1280,720))

    def test_cached_icons_require_explicit_same_frame_write_and_reuse_receipts(self):
        receipt=self.timing()
        original=receipt["samples"]
        receipt["samples"]=original[:2]
        receipt["atlasWrites"]=[dict(x=s["x"],y=s["y"],atlasX=i*48,atlasY=0)
                                 for i,s in enumerate(original[:2])]
        receipt["atlasAliases"]=[dict(x=s["x"],y=s["y"],sourceX=original[i%2]["x"],
            sourceY=original[i%2]["y"],atlasX=(i%2)*48,atlasY=0) for i,s in enumerate(original[2:])]
        self.assertTrue(stopped_timing(receipt,3,(1280,720)))
        for mutate in (lambda d:d["atlasWrites"].clear(), lambda d:d["atlasAliases"].pop(),
                       lambda d:d.update(atlasWrites=None),
                       lambda d:d["atlasAliases"].__setitem__(0,None),
                       lambda d:d["atlasAliases"][0].update(sourceX=999),
                       lambda d:d["atlasAliases"][0].update(atlasX=999),
                       lambda d:d["atlasAliases"].append(d["atlasAliases"][0])):
            invalid=copy.deepcopy(receipt);mutate(invalid)
            with self.assertRaises(ValueError): stopped_timing(invalid,3,(1280,720))
        for i, sample in enumerate(receipt["samples"]): sample["scaledTicks"] = 10000+i*4
        self.assertEqual([10000,10004]*4, observed_timing(receipt,3,(1280,720),10000))
        receipt["samples"][1]["scaledTicks"] = 40000
        with self.assertRaises(ValueError): observed_timing(receipt,3,(1280,720),10000)


if __name__ == "__main__": unittest.main()
