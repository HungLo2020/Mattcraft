package net.vulkanic.bridge;

import java.lang.foreign.MemorySegment;
import java.lang.foreign.ValueLayout;
import java.util.List;
import org.junit.jupiter.api.Test;
import static net.vulkanic.bridge.VulkanicGalBridge.*;
import static org.junit.jupiter.api.Assertions.*;

class GuiItemFoilEncodingTest {
    @Test void frontModelLightingIsExplicitAndSurvivesNativeEncoding() throws Exception {
        var source=batch(1);
        var front=new GuiMeshBatchRecord(source.stratum(),source.layerIndex(),1,GUI_MESH_LIGHTING_FRONT_MODEL,
            source.assetId(),source.sequence(),0,source.modelTransform(),source.guiPose(),
            source.left(),source.top(),source.right(),source.bottom(),source.guiWidth(),source.guiHeight(),
            0,0,0,0,0,0,0,0,source.vertices(),source.indices(),null,3);
        assertEquals(GUI_MESH_LIGHTING_FRONT_MODEL,front.withSequence(44).lightingMode());
        assertThrows(IllegalArgumentException.class,()->front.withItemFoil(new StandardItemFoilRecord(0,0,0.5F)));
        try(var bridge=VulkanicGalBridge.create("rust-vulkan")) {
            var encode=VulkanicGalBridge.class.getDeclaredMethod("encodeGuiMeshBatches",List.class);
            encode.setAccessible(true);
            var encoded=(MemorySegment)encode.invoke(bridge,List.of(front));
            assertEquals(4,encoded.get(ValueLayout.JAVA_INT,Struct.GUI_MESH_BATCH_REQUEST.offset(4)));
            assertEquals(3,encoded.get(ValueLayout.JAVA_INT,Struct.GUI_MESH_BATCH_REQUEST.offset(31)));
        }
    }
    @Test void entityFoilTransportRequiresNativeModelRasterAndPreservesKind() throws Exception {
        var foil = new StandardItemFoilRecord(12345, 0.125, 0.375F, StandardFoilKind.ENTITY);
        var request = flatBatch(3,0,0).withItemFoil(foil).withSequence(44);
        assertEquals(foil, request.itemFoil());
        assertThrows(IllegalArgumentException.class, () -> batch(4).withItemFoil(foil));
        assertThrows(IllegalArgumentException.class, () -> request.withDecalFoil(GuiDecalFoilRecord.forNativeItemLayout()));
        try(var bridge=VulkanicGalBridge.create("rust-vulkan")) {
            var encode=VulkanicGalBridge.class.getDeclaredMethod("encodeGuiMeshBatches",List.class);
            encode.setAccessible(true);
            var encoded=(MemorySegment)encode.invoke(bridge,List.of(request));
            var layout=Struct.GUI_MESH_BATCH_REQUEST;
            assertEquals(2,encoded.get(ValueLayout.JAVA_INT,layout.offset(27)));
            assertEquals(12345,encoded.get(ValueLayout.JAVA_LONG,layout.offset(28)));
            assertEquals(0.125,encoded.get(ValueLayout.JAVA_DOUBLE,layout.offset(29)));
            assertEquals(0.375F,encoded.get(ValueLayout.JAVA_FLOAT,layout.offset(30)));
            assertEquals(3,encoded.get(ValueLayout.JAVA_INT,layout.offset(31)));
            assertEquals(0,encoded.get(ValueLayout.JAVA_INT,layout.offset(17)));
        }
    }

    @Test void blockLayoutCopiesDoubleBoundsAndLeavesRasterSetupToRust() throws Exception {
        var source=batch(1);
        var inventory=new GuiMeshBatchRecord(source.stratum(),source.layerIndex(),source.materialMode(),3,
            source.assetId(),source.sequence(),source.alphaCutoff(),source.modelTransform(),source.guiPose(),
            source.left(),source.top(),source.right(),source.bottom(),source.guiWidth(),source.guiHeight(),
            source.renderWidth(),source.renderHeight(),source.guardPixels(),source.vertices(),source.indices());
        double[] bounds={-0.5,-0.5,-0.5,0.5+1e-9,0.5,0.5};
        var block=new GuiBlockItemRasterRecord(2,bounds);
        var request=inventory.withBlockItemRaster(block).withSequence(73).withItemFoil(null);
        bounds[3]=99;block.modelBounds()[3]=88;
        assertEquals(0.5+1e-9,request.blockItemRaster().modelBounds()[3]);
        assertEquals(0,request.renderWidth());assertEquals(0,request.renderHeight());assertEquals(0,request.guardPixels());
        assertEquals(source.vertices(),request.vertices(),"preserve original normals and geometry");
        assertArrayEquals(source.modelTransform(),request.modelTransform());
        assertThrows(IllegalArgumentException.class,()->flatBatch(2,0,0).withBlockItemRaster(block));
        assertThrows(IllegalArgumentException.class,()->new GuiBlockItemRasterRecord(0,new double[6]));
        assertThrows(IllegalArgumentException.class,()->new GuiBlockItemRasterRecord(2,new double[]{1,0,0,0,0,0}));
        assertThrows(IllegalArgumentException.class,()->new GuiBlockItemRasterRecord(2,new double[]{0,0,0,Double.NaN,1,1}));
        try(var bridge=VulkanicGalBridge.create("rust-vulkan")) {
            var method=VulkanicGalBridge.class.getDeclaredMethod("encodeGuiMeshBatches",List.class);
            method.setAccessible(true);
            var data=(MemorySegment)method.invoke(bridge,List.of(request,inventory));
            var layout=Struct.GUI_MESH_BATCH_REQUEST;
            assertEquals(2,data.get(ValueLayout.JAVA_INT,layout.offset(35)));
            assertEquals(1,data.get(ValueLayout.JAVA_INT,layout.offset(37)));
            var oversized=request.withBlockItemRaster(new GuiBlockItemRasterRecord(2,block.modelBounds(),true)).withSequence(74);
            assertTrue(oversized.blockItemRaster().oversizedGui());
            var large=(MemorySegment)method.invoke(bridge,List.of(oversized));
            assertEquals(2,large.get(ValueLayout.JAVA_INT,layout.offset(37)));
            assertArrayEquals(block.modelBounds(),data.asSlice(layout.offset(36),48).toArray(ValueLayout.JAVA_DOUBLE));
            var absent=data.asSlice(layout.byteSize(),layout.byteSize());
            assertEquals(0,absent.get(ValueLayout.JAVA_INT,layout.offset(35)));
            assertEquals(0,absent.get(ValueLayout.JAVA_INT,layout.offset(37)));
            for(double value:absent.asSlice(layout.offset(36),48).toArray(ValueLayout.JAVA_DOUBLE))
                assertEquals(0L,Double.doubleToRawLongBits(value));
        }
    }

