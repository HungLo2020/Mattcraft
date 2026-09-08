package net.vulkanic.world;

import net.minecraft.client.renderer.texture.SemanticAtlasAnimationSource;
import net.vulkanic.bridge.VulkanicGalBridge;

/** Transport receipt for one immutable atlas resource, not an animation clock. */
final class AtlasAnimationPublication {
    private final VulkanicGalBridge.WorldMeshTextureAssetRecord texture;
    private final SemanticAtlasAnimationSource source;
    private final AtlasAnimationResource resource;
    private long pendingGeneration;
    private long stagedGeneration;

    AtlasAnimationPublication(VulkanicGalBridge.WorldMeshTextureAssetRecord texture,
        AtlasAnimationResource resource) {
        this.texture = java.util.Objects.requireNonNull(texture);
        this.resource = java.util.Objects.requireNonNull(resource);
        this.source = resource.source();
        if (texture.textureId() != resource.semanticTextureId()
            || source.generation() <= 0 || source.mipCount() != texture.mipPngBytes().size() + 1
            || source.mipCount() != texture.requestedMipLevels()) {
            throw new IllegalArgumentException("Incoherent atlas animation publication");
        }
    }

    boolean matches(VulkanicGalBridge.WorldMeshTextureAssetRecord accepted) {
        return texture.sameContent(accepted);
    }

    void textureAccepted(long generation, VulkanicGalBridge.WorldMeshTextureAssetRecord accepted) {
        resource.requireOpen();
        if (generation <= 0 || !matches(accepted) || generation < stagedGeneration
            || generation < pendingGeneration || stagedGeneration != 0 && generation != stagedGeneration) {
            throw new IllegalArgumentException("Stale or mismatched animation texture receipt");
        }
        if (generation != stagedGeneration) pendingGeneration = generation;
    }

    boolean pending() { return pendingGeneration != 0; }
    int textureId() { return texture.textureId(); }
    boolean owns(AtlasAnimationResource candidate) { return resource == candidate; }
    SemanticAtlasAnimationSource source() { return source; }
    long stagedGeneration() { return stagedGeneration; }
    int spriteCount() { return source.sprites().size(); }
    int pendingTickCount() { return resource.pendingTickCount(); }
    boolean recordSpriteUse(net.minecraft.resources.ResourceLocation atlas, net.minecraft.resources.ResourceLocation name) {
        return resource.recordUse(atlas, name);
    }

    void enqueueTick(long tick, boolean onlyVisible) {
        resource.enqueueTick(tick, onlyVisible);
    }

    boolean drainTicks(AtlasAnimationTickDelivery.Submit submit) {
        if (pending() || stagedGeneration == 0) throw new IllegalStateException("Animation resource epoch is not staged");
        return resource.drain(stagedGeneration, submit);
    }

    boolean drainTicks(VulkanicGalBridge bridge) {
        java.util.Objects.requireNonNull(bridge);
        return drainTicks((textureId, generation, tick, visible, onlyVisible) ->
            bridge.tickAtlasAnimation(textureId, generation, tick, visible, onlyVisible).accepted());
    }

    @FunctionalInterface
    interface Stage {
        VulkanicGalBridge.Status accept(int textureId, long generation, long initialTick,
            SemanticAtlasAnimationSource source);
    }

    VulkanicGalBridge.Status flush(Stage stage) {
        if (!pending()) return null;
        resource.requireOpen();
        long generation = pendingGeneration;
        // Tick zero belongs to resource creation, not staging time. Earlier
        // events remain in the resource's FIFO and are delivered after staging.
        var result = stage.accept(texture.textureId(), generation, 0, source);
        stagedGeneration = generation;
        pendingGeneration = 0;
        return result;
    }
}
