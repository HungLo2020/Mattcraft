package net.vulkanic.gui;

import java.util.ArrayList;
import java.util.List;
import net.minecraft.client.gui.render.state.GuiItemRenderState;
import net.minecraft.client.renderer.block.model.BakedQuad;
import net.minecraft.client.renderer.entity.ItemRenderer;
import net.minecraft.client.renderer.item.ItemStackRenderState;
import net.minecraft.world.item.ItemDisplayContext;
import net.sodium.client.model.quad.BakedQuadView;
import net.vulkanic.bridge.VulkanicGalBridge.*;
import static net.vulkanic.bridge.VulkanicGalBridge.GUI_MESH_MATERIAL_MODEL_OVERLAY;

/** Flat item model snapshots; no Java raster target, projection or foil matrix. */
final class GuiFlatItemMeshCollector {
    private static final int MAX_COPIED_VERTICES = 65_536;
    private record Faces(long asset, List<GuiMeshVertexRecord> vertices) {}
    record Snapshot(List<GuiMeshBatchRecord> batches, List<GuiItemTextureSource> sources) {
        Snapshot { batches=List.copyOf(batches); sources=List.copyOf(sources); }
    }

    static Snapshot collect(GuiItemRenderState item, int guiWidth, int guiHeight,
                            int guiScale, int stratum, StandardItemFoilRecord foil) {
        if (item.itemStackRenderState().displayContext()!=ItemDisplayContext.GUI
            || item.itemStackRenderState().usesBlockLight() || guiScale<=0) {
            throw new IllegalArgumentException("flat mesh requires GUI flat-lighting semantics");
        }
        List<GuiMeshBatchRecord> batches=new ArrayList<>();
        List<GuiItemTextureSource> sources=new ArrayList<>();
        int[] copiedVertexCount = {0};
        item.itemStackRenderState().forEachSemanticLayer(layer -> {
            boolean specialFoil=layer.foilType()==ItemStackRenderState.FoilType.SPECIAL;
            if (layer.hasSpecialRenderer() || layer.usesBlockLight() || layer.quads().isEmpty()
                || !RustGalGuiItemRenderer.supportedGuiRenderType(layer.renderType())) {
                throw new IllegalArgumentException("unsupported flat mesh layer");
            }
            int material=RustGalGuiItemRenderer.flatItemMaterial(layer.renderType().pipeline())==2 ? 2 : 3;
            int copies = layer.foilType()!=ItemStackRenderState.FoilType.NONE ? 2 : 1;
            if (layer.quads().size() > (MAX_COPIED_VERTICES-copiedVertexCount[0])/(4*copies))
                throw new IllegalArgumentException("flat mesh vertex bound exceeded");
            copiedVertexCount[0] += layer.quads().size()*4*copies;
            List<Faces> copied=new ArrayList<>();
            for (BakedQuad baked:layer.quads()) {
                // Preserve back and edge faces too. Rust owns normal
                // transformation, winding, depth and raster-cell validation.
                if (!(baked instanceof BakedQuadView quad))
                    throw new IllegalArgumentException("flat mesh face has no semantic quad view");
                var sprite=quad.getSprite();
                if (sprite==null) throw new IllegalArgumentException("flat mesh sprite missing");
                var region=net.vulkanic.world.RustGalTerrainRenderer.requireGuiAtlasSpritePayload(sprite);
                long asset=RustGalGuiRawImageAssets.assetId("gui-atlas-region:"+sprite.atlasLocation()+":"+sprite.contents().name());
                var source=new GuiItemTextureSource.Atlas(new GuiAtlasRegion(asset,region.texture(),
                    region.atlasWidth(),region.atlasHeight(),region.x(),region.y(),region.width(),region.height()));
                boolean newResource = copied.isEmpty() || copied.getLast().asset()!=asset;
                if (newResource) {
                    sources.add(source);
                    copied.add(new Faces(asset,new ArrayList<>()));
                }
                int[] tints=layer.tintLayers();
                int tint=baked.isTinted() && baked.tintIndex()>=0 && baked.tintIndex()<tints.length ? tints[baked.tintIndex()] : -1;
                List<GuiMeshVertexRecord> vertices=new ArrayList<>(4);
                for(int vertex=0;vertex<4;vertex++) {
                    float u=quad.getTexU(vertex), v=quad.getTexV(vertex);
                    float du=sprite.getU1()-sprite.getU0(), dv=sprite.getV1()-sprite.getV0();
                    if (!(du>0) || !(dv>0)) throw new IllegalArgumentException("flat mesh sprite UV extent missing");
                    // Original model position, normal, tint and texture coordinates.
                    // Do not apply the GUI/model matrix or animation here.
                    vertices.add(new GuiMeshVertexRecord(new float[]{quad.getX(vertex),quad.getY(vertex),quad.getZ(vertex)},
                        new float[]{u,v},new float[]{RustGalGuiItemRenderer.itemLocalUv(u,sprite.getU0(),sprite.getU1()),
                            RustGalGuiItemRenderer.itemLocalUv(v,sprite.getV0(),sprite.getV1())},
                        GuiItemMeshSemanticCollector.standard3dVertexColor(quad.getColor(vertex),tint,
                            net.sodium.client.render.immediate.model.BakedModelEncoder.shouldMultiplyAlpha()),
                        quad.getAccurateNormal(vertex)));
                }
                copied.getLast().vertices().addAll(vertices);
            }
            for (var faces : copied) {
                batches.add(batch(item,guiWidth,guiHeight,guiScale,stratum,batches.size(),material,
                    faces.asset(),layer.modelTransform(),faces.vertices(),null));
            }
            if(layer.foilType()!=ItemStackRenderState.FoilType.NONE) {
                var glint=RustGalGuiRawImageAssets.resolve(ItemRenderer.ENCHANTED_GLINT_ITEM);
                if(glint==null) throw new IllegalArgumentException("flat mesh foil image missing");
                sources.add(new GuiItemTextureSource.Raw(glint));
                List<GuiMeshVertexRecord> foilVertices = new ArrayList<>();
                for (var faces : copied) foilVertices.addAll(faces.vertices());
                var foilBatch=batch(item,guiWidth,guiHeight,guiScale,stratum,batches.size(),4,
                    glint.assetId(),layer.modelTransform(),foilVertices,foil);
                batches.add(specialFoil ? foilBatch.withDecalFoil(GuiDecalFoilRecord.forNativeItemLayout()) : foilBatch);
            }
            if(batches.size()>64) throw new IllegalArgumentException("flat mesh layer bound exceeded");
        });
        if(batches.isEmpty()) throw new IllegalArgumentException("flat mesh empty");
        return new Snapshot(batches,sources);
    }

