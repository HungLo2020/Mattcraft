import sys
import unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'Common'))
from PIL import Image
from shield_item_reference import PROBES, BASE_PROBES, PATTERN_PROBES, PATTERN_FOIL_PROBES, compare_images, compare_pattern_images

class ShieldReferenceTests(unittest.TestCase):
    def test_pattern_foil_must_cover_colored_layers_not_only_the_held_model(self):
        reference=Image.new('RGB',(1280,720))
        for _,x,y,pixels in PATTERN_FOIL_PROBES:
            for i,color in enumerate(pixels): reference.putpixel((x-1+i%3,y-1+i//3),tuple(color))
        self.assertTrue(compare_pattern_images(reference,reference,foil=True)['passed'])
        wrong=reference.copy()
        for _,x,y,pixels in PATTERN_PROBES[:5]:
            for i,color in enumerate(pixels): wrong.putpixel((x-1+i%3,y-1+i//3),tuple(color))
        self.assertFalse(compare_pattern_images(reference,wrong,foil=True)['passed'])
        empty=Image.new('RGB',(1280,720))
        self.assertFalse(compare_pattern_images(empty,empty,foil=True)['passed'])
    def test_pattern_colors_and_empty_frames_are_not_accepted_as_equivalent(self):
        reference=Image.new('RGB',(1280,720))
        for _,x,y,pixels in PATTERN_PROBES:
            for i,color in enumerate(pixels): reference.putpixel((x-1+i%3,y-1+i//3),tuple(color))
        self.assertTrue(compare_pattern_images(reference,reference)['passed'])
        wrong=reference.copy()
        wrong.paste((223,190,53),(394,687,397,690)) # missing red cross
        self.assertFalse(compare_pattern_images(reference,wrong)['passed'])
        self.assertFalse(compare_pattern_images(reference,self.reference(False))['passed'])
        empty=Image.new('RGB',(1280,720))
        self.assertFalse(compare_pattern_images(empty,empty)['passed'])
    def reference(self, foil=True):
        image=Image.new('RGB',(1280,720))
        for _,x,y,pixels in (PROBES if foil else BASE_PROBES):
            for i,color in enumerate(pixels):
                image.putpixel((x-1+i%3,y-1+i//3),tuple(color))
        return image
    def test_base_control_is_distinct_from_foil_and_cannot_pass_with_missing_icon(self):
        base=self.reference(False)
        foil=self.reference(True)
        self.assertTrue(compare_images(base,base,foil=False)['passed'])
        self.assertFalse(compare_images(base,foil,foil=False)['passed'])
        self.assertFalse(compare_images(foil,base)['passed'])
        missing=base.copy()
        missing.paste((0,0,0),(370,662,424,716))
        self.assertFalse(compare_images(base,missing,foil=False)['passed'])
        empty=Image.new('RGB',(1280,720))
        self.assertFalse(compare_images(empty,empty,foil=False)['passed'])
    def test_exact_reference_and_missing_icon_negative_control(self):
        reference=self.reference()
        self.assertTrue(compare_images(reference,reference)['passed'])
        missing=reference.copy()
        missing.paste((0,0,0),(370,662,424,716))
        self.assertFalse(compare_images(reference,missing)['passed'])
    def test_identical_empty_frames_and_wrong_dimensions_cannot_pass(self):
        empty=Image.new('RGB',(1280,720))
        self.assertFalse(compare_images(empty,empty)['passed'])
        with self.assertRaises(ValueError): compare_images(empty,Image.new('RGB',(640,360)))
    def test_wrong_held_rim_cannot_hide_in_whole_image_average(self):
        reference=self.reference()
        wrong=reference.copy()
        wrong.paste((255,255,255),(799,474,802,477))
        self.assertFalse(compare_images(reference,wrong)['passed'])
