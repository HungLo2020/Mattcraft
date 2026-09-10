package net.minecraft.client.dev;

import static org.junit.jupiter.api.Assertions.*;
import org.junit.jupiter.api.Test;

class GraphicsAuditGuiItemPlacementFixtureTest {
    @Test void disabledFixtureDoesNotTouchGuiPlayerOrItem() {
        String key="mattmc.dev.graphicsAuditGuiItemPlacement";
        String old=System.getProperty(key);
        try {
            System.setProperty(key,"false");
            assertFalse(GraphicsAuditGuiItemPlacementFixture.render(null,null,null,0,0,0));
            assertEquals("null",GraphicsAuditGuiItemPlacementFixture.receipt());
        } finally {
            if(old==null) System.clearProperty(key); else System.setProperty(key,old);
        }
    }
}
