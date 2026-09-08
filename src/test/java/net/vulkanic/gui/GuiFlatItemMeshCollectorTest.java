package net.vulkanic.gui;

import java.util.ArrayList;
import java.util.List;
import net.minecraft.client.gui.render.state.GuiItemRenderState;
import net.minecraft.client.renderer.item.TrackingItemStackRenderState;
import net.vulkanic.bridge.VulkanicGalBridge.*;
import org.joml.Matrix3x2f;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

class GuiFlatItemMeshCollectorTest {
    @Test void batchCopiesRawGeometryScalePoseAndFoilWithoutJavaRasterSetup() {
        var item=new GuiItemRenderState("test",new Matrix3x2f().translation(3,5),new TrackingItemStackRenderState(),12,34,null);
        float[] transform={1,0,0,0, 0,1,0,0, 0,0,1,0, -0.5F,-0.5F,-0.5F,1};
        var vertex=new GuiMeshVertexRecord(new float[]{0.25F,0.75F,0.5F},new float[]{0.4F,0.6F},
            new float[]{0.1F,0.9F},0x80123456,0x007f0000);
        var vertices=new ArrayList<>(List.of(vertex,vertex,vertex,vertex));
        var foil=new StandardItemFoilRecord(12345,0.5,0.5F);
        var batch=GuiFlatItemMeshCollector.batch(item,320,180,2,420,1,4,99,transform,vertices,foil);
        transform[12]=99; vertices.clear();
        assertEquals(2,batch.itemRasterScale());
        assertEquals(0,batch.renderWidth()); assertEquals(0,batch.renderHeight()); assertEquals(0,batch.guardPixels());
        assertArrayEquals(new float[]{1,0,0,1,3,5},batch.guiPose());
        assertEquals(12,batch.left()); assertEquals(34,batch.top());
        assertEquals(28,batch.right()); assertEquals(50,batch.bottom());
        assertEquals(-0.5F,batch.modelTransform()[12]);
        assertEquals(vertex,batch.vertices().getFirst()); assertEquals(foil,batch.itemFoil());
        assertEquals(0x80123456,batch.vertices().getFirst().colorArgb());
        assertEquals(0x007f0000,batch.vertices().getFirst().normalPacked());
    }

    @Test void incompleteOrNonGuiItemsRejectWithoutAccessingRendererState() {
        var item=new GuiItemRenderState("empty",new Matrix3x2f(),new TrackingItemStackRenderState(),0,0,null);
        assertThrows(IllegalArgumentException.class,()->GuiFlatItemMeshCollector.collect(item,320,180,2,420,new StandardItemFoilRecord(0,0,0.5F)));
    }
}
