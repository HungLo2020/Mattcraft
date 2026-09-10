import sys
import json
import tempfile
import unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/"Common"))
from gui_leaf_vertex_trace import f32,identity,compare


class LeafVertexTraceTest(unittest.TestCase):
    def test_shortest_float_spellings_compare_by_encoded_bits(self):
        self.assertEqual(identity([7.4126773]),identity([f32(7.4126773)]))
        self.assertNotEqual(identity([7.4126773]),identity([7.412678]))

    def test_explicit_atlas_translation_is_rounded_as_float(self):
        # Reference logs and Rust Debug use different shortest decimal precision.
        self.assertEqual(f32(f32(9.6054535)+63),f32(72.60545))

    def test_comparison_requires_complete_sources_and_equal_base_foil_vertices(self):
        vertices=[]; reference=["gui.item.raster-source slot=64,0,32 atlas=512x512 sprite=minecraft:block/oak_leaves_bushy"]
        for face_index,face in enumerate(('west','east','north','south')):
            for index in range(4):
                x=face_index*4+index
                vertices.append([x,0,0,x,1,2,0,0])
                reference.append(f"gui.leaf.vertex item=292,341 face={face} vertex={index} source={x},0,0 encoded={x+63},0,2 uv=0,0")
        record=dict(bounds=[292,341,308,357],extent=[34,34],vertices=vertices)
        with tempfile.TemporaryDirectory() as temporary:
            current=Path(temporary)/'current.log'; frozen=Path(temporary)/'frozen.log'
            def write_current(foil):
                current.write_text('\n'.join('[gui.mesh.vertex-trace] '+json.dumps(dict(record,material=material,vertices=data))
                    for material,data in [('Cutout',vertices),('Glint',foil)]))
            write_current(vertices);frozen.write_text('\n'.join(reference))
            self.assertEqual(compare(current,frozen)['max_abs_delta'],[0,0,0])
            frozen.write_text('\n'.join(reference[:-1]))
            with self.assertRaises(ValueError): compare(current,frozen)
            frozen.write_text('\n'.join(reference))
            foil=[row.copy() for row in vertices];foil[0][5]+=1
            write_current(foil)
            with self.assertRaises(ValueError): compare(current,frozen)


if __name__=="__main__": unittest.main()
