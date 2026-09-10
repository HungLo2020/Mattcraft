import sys
import unittest
from pathlib import Path
from PIL import Image
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "Common"))
from held_item_foil_reference import PROBES, PATTERN_PROBES, compare_images, compare_pattern_images, wide_source_evidence
from held_item_foil_reference import WIDE_PROBES, compare_wide_images
from held_item_foil_reference import compare_moving_images, moving_change_evidence
from held_item_foil_reference import MIXED_GROUND_PROBES, compare_mixed_ground_images


class HeldItemFoilReferenceTest(unittest.TestCase):
    def test_mixed_ground_rejects_missing_foil_even_when_both_images_match(self):
        a=Image.new("RGB",(1280,720)); missing=a.copy()
        base=((133,21,38),(20,119,56),(23,40,132),(132,112,18))
        for (_,x,y,color),unfoiled in zip(MIXED_GROUND_PROBES,base):
            for py in range(y-1,y+2):
                for px in range(x-1,x+2):
                    a.putpixel((px,py),color); missing.putpixel((px,py),unfoiled)
        self.assertTrue(compare_mixed_ground_images(a,a)["passed"])
        self.assertFalse(compare_mixed_ground_images(a,missing)["passed"])
        self.assertFalse(compare_mixed_ground_images(missing,missing)["passed"])
        wrong=a.copy(); wrong.putpixel((433,348),(147,29,69))
        self.assertFalse(compare_mixed_ground_images(a,wrong)["passed"])

    def test_sampler_wide_pair_covers_all_fixed_cells_and_rejects_missing_faces(self):
        image=Image.new("RGB",(1280,720))
        for _,x,y,color in WIDE_PROBES:
            for py in range(y-1,y+2):
                for px in range(x-1,x+2): image.putpixel((px,py),color)
        result=compare_moving_images(image,image,wide=True)
        self.assertTrue(result["passed"])
        self.assertEqual(24,len(result["probes"]))
        wrong=image.copy(); wrong.putpixel((720,410),(159,44,51))
        self.assertFalse(compare_moving_images(image,wrong,wide=True)["passed"])
        missing=Image.new("RGB",(1280,720))
        self.assertFalse(compare_moving_images(missing,missing,wide=True)["passed"])

    def moving_fixture(self, offset=0):
        image=Image.new("RGB",(1280,720))
        for _,x,y,color in PATTERN_PROBES:
            for py in range(y-1,y+2):
                for px in range(x-1,x+2):
                    image.putpixel((px,py),tuple(c+offset for c in color))
        return image

    def test_moving_pair_rejects_missing_faces_and_single_bad_pixel(self):
        a=self.moving_fixture(); b=a.copy()
        self.assertTrue(compare_moving_images(a,b)["passed"])
        b.putpixel((760,420),(147,38,49))
        self.assertFalse(compare_moving_images(a,b)["passed"])
        empty=Image.new("RGB",(1280,720))
        self.assertFalse(compare_moving_images(empty,empty)["passed"])

    def test_temporal_hand_gate_rejects_matching_frozen_animation(self):
        before=compare_moving_images(self.moving_fixture(),self.moving_fixture())
        after=compare_moving_images(self.moving_fixture(10),self.moving_fixture(10))
        self.assertTrue(moving_change_evidence(before,after)["passed"])
        self.assertFalse(moving_change_evidence(before,before)["passed"])
        stale=compare_moving_images(self.moving_fixture(10),self.moving_fixture())
        with self.assertRaises(ValueError): moving_change_evidence(before,stale)

    def test_temporal_hand_gate_requires_both_faces_to_change(self):
        a=self.moving_fixture(); b=self.moving_fixture()
        for name,x,y,color in PATTERN_PROBES:
            if name.startswith("red"):
                for py in range(y-1,y+2):
                    for px in range(x-1,x+2): b.putpixel((px,py),tuple(c+10 for c in color))
        result=moving_change_evidence(compare_moving_images(a,a),compare_moving_images(b,b))
        self.assertFalse(result["passed"])
        self.assertEqual({"red":True,"green":False},result["faces_changed"])

    def test_wide_grid_rejects_constant_foil_and_changed_sampling(self):
        frozen = Image.new("RGB", (1280,720))
        constant = frozen.copy()
        for _,x,y,color in WIDE_PROBES:
            for py in range(y-1,y+2):
                for px in range(x-1,x+2):
                    frozen.putpixel((px,py),color)
                    constant.putpixel((px,py),(150,40,55) if x<1058 else (33,135,70))
        self.assertTrue(compare_wide_images(frozen,frozen)["passed"])
        self.assertFalse(compare_wide_images(frozen,constant)["passed"])
        self.assertFalse(compare_wide_images(constant,constant)["passed"])
        wrong = frozen.copy()
        _,x,y,color = WIDE_PROBES[0]
        wrong.putpixel((x,y),(color[0]+3,color[1],color[2]))
        self.assertFalse(compare_wide_images(frozen,wrong)["passed"])

    def test_repeat_witness_requires_wide_observed_uvs(self):
        source = {"positions":[0,1,.53125,0,0,.53125,1,0,.53125,1,1,.53125],
                  "atlasUvs":[.28125,.4140625,.28125,.6640625,.40625,.6640625,.40625,.4140625]}
        self.assertTrue(wide_source_evidence(source)["passed"])
        narrow = dict(source, atlasUvs=[.375,.8359375,.375,.8515625,.3828125,.8515625,.3828125,.8359375])
        self.assertFalse(wide_source_evidence(narrow)["passed"])
        with self.assertRaises(ValueError): wide_source_evidence({})

    def fixture(self, glint=True):
        image = Image.new("RGB", (1280,720), (175,205,250))
        for name,x,y,color in PROBES:
            if not glint:
                color = tuple(c-d for c,d in zip(color,(1,4,9)))
            for py in range(y-1,y+2):
                for px in range(x-1,x+2):
                    image.putpixel((px,py),color)
        return image

    def test_complete_foil_passes(self):
        self.assertTrue(compare_images(self.fixture(), self.fixture())["passed"])

    def test_missing_foil_fails(self):
        self.assertFalse(compare_images(self.fixture(), self.fixture(False))["passed"])

    def test_matching_missing_foil_cannot_pass(self):
        self.assertFalse(compare_images(self.fixture(False), self.fixture(False))["passed"])

    def test_single_bad_pixel_fails(self):
        current = self.fixture()
        _,x,y,color = PROBES[0]
        current.putpixel((x,y),(color[0]+3,color[1],color[2]))
        self.assertFalse(compare_images(self.fixture(),current)["passed"])

    def test_wrong_extent_fails(self):
        with self.assertRaises(ValueError):
            compare_images(self.fixture(),Image.new("RGB", (640,360)))

    def test_pattern_requires_both_correct_baseline_and_current_samples(self):
        frozen = Image.new("RGB", (1280,720))
        for _,x,y,color in PATTERN_PROBES:
            for py in range(y-1,y+2):
                for px in range(x-1,x+2): frozen.putpixel((px,py),color)
        self.assertTrue(compare_pattern_images(frozen,frozen)["passed"])
        current = frozen.copy()
        _,x,y,color = PATTERN_PROBES[0]
        current.putpixel((x,y),(color[0]+3,color[1],color[2]))
        self.assertFalse(compare_pattern_images(frozen,current)["passed"])
        self.assertFalse(compare_pattern_images(current,current)["passed"])
        self.assertFalse(compare_pattern_images(frozen,self.fixture())["passed"])
