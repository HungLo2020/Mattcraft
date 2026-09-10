package net.vulkanic.bridge;

import java.lang.foreign.Arena;
import java.lang.foreign.ValueLayout;
import java.util.List;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;
import static net.vulkanic.bridge.VulkanicGalBridge.*;

final class ExperienceOrbAssetEncodingTest {
    @Test
    void encodesOnlyImmutableAppearanceAndResourceIdentity() {
        try (var arena = Arena.ofConfined()) {
            var orb = new WorldExperienceOrbAssetRecord(0x0b01, 2, 10, 127, 5, 0x00b00070);
            var bytes = encodeExperienceOrbAssets(arena, List.of(orb));
            assertEquals(40, bytes.byteSize());
            assertEquals(40, bytes.get(ValueLayout.JAVA_INT, 0));
            assertEquals(10, bytes.get(ValueLayout.JAVA_INT, 4));
            assertEquals(0x0b01, bytes.get(ValueLayout.JAVA_LONG, 8));
            assertEquals(2, bytes.get(ValueLayout.JAVA_LONG, 16));
            assertEquals(127, bytes.get(ValueLayout.JAVA_INT, 24));
            assertEquals(5, bytes.get(ValueLayout.JAVA_INT, 28));
            assertEquals(0x00b00070, bytes.get(ValueLayout.JAVA_INT, 32));
            assertEquals(0, bytes.get(ValueLayout.JAVA_INT, 36));
            assertEquals(0, encodeExperienceOrbAssets(arena, List.of()).byteSize());
        }
    }

    @Test
    void rejectsOversizedArraysBeforeAllocationOrElementAccess() {
        var oversized = new java.util.AbstractList<WorldExperienceOrbAssetRecord>() {
            public int size() { return 16_385; }
            public WorldExperienceOrbAssetRecord get(int index) { throw new AssertionError("must check count first"); }
        };
        try (var arena = Arena.ofConfined()) {
            assertThrows(IllegalArgumentException.class, () -> encodeExperienceOrbAssets(arena, oversized));
        }
    }

    @Test
    void rejectsInvalidAppearanceInsteadOfWrappingChannels() {
        assertThrows(IllegalArgumentException.class, () -> new WorldExperienceOrbAssetRecord(0,2,0,0,0,0));
        assertThrows(IllegalArgumentException.class, () -> new WorldExperienceOrbAssetRecord(7,0,0,0,0,0));
        for (int icon : new int[] {-1,11,Integer.MAX_VALUE})
            assertThrows(IllegalArgumentException.class, () -> new WorldExperienceOrbAssetRecord(7,2,icon,0,0,0));
        for (int color : new int[] {-1,256,Integer.MAX_VALUE}) {
            assertThrows(IllegalArgumentException.class, () -> new WorldExperienceOrbAssetRecord(7,2,0,color,0,0));
            assertThrows(IllegalArgumentException.class, () -> new WorldExperienceOrbAssetRecord(7,2,0,0,color,0));
        }
    }
}
