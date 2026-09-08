package net.vulkanic.world;

import net.minecraft.client.renderer.texture.SemanticAtlasAnimationSource;
import net.minecraft.resources.ResourceLocation;

/** Resource-incarnation-owned semantic events, independent of native readiness. */
public final class AtlasAnimationResource implements AutoCloseable {
    private final SemanticAtlasAnimationSource source;
    private final int semanticTextureId;
    private final ResourceLocation atlas;
    private final AtlasAnimationVisibility visibility;
    private final AtlasAnimationTickDelivery ticks;
    private boolean closed;
    private long lastProducedTick;

    /** Validation only until every atlas consumer and paired timing are proven. */
    public static boolean privateTickDeliveryEnabled() {
        return Boolean.getBoolean("mattmc.dev.rustGalAtlasAnimation");
    }

    /** The ID names a semantic asset, never a Java or native GPU texture handle. */
    public AtlasAnimationResource(ResourceLocation atlas, int semanticTextureId, SemanticAtlasAnimationSource source) {
        if (semanticTextureId <= 0) throw new IllegalArgumentException("Invalid semantic atlas texture identity");
        this.semanticTextureId = semanticTextureId;
        this.atlas = java.util.Objects.requireNonNull(atlas);
        this.source = java.util.Objects.requireNonNull(source);
        visibility = new AtlasAnimationVisibility(atlas, source);
        ticks = new AtlasAnimationTickDelivery(semanticTextureId, source.generation(), 0);
    }

    public SemanticAtlasAnimationSource source() { return source; }
    public int semanticTextureId() { return semanticTextureId; }
    public ResourceLocation atlas() { return atlas; }

    /** Read-only capture evidence; this is not native acceptance or frame selection. */
    public synchronized long producedTickForDiagnostics() { return lastProducedTick; }
    public synchronized boolean producedTickNamedSpriteForDiagnostics(int spriteId) {
        return ticks.lastQueuedTickNamedSpriteForDiagnostics(spriteId);
    }
    public synchronized long lastNonemptyTickNamingSpriteForDiagnostics(int spriteId) {
        return ticks.lastNonemptyTickNamingSpriteForDiagnostics(spriteId);
    }

    public synchronized boolean recordUse(ResourceLocation atlas, ResourceLocation name) {
        return !closed && visibility.recordUse(atlas, name);
    }

    /** Tick production is semantic only; frame selection remains entirely in Rust. */
    public synchronized void enqueueTick(long tick, boolean onlyVisible) {
        requireOpen();
        ticks.enqueue(tick, onlyVisible, visibility);
        lastProducedTick = tick;
    }

    public synchronized void enqueueNextTick(boolean onlyVisible) {
        enqueueTick(Math.addExact(lastProducedTick, 1), onlyVisible);
    }

    synchronized int pendingTickCount() { return ticks.pendingCount(); }

    synchronized boolean drain(long acceptedTextureGeneration, AtlasAnimationTickDelivery.Submit submit) {
        requireOpen();
        if (acceptedTextureGeneration <= 0) throw new IllegalArgumentException("Native texture is not accepted");
        // Resource and native storage generations are different identity domains.
        // Bind transport at delivery; never reset history when staging is delayed.
        return ticks.drain((texture, resourceGeneration, tick, ids, onlyVisible) ->
            submit.accept(texture, acceptedTextureGeneration, tick, ids, onlyVisible));
    }

    synchronized void requireOpen() {
        if (closed) throw new IllegalStateException("Animation resource incarnation is retired");
    }

    @Override
    public synchronized void close() {
        closed = true;
        visibility.clearUses();
        ticks.discard();
    }
}
