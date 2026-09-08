package net.vulkanic.bridge;

import java.util.ArrayList;
import java.util.List;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;
import net.vulkanic.bridge.VulkanicGalBridge.*;

class GuiItemLayerRecordTest {
    @Test void authoredGuiDisplayTransformPreservesPlanarSemantics() {
        var pose=new net.blaze3d.vertex.PoseStack.Pose();
        new net.minecraft.client.renderer.block.model.ItemTransform(
            new org.joml.Vector3f(0,0,90),new org.joml.Vector3f(0.125F,0,0),
            new org.joml.Vector3f(0.5F,0.5F,1)).apply(false,pose);
        float[] matrix=new float[16]; pose.pose().get(matrix);
        // These are the exact semantic bytes whose native regression guards
        // quaternion roundoff without admitting tilted or 3D transforms.
        assertArrayEquals(new float[]{0,0.49999997F,0,0,-0.49999997F,0,0,0,
            0,0,0.99999994F,0,0.375F,-0.24999999F,-0.49999997F,1},matrix);
    }
    @Test void modelTransformIsAnImmutableSemanticValue() {
        float[] matrix=layer(17).modelTransform();
        matrix[0]=0.5F; matrix[12]=-0.25F;
        var transformed=new GuiItemRasterLayerRecord(17,-1,1,GuiItemRasterGeometryRecord.FULL,0,0,1,1,matrix);
        matrix[0]=9;
        float[] returned=transformed.modelTransform(); returned[0]=7;
        assertEquals(0.5F,transformed.modelTransform()[0]);
        var equal=new GuiItemRasterLayerRecord(17,-1,1,GuiItemRasterGeometryRecord.FULL,0,0,1,1,transformed.modelTransform());
        assertEquals(transformed,equal);
        assertEquals(transformed.hashCode(),equal.hashCode());
        assertThrows(IllegalArgumentException.class,()->new GuiItemRasterLayerRecord(17,-1,1,GuiItemRasterGeometryRecord.FULL,0,0,1,1,new float[15]));
        matrix[0]=Float.NaN;
        assertThrows(IllegalArgumentException.class,()->new GuiItemRasterLayerRecord(17,-1,1,GuiItemRasterGeometryRecord.FULL,0,0,1,1,matrix));
    }
    private static GuiItemRasterLayerRecord layer(long id) {
        return new GuiItemRasterLayerRecord(id,0x80ff0000,1,GuiItemRasterGeometryRecord.FULL,0,0,1,1);
    }
    private static GuiAffineQuadRecord item() {
        return new GuiAffineQuadRecord(1,17,0,0,16,0,0,16,0,0,0,1,1,-1,320,180)
            .withMaterialMode(1).withItemRasterScale(2);
    }
    @Test void immutableOrderedLayersSurviveEverySchedulerWither() {
        var supplied=new ArrayList<>(List.of(layer(17),layer(18)));
        var request=item().withItemRasterLayers(supplied);
        supplied.clear();
        var changed=request.withSequence(5).withStratum(2).withClip(0,0,20,20)
            .withMaterialMode(2).withItemRasterScale(3).withItemRasterGeometry(GuiItemRasterGeometryRecord.FULL);
        assertEquals(List.of(layer(17),layer(18)),changed.itemRasterLayers());
        assertEquals(5,changed.sequence());
        assertThrows(UnsupportedOperationException.class,()->changed.itemRasterLayers().clear());
    }
    @Test void malformedAndUnboundedLayersCannotEnterTheTransport() {
        assertThrows(IllegalArgumentException.class,()->item().withItemRasterLayers(java.util.Collections.nCopies(65,layer(17))));
        assertThrows(IllegalArgumentException.class,()->item().withItemRasterLayers(List.of(layer(17))).withItemRasterScale(0));
        assertThrows(IllegalArgumentException.class,()->new GuiItemRasterLayerRecord(0,-1,1,GuiItemRasterGeometryRecord.FULL,0,0,1,1));
        assertThrows(IllegalArgumentException.class,()->new GuiItemRasterLayerRecord(17,-1,0,GuiItemRasterGeometryRecord.FULL,0,0,1,1));
        assertThrows(IllegalArgumentException.class,()->new GuiItemRasterLayerRecord(17,-1,1,GuiItemRasterGeometryRecord.FULL,Float.NaN,0,1,1));
    }
    @Test void encoderUsesNativeNestedLayoutAndPreservesOrderedPayloads() throws Exception {
        try (var bridge=VulkanicGalBridge.create("rust-vulkan")) {
            var encode=VulkanicGalBridge.class.getDeclaredMethod("encodeGuiAffineQuads",List.class);
            encode.setAccessible(true);
            float[] matrix=layer(18).modelTransform();
            matrix[0]=0; matrix[1]=0.5F; matrix[4]=-0.5F; matrix[5]=0;
            matrix[12]=0.375F; matrix[13]=-0.25F;
            var transformed=new GuiItemRasterLayerRecord(18,0x80ff0000,1,GuiItemRasterGeometryRecord.FULL,0,0,1,1,matrix);
            var encoded=(java.lang.foreign.MemorySegment)encode.invoke(bridge,List.of(item().withItemRasterLayers(List.of(layer(17),transformed))));
            assertEquals(152,encoded.get(java.lang.foreign.ValueLayout.JAVA_INT,0));
            assertEquals(2,encoded.get(java.lang.foreign.ValueLayout.JAVA_LONG,144));
            var layers=encoded.get(java.lang.foreign.ValueLayout.ADDRESS,136).reinterpret(256);
            for(int i=0;i<2;i++) {
                var child=layers.asSlice(i*128,128);
                assertEquals(128,child.get(java.lang.foreign.ValueLayout.JAVA_INT,0));
                assertEquals(1,child.get(java.lang.foreign.ValueLayout.JAVA_INT,4));
                assertEquals(17+i,child.get(java.lang.foreign.ValueLayout.JAVA_LONG,8));
                assertEquals(0x80ff0000,child.get(java.lang.foreign.ValueLayout.JAVA_INT,16));
                assertArrayEquals(new float[]{0,0,16,0,0,16},child.asSlice(20,24).toArray(java.lang.foreign.ValueLayout.JAVA_FLOAT));
                assertArrayEquals(new float[]{0,0,1,1},child.asSlice(44,16).toArray(java.lang.foreign.ValueLayout.JAVA_FLOAT));
                assertArrayEquals(i==0 ? layer(17).modelTransform() : matrix,child.asSlice(60,64).toArray(java.lang.foreign.ValueLayout.JAVA_FLOAT));
            }
        }
    }
}
