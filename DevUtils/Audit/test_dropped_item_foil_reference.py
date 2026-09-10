import sys
import unittest
from pathlib import Path
from PIL import Image

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'Common'))
from dropped_item_foil_reference import PROBES, STACK_PROBES, compare_images


class DroppedFoilReferenceTests(unittest.TestCase):
    def fixture(self, probes=PROBES):
        image = Image.new('RGB', (1280,720))
        for probe in probes:
            x,y = probe['center']
            for point,color in zip(((px,py) for py in range(y-1,y+2) for px in range(x-1,x+2)), probe['pixels']):
                image.putpixel(point,tuple(color))
        return image

    def test_exact_reference_passes_and_missing_foil_in_either_or_both_rejects(self):
        golden = self.fixture()
        self.assertTrue(compare_images(golden,golden)['passed'])
        missing = golden.copy()
        for probe in PROBES:
            x,y=probe['center']
            for py in range(y-1,y+2):
                for px in range(x-1,x+2):
                    r,g,b = missing.getpixel((px,py))
                    missing.putpixel((px,py),(r,g,max(0,b-31)))
        self.assertFalse(compare_images(golden,missing)['passed'])
        self.assertFalse(compare_images(missing,missing)['passed'])

    def test_single_bad_pixel_and_unreferenced_stack_reject(self):
        golden=self.fixture()
        candidate=golden.copy()
        x,y=PROBES[-1]['center']
        r,g,b=candidate.getpixel((x,y))
        candidate.putpixel((x,y),(r+3,g,b))
        self.assertFalse(compare_images(golden,candidate)['passed'])
        with self.assertRaises(ValueError): compare_images(golden,golden,2)
        with self.assertRaises(ValueError): compare_images(golden,golden,True)
        with self.assertRaises(ValueError): compare_images(golden,Image.new('RGB',(1,1)))

    def test_stack_reference_rejects_missing_copies_and_excess_glint(self):
        golden = self.fixture(STACK_PROBES)
        self.assertTrue(compare_images(golden,golden,64)['passed'])
        single = self.fixture()
        self.assertFalse(compare_images(golden,single,64)['passed'])
        self.assertFalse(compare_images(single,single,64)['passed'])
        doubled = golden.copy()
        x,y=STACK_PROBES[0]['center']
        r,g,b=doubled.getpixel((x,y))
        doubled.putpixel((x,y),(r,g,b+31))
        self.assertFalse(compare_images(golden,doubled,64)['passed'])
