package net.minecraft.client.dev;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;
class GraphicsAuditSpecialFoilFixtureTest {
    @org.junit.jupiter.api.BeforeAll static void bootstrap() {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
    }
    @Test void fixtureUsesRealSpecialFoilItemsAndAnUnfoiledHeldControl() {
        var items=GraphicsAuditSpecialFoilFixture.items();
        assertEquals(9,items.size());
        for(int slot=0;slot<9;slot++) {
            var expected=slot==0 ? net.minecraft.world.item.Items.APPLE :
                slot%2==1 ? net.minecraft.world.item.Items.CLOCK : net.minecraft.world.item.Items.COMPASS;
            assertTrue(items.get(slot).is(expected));
            assertEquals(1,items.get(slot).getCount());
            assertEquals(slot>0,items.get(slot).hasFoil());
        }
        assertThrows(UnsupportedOperationException.class,items::clear);
        items.get(1).setCount(9);
        assertEquals(1,GraphicsAuditSpecialFoilFixture.items().get(1).getCount());
    }
}
