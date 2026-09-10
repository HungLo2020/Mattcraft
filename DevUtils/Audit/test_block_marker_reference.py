import sys
import unittest
from pathlib import Path
from unittest.mock import patch
from PIL import Image

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "Common"))
import block_marker_reference as marker


class MarkerReferenceTest(unittest.TestCase):
    def test_night_lighting_rejects_daylight_or_missing_lightmap_inputs(self):
        day={"captures":[{"lightmapSemanticFingerprint":"0,1,1.5,0,0,0,.5,1,1,1,1,1,1"}]}
        night={"captures":[{"lightmapSemanticFingerprint":"0,.24,1.5,0,0,0,.5,.48000002,.48000002,1,1,1,1"}]}
        self.assertTrue(marker.lighting_inputs(day,6000))
        self.assertTrue(marker.lighting_inputs(night,18000))
        self.assertFalse(marker.lighting_inputs(day,18000))
        self.assertFalse(marker.lighting_inputs(night,6000))
        self.assertFalse(marker.lighting_inputs({"captures":[{}]},18000))
    def test_reload_requires_real_completion_correct_priority_and_retained_pack(self):
        import copy
        receipt={"schema":"normal-world-resource-reload-v1","requested":True,
            "futureComplete":True,"complete":True,"presentations":2,
            "selectedBefore":["vanilla","file/mattmc-block-marker-b","file/mattmc-block-marker-a"],
            "selectedAtCapture":["vanilla","file/mattmc-block-marker-b"]}
        manifest={"cross_repository_parity":{"pairs":[{"comparable":True,
            "baseline_artifact":"baseline","current_artifact":"current"}]}}
        def check(value):
            with patch("graphics_harness.deterministic_capture_document",return_value={"worldResourceReload":value}):
                return marker.reload_evidence(manifest,"a","b")["passed"]
        self.assertTrue(check(receipt))
        for field,value in (("requested",False),("futureComplete",False),("complete",False),
            ("presentations",1),("selectedBefore",list(reversed(receipt["selectedBefore"]))),
            ("selectedBefore",["vanilla","file/mattmc-block-marker-a"]),
            ("selectedAtCapture",receipt["selectedBefore"]),
            ("selectedBefore",receipt["selectedBefore"]+["file/mattmc-block-marker-b"])):
            bad=copy.deepcopy(receipt);bad[field]=value
            self.assertFalse(check(bad),field)
        self.assertFalse(marker.reload_evidence({},"a","b")["passed"])

    def test_replacement_requires_changed_pixels_and_unchanged_gameplay_inputs(self):
        before=Image.new("RGB",(1280,720),(30,40,50))
        after=before.copy()
        after.paste((180,100,80),(560,120,720,280))
        def loaded(manifest, hidden, scenario, variant, game_time=6000):
            self.assertFalse(hidden)
            f={"light":15728640,"sourcePixels":{"rgbaFnv64":marker.pack_source_hash(variant)}}
            return [f,f],[before,before] if variant=="a" else [after,after]
        with patch.object(marker,"load",side_effect=loaded):
            result=marker.replacement_report({}, {}, "light-0", "a", "b")
            self.assertTrue(result["passed"])
            self.assertFalse(marker.replacement_report({}, {}, "light-0", "a", "a")["passed"])
        def stale(*args):
            receipts,images=loaded(*args)
            return receipts,[images[0],before]
        with patch.object(marker,"load",side_effect=stale):
            self.assertFalse(marker.replacement_report({}, {}, "light-0", "a", "b")["passed"])
        def changed_light(*args):
            receipts,images=loaded(*args)
            if args[3]=="b": receipts=[{**r,"light":0} for r in receipts]
            return receipts,images
        with patch.object(marker,"load",side_effect=changed_light):
            self.assertIn("unrelated",marker.replacement_report({}, {}, "light-0", "a", "b")["error"])

    def test_marker_pack_source_fingerprints_are_stable_and_distinct(self):
        self.assertEqual("9043d34032a67310",marker.pack_source_hash("a"))
        self.assertEqual("85dfe168c60c61b8",marker.pack_source_hash("b"))
        with self.assertRaises(ValueError):
            marker.pack_source_hash("vanilla")
    def test_generated_marker_packs_replace_only_all_seventeen_marker_textures(self):
        import tempfile
        import capture_runner
        expected={f"assets/minecraft/textures/item/{name}.png" for name in
            ("barrier",*(f"light_{i:02d}" for i in range(16)))}
        specs=capture_runner.gui_resource_pack_specs("block-marker-replacement")
        self.assertEqual(["b","a"],[spec["variant"] for spec in specs])
        payloads=[]
        with tempfile.TemporaryDirectory() as temporary:
            for spec in specs:
                root=Path(temporary)/spec["name"]
                capture_runner.write_gui_resource_pack(root,spec)
                files={p.relative_to(root).as_posix() for p in root.rglob("*") if p.is_file()}
                self.assertEqual(expected|{"pack.mcmeta"},files)
                snapshots={name:(root/name).read_bytes() for name in expected}
                for name in expected:
                    with Image.open(root/name) as pixels:
                        self.assertEqual((16,16),pixels.size)
                        self.assertEqual("RGBA",pixels.mode)
                        self.assertNotEqual(pixels.getpixel((0,1)),pixels.getpixel((1,0)),
                            "asymmetric edges must expose orientation errors")
                capture_runner.write_gui_resource_pack(root,spec)
                self.assertEqual(snapshots,{name:(root/name).read_bytes() for name in expected})
                payloads.append(snapshots)
        self.assertTrue(all(payloads[0][name]!=payloads[1][name] for name in expected))

    def test_all_light_levels_have_distinct_exact_sprite_identities(self):
        expected = [f"minecraft:item/light_{i:02d}" for i in range(16)]
        self.assertEqual(expected,[marker.expected_sprite(f"light-{i}") for i in range(16)])
        self.assertEqual("minecraft:item/barrier",marker.expected_sprite("barrier"))
        for value in (None,"light-16","light--1","light-01","light-x","stone",""):
            with self.assertRaises(ValueError):
                marker.expected_sprite(value)
    def test_duplicate_or_wrong_frame_marker_is_rejected_without_deduplication(self):
        import copy
        f={"fixture":"native-block-marker-v1","complete":True,"hidden":True,"size":.5,
           "scenario":"barrier","sprite":"minecraft:item/barrier","translucent":False,
           "staticSource":True,"color":[1,1,1,0],"position":[147.64617596880336,102.0991813627766,529.7355156371002],
           "sourcePixels":{"width":16,"height":16,"rgbaFnv64":"e8a36f6130ff8cb0"},
           "localUv":[.00048828125,.9995117,.00048828125,.9995117],"light":15728640}
        r={"frameIndex":42,"spriteId":f["sprite"],"route":"rust-vulkan-whole-frame","materialMode":1,
           "textureId":1419868698,"colorArgb":0xffffff,"projected":True,"quadSize":.5,
           "packedLight":f["light"],"viewport":{"width":1280,"height":720},
           "screenBounds":{"left":559.9,"right":720.1,"top":437.6,"bottom":597.8}}
        pair={"fixture_equivalence":{"status":"passed"},"baseline_artifact":"baseline",
              "current_artifact":"current","baseline_image":"a.png","current_image":"b.png"}
        manifest={"success":True,"cross_repository_visual_parity":{"pairs":[pair]}}
        doc={"blockMarkerFixture":f,"captures":[{"renderedFrameIndex":42,"gameTime":6000,
             "lightmapSemanticFingerprint":"0,1,1.5,0,0,0,.5,1,1,1,1,1,1"}]}
        def check(records, scenario="barrier", source_variant=None, game_time=6000):
            current={**doc,"rustGalWorldTerrainParticles":records}
            with patch("graphics_harness.deterministic_capture_document",side_effect=[doc,current]), \
                 patch("graphics_harness.read_json",return_value={}), \
                 patch("graphics_harness.water_detail_fixture_matches",return_value=True), \
                 patch.object(marker,"ownership",return_value=True), \
                 patch.object(marker.Image,"open",side_effect=lambda _:Image.new("RGB",(1280,720))):
                return marker.load(manifest,True,scenario,source_variant,game_time)
        self.assertEqual(2,len(check([r])[0]))
        with self.assertRaisesRegex(ValueError,"incorrect marker fixture"):
            check([r],game_time=18000)
        with self.assertRaisesRegex(ValueError,"explicit noon or midnight"):
            check([r],game_time=12000)
        with self.assertRaisesRegex(ValueError,"incorrect marker fixture"):
            check([r],"light-15")
        for variant in ("a","b"):
            with self.assertRaisesRegex(ValueError,"source does not match"):
                check([r],source_variant=variant)
        for records in ([],[r,copy.deepcopy(r)],[{**r,"frameIndex":41}]):
            with self.assertRaisesRegex(ValueError,"unique native marker"):
                check(records)

    def test_runtime_ownership_fails_closed_on_each_missing_invariant(self):
        import copy
        artifact = {"implementation_attribution":"rust-vulkan", "validation":{
            **{k:True for k in ("complete","crash_free","device_loss_free","vulkan_validation_clean",
                "vulkan_validation_passed","validation_layer_exercised")},
            "rss_guard_triggered":False, "vanilla_dh_isolation":{"passed":True}},
            "metrics":{"rust_gal_slice":{"backend_sync":{"gl_calls":0}}},
            "capture":{"whole_frame_gameplay_attachments":{"correlation_doc":{
                "java_vulkan_frame_execution":False,"rust_whole_frame_presenter":True}}}}
        self.assertTrue(marker.ownership(artifact))
        for key in artifact["validation"]:
            bad=copy.deepcopy(artifact)
            del bad["validation"][key]
            self.assertFalse(marker.ownership(bad),key)
        for key, value in (("java_vulkan_frame_execution",True),("rust_whole_frame_presenter",False)):
            bad=copy.deepcopy(artifact)
            bad["capture"]["whole_frame_gameplay_attachments"]["correlation_doc"][key]=value
            self.assertFalse(marker.ownership(bad))
        bad=copy.deepcopy(artifact)
        bad["metrics"]["rust_gal_slice"]["backend_sync"]["gl_calls"]=1
        self.assertFalse(marker.ownership(bad))

    def test_source_identity_does_not_admit_missing_or_nonfinite_data(self):
        f={"sourcePixels":{"width":16,"height":16,"rgbaFnv64":"e8a36f6130ff8cb0"},
           "localUv":[.00048828125,.9995117,.00048828125,.9995117],"light":15728640}
        self.assertTrue(marker.source_inputs(f))
        for uv in ([0,1,0,float("nan")],[.25,.5,.25,.5],[],[0,1,0,float("inf")]):
            self.assertFalse(marker.source_inputs({**f,"localUv":uv}))
        self.assertFalse(marker.source_inputs({**f,"sourcePixels":{}}))

    def test_local_checks_whole_footprint_and_guard_band(self):
        a = Image.new("RGB", (1280,720))
        b = a.copy()
        self.assertTrue(marker.local(a,b)["passed"])
        b.paste((255,255,255), (530,60,550,80))
        self.assertFalse(marker.local(a,b)["passed"])
        with self.assertRaises(ValueError):
            marker.local(a,Image.new("RGB",(1,1)))

    def test_equal_visible_images_do_not_pass_without_real_effect(self):
        image = Image.new("RGB",(1280,720))
        def load(manifest, hidden, scenario="barrier", source_variant=None, game_time=6000):
            f = {"hidden":hidden,"color":[1,1,1,0 if hidden else 1]}
            return [f,f],[image,image]
        with patch.object(marker,"load",side_effect=load):
            result = marker.report({}, {})
        self.assertFalse(result["passed"])
        self.assertEqual([0,0],[e["changed_pixels"] for e in result["effects"]])

    def test_rejection_is_reported_without_admission(self):
        self.assertFalse(marker.report({})["passed"])
        with patch.object(marker,"load",side_effect=ValueError("wrong sprite")):
            result=marker.report({})
        self.assertFalse(result["passed"])
        self.assertEqual("wrong sprite",result["error"])


if __name__ == "__main__":
    unittest.main()
