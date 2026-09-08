package net.vulkanic.world;

import net.vulkanic.bridge.VulkanicGalBridge;
import net.minecraft.client.renderer.texture.TextureAtlas;
import net.vulkanic.VulkanicAPI;
import net.vulkanic.backends.vulkan.VulkanWholeFrameSemanticGpuDevice;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

class ParticleAtlasSnapshotPublicationTest {
    @Test void onlyTheSamePublishedAssetAndSourceVersionCanSkipEncoding() throws Exception {
        var device = VulkanicAPI.class.getDeclaredField("device");
        device.setAccessible(true);
        var oldDevice = device.get(null);
        String flag = "mattmc.dev.rustGalVulkanWholeFrame";
        String oldFlag = System.getProperty(flag);
        try {
        System.setProperty(flag, "true");
        device.set(null, new VulkanWholeFrameSemanticGpuDevice());
        var source = new TextureAtlas(TextureAtlas.LOCATION_PARTICLES);
        var replacement = new TextureAtlas(TextureAtlas.LOCATION_PARTICLES);
        var asset = new VulkanicGalBridge.WorldMeshTextureAssetRecord(101, new byte[]{1});
        var publication = new ParticleAtlasSnapshotPublication(source, 7, 91, asset);
        assertTrue(publication.matches(source, 7, 91, asset));
        assertFalse(publication.matches(replacement, 7, 91, asset),
            "a replacement atlas with the same local generation and frame must be re-published");
        assertFalse(publication.matches(null, 7, 91, asset));
        assertFalse(publication.matches(source, 8, 91, asset), "reload changes source incarnation even with identical pixels");
        assertFalse(publication.matches(source, 7, 92, asset), "a changed semantic frame needs a new payload");
        assertFalse(publication.matches(source, 7, 91, null));
        assertFalse(publication.matches(source, 7, 91,
            new VulkanicGalBridge.WorldMeshTextureAssetRecord(101, new byte[]{1})),
            "an unrelated replacement with the same ID/bytes is not this publication");
        publication.texture().clear();
        assertFalse(publication.matches(source, 7, 91, asset), "a cleared cache can only request re-publication");
        var retired = new ParticleAtlasSnapshotPublication(source, 7, 91, asset);
        retired.source().clear();
        assertFalse(retired.matches(source, 7, 91, asset), "a retired source cannot produce a cache hit");
        assertThrows(IllegalArgumentException.class, () -> new ParticleAtlasSnapshotPublication(source, 0, 91, asset));
        } finally {
            device.set(null, oldDevice);
            if (oldFlag == null) System.clearProperty(flag); else System.setProperty(flag, oldFlag);
        }
    }
}
