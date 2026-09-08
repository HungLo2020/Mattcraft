package net.vulkanic.gui;

import java.util.Objects;
import java.util.function.ToLongFunction;
import net.vulkanic.bridge.VulkanicGalBridge.GuiAtlasReferenceRecord;
import net.vulkanic.bridge.VulkanicGalBridge.WorldMeshTextureAssetRecord;

/** Pending semantic region of one immutable copied atlas payload, not a GPU handle. */
record GuiAtlasRegion(long assetId, WorldMeshTextureAssetRecord payload,
                      int atlasWidth, int atlasHeight, int x, int y, int width, int height) {
    GuiAtlasRegion {
        Objects.requireNonNull(payload);
        if (assetId == 0 || payload.textureId() == 0 || atlasWidth <= 0 || atlasHeight <= 0
            || (long) atlasWidth * atlasHeight > 16L * 1024 * 1024
            || x < 0 || y < 0 || width <= 0 || height <= 0
            || (long) x + width > atlasWidth || (long) y + height > atlasHeight) {
            throw new IllegalArgumentException("Invalid semantic GUI atlas region");
        }
    }

    GuiAtlasReferenceRecord resolve(ToLongFunction<WorldMeshTextureAssetRecord> acceptedGeneration) {
        return new GuiAtlasReferenceRecord(assetId, payload.textureId(), acceptedGeneration.applyAsLong(payload),
            atlasWidth, atlasHeight, x, y, width, height);
    }
}
