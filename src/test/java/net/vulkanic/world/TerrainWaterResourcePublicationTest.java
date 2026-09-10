package net.vulkanic.world;

import java.lang.reflect.Field;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import net.vulkanic.bridge.VulkanicGalBridge;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

class TerrainWaterResourcePublicationTest {
    @Test
    void atlasBindingPublishesNoUnusedWaterSheetsAndKeepsResidencyRetryable() throws Exception {
        Map<Field, Object> saved = new LinkedHashMap<>();
        for (String name : List.of("atlasPayload", "atlasMipPayloads", "atlasGeneration",
                "registeredAtlasGeneration", "normalAtlasPayload", "specularAtlasPayload",
                "waterStillAsset", "waterFlowAsset", "waterOverlayAsset")) {
            Field field = RustGalTerrainRenderer.class.getDeclaredField(name);
            field.setAccessible(true);
            saved.put(field, field.get(null));
        }
        try {
            set(saved, "atlasPayload", new byte[]{1});
            set(saved, "atlasMipPayloads", List.of());
            set(saved, "atlasGeneration", 19L);
            set(saved, "registeredAtlasGeneration", 18L);
            set(saved, "normalAtlasPayload", null);
            set(saved, "specularAtlasPayload", null);
            RustGalTerrainRenderer.installTestingFluidSpriteAssetsForUnitTests();
            var atlasBinding = RustGalTerrainRenderer.waterTextureBinding(
                WorldRenderRoutePolicy.Route.RUST_VULKAN_WHOLE_FRAME);
            for (int retry = 0; retry < 2; retry++) {
                var atlas = RustGalTerrainRenderer.atlasTextureUpdatePayload(atlasBinding);
                assertEquals(List.of(RustGalWorldPrimitiveRenderer.MATERIAL_TEXTURE_TERRAIN_BLOCK_ATLAS),
                    atlas.stream().map(VulkanicGalBridge.WorldMeshTextureAssetRecord::textureId).toList());
            }
            var sheets = RustGalTerrainRenderer.atlasTextureUpdatePayload(
                RustGalTerrainRenderer.WaterTextureBinding.SEPARATE_SHEETS);
            assertEquals(List.of(RustGalWorldPrimitiveRenderer.MATERIAL_TEXTURE_TERRAIN_BLOCK_ATLAS,
                RustGalWorldPrimitiveRenderer.MATERIAL_TEXTURE_WATER_STILL,
                RustGalWorldPrimitiveRenderer.MATERIAL_TEXTURE_WATER_FLOW,
                RustGalWorldPrimitiveRenderer.MATERIAL_TEXTURE_WATER_OVERLAY),
                sheets.stream().map(VulkanicGalBridge.WorldMeshTextureAssetRecord::textureId).toList());
            set(saved, "registeredAtlasGeneration", 19L);
            assertTrue(RustGalTerrainRenderer.atlasTextureUpdatePayload(atlasBinding).isEmpty());
        } finally {
            for (var entry : saved.entrySet()) entry.getKey().set(null, entry.getValue());
        }
    }

    private static void set(Map<Field, Object> saved, String name, Object value) throws Exception {
        for (Field field : saved.keySet()) {
            if (field.getName().equals(name)) { field.set(null, value); return; }
        }
        throw new AssertionError("Unknown test field " + name);
    }
}
