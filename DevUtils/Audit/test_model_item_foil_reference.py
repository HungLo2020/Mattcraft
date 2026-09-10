import sys
import unittest
from unittest.mock import patch
from pathlib import Path
from PIL import Image
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/"Common"))
import model_item_foil_reference as reference
import graphics_harness as harness

class ModelFoilReferenceTest(unittest.TestCase):
    def golden(self):
        image=Image.new("RGB",(1280,720))
        for name,x,y,pixels in reference.PROBES:
            for index,pixel in enumerate(pixels):
                image.putpixel((x-1+index%3,y-1+index//3),tuple(pixel))
        return image

    def test_exact_reference_and_three_level_single_pixel_mutation(self):
        baseline=self.golden()
        self.assertTrue(reference.compare_images(baseline,baseline)["passed"])
        for name,x,y,pixels in reference.PROBES:
            altered=baseline.copy()
            r,g,b=altered.getpixel((x,y))
            altered.putpixel((x,y),(r-3,g,b))
            self.assertFalse(reference.compare_images(baseline,altered)["passed"],name)
            self.assertFalse(reference.compare_images(altered,altered)["passed"],name)

    def test_missing_models_both_backends_and_wrong_extent_reject(self):
        empty=Image.new("RGB",(1280,720))
        self.assertFalse(reference.compare_images(empty,empty)["passed"])
        self.assertFalse(reference.compare_images(self.golden(),empty)["passed"])
        with self.assertRaises(ValueError):
            reference.compare_images(self.golden(),Image.new("RGB",(640,360)))

    def test_aggregate_gate_requires_pair_images_scale_and_observed_fixture(self):
        self.assertFalse(harness.model_item_foil_parity_report({},"model-foil")["passed"])
        self.assertTrue(harness.model_item_foil_parity_report({},"flat-items")["passed"])
        receipt={"fixture":"held-trident-foil-v1","selectedSlot":1,"mainHand":"minecraft:trident",
                 "count":1,"foil":True,"usingItem":False,"speed":0.0,"strength":0.5,"complete":True}
        doc={"hotbarItemFixture":"model-foil","modelFoilFixture":receipt}
        pair={"baseline_artifact":"frozen/artifact.json","current_artifact":"current/artifact.json",
              "baseline_image":"frozen.png","current_image":"current.png"}
        with patch.object(harness,"deterministic_capture_document",return_value=doc), \
             patch.object(harness,"latest_capture_meta_path",return_value=Path("meta.txt")), \
             patch.object(harness,"read_key_values",return_value={"forced_option_guiScale":"3"}) as meta, \
             patch.object(reference,"compare_paths",return_value={"passed":True}) as pixels:
            report=lambda: harness.model_item_foil_parity_report({"pairs":[pair]},"model-foil")
            self.assertTrue(report()["passed"])
            pixels.assert_called_once_with("frozen.png","current.png")
            pixels.return_value={"passed":False}
            self.assertFalse(report()["passed"])
            pixels.return_value={"passed":True}
            meta.return_value={"forced_option_guiScale":"2"}
            self.assertFalse(report()["passed"])
            meta.return_value={"forced_option_guiScale":"3"}
            receipt["foil"]=False
            self.assertFalse(report()["passed"])
            receipt["foil"]=True
            del pair["current_image"]
            self.assertFalse(report()["passed"])

if __name__ == "__main__": unittest.main()
