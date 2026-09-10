package net.vulkanic.gui;

import java.util.ArrayList;
import java.util.List;
import net.blaze3d.vertex.PoseStack;
import net.minecraft.client.gui.render.state.GuiItemRenderState;
import net.minecraft.client.model.geom.ModelPart;
import net.minecraft.client.renderer.entity.ItemRenderer;
import net.minecraft.client.renderer.item.ItemStackRenderState;
import net.minecraft.client.renderer.special.ShieldSpecialRenderer;
import net.minecraft.client.resources.model.ModelBakery;
import net.minecraft.client.resources.model.Material;
import net.minecraft.client.renderer.Sheets;
import net.minecraft.client.model.ShieldModel;
import net.minecraft.core.component.DataComponentMap;
import net.minecraft.core.component.DataComponents;
import net.minecraft.world.item.ItemDisplayContext;
import net.minecraft.world.item.DyeColor;
import net.minecraft.world.level.block.entity.BannerPatternLayers;
import org.joml.Vector3f;
import static net.vulkanic.bridge.VulkanicGalBridge.*;

/** Copies model geometry, resource identity and material inputs; Rust owns the
 * item raster, projection, foil transform, depth domain and composition. */
final class GuiShieldItemSemanticCollector {
    private GuiShieldItemSemanticCollector() {}

    static GuiFlatItemMeshCollector.Snapshot collect(GuiItemRenderState item,
            ItemStackRenderState.SpecialRender selected, ShieldSpecialRenderer shield,
            int width, int height, int scale, int stratum, StandardItemFoilRecord foil) {
        if (item.itemStackRenderState().displayContext() != ItemDisplayContext.GUI
                || item.itemStackRenderState().usesBlockLight())
            throw new IllegalArgumentException("shield requires front-lit GUI model semantics");
        var components = selected.argument() instanceof DataComponentMap map ? map : DataComponentMap.EMPTY;
        // This is the copied item/model transform, not GUI raster space.
        var model = new org.joml.Matrix4f(selected.transform().pose()).scale(1, -1, -1);
        float[] transform = new float[16];
        model.get(transform);
        var batches = new ArrayList<GuiMeshBatchRecord>();
        var sources = new ArrayList<GuiItemTextureSource>();
        var cache = net.vulkanic.world.AtlasAnimationResource.privateShieldLifecycleEnabled()
            && Boolean.getBoolean("mattmc.dev.rustGalShieldAtlas")
            ? new GuiItemCacheRecord(GuiItemSemanticIdentities.identity(item.itemStackRenderState().getModelIdentity()),
                item.itemStackRenderState().isAnimated()) : null;
        for (var layer : layers(shield.model(), components, foil != null)) {
            var sprite = shield.sprite(layer.material());
            if (sprite != null && sprite.contents().isAnimated()
                    && !net.vulkanic.world.RustGalWorldPrimitiveRenderer.privateOwnedShieldSprite(sprite))
                throw new IllegalArgumentException("animated shield requires native atlas animation semantics");
            if (sprite == null)
                throw new IllegalArgumentException("shield semantic image is unavailable");
            GuiItemTextureSource source;
            if (!layer.foil() && Boolean.getBoolean("mattmc.dev.rustGalShieldAtlas")) {
                var region = net.vulkanic.world.RustGalWorldPrimitiveRenderer.requireShieldAtlasSpritePayload(sprite);
                long asset = RustGalGuiRawImageAssets.assetId("gui-atlas-region:" + sprite.atlasLocation() + ":" + sprite.contents().name());
                source = new GuiItemTextureSource.Atlas(new GuiAtlasRegion(asset, region.texture(),
                    region.atlasWidth(), region.atlasHeight(), region.x(), region.y(), region.width(), region.height()));
            } else {
                var image = RustGalGuiRawImageAssets.resolve(layer.foil() ? ItemRenderer.ENCHANTED_GLINT_ITEM : layer.material().texture());
                if (image == null) throw new IllegalArgumentException("shield semantic image is unavailable");
                source = new GuiItemTextureSource.Raw(image);
            }
            // Foil retains source atlas coordinates; owned base images use local UVs.
            var vertices = copyPart(layer.part(), layer.foil() ? sprite.getU0() : 0,
                layer.foil() ? sprite.getU1() : 1, layer.foil() ? sprite.getV0() : 0,
                layer.foil() ? sprite.getV1() : 1, layer.tint());
            batches.add(GuiFlatItemMeshCollector.batch(item, width, height, scale, stratum,
                batches.size(), layer.foil() ? 4 : layer.overlay() ? GUI_MESH_MATERIAL_MODEL_OVERLAY : 1,
                source.assetId(), transform, vertices, layer.foil() ? foil : null,
                layer.foil() ? 1 : GUI_MESH_LIGHTING_FRONT_MODEL).withItemCache(cache));
            sources.add(source);
        }
        return new GuiFlatItemMeshCollector.Snapshot(batches, sources);
    }

