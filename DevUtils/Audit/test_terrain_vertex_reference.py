import struct
import unittest
from DevUtils.Common.terrain_vertex_reference import compare, decode


def bits(value):
    return struct.unpack('<I', struct.pack('<f', value))[0]


class VertexReferenceTest(unittest.TestCase):
    def records(self):
        return [[0, 0, *map(bits, (x, y, z, 1.0)), *map(bits, (1., 2., 3., 4.)), bits(.5), bits(.25)]
                for x in (-3.5, -2.5) for y in (-.62, .38) for z in (-1.5, -.5)]

    def test_requires_all_corners_and_untruncated_complete_payload(self):
        records = self.records()
        self.assertEqual(len(decode(8, False, records)), 8)
        for count, truncated, rows in ((0, False, []), (129, True, records), (8, False, records[:-1]),
                                       (7, False, records[:-1])):
            with self.assertRaises(ValueError): decode(count, truncated, rows)

    def test_rounding_for_identity_does_not_hide_one_bit_difference(self):
        records = self.records()
        left = decode(8, False, records)
        records[0][6] += 1
        result = compare(left, decode(8, False, records))
        self.assertFalse(result['clip_bits_equal'])
        self.assertTrue(result['world_bits_equal'])
        self.assertFalse(result['visual_parity_accepted'])

    def test_duplicate_invocations_preserve_observed_value_sets(self):
        records = self.records()
        self.assertTrue(compare(decode(8, False, records), decode(16, False, records * 2))['clip_bits_equal'])


if __name__ == '__main__':
    unittest.main()
