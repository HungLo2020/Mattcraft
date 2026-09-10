package net.minecraft.client.dev;

import net.minecraft.world.entity.EntityType;
import net.minecraft.world.entity.item.ItemEntity;
import net.minecraft.world.phys.Vec3;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

class GraphicsAuditDroppedItemFoilFixtureTest {
    @org.junit.jupiter.api.BeforeAll
    static void bootstrap() {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
    }
    @Test
    void frozenVanillaTimerSuppliesExactNonPlayerPartialTick() {
        var timer = new net.minecraft.client.DeltaTracker.Timer(20, 0, value -> value);
        timer.advanceTime(13, true);
        assertNotEquals(1.0F, timer.getGameTimeDeltaPartialTick(false));
        timer.updateFrozenState(true);
        assertEquals(1.0F, timer.getGameTimeDeltaPartialTick(false));
    }
    @Test
    void phaseMutationIsRestrictedAndObservableThroughVanillaField() {
        ItemEntity entity = new ItemEntity(EntityType.ITEM, null);
        assertThrows(IllegalArgumentException.class, () -> GraphicsAuditDroppedItemFoilFixture.pinPhase(entity));
        entity.setId(Integer.MIN_VALUE + 4100);
        entity.tickCount = 250;
        GraphicsAuditDroppedItemFoilFixture.pinPhase(entity);
        assertEquals((float) Math.PI, entity.bobOffs);
        assertEquals(0, entity.tickCount);
    }
    @Test
    void disabledFixtureDoesNotAccessGameAndInvalidCountsReject() {
        String property = GraphicsAuditDroppedItemFoilFixture.PROPERTY;
        String previous = System.getProperty(property);
        try {
            System.setProperty(property, "0");
            GraphicsAuditDroppedItemFoilFixture.beforeRender(null);
            assertEquals("null", GraphicsAuditDroppedItemFoilFixture.receipt(null));
            for (int count : new int[] {1,64}) {
                System.setProperty(property, Integer.toString(count));
                assertEquals(count, GraphicsAuditDroppedItemFoilFixture.requestedCount());
            }
            System.setProperty(property, "2");
            assertThrows(IllegalArgumentException.class, GraphicsAuditDroppedItemFoilFixture::requestedCount);
        } finally {
            if (previous == null) System.clearProperty(property); else System.setProperty(property, previous);
        }
    }
    @Test
    void fixturePositionsAreDistinctAndBoundedForVerticalLook() {
        assertEquals(new Vec3(1,1.65,2),
            GraphicsAuditDroppedItemFoilFixture.position(new Vec3(0,2,0),new Vec3(0,0,1),0));
        Vec3 first = GraphicsAuditDroppedItemFoilFixture.position(Vec3.ZERO,new Vec3(0,1,0),0);
        Vec3 second = GraphicsAuditDroppedItemFoilFixture.position(Vec3.ZERO,new Vec3(0,1,0),1);
        assertTrue(first.distanceTo(second) > .6);
        assertTrue(first.length() < 3);
    }
}