    record Layer(ModelPart part, Material material, int tint, boolean foil, boolean overlay) {}

    /** Vanilla-authored model parts, materials, tints and layer commands only. */
    static List<Layer> layers(ShieldModel model, DataComponentMap components, boolean foil) {
        var patterns = components.getOrDefault(DataComponents.BANNER_PATTERNS, BannerPatternLayers.EMPTY);
        var baseColor = components.get(DataComponents.BASE_COLOR);
        boolean patterned = baseColor != null || !patterns.layers().isEmpty();
        var material = patterned ? ModelBakery.SHIELD_BASE : ModelBakery.NO_PATTERN_SHIELD;
        var body = patterned ? model.root() : model.plate();
        var layers = new ArrayList<Layer>();
        layers.add(new Layer(model.handle(), material, -1, false, false));
        layers.add(new Layer(body, material, -1, false, false));
        if (foil) layers.add(new Layer(body, material, -1, true, false));
        if (patterned) {
            layers.add(new Layer(body, Sheets.SHIELD_BASE,
                (baseColor == null ? DyeColor.WHITE : baseColor).getTextureDiffuseColor(), false, true));
            for (var pattern : patterns.layers().subList(0, Math.min(16, patterns.layers().size())))
                layers.add(new Layer(body, Sheets.getShieldMaterial(pattern.pattern()),
                    pattern.color().getTextureDiffuseColor(), false, true));
        }
        return List.copyOf(layers);
    }

    static List<GuiMeshVertexRecord> copyPart(ModelPart part, float u0, float u1, float v0, float v1) {
        return copyPart(part, u0, u1, v0, v1, -1);
    }

    static List<GuiMeshVertexRecord> copyPart(ModelPart part, float u0, float u1, float v0, float v1, int tint) {
        if (part == null || !Float.isFinite(u0) || !Float.isFinite(u1)
                || !Float.isFinite(v0) || !Float.isFinite(v1) || u1 <= u0 || v1 <= v0)
            throw new IllegalArgumentException("invalid model part UV region");
        var vertices = new ArrayList<GuiMeshVertexRecord>();
        part.visit(new PoseStack(), (pose, path, cubeIndex, cube) -> {
            for (var polygon : cube.polygons) {
                if (polygon.vertices().length != 4 || vertices.size() > 65536 - 4)
                    throw new IllegalArgumentException("model part requires bounded complete quads");
                var normal = pose.transformNormal(polygon.normal(), new Vector3f());
                int packed = packNormal(normal.x) | (packNormal(normal.y) << 8) | (packNormal(normal.z) << 16);
                for (var vertex : polygon.vertices()) {
                    var position = pose.pose().transformPosition(vertex.worldX(), vertex.worldY(), vertex.worldZ(), new Vector3f());
                    vertices.add(new GuiMeshVertexRecord(new float[]{position.x, position.y, position.z},
                        new float[]{u0 + (u1 - u0) * vertex.u(), v0 + (v1 - v0) * vertex.v()},
                        new float[]{vertex.u(), vertex.v()}, tint, packed));
                }
            }
        });
        if (vertices.isEmpty()) throw new IllegalArgumentException("empty model part");
        return List.copyOf(vertices);
    }

    private static int packNormal(float value) {
        if (!Float.isFinite(value)) throw new IllegalArgumentException("nonfinite model normal");
        return (byte)(Math.clamp(value, -1.0F, 1.0F) * 127.0F) & 255;
    }
}
