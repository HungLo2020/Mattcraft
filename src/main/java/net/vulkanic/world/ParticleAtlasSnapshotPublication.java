package net.vulkanic.world;

import net.vulkanic.bridge.VulkanicGalBridge;
import net.minecraft.client.renderer.texture.TextureAtlas;

/** CPU publication cache; cannot pin a retired payload and contains no GPU object. */
record ParticleAtlasSnapshotPublication(java.lang.ref.WeakReference<TextureAtlas> source, long generation, long frameKey,
    java.lang.ref.WeakReference<VulkanicGalBridge.WorldMeshTextureAssetRecord> texture) {
    ParticleAtlasSnapshotPublication(TextureAtlas source, long generation, long frameKey,
        VulkanicGalBridge.WorldMeshTextureAssetRecord texture) {
        this(new java.lang.ref.WeakReference<>(java.util.Objects.requireNonNull(source)), generation, frameKey,
            new java.lang.ref.WeakReference<>(java.util.Objects.requireNonNull(texture)));
    }
    ParticleAtlasSnapshotPublication {
        if (generation <= 0) throw new IllegalArgumentException("Invalid particle atlas generation");
        java.util.Objects.requireNonNull(source);
        java.util.Objects.requireNonNull(texture);
    }
    boolean matches(TextureAtlas currentSource, long currentGeneration, long currentFrameKey,
        VulkanicGalBridge.WorldMeshTextureAssetRecord currentTexture) {
        // Generations are local to each TextureAtlas instance, not globally unique.
        return currentSource != null && source.get() == currentSource
            && generation == currentGeneration && frameKey == currentFrameKey
            && currentTexture != null && texture.get() == currentTexture;
    }
}
