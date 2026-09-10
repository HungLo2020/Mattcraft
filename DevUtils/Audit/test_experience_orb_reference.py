import unittest
from PIL import Image
from DevUtils.Common.experience_orb_reference import local_difference, paired_effect, validate_document, POSES
from DevUtils.Common.experience_orb_reference import report, environment_signature, pack_signature
from DevUtils.Common.experience_orb_reference import validate_pack_reload
from DevUtils.Common.experience_orb_reference import occlusion_pixels
import copy
import sys
import tempfile
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "Common"))
import capture_runner
from types import SimpleNamespace
from unittest.mock import patch


def background():
    return Image.new("RGB", (1280, 720), (160, 190, 250))


def visible():
    image = background()
    image.paste((210, 230, 139), (718, 322, 744, 347))
    return image


class OrbPixelReferenceTest(unittest.TestCase):
    def test_depth_pack_is_only_uniform_stone_not_an_orb_or_shader_override(self):
        spec, = capture_runner.gui_resource_pack_specs("experience-orb-occlusion")
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            capture_runner.write_gui_resource_pack(root, spec)
            assets = [path.relative_to(root).as_posix() for path in root.rglob("*")
                      if path.is_file() and "assets" in path.parts]
            self.assertEqual(["assets/minecraft/textures/block/stone.png"], assets)
            with Image.open(root/assets[0]) as texture:
                self.assertEqual((16,16), texture.size)
                self.assertEqual([(256,(128,128,128,255))], texture.convert("RGBA").getcolors())

    def test_occlusion_requires_visible_block_and_rejects_even_one_leaked_pixel(self):
        empty = [background() for _ in range(5)]
        block = [visible() for _ in range(5)]
        self.assertFalse(occlusion_pixels(empty,empty,empty,empty,empty,empty)["passed"])
        self.assertTrue(occlusion_pixels(block,block,block,block,empty,empty)["passed"])
        leaked = [image.copy() for image in block]
        leaked[3].putpixel((730,330),(211,230,139))
        self.assertFalse(occlusion_pixels(leaked,block,block,block,empty,empty)["passed"])
        with self.assertRaises(ValueError):
            occlusion_pixels(block[:-1],block,block,block,empty,empty)

    def test_distance_inputs_require_actual_extracted_world_positions(self):
        document = self.document()
        fixture = document["experienceOrbFixture"]
        eye = (150.5,101.62000000476837,530.5)
        expected = [[e+(v-e)*(3+i)/3 for e,v in zip(eye,fixture["position"])] for i in range(5)]
        fixture.update(distanceSequence=True, configuredPosePositions=expected,
                       observedPosePositions=copy.deepcopy(expected), position=expected[-1])
        validate_document(document,hidden=False,native=True,distance_sequence=True)
        fixture["observedPosePositions"][2] = expected[0]
        with self.assertRaisesRegex(ValueError,"distance inputs"):
            validate_document(document,hidden=False,native=True,distance_sequence=True)

    def test_animation_requires_each_pose_age_and_fresh_native_generation(self):
        document = self.document()
        document["experienceOrbFixture"].update(age=12, extractedAge=12, ageBase=4,
            ageStep=2, poseIndex=4, observedPoseAges=[4,6,8,10,12])
        for index, receipt in enumerate(document["rustGalWorldExperienceOrbExecution"]):
            receipt["nativeResources"][0]["meshGeneration"] = str(index + 1)
        validate_document(document, hidden=False, native=True, age_step=2)
        stale = copy.deepcopy(document)
        stale["rustGalWorldExperienceOrbExecution"][3]["nativeResources"][0]["meshGeneration"] = "3"
        with self.assertRaisesRegex(ValueError, "stale appearance"):
            validate_document(stale, hidden=False, native=True, age_step=2)
        document["experienceOrbFixture"]["observedPoseAges"][2] = 4
        with self.assertRaisesRegex(ValueError, "animation inputs"):
            validate_document(document, hidden=False, native=True, age_step=2)

    def test_reload_requires_completion_presentations_and_actual_pack_removal(self):
        a, b = "file/mattmc-experience-orb-a", "file/mattmc-experience-orb-b"
        receipt = dict(schema="normal-world-resource-reload-v1", requested=True,
                       futureComplete=True, complete=True, presentations=2,
                       selectedBefore=["vanilla", b, a], selectedAtCapture=["vanilla", b])
        validate_pack_reload({"worldResourceReload": receipt}, a)
        for key, value in (("futureComplete", False), ("complete", False), ("presentations", 1),
                           ("selectedBefore", ["vanilla", a, b]),
                           ("selectedAtCapture", ["vanilla", b, a])):
            with self.assertRaises(ValueError):
                validate_pack_reload({"worldResourceReload": dict(receipt, **{key:value})}, a)
        with self.assertRaises(ValueError):
            validate_pack_reload({}, a)

    def test_pack_evidence_requires_selection_order_and_content_hashes(self):
        meta = {"gui_resource_pack_scenario": "experience-orb-replacement",
                "gui_resource_pack_selected": '["file/mattmc-experience-orb-b","file/mattmc-experience-orb-a"]',
                "gui_resource_pack_mattmc-experience-orb-a_sha256": "a" * 64,
                "gui_resource_pack_mattmc-experience-orb-b_sha256": "b" * 64}
        self.assertEqual(("b" * 64, "a" * 64), pack_signature(meta, "experience-orb-replacement"))
        for key in meta:
            incomplete = dict(meta)
            del incomplete[key]
            with self.assertRaises(ValueError):
                pack_signature(incomplete, "experience-orb-replacement")
        meta["gui_resource_pack_selected"] = '["file/mattmc-experience-orb-a","file/mattmc-experience-orb-b"]'
        with self.assertRaises(ValueError):
            pack_signature(meta, "experience-orb-replacement")

    def test_resource_pack_changes_only_orb_sheet_with_distinct_cutout_cells(self):
        images = []
        for variant in ("a", "b"):
            spec, = capture_runner.gui_resource_pack_specs("experience-orb-" + variant)
            with tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                capture_runner.write_gui_resource_pack(root, spec)
                textures = list(root.rglob("*.png"))
                self.assertEqual([root / "assets/minecraft/textures/entity/experience_orb.png"], textures)
                image = Image.open(textures[0]).convert("RGBA")
                self.assertEqual((64, 64), image.size)
                cells = [image.getpixel(((i % 4) * 16 + 8, (i // 4) * 16 + 8)) for i in range(11)]
                self.assertEqual(11, len(set(cells)))
                for i in range(11):
                    x, y = (i % 4) * 16, (i // 4) * 16
                    self.assertEqual(0, image.getpixel((x, y))[3])
                    self.assertEqual((255, 255, 255, 255), image.getpixel((x + 3, y + 3)))
                    self.assertEqual(255, image.getpixel((x + 8, y + 8))[3])
                images.append(image.tobytes())
        self.assertNotEqual(*images)
        specs = capture_runner.gui_resource_pack_specs("experience-orb-replacement")
        self.assertEqual(["mattmc-experience-orb-b", "mattmc-experience-orb-a"], [s["name"] for s in specs])
        with self.assertRaises(ValueError):
            capture_runner.experience_orb_sheet_png("unknown")

    def test_control_environment_checks_every_pose_and_rejects_missing_evidence(self):
        document = self.document()
        expected = environment_signature(document)
        for key in ("lightmapSemanticFingerprint", "weatherSemanticFingerprint", "shaderPack"):
            changed = copy.deepcopy(document)
            changed["captures"][4][key] = "different"
            self.assertNotEqual(expected, environment_signature(changed))
            del changed["captures"][4][key]
            with self.assertRaises(ValueError):
                environment_signature(changed)
        document["captures"].pop()
        with self.assertRaises(ValueError):
            environment_signature(document)

    def test_report_cannot_bypass_failed_manifest_or_ownership(self):
        def check(manifest, owns, expected):
            harness = SimpleNamespace(read_json=lambda path: manifest)
            modules = {"graphics_harness":harness,
                       "block_marker_reference":SimpleNamespace(ownership=lambda artifact: owns)}
            with patch.dict("sys.modules", modules):
                result = report("visible.json", "hidden.json")
            self.assertFalse(result["passed"])
            self.assertIn(expected, result["error"])
        check({"success":False}, True, "manifest gates")
        check({"success":True}, True, "equivalent cross-repository")
        check({"success":True,"cross_repository_visual_parity":{"pairs":[{
            "fixture_equivalence":{"status":"passed"},"current_artifact":"current.json"}]}},
            False, "Rust-owned")

    def test_report_rejects_wrong_presented_frame_even_with_typed_receipts(self):
        document = self.document()
        manifest = {"success":True,"cross_repository_visual_parity":{"pairs":[{
            "fixture_equivalence":{"status":"passed"},"current_artifact":"current.json"}]}}
        artifact = {"capture":{"whole_frame_gameplay_attachments":{"correlation_doc":{
            "same_acquired_presented_image":True,"deterministic_rendered_frame_index":999}}}}
        harness = SimpleNamespace(read_json=lambda path: artifact if path.name=="current.json" else manifest,
                                  deterministic_capture_document=lambda path: document)
        with patch.dict("sys.modules", {"graphics_harness":harness,
                "block_marker_reference":SimpleNamespace(ownership=lambda value:True)}):
            result = report("visible.json","hidden.json")
        self.assertFalse(result["passed"])
        self.assertIn("uncorrelated",result["error"])

    @staticmethod
    def document():
        return {
            "status":"complete",
            "experienceOrbFixture":{"fixture":"native-experience-orb-v1","complete":True,
                "hidden":False,"present":True,"icon":1,"value":3,"age":4,"entityId":2147483534,
                "position":[147.80143035574073,101.0991813627766,529.155950217724],
                "extractedIcon":1,"extractedAge":4,"extractedLight":15728752,"extractions":50},
            "captures":[{"poseName":pose,"renderedFrameIndex":10+i,"gameTime":6000,
                "lightmapSemanticFingerprint":"light","weatherSemanticFingerprint":"clear","shaderPack":"off.zip",
                "shaderEnabled":"false","backend":"vulkan","window":{"width":1280,"height":720},
                "position":{"x":150.5,"y":100.0,"z":530.5},"dimension":"minecraft:overworld",
                "observedYaw":105,"requestedYaw":105,"observedPitch":10,"requestedPitch":10,
                "captureMethod":"rust-vulkan-final-output"} for i,pose in enumerate(POSES)],
            "rustGalWorldExperienceOrbExecution":[{"deterministicFrameIndex":10+i,
                "gameplayFrameId":100+i,"submissionId":200+i,"route":"rust-vulkan-whole-frame",
                "quads":1,"nativeOrbCount":1,"nativeResourcesComplete":True,
                "nativeResources":[{"meshKey":"5715703444854013953","meshGeneration":"1",
                                    "entityId":2147483534}]} for i in range(5)]}

    def test_typed_capture_receipts_are_required_for_every_pose(self):
        document = self.document()
        validate_document(document, hidden=False, native=True)
        for field,value in (("nativeOrbCount",0),("nativeOrbCount",True),("quads",True),("nativeResourcesComplete",False),
                            ("deterministicFrameIndex",999),("submissionId",0)):
            bad = copy.deepcopy(document)
            bad["rustGalWorldExperienceOrbExecution"][3][field] = value
            with self.assertRaises(ValueError):
                validate_document(bad,hidden=False,native=True)
        bad = copy.deepcopy(document)
        bad["rustGalWorldExperienceOrbExecution"][0]["nativeResources"][0]["meshGeneration"] = "0"
        with self.assertRaises(ValueError):
            validate_document(bad,hidden=False,native=True)

    def test_wrong_inputs_or_java_capture_cannot_claim_native_parity(self):
        for field,value in (("captureMethod","java-screenshot"),("gameTime",18000),
                            ("shaderEnabled","true"),("observedYaw",0),("backend","opengl")):
            bad = self.document()
            bad["captures"][2][field] = value
            with self.assertRaises(ValueError):
                validate_document(bad,hidden=False,native=True)

    def test_hidden_receipts_must_show_no_extraction_or_orb_submission(self):
        document = self.document()
        fixture = document["experienceOrbFixture"]
        fixture.update(hidden=True,present=False,extractions=0,submits=0,callbacks=0)
        document["rustGalWorldExperienceOrbExecution"] = []
        validate_document(document,hidden=True,native=True)
        fixture["callbacks"] = 1
        with self.assertRaises(ValueError):
            validate_document(document,hidden=True,native=True)

    def test_identical_visible_effects_pass_all_five_poses(self):
        result = paired_effect([visible()]*5, [background()]*5, [visible()]*5, [background()]*5)
        self.assertTrue(result["passed"])
        self.assertEqual(650, result["poses"][0]["frozen_changed_pixels"])

    def test_identical_empty_images_are_not_rendering_proof(self):
        self.assertFalse(paired_effect(*([[background()]*5]*4))["passed"])

    def test_missing_frozen_or_current_orb_fails(self):
        self.assertFalse(paired_effect([visible()]*5, [background()]*5,
                                       [background()]*5, [background()]*5)["passed"])
        self.assertFalse(paired_effect([background()]*5, [background()]*5,
                                       [visible()]*5, [background()]*5)["passed"])

    def test_one_corrupted_pose_or_pixel_is_not_averaged_away(self):
        changed = visible()
        changed.putpixel((730, 333), (0, 0, 0))
        self.assertFalse(paired_effect([visible()]*4+[changed], [background()]*5,
                                       [visible()]*5, [background()]*5)["passed"])
        self.assertGreater(local_difference(changed, visible())["max_channel_abs"], 6)

    def test_partial_sequence_and_wrong_dimensions_rejected(self):
        with self.assertRaises(ValueError):
            paired_effect([visible()]*4, [background()]*5, [visible()]*5, [background()]*5)
        with self.assertRaises(ValueError):
            local_difference(Image.new("RGB", (640,360)), visible())
