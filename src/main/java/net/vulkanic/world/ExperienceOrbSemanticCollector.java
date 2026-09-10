package net.vulkanic.world;

import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import net.vulkanic.bridge.VulkanicGalBridge.WorldExperienceOrbAssetRecord;
import net.vulkanic.bridge.VulkanicGalBridge.WorldExperienceOrbInstanceRecord;

/** Bounded immutable CPU resource publications. All geometry and raster choices live in Rust. */
final class ExperienceOrbSemanticCollector {
    static final int MAX_SLOTS = 4096;
    private final List<WorldExperienceOrbAssetRecord> slots = new ArrayList<>();
    private final Map<Long, WorldExperienceOrbAssetRecord> dirty = new LinkedHashMap<>();
    private final List<WorldExperienceOrbInstanceRecord> pending = new ArrayList<>();
    private long generation;

    WorldExperienceOrbInstanceRecord enqueue(int icon, int red, int blue, int light,
            float[] entityTransform, float[] cameraOrientation, int entityId, int meshIndex) {
        int slot = pending.size();
        if (slot >= MAX_SLOTS) throw new IllegalStateException("orb semantic residency bound exceeded");
        long key = 0x4f52420000000000L | (slot + 1L);
        var previous = slot < slots.size() ? slots.get(slot) : null;
        boolean changed = previous == null || previous.icon() != icon || previous.red() != red
            || previous.blue() != blue || previous.packedLight() != light;
        long next = changed ? Math.incrementExact(generation) : previous.meshGeneration();
        // Construct/validate both records before mutating retained state.
        var asset = changed ? new WorldExperienceOrbAssetRecord(key,next,icon,red,blue,light) : previous;
        var instance = new WorldExperienceOrbInstanceRecord(key,next,meshIndex,entityTransform,cameraOrientation,entityId);
        if (changed) {
            generation = next;
            if (slot == slots.size()) slots.add(asset); else slots.set(slot,asset);
            dirty.put(key,asset);
        }
        pending.add(instance);
        return instance;
    }

    List<WorldExperienceOrbAssetRecord> dirtyAssets() { return List.copyOf(dirty.values()); }
    void accepted(List<WorldExperienceOrbAssetRecord> assets) {
        for (var asset : assets) dirty.remove(asset.meshKey(),asset);
    }
    void invalidatePublications() { for (var asset : slots) dirty.put(asset.meshKey(),asset); }
    List<WorldExperienceOrbInstanceRecord> pendingInstances() { return List.copyOf(pending); }
    void clearFrame() { pending.clear(); }
    int residentCount() { return slots.size(); }
    int projectedResidentCount() { return Math.max(slots.size(),pending.size()+1); }
    static boolean ownsKey(long key) { return (key & 0xffffffffff000000L) == 0x4f52420000000000L; }

    /** Translate collection ordinals through the ordinary mesh admission prefix. */
    List<WorldExperienceOrbInstanceRecord> remap(int[] admittedPrefix) {
        var output = new ArrayList<WorldExperienceOrbInstanceRecord>(pending.size());
        for (var orb : pending) {
            if (orb.meshIndex() >= admittedPrefix.length)
                throw new IllegalStateException("orb ordering exceeds collected meshes");
            output.add(new WorldExperienceOrbInstanceRecord(orb.meshKey(),orb.meshGeneration(),
                admittedPrefix[orb.meshIndex()],orb.entityTransform(),orb.cameraOrientation(),orb.entityId()));
        }
        return List.copyOf(output);
    }
}
