package net.vulkanic.world;

import net.vulkanic.bridge.VulkanicGalBridge.WorldMeshTextureAssetRecord;

/** Immutable image-resource region; no GPU handles or animation frame selection. */
public record AtlasSpritePayload(WorldMeshTextureAssetRecord texture,
        int atlasWidth, int atlasHeight, int x, int y, int width, int height) {
    public AtlasSpritePayload {
        if (texture == null || atlasWidth <= 0 || atlasHeight <= 0
                || (long) atlasWidth * atlasHeight > 16L * 1024 * 1024
                || x < 0 || y < 0 || width <= 0 || height <= 0
                || (long) x + width > atlasWidth || (long) y + height > atlasHeight) {
            throw new IllegalArgumentException("Invalid semantic atlas sprite region");
        }
    }
}
