"""Temporal foil oracle regressions; supplemental, not GPU parity acceptance."""
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "Common"))
import gui_foil_reference as reference


class TemporalFoilReferenceTests(unittest.TestCase):
    def test_moving_pixel_oracle_rejects_two_identical_stale_frames(self):
        import tempfile
        from PIL import Image
        import graphics_harness as harness
        import capture_runner
        sources={"minecraft:item/apple": self.SOURCE}
        boxes,_=harness.flat_item_witness_layout(2)
        rows={}
        with tempfile.TemporaryDirectory() as temporary:
            for ticks in (0,10000,40000):
                image=Image.new("RGB",(1280,720))
                for index,box in enumerate(boxes):
                    for x in range(32):
                        for y in range(32):
                            base=capture_runner.FLAT_ITEM_UV_COLORS[(y>=16)*2+(x>=16)]
                            color=(reference.expected_pixel(base,self.SOURCE,x/2,y/2,2,scaled_ticks=ticks)
                                   if index else [round(v*252/255) for v in base])
                            image.putpixel((box[0]+x-1,box[1]+y),tuple(color))
                path=Path(temporary)/f"{ticks}.png"
                image.save(path)
                row=harness.flat_item_pack_image_pair(path,path,"foil-moving",gui_scale=2,
                    foil_sources=sources,foil_timings=[[max(ticks,10000)]*8]*2)
                self.assertEqual(ticks!=0,row["passed"])
                rows[ticks]=row
        self.assertTrue(reference.temporal_change_evidence(rows[10000],rows[40000])["passed"])
        self.assertFalse(reference.temporal_change_evidence(rows[10000],rows[10000])["passed"])
        import copy
        stale=copy.deepcopy(rows[40000])
        for old,new in zip(rows[10000]["items"],stale["items"]):
            for a,b in zip(old["orientation_samples"],new["orientation_samples"]):
                b["observed"]=a["observed"]
        self.assertFalse(reference.temporal_change_evidence(rows[10000],stale)["passed"])
    def test_capture_timing_requires_exact_slots_and_running_unambiguous_inputs(self):
        import copy
        receipt={"enabled":True,"complete":True,"frameSequence":1,
                 "samples":[{"x":252+20*i,"y":341,"scaledTicks":10000+i} for i in range(8)]}
        self.assertEqual(list(range(10000,10008)),reference.timing_evidence(receipt,2,(1280,720)))
        for sample in receipt["samples"]:
            sample.pop("scaledTicks")
            sample.update(clockMillis=2500,speed=.5,strength=.5)
        self.assertEqual([10000]*8,reference.timing_evidence(receipt,2,(1280,720)))
        for change in ("missing","duplicate","stopped","mixed","incomplete"):
            bad=copy.deepcopy(receipt)
            if change=="missing": bad["samples"].pop()
            if change=="duplicate": bad["samples"][1]=bad["samples"][0]
            if change=="stopped": bad["samples"][0]["speed"]=0
            if change=="mixed": bad["samples"][0]["scaledTicks"]=10000
            if change=="incomplete": bad["complete"]=False
            with self.assertRaises(ValueError): reference.timing_evidence(bad,2,(1280,720))

    def test_observed_scaled_time_matches_clock_oracle(self):
        for clock in (0,12345,7500,27500,2**63-1):
            ticks=min(2**63-1,int(float(clock)*.5*8))
            self.assertEqual(reference.standard_uv((.25,.75),clock,.5),
                             reference.standard_uv((.25,.75),scaled_ticks=ticks))
    SOURCE = {"positions": [0,1,.5,1,1,.5,1,0,.5,0,0,.5],
              "atlasUvs": [.25,.75,.3125,.75,.3125,.8125,.25,.8125]}

    def test_matches_independently_evaluated_frozen_joml_golden_times(self):
        # Frozen Matrix4f translation/rotateZ/scale values, independently
        # evaluated with its JOML 1.10.5, not native renderer output.
        cases = (
            (12345, (0,0), (-.448909104,.646000028)),
            (12345, (.25,.75), (.478817225,6.902142525)),
            (12345, (-.5,2), (-7.166510582,15.708331108)),
            (13750, (1,1), (5.989276409,10.100980759)),
            (3750, (1,1), (6.352912903,9.767646790)),
            (2**63-1, (0,0), (-.780063629,.860233307)),
        )
        for clock, uv, expected in cases:
            with self.subTest(clock=clock, uv=uv):
                for actual, target in zip(reference.standard_uv(uv, clock, .5), expected):
                    self.assertAlmostEqual(target, actual, delta=.000002)

    def test_independent_axis_wraps_and_joint_period(self):
        x, y = reference.animation_offset(7500, .5)
        self.assertNotEqual(0, x)
        self.assertEqual(0, y)
        x, y = reference.animation_offset(27500, .5)
        self.assertEqual(0, x)
        self.assertNotEqual(0, y)
        self.assertEqual(reference.standard_uv((.25,.75),12345,.5),
                         reference.standard_uv((.25,.75),12345+82500,.5))
        self.assertNotEqual(reference.animation_offset(7499,.5),
                            reference.animation_offset(7500,.5))

    def test_actual_pixels_change_with_time_but_not_with_stopped_clock(self):
        def pixels(clock, speed, strength=.5):
            return tuple(tuple(reference.expected_pixel((100,100,100), self.SOURCE,
                x,y,2,clock,speed,strength)) for x,y in ((4,4),(12,4),(4,12),(12,12)))
        frames = [pixels(clock,.5) for clock in (0,3750,7500,12345,27500)]
        self.assertEqual(5,len(set(frames)))
        self.assertEqual(pixels(0,0),pixels(2**63-1,0))
        self.assertEqual(pixels(12345,.5),pixels(94845,.5))
        self.assertNotEqual(pixels(12345,.5,.25),pixels(12345,.5,.5))
        self.assertEqual(((99,99,99),)*4,pixels(12345,.5,0))

    def test_invalid_clock_speed_strength_and_float_overflow_fail_closed(self):
        for clock in (-1,2**63,1.5,True):
            with self.assertRaises(ValueError): reference.animation_offset(clock,.5)
        for speed in (-.01,1.01,float('nan'),float('inf'),True):
            with self.assertRaises(ValueError): reference.animation_offset(0,speed)
        for strength in (-.01,1.01,float('nan'),float('inf'),True):
            with self.assertRaises(ValueError):
                reference.expected_pixel((100,100,100),self.SOURCE,4,4,2,strength=strength)
        with self.assertRaises(ValueError): reference.standard_uv((1e100,0))


if __name__ == "__main__":
    unittest.main()
