package net.vulkanic.world;

import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.atomic.AtomicInteger;
import net.minecraft.client.renderer.texture.SemanticAtlasAnimationSource;
import net.minecraft.client.renderer.texture.SpriteContents;
import net.minecraft.resources.ResourceLocation;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

class AtlasAnimationTickDeliveryTest {
    private static final ResourceLocation ATLAS = ResourceLocation.withDefaultNamespace("textures/atlas/blocks.png");
    private static ResourceLocation name(int id) { return ResourceLocation.withDefaultNamespace("audit/sprite" + id); }

    @Test
    void lastProducedVisibilityObservationSurvivesDeliveryButNotNextTickOrRetirement() {
        var uses = visibility();
        var queue = new AtlasAnimationTickDelivery(17, 23, 0);
        assertFalse(queue.lastQueuedTickNamedSpriteForDiagnostics(1));
        assertEquals(-1, queue.lastNonemptyTickNamingSpriteForDiagnostics(1));
        uses.recordUse(ATLAS, name(1));
        queue.enqueue(1, true, uses);
        assertTrue(queue.lastQueuedTickNamedSpriteForDiagnostics(1));
        assertEquals(1, queue.lastNonemptyTickNamingSpriteForDiagnostics(1));
        assertFalse(queue.lastQueuedTickNamedSpriteForDiagnostics(2));
        assertThrows(IllegalArgumentException.class, () -> queue.enqueue(3, true, uses));
        assertTrue(queue.lastQueuedTickNamedSpriteForDiagnostics(1));
        assertTrue(queue.drain((texture, generation, tick, ids, onlyVisible) -> {
            ids[0] = 2;
            return true;
        }));
        assertTrue(queue.lastQueuedTickNamedSpriteForDiagnostics(1));
        assertEquals(0, queue.pendingCount());
        queue.enqueue(2, true, uses);
        assertFalse(queue.lastQueuedTickNamedSpriteForDiagnostics(1));
        assertEquals(1, queue.lastNonemptyTickNamingSpriteForDiagnostics(1));
        uses.recordUse(ATLAS, name(2));
        queue.enqueue(3, true, uses);
        assertTrue(queue.lastQueuedTickNamedSpriteForDiagnostics(2));
        assertEquals(-1, queue.lastNonemptyTickNamingSpriteForDiagnostics(1));
        assertEquals(3, queue.lastNonemptyTickNamingSpriteForDiagnostics(2));
        queue.discard();
        assertFalse(queue.lastQueuedTickNamedSpriteForDiagnostics(2));
        assertEquals(-1, queue.lastNonemptyTickNamingSpriteForDiagnostics(2));
    }
    private static AtlasAnimationVisibility visibility() {
        var pixels = new SpriteContents.SemanticAnimationSource(1, 1, 1, false,
            List.of(new SpriteContents.SemanticAnimationFrame(0, 1)),
            List.of(new SpriteContents.SemanticAnimationMip(1, 1, new byte[4])));
        return new AtlasAnimationVisibility(net.minecraft.client.renderer.texture.TextureAtlas.LOCATION_BLOCKS, new SemanticAtlasAnimationSource(7, 2, 1, 1, List.of(
            new SemanticAtlasAnimationSource.Sprite(1, name(1), 0, 0, pixels),
            new SemanticAtlasAnimationSource.Sprite(2, name(2), 1, 0, pixels))));
    }

    @Test
    void backpressureAndFailureRetainExactEpochEventAndDoNotReplayAcceptedTicks() {
        var uses = visibility();
        var queue = new AtlasAnimationTickDelivery(17, 23, 39);
        uses.recordUse(ATLAS, name(1));
        queue.enqueue(40, true, uses);
        uses.recordUse(ATLAS, name(2));
        queue.enqueue(41, false, uses);
        assertFalse(queue.drain((texture, generation, tick, ids, onlyVisible) -> {
            assertEquals(17, texture);
            assertEquals(23, generation);
            assertEquals(40, tick);
            assertArrayEquals(new int[]{1}, ids);
            assertTrue(onlyVisible);
            ids[0] = 99;
            return false;
        }));
        assertThrows(IllegalStateException.class, () -> queue.drain((texture, generation, tick, ids, onlyVisible) -> {
            assertArrayEquals(new int[]{1}, ids);
            ids[0] = 98;
            throw new IllegalStateException("injected transport failure");
        }));
        var accepted = new ArrayList<Long>();
        assertTrue(queue.drain((texture, generation, tick, ids, onlyVisible) -> {
            accepted.add(tick);
            assertArrayEquals(new int[]{tick == 40 ? 1 : 2}, ids);
            assertEquals(tick == 40, onlyVisible);
            return true;
        }));
        assertEquals(List.of(40L, 41L), accepted);
        assertEquals(0, queue.pendingCount());
        assertEquals(0, queue.pendingVisibilityBytes());
        assertTrue(queue.drain((texture, generation, tick, ids, onlyVisible) -> fail("must not replay accepted event")));
    }

