package net.vulkanic.bridge;

import java.lang.foreign.MemorySegment;
import java.lang.foreign.ValueLayout;
import java.util.List;
import org.junit.jupiter.api.Test;
import static net.vulkanic.bridge.VulkanicGalBridge.*;
import static org.junit.jupiter.api.Assertions.*;

class GuiItemFoilEncodingTest {
    @Test void inventoryLightingSurvivesNativeEncodingAndScheduling() throws Exception {
        var source=batch(1);
        var inventory=new GuiMeshBatchRecord(source.stratum(),source.layerIndex(),source.materialMode(),3,
            source.assetId(),source.sequence(),source.alphaCutoff(),source.modelTransform(),source.guiPose(),
            source.left(),source.top(),source.right(),source.bottom(),source.guiWidth(),source.guiHeight(),
            source.renderWidth(),source.renderHeight(),source.guardPixels(),source.vertices(),source.indices());
        assertEquals(3,inventory.withSequence(42).lightingMode());
        try(var bridge=VulkanicGalBridge.create("rust-vulkan")) {
            var encode=VulkanicGalBridge.class.getDeclaredMethod("encodeGuiMeshBatches",List.class);
            encode.setAccessible(true);
            var encoded=(MemorySegment)encode.invoke(bridge,List.of(inventory));
            assertEquals(3,encoded.get(ValueLayout.JAVA_INT,Struct.GUI_MESH_BATCH_REQUEST.offset(4)));
        }
    }

    private static GuiMeshBatchRecord flatBatch(int scale, int width, int guard) {
        var source=batch(4).withItemFoil(new StandardItemFoilRecord(12345,0.5,0.5F));
        return new GuiMeshBatchRecord(source.stratum(),source.layerIndex(),source.materialMode(),source.lightingMode(),
            source.assetId(),source.sequence(),source.alphaCutoff(),source.modelTransform(),source.guiPose(),
            source.left(),source.top(),source.right(),source.bottom(),source.guiWidth(),source.guiHeight(),
            width,0,guard,0,0,0,0,0,source.vertices(),source.indices(),source.itemFoil(),scale);
    }

    @Test void flatScaleUsesNativeLayoutWithoutJavaRasterExtent() throws Exception {
        var flat=flatBatch(2,0,0).withSequence(33);
        assertEquals(2,flat.itemRasterScale());
        assertEquals(0,flat.renderWidth());
        assertEquals(2,flat.withItemFoil(new StandardItemFoilRecord(0,0,0.5F)).itemRasterScale());
        assertThrows(IllegalArgumentException.class,()->flatBatch(2,32,0));
        assertThrows(IllegalArgumentException.class,()->flatBatch(2,0,1));
        assertThrows(IllegalArgumentException.class,()->flatBatch(-1,0,0));
        try(var bridge=VulkanicGalBridge.create("rust-vulkan")) {
            var encode=VulkanicGalBridge.class.getDeclaredMethod("encodeGuiMeshBatches",List.class);
            encode.setAccessible(true);
            var encoded=(MemorySegment)encode.invoke(bridge,List.of(flat,batch(4)));
            var layout=Struct.GUI_MESH_BATCH_REQUEST;
            assertEquals(2,encoded.get(ValueLayout.JAVA_INT,layout.offset(31)));
            assertEquals(0,encoded.get(ValueLayout.JAVA_INT,layout.offset(17)));
            assertEquals(0,encoded.get(ValueLayout.JAVA_INT,layout.offset(18)));
            assertEquals(0,encoded.get(ValueLayout.JAVA_INT,layout.offset(19)));
            assertEquals(0,encoded.get(ValueLayout.JAVA_INT,layout.byteSize()+layout.offset(31)));
        }
    }

    private static GuiMeshBatchRecord batch(int material) {
        float[] identity = {1,0,0,0, 0,1,0,0, 0,0,1,0, 0,0,0,1};
        var vertex = new GuiMeshVertexRecord(new float[]{0,0,0}, new float[]{0.25F,0.75F},
            new float[]{0.9F,0.1F}, 0xff123456, 0x007f0000);
        return new GuiMeshBatchRecord(420,0,material,1,7,9,0.0F,identity,
            new float[]{1,0,0,1,0,0},0,0,16,16,320,180,34,34,1,
            List.of(vertex,vertex,vertex),List.of(0,1,2));
    }

    @Test void semanticFoilIsImmutableValidatedAndSurvivesScheduling() {
        var foil = new StandardItemFoilRecord(12345,0.5,0.5F);
        assertEquals(foil,batch(4).withItemFoil(foil).withSequence(22).itemFoil());
        assertNull(batch(4).itemFoil());
        assertThrows(IllegalArgumentException.class,()->batch(1).withItemFoil(foil));
        assertThrows(IllegalArgumentException.class,()->new StandardItemFoilRecord(-1,0.5,0.5F));
        for(double speed:new double[]{Double.NaN,Double.POSITIVE_INFINITY,-0.1,1.1})
            assertThrows(IllegalArgumentException.class,()->new StandardItemFoilRecord(0,speed,0.5F));
        for(float strength:new float[]{Float.NaN,Float.POSITIVE_INFINITY,-0.1F,1.1F})
            assertThrows(IllegalArgumentException.class,()->new StandardItemFoilRecord(0,0.5,strength));
    }

    @Test void actualNativeLayoutReceivesOriginalUvsAndUnquantizedFoilInputs() throws Exception {
        try(var bridge=VulkanicGalBridge.create("rust-vulkan")) {
            var encode=VulkanicGalBridge.class.getDeclaredMethod("encodeGuiMeshBatches",List.class);
            encode.setAccessible(true);
            var layout=Struct.GUI_MESH_BATCH_REQUEST;
            var encoded=(MemorySegment)encode.invoke(bridge,List.of(
                batch(4).withItemFoil(new StandardItemFoilRecord(12345,0.125,0.5F)).withSequence(22),batch(4)));
            assertEquals(1,encoded.get(ValueLayout.JAVA_INT,layout.offset(27)));
            assertEquals(12345,encoded.get(ValueLayout.JAVA_LONG,layout.offset(28)));
            assertEquals(0.125,encoded.get(ValueLayout.JAVA_DOUBLE,layout.offset(29)));
            assertEquals(0.5F,encoded.get(ValueLayout.JAVA_FLOAT,layout.offset(30)));
            assertEquals(22,encoded.get(ValueLayout.JAVA_LONG,layout.offset(6)));
            var vertices=encoded.get(ValueLayout.ADDRESS,layout.offset(25)).reinterpret(Struct.GUI_MESH_VERTEX.byteSize()*3L);
            assertArrayEquals(new float[]{0.25F,0.75F},vertices.asSlice(Struct.GUI_MESH_VERTEX.offset(1),8).toArray(ValueLayout.JAVA_FLOAT));
            assertArrayEquals(new float[]{0.9F,0.1F},vertices.asSlice(Struct.GUI_MESH_VERTEX.offset(2),8).toArray(ValueLayout.JAVA_FLOAT));
            var absent=encoded.asSlice(layout.byteSize(),layout.byteSize());
            assertEquals(0,absent.get(ValueLayout.JAVA_INT,layout.offset(27)));
            assertEquals(0,absent.get(ValueLayout.JAVA_LONG,layout.offset(28)));
            assertEquals(0.0,absent.get(ValueLayout.JAVA_DOUBLE,layout.offset(29)));
            assertEquals(0.0F,absent.get(ValueLayout.JAVA_FLOAT,layout.offset(30)));
        }
    }
}
