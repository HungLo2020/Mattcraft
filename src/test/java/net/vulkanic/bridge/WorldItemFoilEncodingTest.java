package net.vulkanic.bridge;

import java.lang.foreign.Arena;
import java.lang.foreign.MemorySegment;
import java.lang.foreign.ValueLayout;
import org.junit.jupiter.api.Test;
import static net.vulkanic.bridge.VulkanicGalBridge.*;
import static org.junit.jupiter.api.Assertions.*;

class WorldItemFoilEncodingTest {
    private static WorldMeshInstanceRecord instance(int stratum) {
        return new WorldMeshInstanceRecord(stratum, 17, 1, 0, 2, 1, 0, -1,
            new float[]{1,0,0,0,0,1,0,0,0,0,1,0,0,0,0,1}, 128,128,0,0,0,0,-1);
    }

    @Test void semanticCopyDoesNotTransformGeometryOrQuantizeStrength() {
        var source = instance(WORLD_MESH_ENTITY_STRATUM);
        var foil = new StandardItemFoilRecord(12345, 0.125, 0.1234567F);
        var copied = source.withItemFoil(foil);
        assertNull(source.itemFoil());
        assertEquals(foil, copied.itemFoil());
        assertArrayEquals(source.transform(), copied.transform());
        assertEquals(source.colorArgb(), copied.colorArgb());
        assertThrows(IllegalArgumentException.class, () -> instance(60).withItemFoil(foil));
    }

    @Test void exportedNativeLayoutCarriesExactAndCanonicalAbsentFields() throws Exception {
        try (var bridge = VulkanicGalBridge.create("rust-vulkan"); var arena = Arena.ofConfined()) {
            assertEquals(54, ABI_VERSION);
            var layout = Struct.WORLD_MESH_INSTANCE_RECORD;
            var item = arena.allocate(layout.byteSize(), 8);
            var encode = VulkanicGalBridge.class.getDeclaredMethod("encodeWorldItemFoil", MemorySegment.class, StandardItemFoilRecord.class);
            encode.setAccessible(true);
            encode.invoke(null, item, new StandardItemFoilRecord(12345, 0.125, 0.1234567F));
            assertEquals(1, item.get(ValueLayout.JAVA_INT, layout.offset(20)));
            assertEquals(12345, item.get(ValueLayout.JAVA_LONG, layout.offset(21)));
            assertEquals(0.125, item.get(ValueLayout.JAVA_DOUBLE, layout.offset(22)));
            assertEquals(0.1234567F, item.get(ValueLayout.JAVA_FLOAT, layout.offset(23)));
            var entity = new StandardItemFoilRecord(12345, 0.125, 0.1234567F, StandardFoilKind.ENTITY);
            encode.invoke(null, item, entity);
            assertEquals(2, item.get(ValueLayout.JAVA_INT, layout.offset(20)));
            assertEquals(12345, item.get(ValueLayout.JAVA_LONG, layout.offset(21)));
            assertEquals(0.125, item.get(ValueLayout.JAVA_DOUBLE, layout.offset(22)));
            assertEquals(0.1234567F, item.get(ValueLayout.JAVA_FLOAT, layout.offset(23)));
            assertEquals(entity, instance(WORLD_MESH_ENTITY_STRATUM).withItemFoil(entity).itemFoil());
            // Reuse dirty storage: absent data must be actively zeroed.
            encode.invoke(null, item, null);
            assertEquals(0, item.get(ValueLayout.JAVA_INT, layout.offset(20)));
            assertEquals(0, item.get(ValueLayout.JAVA_LONG, layout.offset(21)));
            assertEquals(0, item.get(ValueLayout.JAVA_LONG, layout.offset(22)));
            assertEquals(0, item.get(ValueLayout.JAVA_INT, layout.offset(23)));
        }
    }
}