    @Test
    void fullQueueRejectsBeforeDrainingVisibilityOrAdvancingSequence() {
        var uses = visibility();
        var queue = new AtlasAnimationTickDelivery(17, 23, 0);
        for (int tick = 1; tick <= AtlasAnimationTickDelivery.MAX_PENDING_TICKS; tick++) {
            uses.recordUse(ATLAS, name(1));
            queue.enqueue(tick, true, uses);
        }
        assertEquals(64, queue.pendingCount());
        assertEquals(256, queue.pendingVisibilityBytes());
        uses.recordUse(ATLAS, name(2));
        assertThrows(IllegalStateException.class, () -> queue.enqueue(65, true, uses));
        assertArrayEquals(new int[]{2}, uses.snapshotUses());
        AtomicInteger attempts = new AtomicInteger();
        assertFalse(queue.drain((texture, generation, tick, ids, onlyVisible) -> attempts.incrementAndGet() == 1));
        assertEquals(63, queue.pendingCount());
        queue.enqueue(65, true, uses);
        var ticks = new ArrayList<Long>();
        queue.drain((texture, generation, tick, ids, onlyVisible) -> {
            ticks.add(tick);
            assertArrayEquals(new int[]{tick == 65 ? 2 : 1}, ids);
            return true;
        });
        assertEquals(2L, ticks.getFirst());
        assertEquals(65L, ticks.getLast());
        assertEquals(64, ticks.size());
        uses.recordUse(ATLAS, name(1));
        assertThrows(IllegalArgumentException.class, () -> queue.enqueue(67, true, uses));
        assertArrayEquals(new int[]{1}, uses.snapshotUses());
        queue.enqueue(66, true, uses);
    }

    @Test
    void maximumLegalVisibilityHistoryStaysWithinItsPayloadBudget() {
        var pixels = new SpriteContents.SemanticAnimationSource(1, 1, 1, false,
            List.of(new SpriteContents.SemanticAnimationFrame(0, 1)),
            List.of(new SpriteContents.SemanticAnimationMip(1, 1, new byte[4])));
        var sprites = new ArrayList<SemanticAtlasAnimationSource.Sprite>();
        for (int id = 1; id <= 16384; id++) {
            sprites.add(new SemanticAtlasAnimationSource.Sprite(id, name(id), id - 1, 0, pixels));
        }
        var uses = new AtlasAnimationVisibility(net.minecraft.client.renderer.texture.TextureAtlas.LOCATION_BLOCKS, new SemanticAtlasAnimationSource(7, 16384, 1, 1, sprites));
        var queue = new AtlasAnimationTickDelivery(17, 23, 0);
        for (int tick = 1; tick <= 64; tick++) {
            for (var sprite : sprites) uses.recordUse(ATLAS, sprite.name());
            queue.enqueue(tick, true, uses);
        }
        assertEquals(AtlasAnimationTickDelivery.MAX_PENDING_VISIBILITY_BYTES, queue.pendingVisibilityBytes());
        uses.recordUse(ATLAS, name(1));
        assertThrows(IllegalStateException.class, () -> queue.enqueue(65, false, uses));
        assertArrayEquals(new int[]{1}, uses.snapshotUses());
        assertTrue(queue.drain((texture, generation, tick, ids, onlyVisible) -> {
            assertEquals(16384, ids.length);
            for (int index = 0; index < ids.length; index++) assertEquals(index + 1, ids[index]);
            return true;
        }));
        assertEquals(0, queue.pendingVisibilityBytes());
        queue.enqueue(65, false, uses);
        assertEquals(Integer.BYTES, queue.pendingVisibilityBytes());
    }

    @Test
    void invalidEpochAndCounterOverflowRejectWithoutConsumption() {
        assertThrows(IllegalArgumentException.class, () -> new AtlasAnimationTickDelivery(0, 1, 0));
        assertThrows(IllegalArgumentException.class, () -> new AtlasAnimationTickDelivery(1, 0, 0));
        var uses = visibility();
        uses.recordUse(ATLAS, name(1));
        var queue = new AtlasAnimationTickDelivery(1, 1, Long.MAX_VALUE);
        assertThrows(IllegalArgumentException.class, () -> queue.enqueue(Long.MIN_VALUE, true, uses));
        assertArrayEquals(new int[]{1}, uses.snapshotUses());
    }
}
