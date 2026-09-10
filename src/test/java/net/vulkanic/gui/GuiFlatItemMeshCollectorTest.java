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
    @Test void nonFoilMeshPreservesAllFourAuthoredCornersAndRequestsNativeItemLighting() {
        var item=new GuiItemRenderState("non-affine",new Matrix3x2f(),new TrackingItemStackRenderState(),0,0,null);
        float[] transform={1,0,0,0,0,1,0,0,0,0,1,0,0,0,0,1};
        var vertices=new ArrayList<GuiMeshVertexRecord>();
        for (float[] point:List.of(new float[]{0,0,0.5F},new float[]{1,0,0.5F},
                new float[]{0.6F,1,0.5F},new float[]{0,1,0.5F})) {
            vertices.add(new GuiMeshVertexRecord(point,new float[]{0.4F,0.6F},
                new float[]{0.2F,0.8F},0x8040ff20,0x007f0000));
        }
        var batch=GuiFlatItemMeshCollector.batch(item,320,180,3,420,0,2,99,transform,vertices,null);
        assertEquals(vertices,batch.vertices(),"never reconstruct the fourth corner as an affine rectangle");
        assertEquals(List.of(0,1,2,2,3,0),batch.indices());
        assertNull(batch.itemFoil());
        assertEquals(1,batch.lightingMode());
        assertEquals(3,batch.itemRasterScale(),"native item raster requires the explicit frame lightmap");
        assertEquals(0,batch.renderWidth(),"Rust chooses the raster extent");
        assertEquals(0x8040ff20,batch.vertices().getFirst().colorArgb(),"Java must not compensate RGB");
    }

    @Test void projectedFoilStillRejectsMissingGeometryBeforeResourceAccess() throws Exception {
        var state=new TrackingItemStackRenderState();
        var context=net.minecraft.client.renderer.item.ItemStackRenderState.class.getDeclaredField("displayContext");
        context.setAccessible(true);
        context.set(state,net.minecraft.world.item.ItemDisplayContext.GUI);
        state.newLayer().setFoilType(net.minecraft.client.renderer.item.ItemStackRenderState.FoilType.SPECIAL);
        var item=new GuiItemRenderState("projected",new Matrix3x2f(),state,0,0,null);
        var error=assertThrows(IllegalArgumentException.class,()->GuiFlatItemMeshCollector.collect(
            item,320,180,2,420,new StandardItemFoilRecord(0,0,0.5F)));
        assertEquals("unsupported flat mesh layer",error.getMessage());
    }

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

    @Test void manyFacesUseOneIndexedMeshWithoutDroppingOrReorderingQuads() {
        var item=new GuiItemRenderState("many-faces",new Matrix3x2f(),new TrackingItemStackRenderState(),0,0,null);
        float[] transform={1,0,0,0,0,1,0,0,0,0,1,0,0,0,0,1};
        var vertices=new ArrayList<GuiMeshVertexRecord>();
        for(int i=0;i<320;i++) vertices.add(new GuiMeshVertexRecord(new float[]{i,0,0},
            new float[]{0,0},new float[]{0,0},-1,0));
        var batch=GuiFlatItemMeshCollector.batch(item,320,180,2,420,0,2,99,transform,vertices,null);
        assertEquals(320,batch.vertices().size());
        assertEquals(480,batch.indices().size());
        for(int face=0;face<80;face++) {
            int i=face*4;
            assertEquals(List.of(i,i+1,i+2,i+2,i+3,i),batch.indices().subList(face*6,face*6+6));
            assertEquals(i,batch.vertices().get(i).position()[0]);
        }
        assertThrows(IllegalArgumentException.class,()->GuiFlatItemMeshCollector.batch(item,320,180,2,420,0,2,99,
            transform,vertices.subList(0,319),null));
        assertThrows(IllegalArgumentException.class,()->GuiFlatItemMeshCollector.batch(item,320,180,2,420,0,2,99,
            transform,java.util.Collections.nCopies(65_540,vertices.getFirst()),null));
    }
}
