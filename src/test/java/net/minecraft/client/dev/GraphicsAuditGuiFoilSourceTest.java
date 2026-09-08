package net.minecraft.client.dev;

import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

class GraphicsAuditGuiFoilSourceTest {
    @Test void observesImmutableFiniteSourceDataAndRejectsAmbiguousOrUnboundedEvidence() {
        GraphicsAuditGuiFoilSource.resetForTest();
        try {
            float[] positions = new float[12], uv = new float[8];
            uv[0] = 0.25F;
            var sample = new GraphicsAuditGuiFoilSource.Sample("minecraft:item/feather", positions, uv);
            uv[0] = 0.5F;
            GraphicsAuditGuiFoilSource.observe(sample);
            assertTrue(GraphicsAuditGuiFoilSource.json().contains("0.25"));
            assertTrue(GraphicsAuditGuiFoilSource.json().contains("\"complete\":true"));
            GraphicsAuditGuiFoilSource.observe(new GraphicsAuditGuiFoilSource.Sample("minecraft:item/feather",positions,uv));
            assertTrue(GraphicsAuditGuiFoilSource.json().contains("\"complete\":false"));
            GraphicsAuditGuiFoilSource.resetForTest();
            for (int i=0; i<65; i++) GraphicsAuditGuiFoilSource.observe(
                new GraphicsAuditGuiFoilSource.Sample("minecraft:item/test"+i,positions,uv));
            assertTrue(GraphicsAuditGuiFoilSource.json().contains("\"complete\":false"));
            assertFalse(GraphicsAuditGuiFoilSource.json().contains("test64"));
            uv[0] = Float.NaN;
            assertThrows(IllegalArgumentException.class, () -> new GraphicsAuditGuiFoilSource.Sample("minecraft:item/test",positions,uv));
        } finally { GraphicsAuditGuiFoilSource.resetForTest(); }
    }
}

