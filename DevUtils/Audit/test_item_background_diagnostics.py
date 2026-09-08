import unittest
from PIL import Image
from DevUtils.Common import item_background_diagnostics as diagnostic


class ItemBackgroundDiagnosticsTests(unittest.TestCase):
    def test_reference_and_current_routes_are_not_interchangeable(self):
        reference = {"mode":{"name":"frozen-opengl-shaders-off"},"implementation_attribution":"java-opengl"}
        current = {"mode":{"name":"current-rust-vulkan-shaders-off"},"implementation_attribution":"rust-vulkan"}
        diagnostic.validate_modes(reference,current)
        for bad in (dict(reference,implementation_attribution="java-vulkan"),current,{}):
            with self.assertRaises(ValueError):
                diagnostic.validate_modes(bad,current)
        with self.assertRaises(ValueError):
            diagnostic.validate_modes(reference,dict(current,implementation_attribution="java-vulkan"))

    def test_attachment_identity_and_ownership_are_required(self):
        attachments = dict(gameplay_frame_id=7,correlation_id=8,deterministic_rendered_frame_index=9,
                           gal_submission_id=10,synthetic_shader_scene=False,java_iris_participation=False,
                           png_row_origin="top-left")
        correlation = dict(attachments,java_vulkan_frame_execution=False,
                           rust_whole_frame_presenter=True,same_acquired_presented_image=True)
        diagnostic.validate_attachment_correlation(attachments,correlation,9)
        for changed in (dict(attachments,gal_submission_id=11), dict(attachments,synthetic_shader_scene=True),
                        dict(attachments,java_iris_participation=True), dict(attachments,png_row_origin="bottom-left")):
            with self.assertRaises(ValueError):
                diagnostic.validate_attachment_correlation(changed,correlation,9)
        with self.assertRaises(ValueError):
            diagnostic.validate_attachment_correlation(attachments,correlation,8)

    def samples(self):
        return {point: (10,20,30) for point in diagnostic.probe_points()}

    def test_correlated_parser_rejects_missing_duplicate_wrong_frame_and_channels(self):
        lines = [f"FrozenCelestialStagePixel frame=42 stage=main-after-post screen=({x}, {y}) "
                 "texture=7 rgba=(10,20,30,255) post=null" for x,y in sorted(diagnostic.probe_points())]
        self.assertEqual(self.samples(), diagnostic.read_stage_samples(lines,42,"main-after-post"))
        for bad in (lines[:-1], lines + lines[:1], [line.replace("frame=42", "frame=41") for line in lines],
                    [line.replace("(10,20,30,255)", "(256,20,30,255)") for line in lines]):
            with self.assertRaises(ValueError):
                diagnostic.read_stage_samples(bad,42,"main-after-post")

    def test_window_probe_cannot_be_substituted_for_main_attachment_samples(self):
        lines = [f"FrozenCelestialStagePixel frame=42 stage=window-after-blit screen=({x}, {y}) "
                 "texture=0 rgba=(10,20,30,255) post=null" for x,y in sorted(diagnostic.probe_points())]
        self.assertEqual(self.samples(), diagnostic.read_stage_samples(lines,42,"window-after-blit"))
        with self.assertRaises(ValueError):
            diagnostic.read_stage_samples(lines,42,"main-after-gui")
        with self.assertRaises(ValueError):
            diagnostic.read_stage_samples(lines,41,"window-after-blit")

    def test_attribution_does_not_erase_world_difference_or_accept_wrong_presentation(self):
        world = Image.new("RGB", (1280,720), (10,20,30))
        reference = world.copy()
        gui = world.copy()
        world.putpixel((502,675), (17,20,30))
        gui.putpixel((502,675), (13,20,30))
        report = diagnostic.compare_background_samples(self.samples(),self.samples(),world,gui,reference)
        self.assertTrue(report["diagnostic_only"])
        self.assertNotIn("passed", report)
        paper = next(p for p in report["patches"] if p["item"] == "paper" and p["source_alpha"] == 0)
        self.assertEqual([7/9,0,0],paper["world_mean_rgb_abs"])
        self.assertEqual([3/9,0,0],paper["presented_mean_rgb_abs"])
        reference.putpixel((502,675),(11,20,30))
        with self.assertRaises(ValueError):
            diagnostic.compare_background_samples(self.samples(),self.samples(),world,gui,reference)


if __name__ == "__main__":
    unittest.main()
