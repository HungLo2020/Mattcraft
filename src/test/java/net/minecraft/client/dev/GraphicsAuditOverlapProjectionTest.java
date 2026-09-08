package net.minecraft.client.dev;

import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.world.level.block.Blocks;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

class GraphicsAuditOverlapProjectionTest {
    @org.junit.jupiter.api.BeforeAll static void bootstrap() {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
    }

    @Test void observationMatchesAllThreePlanesAndTheCrossingWall() {
        for (Direction forward : new Direction[]{Direction.NORTH, Direction.SOUTH, Direction.WEST, Direction.EAST}) {
            var cells = GraphicsAuditOverlapProjection.cells(BlockPos.ZERO, forward);
            assertEquals(42, cells.size());
            assertEquals(42, cells.stream().map(GraphicsAuditOverlapProjection.Cell::position).distinct().count());
            for (var block : new net.minecraft.world.level.block.Block[]{Blocks.RED_STAINED_GLASS, Blocks.GREEN_STAINED_GLASS, Blocks.BLUE_STAINED_GLASS})
                assertEquals(9, cells.stream().filter(cell -> cell.block() == block).count());
            assertEquals(15, cells.stream().filter(cell -> cell.block() == Blocks.ORANGE_STAINED_GLASS).count());
            var points = GraphicsAuditOverlapProjection.points(BlockPos.ZERO, forward);
            assertEquals(9, points.size());
            assertEquals(9, points.stream().distinct().count());
            for (var point : points) {
                assertTrue(point.y == .5 || point.y == 1.5 || point.y == 2.5);
                var center = net.minecraft.world.phys.Vec3.atCenterOf(BlockPos.ZERO.relative(forward, 2));
                assertEquals(forward.getAxis() == Direction.Axis.X ? center.x : center.z,
                    forward.getAxis() == Direction.Axis.X ? point.x : point.z);
            }
            assertThrows(UnsupportedOperationException.class, () -> points.clear());
        }
    }
}
