package net.minecraft.client.dev;

import java.util.HashSet;
import java.util.stream.Collectors;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

class GraphicsAuditMixedFluidFixtureTest {
    @org.junit.jupiter.api.BeforeAll static void bootstrap() {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
    }

    @Test void enclosureVariantChangesOnlyTheSolidFixtureCells() {
        String key = "mattmc.dev.graphicsAuditMixedFluidEnclosure";
        String previous = System.getProperty(key);
        try {
            var plan = GraphicsAuditMixedFluidFixture.cells(BlockPos.ZERO, Direction.WEST);
            for (String variant : new String[] {"blue_glass", "ice"}) {
                System.setProperty(key, variant);
                var enclosure = variant.equals("ice") ? net.minecraft.world.level.block.Blocks.ICE
                    : net.minecraft.world.level.block.Blocks.BLUE_STAINED_GLASS;
                for (var cell : plan) {
                    assertEquals((cell.water() ? net.minecraft.world.level.block.Blocks.WATER : enclosure).defaultBlockState(), cell.state());
                }
            }
            System.setProperty(key, "unsupported");
            assertThrows(IllegalArgumentException.class, GraphicsAuditMixedFluidFixture::enclosureBlock);
        } finally {
            if (previous == null) System.clearProperty(key); else System.setProperty(key, previous);
        }
    }
    @Test
    void allOrientationsHaveTheSameSealedSourceWaterAndGlassArrangement() {
        for (Direction forward : new Direction[] {Direction.NORTH, Direction.SOUTH, Direction.EAST, Direction.WEST}) {
            BlockPos target = new BlockPos(146, 99, 532);
            var cells = GraphicsAuditMixedFluidFixture.cells(target, forward);
            assertEquals(64, cells.size());
            assertEquals(64, new HashSet<>(cells.stream().map(GraphicsAuditMixedFluidFixture.Cell::position).toList()).size());
            var byPosition = cells.stream().collect(Collectors.toMap(GraphicsAuditMixedFluidFixture.Cell::position, cell -> cell));
            assertEquals(5, cells.stream().filter(GraphicsAuditMixedFluidFixture.Cell::water).count());
            assertFalse(byPosition.get(target).water());
            assertFalse(byPosition.get(target.above()).water());
            assertFalse(byPosition.get(target.relative(forward).relative(forward.getClockWise())).water());
            for (var cell : cells) {
                if (cell.water()) {
                    for (Direction direction : Direction.values()) {
                        assertNotNull(byPosition.get(cell.position().relative(direction)),
                            "every water neighbor must be another source or the sealed glass shell");
                    }
                }
            }
            assertThrows(UnsupportedOperationException.class, () -> cells.clear());
        }
    }

    @Test
    void verticalOrientationsAreRejected() {
        assertThrows(IllegalArgumentException.class,
            () -> GraphicsAuditMixedFluidFixture.cells(BlockPos.ZERO, Direction.UP));
    }

    @Test void bottomWitnessKeepsWaterSupportedAndPlacesStoneToItsNorth() {
        String key = "mattmc.dev.graphicsAuditMixedFluidEnclosure";
        String previous = System.getProperty(key);
        try {
            System.setProperty(key, "bottom_north");
            var target = new BlockPos(146, 99, 532);
            var plan = GraphicsAuditMixedFluidFixture.cells(target, Direction.WEST);
            var cells = plan.stream().collect(Collectors.toMap(GraphicsAuditMixedFluidFixture.Cell::position, cell -> cell));
            var water = target.north();
            var points = GraphicsAuditMixedFluidFixture.bottomSurfacePoints(water);
            assertEquals(9, new HashSet<>(points).size());
            for (var point : points) {
                assertEquals(99.001, point.y, 0.000001);
                assertTrue(point.x >= 146.25 && point.x <= 146.75);
                assertTrue(point.z >= 531.25 && point.z <= 531.75);
            }
            assertThrows(UnsupportedOperationException.class, () -> points.clear());
            assertEquals(64, cells.size());
            assertEquals(5, plan.stream().filter(GraphicsAuditMixedFluidFixture.Cell::water).count());
            assertTrue(cells.get(water).water());
            assertTrue(cells.get(water.north()).state().is(net.minecraft.world.level.block.Blocks.STONE));
            assertTrue(cells.get(water.below()).state().is(net.minecraft.world.level.block.Blocks.BLUE_STAINED_GLASS));
            assertFalse(cells.get(water.below()).state().canOcclude());
            assertEquals(1, plan.stream().filter(cell -> cell.override() != null).count());
            assertThrows(IllegalArgumentException.class,
                () -> GraphicsAuditMixedFluidFixture.cells(target, Direction.NORTH));
        } finally {
            if (previous == null) System.clearProperty(key); else System.setProperty(key, previous);
        }
    }

    @Test void doorVariantHasTwoConsistentSupportedHalvesInFrontOfSourceWater() {
        String key = "mattmc.dev.graphicsAuditMixedFluidEnclosure";
        String previous = System.getProperty(key);
        try {
            System.setProperty(key, "door");
            for (Direction forward : new Direction[] {Direction.NORTH, Direction.SOUTH, Direction.EAST, Direction.WEST}) {
                var plan = GraphicsAuditMixedFluidFixture.cells(BlockPos.ZERO, forward);
                assertEquals(64, plan.size());
                assertEquals(5, plan.stream().filter(GraphicsAuditMixedFluidFixture.Cell::water).count());
                var doors = plan.stream().filter(cell -> cell.override() != null).toList();
                assertEquals(2, doors.size());
                assertEquals(doors.get(0).position().above(), doors.get(1).position());
                for (int i = 0; i < 2; i++) {
                    var door = doors.get(i);
                    assertEquals(net.minecraft.world.level.block.Blocks.OAK_DOOR, door.state().getBlock());
                    assertEquals(forward.getOpposite(), door.state().getValue(net.minecraft.world.level.block.DoorBlock.FACING));
                    assertEquals(i == 0 ? net.minecraft.world.level.block.state.properties.DoubleBlockHalf.LOWER
                        : net.minecraft.world.level.block.state.properties.DoubleBlockHalf.UPPER,
                        door.state().getValue(net.minecraft.world.level.block.DoorBlock.HALF));
                    assertFalse(door.state().getValue(net.minecraft.world.level.block.DoorBlock.OPEN));
                    assertTrue(plan.stream().anyMatch(cell -> cell.water() && cell.position().equals(door.position().relative(forward))));
                }
                assertTrue(plan.stream().anyMatch(cell -> cell.position().equals(doors.get(0).position().below())
                    && cell.state().is(net.minecraft.world.level.block.Blocks.BLUE_STAINED_GLASS)));
            }
        } finally {
            if (previous == null) System.clearProperty(key); else System.setProperty(key, previous);
        }
    }
}
