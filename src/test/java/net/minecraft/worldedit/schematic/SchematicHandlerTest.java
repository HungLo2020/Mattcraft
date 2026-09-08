package net.minecraft.worldedit.schematic;

import net.minecraft.SharedConstants;
import net.minecraft.core.Direction;
import net.minecraft.server.Bootstrap;
import net.minecraft.world.level.block.DoorBlock;
import net.minecraft.world.level.block.FenceBlock;
import net.minecraft.world.level.block.FenceGateBlock;
import net.minecraft.world.level.block.HorizontalDirectionalBlock;
import net.minecraft.world.level.block.LeavesBlock;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.state.properties.DoorHingeSide;
import net.minecraft.world.level.block.state.properties.DoubleBlockHalf;
import net.minecraft.worldedit.clipboard.Clipboard;
import net.minecraft.worldedit.math.BlockVector3;
import net.minecraft.worldedit.region.CuboidRegion;
import org.junit.jupiter.api.BeforeAll;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.assertEquals;

class SchematicHandlerTest {
    @BeforeAll
    static void bootstrapMinecraft() {
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
    }

    @Test
    void schematicRoundTripPreservesBlockStateProperties(@TempDir Path tempDir) throws Exception {
        Clipboard clipboard = new Clipboard(
            new CuboidRegion(BlockVector3.ZERO, BlockVector3.at(4, 0, 0)),
            BlockVector3.ZERO
        );

        BlockState persistentLeaves = Blocks.OAK_LEAVES.defaultBlockState()
            .setValue(LeavesBlock.PERSISTENT, true)
            .setValue(LeavesBlock.DISTANCE, 1);
        BlockState openEastGate = Blocks.OAK_FENCE_GATE.defaultBlockState()
            .setValue(HorizontalDirectionalBlock.FACING, Direction.EAST)
            .setValue(FenceGateBlock.OPEN, true)
            .setValue(FenceGateBlock.POWERED, true);
        BlockState connectedFence = Blocks.OAK_FENCE.defaultBlockState()
            .setValue(FenceBlock.NORTH, true)
            .setValue(FenceBlock.EAST, true);
        BlockState lowerDoor = Blocks.OAK_DOOR.defaultBlockState()
            .setValue(DoorBlock.FACING, Direction.SOUTH)
            .setValue(DoorBlock.HINGE, DoorHingeSide.RIGHT)
            .setValue(DoorBlock.OPEN, true)
            .setValue(DoorBlock.HALF, DoubleBlockHalf.LOWER);
        BlockState upperDoor = lowerDoor.setValue(DoorBlock.HALF, DoubleBlockHalf.UPPER);

        clipboard.setBlock(BlockVector3.at(0, 0, 0), persistentLeaves);
        clipboard.setBlock(BlockVector3.at(1, 0, 0), openEastGate);
        clipboard.setBlock(BlockVector3.at(2, 0, 0), connectedFence);
        clipboard.setBlock(BlockVector3.at(3, 0, 0), lowerDoor);
        clipboard.setBlock(BlockVector3.at(4, 0, 0), upperDoor);

        SchematicHandler handler = new SchematicHandler(tempDir.toFile());
        handler.save(clipboard, "stateful");

        Clipboard loaded = handler.load("stateful");

        assertEquals(persistentLeaves, loaded.getBlock(BlockVector3.at(0, 0, 0)));
        assertEquals(openEastGate, loaded.getBlock(BlockVector3.at(1, 0, 0)));
        assertEquals(connectedFence, loaded.getBlock(BlockVector3.at(2, 0, 0)));
        assertEquals(lowerDoor, loaded.getBlock(BlockVector3.at(3, 0, 0)));
        assertEquals(upperDoor, loaded.getBlock(BlockVector3.at(4, 0, 0)));
    }
}