    @Test void nativeDecalLayoutEncodesOnlySemanticModeAndRejectsCallerRasterMatrices() throws Exception {
        var decal=GuiDecalFoilRecord.forNativeItemLayout();
        var request=flatBatch(3,0,0).withDecalFoil(decal).withSequence(23);
        assertTrue(request.decalFoil().nativeItemLayout());
        assertThrows(IllegalArgumentException.class,()->batch(4).withItemFoil(new StandardItemFoilRecord(0,0,0.5F)).withDecalFoil(decal));
        assertThrows(IllegalArgumentException.class,()->new GuiDecalFoilRecord(batch(4).modelTransform(),new float[9],true));
        try(var bridge=VulkanicGalBridge.create("rust-vulkan")) {
            var encode=VulkanicGalBridge.class.getDeclaredMethod("encodeGuiMeshBatches",List.class);
            encode.setAccessible(true);
            var layout=Struct.GUI_MESH_BATCH_REQUEST;
            var encoded=(MemorySegment)encode.invoke(bridge,List.of(request));
            assertEquals(2,encoded.get(ValueLayout.JAVA_INT,layout.offset(32)));
            for(float value:encoded.asSlice(layout.offset(33),64).toArray(ValueLayout.JAVA_FLOAT)) assertEquals(0,Float.floatToRawIntBits(value));
            for(float value:encoded.asSlice(layout.offset(34),36).toArray(ValueLayout.JAVA_FLOAT)) assertEquals(0,Float.floatToRawIntBits(value));
        }
    }

    @Test void decalPosesAreCopiedAndEncodedWithoutJavaProjection() throws Exception {
        var source=batch(4).withItemFoil(new StandardItemFoilRecord(12345,0.5,0.375F));
        float[] model=source.modelTransform();
        float[] normal={1,0,0,0,1,0,0,0,1};
        model[12]=17;
        var decal=new GuiDecalFoilRecord(model,normal);
        var request=source.withDecalFoil(decal).withSequence(22)
            .withItemFoil(new StandardItemFoilRecord(34567,0.25,0.625F));
        model[12]=99;normal[0]=99;
        decal.modelPose()[12]=77;decal.normalPose()[0]=77;
        assertEquals(17,request.decalFoil().modelPose()[12]);
        assertEquals(1,request.decalFoil().normalPose()[0]);
        assertThrows(IllegalArgumentException.class,()->batch(4).withDecalFoil(decal));
        assertThrows(IllegalArgumentException.class,()->request.withItemFoil(null));
        assertNull(request.withDecalFoil(null).decalFoil());
        assertThrows(IllegalArgumentException.class,()->new GuiDecalFoilRecord(new float[16],normal));
        normal[0]=Float.NaN;
        assertThrows(IllegalArgumentException.class,()->new GuiDecalFoilRecord(source.modelTransform(),normal));
        try(var bridge=VulkanicGalBridge.create("rust-vulkan")) {
            var encode=VulkanicGalBridge.class.getDeclaredMethod("encodeGuiMeshBatches",List.class);
            encode.setAccessible(true);
            var layout=Struct.GUI_MESH_BATCH_REQUEST;
            var encoded=(MemorySegment)encode.invoke(bridge,List.of(request,source));
            assertEquals(1,encoded.get(ValueLayout.JAVA_INT,layout.offset(32)));
            assertArrayEquals(decal.modelPose(),encoded.asSlice(layout.offset(33),64).toArray(ValueLayout.JAVA_FLOAT));
            assertArrayEquals(decal.normalPose(),encoded.asSlice(layout.offset(34),36).toArray(ValueLayout.JAVA_FLOAT));
            assertEquals(34567,encoded.get(ValueLayout.JAVA_LONG,layout.offset(28)));
            assertEquals(0.625F,encoded.get(ValueLayout.JAVA_FLOAT,layout.offset(30)));
            var absent=encoded.asSlice(layout.byteSize(),layout.byteSize());
            assertEquals(0,absent.get(ValueLayout.JAVA_INT,layout.offset(32)));
            for(float value:absent.asSlice(layout.offset(33),64).toArray(ValueLayout.JAVA_FLOAT)) assertEquals(0,Float.floatToRawIntBits(value));
            for(float value:absent.asSlice(layout.offset(34),36).toArray(ValueLayout.JAVA_FLOAT)) assertEquals(0,Float.floatToRawIntBits(value));
        }
    }

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
