package net.minecraft.client.dev;

import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

class GraphicsAuditShieldFoilFixtureTest {
    @Test void patternedControlHasDistinctOrderedLayersAndIndependentStacks() {
        var foil=GraphicsAuditPatternedShieldFixture.items(true).getFirst();
        assertTrue(foil.hasFoil());
        assertEquals(GraphicsAuditPatternedShieldFixture.items(false).getFirst().get(net.minecraft.core.component.DataComponents.BANNER_PATTERNS),
            foil.get(net.minecraft.core.component.DataComponents.BANNER_PATTERNS));
        var items=GraphicsAuditPatternedShieldFixture.items();
        var stack=items.getFirst();
        assertFalse(stack.hasFoil());
        assertEquals(net.minecraft.world.item.DyeColor.YELLOW,stack.get(net.minecraft.core.component.DataComponents.BASE_COLOR));
        var layers=stack.get(net.minecraft.core.component.DataComponents.BANNER_PATTERNS).layers();
        assertEquals(2,layers.size());
        assertEquals("minecraft:cross",layers.get(0).pattern().value().assetId().toString());
        assertEquals(net.minecraft.world.item.DyeColor.RED,layers.get(0).color());
        assertEquals("minecraft:border",layers.get(1).pattern().value().assetId().toString());
        assertEquals(net.minecraft.world.item.DyeColor.BLUE,layers.get(1).color());
        stack.setCount(9);
        assertEquals(1,GraphicsAuditPatternedShieldFixture.items().getFirst().getCount());
    }
    @org.junit.jupiter.api.BeforeAll static void bootstrap() {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
    }
    @Test void shieldAndFoilControlDifferOnlyInExplicitFoilComponent() {
        for (boolean foil : new boolean[]{false, true}) {
            var items = GraphicsAuditShieldFoilFixture.items(foil);
            assertEquals(9, items.size());
            assertTrue(items.getFirst().is(net.minecraft.world.item.Items.SHIELD));
            assertEquals(1, items.getFirst().getCount());
            assertEquals(foil, items.getFirst().hasFoil());
            for (int i = 1; i < 9; i++) assertTrue(items.get(i).isEmpty());
            assertThrows(UnsupportedOperationException.class, items::clear);
            items.getFirst().setCount(3);
            assertEquals(1, GraphicsAuditShieldFoilFixture.items(foil).getFirst().getCount());
        }
    }
}
