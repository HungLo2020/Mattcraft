package net.vulkanic.world;

import java.util.ArrayDeque;

/** Bounded transport of semantic tick events; all animation decisions stay in Rust. */
final class AtlasAnimationTickDelivery {
    static final int MAX_PENDING_TICKS = 64;
    static final long MAX_PENDING_VISIBILITY_BYTES = 4L * 1024L * 1024L;
    private final int textureId;
    private final long generation;
    private final ArrayDeque<Event> pending = new ArrayDeque<>(MAX_PENDING_TICKS);
    private long lastQueuedTick;
    private long pendingBytes;
    // Retain at most one immutable, already-allocated semantic event for
    // capture observation (visibility is bounded to 16384 IDs / 64 KiB).
    private Event lastQueuedEvent;
    // One additional bounded immutable event, diagnostics only. Empty catch-up
    // ticks must not erase evidence of the real use which preceded them.
    private Event lastNonemptyQueuedEvent;

    private record Event(long tick, int[] visible, boolean onlyVisible) {}

    AtlasAnimationTickDelivery(int textureId, long generation, long initialTick) {
        if (textureId <= 0 || generation <= 0 || initialTick < 0) {
            throw new IllegalArgumentException("Invalid animation tick epoch");
        }
        this.textureId = textureId;
        this.generation = generation;
        this.lastQueuedTick = initialTick;
    }

    void enqueue(long tick, boolean onlyVisible, AtlasAnimationVisibility visibility) {
        if (lastQueuedTick == Long.MAX_VALUE || tick != lastQueuedTick + 1) {
            throw new IllegalArgumentException("Animation tick events must be consecutive");
        }
        long bytes = (long)visibility.pendingCount() * Integer.BYTES;
        if (pending.size() >= MAX_PENDING_TICKS || bytes > MAX_PENDING_VISIBILITY_BYTES - pendingBytes) {
            throw new IllegalStateException("Animation tick delivery capacity exhausted; event not consumed");
        }
        // Publish ownership only after validation/allocation. A rejected event
        // must not drain uses or advance the transport's sequence cursor.
        var event = new Event(tick, visibility.snapshotUses(), onlyVisible);
        pending.addLast(event);
        pendingBytes += bytes;
        lastQueuedTick = tick;
        lastQueuedEvent = event;
        if (event.visible.length > 0) lastNonemptyQueuedEvent = event;
        visibility.clearUses();
    }

    @FunctionalInterface
    interface Submit {
        boolean accept(int textureId, long generation, long tick, int[] visible, boolean onlyVisible);
    }

    /** False means backend completion backpressure; exceptions also retain the head. */
    boolean drain(Submit submit) {
        while (!pending.isEmpty()) {
            Event event = pending.getFirst();
            // Do not expose the queued event's array to a failed transport.
            if (!submit.accept(textureId, generation, event.tick, event.visible.clone(), event.onlyVisible)) return false;
            pending.removeFirst();
            pendingBytes -= (long)event.visible.length * Integer.BYTES;
        }
        return true;
    }

    int pendingCount() { return pending.size(); }
    long pendingVisibilityBytes() { return pendingBytes; }
    boolean lastQueuedTickNamedSpriteForDiagnostics(int spriteId) {
        return lastQueuedEvent != null
            && java.util.Arrays.binarySearch(lastQueuedEvent.visible, spriteId) >= 0;
    }
    void discard() {
        pending.clear();
        pendingBytes = 0;
        lastQueuedEvent = null;
        lastNonemptyQueuedEvent = null;
    }
    long lastNonemptyTickNamingSpriteForDiagnostics(int spriteId) {
        return lastNonemptyQueuedEvent != null
            && java.util.Arrays.binarySearch(lastNonemptyQueuedEvent.visible, spriteId) >= 0
            ? lastNonemptyQueuedEvent.tick : -1;
    }
}
