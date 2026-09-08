package net.sodium.client.render;

import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.assertEquals;

class AppearanceDiagnosticLimitTest {
    @Test void requestedVertexReceiptIsAlwaysBounded() {
        for (int requested : new int[] {Integer.MIN_VALUE, -1, 0, 1, 32, 1024, 4096, 4097, Integer.MAX_VALUE}) {
            assertEquals(Math.max(0, Math.min(4096, requested)),
                StaticTerrainParityDiagnostics.appearanceSampleLimit(requested));
        }
    }
}
