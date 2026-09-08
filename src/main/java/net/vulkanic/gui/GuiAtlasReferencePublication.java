package net.vulkanic.gui;

import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.HashSet;
import java.util.stream.LongStream;
import java.util.function.BiConsumer;
import java.util.function.ToLongFunction;
import net.vulkanic.bridge.VulkanicGalBridge.GuiAtlasReferenceRecord;
import net.vulkanic.bridge.VulkanicGalBridge.GuiAffineQuadRecord;
import net.vulkanic.bridge.VulkanicGalBridge.GuiTiledQuadRecord;
import net.vulkanic.bridge.VulkanicGalBridge.GuiMeshBatchRecord;
import net.vulkanic.bridge.VulkanicGalBridge.WorldMeshTextureAssetRecord;

/** Bounded semantic declaration table. The frame coordinator supplies synchronization. */
final class GuiAtlasReferencePublication {
    private static final int LIMIT = 4096;
    private final Map<Long, GuiAtlasRegion> references = new LinkedHashMap<>();
    private List<GuiAtlasReferenceRecord> acceptedReferences = List.of();
    private long revision;
    private long acceptedRevision;

    void stage(GuiAtlasRegion reference) {
        var previous = references.get(reference.assetId());
        if (reference.equals(previous)) return;
        if (previous == null && references.size() >= LIMIT)
            throw new IllegalStateException("GUI atlas reference bound exceeded");
        long next = Math.incrementExact(revision);
        references.put(reference.assetId(), reference);
        revision = next;
    }

    void invalidate() {
        long next = Math.incrementExact(revision);
        references.clear();
        revision = next;
    }

    void resetAcceptance() { acceptedRevision = 0; }

    /** Retire declarations absent from the exact command batch about to submit. */
    void retainUsedCommands(List<GuiAffineQuadRecord> affine, List<GuiTiledQuadRecord> tiled,
                            List<GuiMeshBatchRecord> meshes) {
        // Nested layers carry their own semantic source dependencies even when
        // no top-level draw names that source. Retire only after all uses end.
        retainUsed(LongStream.concat(affine.stream().flatMapToLong(quad -> LongStream.concat(
                LongStream.of(quad.assetId()), quad.itemRasterLayers().stream().mapToLong(layer -> layer.assetId()))),
            LongStream.concat(tiled.stream().mapToLong(GuiTiledQuadRecord::assetId),
                meshes.stream().mapToLong(GuiMeshBatchRecord::assetId))));
    }

    /** Retire declarations absent from the exact command batch about to submit. */
    void retainUsed(LongStream assetIds) {
        if (references.isEmpty()) return;
        var active = new HashSet<Long>();
        assetIds.forEach(id -> {
            if (references.containsKey(id)) active.add(id);
        });
        if (active.size() == references.size()) return;
        long next = Math.incrementExact(revision);
        references.keySet().retainAll(active);
        revision = next;
    }

    void flush(ToLongFunction<WorldMeshTextureAssetRecord> acceptedGeneration,
               BiConsumer<Long, List<GuiAtlasReferenceRecord>> publish) {
        // Resolve even an unchanged table: dirty/replaced textures cannot reuse
        // an old GUI acceptance receipt, and reuploads may have new generations.
        var snapshot = references.values().stream().map(region -> region.resolve(acceptedGeneration)).toList();
        if (revision == acceptedRevision && !snapshot.equals(acceptedReferences))
            revision = Math.incrementExact(revision);
        if (revision == acceptedRevision) return;
        long candidate = revision;
        publish.accept(candidate, snapshot);
        acceptedRevision = candidate;
        acceptedReferences = snapshot;
    }
}
