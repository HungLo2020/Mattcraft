package net.vulkanic.world;

import java.util.LinkedHashMap;
import java.util.Map;
import net.minecraft.client.Minecraft;
import net.minecraft.client.renderer.texture.TextureAtlas;
import net.minecraft.client.renderer.texture.TextureAtlasSprite;
import net.minecraft.client.renderer.texture.SpriteContents;
import net.minecraft.resources.ResourceLocation;
import net.vulkanic.bridge.VulkanicGalBridge;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;
import static org.mockito.Mockito.*;

class GuiAtlasSpriteExtractionTest {
    @org.junit.jupiter.api.BeforeAll static void bootstrap() {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
    }

    @Test void collectionReusesPublishedPayloadAndRejectsStaleSpriteOrFrame() throws Exception {
        var fields = new LinkedHashMap<java.lang.reflect.Field, Object>();
        var payload = new VulkanicGalBridge.WorldMeshTextureAssetRecord(17, new byte[]{1, 2});
        Map<String, Object> state = Map.of(
            "atlasPayload", new byte[]{1, 2}, "atlasGeneration", 1L,
            "publishedWorldMeshAtlasGeneration", 1L, "publishedWorldMeshAtlasPayload", payload,
            "copiedAtlasSemanticGeneration", 2L, "copiedAtlasSemanticFrameKey", 3L);
        for (var entry : state.entrySet()) {
            var field = RustGalTerrainRenderer.class.getDeclaredField(entry.getKey());
            field.setAccessible(true);
            fields.put(field, field.get(null));
            field.set(null, entry.getValue());
        }
        try (var game = mockStatic(Minecraft.class)) {
            var minecraft = mock(Minecraft.class, RETURNS_DEEP_STUBS);
            game.when(Minecraft::getInstance).thenReturn(minecraft);
            var atlas = mock(TextureAtlas.class);
            when(minecraft.getAtlasManager().getAtlasOrThrow(any())).thenReturn(atlas);
            atlas.width = 64; atlas.height = 32;
            when(atlas.semanticReloadGeneration()).thenReturn(2L);
            when(atlas.semanticSnapshotFrameKey()).thenReturn(3L);
            var sprite = mock(TextureAtlasSprite.class);
            var contents = mock(SpriteContents.class);
            var name = ResourceLocation.parse("minecraft:item/test");
            when(sprite.contents()).thenReturn(contents);
            when(contents.name()).thenReturn(name);
            when(contents.width()).thenReturn(8);
            when(contents.height()).thenReturn(4);
            when(sprite.getX()).thenReturn(16);
            when(sprite.getY()).thenReturn(12);
            when(atlas.getSprite(name)).thenReturn(sprite);
            var region = RustGalTerrainRenderer.requireGuiAtlasSpritePayload(sprite);
            assertSame(payload, region.texture(), "collection must reuse the owner payload, not copy an image");
            assertEquals(64, region.atlasWidth()); assertEquals(32, region.atlasHeight());
            assertEquals(16, region.x()); assertEquals(12, region.y());
            assertEquals(8, region.width()); assertEquals(4, region.height());
            verify(atlas, never()).semanticRawSnapshot();
            when(atlas.getSprite(name)).thenReturn(mock(TextureAtlasSprite.class));
            assertThrows(IllegalStateException.class, () -> RustGalTerrainRenderer.requireGuiAtlasSpritePayload(sprite));
            when(atlas.getSprite(name)).thenReturn(sprite);
            when(atlas.semanticSnapshotFrameKey()).thenReturn(3L, 4L);
            assertThrows(IllegalStateException.class, () -> RustGalTerrainRenderer.requireGuiAtlasSpritePayload(sprite));
        } finally {
            for (var entry : fields.entrySet()) entry.getKey().set(null, entry.getValue());
        }
    }
}
