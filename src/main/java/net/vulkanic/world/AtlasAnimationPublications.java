package net.vulkanic.world;

import java.util.LinkedHashMap;
import java.util.Map;
import net.vulkanic.bridge.VulkanicGalBridge;

/** Bounded resource transport, not animation scheduling. Caller serializes access. */
final class AtlasAnimationPublications {
    static final int MAX_ATLASES = 64;
    private final Map<Integer, AtlasAnimationPublication> publications = new LinkedHashMap<>();
    // Native uploads have one pending semantic retry. Preserve its identity
    // even if an earlier map entry receives new events before the next pump.
    private Integer retryTexture;

    void register(AtlasAnimationPublication publication, Runnable publishTexture) {
        long pixels = 0, sprites = 0, frames = 0, mips = 0;
        if (!publications.containsKey(publication.textureId()) && publications.size() >= MAX_ATLASES) {
            throw new IllegalStateException("Atlas publication count exceeded");
        }
        var candidates = new java.util.ArrayList<>(publications.values());
        candidates.removeIf(old -> old.textureId() == publication.textureId());
        candidates.add(publication);
        for (var candidate : candidates) {
            sprites = Math.addExact(sprites, candidate.source().sprites().size());
            for (var sprite : candidate.source().sprites()) {
                frames = Math.addExact(frames, sprite.source().frames().size());
                mips = Math.addExact(mips, sprite.source().mips().size());
                for (var mip : sprite.source().mips()) {
                    // Immutable mip construction validated byte length. Do not
                    // clone entire source sheets merely to account residency.
                    pixels = Math.addExact(pixels, Math.multiplyExact(4L,
                        Math.multiplyExact((long)mip.width(), mip.height())));
                }
            }
        }
        if (pixels > 96L * 1024 * 1024 || sprites > 16384 || frames > 65536 || mips > 65536) {
            throw new IllegalStateException("Atlas publication source residency exceeded");
        }
        // Rejected resource publication leaves the old incarnation and retry
        // intact. The caller's existing texture budget is checked here too.
        publishTexture.run();
        publications.put(publication.textureId(), publication);
    }

    void textureAccepted(long generation, VulkanicGalBridge.WorldMeshTextureAssetRecord texture) {
        var publication = publications.get(texture.textureId());
        if (publication == null) return;
        if (publication.matches(texture)) {
            publication.textureAccepted(generation, texture);
        } else {
            publications.remove(texture.textureId());
            if (Integer.valueOf(texture.textureId()).equals(retryTexture)) retryTexture = null;
        }
    }

    void stagePending(AtlasAnimationPublication.Stage stage) {
        for (var publication : publications.values()) publication.flush(stage);
    }

    boolean drain(AtlasAnimationPublication.Stage stage, AtlasAnimationTickDelivery.Submit submit) {
        if (retryTexture != null) {
            var retry = publications.get(retryTexture);
            if (retry == null) throw new IllegalStateException("Missing atlas retry resource");
            if (!drainOne(retry, stage, submit)) return false;
        }
        for (var publication : publications.values()) {
            if (!drainOne(publication, stage, submit)) return false;
        }
        return true;
    }

    private boolean drainOne(AtlasAnimationPublication publication,
        AtlasAnimationPublication.Stage stage, AtlasAnimationTickDelivery.Submit submit) {
        publication.flush(stage);
        // Startup may collect semantic events before native texture acceptance.
        boolean ownsRetry = Integer.valueOf(publication.textureId()).equals(retryTexture);
        if (publication.stagedGeneration() == 0) {
            // A newly published resource is a frame dependency even before
            // its first tick. An old native image with the same ID is not it.
            return false;
        }
        if (publication.pendingTickCount() == 0) {
            if (ownsRetry) retryTexture = null;
            return true;
        }
        retryTexture = publication.textureId();
        if (!publication.drainTicks(submit)) return false;
        retryTexture = null;
        return true;
    }

    int size() { return publications.size(); }
    boolean contains(AtlasAnimationResource resource) {
        var publication = publications.get(resource.semanticTextureId());
        return publication != null && publication.owns(resource);
    }
    void clear() { publications.clear(); retryTexture = null; }
}