    static GuiMeshBatchRecord batch(GuiItemRenderState item,int width,int height,int scale,int stratum,int layer,
                                           int material,long asset,float[] transform,List<GuiMeshVertexRecord> vertices,
                                           StandardItemFoilRecord foil) {
        return batch(item,width,height,scale,stratum,layer,material,asset,transform,vertices,foil,1);
    }

    static GuiMeshBatchRecord batch(GuiItemRenderState item,int width,int height,int scale,int stratum,int layer,
                                           int material,long asset,float[] transform,List<GuiMeshVertexRecord> vertices,
                                           StandardItemFoilRecord foil,int lighting) {
        var clip=item.scissorArea();
        var pose=item.pose();
        if (vertices.isEmpty() || vertices.size()%4!=0 || vertices.size()>MAX_COPIED_VERTICES)
            throw new IllegalArgumentException("flat mesh requires bounded complete quads");
        List<Integer> indices = new ArrayList<>(vertices.size()/4*6);
        for (int first=0;first<vertices.size();first+=4) {
            indices.add(first); indices.add(first+1); indices.add(first+2);
            indices.add(first+2); indices.add(first+3); indices.add(first);
        }
        return new GuiMeshBatchRecord(stratum,layer,material,lighting,asset,0L,material == 1 || material == GUI_MESH_MATERIAL_MODEL_OVERLAY ? 0.0F : 0.1F,transform,
            new float[]{pose.m00(),pose.m01(),pose.m10(),pose.m11(),pose.m20(),pose.m21()},
            item.x(),item.y(),item.x()+16,item.y()+16,width,height,0,0,0,
            clip==null?0:1,clip==null?0:clip.left(),clip==null?0:clip.top(),
            clip==null?0:clip.width(),clip==null?0:clip.height(),vertices,indices,foil,scale);
    }
}
