package net.vulkanic.world;

import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.assertEquals;

class TerrainLightCoordinateTest {
    @Test void everyCompactLightByteSurvivesTheSemanticCopy() {
        for (int block = 0; block < 256; block++) {
            for (int sky = 0; sky < 256; sky++) {
                int compact = 0xabcd0000 | block | (sky << 8);
                assertEquals(block | (sky << 16), RustGalTerrainRenderer.decodeLight(compact, false));
                assertEquals(sky | (block << 16), RustGalTerrainRenderer.decodeLight(compact, true));
            }
        }
    }
}
