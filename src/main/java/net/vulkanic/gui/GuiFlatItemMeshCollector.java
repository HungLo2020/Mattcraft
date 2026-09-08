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

/** Flat item model snapshots; no Java raster target, projection or foil matrix. */
final class GuiFlatItemMeshCollector {
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
        item.itemStackRenderState().forEachSemanticLayer(layer -> {
            if (layer.hasSpecialRenderer() || layer.usesBlockLight() || layer.quads().isEmpty()
                || layer.foilType()==ItemStackRenderState.FoilType.SPECIAL
                || !RustGalGuiItemRenderer.supportedGuiRenderType(layer.renderType())) {
                throw new IllegalArgumentException("unsupported flat mesh layer");
            }
            int material=RustGalGuiItemRenderer.flatItemMaterial(layer.renderType().pipeline())==2 ? 2 : 3;
            List<List<GuiMeshVertexRecord>> copied=new ArrayList<>();
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
                sources.add(source);
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
                        new float[]{u,v},new float[]{(u-sprite.getU0())/du,(v-sprite.getV0())/dv},
                        GuiItemMeshSemanticCollector.standard3dVertexColor(quad.getColor(vertex),tint,
                            net.sodium.client.render.immediate.model.BakedModelEncoder.shouldMultiplyAlpha()),
                        quad.getAccurateNormal(vertex)));
                }
                copied.add(List.copyOf(vertices));
                batches.add(batch(item,guiWidth,guiHeight,guiScale,stratum,batches.size(),material,
                    asset,layer.modelTransform(),vertices,null));
            }
            if(layer.foilType()==ItemStackRenderState.FoilType.STANDARD) {
                var glint=RustGalGuiRawImageAssets.resolve(ItemRenderer.ENCHANTED_GLINT_ITEM);
                if(glint==null) throw new IllegalArgumentException("flat mesh foil image missing");
                sources.add(new GuiItemTextureSource.Raw(glint));
                for(var vertices:copied) batches.add(batch(item,guiWidth,guiHeight,guiScale,stratum,batches.size(),4,
                    glint.assetId(),layer.modelTransform(),vertices,foil));
            }
            if(batches.size()>64) throw new IllegalArgumentException("flat mesh layer bound exceeded");
        });
        if(batches.isEmpty()) throw new IllegalArgumentException("flat mesh empty");
        return new Snapshot(batches,sources);
    }

    static GuiMeshBatchRecord batch(GuiItemRenderState item,int width,int height,int scale,int stratum,int layer,
                                           int material,long asset,float[] transform,List<GuiMeshVertexRecord> vertices,
                                           StandardItemFoilRecord foil) {
        var clip=item.scissorArea();
        var pose=item.pose();
        return new GuiMeshBatchRecord(stratum,layer,material,1,asset,0L,0.1F,transform,
            new float[]{pose.m00(),pose.m01(),pose.m10(),pose.m11(),pose.m20(),pose.m21()},
            item.x(),item.y(),item.x()+16,item.y()+16,width,height,0,0,0,
            clip==null?0:1,clip==null?0:clip.left(),clip==null?0:clip.top(),
            clip==null?0:clip.width(),clip==null?0:clip.height(),vertices,List.of(0,1,2,2,3,0),foil,scale);
    }
}
