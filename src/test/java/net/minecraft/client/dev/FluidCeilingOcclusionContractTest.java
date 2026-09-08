package net.minecraft.client.dev;

import net.minecraft.core.Direction;
import net.minecraft.world.phys.shapes.Shapes;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

/** Actual shape operation used by Java OpenGL's fluid-side exposure test. */
class FluidCeilingOcclusionContractTest {
    @Test void aCeilingOnlyOccludesFluidThatReachesItsBoundary() {
        float[] heights = {0.5F, 8.0F / 9.0F, Math.nextDown(Math.nextDown(1.0F)), Math.nextDown(1.0F), 1.0F};
        boolean[] occluded = {false, false, false, true, true};
        for (int i = 0; i < heights.length; i++) {
            var fluid = Shapes.box(0, 0, 0, 1, heights[i], 1);
            assertEquals(occluded[i], Shapes.blockOccludes(fluid, Shapes.block(), Direction.UP), "height=" + heights[i]);
            for (Direction direction : new Direction[]{Direction.DOWN, Direction.NORTH, Direction.SOUTH, Direction.WEST, Direction.EAST})
                assertTrue(Shapes.blockOccludes(fluid, Shapes.block(), direction), direction.toString());
            assertFalse(Shapes.blockOccludes(fluid, Shapes.empty(), Direction.UP));
        }
    }
}
