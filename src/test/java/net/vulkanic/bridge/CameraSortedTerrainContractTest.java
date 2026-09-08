package net.vulkanic.bridge;

import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

class CameraSortedTerrainContractTest {
    private static VulkanicGalBridge.WorldMeshInstanceRecord instance(int stratum, int section, int depth, int flags) {
        return new VulkanicGalBridge.WorldMeshInstanceRecord(stratum, 1, 1, section, depth, 1, 0,
            0xffffffff, new float[]{1,0,0,0, 0,1,0,0, 0,0,1,0, 2,3,4,1},
            1280, 720, 0, 0, 0, flags, -1);
    }

    @Test void explicitCameraSortIsLimitedToCompleteTranslucentTerrain() {
        assertEquals(2, instance(60, -1, 2, 2).flags());
        assertThrows(IllegalArgumentException.class, () -> instance(67, -1, 2, 2));
        assertThrows(IllegalArgumentException.class, () -> instance(60, 0, 2, 2));
        assertEquals(2, instance(60, -1, 1, 2).flags());
        assertThrows(IllegalArgumentException.class, () -> instance(60, -1, 0, 2));
        assertThrows(IllegalArgumentException.class, () -> instance(60, -1, 2, 4));
        assertEquals(0, instance(60, -1, 1, 0).flags());
    }
}
