package net.vulkanic.world;

import java.util.HashMap;
import java.util.HashSet;
import java.util.Map;
import java.util.TreeSet;
import net.minecraft.client.renderer.texture.SemanticAtlasAnimationSource;
import net.minecraft.resources.ResourceLocation;

/** Collects immutable sprite-use semantics; does not select animation frames. */
final class AtlasAnimationVisibility {
    private final ResourceLocation atlas;
    private final Map<ResourceLocation, Integer> ids;
    private final TreeSet<Integer> used = new TreeSet<>();

    AtlasAnimationVisibility(ResourceLocation atlas, SemanticAtlasAnimationSource source) {
        this.atlas = java.util.Objects.requireNonNull(atlas);
        if (source.sprites().size() > 16384) throw new IllegalArgumentException("Animation visibility bound exceeded");
        var mapping = new HashMap<ResourceLocation, Integer>();
        var uniqueIds = new HashSet<Integer>();
        for (var sprite : source.sprites()) {
            if (sprite.id() <= 0 || sprite.name() == null || !uniqueIds.add(sprite.id())
                || mapping.putIfAbsent(sprite.name(), sprite.id()) != null) {
                throw new IllegalArgumentException("Ambiguous animation sprite identity");
            }
        }
        ids = Map.copyOf(mapping);
    }

    boolean recordUse(ResourceLocation atlas, ResourceLocation name) {
        if (!this.atlas.equals(java.util.Objects.requireNonNull(atlas))) return false;
        // Uses of static sprites are valid but have no animation declaration.
        Integer id = ids.get(java.util.Objects.requireNonNull(name));
        return id != null && used.add(id);
    }

    /** Detaches one tick's owned event payload; later draws form the next set. */
    int[] takeUses() {
        int[] result = snapshotUses();
        clearUses();
        return result;
    }

    int pendingCount() { return used.size(); }
    int[] snapshotUses() { return used.stream().mapToInt(Integer::intValue).toArray(); }
    void clearUses() { used.clear(); }
}
