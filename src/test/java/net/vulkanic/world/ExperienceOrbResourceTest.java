package net.vulkanic.world;

import java.io.ByteArrayInputStream;
import java.util.List;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.server.packs.PackResources;
import net.minecraft.server.packs.PackType;
import net.minecraft.server.packs.resources.FallbackResourceManager;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;
import static org.mockito.Mockito.*;

final class ExperienceOrbResourceTest {
    @org.junit.jupiter.api.BeforeAll static void bootstrap() {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
    }
    @Test void vanillaAndOverridesBothCopyTheResolvedResourceWithoutRowConversion() throws Exception {
        var identity = ResourceLocation.withDefaultNamespace("textures/entity/experience_orb.png");
        var vanilla = mock(PackResources.class);
        var override = mock(PackResources.class);
        when(vanilla.packId()).thenReturn("vanilla");
        when(override.packId()).thenReturn("override");
        byte[] base = {1,2,3,4}, replacement = {5,6,7,8};
        when(vanilla.getResource(PackType.CLIENT_RESOURCES,identity)).thenReturn(()->new ByteArrayInputStream(base));
        when(override.getResource(PackType.CLIENT_RESOURCES,identity)).thenReturn(()->new ByteArrayInputStream(replacement));
        for (var packs : List.of(List.of(vanilla),List.of(vanilla,override),List.of(vanilla))) {
            var manager = new FallbackResourceManager(PackType.CLIENT_RESOURCES,"minecraft");
            packs.forEach(manager::push);
            var resource = manager.getResource(identity).orElseThrow();
            var asset = RustGalWorldPrimitiveRenderer.copyExperienceOrbTexture(resource);
            assertArrayEquals(packs.size()==1 ? base : replacement,asset.pngBytes());
            assertEquals(0x4f524233,asset.textureId());
            assertEquals(1,asset.requestedMipLevels());
        }
    }
}
