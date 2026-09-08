package net.minecraft.client.dev;
import java.util.HashSet;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;
class GraphicsAuditFlowingWaterFixtureTest {
    @Test void channelHasOneSourceSealedBottomAndEndsAndOpenAirAbove() {
        for (Direction direction : new Direction[] {Direction.NORTH, Direction.SOUTH, Direction.EAST, Direction.WEST}) {
            var plan = GraphicsAuditFlowingWaterFixture.cells(new BlockPos(146, 99, 532), direction);
            assertEquals(150, plan.size());
            assertEquals(150, new HashSet<>(plan.stream().map(GraphicsAuditFlowingWaterFixture.Cell::position).toList()).size());
            assertEquals(1, plan.stream().filter(c -> c.kind() == 1).count());
            var source = plan.stream().filter(c -> c.kind() == 1).findFirst().orElseThrow().position();
            for (int x = 0; x < 8; x++) {
                BlockPos p = source.relative(direction, x);
                assertEquals(x == 0 ? 1 : 0, kindAt(plan, p));
                assertEquals(-1, kindAt(plan, p.below()));
                assertEquals(0, kindAt(plan, p.above()));
                assertEquals(-1, kindAt(plan, p.relative(direction.getClockWise())));
                assertEquals(-1, kindAt(plan, p.relative(direction.getCounterClockWise())));
            }
            assertEquals(-1, kindAt(plan, source.relative(direction, -1)));
            assertEquals(-1, kindAt(plan, source.relative(direction, 8)));
            assertThrows(UnsupportedOperationException.class, () -> plan.clear());
        }
    }
    private static int kindAt(java.util.List<GraphicsAuditFlowingWaterFixture.Cell> plan, BlockPos p) {
        return plan.stream().filter(c -> c.position().equals(p)).findFirst().orElseThrow().kind();
    }
    @Test void verticalChannelIsRejected() {
        assertThrows(IllegalArgumentException.class,
            () -> GraphicsAuditFlowingWaterFixture.cells(BlockPos.ZERO, Direction.UP));
    }
    @Test void surfaceWitnessesAreInsideFiveFlowingCellsInEveryOrientation() {
        var source = new BlockPos(145, 98, 530);
        for (Direction direction : new Direction[] {Direction.NORTH, Direction.SOUTH, Direction.EAST, Direction.WEST}) {
            var points = GraphicsAuditFlowingWaterFixture.surfacePoints(source, direction);
            assertEquals(15, new HashSet<>(points).size());
            for (int index = 0; index < points.size(); index++) {
                int level = 2 + index / 3;
                var cell = source.relative(direction, level);
                var point = points.get(index);
                assertTrue(point.x >= cell.getX() + 0.25 && point.x <= cell.getX() + 0.75);
                assertTrue(point.z >= cell.getZ() + 0.25 && point.z <= cell.getZ() + 0.75);
                assertEquals(cell.getY() + (8 - level) / 9.0 - 0.001, point.y, 1e-9);
            }
        }
    }
}
