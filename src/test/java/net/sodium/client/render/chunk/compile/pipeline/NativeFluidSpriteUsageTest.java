package net.sodium.client.render.chunk.compile.pipeline;

import java.util.ArrayList;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

class NativeFluidSpriteUsageTest {
    @Test void everyEmittedFluidSpriteSurvivesSemanticExtraction() {
        int[] bits = {1, 2, 4, 256, 512};
        String[] names = {"water_still", "water_flow", "water_overlay", "lava_still", "lava_flow"};
        for (int selection = 0; selection < 32; selection++) {
            int mask = 0;
            var expected = new ArrayList<String>();
            for (int i = 0; i < bits.length; i++) {
                if ((selection & (1 << i)) != 0) {
                    mask |= bits[i];
                    expected.add("minecraft:block/" + names[i]);
                }
            }
            var actual = NativeStaticBlockModelRegistry.nativeFluidSpriteNames(mask);
            assertEquals(expected, actual);
            assertThrows(UnsupportedOperationException.class, () -> actual.add("other"));
        }
    }
}
