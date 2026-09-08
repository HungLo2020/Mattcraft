package net.minecraft.client.dev;
import java.util.HashSet;
import net.minecraft.core.BlockPos;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;
class GraphicsAuditLavaFixtureTest {
    @Test void lavaChannelContainsOneSourceAndThreeNaturallyFlowingCells() {
        var source = new BlockPos(145, 98, 530);
        for (var direction : new net.minecraft.core.Direction[] {
                net.minecraft.core.Direction.NORTH, net.minecraft.core.Direction.SOUTH,
                net.minecraft.core.Direction.EAST, net.minecraft.core.Direction.WEST}) {
            var cells = GraphicsAuditLavaFixture.flowingCells(source, direction);
            assertEquals(90, cells.size());
            assertEquals(90, new HashSet<>(cells.stream().map(GraphicsAuditLavaFixture.Cell::position).toList()).size());
            assertEquals(1, cells.stream().filter(cell -> cell.kind() == 2).count());
            for (int offset = 0; offset < 4; offset++) {
                var position = source.relative(direction, offset);
                assertEquals(offset == 0 ? 2 : 0, kind(cells, position));
                assertEquals(-1, kind(cells, position.below()));
                assertEquals(0, kind(cells, position.above()));
                assertEquals(-1, kind(cells, position.relative(direction.getClockWise())));
                assertEquals(-1, kind(cells, position.relative(direction.getCounterClockWise())));
            }
            assertEquals(-1, kind(cells, source.relative(direction, -1)));
            assertEquals(-1, kind(cells, source.relative(direction, 4)));
            var points = GraphicsAuditLavaFixture.flowingSurfacePoints(source, direction);
            assertEquals(9, new HashSet<>(points).size());
            var middle = source.relative(direction, 2);
            for (var point : points) {
                assertTrue(point.x >= middle.getX() + 0.25 && point.x <= middle.getX() + 0.75);
                assertTrue(point.z >= middle.getZ() + 0.25 && point.z <= middle.getZ() + 0.75);
                double forward = (point.x - middle.getX() - 0.5) * direction.getStepX()
                    + (point.z - middle.getZ() - 0.5) * direction.getStepZ();
                assertEquals(middle.getY() + (4 - 2 * forward) / 9.0 - 0.001, point.y, 1e-9);
            }
        }
        assertThrows(IllegalArgumentException.class, () ->
            GraphicsAuditLavaFixture.flowingCells(source, net.minecraft.core.Direction.UP));
    }
    private static int kind(java.util.List<GraphicsAuditLavaFixture.Cell> cells, BlockPos position) {
        return cells.stream().filter(cell -> cell.position().equals(position)).findFirst().orElseThrow().kind();
    }
    @Test void sourceHasCompleteSupportContainmentAndClearance() {
        var source = new BlockPos(145, 99, 530);
        var plan = GraphicsAuditLavaFixture.cells(source);
        assertEquals(75, plan.size());
        assertEquals(75, new HashSet<>(plan.stream().map(GraphicsAuditLavaFixture.Cell::position).toList()).size());
        for (var cell : plan) {
            int dy = cell.position().getY() - source.getY();
            boolean interior = Math.abs(cell.position().getX() - source.getX()) <= 1
                && Math.abs(cell.position().getZ() - source.getZ()) <= 1;
            assertEquals(dy < 0 ? -1 : dy > 0 ? 0 : interior ? 2 : 1, cell.kind());
        }
        assertEquals(9, plan.stream().filter(cell -> cell.kind() == 2).count());
        assertThrows(UnsupportedOperationException.class, plan::clear);
    }
    @Test void nineWitnessesStayInsideTheActualPartialHeightSurface() {
        var source = new BlockPos(145, 99, 530);
        var points = GraphicsAuditLavaFixture.surfacePoints(source);
        assertEquals(9, new HashSet<>(points).size());
        for (var point : points) {
            assertTrue(point.x >= 145.25 && point.x <= 145.75);
            assertTrue(point.z >= 530.25 && point.z <= 530.75);
            assertEquals(99 + 8.0 / 9.0 - 0.001, point.y, 1e-9);
        }
        assertThrows(UnsupportedOperationException.class, points::clear);
    }
}
